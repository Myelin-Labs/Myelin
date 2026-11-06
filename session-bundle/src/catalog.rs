//! Independently trusted cold catalogs shared by retention and backup.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveCatalog {
    pub path: PathBuf,
    pub commitment: BlobId,
    pub trust: Vec<TrustAnchor>,
}

pub(crate) fn register_catalogs(
    archives: &BundleArchives,
    catalogs: &[ArchiveCatalog],
    limits: &BundleLimits,
    stage: ArchiveStage,
) -> Result<CatalogBudget> {
    limits.validate()?;
    if catalogs.len() > 1024 {
        return Err(BundleError::new(ErrorCode::Limit, "too many archive catalogs"));
    }
    let mut identities = std::collections::BTreeSet::new();
    for catalog in catalogs {
        if !identities.insert(catalog.commitment) {
            return Err(BundleError::new(ErrorCode::Format, "duplicate archive catalog commitment"));
        }
    }
    let mut budget = CatalogBudget::new(limits);
    for archive in catalogs {
        crate::work::checkpoint(stage, Some(archive.commitment.hex()), 0)?;
        let directory = Arc::new(BundleDirectory::open(&archive.path, limits.clone())?);
        budget.charge(directory.manifest())?;
        if directory.manifest().commitment()? != archive.commitment.0 {
            return Err(BundleError::new(ErrorCode::Integrity, "archive catalog commitment differs from plan"));
        }
        let (verified, _) = BundleStore::open(directory, &archive.trust)?;
        archives.register(verified)?;
    }
    Ok(budget)
}

pub(crate) struct CatalogBudget {
    totals: [u64; 5],
    caps: [u64; 5],
}
impl CatalogBudget {
    pub(crate) fn new(limits: &BundleLimits) -> Self {
        Self {
            totals: [0; 5],
            caps: [limits.total_bytes, limits.nodes as u64, limits.edges as u64, limits.sessions as u64, limits.blocks as u64],
        }
    }
    pub(crate) fn charge(&mut self, manifest: &BundleManifest) -> Result<()> {
        let counts = [
            manifest.blobs.iter().map(|b| b.bytes).sum(),
            manifest.blobs.len() as u64,
            manifest.blobs.iter().map(|b| b.dependencies.len() as u64).sum(),
            manifest.sessions.len() as u64,
            manifest.sessions.iter().map(|s| s.blocks.len() as u64).sum(),
        ];
        for ((total, count), cap) in self.totals.iter_mut().zip(counts).zip(self.caps) {
            *total = total
                .checked_add(count)
                .filter(|n| *n <= cap)
                .ok_or_else(|| BundleError::new(ErrorCode::Limit, "combined archive catalogs exceed bundle limits"))?;
        }
        Ok(())
    }
}
