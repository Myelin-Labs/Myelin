//! Finite, cancellable finality/index audit. A pause never grants write rights.
//! Cursors are in-process capabilities bound to one store and captured head;
//! they cannot be deserialized as a shortcut around genesis verification.
use crate::*;
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

const RECORD_RESERVATION: u64 = codec::MAX_RECORD_BYTES as u64;

/// Local budgets for one invocation, independent of consensus validity limits.
#[derive(Clone, Debug)]
pub struct RecoveryBudget {
    pub max_blocks: u64,
    pub max_store_reads: u64,
    /// Returned canonical record bytes, including repeated dependency reads.
    /// Each variable-sized read first reserves the 128 MiB codec ceiling;
    /// unused reservation is released after measuring the returned record.
    pub max_read_bytes: u64,
    /// Checked between storage/verification operations. A synchronous backend
    /// read or finality verifier cannot be interrupted inside its own call.
    pub max_elapsed: Duration,
}
impl Default for RecoveryBudget {
    fn default() -> Self {
        Self { max_blocks: 10_000, max_store_reads: 100_000, max_read_bytes: 512 * 1024 * 1024, max_elapsed: Duration::from_secs(30) }
    }
}

#[derive(Clone, Default)]
pub struct RecoveryCancellation(Arc<AtomicBool>);
impl RecoveryCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RecoveryPauseReason {
    Blocks,
    StoreReads,
    ReadBytes,
    Deadline,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RecoveryUsage {
    pub blocks: u64,
    pub store_reads: u64,
    pub read_bytes: u64,
    pub elapsed_ms: u64,
}
impl RecoveryUsage {
    pub(crate) fn add(&mut self, other: Self) -> Result<()> {
        let sum = |a: u64, b: u64| a.checked_add(b).ok_or_else(|| SessionError::Recovery("audit usage overflow".into()));
        self.blocks = sum(self.blocks, other.blocks)?;
        self.store_reads = sum(self.store_reads, other.store_reads)?;
        self.read_bytes = sum(self.read_bytes, other.read_bytes)?;
        self.elapsed_ms = sum(self.elapsed_ms, other.elapsed_ms)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RecoveryProgress {
    pub session_id: Hash32,
    pub captured_head: Option<Hash32>,
    pub target_height: Option<u64>,
    pub next_height: u64,
    pub step: RecoveryUsage,
    pub total: RecoveryUsage,
}

pub enum RecoveryOutcome {
    Complete(Box<AuditedRecovery>),
    Paused(Box<PausedRecovery>),
}

/// A resumable cursor, not a checkpoint trust anchor or an audit receipt.
pub struct PausedRecovery {
    reason: RecoveryPauseReason,
    progress: RecoveryProgress,
    audit: RecoveryAudit,
}
impl PausedRecovery {
    pub fn reason(&self) -> RecoveryPauseReason {
        self.reason
    }
    pub fn progress(&self) -> &RecoveryProgress {
        &self.progress
    }
    pub fn resume(self, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Result<RecoveryOutcome> {
        self.audit.advance(budget, cancellation)
    }
}
impl fmt::Debug for PausedRecovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PausedRecovery").field("reason", &self.reason).field("progress", &self.progress).finish_non_exhaustive()
    }
}
impl fmt::Display for PausedRecovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} before height {}; resume this cursor to continue verification", self.reason, self.progress.next_height)
    }
}

struct AuditState {
    genesis: SessionGenesis,
    captured: SessionHead,
    audited: SessionHead,
}

/// Owns the exact store and verifier across pauses. Restarting the process
/// starts a new genesis audit; serialized counters do not establish trust.
pub struct RecoveryAudit {
    session_id: Hash32,
    expected_profile: ApplicationProfile,
    verifier: Arc<dyn FinalityVerifier>,
    store: Arc<dyn SessionStore>,
    state: Option<AuditState>,
    total: RecoveryUsage,
}
impl RecoveryAudit {
    pub fn new(
        session_id: Hash32,
        expected_profile: ApplicationProfile,
        verifier: Arc<dyn FinalityVerifier>,
        store: Arc<dyn SessionStore>,
    ) -> Self {
        Self { session_id, expected_profile, verifier, store, state: None, total: RecoveryUsage::default() }
    }

    pub fn advance(mut self, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Result<RecoveryOutcome> {
        let reads = MeteredStore::new(self.store.clone(), budget, cancellation);
        let result = self.run(&reads);
        let step = reads.usage();
        self.total.add(step)?;
        match result {
            Ok(mut audited) => {
                audited.usage = self.total;
                Ok(RecoveryOutcome::Complete(Box::new(audited)))
            }
            Err(SessionError::Store(StoreError::ReadBudget(reason))) => {
                let progress = RecoveryProgress {
                    session_id: self.session_id,
                    captured_head: self.state.as_ref().map(|s| s.captured.block_hash),
                    target_height: self.state.as_ref().and_then(|s| s.captured.finalised_height),
                    next_height: self.state.as_ref().map(|s| s.audited.next_height()).transpose()?.unwrap_or(0),
                    step,
                    total: self.total,
                };
                Ok(RecoveryOutcome::Paused(Box::new(PausedRecovery { reason, progress, audit: self })))
            }
            Err(error) => Err(error),
        }
    }

    fn run(&mut self, store: &MeteredStore) -> Result<AuditedRecovery> {
        store.check()?;
        if self.state.is_none() {
            let (genesis, captured) = initialise(store, self.session_id, &self.expected_profile, self.verifier.as_ref())?;
            self.state = Some(AuditState { audited: genesis_head(&genesis), genesis, captured });
        }
        let state = self.state.as_mut().expect("initialized above");
        if store.load_head(self.session_id)? != state.captured {
            return Err(SessionError::Recovery("captured head changed during recovery; start a new audit".into()));
        }
        while state.audited.finalised_height != state.captured.finalised_height {
            store.check()?;
            let used = store.usage.lock().expect("private recovery usage lock").blocks;
            if used >= store.budget.max_blocks {
                return Err(StoreError::ReadBudget(RecoveryPauseReason::Blocks).into());
            }
            let count = (store.budget.max_blocks - used).min(RECOVERY_PAGE_SIZE as u64) as usize;
            let page = store.load_chain_page(self.session_id, state.audited.next_height()?, count)?;
            if page.is_empty() {
                return Err(SessionError::Recovery("required history is missing before the captured head".into()));
            }
            for record in page {
                store.check()?;
                if state.captured.finalised_height.is_none_or(|h| record.block.number > h) {
                    return Err(SessionError::Recovery("history extends beyond the captured head".into()));
                }
                // An interrupted dependency check must not advance the cursor.
                let mut candidate = state.audited.clone();
                audit_record(&state.genesis, self.verifier.as_ref(), &mut candidate, &record)?;
                audit_indexes(store, &state.genesis, &record)?;
                state.audited = candidate;
                store.usage.lock().expect("private recovery usage lock").blocks += 1;
            }
        }
        if !store.load_chain_page(self.session_id, state.audited.next_height()?, 1)?.is_empty() {
            return Err(SessionError::Recovery("unexpected history after the captured head".into()));
        }
        let checkpoint = finish(store, &state.genesis, &state.audited, &state.captured)?;
        if store.load_head(self.session_id)? != state.captured {
            return Err(SessionError::Recovery("captured head changed during recovery; start a new audit".into()));
        }
        store.check()?;
        Ok(AuditedRecovery {
            config: state.genesis.config.clone(),
            head: state.captured.clone(),
            checkpoint,
            store: self.store.clone(),
            usage: RecoveryUsage::default(),
        })
    }
}

/// The adapter charges dependency reads as well as the main block scan. It
/// deliberately rejects whole-history reads and every mutation.
pub(crate) struct MeteredStore {
    inner: Arc<dyn SessionStore>,
    budget: RecoveryBudget,
    cancellation: RecoveryCancellation,
    started: Instant,
    usage: Mutex<RecoveryUsage>,
}
impl MeteredStore {
    pub(crate) fn new(inner: Arc<dyn SessionStore>, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Self {
        Self {
            inner,
            budget,
            cancellation: cancellation.clone(),
            started: Instant::now(),
            usage: Mutex::new(RecoveryUsage::default()),
        }
    }
    pub(crate) fn usage(&self) -> RecoveryUsage {
        let mut usage = *self.usage.lock().expect("private recovery usage lock");
        usage.elapsed_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        usage
    }
    pub(crate) fn check_block(&self) -> std::result::Result<(), StoreError> {
        self.check()?;
        if self.usage.lock().expect("private recovery usage lock").blocks >= self.budget.max_blocks {
            return Err(StoreError::ReadBudget(RecoveryPauseReason::Blocks));
        }
        Ok(())
    }
    pub(crate) fn completed_block(&self) {
        self.usage.lock().expect("private recovery usage lock").blocks += 1;
    }
    pub(crate) fn check(&self) -> std::result::Result<(), StoreError> {
        if self.cancellation.is_cancelled() {
            return Err(StoreError::ReadBudget(RecoveryPauseReason::Cancelled));
        }
        if self.started.elapsed() >= self.budget.max_elapsed {
            return Err(StoreError::ReadBudget(RecoveryPauseReason::Deadline));
        }
        Ok(())
    }
    fn read<T>(
        &self,
        reservation: u64,
        load: impl FnOnce() -> std::result::Result<T, StoreError>,
        size: impl FnOnce(&T) -> Result<u64>,
    ) -> std::result::Result<T, StoreError> {
        self.check()?;
        {
            let mut usage = self.usage.lock().expect("private recovery usage lock");
            if usage.store_reads >= self.budget.max_store_reads {
                return Err(StoreError::ReadBudget(RecoveryPauseReason::StoreReads));
            }
            if reservation > self.budget.max_read_bytes.saturating_sub(usage.read_bytes) {
                return Err(StoreError::ReadBudget(RecoveryPauseReason::ReadBytes));
            }
            usage.store_reads += 1;
        }
        let value = load()?;
        let bytes = size(&value).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if bytes > reservation {
            return Err(StoreError::Corrupt("backend returned data beyond the reserved read ceiling".into()));
        }
        self.usage.lock().expect("private recovery usage lock").read_bytes += bytes;
        self.check()?;
        Ok(value)
    }
}
fn encoded<T: Serialize>(value: &T) -> Result<u64> {
    record_size::record_size(value, RECORD_RESERVATION)
}
fn forbidden<T>() -> std::result::Result<T, StoreError> {
    Err(StoreError::Backend("operation is not part of bounded read-only recovery".into()))
}
impl SessionStore for MeteredStore {
    fn load_genesis(&self, id: Hash32) -> std::result::Result<SessionGenesis, StoreError> {
        self.read(
            RECORD_RESERVATION,
            || self.inner.load_genesis(id),
            |v| encoded(&(SessionConfigWire::from(&v.config), &v.state_snapshot)),
        )
    }
    fn load_head(&self, id: Hash32) -> std::result::Result<SessionHead, StoreError> {
        self.read(1024, || self.inner.load_head(id), |v| encoded(&SessionHeadWire::from(v)))
    }
    fn load_chain_page(&self, id: Hash32, start: u64, limit: usize) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
        self.read(
            RECORD_RESERVATION,
            || self.inner.load_chain_page(id, start, limit),
            |records| {
                if records.len() > limit {
                    return Err(SessionError::Recovery("backend exceeded requested page length".into()));
                }
                let mut bytes = 0;
                for record in records {
                    bytes += record.retained_size_with_limit(RECORD_RESERVATION - bytes)?;
                }
                Ok(bytes)
            },
        )
    }
    fn load_block(&self, id: Hash32, height: u64) -> std::result::Result<FinalisedBlockRecord, StoreError> {
        self.read(RECORD_RESERVATION, || self.inner.load_block(id, height), |v| v.retained_size_with_limit(RECORD_RESERVATION))
    }
    fn load_checkpoint(&self, id: Hash32) -> std::result::Result<StateCheckpoint, StoreError> {
        self.read(
            RECORD_RESERVATION,
            || self.inner.load_checkpoint(id),
            |v| encoded(&(v.session_id, v.finalised_height, v.state_root, &v.state_snapshot)),
        )
    }
    fn load_command_receipt(&self, id: Hash32, command: Hash32) -> std::result::Result<Option<CommandReceipt>, StoreError> {
        self.read(
            1024,
            || self.inner.load_command_receipt(id, command),
            |v| v.as_ref().map(encoded).transpose().map(|n| n.unwrap_or(0)),
        )
    }
    fn load_handoff(&self, id: Hash32) -> std::result::Result<Option<StoredHandoff>, StoreError> {
        self.read(RECORD_RESERVATION, || self.inner.load_handoff(id), |v| v.as_ref().map(encoded).transpose().map(|n| n.unwrap_or(0)))
    }
    fn load_checkpoint_before(&self, _: Hash32, _: u64) -> std::result::Result<StateCheckpoint, StoreError> {
        forbidden()
    }
    fn create_session(&self, _: &SessionGenesis, _: &SessionHead) -> std::result::Result<(), StoreError> {
        forbidden()
    }
    fn commit_block(&self, _: &SessionHead, _: &SessionHead, _: &FinalisedBlockRecord) -> std::result::Result<(), StoreError> {
        forbidden()
    }
    fn load_consensus_wal(&self, _: Hash32) -> std::result::Result<Option<ConsensusWal>, StoreError> {
        forbidden()
    }
    fn compare_and_set_consensus_wal(&self, _: Option<u64>, _: &ConsensusWal) -> std::result::Result<(), StoreError> {
        forbidden()
    }
    fn pending_outbox(&self, _: Hash32, _: usize) -> std::result::Result<Vec<PendingDelivery>, StoreError> {
        forbidden()
    }
    fn acknowledge_outbox(&self, _: Hash32, _: Hash32) -> std::result::Result<(), StoreError> {
        forbidden()
    }
}

fn initialise(
    store: &MeteredStore,
    session_id: Hash32,
    expected_application_profile: &ApplicationProfile,
    finality_verifier: &dyn FinalityVerifier,
) -> Result<(SessionGenesis, SessionHead)> {
    expected_application_profile.validate()?;
    let genesis = store.load_genesis(session_id)?;
    genesis.config.validate()?;
    if &genesis.config.application_profile != expected_application_profile {
        return Err(SessionError::Recovery("local application profile does not match the genesis-bound profile".to_owned()));
    }
    if let Some(predecessor) = &genesis.config.predecessor {
        let source = store.load_block(predecessor.session_id, predecessor.final_height)?;
        if source.block.state_root_after != predecessor.final_state_root
            || source.block.application_profile_commitment != predecessor.application_profile_commitment
            || source.successor.as_ref().map(|value| &value.target_config) != Some(&genesis.config)
        {
            return Err(SessionError::Recovery("successor reverse lineage does not match the final predecessor block".to_owned()));
        }
        let declaration = source.successor.as_ref().expect("checked predecessor declaration");
        let source_genesis = store.load_genesis(predecessor.session_id)?;
        declaration.validate_source(&source_genesis.config, source.frame_input.root())?;
        let source_head = store.load_head(predecessor.session_id)?;
        let checkpoint = store.load_checkpoint(predecessor.session_id)?;
        if source_head.sealed_by_successor != Some(session_id)
            || source_head.finalised_height != Some(predecessor.final_height)
            || source_head.block_hash != source.block.hash()
            || checkpoint.finalised_height != source_head.finalised_height
            || checkpoint.state_root != predecessor.final_state_root
            || genesis.state_snapshot != declaration.target_snapshot(&checkpoint.state_snapshot)?
        {
            return Err(SessionError::Recovery("successor state does not match sealed predecessor transition".into()));
        }
    }
    ensure_consensus_config(finality_verifier, &genesis.config)?;
    let persisted_head = store.load_head(session_id)?;
    Ok((genesis, persisted_head))
}

fn audit_indexes(store: &MeteredStore, genesis: &SessionGenesis, record: &FinalisedBlockRecord) -> Result<()> {
    let session_id = genesis.config.session_id;
    let mut handoff_usage = genesis
        .config
        .commit_limits
        .handoffs(record.issued_handoffs.iter().chain(record.handoff_actions.iter().map(|a| &a.handoff)))?;
    for (index, command) in record.frame_input.commands.iter().enumerate() {
        let expected = CommandReceipt::for_record(record, index)?;
        if store.load_command_receipt(session_id, command.id)?.as_ref() != Some(&expected) {
            return Err(SessionError::Recovery("command index does not match audited history".to_owned()));
        }
    }
    lifecycle::audit_actions(store, record)?;
    for issued in &record.issued_handoffs {
        let stored = store
            .load_handoff(issued.id())?
            .ok_or_else(|| SessionError::Recovery("issued handoff is absent from the durable index".to_owned()))?;
        audit_handoff_lifecycle(store, &stored)?;
        if stored.handoff != *issued {
            return Err(SessionError::Recovery("issued handoff index does not match audited history".to_owned()));
        }
    }
    for handoff_id in &record.consumed_handoff_ids {
        let stored = store
            .load_handoff(*handoff_id)?
            .ok_or_else(|| SessionError::Recovery("consumed handoff is absent from the durable index".to_owned()))?;
        {
            let limits = &genesis.config.commit_limits;
            handoff_usage.add(limits, stored.handoff.payload.len(), stored.handoff.authorization.len())?;
        }
        if stored.handoff.id() != *handoff_id
            || stored.consumption().map(|at| at.session_id) != Some(session_id)
            || stored.consumption().map(|at| at.height) != Some(record.block.number)
            || stored.consumption().map(|at| at.block_hash) != Some(record.block.hash())
            || !stored.handoff.accepts_target(&genesis.config)
            || record.block.timestamp_ms > stored.handoff.expires_at_ms
        {
            return Err(SessionError::Recovery("handoff consumption index does not match audited history".to_owned()));
        }
        let source_genesis = store.load_genesis(stored.handoff.source_session_id)?;
        validate_handoff_trust(&genesis.config, &source_genesis.config)?;
        let source = store.load_block(stored.handoff.source_session_id, stored.handoff.source_height)?;
        validate_handoff_source(&source_genesis.config, &source, &stored.handoff)?;
    }
    Ok(())
}

fn finish(
    store: &MeteredStore,
    genesis: &SessionGenesis,
    audited_head: &SessionHead,
    persisted_head: &SessionHead,
) -> Result<StateCheckpoint> {
    let session_id = genesis.config.session_id;
    if audited_head != persisted_head {
        return Err(SessionError::Recovery("persisted head does not match audited chain".to_owned()));
    }
    let successor_snapshot = if let Some(target_id) = persisted_head.sealed_by_successor {
        let final_height = persisted_head
            .finalised_height
            .ok_or_else(|| SessionError::Recovery("a genesis head cannot be successor-sealed".to_owned()))?;
        let final_record = store.load_block(session_id, final_height)?;
        let declaration = final_record
            .successor
            .as_ref()
            .ok_or_else(|| SessionError::Recovery("sealed head has no final successor declaration".to_owned()))?;
        let target_genesis = store.load_genesis(target_id)?;
        if declaration.target_config != target_genesis.config || target_genesis.state_snapshot.is_empty() {
            return Err(SessionError::Recovery("successor genesis does not match the final predecessor declaration".to_owned()));
        }
        Some((declaration.clone(), target_genesis.state_snapshot))
    } else {
        None
    };
    let checkpoint = store.load_checkpoint(session_id)?;
    {
        let limits = &genesis.config.commit_limits;
        limits.snapshot(&checkpoint.state_snapshot)?;
    }
    if checkpoint.session_id != session_id
        || checkpoint.finalised_height != persisted_head.finalised_height
        || checkpoint.state_root != persisted_head.state_root
        || checkpoint.state_snapshot.is_empty()
    {
        return Err(SessionError::Recovery("restart checkpoint does not match the audited head".to_owned()));
    }
    if let Some((declaration, snapshot)) = successor_snapshot {
        if snapshot != declaration.target_snapshot(&checkpoint.state_snapshot)? {
            return Err(SessionError::Recovery("successor genesis snapshot does not match the declared transition".into()));
        }
    }
    Ok(checkpoint)
}
