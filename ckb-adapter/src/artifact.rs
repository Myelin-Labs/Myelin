//! Language-independent handoff of an already compiled CKB program.

use myelin_exec::{ckb_cell_data_hash, CellDep};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};

/// Current-only artifact manifest. Provenance and ABI labels do not prove program correctness.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractArtifact {
    /// Manifest version; only 1 is accepted.
    pub version: u32,
    /// Execution semantics; this boundary accepts `ckb-strict-vm2`.
    pub execution_profile: String,
    /// Program format; currently `riscv64-elf`.
    pub format: String,
    /// Program path relative to the manifest directory, without parent components.
    pub artifact: PathBuf,
    /// SHA-256 of the exact program bytes, in 0x-prefixed hex.
    pub artifact_sha256: String,
    /// CKB-personalized Blake2b-256 of those bytes (Data2 code identity).
    pub code_hash: String,
    /// Build producer identity, recorded as provenance rather than a compiler allowlist.
    pub compiler_identity: String,
    /// Optional source digest; consuming a binary never requires source or a compiler.
    pub source_sha256: Option<String>,
    /// Application-defined entry name.
    pub entry: String,
    /// Application-defined witness ABI. Callers supply encoded witness bytes.
    pub witness_abi: String,
    /// Additional deployment dependencies required in the consuming transaction.
    pub dependencies: Vec<CellDep>,
}

/// A malformed or mismatched contract artifact.
#[derive(Debug, thiserror::Error)]
#[error("invalid contract artifact: {0}")]
pub struct ArtifactError(String);

fn invalid(message: impl Into<String>) -> ArtifactError {
    ArtifactError(message.into())
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, ArtifactError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| invalid(format!("{}: {e}", path.display())))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| invalid(e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(invalid("file exceeds size limit"));
    }
    Ok(bytes)
}

impl ContractArtifact {
    /// Read a bounded manifest and verify its program before submission.
    /// The caller adds the declared dependencies to the consuming transaction.
    pub fn load(path: &Path, expected_code_hash: &str) -> Result<Self, ArtifactError> {
        let manifest: Self = serde_json::from_slice(&bounded_read(path, 65_536)?).map_err(|e| invalid(e.to_string()))?;
        if manifest.artifact.as_os_str().is_empty() || manifest.artifact.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(invalid("artifact path must be relative without parent components"));
        }
        let program = bounded_read(&path.parent().unwrap_or(Path::new(".")).join(&manifest.artifact), 4 * 1024 * 1024)?;
        manifest.verify(&program, expected_code_hash, &manifest.dependencies)?;
        Ok(manifest)
    }

    /// Verify exact program identity and dependency inclusion without invoking a compiler.
    pub fn verify(&self, program: &[u8], expected_code_hash: &str, transaction_deps: &[CellDep]) -> Result<(), ArtifactError> {
        if self.version != 1 || self.execution_profile != "ckb-strict-vm2" || self.format != "riscv64-elf" {
            return Err(invalid("unsupported version, execution profile or program format"));
        }
        if program.len() < 64 || program.len() > 4 * 1024 * 1024 || &program[..6] != b"\x7fELF\x02\x01" || program[18..20] != [243, 0]
        {
            return Err(invalid("expected a little-endian RISC-V ELF64 program"));
        }
        for label in [&self.compiler_identity, &self.entry, &self.witness_abi] {
            if label.is_empty() || label.len() > 1024 {
                return Err(invalid("empty or oversized provenance/ABI label"));
            }
        }
        let sha = format!("0x{}", hex::encode(myelin_hashes::sha256(program)));
        let code = format!("0x{}", hex::encode(ckb_cell_data_hash(program)));
        if self.artifact_sha256 != sha || self.code_hash != code || !code.eq_ignore_ascii_case(expected_code_hash) {
            return Err(invalid("program digest or deployment code hash mismatch"));
        }
        if self.source_sha256.as_ref().is_some_and(|s| s.len() != 66 || !s.starts_with("0x") || hex::decode(&s[2..]).is_err()) {
            return Err(invalid("malformed source SHA-256"));
        }
        if self.dependencies.len() > 128
            || self.dependencies.iter().enumerate().any(|(i, d)| self.dependencies[..i].contains(d) || !transaction_deps.contains(d))
        {
            return Err(invalid("missing or duplicate artifact dependency"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_exec::{DepType, OutPoint};

    #[test]
    fn binary_handoff_binds_program_profile_and_dependencies_without_source() {
        let mut program = vec![0; 64];
        program[..6].copy_from_slice(b"\x7fELF\x02\x01");
        program[18] = 243;
        let dependency = CellDep { out_point: OutPoint::new([7; 32], 1), dep_type: DepType::Code };
        let mut manifest = ContractArtifact {
            version: 1,
            execution_profile: "ckb-strict-vm2".into(),
            format: "riscv64-elf".into(),
            artifact: "program.elf".into(),
            artifact_sha256: format!("0x{}", hex::encode(myelin_hashes::sha256(&program))),
            code_hash: format!("0x{}", hex::encode(ckb_cell_data_hash(&program))),
            compiler_identity: "independent fixture producer".into(),
            source_sha256: None,
            entry: "main".into(),
            witness_abi: "counter-v1".into(),
            dependencies: vec![dependency.clone()],
        };
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("program.elf"), &program).unwrap();
        let path = directory.path().join("manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(ContractArtifact::load(&path, &manifest.code_hash).unwrap(), manifest);
        assert!(manifest.verify(&program, &manifest.code_hash, &[]).is_err());
        assert!(manifest.verify(&program, &format!("0x{}", "00".repeat(32)), std::slice::from_ref(&dependency)).is_err());
        program[63] ^= 1;
        assert!(manifest.verify(&program, &manifest.code_hash, std::slice::from_ref(&dependency)).is_err());
        program[63] ^= 1;
        manifest.execution_profile = "application-extended".into();
        assert!(manifest.verify(&program, &manifest.code_hash, &[dependency]).is_err());
    }
}
