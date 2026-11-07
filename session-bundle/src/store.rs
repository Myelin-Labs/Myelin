use crate::*;
use myelin_consensus::ConsensusConfig;
use myelin_session::*;
use myelin_session_runtime::RegisteredFinalityVerifier;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
};

const RECORD_CAP: u64 = bounded_codec::MAX_RECORD_BYTES as u64;
const INDEX_CAP: usize = 1_048_576;
pub(crate) type Edges = BTreeMap<BlobId, BTreeSet<BlobId>>;

pub(crate) fn integrity(error: impl std::fmt::Display) -> BundleError {
    BundleError::new(ErrorCode::InvalidHistory, error.to_string())
}
pub(crate) fn missing(message: impl Into<String>) -> BundleError {
    BundleError::new(ErrorCode::MissingDependency, message)
}
fn backend(error: BundleError) -> StoreError {
    match error.code {
        ErrorCode::MissingArtifact | ErrorCode::MissingDependency => StoreError::NotFound(error.to_string()),
        _ => StoreError::Corrupt(error.to_string()),
    }
}
fn readonly<T>() -> std::result::Result<T, StoreError> {
    Err(StoreError::Backend("bundle store is read-only; no authority, ACK or write rights are imported".into()))
}
#[cfg(target_os = "linux")]
fn running_digest(length: u64) -> crate::Result<Hash32> {
    use std::io::Read;
    let file = std::fs::File::open("/proc/self/exe").map_err(integrity)?;
    if file.metadata().map_err(integrity)?.len() != length {
        return Err(integrity("current executable length differs from the pinned historical replayer"));
    }
    let mut input = file.take(length + 1);
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    let mut seen = 0u64;
    loop {
        let n = input.read(&mut buffer).map_err(integrity)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        seen += n as u64;
    }
    if seen != length {
        return Err(integrity("running executable changed during identity check"));
    }
    Ok(*hash.finalize().as_bytes())
}
#[cfg(not(target_os = "linux"))]
fn running_digest(_: u64) -> crate::Result<Hash32> {
    Err(BundleError::new(ErrorCode::UnsupportedPlatform, "current-process artifact pinning requires Linux /proc/self/exe"))
}

struct SessionIndex {
    inventory: SessionInventory,
    config: SessionConfig,
    verifier: Arc<RegisteredFinalityVerifier>,
    checkpoints: BTreeSet<u64>,
}

/// All index entries are reconstructed from content-checked immutable records.
/// Read methods recheck bytes; no block/state history is eagerly held in memory.
pub struct BundleStore {
    directory: Arc<BundleDirectory>,
    sessions: BTreeMap<Hash32, SessionIndex>,
    commands: BTreeMap<(Hash32, Hash32), (u64, usize)>,
    handoffs: BTreeMap<Hash32, BlobId>,
    remote_handoffs: BTreeMap<Hash32, crate::HandoffImportInventory>,
    remote_completions: BTreeMap<Hash32, crate::HandoffCompletionInventory>,
    evidence: BTreeMap<(Hash32, Hash32), (BlobId, u64)>,
}

/// Verified bundle catalogs used by a live store's explicit cold tier. Entries
/// cannot be evicted through this interface. Persisted RocksDB archive pins
/// require the same manifests/dependency closures after restart. Registration
/// is not an application-reexecution or external availability guarantee.
#[derive(Default)]
pub struct BundleArchives {
    stores: RwLock<BTreeMap<Hash32, Arc<BundleStore>>>,
}
impl BundleArchives {
    pub(crate) fn catalog(&self, id: Hash32) -> crate::Result<Arc<BundleStore>> {
        self.stores
            .read()
            .map_err(|_| integrity("archive registry lock poisoned"))?
            .get(&id)
            .cloned()
            .ok_or_else(|| missing("required archive catalog is not registered"))
    }
    pub fn register(&self, store: Arc<BundleStore>) -> crate::Result<Hash32> {
        let id = store.manifest().commitment()?;
        let mut stores = self.stores.write().map_err(|_| integrity("archive registry lock poisoned"))?;
        if !stores.contains_key(&id) && stores.len() >= 1024 {
            return Err(BundleError::new(ErrorCode::Limit, "archive registry exceeds 1024 manifests"));
        }
        // A fresh, audited copy can repair the location of the same immutable
        // manifest. In-flight readers retain their Arc to the previous copy.
        stores.insert(id, store);
        Ok(id)
    }
    pub fn reference(
        &self,
        archive: Hash32,
        session: Hash32,
        height: u64,
    ) -> crate::Result<myelin_session_store_rocksdb::ArchiveReference> {
        let stores = self.stores.read().map_err(|_| integrity("archive registry lock poisoned"))?;
        let store = stores.get(&archive).ok_or_else(|| missing("archive manifest is not registered"))?;
        let blob = store.block_id(session, height)?;
        let bytes = store.directory.descriptor(blob)?.bytes;
        Ok(myelin_session_store_rocksdb::ArchiveReference { archive, blob: blob.0, bytes })
    }
}
impl myelin_session_store_rocksdb::ArchiveRecordSource for BundleArchives {
    fn read_record(&self, archive: Hash32, blob: Hash32, expected_bytes: u64) -> std::result::Result<Vec<u8>, StoreError> {
        let store = self
            .stores
            .read()
            .map_err(|_| StoreError::Backend("archive registry lock poisoned".into()))?
            .get(&archive)
            .cloned()
            .ok_or_else(|| StoreError::NotFound("required archive manifest is not registered".into()))?;
        let id = BlobId(blob);
        let descriptor = store.directory.descriptor(id).map_err(backend)?;
        if descriptor.bytes != expected_bytes || !descriptor.roles.contains(&BlobKind::Block) {
            return Err(StoreError::Corrupt("cold reference is not the expected block blob".into()));
        }
        store.directory.read_blob(id, expected_bytes.min(RECORD_CAP)).map_err(backend)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionAudit {
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub profile_commitment: Hash32,
    pub consensus_module_commitment: Hash32,
    pub consensus_config_commitment: Hash32,
    pub finalised_height: Option<u64>,
    pub block_hash: Hash32,
    pub state_root: Hash32,
}
#[derive(Clone, Debug, Serialize)]
pub struct FinalityReport {
    pub stages: InventoryReport,
    pub sessions: Vec<SessionAudit>,
    pub mandatory_dependency_edges: usize,
    /// Consensus proofs are not the separate gateway's epoch/membership proofs.
    pub canonical_authority_membership: VerificationStage,
}

impl BundleStore {
    pub(crate) fn directory(&self) -> &BundleDirectory {
        &self.directory
    }
    pub fn open(directory: Arc<BundleDirectory>, anchors: &[TrustAnchor]) -> crate::Result<(Arc<Self>, FinalityReport)> {
        if !crate::work::active() {
            let ArchiveOutput::Verified { store, report } = ArchiveTask::verify(directory, anchors.to_vec())?.default_step()? else {
                unreachable!("verify task result")
            };
            return Ok((store, report));
        }
        let mut stages = directory.verify_inventory(anchors)?;
        let store = Arc::new(Self::index(directory)?);
        let edges = store.required_edges()?;
        for (id, deps) in &edges {
            crate::work::io(0)?;
            let declared = &store.directory.descriptor(*id)?.dependencies;
            if let Some(absent) = deps.iter().find(|d| !declared.contains(d)) {
                return Err(missing(format!("mandatory causal edge {} -> {} is absent", id.hex(), absent.hex())));
            }
        }
        let mut sessions = vec![];
        for (id, index) in &store.sessions {
            crate::work::checkpoint(ArchiveStage::Finality, Some(BlobId(*id).hex()), 0)?;
            // The archive worker's I/O boundaries enforce the caller's global
            // budget. Core audit cursors preserve work across their own slices;
            // parking time must not cause an otherwise valid bundle to fail.
            let mut outcome = RecoveryAudit::new(*id, index.config.application_profile.clone(), index.verifier.clone(), store.clone())
                .advance(RecoveryBudget::default(), &RecoveryCancellation::default())
                .map_err(integrity)?;
            let audited = loop {
                match outcome {
                    RecoveryOutcome::Complete(audited) => break audited,
                    RecoveryOutcome::Paused(cursor) => {
                        crate::work::checkpoint(
                            ArchiveStage::Finality,
                            Some(format!("{}:{}", BlobId(*id).hex(), cursor.progress().next_height)),
                            0,
                        )?;
                        outcome = cursor.resume(RecoveryBudget::default(), &RecoveryCancellation::default()).map_err(integrity)?;
                    }
                }
            };
            let head = audited.head();
            sessions.push(SessionAudit {
                session_id: *id,
                genesis_commitment: index.config.commitment(),
                profile_commitment: index.config.application_profile.commitment(),
                consensus_module_commitment: index.config.consensus_module_commitment,
                consensus_config_commitment: index.config.consensus_config_commitment,
                finalised_height: head.finalised_height,
                block_hash: head.block_hash,
                state_root: head.state_root,
            });
        }
        stages.finality = VerificationStage::Verified;
        Ok((
            store,
            FinalityReport {
                stages,
                sessions,
                mandatory_dependency_edges: edges.values().map(BTreeSet::len).sum(),
                canonical_authority_membership: VerificationStage::NotRun,
            },
        ))
    }
    pub fn manifest(&self) -> &BundleManifest {
        self.directory.manifest()
    }
    /// Load a declared artifact for a historical application's rules or migration
    /// interpreter. Every call bounds allocation and rechecks the exact content
    /// digest; a prior catalog audit does not authorize changed artifact bytes.
    /// Reading bytes neither executes them nor establishes application validity.
    pub fn read_artifact(&self, id: BlobId, max_bytes: u64) -> crate::Result<Vec<u8>> {
        self.directory.read_blob(id, max_bytes)
    }
    pub fn config(&self, session: Hash32) -> crate::Result<&SessionConfig> {
        Ok(&self.session(session)?.config)
    }
    fn session(&self, id: Hash32) -> crate::Result<&SessionIndex> {
        self.sessions.get(&id).ok_or_else(|| missing(format!("session {} is absent", BlobId(id).hex())))
    }
    fn block_id(&self, session: Hash32, height: u64) -> crate::Result<BlobId> {
        usize::try_from(height)
            .ok()
            .and_then(|h| self.sessions.get(&session)?.inventory.blocks.get(h).copied())
            .ok_or_else(|| missing(format!("source block {}:{height} is absent", BlobId(session).hex())))
    }
    fn bytes(&self, id: BlobId) -> crate::Result<Vec<u8>> {
        let bytes = self.directory.read_blob(id, RECORD_CAP)?;
        if bytes.len() < 6
            || &bytes[..4] != b"MREC"
            || !self.manifest().record_versions.contains(&u16::from_le_bytes([bytes[4], bytes[5]]))
        {
            return Err(integrity("record envelope version is absent from the manifest"));
        }
        Ok(bytes)
    }
    fn record(&self, session: Hash32, height: u64) -> crate::Result<FinalisedBlockRecord> {
        let bytes = self.bytes(self.block_id(session, height)?)?;
        FinalisedBlockRecord::decode_for_config(&bytes, &self.session(session)?.config).map_err(integrity)
    }
    fn handoff(&self, id: Hash32) -> crate::Result<StoredHandoff> {
        let blob = *self.handoffs.get(&id).ok_or_else(|| missing("handoff terminal index is absent"))?;
        StoredHandoff::decode(&self.bytes(blob)?).map_err(integrity)
    }
    pub(crate) fn index(directory: Arc<BundleDirectory>) -> crate::Result<Self> {
        crate::work::checkpoint(ArchiveStage::Index, None, 0)?;
        let mut sessions = BTreeMap::new();
        for inventory in &directory.manifest().sessions {
            crate::work::checkpoint(ArchiveStage::Index, Some(BlobId(inventory.session_id).hex()), 0)?;
            let genesis = SessionGenesis::decode(&directory.read_blob(inventory.genesis, RECORD_CAP)?).map_err(integrity)?;
            if genesis.config.session_id != inventory.session_id
                || genesis.config.commitment() != inventory.genesis_commitment
                || genesis.config.application_profile != inventory.profile
            {
                return Err(integrity("genesis/profile identity differs from inventory"));
            }
            let bytes = directory.read_blob(inventory.consensus, 1024 * 1024)?;
            let config = ConsensusConfig::from_toml_str(std::str::from_utf8(&bytes).map_err(integrity)?).map_err(integrity)?;
            let verifier = Arc::new(RegisteredFinalityVerifier::new(config).map_err(integrity)?);
            let d = verifier.descriptor();
            let cfg = &genesis.config;
            if d.commitment() != cfg.consensus_module_commitment
                || d.config_commitment != cfg.consensus_config_commitment
                || d.wal_schema_hash != cfg.consensus_wal_schema_hash
                || d.consensus_kind != cfg.consensus_kind
            {
                return Err(integrity("consensus authority/configuration differs from genesis"));
            }
            let head = SessionHead::decode(&directory.read_blob(inventory.head, RECORD_CAP)?).map_err(integrity)?;
            if head.session_id != inventory.session_id || head.next_height().map_err(integrity)? != inventory.blocks.len() as u64 {
                return Err(integrity("head height differs from full genesis-prefix block inventory"));
            }
            sessions.insert(
                inventory.session_id,
                SessionIndex { inventory: inventory.clone(), config: genesis.config, verifier, checkpoints: BTreeSet::new() },
            );
        }
        let mut store = Self {
            directory,
            sessions,
            commands: BTreeMap::new(),
            handoffs: BTreeMap::new(),
            remote_handoffs: BTreeMap::new(),
            remote_completions: BTreeMap::new(),
            evidence: BTreeMap::new(),
        };
        for session in store.sessions.values() {
            for blob in &session.inventory.handoffs {
                let handoff = StoredHandoff::decode(&store.bytes(*blob)?).map_err(integrity)?;
                if store.handoffs.insert(handoff.handoff.id(), *blob).is_some_and(|old| old != *blob) {
                    return Err(integrity("conflicting terminal records for the same handoff"));
                }
            }
        }
        for session in store.sessions.values() {
            for inventory in &session.inventory.remote_handoffs {
                let record = myelin_session::HandoffInboxRecord::decode(&store.bytes(inventory.proof)?).map_err(integrity)?;
                let registry = store.directory.remote_imports.as_ref().ok_or_else(|| {
                    BundleError::new(
                        ErrorCode::UnknownTrustAnchor,
                        "remote archive requires independently configured import verifiers",
                    )
                })?;
                let verified = registry.verify(&record).map_err(integrity)?;
                let source = myelin_session::SessionConfig::decode(&record.proof.source_config).map_err(integrity)?;
                if verified.policy().target_genesis != session.config.commitment()
                    || source.session_id != inventory.source.session_id
                    || source.commitment() != inventory.source.genesis_commitment
                    || source.application_profile != inventory.source.profile
                {
                    return Err(integrity("external proof differs from source/target inventory"));
                }
                let consensus = store.directory.read_blob(inventory.source.consensus, 1024 * 1024)?;
                let config = ConsensusConfig::from_toml_str(std::str::from_utf8(&consensus).map_err(integrity)?).map_err(integrity)?;
                let verifier = RegisteredFinalityVerifier::new(config).map_err(integrity)?;
                if verifier.descriptor().commitment() != source.consensus_module_commitment {
                    return Err(integrity("external source consensus artifact differs from genesis"));
                }
                let id = verified.handoff().id();
                if store.remote_handoffs.insert(id, inventory.clone()).is_some() {
                    return Err(integrity("duplicate remote handoff inventory"));
                }
            }
        }
        let ids: Vec<_> = store.sessions.keys().copied().collect();
        for id in ids {
            let inventory = store.session(id)?.inventory.clone();
            let mut expected_handoffs = BTreeSet::new();
            let mut expected_completions = BTreeSet::new();
            for completion in &inventory.remote_completions {
                let bytes = store.bytes(completion.retirement)?;
                let retirement = FinalisedBlockRecord::decode_for_config(&bytes, &store.session(id)?.config).map_err(integrity)?;
                let action = retirement
                    .handoff_actions
                    .iter()
                    .find(|action| action.action.handoff_id() == completion.handoff_id)
                    .ok_or_else(|| missing("remote retirement action absent"))?;
                let HandoffAction::RemoteRetire { proof, .. } = &action.action else {
                    return Err(integrity("completion inventory names another action"));
                };
                let registry = store.directory.remote_imports.as_ref().ok_or_else(|| {
                    BundleError::new(ErrorCode::UnknownTrustAnchor, "remote completion needs independent peer verifiers")
                })?;
                proof.verify(registry, &store.session(id)?.config, &action.handoff).map_err(integrity)?;
                let (_, target) = registry.configs(proof.import_policy).map_err(integrity)?;
                if target.session_id != completion.target.session_id
                    || target.commitment() != completion.target.genesis_commitment
                    || target.application_profile != completion.target.profile
                {
                    return Err(integrity("completion target inventory differs from local trust"));
                }
                let consensus = store.directory.read_blob(completion.target.consensus, 1024 * 1024)?;
                let config = ConsensusConfig::from_toml_str(std::str::from_utf8(&consensus).map_err(integrity)?).map_err(integrity)?;
                if RegisteredFinalityVerifier::new(config).map_err(integrity)?.descriptor().commitment()
                    != target.consensus_module_commitment
                {
                    return Err(integrity("completion consensus artifact differs from target genesis"));
                }
                if store.remote_completions.insert(completion.handoff_id, completion.clone()).is_some() {
                    return Err(integrity("duplicate remote completion"));
                }
            }

            let mut outbox = BTreeMap::new();
            for height in 0..inventory.blocks.len() as u64 {
                crate::work::checkpoint(ArchiveStage::Index, Some(format!("{}:{height}", BlobId(id).hex())), 0)?;
                let record = store.record(id, height)?;
                if record.block.session_id != id || record.block.number != height {
                    return Err(integrity("block inventory order/session mismatch"));
                }
                if !record.state_snapshot.is_empty() {
                    store.sessions.get_mut(&id).expect("indexed session").checkpoints.insert(height);
                }
                if record.frame_input.commands.len() > INDEX_CAP.saturating_sub(store.commands.len())
                    || record.outbox.len() > INDEX_CAP.saturating_sub(outbox.len())
                {
                    return Err(BundleError::new(ErrorCode::Limit, "reconstructed command/outbox index exceeds 1048576 entries"));
                }
                for (offset, command) in record.frame_input.commands.iter().enumerate() {
                    crate::work::io(0)?;
                    if store.commands.insert((id, command.id), (height, offset)).is_some() {
                        return Err(integrity("duplicate command identity in archived history"));
                    }
                }
                for action in &record.handoff_actions {
                    if matches!(action.action, HandoffAction::RemoteRetire { .. }) {
                        expected_completions.insert(action.action.handoff_id());
                        let declared = store
                            .remote_completions
                            .get(&action.action.handoff_id())
                            .ok_or_else(|| missing("remote completion inventory absent"))?;
                        if declared.retirement != inventory.blocks[height as usize] {
                            return Err(integrity("completion names another retirement block"));
                        }
                    }
                }
                expected_handoffs.extend(record.issued_handoffs.iter().map(SessionHandoff::id));
                expected_handoffs.extend(record.consumed_handoff_ids);
                expected_handoffs.extend(record.handoff_actions.iter().map(|a| a.action.handoff_id()));
                for message in record.outbox {
                    crate::work::io(0)?;
                    if outbox.insert(message.id, height).is_some() {
                        return Err(integrity("duplicate outbox identity"));
                    }
                }
            }
            if expected_completions != inventory.remote_completions.iter().map(|completion| completion.handoff_id).collect() {
                return Err(integrity("completion inventory differs from actual source actions"));
            }
            let declared: BTreeSet<_> = inventory.handoffs.iter().copied().collect();
            let required: BTreeSet<_> = expected_handoffs
                .iter()
                .map(|id| store.handoffs.get(id).copied().ok_or_else(|| missing("block's required handoff index is absent")))
                .collect::<crate::Result<_>>()?;
            if declared != required {
                return Err(integrity("session handoff inventory differs from its actual blocks"));
            }
            let declared_remote: BTreeSet<_> = inventory.remote_handoffs.iter().map(|r| r.proof).collect();
            let required_remote: BTreeSet<_> =
                expected_handoffs.iter().filter_map(|id| store.remote_handoffs.get(id).map(|r| r.proof)).collect();
            if declared_remote != required_remote {
                return Err(integrity("external proof inventory differs from actual handoff dependencies"));
            }
            for blob in &inventory.evidence {
                let record = EvidenceRecord::decode(&store.bytes(*blob)?).map_err(integrity)?;
                let height = outbox.get(&record.message_id).ok_or_else(|| missing("evidence outbox message absent"))?;
                let block = store.record(id, *height)?;
                let message = block
                    .outbox
                    .iter()
                    .find(|m| m.id == record.message_id)
                    .ok_or_else(|| missing("evidence outbox message changed"))?;
                record.validate_for(id, message, &record.descriptor).map_err(integrity)?;
                if store.evidence.insert((id, record.message_id), (*blob, *height)).is_some() {
                    return Err(integrity("duplicate evidence record"));
                }
            }
        }
        Ok(store)
    }
    /// Derive mandatory forward causal edges from actual records. Reverse
    /// ownership links (source -> successor or source -> terminal index) are
    /// checked by recovery but do not become causality cycles.
    pub(crate) fn required_edges(&self) -> crate::Result<Edges> {
        crate::work::checkpoint(ArchiveStage::Dependencies, None, 0)?;
        let mut edges: Edges = BTreeMap::new();
        for (id, entry) in &self.sessions {
            crate::work::io(0)?;
            let s = &entry.inventory;
            edges.entry(s.genesis).or_default().extend([
                s.consensus,
                s.execution.program,
                s.execution.replayer,
                s.execution.rules,
                s.execution.build_inputs,
                s.execution.toolchain,
            ]);
            edges.entry(s.genesis).or_default().extend(s.execution.runtime_dependencies.iter().copied());
            if let Some(p) = &entry.config.predecessor {
                edges.entry(s.genesis).or_default().insert(self.block_id(p.session_id, p.final_height)?);
            }
            edges.entry(s.head).or_default().insert(s.blocks.last().copied().unwrap_or(s.genesis));
            edges.entry(s.checkpoint).or_default().insert(s.head);
            for (height, blob) in s.blocks.iter().enumerate() {
                let record = self.record(*id, height as u64)?;
                let deps = edges.entry(*blob).or_default();
                deps.insert(s.genesis);
                if height > 0 {
                    deps.insert(s.blocks[height - 1]);
                }
                for handoff in &record.consumed_handoff_ids {
                    crate::work::io(0)?;
                    let h = self.handoff(*handoff)?.handoff;
                    if let Some(import) = self.remote_handoffs.get(handoff) {
                        deps.insert(import.proof);
                    } else {
                        deps.insert(self.block_id(h.source_session_id, h.source_height)?);
                        if let Some(requirement) = h.evidence_requirement {
                            let (blob, _) = self
                                .evidence
                                .get(&(h.source_session_id, requirement.message_id))
                                .ok_or_else(|| missing("consumed handoff's required evidence is absent"))?;
                            let evidence = EvidenceRecord::decode(&self.bytes(*blob)?).map_err(integrity)?;
                            if evidence.descriptor.commitment() != requirement.pipeline_commitment
                                || evidence.receipts.len() <= requirement.minimum_stage_index as usize
                            {
                                return Err(integrity("consumed handoff's required evidence stage differs"));
                            }
                            deps.insert(*blob);
                        }
                    }
                }
                for action in &record.handoff_actions {
                    crate::work::io(0)?;
                    for seal in
                        myelin_session::validate_handoff_action(self, &entry.config, record.block.number, action).map_err(integrity)?
                    {
                        deps.insert(self.block_id(seal.session_id, seal.height)?);
                    }
                    if let HandoffAction::Retire { consumed_at, .. } = &action.action {
                        deps.insert(self.block_id(consumed_at.session_id, consumed_at.height)?);
                    }
                    deps.insert(self.block_id(action.handoff.source_session_id, action.handoff.source_height)?);
                    if let Some(at) = &action.cancelled_at {
                        deps.insert(self.block_id(at.session_id, at.height)?);
                    }
                }
                if let Some(successor) = &record.successor {
                    {
                        let transition = &successor.transition;
                        for artifact in transition.artifact_dependencies() {
                            let id = BlobId(artifact);
                            self.directory.descriptor(id)?;
                            deps.insert(id);
                        }
                    }
                    self.session(successor.target_config.session_id)?;
                }
            }
        }
        for (id, blob) in &self.handoffs {
            let stored = self.handoff(*id)?;
            let deps = edges.entry(*blob).or_default();
            if let Some(import) = self.remote_handoffs.get(id) {
                deps.insert(import.proof);
            } else {
                deps.insert(self.block_id(stored.handoff.source_session_id, stored.handoff.source_height)?);
            }
            {
                let lifecycle = &stored.lifecycle;
                for at in match &lifecycle.terminal {
                    HandoffTerminal::Issued => vec![],
                    HandoffTerminal::Consumed { at } | HandoffTerminal::Cancelled { at } => vec![at],
                    HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at } => vec![cancelled_at, reclaimed_at],
                    HandoffTerminal::Retired { retired_at, .. } if self.remote_completions.contains_key(id) => vec![retired_at],
                    HandoffTerminal::Retired { consumed_at, retired_at } => vec![consumed_at, retired_at],
                } {
                    deps.insert(self.block_id(at.session_id, at.height)?);
                }
            }
        }
        for import in self.remote_handoffs.values() {
            let source = &import.source;
            let deps = edges.entry(import.proof).or_default();
            deps.extend([
                source.consensus,
                source.execution.program,
                source.execution.replayer,
                source.execution.rules,
                source.execution.build_inputs,
                source.execution.toolchain,
            ]);
            deps.extend(source.execution.runtime_dependencies.iter().copied());
        }
        for completion in self.remote_completions.values() {
            let target = &completion.target;
            let deps = edges.entry(completion.retirement).or_default();
            deps.extend([
                target.consensus,
                target.execution.program,
                target.execution.replayer,
                target.execution.rules,
                target.execution.build_inputs,
                target.execution.toolchain,
            ]);
            deps.extend(target.execution.runtime_dependencies.iter().copied());
        }
        for ((session, _), (blob, height)) in &self.evidence {
            edges.entry(*blob).or_default().insert(self.block_id(*session, *height)?);
        }
        Ok(edges)
    }
    /// Explicit trusted-code hook. The running executable must match the pinned
    /// replayer blob before the compiled restore callback is invoked. This never
    /// discovers, dlopens or executes a program supplied by the bundle.
    pub fn replay_current_process<E: TransitionExecutor>(
        self: &Arc<Self>,
        session: Hash32,
        restore: impl Fn(&[u8], &Self) -> myelin_session::Result<E> + Send + Sync + 'static,
    ) -> crate::Result<Option<RangeReplayReceipt>> {
        let index = self.session(session)?;
        let descriptor = self.directory.descriptor(index.inventory.execution.replayer)?;
        if running_digest(descriptor.bytes)? != descriptor.id.0 {
            return Err(integrity("current executable is not the pinned historical replayer"));
        }
        let audited =
            SessionChain::<E, Self>::audit_recovery(session, &index.config.application_profile, index.verifier.clone(), self.clone())
                .map_err(integrity)?;
        let executor = restore(audited.state_snapshot(), self.as_ref()).map_err(integrity)?;
        let chain =
            SessionChain::recover_audited(audited, &index.config.application_profile, index.verifier.clone(), executor, self.clone())
                .map_err(integrity)?;
        let store = self.clone();
        chain
            .head()
            .finalised_height
            .map(|end| chain.replay_range(0, end, move |bytes| restore(bytes, store.as_ref())).map_err(integrity))
            .transpose()
    }
}

impl SessionStore for BundleStore {
    fn create_session(&self, _: &SessionGenesis, _: &SessionHead) -> std::result::Result<(), StoreError> {
        readonly()
    }
    fn load_genesis(&self, id: Hash32) -> std::result::Result<SessionGenesis, StoreError> {
        let index = self.session(id).map_err(backend)?;
        SessionGenesis::decode(&self.bytes(index.inventory.genesis).map_err(backend)?).map_err(|e| StoreError::Corrupt(e.to_string()))
    }
    fn load_head(&self, id: Hash32) -> std::result::Result<SessionHead, StoreError> {
        let index = self.session(id).map_err(backend)?;
        SessionHead::decode(&self.bytes(index.inventory.head).map_err(backend)?).map_err(|e| StoreError::Corrupt(e.to_string()))
    }
    fn load_chain_page(&self, id: Hash32, start: u64, limit: usize) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
        let len = self.session(id).map_err(backend)?.inventory.blocks.len() as u64;
        let mut records = vec![];
        let mut bytes = 0u64;
        for h in start..start.saturating_add(limit.min(64) as u64).min(len) {
            let blob = self.block_id(id, h).map_err(backend)?;
            let size = self.directory.descriptor(blob).map_err(backend)?.bytes;
            if !records.is_empty() && bytes.saturating_add(size) > 16 * 1024 * 1024 {
                break;
            }
            records.push(self.load_block(id, h)?);
            bytes += size;
        }
        Ok(records)
    }
    fn load_block(&self, id: Hash32, height: u64) -> std::result::Result<FinalisedBlockRecord, StoreError> {
        self.record(id, height).map_err(backend)
    }
    fn load_command_receipt(&self, id: Hash32, command: Hash32) -> std::result::Result<Option<CommandReceipt>, StoreError> {
        self.commands
            .get(&(id, command))
            .map(|(h, i)| CommandReceipt::for_record(&self.load_block(id, *h)?, *i).map_err(|e| StoreError::Corrupt(e.to_string())))
            .transpose()
    }
    fn load_checkpoint(&self, id: Hash32) -> std::result::Result<StateCheckpoint, StoreError> {
        let index = self.session(id).map_err(backend)?;
        StateCheckpoint::decode(&self.bytes(index.inventory.checkpoint).map_err(backend)?)
            .map_err(|e| StoreError::Corrupt(e.to_string()))
    }
    fn load_checkpoint_before(&self, id: Hash32, height: u64) -> std::result::Result<StateCheckpoint, StoreError> {
        if let Some(h) = self.session(id).map_err(backend)?.checkpoints.range(..height).next_back().copied() {
            let record = self.load_block(id, h)?;
            if record.state_snapshot.is_empty() {
                return Err(StoreError::Corrupt("indexed archival snapshot is absent".into()));
            }
            return Ok(StateCheckpoint {
                session_id: id,
                finalised_height: Some(h),
                state_root: record.block.state_root_after,
                state_snapshot: record.state_snapshot,
            });
        }
        let g = self.load_genesis(id)?;
        Ok(StateCheckpoint {
            session_id: id,
            finalised_height: None,
            state_root: g.config.initial_state_root,
            state_snapshot: g.state_snapshot,
        })
    }
    fn commit_block(&self, _: &SessionHead, _: &SessionHead, _: &FinalisedBlockRecord) -> std::result::Result<(), StoreError> {
        readonly()
    }
    fn load_consensus_wal(&self, _: Hash32) -> std::result::Result<Option<ConsensusWal>, StoreError> {
        Ok(None)
    }
    fn compare_and_set_consensus_wal(&self, _: Option<u64>, _: &ConsensusWal) -> std::result::Result<(), StoreError> {
        readonly()
    }
    fn pending_outbox(&self, _: Hash32, _: usize) -> std::result::Result<Vec<PendingDelivery>, StoreError> {
        readonly()
    }
    fn acknowledge_outbox(&self, _: Hash32, _: Hash32) -> std::result::Result<(), StoreError> {
        readonly()
    }
    fn load_handoff(&self, id: Hash32) -> std::result::Result<Option<StoredHandoff>, StoreError> {
        self.handoffs.contains_key(&id).then(|| self.handoff(id).map_err(backend)).transpose()
    }
    fn load_handoff_import(&self, id: Hash32) -> std::result::Result<Option<myelin_session::VerifiedHandoffImport>, StoreError> {
        let Some(inventory) = self.remote_handoffs.get(&id) else { return Ok(None) };
        let registry = self
            .directory
            .remote_imports
            .as_ref()
            .ok_or_else(|| StoreError::Corrupt("archive import verifier configuration absent".into()))?;
        let bytes = self.bytes(inventory.proof).map_err(backend)?;
        let record = myelin_session::HandoffInboxRecord::decode(&bytes).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let verified = registry.verify(&record).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if verified.handoff().id() != id {
            return Err(StoreError::Corrupt("archive remote proof identity mismatch".into()));
        }
        Ok(Some(verified))
    }
    fn handoff_import_registry(&self) -> Option<Arc<myelin_session::HandoffImportRegistry>> {
        self.directory.remote_imports.clone()
    }
    fn load_evidence(&self, id: Hash32, message: Hash32) -> std::result::Result<Option<EvidenceRecord>, StoreError> {
        self.evidence
            .get(&(id, message))
            .map(|(blob, _)| {
                EvidenceRecord::decode(&self.bytes(*blob).map_err(backend)?).map_err(|e| StoreError::Corrupt(e.to_string()))
            })
            .transpose()
    }
}
