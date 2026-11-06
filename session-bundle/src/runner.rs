//! Explicit, externally recognized native replay with a restricted Linux runner.
//! The archived application remains trusted for its replay semantics; isolation
//! protects the host and cannot make a dishonest program prove valid execution.
use crate::*;
use myelin_session::{Hash32, RangeReplayReceipt, SessionStore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod linux;
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
mod linux {
    use super::*;
    pub(super) fn run(
        _: &BundleDirectory,
        _: &[TrustAnchor],
        _: &SessionInventory,
        _: &SandboxPolicy,
        _: &Cancellation,
    ) -> Result<(Vec<u8>, RunReport)> {
        Err(BundleError::new(ErrorCode::UnsupportedPlatform, "isolated replay requires Linux x86_64/aarch64 and bubblewrap"))
    }
}

pub const RUNNER_ABI: &str = "myelin-bundle-replayer-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFile {
    /// Absolute sandbox destination, never a host path. Only library trees are
    /// admitted; no symlinks, directories, sockets or arbitrary mount options.
    pub path: String,
    pub blob: BlobId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLayout {
    pub files: Vec<RuntimeFile>,
}
impl RuntimeLayout {
    pub fn validate(&self, dependencies: &[BlobId]) -> Result<()> {
        if self.files.len() > 128 || !self.files.windows(2).all(|p| p[0].path < p[1].path) {
            return Err(BundleError::new(ErrorCode::Format, "runtime files must be sorted, unique and bounded to 128"));
        }
        let mut used = BTreeSet::new();
        for file in &self.files {
            let path = file.path.as_str();
            if path.len() > 256
                || !["/lib/", "/lib64/", "/usr/lib/", "/usr/lib64/"].iter().any(|root| path.starts_with(root))
                || path[1..].split('/').any(|p| {
                    p.is_empty() || p == "." || p == ".." || !p.bytes().all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
                })
                || self.files.iter().any(|other| other.path.starts_with(&format!("{path}/")))
            {
                return Err(BundleError::new(ErrorCode::UnsafePath, "invalid or overlapping sandbox runtime destination"));
            }
            used.insert(file.blob);
        }
        if used != dependencies.iter().copied().collect() {
            return Err(BundleError::new(
                ErrorCode::MissingDependency,
                "runtime layout must name exactly the pinned runtime dependencies",
            ));
        }
        Ok(())
    }
}

/// Supplied by the local operator, never taken from an archive. The launcher is
/// trusted host infrastructure and must be a pinned bubblewrap installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPolicy {
    pub launcher: PathBuf,
    pub launcher_digest: BlobId,
    pub wall_time_ms: u64,
    pub cpu_seconds: u64,
    pub address_space_bytes: u64,
    pub output_bytes: usize,
    pub stderr_bytes: usize,
}
impl SandboxPolicy {
    pub fn validate(&self) -> Result<()> {
        if !self.launcher.is_absolute()
            || self.launcher_digest.0 == [0; 32]
            || self.wall_time_ms == 0
            || self.wall_time_ms > 3_600_000
            || self.cpu_seconds == 0
            || self.cpu_seconds > 3600
            || self.address_space_bytes < 16 * 1024 * 1024
            || self.address_space_bytes > 16 * 1024 * 1024 * 1024
            || self.output_bytes == 0
            || self.output_bytes > 4 * 1024 * 1024
            || self.stderr_bytes > 1024 * 1024
        {
            return Err(BundleError::new(ErrorCode::Limit, "invalid sandbox launcher or operational limits"));
        }
        Ok(())
    }
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RunReport {
    pub launcher_digest: BlobId,
    pub replayer_digest: BlobId,
    pub policy: SandboxPolicy,
    pub preparation_time_us: u64,
    pub wall_time_us: u64,
    /// wait4 accounting for the launcher and its reaped sandbox child. This is
    /// host process work, never CKB-VM cycles or application-only peak memory.
    pub user_cpu_us: u64,
    pub system_cpu_us: u64,
    pub lifetime_peak_rss_bytes: u64,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub failure: Option<ErrorCode>,
    pub stderr: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayOutput {
    pub abi: String,
    pub manifest_commitment: Hash32,
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub profile_commitment: Hash32,
    pub state_root_after: Hash32,
    pub receipt: Option<RangeReplayReceipt>,
}
#[derive(Clone, Debug, Serialize)]
pub struct IsolatedReplayReport {
    /// Full catalog audit. Its global host-replay stage remains not-run: only
    /// the separately named session below was executed by this invocation.
    pub audit: FinalityReport,
    pub replayed_session: Hash32,
    pub host_replay: VerificationStage,
    pub result: ReplayOutput,
    pub execution: RunReport,
}

/// Audit the catalog, copy digest-checked artifacts into private staging, and
/// run only the externally pinned replayer. Unsupported isolation fails closed.
pub fn replay_isolated(
    directory: Arc<BundleDirectory>,
    anchors: &[TrustAnchor],
    session: Hash32,
    policy: &SandboxPolicy,
    cancellation: &Cancellation,
) -> Result<IsolatedReplayReport> {
    policy.validate()?;
    let (store, audit) = BundleStore::open(directory.clone(), anchors)?;
    let inventory = directory
        .manifest()
        .sessions
        .iter()
        .find(|s| s.session_id == session)
        .ok_or_else(|| BundleError::new(ErrorCode::MissingDependency, "replay session absent"))?;
    if inventory.execution.runner_abi != RUNNER_ABI || inventory.execution.runtime.is_none() {
        return Err(BundleError::new(ErrorCode::Format, "replayer requires the isolated runner ABI and an explicit runtime layout"));
    }
    let expected = expected_output(&store, session)?;
    let (bytes, mut execution) = linux::run(&directory, anchors, inventory, policy, cancellation)?;
    let output = serde_json::from_slice::<ReplayOutput>(&bytes);
    match output {
        Ok(output) if output == expected => Ok(IsolatedReplayReport {
            audit,
            replayed_session: session,
            host_replay: VerificationStage::Verified,
            result: output,
            execution,
        }),
        _ => {
            execution.failure = Some(ErrorCode::ReplayMismatch);
            let mut error = BundleError::new(
                ErrorCode::ReplayMismatch,
                "replayer returned malformed or mismatched session/range/root/resource bindings",
            );
            error.execution = Some(Box::new(execution));
            Err(error)
        }
    }
}

// Derive only the expected output bindings from already-audited history. This
// does not execute state transitions and is never exposed as a replay result.
fn expected_output(store: &BundleStore, session: Hash32) -> Result<ReplayOutput> {
    let g = store.load_genesis(session).map_err(crate::store::integrity)?;
    let head = store.load_head(session).map_err(crate::store::integrity)?;
    let receipt = if let Some(end) = head.finalised_height {
        let first = store.load_block(session, 0).map_err(crate::store::integrity)?;
        let last = store.load_block(session, end).map_err(crate::store::integrity)?;
        let mut receipt = RangeReplayReceipt {
            session_id: session,
            application_profile_commitment: g.config.application_profile.commitment(),
            checkpoint_height: None,
            checkpoint_block_hash: [0; 32],
            checkpoint_state_root: g.config.initial_state_root,
            checkpoint_snapshot_hash: *blake3::hash(&g.state_snapshot).as_bytes(),
            start_height: 0,
            end_height: end,
            state_root_before: g.config.initial_state_root,
            state_root_after: head.state_root,
            first_frame_commitment: first.frame.commitment(),
            last_frame_commitment: last.frame.commitment(),
            frame_count: end.checked_add(1).ok_or_else(|| crate::store::integrity("range overflow"))?,
            total_cycles: 0,
            total_transaction_bytes: 0,
            total_input_bytes: 0,
        };
        for height in 0..=end {
            let r = store.load_block(session, height).map_err(crate::store::integrity)?.frame.resources;
            for (total, value) in [
                (&mut receipt.total_cycles, r.cycles),
                (&mut receipt.total_transaction_bytes, r.transaction_bytes),
                (&mut receipt.total_input_bytes, r.input_bytes),
            ] {
                *total = total.checked_add(value).ok_or_else(|| crate::store::integrity("replay resource total overflow"))?;
            }
        }
        Some(receipt)
    } else {
        None
    };
    Ok(ReplayOutput {
        abi: RUNNER_ABI.into(),
        manifest_commitment: store.manifest().commitment()?,
        session_id: session,
        genesis_commitment: g.config.commitment(),
        profile_commitment: g.config.application_profile.commitment(),
        state_root_after: head.state_root,
        receipt,
    })
}

/// Standard runner entry point for a compiled application. Inputs use fixed
/// sandbox paths; argv[1] binds the exact manifest and stdin is closed.
pub fn replayer_input() -> Result<(Arc<BundleStore>, Hash32)> {
    fn read<T: serde::de::DeserializeOwned>(path: &str, cap: u64) -> Result<T> {
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(path)
            .map_err(crate::store::integrity)?
            .take(cap + 1)
            .read_to_end(&mut bytes)
            .map_err(crate::store::integrity)?;
        if bytes.len() as u64 > cap {
            return Err(BundleError::new(ErrorCode::Limit, "runner input budget"));
        }
        serde_json::from_slice(&bytes).map_err(crate::store::integrity)
    }
    let limits: BundleLimits = read("/run/limits.json", 16 * 1024)?;
    let anchors: Vec<TrustAnchor> = read("/run/trust.json", 1024 * 1024)?;
    let session: Hash32 = read("/run/session.json", 1024)?;
    let directory = Arc::new(BundleDirectory::open("/bundle", limits)?);
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args != [BlobId(directory.manifest().commitment()?).hex()] {
        return Err(BundleError::new(ErrorCode::ReplayMismatch, "runner argv does not bind the archived manifest"));
    }
    let (store, _) = BundleStore::open(directory, &anchors)?;
    Ok((store, session))
}

pub fn replay_output(store: &Arc<BundleStore>, session: Hash32, receipt: Option<RangeReplayReceipt>) -> Result<ReplayOutput> {
    let cfg = store.config(session)?;
    Ok(ReplayOutput {
        abi: RUNNER_ABI.into(),
        manifest_commitment: store.manifest().commitment()?,
        session_id: session,
        genesis_commitment: cfg.commitment(),
        profile_commitment: cfg.application_profile.commitment(),
        state_root_after: store.load_head(session).map_err(crate::store::integrity)?.state_root,
        receipt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_destinations_cannot_replace_inputs_or_escape_library_trees() {
        let id = BlobId([1; 32]);
        for path in [
            "",
            "lib/libc.so",
            "/proc/self/fd/0",
            "/bundle/manifest.json",
            "/run/trust.json",
            "/lib/../run/trust.json",
            "/lib//libc.so",
            "/lib/./libc.so",
            "/lib/libc.so\0",
        ] {
            let layout = RuntimeLayout { files: vec![RuntimeFile { path: path.into(), blob: id }] };
            assert_eq!(layout.validate(&[id]).unwrap_err().code, ErrorCode::UnsafePath, "{path:?}");
        }
        let layout = RuntimeLayout {
            files: vec![
                RuntimeFile { path: "/lib/subtree".into(), blob: id },
                RuntimeFile { path: "/lib/subtree/libc.so".into(), blob: id },
            ],
        };
        assert_eq!(layout.validate(&[id]).unwrap_err().code, ErrorCode::UnsafePath);
        let layout = RuntimeLayout { files: vec![RuntimeFile { path: "/lib/libc.so".into(), blob: id }] };
        assert_eq!(layout.validate(&[]).unwrap_err().code, ErrorCode::MissingDependency);
        layout.validate(&[id]).unwrap();
    }
}
