//! Incremental hot-record relocation and exact cold-manifest rebinding.
use crate::*;
use myelin_session::{HandoffRetentionPolicy, SessionStore, StoreError};
use myelin_session_store_rocksdb::{OffloadOutcome, RetentionHold, RetentionWindow, RocksSessionStore};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionHeights {
    pub first: u64,
    pub last: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionRange {
    pub session: BlobId,
    pub first: u64,
    pub last: u64,
    pub archive: BlobId,
    pub hot_from_height: u64,
    pub protected: Vec<RetentionHeights>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPlan {
    /// Each catalog is independently audited before any live record changes.
    pub archives: Vec<ArchiveCatalog>,
    /// Sorted by (session, first), with no overlapping ranges.
    pub ranges: Vec<RetentionRange>,
    pub limits: BundleLimits,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RetentionHolds {
    pub current_tip: u64,
    pub recovery_window: u64,
    pub protected_range: u64,
    pub pending_delivery: u64,
    pub outstanding_handoff: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RetentionProgress {
    pub requested: u64,
    pub processed: u64,
    pub archived: u64,
    pub rebound: u64,
    pub already_archived: u64,
    pub held: RetentionHolds,
    /// Logical hot block bytes, before database compaction; excludes pin size.
    pub hot_record_bytes_removed: u64,
    /// Permanent terminal markers replace full handoff payload/auth copies.
    pub handoffs_compacted: u64,
    pub handoff_bytes_removed: u64,
}
fn invalid(message: &str) -> BundleError {
    BundleError::new(ErrorCode::Format, message)
}
fn store_error(error: StoreError) -> BundleError {
    let code = match &error {
        StoreError::Conflict(_) => ErrorCode::SourceChanged,
        StoreError::NotFound(_) => ErrorCode::MissingDependency,
        StoreError::Corrupt(_) => ErrorCode::InvalidHistory,
        StoreError::Backend(_) => ErrorCode::Io,
        StoreError::ReadBudget(_) => ErrorCode::Limit,
    };
    BundleError::new(code, error.to_string())
}
impl RetentionPlan {
    pub(crate) fn validate(&self) -> Result<u64> {
        self.limits.validate()?;
        if self.archives.is_empty() || self.archives.len() > 1024 || self.ranges.is_empty() || self.ranges.len() > 1024 {
            return Err(invalid("retention requires 1..=1024 archives and ranges"));
        }
        let mut catalogs = std::collections::BTreeSet::new();
        for archive in &self.archives {
            if archive.path.as_os_str().is_empty() || archive.commitment.0 == [0; 32] || !catalogs.insert(archive.commitment) {
                return Err(invalid("retention archive paths/commitments must be present and commitments unique"));
            }
        }
        let mut previous = None;
        let mut requested = 0u64;
        for range in &self.ranges {
            if range.first > range.last
                || !catalogs.contains(&range.archive)
                || range.protected.len() > 256
                || range.protected.iter().any(|r| r.first > r.last)
                || previous.is_some_and(|(session, last)| range.session < session || range.session == session && range.first <= last)
            {
                return Err(invalid("retention ranges must be sorted, nonoverlapping, bounded and name a plan archive"));
            }
            previous = Some((range.session, range.last));
            requested = range
                .last
                .checked_sub(range.first)
                .and_then(|n| n.checked_add(1))
                .and_then(|n| requested.checked_add(n))
                .ok_or_else(|| invalid("retention range count overflow"))?;
        }
        requested
            .checked_mul(myelin_session::bounded_codec::MAX_RECORD_BYTES as u64)
            .ok_or_else(|| invalid("retention byte counters would overflow"))?;
        Ok(requested)
    }
}

pub(crate) fn retain(
    store: Arc<RocksSessionStore>,
    archives: Arc<BundleArchives>,
    plan: RetentionPlan,
    policy: Arc<dyn HandoffRetentionPolicy>,
) -> Result<RetentionProgress> {
    let mut progress = RetentionProgress { requested: plan.validate()?, ..RetentionProgress::default() };
    crate::work::retention_progress(progress.clone());
    crate::catalog::register_catalogs(&archives, &plan.archives, &plan.limits, ArchiveStage::Retention)?;
    let mut heads = BTreeMap::new();
    for range in &plan.ranges {
        crate::work::checkpoint(ArchiveStage::Retention, Some(format!("{}:{}", range.session.hex(), range.first)), 0)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = heads.entry(range.session.0) {
            crate::work::record_read()?;
            entry.insert(store.load_head(range.session.0).map_err(store_error)?);
        }
        let head = &heads[&range.session.0];
        if head.finalised_height.is_none_or(|height| range.last > height) {
            return Err(invalid("retention range exceeds captured source head"));
        }
        // Audited inventories are contiguous. Check both bounds up front so a
        // missing target range does not first appear after partial relocation.
        archives.reference(range.archive.0, range.session.0, range.first)?;
        archives.reference(range.archive.0, range.session.0, range.last)?;
    }
    let check = |bytes| crate::work::io(bytes).map_err(|e| StoreError::Backend(e.to_string()));
    for range in &plan.ranges {
        let window = RetentionWindow {
            hot_from_height: range.hot_from_height,
            protected: range.protected.iter().map(|r| r.first..=r.last).collect(),
        };
        for height in range.first..=range.last {
            crate::work::checkpoint(ArchiveStage::Retention, Some(format!("{}:{height}", range.session.hex())), 0)?;
            let reference = archives.reference(range.archive.0, range.session.0, height)?;
            let outcome = store
                .offload_block(&heads[&range.session.0], height, reference, &window, policy.as_ref(), &check)
                .map_err(store_error)?;
            let handoffs = match outcome {
                OffloadOutcome::Archived { hot_record_bytes_removed, handoffs } => {
                    progress.archived += 1;
                    progress.hot_record_bytes_removed += hot_record_bytes_removed;
                    handoffs
                }
                OffloadOutcome::AlreadyArchived { handoffs } => {
                    progress.already_archived += 1;
                    handoffs
                }
                OffloadOutcome::Rebound { handoffs, .. } => {
                    progress.rebound += 1;
                    handoffs
                }
                OffloadOutcome::Held(hold) => {
                    match hold {
                        RetentionHold::CurrentTip => progress.held.current_tip += 1,
                        RetentionHold::RecoveryWindow => progress.held.recovery_window += 1,
                        RetentionHold::ProtectedRange => progress.held.protected_range += 1,
                        RetentionHold::PendingDelivery => progress.held.pending_delivery += 1,
                        RetentionHold::OutstandingHandoff => progress.held.outstanding_handoff += 1,
                    }
                    Default::default()
                }
            };
            progress.handoffs_compacted += handoffs.records;
            progress.handoff_bytes_removed += handoffs.bytes_removed;
            progress.processed += 1;
            crate::work::retention_progress(progress.clone());
        }
    }
    Ok(progress)
}
