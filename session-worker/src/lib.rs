//! Native execution in one bounded, terminable worker process per request.
//!
//! Linux enforces address-space and CPU limits, prohibits descendant processes
//! and threads, and reports kernel peak RSS. This protects against runaway
//! trusted application code; it is not a filesystem/network sandbox for unknown
//! binaries. Operational failures do not alter deterministic consensus validity.

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod linux;
pub mod wire;

use myelin_session::{
    ApplicationProfile, CommitResourceLimits, ExecutionOutcome, FrameInput, Hash32, SessionHandoff, TransitionExecutor,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct OperationalLimits {
    pub wall_time: Duration,
    /// Kernel RLIMIT_CPU, in whole seconds. Hard limit is one second later.
    pub cpu_seconds: u64,
    /// RLIMIT_AS caps virtual address space; peak RSS is measured separately.
    pub address_space_bytes: u64,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub stderr_bytes: usize,
}

impl Default for OperationalLimits {
    fn default() -> Self {
        Self {
            wall_time: Duration::from_secs(5),
            cpu_seconds: 2,
            address_space_bytes: 256 * 1024 * 1024,
            input_bytes: wire::MAX_PACKET_BYTES,
            output_bytes: wire::MAX_PACKET_BYTES,
            stderr_bytes: 4096,
        }
    }
}

impl OperationalLimits {
    pub fn validate(&self) -> Result<(), WorkerError> {
        if self.wall_time.is_zero()
            || self.wall_time > Duration::from_secs(3600)
            || self.cpu_seconds == 0
            || self.cpu_seconds > 3600
            || self.address_space_bytes == 0
            || self.input_bytes < 44
            || self.output_bytes < 44
            || self.input_bytes > wire::MAX_PACKET_BYTES
            || self.output_bytes > wire::MAX_PACKET_BYTES
            || self.stderr_bytes > 1024 * 1024
        {
            return Err(WorkerError::new(FailureKind::Configuration, "invalid operational worker limits"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct WorkerSettings {
    pub profile: ApplicationProfile,
    pub commit_limits: CommitResourceLimits,
    pub operational: OperationalLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Configuration,
    UnsupportedPlatform,
    Artifact,
    Spawn,
    Transport,
    Deadline,
    Cancelled,
    OutputLimit,
    CpuLimit,
    ProcessExit,
    Protocol,
    Application,
}

#[derive(Clone, Debug)]
pub struct RunMeasurements {
    pub pid: u32,
    pub wall_time_us: u64,
    pub user_cpu_us: u64,
    pub system_cpu_us: u64,
    /// Kernel lifetime high-water mark, including the pre-exec fork image.
    pub lifetime_peak_rss_bytes: u64,
    /// Maximum post-exec VmHWM observed by the parent. A sampled lower bound;
    /// None means the child exited before a live measurement was available.
    pub observed_post_exec_peak_rss_bytes: Option<u64>,
    pub rss_samples: u64,
}

#[derive(Clone, Debug)]
pub struct RunReport {
    pub artifact_hash: Hash32,
    pub request_hash: Hash32,
    pub limits: OperationalLimits,
    pub measurements: RunMeasurements,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub failure: Option<FailureKind>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
#[error("worker {kind:?}: {message}")]
pub struct WorkerError {
    pub kind: FailureKind,
    pub message: String,
    pub report: Option<Box<RunReport>>,
}

impl WorkerError {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), report: None }
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    /// Permanent cancellation. Create a new executor for a new cancellation scope.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Bounded telemetry shared with isolated candidate clones, including failures.
#[derive(Clone, Default)]
pub struct ExecutionMonitor(Arc<Mutex<Option<RunReport>>>);
impl ExecutionMonitor {
    pub fn last_report(&self) -> Option<RunReport> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    fn record(&self, report: RunReport) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(report);
    }
}

#[derive(Clone)]
pub struct WorkerProgram {
    #[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
    executable: Arc<linux::Executable>,
    args: Arc<Vec<String>>,
    digest: Hash32,
}

impl WorkerProgram {
    /// Pin a local ELF by digest and copy it into a sealed executable memfd.
    /// Replacing or editing the source file cannot change subsequent executions.
    pub fn load(path: impl AsRef<Path>, expected_digest: Hash32, args: Vec<String>) -> Result<Self, WorkerError> {
        if expected_digest == [0; 32] || args.len() > 64 || args.iter().any(|a| a.len() > 4096 || a.contains('\0')) {
            return Err(WorkerError::new(FailureKind::Configuration, "invalid executable identity or arguments"));
        }
        #[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Ok(Self {
                executable: Arc::new(linux::Executable::load(path.as_ref(), expected_digest)?),
                args: Arc::new(args),
                digest: expected_digest,
            })
        }
        #[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
        {
            let _ = path;
            Err(WorkerError::new(
                FailureKind::UnsupportedPlatform,
                "native worker requires supported Linux x86_64/aarch64 enforcement",
            ))
        }
    }
    pub fn digest(&self) -> Hash32 {
        self.digest
    }
}

#[derive(Clone)]
pub struct ProcessExecutor {
    program: WorkerProgram,
    settings: Arc<WorkerSettings>,
    snapshot: Arc<[u8]>,
    root: Hash32,
    cancellation: Cancellation,
    monitor: ExecutionMonitor,
}

impl ProcessExecutor {
    /// Restore in the bounded child before accepting the initial state root.
    pub fn restore(
        program: WorkerProgram,
        settings: WorkerSettings,
        snapshot: Vec<u8>,
        expected_root: Hash32,
    ) -> Result<Self, WorkerError> {
        settings.operational.validate()?;
        settings
            .profile
            .validate()
            .and_then(|_| settings.commit_limits.validate())
            .and_then(|_| settings.commit_limits.snapshot(&snapshot))
            .map_err(|e| WorkerError::new(FailureKind::Configuration, e.to_string()))?;
        let executor = Self {
            program,
            settings: Arc::new(settings),
            snapshot: snapshot.into(),
            root: expected_root,
            cancellation: Cancellation::default(),
            monitor: ExecutionMonitor::default(),
        };
        executor.invoke(wire::OperationRef::Restore)?;
        Ok(executor)
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }
    pub fn monitor(&self) -> ExecutionMonitor {
        self.monitor.clone()
    }

    /// Typed execution errors retain this invocation's report even when
    /// multiple candidate clones publish to the shared last-report monitor.
    pub fn execute_frame(
        &mut self,
        height: u64,
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, WorkerError> {
        let state = self.invoke(wire::OperationRef::Frame { height, input, handoffs, transactions })?;
        let outcome = state.outcome.expect("execute reply validated by invoke");
        self.snapshot = state.snapshot.into();
        self.root = state.state_root;
        Ok(outcome)
    }

    fn invoke(&self, operation: wire::OperationRef<'_>) -> Result<wire::StateResult, WorkerError> {
        if self.cancellation.is_cancelled() {
            return Err(WorkerError::new(FailureKind::Cancelled, "cancelled before spawn"));
        }
        let request =
            wire::RequestRef { profile: self.settings.profile.commitment(), pre_root: self.root, snapshot: &self.snapshot, operation };
        let packet = wire::encode(&request, self.settings.operational.input_bytes)
            .map_err(|e| WorkerError::new(FailureKind::Configuration, e))?;
        #[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
        let result = linux::run(&self.program, &packet, &self.settings.operational, &self.cancellation);
        #[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
        let result: Result<(Vec<u8>, RunReport), WorkerError> = {
            let _ = (&self.program.args, &packet);
            Err(WorkerError::new(
                FailureKind::UnsupportedPlatform,
                "native worker requires supported Linux x86_64/aarch64 enforcement",
            ))
        };
        let (output, mut report) = result.inspect_err(|error| {
            if let Some(report) = &error.report {
                self.monitor.record(*report.clone());
            }
        })?;
        let validate = || {
            let protocol = |message: String| WorkerError::new(FailureKind::Protocol, message);
            let reply: wire::Reply = wire::decode(&output, self.settings.operational.output_bytes).map_err(protocol)?;
            if reply.request_hash != *blake3::hash(&packet).as_bytes() {
                return Err(protocol("worker reply belongs to another request".into()));
            }
            let state = reply.result.map_err(|e| WorkerError::new(FailureKind::Application, e.message))?;
            if state.pre_root != self.root || state.state_root == [0; 32] || state.snapshot.is_empty() {
                return Err(protocol("worker reply state identity is invalid".into()));
            }
            if matches!(request.operation, wire::OperationRef::Restore) {
                if state.outcome.is_some() || state.state_root != self.root || state.snapshot.as_slice() != self.snapshot.as_ref() {
                    return Err(protocol("worker changed the snapshot or root during restore".into()));
                }
            } else if state.outcome.is_none() {
                return Err(protocol("execute reply has no outcome".into()));
            }
            self.settings.commit_limits.snapshot(&state.snapshot).map_err(|e| protocol(e.to_string()))?;
            Ok(state)
        };
        match validate() {
            Ok(state) => {
                self.monitor.record(report);
                Ok(state)
            }
            Err(mut error) => {
                report.failure = Some(error.kind);
                self.monitor.record(report.clone());
                error.report = Some(Box::new(report));
                Err(error)
            }
        }
    }
}

impl TransitionExecutor for ProcessExecutor {
    fn state_root(&self) -> Hash32 {
        self.root
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        Ok(self.snapshot.to_vec())
    }
    fn execute_block_with_lifecycle(
        &mut self,
        context: &myelin_session::ProtocolEffectsContext,
        actions: &[myelin_session::ResolvedHandoffAction],
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        context.validate().map_err(|e| e.to_string())?;
        if context.pre_state_root != self.root || context.input_root != input.root() {
            return Err("lifecycle context does not match execution".into());
        }
        let state =
            self.invoke(wire::OperationRef::Block { context, actions, input, handoffs, transactions }).map_err(|e| e.to_string())?;
        let outcome = state.outcome.expect("execute reply validated by invoke");
        self.snapshot = state.snapshot.into();
        self.root = state.state_root;
        Ok(outcome)
    }
    fn execute_block_with_context(
        &mut self,
        context: &myelin_session::ProtocolEffectsContext,
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        self.execute_block_with_lifecycle(context, &[], input, handoffs, transactions)
    }
    fn execute_block(
        &mut self,
        height: u64,
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        self.execute_frame(height, input, handoffs, transactions).map_err(|e| e.to_string())
    }
}
