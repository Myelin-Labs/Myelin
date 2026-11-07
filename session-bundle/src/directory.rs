use crate::{Blob, BlobId, BundleError, BundleLimits, BundleManifest, ErrorCode, InventorySummary, Result, TrustAnchor};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

/// Pinned directory handles, not a registry of executable native plugins.
pub struct BundleDirectory {
    // Retain the shared lock for every derived reader, including parked tasks.
    root: File,
    blobs: File,
    manifest: BundleManifest,
    limits: BundleLimits,
    pub(crate) remote_imports: Option<std::sync::Arc<myelin_session::HandoffImportRegistry>>,
}

/// A completed inventory pass makes no statement about execution or finality.
/// Later stages must earn their own reports, never deserialize a bundle's flags.
#[derive(Clone, Debug, Serialize)]
pub struct InventoryReport {
    pub inventory: InventorySummary,
    /// Typed package bytes and assembled-file digests were checked. This does
    /// not mean their build recipe was executed or reproduced.
    pub file_packages: Vec<crate::FilePackageReport>,
    pub externally_recognized_sessions: usize,
    pub host_replay: VerificationStage,
    pub ckb_vm: VerificationStage,
    pub finality: VerificationStage,
    pub data_availability: VerificationStage,
    pub ckb_inclusion: VerificationStage,
    pub court_adjudication: VerificationStage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationStage {
    NotRun,
    Verified,
}

fn io_error(error: std::io::Error, name: &str) -> BundleError {
    let code = if error.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::MissingArtifact
    } else if matches!(error.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)) {
        ErrorCode::UnsafePath
    } else {
        ErrorCode::Io
    };
    BundleError::new(code, format!("{name}: {error}"))
}

#[cfg(unix)]
pub(crate) fn open_root(path: &Path) -> Result<File> {
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| io_error(e, "bundle root"))
}
#[cfg(not(unix))]
pub(crate) fn open_root(_path: &Path) -> Result<File> {
    Err(BundleError::new(ErrorCode::UnsupportedPlatform, "bundle directory pinning requires Unix openat"))
}

#[cfg(unix)]
pub(crate) fn open_child(parent: &File, name: &str, directory: bool) -> Result<File> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };
    if name.contains('/') || name == "." || name == ".." {
        return Err(BundleError::new(ErrorCode::UnsafePath, "invalid bundle member name"));
    }
    let name_c = CString::new(name).map_err(|_| BundleError::new(ErrorCode::UnsafePath, "NUL member name"))?;
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: parent is a live directory descriptor, name is a single NUL-
    // terminated component, and the returned descriptor gains one File owner.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name_c.as_ptr(), flags) };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error(), name));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata().map_err(|e| io_error(e, name))?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(BundleError::new(ErrorCode::UnsafePath, "bundle member is not a regular file/directory"));
    }
    Ok(file)
}
#[cfg(not(unix))]
pub(crate) fn open_child(_parent: &File, _name: &str, _directory: bool) -> Result<File> {
    Err(BundleError::new(ErrorCode::UnsupportedPlatform, "bundle directory pinning requires Unix openat"))
}

pub(crate) fn unchanged(file: &File, before: &std::fs::Metadata) -> Result<()> {
    let after = file.metadata().map_err(|e| io_error(e, "source metadata"))?;
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        before.len() == after.len()
            && before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    };
    #[cfg(not(unix))]
    let same = before.len() == after.len()
        && before.modified().map_err(|e| io_error(e, "source metadata"))?
            == after.modified().map_err(|e| io_error(e, "source metadata"))?;
    if !same {
        return Err(BundleError::new(ErrorCode::SourceChanged, "source changed during streaming read"));
    }
    Ok(())
}
fn bounded_read(mut file: File, cap: u64, name: &str) -> Result<Vec<u8>> {
    let metadata = file.metadata().map_err(|e| io_error(e, name))?;
    if metadata.len() > cap {
        return Err(BundleError::new(ErrorCode::Limit, format!("{name} exceeds byte limit")));
    }
    crate::work::io(cap + 1)?;
    let mut bytes = vec![];
    (&mut file).take(cap + 1).read_to_end(&mut bytes).map_err(|e| io_error(e, name))?;
    if bytes.len() as u64 > cap {
        return Err(BundleError::new(ErrorCode::Limit, format!("{name} grew beyond byte limit")));
    }
    unchanged(&file, &metadata)?;
    Ok(bytes)
}

/// Nonblocking advisory locks coordinate archive readers and collection. Archive
/// directories must be immutable outside these APIs; this is not a filesystem ACL.
#[cfg(unix)]
pub(crate) fn lock_directory(root: &File, exclusive: bool) -> Result<()> {
    use std::os::fd::AsRawFd;
    let operation = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH } | libc::LOCK_NB;
    // SAFETY: root owns a live descriptor and flock does not retain a pointer.
    if unsafe { libc::flock(root.as_raw_fd(), operation) } != 0 {
        let error = std::io::Error::last_os_error();
        let code = if error.kind() == std::io::ErrorKind::WouldBlock { ErrorCode::SourceChanged } else { ErrorCode::Io };
        return Err(BundleError::new(code, format!("archive is busy or cannot be locked: {error}")));
    }
    Ok(())
}
#[cfg(not(unix))]
pub(crate) fn lock_directory(_root: &File, _exclusive: bool) -> Result<()> {
    Err(BundleError::new(ErrorCode::UnsupportedPlatform, "archive locking requires Unix flock"))
}

impl BundleDirectory {
    pub fn open(path: impl AsRef<Path>, limits: BundleLimits) -> Result<Self> {
        Self::open_named(path.as_ref(), limits, "manifest.json")
    }
    pub(crate) fn open_named(path: &Path, limits: BundleLimits, manifest_name: &str) -> Result<Self> {
        limits.validate()?;
        let root = open_root(path)?;
        lock_directory(&root, false)?;
        Self::from_locked_root(root, limits, manifest_name, false)
    }
    pub(crate) fn from_locked_root(root: File, limits: BundleLimits, manifest_name: &str, exclusive: bool) -> Result<Self> {
        let bytes = bounded_read(open_child(&root, manifest_name, false)?, limits.manifest_bytes, manifest_name)?;
        let manifest = BundleManifest::decode(&bytes, &limits)?;
        let blobs = open_child(&root, "blobs", true)?;
        // A separate mount can expose this payload directory under another
        // archive root. Its readers must also exclude collection.
        lock_directory(&blobs, exclusive)?;
        Ok(Self { root, blobs, manifest, limits, remote_imports: None })
    }
    pub fn with_handoff_imports(mut self, registry: std::sync::Arc<myelin_session::HandoffImportRegistry>) -> Self {
        self.remote_imports = Some(registry);
        self
    }
    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }
    pub fn limits(&self) -> &BundleLimits {
        &self.limits
    }
    pub(crate) fn duplicate(&self) -> Result<std::sync::Arc<Self>> {
        Ok(std::sync::Arc::new(Self {
            root: self.root.try_clone().map_err(|e| io_error(e, "bundle root handle"))?,
            blobs: self.blobs.try_clone().map_err(|e| io_error(e, "bundle handle"))?,
            manifest: self.manifest.clone(),
            limits: self.limits.clone(),
            remote_imports: self.remote_imports.clone(),
        }))
    }
    /// Stream a checked copy into caller-owned staging. A failed digest check
    /// can leave partial output; publish it only after this method succeeds.
    pub fn copy_blob(&self, id: BlobId, output: &mut impl Write) -> Result<()> {
        let descriptor = self.descriptor(id)?;
        let mut input = open_child(&self.blobs, &id.hex(), false)?;
        let metadata = input.metadata().map_err(|e| io_error(e, "copy source"))?;
        if metadata.len() != descriptor.bytes {
            return Err(BundleError::new(ErrorCode::Integrity, "copy source length changed"));
        }
        let mut hash = blake3::Hasher::new();
        let mut buffer = [0u8; 65536];
        let mut remaining = descriptor.bytes;
        while remaining != 0 {
            let cap = remaining.min(buffer.len() as u64) as usize;
            crate::work::io(cap as u64 * 2)?;
            let count = input.read(&mut buffer[..cap]).map_err(|e| io_error(e, "copy source"))?;
            if count == 0 {
                return Err(BundleError::new(ErrorCode::Integrity, "copy source truncated"));
            }
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count]).map_err(|e| io_error(e, "copy destination"))?;
            remaining -= count as u64;
        }
        crate::work::io(1)?;
        if input.read(&mut buffer[..1]).map_err(|e| io_error(e, "copy source"))? != 0 || hash.finalize().as_bytes() != &id.0 {
            return Err(BundleError::new(ErrorCode::Integrity, "copy source digest changed"));
        }
        unchanged(&input, &metadata)?;
        Ok(())
    }
    pub(crate) fn descriptor(&self, id: BlobId) -> Result<&Blob> {
        self.manifest
            .blobs
            .binary_search_by_key(&id, |b| b.id)
            .map(|i| &self.manifest.blobs[i])
            .map_err(|_| BundleError::new(ErrorCode::MissingDependency, format!("blob {} is not in the inventory", id.hex())))
    }
    /// Read a single artifact into owned bytes, checking it on this very read.
    /// Callers cannot reuse a previous scan as proof that mutable files are safe.
    pub fn read_blob(&self, id: BlobId, caller_limit: u64) -> Result<Vec<u8>> {
        let descriptor = self.descriptor(id)?;
        if descriptor.bytes > caller_limit.min(self.limits.blob_bytes) {
            return Err(BundleError::new(ErrorCode::Limit, "artifact exceeds caller read limit"));
        }
        let name = id.hex();
        let bytes = bounded_read(open_child(&self.blobs, &name, false)?, descriptor.bytes, &name)?;
        if bytes.len() as u64 != descriptor.bytes || BlobId::of(&bytes) != id {
            return Err(BundleError::new(ErrorCode::Integrity, format!("artifact bytes changed: {name}")));
        }
        Ok(bytes)
    }
    /// Streaming integrity pass, using 64 KiB working space per artifact.
    /// It does not execute programs, accept consensus, or upgrade DA/L1 claims.
    pub fn verify_inventory(&self, externally_supplied_anchors: &[TrustAnchor]) -> Result<InventoryReport> {
        if !crate::work::active() {
            let crate::ArchiveOutput::Inventory(report) =
                crate::ArchiveTask::inventory(self.duplicate()?, externally_supplied_anchors.to_vec())?.default_step()?
            else {
                unreachable!("inventory task result")
            };
            return Ok(report);
        }
        crate::work::checkpoint(crate::ArchiveStage::Inventory, None, 0)?;
        let summary = self.manifest.validate_structure(&self.limits)?;
        self.manifest.recognize(externally_supplied_anchors)?;
        for descriptor in &self.manifest.blobs {
            let name = descriptor.id.hex();
            crate::work::checkpoint(crate::ArchiveStage::Inventory, Some(name.clone()), 0)?;
            let mut file = open_child(&self.blobs, &name, false)?;
            let metadata = file.metadata().map_err(|e| io_error(e, &name))?;
            if metadata.len() != descriptor.bytes {
                return Err(BundleError::new(ErrorCode::Integrity, format!("artifact length changed: {name}")));
            }
            let mut hash = blake3::Hasher::new();
            let mut buffer = [0u8; 65536];
            let mut remaining = descriptor.bytes;
            while remaining != 0 {
                let limit = (remaining as usize).min(buffer.len());
                crate::work::io(limit as u64)?;
                let n = file.read(&mut buffer[..limit]).map_err(|e| io_error(e, &name))?;
                if n == 0 {
                    return Err(BundleError::new(ErrorCode::Integrity, format!("truncated artifact: {name}")));
                }
                hash.update(&buffer[..n]);
                remaining -= n as u64;
            }
            crate::work::io(1)?;
            if file.read(&mut buffer[..1]).map_err(|e| io_error(e, &name))? != 0 || hash.finalize().as_bytes() != &descriptor.id.0 {
                return Err(BundleError::new(ErrorCode::Integrity, format!("artifact digest changed: {name}")));
            }
            unchanged(&file, &metadata)?;
        }
        // Recognition must bind the actual genesis bytes, not merely fields an
        // attacker can copy from a public allowlist into a different inventory.
        for session in &self.manifest.sessions {
            let bytes = self.read_blob(session.genesis, myelin_session::bounded_codec::MAX_RECORD_BYTES as u64)?;
            let genesis = myelin_session::SessionGenesis::decode(&bytes)
                .map_err(|e| BundleError::new(ErrorCode::Integrity, format!("genesis: {e}")))?;
            if genesis.config.session_id != session.session_id
                || genesis.config.commitment() != session.genesis_commitment
                || genesis.config.application_profile != session.profile
            {
                return Err(BundleError::new(ErrorCode::Integrity, "genesis bytes disagree with the recognized session/profile"));
            }
            let version = u16::from_le_bytes([bytes[4], bytes[5]]);
            if !self.manifest.record_versions.contains(&version) {
                return Err(BundleError::new(ErrorCode::Format, "genesis record version absent from manifest"));
            }
        }
        Ok(InventoryReport {
            inventory: summary,
            file_packages: self.verify_file_packages()?,
            externally_recognized_sessions: self
                .manifest
                .sessions
                .iter()
                .map(|s| s.session_id)
                .chain(self.manifest.sessions.iter().flat_map(|s| {
                    s.remote_handoffs
                        .iter()
                        .map(|r| r.source.session_id)
                        .chain(s.remote_completions.iter().map(|r| r.target.session_id))
                }))
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            host_replay: VerificationStage::NotRun,
            ckb_vm: VerificationStage::NotRun,
            finality: VerificationStage::NotRun,
            data_availability: VerificationStage::NotRun,
            ckb_inclusion: VerificationStage::NotRun,
            court_adjudication: VerificationStage::NotRun,
        })
    }
}
