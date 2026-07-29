// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Fail-closed process boundary to the independently versioned CellScript compiler.

use myelin_exec::{
    scheduler::{AccessMode, SchedulerPlan},
    CellTx,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Embedded compiler source and toolchain lock.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolchainLock {
    /// Lock schema.
    pub schema: String,
    /// Canonical upstream repository.
    pub repository: String,
    /// Exact annotated release tag whose peeled commit is `source_revision`.
    pub release_tag: String,
    /// Exact package version expected from `cellc --version`.
    pub package_version: String,
    /// Exact upstream Git revision.
    pub source_revision: String,
    /// Rust toolchain used by the independent compiler workspace.
    pub rust_toolchain: String,
    /// Exact top-level compiler metadata schema.
    pub metadata_schema_version: u32,
    /// Exact source-metadata component schema.
    pub source_metadata_schema_version: u32,
    /// Exact artifact-metadata component schema.
    pub artifact_metadata_schema_version: u32,
    /// Exact constraint-metadata component schema.
    pub constraints_metadata_schema_version: u32,
    /// Canonical repository for the upstream workspace's sibling CKB SDK manifest dependency.
    pub ckb_sdk_repository: String,
    /// Exact CKB SDK release tag.
    pub ckb_sdk_release_tag: String,
    /// Exact CKB SDK package version.
    pub ckb_sdk_package_version: String,
    /// Exact peeled CKB SDK release commit.
    pub ckb_sdk_source_revision: String,
}

impl ToolchainLock {
    /// Load the lock committed with this adapter.
    pub fn embedded() -> Result<Self, AdapterError> {
        let lock: Self = serde_json::from_str(include_str!("../cellscript-toolchain.lock.json")).map_err(AdapterError::Json)?;
        if lock.schema != "myelin-cellscript-toolchain-lock-v2" {
            return Err(AdapterError::Attestation("unsupported toolchain lock schema".to_owned()));
        }
        if lock.repository != "https://github.com/CellScript-Labs/CellScript" {
            return Err(AdapterError::Attestation("toolchain lock repository is not the canonical upstream".to_owned()));
        }
        if lock.ckb_sdk_repository != "https://github.com/nervosnetwork/ckb-sdk-rust"
            || lock.ckb_sdk_release_tag != format!("v{}", lock.ckb_sdk_package_version)
            || lock.ckb_sdk_source_revision.len() != 40
            || !lock.ckb_sdk_source_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AdapterError::Attestation("toolchain lock CKB SDK identity is invalid".to_owned()));
        }
        if lock.source_revision.len() != 40 || !lock.source_revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AdapterError::Attestation("toolchain lock source revision must be a 40-character Git hash".to_owned()));
        }
        if lock.release_tag != format!("v{}", lock.package_version) || lock.rust_toolchain.is_empty() {
            return Err(AdapterError::Attestation("toolchain lock version fields must not be empty".to_owned()));
        }
        if lock.metadata_schema_version == 0
            || lock.source_metadata_schema_version == 0
            || lock.artifact_metadata_schema_version == 0
            || lock.constraints_metadata_schema_version == 0
        {
            return Err(AdapterError::Attestation("toolchain lock metadata schema versions must be nonzero".to_owned()));
        }
        Ok(lock)
    }
}

/// Local proof that a particular compiler binary was built from the locked source.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerAttestation {
    /// Attestation schema.
    pub schema: String,
    /// Canonical upstream repository checked while building.
    pub repository: String,
    /// Exact release tag checked while building.
    pub release_tag: String,
    /// Package version reported by the compiler.
    pub package_version: String,
    /// Exact source revision checked by the attester.
    pub source_revision: String,
    /// Exact Rust toolchain used for the build.
    pub rust_toolchain: String,
    /// Canonical CKB SDK repository checked while resolving the upstream workspace.
    pub ckb_sdk_repository: String,
    /// Exact CKB SDK release tag checked while resolving the upstream workspace.
    pub ckb_sdk_release_tag: String,
    /// Exact CKB SDK source revision checked while resolving the upstream workspace.
    pub ckb_sdk_source_revision: String,
    /// Host target triple of the compiler executable.
    pub host_target: String,
    /// BLAKE3 digest of the compiler executable.
    pub binary_blake3: String,
}

/// A verified compiler installation.
#[derive(Clone, Debug)]
pub struct CellScriptInstallation {
    binary: PathBuf,
    attestation: CompilerAttestation,
    lock: ToolchainLock,
}

/// One direct compiler invocation.
#[derive(Clone, Debug)]
pub struct CompileRequest<'a> {
    /// Absolute CellScript source path.
    pub source: &'a Path,
    /// Absolute output artifact path.
    pub artifact: &'a Path,
    /// CellScript target profile. Myelin production code uses `ckb`.
    pub target_profile: &'a str,
    /// Optional action-specific entry point.
    pub entry_action: Option<&'a str>,
}

/// Hash-bound result returned by the adapter.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledArtifact {
    /// Compiler package version.
    pub compiler_version: String,
    /// Exact compiler source revision.
    pub compiler_revision: String,
    /// Exact top-level metadata schema accepted by the adapter.
    pub metadata_schema_version: u32,
    /// Exact source-metadata component schema accepted by the adapter.
    pub source_metadata_schema_version: u32,
    /// Exact artifact-metadata component schema accepted by the adapter.
    pub artifact_metadata_schema_version: u32,
    /// Exact constraint-metadata component schema accepted by the adapter.
    pub constraints_metadata_schema_version: u32,
    /// BLAKE3 source digest.
    pub source_blake3: String,
    /// BLAKE3 artifact digest.
    pub artifact_blake3: String,
    /// BLAKE3 metadata digest.
    pub metadata_blake3: String,
    /// Artifact path.
    pub artifact: PathBuf,
    /// Compiler metadata path.
    pub metadata: PathBuf,
    /// Compiler-emitted target profile.
    pub target_profile: String,
}

/// Concrete CKB source selected by compiler scheduling metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompilerSchedulerSource {
    /// A consumed transaction input.
    Input,
    /// A read-only cell dependency.
    CellDep,
    /// A newly created transaction output.
    Output,
}

/// One compiler-declared Cell access whose logical conflict domain is unresolved.
///
/// `binding` is a source-language diagnostic label. It is deliberately never
/// hashed or used as a conflict key: two different bindings can refer to the same
/// logical state, and one binding can refer to different state in different
/// transactions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerSchedulerAccess {
    operation: String,
    syscall: String,
    source: CompilerSchedulerSource,
    index: u32,
    binding: String,
}

impl CompilerSchedulerAccess {
    /// Compiler operation name.
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Compiler syscall name, retained for diagnostics.
    pub fn syscall(&self) -> &str {
        &self.syscall
    }

    /// Concrete transaction source class.
    pub fn source(&self) -> CompilerSchedulerSource {
        self.source
    }

    /// Index within the selected source class.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Source-language binding name, retained for diagnostics only.
    pub fn binding(&self) -> &str {
        &self.binding
    }
}

/// Hash-authenticated compiler access template awaiting state-side conflict resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerSchedulerTemplate {
    action: String,
    effect_class: String,
    parallelizable: bool,
    estimated_cycles: u64,
    accesses: Vec<CompilerSchedulerAccess>,
    artifact_blake3: String,
    metadata_blake3: String,
}

impl CompilerSchedulerTemplate {
    /// Selected action name.
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Compiler effect class.
    pub fn effect_class(&self) -> &str {
        &self.effect_class
    }

    /// Whether the compiler explicitly admitted parallel execution.
    pub fn parallelizable(&self) -> bool {
        self.parallelizable
    }

    /// Compiler cycle estimate. This is not the VM cycle limit.
    pub fn estimated_cycles(&self) -> u64 {
        self.estimated_cycles
    }

    /// Cell-state accesses that require host conflict-domain resolution.
    pub fn accesses(&self) -> &[CompilerSchedulerAccess] {
        &self.accesses
    }

    /// BLAKE3 digest of the compiler artifact that produced this template.
    pub fn artifact_blake3(&self) -> &str {
        &self.artifact_blake3
    }

    /// BLAKE3 digest of the compiler metadata that produced this template.
    pub fn metadata_blake3(&self) -> &str {
        &self.metadata_blake3
    }

    /// Bind every compiler access to a logical conflict domain resolved from
    /// trusted live/output Cell state.
    ///
    /// The resolver is the explicit trust boundary. It must apply Myelin's
    /// `ConflictKeySpec` to the concrete Cell selected by `source` and `index`.
    /// Returning a source binding-name hash is invalid policy.
    pub fn bind<R>(&self, tx: &CellTx, mut resolver: R) -> Result<SchedulerPlan, AdapterError>
    where
        R: FnMut(&CellTx, &CompilerSchedulerAccess) -> Result<[u8; 32], String>,
    {
        let mut resolved = Vec::with_capacity(self.accesses.len() + 1);
        for access in &self.accesses {
            validate_access_bounds(tx, access)?;
            let conflict_hash = resolver(tx, access).map_err(|error| {
                AdapterError::SchedulerBinding(format!(
                    "cannot resolve conflict domain for action {} binding {}: {error}",
                    self.action, access.binding
                ))
            })?;
            if conflict_hash == [0; 32] {
                return Err(AdapterError::SchedulerBinding(format!(
                    "resolver returned a zero conflict hash for action {} binding {}",
                    self.action, access.binding
                )));
            }
            resolved.push((conflict_hash, access_mode(&access.operation)?));
        }

        // Every compiler-backed plan participates in this barrier. Read/Read
        // remains parallel; any action not explicitly marked parallelizable
        // conflicts with every other compiler-backed action.
        let barrier = *blake3::hash(b"myelin:cellscript-scheduler-global-barrier:v1").as_bytes();
        resolved.push((barrier, if self.parallelizable { AccessMode::Read } else { AccessMode::Write }));
        SchedulerPlan::new(tx, resolved).map_err(|error| AdapterError::SchedulerBinding(error.to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct CompilerMetadataEnvelope {
    compiler_version: String,
    metadata_schema_version: u32,
    source_metadata_schema_version: u32,
    artifact_metadata_schema_version: u32,
    constraints_metadata_schema_version: u32,
    target_profile: CompilerTargetProfile,
    actions: Vec<CompilerActionMetadata>,
}

#[derive(Debug, Deserialize)]
struct CompilerTargetProfile {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CompilerActionMetadata {
    name: String,
    effect_class: String,
    parallelizable: bool,
    estimated_cycles: u64,
    ckb_runtime_accesses: Vec<CompilerRuntimeAccess>,
}

#[derive(Debug, Deserialize)]
struct CompilerRuntimeAccess {
    operation: String,
    syscall: String,
    source: String,
    index: usize,
    binding: String,
}

impl CellScriptInstallation {
    /// Verify a compiler binary against an attestation and the embedded source lock.
    pub fn verify(binary: impl Into<PathBuf>, attestation_path: &Path) -> Result<Self, AdapterError> {
        let binary = binary.into();
        let lock = ToolchainLock::embedded()?;
        let attestation: CompilerAttestation =
            serde_json::from_slice(&fs::read(attestation_path).map_err(AdapterError::Io)?).map_err(AdapterError::Json)?;
        if attestation.schema != "myelin-cellscript-compiler-attestation-v2" {
            return Err(AdapterError::Attestation("unsupported attestation schema".to_owned()));
        }
        if attestation.repository != lock.repository
            || attestation.release_tag != lock.release_tag
            || attestation.package_version != lock.package_version
            || attestation.source_revision != lock.source_revision
            || attestation.rust_toolchain != lock.rust_toolchain
            || attestation.ckb_sdk_repository != lock.ckb_sdk_repository
            || attestation.ckb_sdk_release_tag != lock.ckb_sdk_release_tag
            || attestation.ckb_sdk_source_revision != lock.ckb_sdk_source_revision
            || attestation.host_target.is_empty()
        {
            return Err(AdapterError::Attestation("attestation does not match the committed toolchain lock".to_owned()));
        }
        let actual_hash = digest_file(&binary)?;
        if actual_hash != attestation.binary_blake3 {
            return Err(AdapterError::Attestation("compiler binary digest mismatch".to_owned()));
        }
        verify_version(&binary, &lock.package_version)?;
        Ok(Self { binary, attestation, lock })
    }

    /// Compile one source after rechecking the compiler binary.
    pub fn compile(&self, request: CompileRequest<'_>) -> Result<CompiledArtifact, AdapterError> {
        require_absolute(request.source, "source")?;
        require_absolute(request.artifact, "artifact")?;
        if request.target_profile != "ckb" {
            return Err(AdapterError::InvalidRequest("only the upstream ckb target profile is admitted".to_owned()));
        }
        if digest_file(&self.binary)? != self.attestation.binary_blake3 {
            return Err(AdapterError::Attestation("compiler binary changed after verification".to_owned()));
        }
        let mut command = Command::new(&self.binary);
        command
            .arg(request.source)
            .arg("--output")
            .arg(request.artifact)
            .args(["--target", "riscv64-elf", "--target-profile"])
            .arg(request.target_profile)
            .arg("--json");
        if let Some(action) = request.entry_action {
            command.arg("--entry-action").arg(action);
        }
        let output = command.output().map_err(AdapterError::Io)?;
        if !output.status.success() {
            return Err(AdapterError::CompilerFailure {
                status: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let payload: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(AdapterError::Json)?;
        if payload.get("status").and_then(serde_json::Value::as_str) != Some("ok")
            || payload.get("mode").and_then(serde_json::Value::as_str) != Some("direct-build")
        {
            return Err(AdapterError::CompilerProtocol("compiler did not return a successful direct-build result".to_owned()));
        }
        let artifact = json_path(&payload, "artifact")?;
        let metadata = json_path(&payload, "metadata")?;
        if artifact != request.artifact {
            return Err(AdapterError::CompilerProtocol("compiler reported a different artifact path".to_owned()));
        }
        let target_profile = payload
            .get("target_profile")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| AdapterError::CompilerProtocol("missing target_profile".to_owned()))?;
        if target_profile != request.target_profile {
            return Err(AdapterError::CompilerProtocol("compiler emitted the wrong target profile".to_owned()));
        }
        let metadata_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&metadata).map_err(AdapterError::Io)?).map_err(AdapterError::Json)?;
        if metadata_json.pointer("/target_profile/name").and_then(serde_json::Value::as_str) != Some(request.target_profile) {
            return Err(AdapterError::CompilerProtocol("artifact metadata has the wrong target profile".to_owned()));
        }
        Ok(CompiledArtifact {
            compiler_version: self.lock.package_version.clone(),
            compiler_revision: self.lock.source_revision.clone(),
            metadata_schema_version: self.lock.metadata_schema_version,
            source_metadata_schema_version: self.lock.source_metadata_schema_version,
            artifact_metadata_schema_version: self.lock.artifact_metadata_schema_version,
            constraints_metadata_schema_version: self.lock.constraints_metadata_schema_version,
            source_blake3: digest_file(request.source)?,
            artifact_blake3: digest_file(&artifact)?,
            metadata_blake3: digest_file(&metadata)?,
            artifact,
            metadata,
            target_profile: target_profile.to_owned(),
        })
    }
}

impl CompiledArtifact {
    /// Load one action's compiler-declared access template from hash-bound metadata.
    ///
    /// The upstream v1 scheduler witness is intentionally ignored: its
    /// `binding_hash` commits only to a source-language name and is not a Myelin
    /// logical conflict domain.
    pub fn scheduler_template(&self, action: &str) -> Result<CompilerSchedulerTemplate, AdapterError> {
        if action.is_empty() {
            return Err(AdapterError::InvalidRequest("scheduler action must not be empty".to_owned()));
        }
        if digest_file(&self.artifact)? != self.artifact_blake3 || digest_file(&self.metadata)? != self.metadata_blake3 {
            return Err(AdapterError::SchedulerBinding("compiled artifact or metadata changed after compilation".to_owned()));
        }
        let metadata: CompilerMetadataEnvelope =
            serde_json::from_slice(&fs::read(&self.metadata).map_err(AdapterError::Io)?).map_err(AdapterError::Json)?;
        if metadata.compiler_version != self.compiler_version
            || metadata.target_profile.name != self.target_profile
            || metadata.metadata_schema_version != self.metadata_schema_version
            || metadata.source_metadata_schema_version != self.source_metadata_schema_version
            || metadata.artifact_metadata_schema_version != self.artifact_metadata_schema_version
            || metadata.constraints_metadata_schema_version != self.constraints_metadata_schema_version
        {
            return Err(AdapterError::SchedulerBinding(
                "compiler version, target profile, or metadata schema does not match the locked compiler contract".to_owned(),
            ));
        }
        let mut matching = metadata.actions.into_iter().filter(|candidate| candidate.name == action);
        let selected = matching
            .next()
            .ok_or_else(|| AdapterError::SchedulerBinding(format!("compiler metadata has no action named {action}")))?;
        if matching.next().is_some() {
            return Err(AdapterError::SchedulerBinding(format!("compiler metadata contains duplicate action {action}")));
        }
        if !matches!(selected.effect_class.as_str(), "Pure" | "ReadOnly" | "Mutating" | "Creating" | "Destroying") {
            return Err(AdapterError::SchedulerBinding(format!("unknown compiler effect class {}", selected.effect_class)));
        }

        let mut seen = BTreeSet::new();
        let mut accesses = Vec::new();
        for access in selected.ckb_runtime_accesses {
            if !compiler_marks_cell_state_access(&access) {
                continue;
            }
            let source = match access.source.as_str() {
                "Input" => CompilerSchedulerSource::Input,
                "CellDep" => CompilerSchedulerSource::CellDep,
                "Output" => CompilerSchedulerSource::Output,
                _ => unreachable!("cell-state predicate admits only concrete Cell sources"),
            };
            let index = u32::try_from(access.index)
                .map_err(|_| AdapterError::SchedulerBinding("compiler access index exceeds u32".to_owned()))?;
            let key = (access.operation.clone(), source as u8, index, access.binding.clone());
            if !seen.insert(key) {
                return Err(AdapterError::SchedulerBinding(format!(
                    "duplicate compiler Cell access {} {}:{} ({})",
                    access.operation, access.source, access.index, access.binding
                )));
            }
            accesses.push(CompilerSchedulerAccess {
                operation: access.operation,
                syscall: access.syscall,
                source,
                index,
                binding: access.binding,
            });
        }
        Ok(CompilerSchedulerTemplate {
            action: selected.name,
            effect_class: selected.effect_class,
            parallelizable: selected.parallelizable,
            estimated_cycles: selected.estimated_cycles,
            accesses,
            artifact_blake3: self.artifact_blake3.clone(),
            metadata_blake3: self.metadata_blake3.clone(),
        })
    }
}

fn compiler_marks_cell_state_access(access: &CompilerRuntimeAccess) -> bool {
    matches!(access.source.as_str(), "Input" | "CellDep" | "Output")
        && matches!(
            access.operation.as_str(),
            "input"
                | "consume"
                | "transfer"
                | "destroy"
                | "claim"
                | "settle"
                | "read_ref"
                | "output"
                | "create"
                | "create_unique"
                | "replace_unique"
                | "create_unique-identity-lock_hash"
                | "create_unique-identity-type_hash"
                | "replace_unique-identity-lock_hash"
                | "replace_unique-identity-type_hash"
                | "mutate-input"
                | "mutate-output"
        )
}

fn access_mode(operation: &str) -> Result<AccessMode, AdapterError> {
    match operation {
        "read_ref" => Ok(AccessMode::Read),
        "input"
        | "consume"
        | "transfer"
        | "destroy"
        | "claim"
        | "settle"
        | "output"
        | "create"
        | "create_unique"
        | "replace_unique"
        | "create_unique-identity-lock_hash"
        | "create_unique-identity-type_hash"
        | "replace_unique-identity-lock_hash"
        | "replace_unique-identity-type_hash"
        | "mutate-input"
        | "mutate-output" => Ok(AccessMode::Write),
        other => Err(AdapterError::SchedulerBinding(format!("unsupported compiler scheduler operation {other}"))),
    }
}

fn validate_access_bounds(tx: &CellTx, access: &CompilerSchedulerAccess) -> Result<(), AdapterError> {
    let available = match access.source {
        CompilerSchedulerSource::Input => tx.inputs.len(),
        CompilerSchedulerSource::CellDep => tx.cell_deps.len(),
        CompilerSchedulerSource::Output => tx.outputs.len(),
    };
    if usize::try_from(access.index).unwrap_or(usize::MAX) >= available {
        return Err(AdapterError::SchedulerBinding(format!(
            "compiler access {}:{} is out of bounds for {} entries",
            match access.source {
                CompilerSchedulerSource::Input => "Input",
                CompilerSchedulerSource::CellDep => "CellDep",
                CompilerSchedulerSource::Output => "Output",
            },
            access.index,
            available
        )));
    }
    Ok(())
}

/// Build and attest the compiler from a clean checkout at the locked Git revision.
pub fn build_and_attest(source_root: &Path) -> Result<(PathBuf, CompilerAttestation), AdapterError> {
    let lock = ToolchainLock::embedded()?;
    require_absolute(source_root, "source-root")?;
    let output = Command::new("git").arg("-C").arg(source_root).args(["rev-parse", "HEAD"]).output().map_err(AdapterError::Io)?;
    if !output.status.success() {
        return Err(AdapterError::Attestation("cannot resolve CellScript source revision".to_owned()));
    }
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if revision != lock.source_revision {
        return Err(AdapterError::Attestation(format!("source revision {revision} does not match lock {}", lock.source_revision)));
    }
    let tagged_revision = git_stdout(source_root, &["rev-parse", &format!("{}^{{commit}}", lock.release_tag)])?;
    if tagged_revision != lock.source_revision {
        return Err(AdapterError::Attestation(format!(
            "release tag {} resolves to {tagged_revision}, expected {}",
            lock.release_tag, lock.source_revision
        )));
    }
    let origin = git_stdout(source_root, &["remote", "get-url", "origin"])?;
    if normalize_repository(&origin) != normalize_repository(&lock.repository) {
        return Err(AdapterError::Attestation(format!("CellScript origin {origin} is not the locked repository {}", lock.repository)));
    }
    let ckb_sdk_root = source_root
        .parent()
        .ok_or_else(|| AdapterError::Attestation("CellScript source root has no parent for its locked CKB SDK sibling".to_owned()))?
        .join("ckb-sdk-rust");
    let ckb_sdk_revision = git_stdout(&ckb_sdk_root, &["rev-parse", "HEAD"])?;
    if ckb_sdk_revision != lock.ckb_sdk_source_revision {
        return Err(AdapterError::Attestation(format!(
            "CKB SDK source revision {ckb_sdk_revision} does not match lock {}",
            lock.ckb_sdk_source_revision
        )));
    }
    let ckb_sdk_tagged_revision = git_stdout(&ckb_sdk_root, &["rev-parse", &format!("{}^{{commit}}", lock.ckb_sdk_release_tag)])?;
    if ckb_sdk_tagged_revision != lock.ckb_sdk_source_revision {
        return Err(AdapterError::Attestation(format!(
            "CKB SDK release tag {} resolves to {ckb_sdk_tagged_revision}, expected {}",
            lock.ckb_sdk_release_tag, lock.ckb_sdk_source_revision
        )));
    }
    let ckb_sdk_origin = git_stdout(&ckb_sdk_root, &["remote", "get-url", "origin"])?;
    if normalize_repository(&ckb_sdk_origin) != normalize_repository(&lock.ckb_sdk_repository) {
        return Err(AdapterError::Attestation(format!(
            "CKB SDK origin {ckb_sdk_origin} is not the locked repository {}",
            lock.ckb_sdk_repository
        )));
    }
    require_clean_checkout(&ckb_sdk_root, "CKB SDK")?;
    require_clean_checkout(source_root, "CellScript source")?;
    let submodules = Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["submodule", "status", "--recursive"])
        .output()
        .map_err(AdapterError::Io)?;
    if !submodules.status.success()
        || submodules.stdout.split(|byte| *byte == b'\n').any(|line| matches!(line.first(), Some(b'-' | b'+' | b'U')))
    {
        return Err(AdapterError::Attestation("CellScript submodules are not initialized at their locked revisions".to_owned()));
    }
    let manifest = source_root.join("Cargo.toml");
    let build = Command::new("cargo")
        .arg(format!("+{}", lock.rust_toolchain))
        .args(["build", "--locked", "--release", "--bin", "cellc", "--manifest-path"])
        .arg(&manifest)
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .map_err(AdapterError::Io)?;
    if !build.success() {
        return Err(AdapterError::Attestation("building the locked CellScript compiler failed".to_owned()));
    }
    let binary = source_root.join("target").join("release").join(if cfg!(windows) { "cellc.exe" } else { "cellc" });
    verify_version(&binary, &lock.package_version)?;
    let host_target = rustc_host_target(&lock.rust_toolchain)?;
    let attestation = CompilerAttestation {
        schema: "myelin-cellscript-compiler-attestation-v2".to_owned(),
        repository: lock.repository,
        release_tag: lock.release_tag,
        package_version: lock.package_version,
        source_revision: revision,
        rust_toolchain: lock.rust_toolchain,
        ckb_sdk_repository: lock.ckb_sdk_repository,
        ckb_sdk_release_tag: lock.ckb_sdk_release_tag,
        ckb_sdk_source_revision: ckb_sdk_revision,
        host_target,
        binary_blake3: digest_file(&binary)?,
    };
    Ok((binary, attestation))
}

fn require_clean_checkout(source_root: &Path, label: &str) -> Result<(), AdapterError> {
    let status = Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .map_err(AdapterError::Io)?;
    if !status.status.success() || !status.stdout.is_empty() {
        return Err(AdapterError::Attestation(format!("{label} checkout is not clean")));
    }
    Ok(())
}

fn git_stdout(source_root: &Path, args: &[&str]) -> Result<String, AdapterError> {
    let output = Command::new("git").arg("-C").arg(source_root).args(args).output().map_err(AdapterError::Io)?;
    if !output.status.success() {
        return Err(AdapterError::Attestation(format!("git {} failed", args.join(" "))));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn normalize_repository(repository: &str) -> String {
    repository.trim_end_matches('/').trim_end_matches(".git").to_ascii_lowercase()
}

fn rustc_host_target(toolchain: &str) -> Result<String, AdapterError> {
    let output = Command::new("rustc").arg(format!("+{toolchain}")).arg("-vV").output().map_err(AdapterError::Io)?;
    if !output.status.success() {
        return Err(AdapterError::Attestation(format!("rustc +{toolchain} -vV failed")));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .filter(|host| !host.is_empty())
        .ok_or_else(|| AdapterError::Attestation("rustc did not report a host target".to_owned()))
}

fn verify_version(binary: &Path, expected: &str) -> Result<(), AdapterError> {
    let output = Command::new(binary).arg("--version").output().map_err(AdapterError::Io)?;
    if !output.status.success() {
        return Err(AdapterError::Attestation("cellc --version failed".to_owned()));
    }
    let version = String::from_utf8_lossy(&output.stdout);
    if version.trim() != format!("cellc {expected}") {
        return Err(AdapterError::Attestation(format!("expected cellc {expected}, got {}", version.trim())));
    }
    Ok(())
}

fn require_absolute(path: &Path, field: &str) -> Result<(), AdapterError> {
    if !path.is_absolute() {
        return Err(AdapterError::InvalidRequest(format!("{field} path must be absolute")));
    }
    Ok(())
}

fn json_path(value: &serde_json::Value, field: &str) -> Result<PathBuf, AdapterError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| AdapterError::CompilerProtocol(format!("missing {field}")))
}

fn digest_file(path: &Path) -> Result<String, AdapterError> {
    Ok(blake3::hash(&fs::read(path).map_err(AdapterError::Io)?).to_hex().to_string())
}

/// Adapter failure.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// File or process I/O failed.
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    /// JSON is malformed.
    #[error("JSON error: {0}")]
    Json(#[source] serde_json::Error),
    /// Installation proof failed.
    #[error("invalid compiler attestation: {0}")]
    Attestation(String),
    /// Compile request is outside the admitted profile.
    #[error("invalid compile request: {0}")]
    InvalidRequest(String),
    /// Compiler returned a nonzero status.
    #[error("CellScript compiler failed with status {status:?}: stdout={stdout}; stderr={stderr}")]
    CompilerFailure {
        /// Exit status, or `None` if terminated by a signal.
        status: Option<i32>,
        /// Machine-readable compiler diagnostics, which CellScript writes to stdout.
        stdout: String,
        /// Compiler diagnostic stream.
        stderr: String,
    },
    /// Compiler output violated the adapter contract.
    #[error("invalid CellScript compiler response: {0}")]
    CompilerProtocol(String),
    /// Compiler scheduling metadata or state-side conflict resolution is invalid.
    #[error("invalid scheduler binding: {0}")]
    SchedulerBinding(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_exec::{CellInput, OutPoint};

    fn transaction() -> CellTx {
        CellTx::new(vec![CellInput::new(OutPoint::new([7; 32], 0), 0)], vec![], vec![], vec![], vec![]).unwrap()
    }

    fn compiled_artifact(dir: &Path, metadata: serde_json::Value) -> CompiledArtifact {
        let artifact = dir.join("program.elf");
        let metadata_path = dir.join("program.elf.meta.json");
        fs::write(&artifact, b"elf").unwrap();
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        CompiledArtifact {
            compiler_version: "0.22.0".to_owned(),
            compiler_revision: "revision".to_owned(),
            metadata_schema_version: 55,
            source_metadata_schema_version: 1,
            artifact_metadata_schema_version: 1,
            constraints_metadata_schema_version: 1,
            source_blake3: "source".to_owned(),
            artifact_blake3: digest_file(&artifact).unwrap(),
            metadata_blake3: digest_file(&metadata_path).unwrap(),
            artifact,
            metadata: metadata_path,
            target_profile: "ckb".to_owned(),
        }
    }

    #[test]
    fn scheduler_template_requires_state_side_conflict_resolution() {
        let tx = transaction();
        let dir = tempfile::tempdir().unwrap();
        let compiled = compiled_artifact(
            dir.path(),
            serde_json::json!({
                "compiler_version": "0.22.0",
                "metadata_schema_version": 55,
                "source_metadata_schema_version": 1,
                "artifact_metadata_schema_version": 1,
                "constraints_metadata_schema_version": 1,
                "target_profile": { "name": "ckb" },
                "actions": [{
                    "name": "check",
                    "effect_class": "ReadOnly",
                    "parallelizable": true,
                    "estimated_cycles": 10,
                    "scheduler_witness_hex": "ignored-v1-binding-hash",
                    "ckb_runtime_accesses": [{
                        "operation": "read_ref",
                        "syscall": "LOAD_CELL",
                        "source": "Input",
                        "index": 0,
                        "binding": "receipt"
                    }]
                }]
            }),
        );
        let template = compiled.scheduler_template("check").unwrap();
        assert_eq!(template.accesses()[0].binding(), "receipt");
        let plan = template
            .bind(&tx, |_, access| {
                assert_eq!(access.source(), CompilerSchedulerSource::Input);
                Ok([9; 32])
            })
            .unwrap();
        assert_eq!(plan.txid(), tx.id());
        assert!(plan.accesses().contains(&([9; 32], AccessMode::Read)));
    }

    #[test]
    fn scheduler_template_rejects_mutated_metadata_and_out_of_bounds_access() {
        let tx = transaction();
        let dir = tempfile::tempdir().unwrap();
        let metadata = serde_json::json!({
            "compiler_version": "0.22.0",
            "metadata_schema_version": 55,
            "source_metadata_schema_version": 1,
            "artifact_metadata_schema_version": 1,
            "constraints_metadata_schema_version": 1,
            "target_profile": { "name": "ckb" },
            "actions": [{
                "name": "update",
                "effect_class": "Mutating",
                "parallelizable": false,
                "estimated_cycles": 20,
                "ckb_runtime_accesses": [{
                    "operation": "consume", "syscall": "LOAD_CELL", "source": "Input", "index": 1, "binding": "state"
                }]
            }]
        });
        let compiled = compiled_artifact(dir.path(), metadata.clone());
        let template = compiled.scheduler_template("update").unwrap();
        assert!(template.bind(&tx, |_, _| Ok([7; 32])).is_err());

        fs::write(&compiled.metadata, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
        assert!(compiled.scheduler_template("update").is_err());
    }
}
