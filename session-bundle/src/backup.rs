//! Full current-schema database backups, including operational state and every
//! pinned cold catalog. Restore establishes storage identity, not writer rights.
use crate::*;
use myelin_session_store_rocksdb::{RocksSessionStore, RocksSessionStoreOptions, SnapshotInventory, SnapshotLimits};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const FORMAT: &str = "myelin-session-backup-v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupPlan {
    pub archives: Vec<ArchiveCatalog>,
    pub limits: BundleLimits,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupFile {
    pub name: String,
    pub bytes: u64,
    pub digest: BlobId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    pub format: String,
    pub storage: SnapshotInventory,
    pub options: RocksSessionStoreOptions,
    /// Local clock bounds around the native checkpoint, not authority timestamps.
    pub capture_started_ms: u64,
    pub capture_finished_ms: u64,
    pub files: Vec<BackupFile>,
}
#[derive(Clone, Debug, Serialize)]
pub struct RestoreReport {
    pub backup: BlobId,
    pub storage: SnapshotInventory,
    pub database: PathBuf,
    pub archives: Vec<PathBuf>,
    pub writer_authority: VerificationStage,
    pub application_replay: VerificationStage,
}
fn io(e: impl std::fmt::Display) -> BundleError {
    BundleError::new(ErrorCode::Io, e.to_string())
}
fn integrity(message: &str) -> BundleError {
    BundleError::new(ErrorCode::Integrity, message)
}
fn limit(message: &str) -> BundleError {
    BundleError::new(ErrorCode::Limit, message)
}
fn now() -> Result<u64> {
    u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(io)?.as_millis()).map_err(io)
}
fn snapshot_limits(limits: &BundleLimits) -> SnapshotLimits {
    SnapshotLimits {
        max_entries: limits.edges as u64,
        max_bytes: limits.total_bytes,
        max_sessions: limits.sessions,
        max_archives: 1024,
    }
}
fn inventory(store: &RocksSessionStore, limits: &BundleLimits) -> Result<SnapshotInventory> {
    store
        .snapshot_inventory(snapshot_limits(limits), &|n| {
            crate::work::io(n).map_err(|e| myelin_session::StoreError::Backend(e.to_string()))
        })
        .map_err(io)
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir(path).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io)?;
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    File::open(parent).map_err(io)?.sync_all().map_err(io)?;
    Ok(())
}
fn name_valid(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    match parts.as_slice() {
        ["db", file] => {
            !file.is_empty()
                && *file != "."
                && *file != ".."
                && file.len() <= 128
                && file.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        }
        ["archives", id, "manifest.json"] => hex(id),
        ["archives", id, "blobs", blob] => hex(id) && hex(blob),
        _ => false,
    }
}
fn nested(root: &File, name: &str) -> Result<File> {
    if !name_valid(name) {
        return Err(BundleError::new(ErrorCode::UnsafePath, "invalid backup member path"));
    }
    let parts: Vec<_> = name.split('/').collect();
    let mut parent = root.try_clone().map_err(io)?;
    for (index, part) in parts.iter().enumerate() {
        parent = crate::directory::open_child(&parent, part, index + 1 != parts.len())?;
    }
    Ok(parent)
}
impl BackupManifest {
    pub fn commitment(&self) -> Result<BlobId> {
        let mut hash = blake3::Hasher::new();
        hash.update(b"myelin:session-backup-manifest:v1");
        hash.update(&serde_json::to_vec(self).map_err(io)?);
        Ok(BlobId(*hash.finalize().as_bytes()))
    }
    fn validate(&self, limits: &BundleLimits) -> Result<()> {
        limits.validate()?;
        if self.format != FORMAT
            || self.files.is_empty()
            || self.files.len() > limits.nodes
            || self.storage.heads.len() > limits.sessions
            || self.storage.archives.len() > 1024
            || self.storage.entries > limits.edges as u64
            || self.storage.encoded_bytes > limits.total_bytes
            || !self.storage.archives.windows(2).all(|w| w[0] < w[1])
            || !self.files.windows(2).all(|w| w[0].name < w[1].name)
            || self.files.iter().any(|f| !name_valid(&f.name))
            || !self.files.iter().any(|f| f.name == "db/CURRENT")
        {
            return Err(integrity("backup format, keyspace limits or member names are invalid"));
        }
        let mut total = 0u64;
        for f in &self.files {
            total = total
                .checked_add(f.bytes)
                .filter(|n| *n <= limits.total_bytes)
                .ok_or_else(|| limit("backup files exceed byte limit"))?;
        }
        if total.checked_add(serde_json::to_vec(self).map_err(io)?.len() as u64).is_none_or(|n| n > limits.total_bytes) {
            return Err(limit("backup files plus manifest exceed byte limit"));
        }
        let manifests: BTreeSet<_> = self
            .files
            .iter()
            .filter_map(|f| f.name.strip_prefix("archives/").and_then(|p| p.strip_suffix("/manifest.json")))
            .collect();
        let expected: BTreeSet<_> = self.storage.archives.iter().map(|id| BlobId(*id).hex()).collect();
        if manifests != expected.iter().map(String::as_str).collect()
            || self
                .files
                .iter()
                .filter(|f| f.name.starts_with("archives/"))
                .any(|f| !expected.contains(f.name.split('/').nth(1).unwrap()))
        {
            return Err(integrity("backup cold manifest set does not match captured pins"));
        }
        Ok(())
    }
}
struct Copier<'a> {
    root: &'a Path,
    limits: &'a BundleLimits,
    files: Vec<BackupFile>,
    total: u64,
    stage: ArchiveStage,
}
impl Copier<'_> {
    fn add(&mut self, name: String, bytes: u64, mut copy: impl FnMut(&mut File) -> Result<BlobId>) -> Result<()> {
        if !name_valid(&name) || self.files.len() >= self.limits.nodes {
            return Err(limit("backup member name/count limit"));
        }
        self.total = self
            .total
            .checked_add(bytes)
            .filter(|n| *n <= self.limits.total_bytes)
            .ok_or_else(|| limit("backup expanded byte limit"))?;
        crate::work::checkpoint(self.stage, Some(name.clone()), 0)?;
        let path = self.root.join(&name);
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(io)?;
        let mut output = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
        let digest = copy(output.as_file_mut())?;
        output.as_file().sync_all().map_err(io)?;
        output.persist_noclobber(&path).map_err(io)?;
        let mut directory = parent;
        loop {
            crate::work::io(0)?;
            File::open(directory).map_err(io)?.sync_all().map_err(io)?;
            if directory == self.root {
                break;
            }
            directory = directory.parent().ok_or_else(|| integrity("backup directory escaped destination"))?;
        }
        self.files.push(BackupFile { name, bytes, digest });
        Ok(())
    }
    fn file(&mut self, name: String, mut input: File, expected: Option<BlobId>) -> Result<()> {
        let metadata = input.metadata().map_err(io)?;
        if !metadata.is_file() {
            return Err(BundleError::new(ErrorCode::UnsafePath, "backup member is not a regular file"));
        }
        let bytes = metadata.len();
        self.add(name, bytes, |output| {
            let mut hash = blake3::Hasher::new();
            let mut left = bytes;
            let mut buffer = [0u8; 65536];
            while left != 0 {
                let n = left.min(buffer.len() as u64) as usize;
                crate::work::io(n as u64 * 2)?;
                let n = input.read(&mut buffer[..n]).map_err(io)?;
                if n == 0 {
                    return Err(integrity("backup member truncated"));
                }
                output.write_all(&buffer[..n]).map_err(io)?;
                hash.update(&buffer[..n]);
                left -= n as u64;
            }
            crate::work::io(1)?;
            let digest = BlobId(*hash.finalize().as_bytes());
            if input.read(&mut buffer[..1]).map_err(io)? != 0 || expected.is_some_and(|v| v != digest) {
                return Err(integrity("backup member length/digest changed"));
            }
            crate::directory::unchanged(&input, &metadata)?;
            Ok(digest)
        })
    }
    fn bytes(&mut self, name: String, bytes: &[u8]) -> Result<()> {
        self.add(name, bytes.len() as u64, |out| {
            crate::work::io(bytes.len() as u64)?;
            out.write_all(bytes).map_err(io)?;
            Ok(BlobId::of(bytes))
        })
    }
}
fn validate_pins(store: &RocksSessionStore, archives: &BundleArchives, expected: u64) -> Result<()> {
    let mut after = None;
    let mut count = 0u64;
    loop {
        crate::work::record_read()?;
        let page = store.archived_blocks(after, 128).map_err(io)?;
        if page.is_empty() {
            break;
        }
        for pin in page {
            crate::work::io(0)?;
            if archives.reference(pin.reference.archive, pin.session_id, pin.height)? != pin.reference {
                return Err(integrity("captured cold pin differs from its audited catalog"));
            }
            after = Some((pin.session_id, pin.height));
            count += 1;
            if count > expected {
                return Err(integrity("cold pins changed during backup verification"));
            }
        }
    }
    if count != expected {
        return Err(integrity("cold pin count differs from snapshot"));
    }
    Ok(())
}
fn publish(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    crate::work::checkpoint(ArchiveStage::Publish, Some(name.into()), bytes.len() as u64)?;
    let mut staged = tempfile::NamedTempFile::new_in(root).map_err(io)?;
    staged.write_all(bytes).map_err(io)?;
    staged.as_file().sync_all().map_err(io)?;
    crate::work::before_publish()?;
    staged.persist_noclobber(root.join(name)).map_err(io)?;
    crate::work::published();
    File::open(root).map_err(io)?.sync_all().map_err(io)?;
    Ok(())
}
pub(crate) fn backup(store: Arc<RocksSessionStore>, plan: BackupPlan, destination: PathBuf) -> Result<BackupManifest> {
    plan.limits.validate()?;
    private_dir(&destination)?;
    let staging = tempfile::tempdir_in(&destination).map_err(io)?;
    let capture = staging.path().join("checkpoint");
    crate::work::checkpoint(ArchiveStage::BackupCapture, None, 0)?;
    let capture_started_ms = now()?;
    store.create_checkpoint(&capture).map_err(io)?;
    let capture_finished_ms = now()?;
    let options = store.storage_options();
    let captured = RocksSessionStore::open_with_options(&capture, options).map_err(io)?;
    let storage = inventory(&captured, &plan.limits)?;
    let supplied: BTreeSet<_> = plan.archives.iter().map(|a| a.commitment.0).collect();
    if supplied != storage.archives.iter().copied().collect() {
        return Err(BundleError::new(ErrorCode::MissingDependency, "backup requires exactly the captured cold catalogs"));
    }
    let archives = BundleArchives::default();
    crate::catalog::register_catalogs(&archives, &plan.archives, &plan.limits, ArchiveStage::BackupArchives)?;
    validate_pins(&captured, &archives, storage.cold_blocks)?;
    drop(captured); // Close native writers before copying its physical files.
    let mut writer = Copier { root: &destination, limits: &plan.limits, files: vec![], total: 0, stage: ArchiveStage::BackupFiles };
    let root = crate::directory::open_root(&capture)?;
    // Checkpoints contain only flat engine files. Bound the listing before sorting.
    let mut names = vec![];
    for entry in fs::read_dir(&capture).map_err(io)? {
        crate::work::io(0)?;
        let name = entry.map_err(io)?.file_name().into_string().map_err(|_| integrity("non-UTF8 checkpoint filename"))?;
        if names.len() >= plan.limits.nodes {
            return Err(limit("checkpoint file count exceeds budget"));
        }
        names.push(name);
    }
    names.sort();
    for name in names {
        writer.file(format!("db/{name}"), crate::directory::open_child(&root, &name, false)?, None)?;
    }
    for id in &storage.archives {
        let catalog = archives.catalog(*id)?;
        let prefix = format!("archives/{}", BlobId(*id).hex());
        writer.bytes(format!("{prefix}/manifest.json"), &catalog.manifest().encode()?)?;
        for blob in &catalog.manifest().blobs {
            writer.add(format!("{prefix}/blobs/{}", blob.id.hex()), blob.bytes, |out| {
                catalog.directory().copy_blob(blob.id, out)?;
                Ok(blob.id)
            })?;
        }
    }
    writer.files.sort_by(|a, b| a.name.cmp(&b.name));
    let manifest =
        BackupManifest { format: FORMAT.into(), storage, options, capture_started_ms, capture_finished_ms, files: writer.files };
    manifest.validate(&plan.limits)?;
    let bytes = serde_json::to_vec(&manifest).map_err(io)?;
    if bytes.len() as u64 > plan.limits.manifest_bytes {
        return Err(limit("backup manifest exceeds limit"));
    }
    // Drop the temporary capture before publishing: no path in the manifest refers to it.
    drop(root);
    staging.close().map_err(io)?;
    publish(&destination, "backup.json", &bytes)?;
    Ok(manifest)
}
pub(crate) fn restore(
    source: PathBuf,
    expected: BlobId,
    trust: Vec<TrustAnchor>,
    limits: BundleLimits,
    destination: PathBuf,
) -> Result<(Arc<RocksSessionStore>, RestoreReport)> {
    limits.validate()?;
    let root = crate::directory::open_root(&source)?;
    let input = crate::directory::open_child(&root, "backup.json", false)?;
    if input.metadata().map_err(io)?.len() > limits.manifest_bytes {
        return Err(limit("backup manifest too large"));
    }
    crate::work::io(limits.manifest_bytes + 1)?;
    let mut bytes = vec![];
    input.take(limits.manifest_bytes + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limits.manifest_bytes {
        return Err(limit("backup manifest grew"));
    }
    let manifest: BackupManifest = serde_json::from_slice(&bytes).map_err(|e| BundleError::new(ErrorCode::Format, e.to_string()))?;
    if serde_json::to_vec(&manifest).map_err(io)? != bytes || manifest.commitment()? != expected {
        return Err(integrity("backup manifest differs from independent commitment"));
    }
    manifest.validate(&limits)?;
    private_dir(&destination)?;
    let mut writer = Copier { root: &destination, limits: &limits, files: vec![], total: 0, stage: ArchiveStage::Restore };
    for file in &manifest.files {
        let input = nested(&root, &file.name)?;
        if input.metadata().map_err(io)?.len() != file.bytes {
            return Err(integrity("backup file length differs"));
        }
        writer.file(file.name.clone(), input, Some(file.digest))?;
    }
    let archives = Arc::new(BundleArchives::default());
    let catalogs: Vec<_> = manifest
        .storage
        .archives
        .iter()
        .map(|id| ArchiveCatalog {
            path: destination.join("archives").join(BlobId(*id).hex()),
            commitment: BlobId(*id),
            trust: trust.clone(),
        })
        .collect();
    crate::catalog::register_catalogs(&archives, &catalogs, &limits, ArchiveStage::Restore)?;
    let database = destination.join("db");
    let store = Arc::new(RocksSessionStore::open_archived(&database, manifest.options, archives.clone()).map_err(io)?);
    if inventory(&store, &limits)? != manifest.storage {
        return Err(integrity("restored keyspace differs from captured fingerprint/heads/pins"));
    }
    validate_pins(&store, &archives, manifest.storage.cold_blocks)?;
    let report = RestoreReport {
        backup: expected,
        storage: manifest.storage,
        database,
        archives: catalogs.into_iter().map(|c| c.path).collect(),
        writer_authority: VerificationStage::NotRun,
        application_replay: VerificationStage::NotRun,
    };
    publish(&destination, "restore.json", &serde_json::to_vec(&report).map_err(io)?)?;
    Ok((store, report))
}
