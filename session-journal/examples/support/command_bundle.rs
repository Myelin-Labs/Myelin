//! Example wiring for the existing bundle format and isolated runner ABI.
use super::{artifact_digest, config, file_digest, restore, AnyResult, Kind};
use myelin_consensus::CommitteeSigner;
use myelin_session::{Hash32, SessionStore};
use myelin_session_bundle::{runner::*, *};
use myelin_session_store_rocksdb::RocksSessionStore;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSource {
    pub path: String,
    pub source: PathBuf,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportArtifacts {
    pub build: PackedFiles,
    pub runtime: Vec<RuntimeSource>,
    pub limits: BundleLimits,
    pub work_budget: ArchiveBudget,
}
#[derive(Serialize)]
pub struct Exported {
    pub manifest: Hash32,
    /// Established by this source process and supplied plan, before export.
    /// It is a development-fixture identity, not official deployment approval.
    pub source_anchor: TrustAnchor,
    pub state_root: Hash32,
    pub blocks: usize,
    pub work: ArchiveUsage,
}

pub fn export(kind: Kind, root: &Path, input: &Path, destination: &Path) -> AnyResult<Exported> {
    if !cfg!(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))) {
        return Err("example archive replay requires supported Linux isolation".into());
    }
    if !root.join("canonical/CURRENT").is_file() {
        return Err("export requires an existing canonical store".into());
    }
    let mut bytes = vec![];
    fs::File::open(input)?.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("export artifacts JSON limit".into());
    }
    let supplied: ExportArtifacts = serde_json::from_slice(&bytes)?;
    supplied.limits.validate()?;
    let program = BlobId(artifact_digest()?);
    let cfg = config(kind, program.0);
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    if store.load_genesis(cfg.session_id)?.config != cfg {
        return Err("example program/profile differs from canonical genesis".into());
    }
    let source_count = supplied.build.artifacts.len();
    let mut sources: BTreeMap<_, _> = supplied.build.artifacts.into_iter().map(|a| (a.id, a)).collect();
    if sources.len() != source_count {
        return Err("duplicate build artifact identity".into());
    }
    if !sources.contains_key(&supplied.build.root) {
        return Err("build package root is missing".into());
    }
    let base = input.parent().unwrap_or(Path::new("."));
    for source in sources.values_mut() {
        if source.path.is_relative() {
            source.path = base.join(&source.path);
        }
    }
    sources.insert(program, ArtifactSource { id: program, path: std::env::current_exe()?, dependencies: vec![] });
    let mut files = vec![];
    for source in supplied.runtime {
        let path = if source.source.is_relative() { base.join(source.source) } else { source.source };
        let blob = BlobId(file_digest(&path)?);
        sources.entry(blob).or_insert(ArtifactSource { id: blob, path, dependencies: vec![] });
        files.push(RuntimeFile { path: source.path, blob });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let runtime = RuntimeLayout { files };
    let dependencies: std::collections::BTreeSet<_> = runtime.files.iter().map(|f| f.blob).collect();
    let dependencies: Vec<_> = dependencies.into_iter().collect();
    runtime.validate(&dependencies)?;
    let rules = serde_json::to_vec(&cfg.application_profile)?;
    let rules_id = BlobId::of(&rules);
    let scratch = tempfile::tempdir()?;
    let rules_path = scratch.path().join("rules.json");
    fs::write(&rules_path, rules)?;
    sources.insert(rules_id, ArtifactSource { id: rules_id, path: rules_path, dependencies: vec![] });
    let execution = ExecutionArtifacts {
        program,
        replayer: program,
        rules: rules_id,
        build_inputs: supplied.build.root,
        toolchain: supplied.build.root,
        runtime_dependencies: dependencies,
        target: format!("{}-unknown-linux-gnu", std::env::consts::ARCH),
        runner_abi: RUNNER_ABI.into(),
        runtime: Some(runtime),
    };
    let network = *blake3::hash(b"myelin:command-examples-development-network:v1").as_bytes();
    let source_anchor = TrustAnchor {
        network,
        session_id: cfg.session_id,
        genesis_commitment: cfg.commitment(),
        profile_commitment: cfg.application_profile.commitment(),
        execution_commitment: execution.commitment(),
    };
    let signer = CommitteeSigner::new("command-game-validator", [7; 32])?;
    let key: String = signer.ckb_public_key().iter().map(|b| format!("{b:02x}")).collect();
    let consensus_toml = format!(
        "kind = \"proof-of-authority\"\n[[proof_of_authority.authorities]]\nid = \"{}\"\npublic_key = \"{key}\"\n",
        signer.validator_id()
    );
    let plan = ExportPlan {
        remote_sessions: vec![],
        roots: vec![cfg.session_id],
        sessions: vec![ExportSession { session_id: cfg.session_id, network, consensus_toml, execution }],
        artifacts: sources.into_values().collect(),
        limits: supplied.limits,
    };
    let outcome = ArchiveTask::export(store.clone(), plan, destination.into())?
        .advance(supplied.work_budget, &myelin_session::RecoveryCancellation::default())?;
    let (output, usage) = match outcome {
        ArchiveOutcome::Complete { output, usage } => (output, usage),
        ArchiveOutcome::Paused(cursor) => return Err(format!("export work paused: {}", serde_json::to_string(&cursor)?).into()),
    };
    let ArchiveOutput::Exported(manifest) = *output else { unreachable!("export task result") };
    Ok(Exported {
        manifest: manifest.commitment()?,
        source_anchor,
        state_root: store.load_head(cfg.session_id)?.state_root,
        blocks: manifest.sessions[0].blocks.len(),
        work: usage,
    })
}

pub fn replay(kind: Kind) -> AnyResult<()> {
    let (store, session) = replayer_input()?;
    let cfg = config(kind, artifact_digest()?);
    if store.config(session)? != &cfg {
        return Err("historical example profile/program mismatch".into());
    }
    let receipt = store.replay_current_process(session, move |bytes, _| restore(&cfg, kind, bytes))?;
    println!("{}", serde_json::to_string(&replay_output(&store, session, receipt)?)?);
    Ok(())
}
