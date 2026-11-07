use crate::{
    store::{integrity, missing},
    *,
};
use myelin_session::{Hash32, SessionHead, SessionStore};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Only explicitly supplied local files are copied. Paths are export inputs,
/// never manifest paths interpreted by a downloader.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSource {
    pub id: BlobId,
    pub path: PathBuf,
    /// Content prerequisites such as a build package's immutable file chunks.
    /// These supplement, and cannot replace, protocol-derived causal edges.
    pub dependencies: Vec<BlobId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportSession {
    pub session_id: Hash32,
    pub network: Hash32,
    pub consensus_toml: String,
    pub execution: ExecutionArtifacts,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPlan {
    pub roots: Vec<Hash32>,
    /// Complete catalog, including sources, successors and opaque app-owned
    /// restore dependencies. Every entry is exported and separately recognized.
    pub sessions: Vec<ExportSession>,
    /// Explicit artifact catalog for independently stored proof peers.
    pub remote_sessions: Vec<ExportSession>,
    pub artifacts: Vec<ArtifactSource>,
    #[serde(default)]
    pub limits: BundleLimits,
}
fn io(error: impl std::fmt::Display) -> BundleError {
    BundleError::new(ErrorCode::Io, error.to_string())
}

struct Writer {
    destination: PathBuf,
    limits: BundleLimits,
    blobs: BTreeMap<BlobId, Blob>,
    versions: BTreeSet<u16>,
    total: u64,
    serial: u64,
}
impl Writer {
    fn add(&mut self, bytes: &[u8], role: BlobKind) -> crate::Result<BlobId> {
        let id = BlobId::of(bytes);
        if matches!(
            role,
            BlobKind::Genesis
                | BlobKind::Head
                | BlobKind::Checkpoint
                | BlobKind::Block
                | BlobKind::Handoff
                | BlobKind::HandoffImport
                | BlobKind::Evidence
        ) {
            if bytes.len() < 6 || &bytes[..4] != b"MREC" {
                return Err(integrity("exported record has no supported envelope"));
            }
            self.versions.insert(u16::from_le_bytes([bytes[4], bytes[5]]));
        }
        self.copy(id, &mut &bytes[..], bytes.len() as u64, role)?;
        Ok(id)
    }
    fn copy(&mut self, id: BlobId, input: &mut impl Read, length: u64, role: BlobKind) -> crate::Result<()> {
        crate::work::checkpoint(ArchiveStage::Export, Some(id.hex()), 0)?;
        if let Some(existing) = self.blobs.get_mut(&id) {
            if existing.bytes != length {
                return Err(integrity("inconsistent content length for a repeated digest"));
            }
            if !existing.roles.contains(&role) {
                existing.roles.push(role);
            }
            return Ok(());
        }
        if length == 0
            || length > self.limits.blob_bytes
            || self.blobs.len() >= self.limits.nodes
            || length > self.limits.total_bytes.saturating_sub(self.total)
        {
            return Err(BundleError::new(ErrorCode::Limit, "export blob/node/expanded-size budget exceeded"));
        }
        let directory = self.destination.join("blobs");
        let temporary = directory.join(format!(".pending-{}", self.serial));
        self.serial += 1;
        let mut output = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(io)?;
        let mut hash = blake3::Hasher::new();
        let mut left = length;
        let mut buffer = [0u8; 65536];
        while left != 0 {
            let cap = (left as usize).min(buffer.len());
            crate::work::io(cap as u64 * 2)?;
            let n = input.read(&mut buffer[..cap]).map_err(io)?;
            if n == 0 {
                return Err(integrity("export artifact was truncated"));
            }
            hash.update(&buffer[..n]);
            output.write_all(&buffer[..n]).map_err(io)?;
            left -= n as u64;
        }
        crate::work::io(1)?;
        if input.read(&mut buffer[..1]).map_err(io)? != 0 || hash.finalize().as_bytes() != &id.0 {
            return Err(integrity("export artifact length or digest changed"));
        }
        output.sync_all().map_err(io)?;
        // Both paths are in a newly created export directory. A hard link makes
        // publication no-clobber even if a conflicting member appeared meanwhile.
        fs::hard_link(&temporary, directory.join(id.hex())).map_err(io)?;
        fs::remove_file(&temporary).map_err(io)?;
        self.total += length;
        self.blobs.insert(id, Blob { id, bytes: length, roles: vec![role], dependencies: vec![] });
        Ok(())
    }
    fn artifact(&mut self, sources: &BTreeMap<BlobId, &ArtifactSource>, id: BlobId, role: BlobKind) -> crate::Result<()> {
        let mut pending = vec![id];
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            crate::work::io(0)?;
            if !visited.insert(id) {
                continue;
            }
            let source = sources.get(&id).ok_or_else(|| missing(format!("no artifact source for {}", id.hex())))?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NONBLOCK);
            }
            let mut file = options.open(&source.path).map_err(io)?;
            let metadata = file.metadata().map_err(io)?;
            if !metadata.is_file() {
                return Err(BundleError::new(ErrorCode::UnsafePath, "artifact source is not a regular file"));
            }
            self.copy(id, &mut file, metadata.len(), role)?;
            self.blobs.get_mut(&id).expect("copied above").dependencies = source.dependencies.clone();
            pending.extend(source.dependencies.iter().copied());
        }
        Ok(())
    }
}

// Reject malformed artifact graphs before creating an output directory or
// copying potentially large toolchain packages. The full combined graph is
// validated again after deriving the mandatory session-history edges.
fn export_execution(
    writer: &mut Writer,
    sources: &BTreeMap<BlobId, &ArtifactSource>,
    execution: &ExecutionArtifacts,
) -> crate::Result<()> {
    for (blob, role) in [
        (execution.program, BlobKind::Program),
        (execution.replayer, BlobKind::Replayer),
        (execution.rules, BlobKind::Rules),
        (execution.build_inputs, BlobKind::BuildInputs),
        (execution.toolchain, BlobKind::Toolchain),
    ] {
        writer.artifact(sources, blob, role)?;
    }
    for blob in &execution.runtime_dependencies {
        writer.artifact(sources, *blob, BlobKind::RuntimeDependency)?;
    }
    Ok(())
}

fn validate_artifacts(sources: &BTreeMap<BlobId, &ArtifactSource>, limits: &BundleLimits) -> crate::Result<()> {
    let index: BTreeMap<_, _> = sources.keys().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut remaining = vec![0usize; sources.len()];
    let mut dependents = vec![vec![]; sources.len()];
    let mut edge_count = 0usize;
    for (id, source) in sources {
        crate::work::io(0)?;
        if !source.dependencies.windows(2).all(|p| p[0] < p[1]) {
            return Err(BundleError::new(ErrorCode::Format, "artifact dependencies must be sorted and unique"));
        }
        edge_count = edge_count
            .checked_add(source.dependencies.len())
            .ok_or_else(|| BundleError::new(ErrorCode::Limit, "artifact edge count overflow"))?;
        if edge_count > limits.edges {
            return Err(BundleError::new(ErrorCode::Limit, "artifact dependency edge budget exceeded"));
        }
        let i = index[id];
        remaining[i] = source.dependencies.len();
        for dependency in &source.dependencies {
            crate::work::io(0)?;
            let j = index.get(dependency).ok_or_else(|| missing(format!("no artifact source for {}", dependency.hex())))?;
            dependents[*j].push(i);
        }
    }
    let mut queue: VecDeque<_> = remaining.iter().enumerate().filter(|(_, n)| **n == 0).map(|(i, _)| i).collect();
    let mut depths = vec![1usize; sources.len()];
    let mut visited = 0usize;
    while let Some(i) = queue.pop_front() {
        crate::work::io(0)?;
        visited += 1;
        if depths[i] > limits.dependency_depth {
            return Err(BundleError::new(ErrorCode::Limit, "artifact dependency depth budget exceeded"));
        }
        for j in &dependents[i] {
            depths[*j] = depths[*j].max(depths[i] + 1);
            remaining[*j] -= 1;
            if remaining[*j] == 0 {
                queue.push_back(*j);
            }
        }
    }
    if visited != sources.len() {
        return Err(BundleError::new(ErrorCode::DependencyCycle, "artifact content dependency cycle"));
    }
    Ok(())
}

/// Stream a complete, quiescent catalog into a new directory. No database keys,
/// WAL, delivery ACKs or authority secrets are copied. Pausing or failing before
/// publication leaves staging for diagnosis and no manifest.json. A sync error
/// after publication reports I/O failure with durability still unconfirmed.
/// Concurrent changes of captured heads/checkpoints/indexes fail explicitly.
pub fn export_store(store: Arc<dyn SessionStore>, plan: &ExportPlan, destination: impl AsRef<Path>) -> crate::Result<BundleManifest> {
    let ArchiveOutput::Exported(manifest) =
        ArchiveTask::export(store, plan.clone(), destination.as_ref().to_owned())?.default_step()?
    else {
        unreachable!("export task result")
    };
    Ok(manifest)
}
pub(crate) fn export_inner(store: &dyn SessionStore, plan: &ExportPlan, destination: &Path) -> crate::Result<BundleManifest> {
    plan.limits.validate()?;
    if plan.sessions.is_empty() || plan.sessions.len() > plan.limits.sessions || plan.artifacts.len() > plan.limits.nodes {
        return Err(BundleError::new(ErrorCode::Limit, "invalid export catalog size"));
    }
    let catalog: BTreeMap<_, _> = plan.sessions.iter().map(|s| (s.session_id, s)).collect();
    let remote_catalog: BTreeMap<_, _> = plan.remote_sessions.iter().map(|s| (s.session_id, s)).collect();
    let sources: BTreeMap<_, _> = plan.artifacts.iter().map(|s| (s.id, s)).collect();
    if remote_catalog.len() != plan.remote_sessions.len()
        || remote_catalog.len() > plan.limits.sessions
        || catalog.len() != plan.sessions.len()
        || sources.len() != plan.artifacts.len()
        || plan.roots.is_empty()
        || plan.roots.iter().any(|id| !catalog.contains_key(id))
    {
        return Err(BundleError::new(ErrorCode::Format, "duplicate catalog/artifact or absent root session"));
    }
    validate_artifacts(&sources, &plan.limits)?;
    fs::create_dir(destination).map_err(io)?;
    fs::create_dir(destination.join("blobs")).map_err(io)?;
    let mut writer = Writer {
        destination: destination.into(),
        limits: plan.limits.clone(),
        blobs: BTreeMap::new(),
        versions: BTreeSet::new(),
        total: 0,
        serial: 0,
    };
    let mut captured: BTreeMap<Hash32, (SessionHead, BlobId)> = BTreeMap::new();
    let mut handoffs = BTreeMap::new();
    let mut imports = BTreeMap::new();
    let mut used_remote_sessions = BTreeSet::new();
    let mut evidence = BTreeMap::new();
    let mut sessions = vec![];
    let mut total_blocks = 0usize;
    for (id, entry) in catalog {
        crate::work::checkpoint(ArchiveStage::Export, Some(BlobId(id).hex()), 0)?;
        crate::work::record_read()?;
        let genesis = store.load_genesis(id).map_err(integrity)?;
        crate::work::record_read()?;
        let head = store.load_head(id).map_err(integrity)?;
        crate::work::record_read()?;
        let checkpoint = store.load_checkpoint(id).map_err(integrity)?;
        if genesis.config.session_id != id
            || checkpoint.session_id != id
            || head.session_id != id
            || checkpoint.finalised_height != head.finalised_height
            || checkpoint.state_root != head.state_root
        {
            return Err(BundleError::new(ErrorCode::SourceChanged, "source head/checkpoint do not form one capture"));
        }
        let count = usize::try_from(head.next_height().map_err(integrity)?).map_err(integrity)?;
        total_blocks = total_blocks.checked_add(count).ok_or_else(|| integrity("export block count overflow"))?;
        if total_blocks > plan.limits.blocks {
            return Err(BundleError::new(ErrorCode::Limit, "export block budget exceeded"));
        }
        export_execution(&mut writer, &sources, &entry.execution)?;

        let genesis_blob = writer.add(&genesis.encode().map_err(integrity)?, BlobKind::Genesis)?;
        let head_blob = writer.add(&head.encode().map_err(integrity)?, BlobKind::Head)?;
        let checkpoint_blob = writer.add(&checkpoint.encode().map_err(integrity)?, BlobKind::Checkpoint)?;
        let consensus = writer.add(entry.consensus_toml.as_bytes(), BlobKind::Consensus)?;
        captured.insert(id, (head, checkpoint_blob));
        let mut blocks = Vec::with_capacity(count);
        let mut session_handoffs = BTreeSet::new();
        let mut remote_handoffs = BTreeMap::new();
        let mut remote_completions = BTreeMap::new();
        let mut session_evidence = BTreeSet::new();
        for height in 0..count as u64 {
            crate::work::checkpoint(ArchiveStage::Export, Some(format!("{}:{height}", BlobId(id).hex())), 0)?;
            crate::work::record_read()?;
            let record = store.load_block(id, height).map_err(integrity)?;
            let transition_artifacts = record
                .successor
                .as_ref()
                .map(|s| &s.transition)
                .map(|transition| transition.artifact_dependencies())
                .unwrap_or_default();
            for artifact in &transition_artifacts {
                writer.artifact(&sources, BlobId(*artifact), BlobKind::RuntimeDependency)?;
            }
            let block = writer.add(&record.encode().map_err(integrity)?, BlobKind::Block)?;
            // These files have no inventory slot. Register their actual source
            // edge before the preliminary manifest's reachability validation.
            let dependencies = &mut writer.blobs.get_mut(&block).expect("block inserted above").dependencies;
            dependencies.extend(transition_artifacts.into_iter().map(BlobId));
            dependencies.sort();
            dependencies.dedup();
            blocks.push(block);
            for action in &record.handoff_actions {
                if let myelin_session::HandoffAction::RemoteRetire { handoff_id, proof } = &action.action {
                    let registry = store.handoff_import_registry().ok_or_else(|| missing("remote completion peer trust absent"))?;
                    proof.verify(&registry, &genesis.config, &action.handoff).map_err(integrity)?;
                    let (_, target) = registry.configs(proof.import_policy).map_err(integrity)?;
                    let peer =
                        remote_catalog.get(&target.session_id).ok_or_else(|| missing("remote completion artifact catalog absent"))?;
                    used_remote_sessions.insert(target.session_id);
                    export_execution(&mut writer, &sources, &peer.execution)?;
                    let consensus = writer.add(peer.consensus_toml.as_bytes(), BlobKind::Consensus)?;
                    remote_completions.insert(
                        *handoff_id,
                        HandoffCompletionInventory {
                            handoff_id: *handoff_id,
                            retirement: block,
                            target: RemoteSessionInventory {
                                network: peer.network,
                                session_id: target.session_id,
                                genesis_commitment: target.commitment(),
                                profile: target.application_profile.clone(),
                                consensus,
                                execution: peer.execution.clone(),
                            },
                        },
                    );
                }
            }

            let handoff_ids = record
                .issued_handoffs
                .iter()
                .map(|h| h.id())
                .chain(record.consumed_handoff_ids.iter().copied())
                .chain(record.handoff_actions.iter().map(|a| a.action.handoff_id()));
            for handoff_id in handoff_ids {
                crate::work::record_read()?;
                let stored =
                    store.load_handoff(handoff_id).map_err(integrity)?.ok_or_else(|| missing("source handoff index absent"))?;
                let blob = writer.add(&stored.encode().map_err(integrity)?, BlobKind::Handoff)?;
                if handoffs.insert(handoff_id, blob).is_some_and(|old| old != blob) {
                    return Err(BundleError::new(ErrorCode::SourceChanged, "handoff changed during export"));
                }
                session_handoffs.insert(blob);
                if let Some(import) = store.load_handoff_import(handoff_id).map_err(integrity)? {
                    let inbox = import.inbox_record().map_err(integrity)?;
                    let source = myelin_session::SessionConfig::decode(&inbox.proof.source_config).map_err(integrity)?;
                    let source_entry =
                        remote_catalog.get(&source.session_id).ok_or_else(|| missing("external source artifact catalog absent"))?;
                    used_remote_sessions.insert(source.session_id);
                    export_execution(&mut writer, &sources, &source_entry.execution)?;
                    let consensus = writer.add(source_entry.consensus_toml.as_bytes(), BlobKind::Consensus)?;
                    let proof = writer.add(&inbox.encode().map_err(integrity)?, BlobKind::HandoffImport)?;
                    if imports.insert(handoff_id, proof).is_some_and(|old| old != proof) {
                        return Err(BundleError::new(ErrorCode::SourceChanged, "remote proof changed during export"));
                    }
                    remote_handoffs.insert(
                        proof,
                        HandoffImportInventory {
                            proof,
                            source: RemoteSessionInventory {
                                network: source_entry.network,
                                session_id: source.session_id,
                                genesis_commitment: source.commitment(),
                                profile: source.application_profile,
                                consensus,
                                execution: source_entry.execution.clone(),
                            },
                        },
                    );
                }
            }
            for message in &record.outbox {
                crate::work::record_read()?;
                if let Some(stored) = store.load_evidence(id, message.id).map_err(integrity)? {
                    let blob = writer.add(&stored.encode().map_err(integrity)?, BlobKind::Evidence)?;
                    evidence.insert((id, message.id), blob);
                    session_evidence.insert(blob);
                }
            }
        }
        sessions.push(SessionInventory {
            network: entry.network,
            session_id: id,
            genesis_commitment: genesis.config.commitment(),
            profile: genesis.config.application_profile,
            genesis: genesis_blob,
            consensus,
            head: head_blob,
            checkpoint: checkpoint_blob,
            blocks,
            handoffs: session_handoffs.into_iter().collect(),
            remote_handoffs: remote_handoffs.into_values().collect(),
            remote_completions: remote_completions.into_values().collect(),
            evidence: session_evidence.into_iter().collect(),
            execution: entry.execution.clone(),
        });
    }
    if used_remote_sessions.len() != remote_catalog.len() {
        return Err(missing("unused external source catalog entry"));
    }
    let publication_reads = captured
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(handoffs.len()))
        .and_then(|n| n.checked_add(evidence.len()))
        .and_then(|n| n.checked_add(imports.len()))
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| BundleError::new(ErrorCode::Limit, "publication record count overflow"))?;
    let check_source = || -> crate::Result<bool> {
        for (id, (head, checkpoint)) in &captured {
            if !crate::work::publication_check_active()? {
                return Ok(false);
            }
            let current_head = store.load_head(*id).map_err(integrity)?;
            if !crate::work::publication_check_active()? {
                return Ok(false);
            }
            if current_head != *head
                || BlobId::of(&store.load_checkpoint(*id).map_err(integrity)?.encode().map_err(integrity)?) != *checkpoint
            {
                return Err(BundleError::new(
                    ErrorCode::SourceChanged,
                    "source advanced during export; retry from a quiescent capture",
                ));
            }
        }
        for (id, blob) in &handoffs {
            if !crate::work::publication_check_active()? {
                return Ok(false);
            }
            let value = store.load_handoff(*id).map_err(integrity)?.ok_or_else(|| missing("handoff removed during export"))?;
            if BlobId::of(&value.encode().map_err(integrity)?) != *blob {
                return Err(BundleError::new(ErrorCode::SourceChanged, "handoff terminal state changed"));
            }
        }
        for ((id, message), blob) in &evidence {
            if !crate::work::publication_check_active()? {
                return Ok(false);
            }
            let value =
                store.load_evidence(*id, *message).map_err(integrity)?.ok_or_else(|| missing("evidence removed during export"))?;
            if BlobId::of(&value.encode().map_err(integrity)?) != *blob {
                return Err(BundleError::new(ErrorCode::SourceChanged, "evidence stage changed"));
            }
        }
        for (id, blob) in &imports {
            if !crate::work::publication_check_active()? {
                return Ok(false);
            }
            let value =
                store.load_handoff_import(*id).map_err(integrity)?.ok_or_else(|| missing("remote proof removed during export"))?;
            if BlobId::of(&value.inbox_record().map_err(integrity)?.encode().map_err(integrity)?) != *blob {
                return Err(BundleError::new(ErrorCode::SourceChanged, "remote proof changed during export"));
            }
        }
        Ok(true)
    };
    let mut roots = plan.roots.clone();
    roots.sort();
    roots.dedup();
    let mut manifest = BundleManifest {
        format: FORMAT.into(),
        myelin_version: env!("CARGO_PKG_VERSION").into(),
        record_versions: writer.versions.into_iter().collect(),
        roots,
        blobs: writer.blobs.into_values().collect(),
        sessions,
    };
    manifest.validate_structure(&plan.limits)?;
    let pending = destination.join("manifest.pending");
    let encoded = manifest.encode()?;
    crate::work::io(encoded.len() as u64)?;
    fs::write(&pending, encoded).map_err(io)?;
    let mut directory = BundleDirectory::open_named(destination, plan.limits.clone(), "manifest.pending")?;
    directory.remote_imports = store.handoff_import_registry();
    let directory = Arc::new(directory);
    let indexed = BundleStore::index(directory)?;
    for (id, dependencies) in indexed.required_edges()? {
        let i = manifest.blobs.binary_search_by_key(&id, |b| b.id).map_err(|_| missing("derived edge owner absent"))?;
        manifest.blobs[i].dependencies.extend(dependencies);
        manifest.blobs[i].dependencies.sort();
        manifest.blobs[i].dependencies.dedup();
    }
    manifest.validate_structure(&plan.limits)?;
    let encoded = manifest.encode()?;
    crate::work::io(encoded.len() as u64)?;
    fs::write(&pending, encoded).map_err(io)?;
    let mut directory = BundleDirectory::open_named(destination, plan.limits.clone(), "manifest.pending")?;
    directory.remote_imports = store.handoff_import_registry();
    let directory = Arc::new(directory);
    // Source-local internal consistency audit. These proposed anchors are NOT
    // exported as an independent trust file and establish no official identity.
    let mut anchors: Vec<_> = manifest.sessions.iter().map(SessionInventory::proposed_anchor).collect();
    anchors.extend(manifest.sessions.iter().flat_map(|s| {
        s.remote_handoffs
            .iter()
            .map(|r| r.source.proposed_anchor())
            .chain(s.remote_completions.iter().map(|r| r.target.proposed_anchor()))
    }));
    BundleStore::open(directory, &anchors)?;
    File::open(&pending).map_err(io)?.sync_all().map_err(io)?;
    File::open(destination.join("blobs")).map_err(io)?.sync_all().map_err(io)?;
    crate::work::checkpoint(ArchiveStage::Publish, None, 0)?;
    loop {
        crate::work::reserve_publication_reads(publication_reads)?;
        let generation = crate::work::generation();
        let complete = check_source()?;
        crate::work::before_publish()?;
        if complete && generation == crate::work::generation() {
            break;
        }
    }
    fs::hard_link(&pending, destination.join("manifest.json")).map_err(io)?;
    crate::work::published();
    fs::remove_file(pending).map_err(io)?;
    File::open(destination).map_err(io)?.sync_all().map_err(io)?;
    Ok(manifest)
}
