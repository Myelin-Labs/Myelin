//! Bounded raw file packages for build inputs and toolchains. Packages contain
//! no executable instructions, archive extraction, links or file permissions.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8] = b"MFPK";
pub const PACKAGE_FORMAT: &str = "myelin-file-package-v1";
const MAX_PACKAGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChunk {
    pub id: BlobId,
    pub bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    /// Portable relative name, never a host source path.
    pub name: String,
    pub bytes: u64,
    pub digest: BlobId,
    /// Ordered references; repeated content is allowed but charged each time.
    pub chunks: Vec<FileChunk>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePackage {
    pub format: String,
    pub files: Vec<PackageFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSource {
    pub name: String,
    pub path: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackPlan {
    pub files: Vec<FileSource>,
    pub chunk_bytes: u64,
    #[serde(default)]
    pub limits: BundleLimits,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedFiles {
    pub root: BlobId,
    /// These sources can be merged into ExportPlan.artifacts. The root becomes
    /// execution.build_inputs or execution.toolchain, pinned by external trust.
    pub artifacts: Vec<ArtifactSource>,
}
#[derive(Clone, Debug, Serialize)]
pub struct FilePackageReport {
    pub root: BlobId,
    pub files: usize,
    pub expanded_bytes: u64,
    pub chunk_references: usize,
}

fn fail(code: ErrorCode, message: impl Into<String>) -> BundleError {
    BundleError::new(code, message)
}
fn io(error: impl std::fmt::Display) -> BundleError {
    fail(ErrorCode::Io, error.to_string())
}
fn names<'a>(values: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut files = BTreeSet::new();
    let mut directories = BTreeMap::new();
    for name in values {
        crate::work::io(0)?;
        if name.is_empty() || name.len() > 1024 || !name.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._/".contains(&c)) {
            return Err(fail(ErrorCode::UnsafePath, "package name must be a bounded portable relative path"));
        }
        let parts: Vec<_> = name.split('/').collect();
        for part in &parts {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            if part.is_empty()
                || *part == "."
                || *part == ".."
                || part.len() > 255
                || part.ends_with('.')
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            {
                return Err(fail(ErrorCode::UnsafePath, "unsafe or platform-reserved package path component"));
            }
        }
        let lower = name.to_ascii_lowercase();
        if directories.contains_key(&lower) || !files.insert(lower) {
            return Err(fail(ErrorCode::UnsafePath, "duplicate, case-colliding or file/directory package name"));
        }
        for end in 1..parts.len() {
            let spelling = parts[..end].join("/");
            let directory = spelling.to_ascii_lowercase();
            if files.contains(&directory) {
                return Err(fail(ErrorCode::UnsafePath, "package file is also used as a directory"));
            }
            if directories.get(&directory).is_some_and(|old| old != &spelling) {
                return Err(fail(ErrorCode::UnsafePath, "package directories collide by case"));
            }
            directories.insert(directory, spelling);
        }
    }
    Ok(())
}
impl FilePackage {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(io)?);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8], limits: &BundleLimits) -> Result<Self> {
        if bytes.len() as u64 > MAX_PACKAGE_BYTES.min(limits.blob_bytes).min(limits.manifest_bytes) {
            return Err(fail(ErrorCode::Limit, "file package manifest exceeds byte budget"));
        }
        let json = bytes.strip_prefix(MAGIC).ok_or_else(|| fail(ErrorCode::Format, "not a typed file package"))?;
        let package: Self = serde_json::from_slice(json).map_err(|e| fail(ErrorCode::Format, e.to_string()))?;
        if package.encode()? != bytes {
            return Err(fail(ErrorCode::Format, "noncanonical file package"));
        }
        package.validate(limits)?;
        Ok(package)
    }
    pub fn validate(&self, limits: &BundleLimits) -> Result<FilePackageReport> {
        limits.validate()?;
        if self.format != PACKAGE_FORMAT || self.files.is_empty() || !self.files.windows(2).all(|p| p[0].name < p[1].name) {
            return Err(fail(ErrorCode::Format, "unsupported file package or empty/unsorted files"));
        }
        if self.files.len() > limits.nodes
            || self.encode()?.len() as u64 > MAX_PACKAGE_BYTES.min(limits.blob_bytes).min(limits.manifest_bytes)
        {
            return Err(fail(ErrorCode::Limit, "file package manifest/file-count budget exceeded"));
        }
        names(self.files.iter().map(|f| f.name.as_str()))?;
        let mut expanded_bytes = 0u64;
        let mut chunk_references = 0usize;
        for file in &self.files {
            crate::work::io(0)?;
            expanded_bytes = expanded_bytes.checked_add(file.bytes).ok_or_else(|| fail(ErrorCode::Limit, "package bytes overflow"))?;
            chunk_references = chunk_references
                .checked_add(file.chunks.len())
                .ok_or_else(|| fail(ErrorCode::Limit, "package references overflow"))?;
            if expanded_bytes > limits.total_bytes || chunk_references > limits.edges {
                return Err(fail(ErrorCode::Limit, "package expanded bytes or repeated chunk budget exceeded"));
            }
            let mut total = 0u64;
            for chunk in &file.chunks {
                crate::work::io(0)?;
                if chunk.bytes == 0 || chunk.bytes > limits.blob_bytes {
                    return Err(fail(ErrorCode::Limit, "package chunk outside blob budget"));
                }
                total = total.checked_add(chunk.bytes).ok_or_else(|| fail(ErrorCode::Limit, "package chunk bytes overflow"))?;
            }
            if total != file.bytes || (file.bytes == 0 && file.digest != BlobId::of(b"")) {
                return Err(fail(ErrorCode::Integrity, "package file size or empty-file digest disagrees"));
            }
        }
        Ok(FilePackageReport { root: BlobId::of(&self.encode()?), files: self.files.len(), expanded_bytes, chunk_references })
    }
    fn dependencies(&self) -> Vec<BlobId> {
        self.files.iter().flat_map(|f| f.chunks.iter().map(|c| c.id)).collect::<BTreeSet<_>>().into_iter().collect()
    }
}

struct HashedWriter<'a, W: Write> {
    output: &'a mut W,
    hash: blake3::Hasher,
}
impl<W: Write> Write for HashedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.output.write(bytes)?;
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
impl BundleDirectory {
    fn package(&self, root: BlobId) -> Result<FilePackage> {
        let cap = MAX_PACKAGE_BYTES.min(self.limits().manifest_bytes).min(self.limits().blob_bytes);
        let package = FilePackage::decode(&self.read_blob(root, cap)?, self.limits())?;
        let owner = self.descriptor(root)?;
        for dependency in package.dependencies() {
            if !owner.dependencies.contains(&dependency) {
                return Err(fail(ErrorCode::MissingDependency, "file package chunk missing from dependency edges"));
            }
        }
        for file in &package.files {
            for chunk in &file.chunks {
                if self.descriptor(chunk.id)?.bytes != chunk.bytes {
                    return Err(fail(ErrorCode::Integrity, "file package chunk length differs from inventory"));
                }
            }
        }
        Ok(package)
    }
    fn copy_package_file(&self, file: &PackageFile, output: &mut impl Write) -> Result<()> {
        let mut writer = HashedWriter { output, hash: blake3::Hasher::new() };
        for chunk in &file.chunks {
            self.copy_blob(chunk.id, &mut writer)?;
        }
        if writer.hash.finalize().as_bytes() != &file.digest.0 {
            return Err(fail(ErrorCode::Integrity, "assembled file digest differs from package"));
        }
        Ok(())
    }
    pub(crate) fn verify_file_packages(&self) -> Result<Vec<FilePackageReport>> {
        crate::work::checkpoint(ArchiveStage::Packages, None, 0)?;
        let roots: BTreeSet<_> =
            self.manifest().sessions.iter().flat_map(|s| [s.execution.build_inputs, s.execution.toolchain]).collect();
        let mut reports = vec![];
        let mut expanded = 0u64;
        let mut references = 0usize;
        for root in roots {
            crate::work::checkpoint(ArchiveStage::Packages, Some(root.hex()), 0)?;
            let package = self.package(root)?;
            let report = package.validate(self.limits())?;
            expanded = expanded.checked_add(report.expanded_bytes).ok_or_else(|| fail(ErrorCode::Limit, "package total overflow"))?;
            references = references
                .checked_add(report.chunk_references)
                .ok_or_else(|| fail(ErrorCode::Limit, "package references overflow"))?;
            if expanded > self.limits().total_bytes || references > self.limits().edges {
                return Err(fail(ErrorCode::Limit, "combined file packages exceed expanded-byte/reference budgets"));
            }
            for file in &package.files {
                self.copy_package_file(file, &mut std::io::sink())?;
            }
            reports.push(report);
        }
        Ok(reports)
    }
    /// Reconstruct a recognized build/toolchain root into a new caller-owned
    /// directory. Failed copies never publish that file; earlier completed
    /// files may remain, but no success report is returned. No code is run.
    pub fn assemble_file_package(
        &self,
        anchors: &[TrustAnchor],
        root: BlobId,
        destination: impl AsRef<Path>,
    ) -> Result<FilePackageReport> {
        if !crate::work::active() {
            let ArchiveOutput::Assembled(report) =
                ArchiveTask::assemble(self.duplicate()?, anchors.to_vec(), root, destination.as_ref().into())?.default_step()?
            else {
                unreachable!("assemble task result")
            };
            return Ok(report);
        }
        let inventory = self.verify_inventory(anchors)?;
        let report = inventory
            .file_packages
            .into_iter()
            .find(|p| p.root == root)
            .ok_or_else(|| fail(ErrorCode::Format, "requested root is not a recognized typed build/toolchain package"))?;
        let package = self.package(root)?;
        let destination = destination.as_ref();
        fs::create_dir(destination).map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(destination, fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        for file in &package.files {
            crate::work::checkpoint(ArchiveStage::Assemble, Some(file.name.clone()), 0)?;
            let path = destination.join(&file.name);
            let parent = path.parent().expect("destination supplies a parent");
            fs::create_dir_all(parent).map_err(io)?;
            let mut output = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
            self.copy_package_file(file, &mut output)?;
            output.as_file().sync_all().map_err(io)?;
            output.persist_noclobber(&path).map_err(io)?;
            File::open(parent).map_err(io)?.sync_all().map_err(io)?;
        }
        File::open(destination).map_err(io)?.sync_all().map_err(io)?;
        Ok(report)
    }
}

/// Create a package from explicitly supplied local regular files. Hashes bind
/// the observed file bytes; packaging establishes no official recognition.
pub fn pack_files(plan: &PackPlan, destination: impl AsRef<Path>) -> Result<PackedFiles> {
    if !crate::work::active() {
        let ArchiveOutput::Packed(files) = ArchiveTask::pack(plan.clone(), destination.as_ref().into())?.default_step()? else {
            unreachable!("pack task result")
        };
        return Ok(files);
    }
    plan.limits.validate()?;
    if plan.files.is_empty()
        || plan.files.len() > plan.limits.nodes
        || plan.chunk_bytes == 0
        || plan.chunk_bytes > plan.limits.blob_bytes
    {
        return Err(fail(ErrorCode::Limit, "file packing count/chunk budget exceeded"));
    }
    names(plan.files.iter().map(|f| f.name.as_str()))?;
    let destination = destination.as_ref();
    fs::create_dir(destination).map_err(io)?;
    let destination = destination.canonicalize().map_err(io)?;
    let mut sources = plan.files.clone();
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    let mut artifacts = BTreeMap::new();
    let mut files = vec![];
    let mut expanded = 0u64;
    let mut references = 0usize;
    for source in sources {
        crate::work::checkpoint(ArchiveStage::Pack, Some(source.name.clone()), 0)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let mut input = options.open(&source.path).map_err(io)?;
        let metadata = input.metadata().map_err(io)?;
        if !metadata.is_file() {
            return Err(fail(ErrorCode::UnsafePath, "package source must be a regular file"));
        }
        let length = metadata.len();
        expanded = expanded.checked_add(length).ok_or_else(|| fail(ErrorCode::Limit, "source bytes overflow"))?;
        if expanded > plan.limits.total_bytes {
            return Err(fail(ErrorCode::Limit, "expanded source budget exceeded"));
        }
        let count = length / plan.chunk_bytes + u64::from(length % plan.chunk_bytes != 0);
        references = references
            .checked_add(usize::try_from(count).map_err(io)?)
            .ok_or_else(|| fail(ErrorCode::Limit, "chunk count overflow"))?;
        if references > plan.limits.edges {
            return Err(fail(ErrorCode::Limit, "chunk reference budget exceeded"));
        }
        let mut chunks = vec![];
        let mut remaining = length;
        let mut whole = blake3::Hasher::new();
        let mut buffer = [0u8; 65536];
        while remaining != 0 {
            let bytes = remaining.min(plan.chunk_bytes);
            let mut left = bytes;
            let mut output = tempfile::NamedTempFile::new_in(&destination).map_err(io)?;
            let mut hash = blake3::Hasher::new();
            while left != 0 {
                crate::work::io(left.min(65536) * 2)?;
                let n = input.read(&mut buffer[..left.min(65536) as usize]).map_err(io)?;
                if n == 0 {
                    return Err(fail(ErrorCode::SourceChanged, "package source truncated during packing"));
                }
                hash.update(&buffer[..n]);
                whole.update(&buffer[..n]);
                output.write_all(&buffer[..n]).map_err(io)?;
                left -= n as u64;
            }
            let id = BlobId(*hash.finalize().as_bytes());
            if !artifacts.contains_key(&id) {
                if artifacts.len() + 1 >= plan.limits.nodes {
                    return Err(fail(ErrorCode::Limit, "package blob count exceeded"));
                }
                output.as_file().sync_all().map_err(io)?;
                let path = destination.join(id.hex());
                output.persist_noclobber(&path).map_err(io)?;
                artifacts.insert(id, ArtifactSource { id, path, dependencies: vec![] });
            }
            chunks.push(FileChunk { id, bytes });
            remaining -= bytes;
        }
        crate::work::io(1)?;
        if input.read(&mut buffer[..1]).map_err(io)? != 0 {
            return Err(fail(ErrorCode::SourceChanged, "package source grew during packing"));
        }
        crate::directory::unchanged(&input, &metadata)?;
        files.push(PackageFile { name: source.name, bytes: length, digest: BlobId(*whole.finalize().as_bytes()), chunks });
    }
    let package = FilePackage { format: PACKAGE_FORMAT.into(), files };
    package.validate(&plan.limits)?;
    let bytes = package.encode()?;
    if expanded.checked_add(bytes.len() as u64).is_none_or(|n| n > plan.limits.total_bytes) {
        return Err(fail(ErrorCode::Limit, "package plus expanded files exceed total byte budget"));
    }
    let root = BlobId::of(&bytes);
    let path = destination.join(root.hex());
    crate::work::io(bytes.len() as u64)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(&path).map_err(io)?;
    output.write_all(&bytes).map_err(io)?;
    output.sync_all().map_err(io)?;
    artifacts.insert(root, ArtifactSource { id: root, path, dependencies: package.dependencies() });
    File::open(&destination).map_err(io)?.sync_all().map_err(io)?;
    Ok(PackedFiles { root, artifacts: artifacts.into_values().collect() })
}
