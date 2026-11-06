//! Reclaim superseded archive payloads in an explicitly owned, quiescent scope.
//! A retained canonical manifest is a restart marker, never a verification receipt.
use crate::*;
use myelin_session_store_rocksdb::SnapshotInventory;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionPlan {
    /// Every database that can reference these managed archive directories,
    /// including retained database checkpoints. All must be closed by callers.
    pub databases: Vec<PathBuf>,
    /// Independently trusted replacements, retained throughout collection.
    pub archives: Vec<ArchiveCatalog>,
    pub candidates: Vec<ArchiveCatalog>,
    /// Recovery/dispute roots whose exact checkpoint must remain available.
    pub protected: Vec<BlobId>,
    pub limits: BundleLimits,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CollectionProgress {
    pub completed: u64,
    pub blobs_removed: u64,
    pub already_missing: u64,
    /// Unlinked file lengths, not physical disk savings (files may be hardlinked).
    pub unlinked_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CollectionReport {
    /// Ordered like the plan databases; no database keys are removed or rebound.
    pub storage: Vec<SnapshotInventory>,
    pub progress: CollectionProgress,
}
impl CollectionPlan {
    pub(crate) fn validate(&self) -> Result<()> {
        self.limits.validate()?;
        if self.databases.is_empty()
            || self.databases.len() > 1024
            || self.archives.is_empty()
            || self.candidates.is_empty()
            || self.archives.len() + self.candidates.len() > 1024
            || self.protected.len() > 1024
        {
            return Err(BundleError::new(ErrorCode::Limit, "collection requires bounded databases, keepers and candidates"));
        }
        let mut paths = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for catalog in self.archives.iter().chain(&self.candidates) {
            if catalog.path.as_os_str().is_empty()
                || catalog.commitment.0 == [0; 32]
                || !paths.insert(&catalog.path)
                || !identities.insert(catalog.commitment)
            {
                return Err(BundleError::new(ErrorCode::Format, "collection requires distinct archive paths and commitments"));
            }
        }
        if self.databases.iter().any(|p| p.as_os_str().is_empty() || !paths.insert(p))
            || !self.protected.windows(2).all(|p| p[0] < p[1])
            || self.protected.iter().any(|id| !identities.contains(id))
        {
            return Err(BundleError::new(ErrorCode::Format, "invalid collection database paths or protected roots"));
        }
        if self.candidates.iter().any(|c| self.protected.contains(&c.commitment)) {
            return Err(BundleError::new(ErrorCode::MissingDependency, "collection candidate is a protected recovery/dispute root"));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn collect(_plan: CollectionPlan) -> Result<CollectionReport> {
    Err(BundleError::new(ErrorCode::UnsupportedPlatform, "archive collection requires Linux pinned directory enumeration"))
}
#[cfg(target_os = "linux")]
pub(crate) use linux::collect;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::directory::{lock_directory, open_child, open_root};
    use myelin_session_store_rocksdb::{RocksSessionStore, SnapshotLimits};
    use std::{collections::BTreeMap, ffi::CString, fs::File, os::fd::AsRawFd, sync::Arc};

    const PENDING: &str = "gc.pending";
    struct Candidate {
        root: File,
        blobs: File,
        directory: Arc<BundleDirectory>,
        pending: bool,
    }
    fn io(error: impl std::fmt::Display) -> BundleError {
        BundleError::new(ErrorCode::Io, error.to_string())
    }
    fn missing(message: &str) -> BundleError {
        BundleError::new(ErrorCode::MissingDependency, message)
    }
    fn entries(directory: &File, cap: usize) -> Result<BTreeSet<String>> {
        let mut names = BTreeSet::new();
        // /proc resolves our already-pinned descriptor, never a caller's path.
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd())).map_err(io)? {
            crate::work::checkpoint(ArchiveStage::Collection, None, 0)?;
            let name = entry
                .map_err(io)?
                .file_name()
                .into_string()
                .map_err(|_| BundleError::new(ErrorCode::UnsafePath, "archive collection encountered a non-UTF8 member"))?;
            if names.len() >= cap || !names.insert(name) {
                return Err(BundleError::new(ErrorCode::UnsafePath, "unexpected archive directory members"));
            }
        }
        Ok(names)
    }
    fn open_candidate(catalog: &ArchiveCatalog, limits: &BundleLimits) -> Result<Candidate> {
        let root = open_root(&catalog.path)?;
        lock_directory(&root, true)?;
        let names = entries(&root, 2)?;
        let pending = names.contains(PENDING);
        let manifest_name = if pending { PENDING } else { "manifest.json" };
        if names != BTreeSet::from(["blobs".into(), manifest_name.into()]) {
            return Err(BundleError::new(ErrorCode::UnsafePath, "candidate must contain only its manifest and blobs directory"));
        }
        let directory =
            Arc::new(BundleDirectory::from_locked_root(root.try_clone().map_err(io)?, limits.clone(), manifest_name, true)?);
        if directory.manifest().commitment()? != catalog.commitment.0 {
            return Err(BundleError::new(ErrorCode::Integrity, "collection candidate commitment differs from plan"));
        }
        directory.manifest().recognize(&catalog.trust)?;
        let blobs = open_child(&root, "blobs", true)?;
        for name in entries(&blobs, limits.nodes)? {
            let id: BlobId = serde_json::from_value(serde_json::Value::String(name.clone()))
                .map_err(|_| BundleError::new(ErrorCode::UnsafePath, "unexpected candidate blob filename"))?;
            let descriptor = directory.descriptor(id)?;
            let file = open_child(&blobs, &name, false)?;
            if file.metadata().map_err(io)?.len() != descriptor.bytes {
                return Err(BundleError::new(ErrorCode::Integrity, "candidate blob size changed"));
            }
        }
        if !pending {
            // Before the first unlink, validate the original snapshot too. A
            // restart cannot repeat this after some payloads have been removed;
            // it instead re-establishes every keeper/pin/coverage condition.
            BundleStore::open(directory.clone(), &catalog.trust)?;
        }
        Ok(Candidate { root, blobs, directory, pending })
    }
    struct Coverage<'a> {
        sessions: BTreeMap<myelin_session::Hash32, Vec<&'a SessionInventory>>,
        blobs: BTreeMap<BlobId, Vec<&'a Blob>>,
    }
    impl<'a> Coverage<'a> {
        fn new(keepers: &'a [Arc<BundleStore>]) -> Result<Self> {
            let mut index = Self { sessions: BTreeMap::new(), blobs: BTreeMap::new() };
            for keeper in keepers {
                for session in &keeper.manifest().sessions {
                    crate::work::checkpoint(ArchiveStage::Collection, Some("index-session".into()), 0)?;
                    index.sessions.entry(session.session_id).or_default().push(session);
                }
                for blob in &keeper.manifest().blobs {
                    crate::work::checkpoint(ArchiveStage::Collection, Some(blob.id.hex()), 0)?;
                    index.blobs.entry(blob.id).or_default().push(blob);
                }
            }
            Ok(index)
        }
        fn check(&self, candidate: &BundleManifest) -> Result<()> {
            let mut snapshots = BTreeSet::new();
            for session in &candidate.sessions {
                crate::work::checkpoint(ArchiveStage::Collection, Some(BlobId(session.session_id).hex()), 0)?;
                if !self.sessions.get(&session.session_id).is_some_and(|copies| {
                    copies.iter().any(|kept| {
                        kept.proposed_anchor() == session.proposed_anchor()
                            && kept.genesis == session.genesis
                            && kept.consensus == session.consensus
                            && kept.blocks.starts_with(&session.blocks)
                    })
                }) {
                    return Err(missing("candidate history is not a prefix of a recognized keeper session"));
                }
                snapshots.extend([session.head, session.checkpoint]);
            }
            for blob in &candidate.blobs {
                crate::work::checkpoint(ArchiveStage::Collection, Some(blob.id.hex()), 0)?;
                // Only indexed operational head/checkpoint snapshots may be
                // superseded. Blocks, claims, evidence and complete executable,
                // migration and build dependency closures must remain byte-exact.
                let snapshot =
                    blob.roles.iter().all(|r| matches!(r, BlobKind::Head | BlobKind::Checkpoint)) && snapshots.contains(&blob.id);
                if !snapshot
                    && !self.blobs.get(&blob.id).is_some_and(|copies| {
                        copies.iter().any(|kept| {
                            kept.bytes == blob.bytes
                                && blob.roles.iter().all(|r| kept.roles.contains(r))
                                && blob.dependencies.iter().all(|d| kept.dependencies.binary_search(d).is_ok())
                        })
                    })
                {
                    return Err(missing("keeper lacks a candidate artifact, claim, evidence or dependency edge"));
                }
            }
            Ok(())
        }
    }
    fn begin(candidate: &mut Candidate) -> Result<()> {
        if !candidate.pending {
            // SAFETY: both names are fixed single components under a live root.
            let result = unsafe {
                libc::renameat2(
                    candidate.root.as_raw_fd(),
                    c"manifest.json".as_ptr(),
                    candidate.root.as_raw_fd(),
                    c"gc.pending".as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if result != 0 {
                return Err(io(std::io::Error::last_os_error()));
            }
            candidate.pending = true;
        }
        // Also sync an existing marker after a process died around the rename.
        candidate.root.sync_all().map_err(io)
    }
    fn unlink(directory: &File, name: &str) -> Result<()> {
        let name = CString::new(name).map_err(io)?;
        // SAFETY: the generated lowercase digest is a single path component.
        if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub(crate) fn collect(plan: CollectionPlan) -> Result<CollectionReport> {
        let mut report = CollectionReport { storage: vec![], progress: CollectionProgress::default() };
        crate::work::collection_progress(report.progress.clone());
        // Ownership stays private to this task: native RocksDB locks exclude all
        // writers until the last unlink, including while its cursor is parked.
        let mut stores = vec![];
        let mut remaining = SnapshotLimits {
            max_entries: plan.limits.edges as u64,
            max_bytes: plan.limits.total_bytes,
            max_sessions: plan.limits.sessions,
            max_archives: 1024,
        };
        for path in &plan.databases {
            crate::work::checkpoint(ArchiveStage::Collection, Some(path.display().to_string()), 0)?;
            open_child(&open_root(path)?, "CURRENT", false)?;
            let store = RocksSessionStore::open(path).map_err(io)?;
            let storage = store
                .snapshot_inventory(remaining, &|n| crate::work::io(n).map_err(|e| myelin_session::StoreError::Backend(e.to_string())))
                .map_err(io)?;
            remaining.max_entries -= storage.entries;
            remaining.max_bytes -= storage.encoded_bytes;
            remaining.max_sessions -= storage.heads.len();
            report.storage.push(storage);
            stores.push(store);
        }
        let registry = BundleArchives::default();
        let mut budget = crate::catalog::register_catalogs(&registry, &plan.archives, &plan.limits, ArchiveStage::Collection)?;
        let keepers: Vec<_> = plan.archives.iter().map(|c| registry.catalog(c.commitment.0)).collect::<Result<_>>()?;
        let coverage = Coverage::new(&keepers)?;
        let mut cold_blocks = 0usize;
        for store in &stores {
            let mut after = None;
            loop {
                crate::work::checkpoint(ArchiveStage::Collection, Some("database-pins".into()), 1024 * 128)?;
                let page = store.archived_blocks(after, 1024).map_err(io)?;
                if page.is_empty() {
                    break;
                }
                for pin in page {
                    cold_blocks += 1;
                    if cold_blocks > plan.limits.blocks {
                        return Err(BundleError::new(ErrorCode::Limit, "collection cold references exceed block limit"));
                    }
                    if registry.reference(pin.reference.archive, pin.session_id, pin.height)? != pin.reference {
                        return Err(BundleError::new(ErrorCode::Integrity, "database pin differs from retained archive"));
                    }
                    after = Some((pin.session_id, pin.height));
                }
            }
        }
        // Check the entire plan before any candidate changes. No candidate can
        // cover another candidate, and readers prevent exclusive acquisition.
        let mut candidates = vec![];
        for catalog in &plan.candidates {
            crate::work::checkpoint(ArchiveStage::Collection, Some(catalog.commitment.hex()), 0)?;
            let candidate = open_candidate(catalog, &plan.limits)?;
            budget.charge(candidate.directory.manifest())?;
            coverage.check(candidate.directory.manifest())?;
            candidates.push(candidate);
        }
        for candidate in &mut candidates {
            let commitment = BlobId(candidate.directory.manifest().commitment()?);
            crate::work::checkpoint(ArchiveStage::Collection, Some(commitment.hex()), 0)?;
            begin(candidate)?;
            for blob in &candidate.directory.manifest().blobs {
                let name = blob.id.hex();
                crate::work::checkpoint(ArchiveStage::Collection, Some(format!("{}:{name}", commitment.hex())), 0)?;
                match open_child(&candidate.blobs, &name, false) {
                    Ok(file) => {
                        if file.metadata().map_err(io)?.len() != blob.bytes {
                            return Err(BundleError::new(ErrorCode::Integrity, "candidate changed before unlink"));
                        }
                        unlink(&candidate.blobs, &name)?;
                        report.progress.blobs_removed += 1;
                        report.progress.unlinked_bytes += blob.bytes;
                    }
                    Err(error) if error.code == ErrorCode::MissingArtifact => report.progress.already_missing += 1,
                    Err(error) => return Err(error),
                }
                crate::work::collection_progress(report.progress.clone());
            }
            candidate.blobs.sync_all().map_err(io)?;
            report.progress.completed += 1;
            crate::work::collection_progress(report.progress.clone());
        }
        // Leave gc.pending and an empty blobs directory. Restart re-audits the
        // keepers and database pins; this marker cannot restore reader trust.
        Ok(report)
    }
}
