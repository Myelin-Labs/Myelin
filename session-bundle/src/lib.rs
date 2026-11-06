//! Content-addressed evidence export and read-only historical auditing.
//! Integrity, external recognition, finality and application replay remain
//! separate claims. This module never loads archive-supplied native code.
mod backup;
mod catalog;
mod collection;
mod directory;
mod export;
mod manifest;
mod package;
mod retention;
pub mod runner;
mod store;
mod work;

pub use backup::{BackupFile, BackupManifest, BackupPlan, RestoreReport};
pub use catalog::ArchiveCatalog;
pub use collection::{CollectionPlan, CollectionProgress, CollectionReport};
pub use directory::{BundleDirectory, InventoryReport, VerificationStage};
pub use export::{export_store, ArtifactSource, ExportPlan, ExportSession};
pub use manifest::*;
pub use package::{pack_files, FileChunk, FilePackage, FilePackageReport, FileSource, PackPlan, PackageFile, PackedFiles};
pub use retention::{RetentionHeights, RetentionHolds, RetentionPlan, RetentionProgress, RetentionRange};
pub use store::{BundleArchives, BundleStore, FinalityReport, SessionAudit};
pub use work::{
    ArchiveBudget, ArchiveOutcome, ArchiveOutput, ArchivePauseReason, ArchiveProgress, ArchiveStage, ArchiveTask, ArchiveUsage,
    PausedArchive,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    Format,
    Limit,
    MissingArtifact,
    MissingDependency,
    Integrity,
    UnknownTrustAnchor,
    DependencyCycle,
    UnsafePath,
    UnsupportedPlatform,
    Io,
    InvalidHistory,
    SourceChanged,
    Sandbox,
    Deadline,
    CpuLimit,
    Cancelled,
    OutputLimit,
    ReplayMismatch,
    Paused,
}

#[derive(Debug, thiserror::Error, Serialize)]
#[error("{code:?}: {message}")]
pub struct BundleError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<Box<runner::RunReport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Box<PausedArchive>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Box<ArchiveProgress>>,
}
pub type Result<T> = std::result::Result<T, BundleError>;

impl BundleError {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), execution: None, paused: None, progress: None }
    }
    pub(crate) fn paused(cursor: Box<PausedArchive>) -> Self {
        Self {
            code: ErrorCode::Paused,
            message: "archive work paused; resume the owned cursor or use the interactive work command".into(),
            execution: None,
            paused: Some(cursor),
            progress: None,
        }
    }
}
