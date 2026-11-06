use super::*;
use myelin_consensus::{CommitteeSigner, FinalityProof, SelectedConsensus};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

#[derive(Clone)]
struct Frozen;
impl TransitionExecutor for Frozen {
    fn state_root(&self) -> Hash32 {
        *blake3::hash(b"frozen metadata state").as_bytes()
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        Ok(b"frozen metadata state".to_vec())
    }
    fn execute_block(
        &mut self,
        _: u64,
        _: &FrameInput,
        _: &[SessionHandoff],
        _: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        Ok(ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![], outbox: vec![], cycles: 0 })
    }
    fn execute_block_with_context(
        &mut self,
        context: &ProtocolEffectsContext,
        frame: &FrameInput,
        consumed: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        context.require_authority_metadata([99; 32])?;
        self.execute_block(context.height, frame, consumed, txs)
    }
}

#[test]
fn successor_catalog_requires_both_directions_and_each_authority_anchor() {
    let f = Fixture::new(TransferKind::Scarce);
    let mut cfg = f.source.config().clone();
    cfg.session_id = [10; 32];
    cfg.initial_state_root = Frozen.state_root();
    cfg.application_profile.application_id = "bundle-metadata-lineage-test".into();
    cfg.application_profile.command_identity_required = false;
    cfg.protocol_effects = ProtocolEffectsPolicy::AuthorityMetadata { policy_hash: [99; 32] };
    let mut chain = SessionChain::create(cfg.clone(), f.verifier.clone(), Frozen, f.store.clone()).unwrap();
    let mut target = cfg.clone();
    target.session_id = [11; 32];
    target.initial_input_position = 1;
    target.initial_logical_time = 1;
    target.initial_timestamp_ms = 10;
    target.predecessor = Some(PredecessorReference {
        session_id: cfg.session_id,
        final_height: 0,
        final_state_root: cfg.initial_state_root,
        application_profile_commitment: cfg.application_profile.commitment(),
    });
    let candidate = chain
        .prepare_block(BlockRequest {
            transactions: vec![],
            frame_input: FrameInput {
                commands: vec![],
                start_position: 0,
                end_position: 1,
                logical_time_start: 0,
                logical_time_end: 1,
                payload: b"seal".to_vec(),
            },
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 10,
            local_now_ms: 10,
            successor: Some(target.clone()),
            handoffs: vec![],
            consume_handoffs: vec![],
            handoff_actions: vec![],
        })
        .unwrap();
    let SelectedConsensus::ProofOfAuthority(engine) = f.verifier.selected_consensus() else { unreachable!() };
    let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(candidate.block().hash(), 0, &f.validator).unwrap());
    chain.commit_finalised(candidate, proof).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let plan = plan_for(&[&cfg, &target], &f.validator, dir.path());
    let archive = dir.path().join("archive");
    let mut manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
    let trusted = anchors(&manifest);
    error(open(&archive, &trusted[..1]), ErrorCode::UnknownTrustAnchor);
    let (store, report) = open(&archive, &trusted).unwrap();
    assert_eq!(report.sessions.len(), 2);
    assert_eq!(store.load_head(cfg.session_id).unwrap().sealed_by_successor, Some(target.session_id));
    assert_eq!(store.load_genesis(target.session_id).unwrap().config.predecessor, target.predecessor);
    // Both source-only and successor-only captures omit mandatory history.
    for (i, only) in [&cfg, &target].into_iter().enumerate() {
        let partial = plan_for(&[only], &f.validator, dir.path());
        let path = dir.path().join(format!("missing-{i}"));
        error(export_store(f.store.clone(), &partial, &path), ErrorCode::MissingDependency);
        assert!(!path.join("manifest.json").exists());
    }
    let predecessor = manifest.sessions[0].blocks[0];
    let successor = manifest.sessions[1].genesis;
    manifest.blobs.iter_mut().find(|b| b.id == successor).unwrap().dependencies.retain(|id| *id != predecessor);
    fs::write(archive.join("manifest.json"), manifest.encode().unwrap()).unwrap();
    error(open(&archive, &trusted), ErrorCode::MissingDependency);
}

struct AdvancingStore {
    inner: Arc<RocksSessionStore>,
    source: Mutex<Chain>,
    request: Mutex<Option<BlockRequest>>,
    verifier: Arc<RegisteredFinalityVerifier>,
    signer: CommitteeSigner,
    reads: AtomicUsize,
}
impl SessionStore for AdvancingStore {
    fn load_genesis(&self, id: Hash32) -> std::result::Result<SessionGenesis, StoreError> {
        self.inner.load_genesis(id)
    }
    fn load_head(&self, id: Hash32) -> std::result::Result<SessionHead, StoreError> {
        if id == [3; 32] && self.reads.fetch_add(1, Ordering::SeqCst) == 1 {
            let req = self.request.lock().unwrap().take().unwrap();
            commit(&mut self.source.lock().unwrap(), &self.verifier, &self.signer, req);
        }
        self.inner.load_head(id)
    }
    fn load_block(&self, id: Hash32, height: u64) -> std::result::Result<FinalisedBlockRecord, StoreError> {
        self.inner.load_block(id, height)
    }
    fn load_checkpoint(&self, id: Hash32) -> std::result::Result<StateCheckpoint, StoreError> {
        self.inner.load_checkpoint(id)
    }
    fn load_chain_page(&self, id: Hash32, start: u64, limit: usize) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
        self.inner.load_chain_page(id, start, limit)
    }
    fn load_command_receipt(&self, id: Hash32, command: Hash32) -> std::result::Result<Option<CommandReceipt>, StoreError> {
        self.inner.load_command_receipt(id, command)
    }
    fn load_checkpoint_before(&self, id: Hash32, height: u64) -> std::result::Result<StateCheckpoint, StoreError> {
        self.inner.load_checkpoint_before(id, height)
    }
    fn load_handoff(&self, id: Hash32) -> std::result::Result<Option<StoredHandoff>, StoreError> {
        self.inner.load_handoff(id)
    }
    fn load_evidence(&self, id: Hash32, message: Hash32) -> std::result::Result<Option<EvidenceRecord>, StoreError> {
        self.inner.load_evidence(id, message)
    }
    fn create_session(&self, _: &SessionGenesis, _: &SessionHead) -> std::result::Result<(), StoreError> {
        panic!("export is read-only")
    }
    fn commit_block(&self, _: &SessionHead, _: &SessionHead, _: &FinalisedBlockRecord) -> std::result::Result<(), StoreError> {
        panic!("export is read-only")
    }
    fn load_consensus_wal(&self, _: Hash32) -> std::result::Result<Option<ConsensusWal>, StoreError> {
        panic!("export must exclude WAL")
    }
    fn compare_and_set_consensus_wal(&self, _: Option<u64>, _: &ConsensusWal) -> std::result::Result<(), StoreError> {
        panic!("export is read-only")
    }
    fn pending_outbox(&self, _: Hash32, _: usize) -> std::result::Result<Vec<PendingDelivery>, StoreError> {
        panic!("export must exclude delivery state")
    }
    fn acknowledge_outbox(&self, _: Hash32, _: Hash32) -> std::result::Result<(), StoreError> {
        panic!("export is read-only")
    }
}

#[test]
fn source_commit_during_capture_rejects_publication_and_a_new_quiescent_export_succeeds() {
    let f = transfer();
    let req = issue_request(&f.source, vec![f.intent(2, 1, 1)]);
    let dir = tempfile::tempdir().unwrap();
    let plan = plan(&f, dir.path());
    let store = Arc::new(AdvancingStore {
        inner: f.store.clone(),
        source: Mutex::new(f.source),
        request: Mutex::new(Some(req)),
        verifier: f.verifier,
        signer: f.validator,
        reads: AtomicUsize::new(0),
    });
    let archive = dir.path().join("moving-source");
    error(export_store(store.clone(), &plan, &archive), ErrorCode::SourceChanged);
    assert!(!archive.join("manifest.json").exists());
    assert_eq!(store.inner.load_head([3; 32]).unwrap().finalised_height, Some(1));
    let retry = dir.path().join("quiescent-source");
    let manifest = export_store(store.inner.clone(), &plan, &retry).unwrap();
    let (_, report) = open(&retry, &anchors(&manifest)).unwrap();
    assert_eq!(report.sessions[0].finalised_height, Some(1));
}

#[test]
fn build_artifact_dependencies_are_exported_and_invalid_graphs_fail_before_copying() {
    let f = transfer();
    let dir = tempfile::tempdir().unwrap();
    let mut plan = plan(&f, dir.path());
    let mut dependencies = vec![];
    let mut files = vec![];
    for (name, bytes) in [("source-chunk", b"source bytes".as_slice()), ("toolchain-chunk", b"toolchain bytes".as_slice())] {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        let id = BlobId::of(bytes);
        dependencies.push(id);
        files.push(PackageFile {
            name: name.into(),
            bytes: bytes.len() as u64,
            digest: id,
            chunks: vec![FileChunk { id, bytes: bytes.len() as u64 }],
        });
        plan.artifacts.push(ArtifactSource { id, path, dependencies: vec![] });
    }
    dependencies.sort();
    let package = FilePackage { format: "myelin-file-package-v1".into(), files }.encode().unwrap();
    let root = BlobId::of(&package);
    let path = dir.path().join("package");
    fs::write(&path, &package).unwrap();
    plan.artifacts.push(ArtifactSource { id: root, path, dependencies: dependencies.clone() });
    for session in &mut plan.sessions {
        session.execution.build_inputs = root;
    }
    let archive = dir.path().join("archive");
    let manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
    open(&archive, &anchors(&manifest)).unwrap();
    let bundle = BundleDirectory::open(&archive, test_limits()).unwrap();
    assert_eq!(manifest.blobs.iter().find(|blob| blob.id == root).unwrap().dependencies, dependencies);
    assert_eq!(bundle.read_blob(root, 1024).unwrap(), package);
    for child in &dependencies {
        assert!(manifest.blobs.iter().find(|blob| blob.id == *child).unwrap().roles.contains(&BlobKind::BuildInputs));
        assert!(!bundle.read_blob(*child, 1024).unwrap().is_empty());
    }
    for (name, expected) in [
        ("missing", ErrorCode::MissingDependency),
        ("cycle", ErrorCode::DependencyCycle),
        ("edge-budget", ErrorCode::Limit),
        ("depth-budget", ErrorCode::Limit),
        ("duplicate", ErrorCode::Format),
    ] {
        let mut invalid = plan.clone();
        match name {
            "missing" => invalid.artifacts.retain(|s| s.id != dependencies[0]),
            "cycle" => invalid.artifacts.iter_mut().find(|s| s.id == dependencies[0]).unwrap().dependencies = vec![root],
            "edge-budget" => invalid.limits.edges = 1,
            "depth-budget" => invalid.limits.dependency_depth = 1,
            "duplicate" => invalid.artifacts.iter_mut().find(|s| s.id == root).unwrap().dependencies.push(dependencies[1]),
            _ => unreachable!(),
        }
        let destination = dir.path().join(name);
        error(export_store(f.store.clone(), &invalid, &destination), expected);
        assert!(!destination.exists(), "invalid graph must reject before creating or copying output");
    }
}
