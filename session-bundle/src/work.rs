//! Cooperative, process-local archive continuations. One parked worker retains
//! the existing streaming/hash/index call stack; every resumed step receives a
//! new budget. No serialized progress can restore trust or publication rights.
use crate::*;
use myelin_session::{RecoveryCancellation, SessionStore};
use serde::{Deserialize, Serialize, Serializer};
use std::{
    cell::RefCell,
    fmt,
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveBudget {
    pub max_steps: u64,
    /// Known blob I/O lengths or conservative 128 MiB reservations for opaque
    /// SessionStore record reads. This is not physical disk I/O or RSS.
    pub max_io_bytes: u64,
    pub max_elapsed_ms: u64,
}
impl Default for ArchiveBudget {
    fn default() -> Self {
        Self { max_steps: 100_000, max_io_bytes: 8 * 1024 * 1024 * 1024, max_elapsed_ms: 30_000 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArchiveStage {
    Inventory,
    Packages,
    Index,
    Dependencies,
    Finality,
    Export,
    Pack,
    Assemble,
    Publish,
    Retention,
    Collection,
    BackupCapture,
    BackupFiles,
    BackupArchives,
    Restore,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArchivePauseReason {
    Steps,
    Bytes,
    Deadline,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveUsage {
    pub steps: u64,
    pub io_bytes: u64,
    pub elapsed_ms: u64,
}
impl ArchiveUsage {
    fn add(self, other: Self) -> Result<Self> {
        let add = |a: u64, b: u64| a.checked_add(b).ok_or_else(|| BundleError::new(ErrorCode::Limit, "archive usage overflow"));
        Ok(Self {
            steps: add(self.steps, other.steps)?,
            io_bytes: add(self.io_bytes, other.io_bytes)?,
            elapsed_ms: add(self.elapsed_ms, other.elapsed_ms)?,
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ArchiveProgress {
    pub stage: ArchiveStage,
    pub position: Option<String>,
    pub step: ArchiveUsage,
    pub total: ArchiveUsage,
    /// Work that must fit in one grant before the pending operation starts.
    /// `elapsed_ms` is zero: synchronous I/O duration cannot be known in advance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_operation: Option<ArchiveUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<RetentionProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<CollectionProgress>,
}
pub enum ArchiveOutput {
    Inventory(InventoryReport),
    Verified { store: Arc<BundleStore>, report: FinalityReport },
    Exported(BundleManifest),
    Packed(PackedFiles),
    Assembled(FilePackageReport),
    Retained(RetentionProgress),
    Collected(CollectionReport),
    BackedUp(BackupManifest),
    Restored { store: Arc<myelin_session_store_rocksdb::RocksSessionStore>, report: RestoreReport },
}
impl ArchiveOutput {
    pub fn report(&self) -> Result<serde_json::Value> {
        let value = match self {
            Self::Inventory(v) => serde_json::to_value(v),
            Self::Verified { report, .. } => serde_json::to_value(report),
            Self::Exported(v) => Ok(
                serde_json::json!({"stage":"exported-internally-audited", "manifest_commitment":v.commitment()?, "sessions":v.sessions.len()}),
            ),
            Self::Packed(v) => serde_json::to_value(v),
            Self::Assembled(v) => serde_json::to_value(v),
            Self::Retained(v) => Ok(serde_json::json!({"stage":"retained", "progress":v})),
            Self::Collected(v) => Ok(serde_json::json!({"stage":"collected", "report":v})),
            Self::BackedUp(v) => {
                Ok(serde_json::json!({"stage":"backed-up", "commitment":v.commitment()?, "storage":v.storage, "files":v.files.len()}))
            }
            Self::Restored { report, .. } => Ok(serde_json::json!({"stage":"restored", "report":report})),
        };
        value.map_err(|e| BundleError::new(ErrorCode::Format, e.to_string()))
    }
}
pub enum ArchiveOutcome {
    Complete { output: Box<ArchiveOutput>, usage: ArchiveUsage },
    Paused(Box<PausedArchive>),
}
pub struct PausedArchive {
    reason: ArchivePauseReason,
    progress: ArchiveProgress,
    task: ArchiveTask,
}
impl PausedArchive {
    pub fn reason(&self) -> ArchivePauseReason {
        self.reason
    }
    pub fn progress(&self) -> &ArchiveProgress {
        &self.progress
    }
    pub fn resume(self, budget: ArchiveBudget, cancellation: &RecoveryCancellation) -> Result<ArchiveOutcome> {
        self.task.advance(budget, cancellation)
    }
}
impl fmt::Debug for PausedArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PausedArchive").field("reason", &self.reason).field("progress", &self.progress).finish_non_exhaustive()
    }
}
impl Serialize for PausedArchive {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct View<'a> {
            reason: ArchivePauseReason,
            progress: &'a ArchiveProgress,
        }
        View { reason: self.reason, progress: &self.progress }.serialize(s)
    }
}
struct Grant {
    budget: ArchiveBudget,
    cancellation: RecoveryCancellation,
    start: Instant,
    usage: ArchiveUsage,
}
struct State {
    grant: Option<Grant>,
    aborted: bool,
    published: bool,
    generation: u64,
    total: ArchiveUsage,
    stage: ArchiveStage,
    position: Option<String>,
    retention: Option<RetentionProgress>,
    collection: Option<CollectionProgress>,
}
enum Event {
    Paused(ArchivePauseReason, ArchiveProgress),
    Finished(Result<ArchiveOutput>, ArchiveUsage),
}
struct Control {
    state: Mutex<State>,
    ready: Condvar,
    events: SyncSender<Event>,
}
thread_local! { static CONTROL:RefCell<Option<Arc<Control>>> = const { RefCell::new(None) }; }

/// One pending task owns one worker thread. Dropping it aborts at the next
/// cooperative boundary and wakes a parked worker. No extra work is authorized
/// by a serialized progress report. Synchronous I/O cannot be preempted.
pub struct ArchiveTask {
    control: Arc<Control>,
    events: Mutex<Receiver<Event>>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for ArchiveTask {
    fn drop(&mut self) {
        let mut state = self.control.state.lock().expect("archive control lock");
        state.aborted = true;
        self.control.ready.notify_all();
    }
}
impl ArchiveTask {
    fn spawn(stage: ArchiveStage, run: impl FnOnce() -> Result<ArchiveOutput> + Send + 'static) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(1);
        let control = Arc::new(Control {
            state: Mutex::new(State {
                grant: None,
                aborted: false,
                published: false,
                generation: 0,
                total: ArchiveUsage::default(),
                stage,
                position: None,
                retention: None,
                collection: None,
            }),
            ready: Condvar::new(),
            events: tx,
        });
        let thread_control = control.clone();
        let worker = thread::Builder::new()
            .name("myelin-archive".into())
            .spawn(move || {
                CONTROL.with(|slot| *slot.borrow_mut() = Some(thread_control.clone()));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    checkpoint(stage, None, 0)?;
                    let output = run()?;
                    if !thread_control.state.lock().expect("archive control lock").published {
                        thread_control.charge(0, 0)?;
                    }
                    Ok(output)
                }))
                .unwrap_or_else(|_| Err(BundleError::new(ErrorCode::Io, "archive worker panicked")));
                let (result, usage) = match thread_control.usage() {
                    Ok(usage) => (result, usage),
                    Err(error) => (Err(error), ArchiveUsage::default()),
                };
                let _ = thread_control.events.send(Event::Finished(result, usage));
                CONTROL.with(|slot| *slot.borrow_mut() = None);
            })
            .map_err(|e| BundleError::new(ErrorCode::Io, e.to_string()))?;
        Ok(Self { control, events: Mutex::new(rx), worker: Some(worker) })
    }
    pub fn inventory(directory: Arc<BundleDirectory>, anchors: Vec<TrustAnchor>) -> Result<Self> {
        Self::spawn(ArchiveStage::Inventory, move || directory.verify_inventory(&anchors).map(ArchiveOutput::Inventory))
    }
    pub fn verify(directory: Arc<BundleDirectory>, anchors: Vec<TrustAnchor>) -> Result<Self> {
        Self::spawn(ArchiveStage::Inventory, move || {
            let (store, report) = BundleStore::open(directory, &anchors)?;
            Ok(ArchiveOutput::Verified { store, report })
        })
    }
    pub fn export(store: Arc<dyn SessionStore>, plan: ExportPlan, destination: PathBuf) -> Result<Self> {
        Self::spawn(ArchiveStage::Export, move || {
            crate::export::export_inner(store.as_ref(), &plan, &destination).map(ArchiveOutput::Exported)
        })
    }
    pub fn pack(plan: PackPlan, destination: PathBuf) -> Result<Self> {
        Self::spawn(ArchiveStage::Pack, move || pack_files(&plan, destination).map(ArchiveOutput::Packed))
    }
    pub fn assemble(directory: Arc<BundleDirectory>, anchors: Vec<TrustAnchor>, root: BlobId, destination: PathBuf) -> Result<Self> {
        Self::spawn(ArchiveStage::Assemble, move || {
            directory.assemble_file_package(&anchors, root, destination).map(ArchiveOutput::Assembled)
        })
    }
    /// Relocate a bounded plan using the registry configured on `store`.
    /// Each block commits independently; paused/error progress reports include
    /// completed changes. Dropping a cursor does not roll those changes back.
    pub fn retain(
        store: Arc<myelin_session_store_rocksdb::RocksSessionStore>,
        archives: Arc<BundleArchives>,
        plan: RetentionPlan,
        policy: Arc<dyn myelin_session::HandoffRetentionPolicy>,
    ) -> Result<Self> {
        plan.validate()?;
        Self::spawn(ArchiveStage::Retention, move || {
            crate::retention::retain(store, archives, plan, policy).map(ArchiveOutput::Retained)
        })
    }
    pub fn backup(
        store: Arc<myelin_session_store_rocksdb::RocksSessionStore>,
        plan: BackupPlan,
        destination: PathBuf,
    ) -> Result<Self> {
        plan.limits.validate()?;
        Self::spawn(ArchiveStage::BackupCapture, move || crate::backup::backup(store, plan, destination).map(ArchiveOutput::BackedUp))
    }
    /// Collect superseded payload files. The supplied scope must include every
    /// referencing database; this task owns their exclusive handles. A dropped
    /// cursor leaves completed unlinks in place. Re-running the same plan checks
    /// all trust/coverage/pin conditions again and resumes from gc.pending.
    pub fn collect(plan: CollectionPlan) -> Result<Self> {
        plan.validate()?;
        Self::spawn(ArchiveStage::Collection, move || crate::collection::collect(plan).map(ArchiveOutput::Collected))
    }
    pub fn restore(
        source: PathBuf,
        expected: BlobId,
        trust: Vec<TrustAnchor>,
        limits: BundleLimits,
        destination: PathBuf,
    ) -> Result<Self> {
        limits.validate()?;
        Self::spawn(ArchiveStage::Restore, move || {
            let (store, report) = crate::backup::restore(source, expected, trust, limits, destination)?;
            Ok(ArchiveOutput::Restored { store, report })
        })
    }
    pub fn advance(mut self, budget: ArchiveBudget, cancellation: &RecoveryCancellation) -> Result<ArchiveOutcome> {
        {
            let mut state = self.control.state.lock().expect("archive control lock");
            state.grant =
                Some(Grant { budget, cancellation: cancellation.clone(), start: Instant::now(), usage: ArchiveUsage::default() });
            self.control.ready.notify_all();
        }
        let event = self
            .events
            .lock()
            .expect("archive event lock")
            .recv()
            .map_err(|_| BundleError::new(ErrorCode::Io, "archive worker disconnected"))?;
        match event {
            Event::Paused(reason, progress) => Ok(ArchiveOutcome::Paused(Box::new(PausedArchive { reason, progress, task: self }))),
            Event::Finished(output, usage) => {
                if let Some(worker) = self.worker.take() {
                    worker.join().map_err(|_| BundleError::new(ErrorCode::Io, "archive worker join failed"))?;
                }
                let output = output.map_err(|mut error| {
                    error.progress = self.control.progress().ok().map(Box::new);
                    error
                })?;
                Ok(ArchiveOutcome::Complete { output: Box::new(output), usage })
            }
        }
    }
    pub(crate) fn default_step(self) -> Result<ArchiveOutput> {
        match self.advance(ArchiveBudget::default(), &RecoveryCancellation::default())? {
            ArchiveOutcome::Complete { output, .. } => Ok(*output),
            ArchiveOutcome::Paused(cursor) => Err(BundleError::paused(cursor)),
        }
    }
}
impl Control {
    fn progress(&self) -> Result<ArchiveProgress> {
        let state = self.state.lock().expect("archive control lock");
        let mut step = state.grant.as_ref().map(|g| g.usage).unwrap_or_default();
        if let Some(g) = &state.grant {
            step.elapsed_ms = u64::try_from(g.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        }
        Ok(ArchiveProgress {
            stage: state.stage,
            position: state.position.clone(),
            step,
            total: state.total.add(step)?,
            next_operation: None,
            retention: state.retention.clone(),
            collection: state.collection.clone(),
        })
    }
    fn usage(&self) -> Result<ArchiveUsage> {
        let state = self.state.lock().expect("archive control lock");
        let mut usage = state.grant.as_ref().map(|g| g.usage).unwrap_or_default();
        if let Some(g) = &state.grant {
            usage.elapsed_ms = u64::try_from(g.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        }
        state.total.add(usage)
    }
    fn charge(&self, units: u64, bytes: u64) -> Result<()> {
        let mut state = self.state.lock().expect("archive control lock");
        loop {
            while state.grant.is_none() && !state.aborted {
                state = self.ready.wait(state).expect("archive control wait");
            }
            if state.aborted {
                return Err(BundleError::new(ErrorCode::Cancelled, "archive task dropped"));
            }
            let g = state.grant.as_mut().expect("grant supplied above");
            let reason = if g.cancellation.is_cancelled() {
                Some(ArchivePauseReason::Cancelled)
            } else if g.start.elapsed().as_millis() >= u128::from(g.budget.max_elapsed_ms) {
                Some(ArchivePauseReason::Deadline)
            } else if units > g.budget.max_steps.saturating_sub(g.usage.steps) {
                Some(ArchivePauseReason::Steps)
            } else if bytes > g.budget.max_io_bytes.saturating_sub(g.usage.io_bytes) {
                Some(ArchivePauseReason::Bytes)
            } else {
                None
            };
            if let Some(reason) = reason {
                let mut step = g.usage;
                step.elapsed_ms = u64::try_from(g.start.elapsed().as_millis()).unwrap_or(u64::MAX);
                state.total = state.total.add(step)?;
                state.generation = state
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| BundleError::new(ErrorCode::Limit, "archive generation overflow"))?;
                let progress = ArchiveProgress {
                    stage: state.stage,
                    position: state.position.clone(),
                    step,
                    total: state.total,
                    next_operation: Some(ArchiveUsage { steps: units, io_bytes: bytes, elapsed_ms: 0 }),
                    retention: state.retention.clone(),
                    collection: state.collection.clone(),
                };
                state.grant = None;
                drop(state);
                self.events
                    .send(Event::Paused(reason, progress))
                    .map_err(|_| BundleError::new(ErrorCode::Cancelled, "archive task receiver dropped"))?;
                state = self.state.lock().expect("archive control lock");
            } else {
                g.usage.steps += units;
                g.usage.io_bytes += bytes;
                return Ok(());
            }
        }
    }
}
pub(crate) fn active() -> bool {
    CONTROL.with(|s| s.borrow().is_some())
}
pub(crate) fn retention_progress(progress: RetentionProgress) {
    CONTROL.with(|s| {
        if let Some(c) = s.borrow().as_ref() {
            c.state.lock().expect("archive control lock").retention = Some(progress);
        }
    });
}
#[cfg(target_os = "linux")]
pub(crate) fn collection_progress(progress: CollectionProgress) {
    CONTROL.with(|s| {
        if let Some(c) = s.borrow().as_ref() {
            c.state.lock().expect("archive control lock").collection = Some(progress);
        }
    });
}
pub(crate) fn checkpoint(stage: ArchiveStage, position: Option<String>, bytes: u64) -> Result<()> {
    CONTROL
        .with(|s| s.borrow().clone())
        .map(|control| {
            {
                let mut state = control.state.lock().expect("archive control lock");
                state.stage = stage;
                state.position = position;
            }
            control.charge(1, bytes)
        })
        .unwrap_or(Ok(()))
}
pub(crate) fn io(bytes: u64) -> Result<()> {
    CONTROL.with(|s| s.borrow().clone()).map(|c| c.charge(1, bytes)).unwrap_or(Ok(()))
}
pub(crate) fn record_read() -> Result<()> {
    io(myelin_session::bounded_codec::MAX_RECORD_BYTES as u64)
}
/// Reserve a complete mutable-source publication check before reading any part
/// of it. A smaller grant parks without rescanning a prefix indefinitely.
pub(crate) fn reserve_publication_reads(records: u64) -> Result<()> {
    let bytes = records
        .checked_mul(myelin_session::bounded_codec::MAX_RECORD_BYTES as u64)
        .ok_or_else(|| BundleError::new(ErrorCode::Limit, "publication read reservation overflow"))?;
    CONTROL.with(|s| s.borrow().clone()).map(|c| c.charge(records, bytes)).unwrap_or(Ok(()))
}
/// A prepaid publication check can still stop between opaque reads when its
/// deadline/cancellation changes. Do not park inside it: the caller must restart
/// the whole check and reserve its reads under the new grant after resumption.
pub(crate) fn publication_check_active() -> Result<bool> {
    CONTROL.with(|s| {
        let Some(control) = s.borrow().clone() else { return Ok(true) };
        let state = control.state.lock().expect("archive control lock");
        if state.aborted {
            return Err(BundleError::new(ErrorCode::Cancelled, "archive task dropped"));
        }
        let grant = state.grant.as_ref().expect("publication reads were reserved");
        Ok(!grant.cancellation.is_cancelled() && grant.start.elapsed().as_millis() < u128::from(grant.budget.max_elapsed_ms))
    })
}
pub(crate) fn generation() -> u64 {
    CONTROL.with(|s| s.borrow().as_ref().map(|c| c.state.lock().expect("archive control lock").generation).unwrap_or(0))
}
/// Validate the final cancellation/deadline boundary before publication. A
/// caller must redo mutable-source checks if this parks and changes generation.
pub(crate) fn before_publish() -> Result<()> {
    CONTROL.with(|s| s.borrow().clone()).map(|c| c.charge(0, 0)).unwrap_or(Ok(()))
}
/// Publication is the operation's linearization point. No later pause can
/// claim that a completed manifest is still awaiting authorization to publish.
pub(crate) fn published() {
    CONTROL.with(|s| {
        if let Some(c) = s.borrow().as_ref() {
            c.state.lock().expect("archive control lock").published = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_a_parked_cursor_releases_its_worker_and_owned_inputs() {
        struct Input(mpsc::Sender<()>);
        impl Drop for Input {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (tx, rx) = mpsc::channel();
        let input = Input(tx);
        let task = ArchiveTask::spawn(ArchiveStage::Pack, move || {
            let _input = input;
            checkpoint(ArchiveStage::Pack, None, 0)?;
            panic!("dropping the paused cursor must abort before this point");
        })
        .unwrap();
        let ArchiveOutcome::Paused(cursor) =
            task.advance(ArchiveBudget { max_steps: 1, ..ArchiveBudget::default() }, &RecoveryCancellation::default()).unwrap()
        else {
            panic!("task must park");
        };
        assert!(rx.try_recv().is_err());
        drop(cursor);
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("parked worker leaked its input");
    }
}
