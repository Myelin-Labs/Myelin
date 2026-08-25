use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ConsensusKind, FinalityProof, ProofOfAuthorityConfig};
use myelin_session::{
    ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, BlockRequest, ConsensusWal, EvidenceRequirement,
    ExecutionOutcome, FrameInput, HandoffIntent, HandoffTarget, PendingOutboxMessage, SessionChain, SessionConfig, SessionStore,
    StoreError, TransitionExecutor,
};
use myelin_session_network::{
    queue_outbound, Clock, EnqueueStatus, MessageClass, MessageType, NetworkBinding, NetworkSigner, NetworkStore,
};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::{RocksSessionStore, RocksSessionStoreOptions};
use rocksdb::{OptimisticTransactionDB, Options};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct FixedClock(u64);

impl Clock for FixedClock {
    fn now_ms(&self) -> Result<u64, myelin_session_network::NetworkError> {
        Ok(self.0)
    }
}

#[derive(Clone)]
struct TestExecutor {
    root: [u8; 32],
}

impl TestExecutor {
    fn from_snapshot(snapshot: &[u8]) -> Self {
        Self { root: snapshot.try_into().unwrap() }
    }
}

impl TransitionExecutor for TestExecutor {
    fn state_root(&self) -> [u8; 32] {
        self.root
    }

    fn execute_block(
        &mut self,
        height: u64,
        frame_input: &FrameInput,
        consumed_handoffs: &[myelin_session::SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        let mut txids = Vec::new();
        for transaction in transactions {
            txids.push(myelin_exec::deserialize_transaction_molecule(transaction).map_err(|error| error.to_string())?.id());
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.root);
        hasher.update(&height.to_le_bytes());
        hasher.update(&frame_input.root());
        for handoff in consumed_handoffs {
            hasher.update(&handoff.id());
        }
        for txid in &txids {
            hasher.update(txid);
        }
        self.root = *hasher.finalize().as_bytes();
        Ok(ExecutionOutcome {
            ordered_cell_tx_commitments: txids,
            data_commitments: vec![],
            outbox: vec![PendingOutboxMessage { topic: "asset-exit/test-v1".to_owned(), payload: vec![9, height as u8] }],
            cycles: transactions.len() as u64 * 10,
        })
    }

    fn snapshot(&self) -> Result<Vec<u8>, String> {
        Ok(self.root.to_vec())
    }
}

fn consensus() -> (Arc<RegisteredFinalityVerifier>, CommitteeSigner) {
    let signer = CommitteeSigner::new("authority", [7; 32]).unwrap();
    let config = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().to_owned(), public_key: signer.ckb_public_key() }],
    });
    (Arc::new(RegisteredFinalityVerifier::new(config).unwrap()), signer)
}

fn config(consensus: &RegisteredFinalityVerifier) -> SessionConfig {
    use myelin_session::FinalityVerifier;
    let descriptor = consensus.descriptor();
    SessionConfig {
        session_id: [3; 32],
        consensus_kind: ConsensusKind::ProofOfAuthority,
        consensus_config_commitment: descriptor.config_commitment,
        consensus_module_commitment: descriptor.commitment(),
        consensus_wal_schema_hash: descriptor.wal_schema_hash,
        application_profile: application_profile(),
        initial_state_root: [1; 32],
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 100,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1_000,
    }
}

fn application_profile() -> ApplicationProfile {
    ApplicationProfile {
        application_id: "rocks-store-test".to_owned(),
        program_digest: [11; 32],
        input_schema_hash: [12; 32],
        state_codec_hash: [13; 32],
        logical_time_policy_hash: [14; 32],
        entropy_policy_hash: [15; 32],
        vm: ApplicationVmProfile { spawn_ipc_required: false },
        resources: ApplicationResourceEnvelope {
            max_frame_inputs: 10_000,
            max_frame_input_bytes: 1_000_000,
            max_frame_cycles: 10_000_000,
            max_logical_time_span: 10_000,
            max_inspect_query_bytes: 1_024,
            max_inspect_result_bytes: 4_096,
        },
        court_profile_hash: [16; 32],
        handoff_policy_hash: [17; 32],
    }
}

fn frame_input(position: u64) -> FrameInput {
    FrameInput {
        start_position: position,
        end_position: position + 1,
        logical_time_start: position,
        logical_time_end: position + 1,
        payload: vec![position as u8],
    }
}

fn transaction(seed: u8) -> Vec<u8> {
    let tx = myelin_exec::CellTx::new(vec![], vec![], vec![], vec![], vec![vec![seed]]).unwrap();
    myelin_exec::serialize_transaction_molecule(&tx).unwrap()
}

fn expected_root(root: [u8; 32], height: u64, frame_input: &FrameInput, transactions: &[Vec<u8>], handoffs: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&root);
    hasher.update(&height.to_le_bytes());
    hasher.update(&frame_input.root());
    for handoff in handoffs {
        hasher.update(handoff);
    }
    for transaction in transactions {
        let tx = myelin_exec::deserialize_transaction_molecule(transaction).unwrap();
        hasher.update(&tx.id());
    }
    *hasher.finalize().as_bytes()
}

#[test]
fn store_rejects_unversioned_data_and_unknown_schema() {
    let directory = tempfile::tempdir().unwrap();
    let unversioned_path = directory.path().join("unversioned");
    let mut options = Options::default();
    options.create_if_missing(true);
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&options, &unversioned_path).unwrap();
    db.put(b"foreign-data", b"value").unwrap();
    drop(db);
    assert!(matches!(RocksSessionStore::open(&unversioned_path), Err(StoreError::Corrupt(_))));

    let unknown_path = directory.path().join("unknown-version");
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&options, &unknown_path).unwrap();
    db.put(b"\0myelin-session-store-schema", b"1").unwrap();
    drop(db);
    assert!(matches!(RocksSessionStore::open(&unknown_path), Err(StoreError::Corrupt(_))));
}

#[test]
fn store_rejects_superseded_internal_schema() {
    let directory = tempfile::tempdir().unwrap();
    let mut options = Options::default();
    options.create_if_missing(true);
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&options, directory.path()).unwrap();
    db.put(b"\0myelin-session-store-schema", b"4").unwrap();
    drop(db);

    assert!(matches!(RocksSessionStore::open(directory.path()), Err(StoreError::Corrupt(_))));
}

#[test]
fn atomic_commit_recovery_wal_and_outbox_lifecycle() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (consensus, signer) = consensus();
    let mut chain =
        SessionChain::create(config(&consensus), consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();

    let descriptor = myelin_session::FinalityVerifier::descriptor(consensus.as_ref());
    let wal = |payload: &[u8]| {
        ConsensusWal::new(
            [3; 32],
            0,
            descriptor.commitment(),
            descriptor.config_commitment,
            descriptor.wal_schema_hash,
            0,
            payload.to_vec(),
        )
        .unwrap()
    };

    store.compare_and_set_consensus_wal(None, &wal(b"round-0")).unwrap();
    assert!(matches!(store.compare_and_set_consensus_wal(None, &wal(b"stale")), Err(StoreError::Conflict(_))));
    let wrong_module =
        ConsensusWal::new([3; 32], 0, [9; 32], descriptor.config_commitment, descriptor.wal_schema_hash, 1, b"wrong-module".to_vec())
            .unwrap();
    assert!(matches!(store.compare_and_set_consensus_wal(Some(0), &wrong_module), Err(StoreError::Conflict(_))));
    let wrong_schema =
        ConsensusWal::new([3; 32], 0, descriptor.commitment(), descriptor.config_commitment, [8; 32], 1, b"wrong-schema".to_vec())
            .unwrap();
    assert!(matches!(store.compare_and_set_consensus_wal(Some(0), &wrong_schema), Err(StoreError::Conflict(_))));

    let prepared = chain
        .prepare_block(BlockRequest {
            transactions: vec![transaction(0)],
            frame_input: frame_input(0),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 10,
            local_now_ms: 10,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![],
        })
        .unwrap();
    let engine = match consensus.selected_consensus() {
        myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) => engine,
        _ => unreachable!(),
    };
    let seal = engine.seal_from_signer(prepared.block().hash(), 0, &signer).unwrap();
    let head = chain.commit_finalised(prepared, FinalityProof::ProofOfAuthority(seal)).unwrap();
    assert_eq!(head.finalised_height, Some(0));
    assert!(store.load_consensus_wal([3; 32]).unwrap().is_none());

    let pending = store.pending_outbox([3; 32], 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].height, 0);
    store.acknowledge_outbox([3; 32], pending[0].message.id).unwrap();
    assert!(store.pending_outbox([3; 32], 10).unwrap().is_empty());

    drop(chain);
    drop(store);
    let reopened = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (_, snapshot) = SessionChain::<TestExecutor, RocksSessionStore>::recovery_snapshot(&reopened, [3; 32]).unwrap();
    let recovered =
        SessionChain::recover([3; 32], &application_profile(), consensus, TestExecutor::from_snapshot(&snapshot), reopened).unwrap();
    assert_eq!(recovered.head(), &head);
}

#[test]
fn durable_head_cas_rejects_two_writers_at_one_height() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (consensus, signer) = consensus();
    let chain_a =
        SessionChain::create(config(&consensus), consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let mut chain_a = chain_a;
    let mut chain_b =
        SessionChain::recover([3; 32], &application_profile(), consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store))
            .unwrap();
    let request = || BlockRequest {
        transactions: vec![transaction(0)],
        frame_input: frame_input(0),
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: 10,
        local_now_ms: 10,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
    };
    let prepared_a = chain_a.prepare_block(request()).unwrap();
    let prepared_b = chain_b.prepare_block(request()).unwrap();
    let engine = match consensus.selected_consensus() {
        myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) => engine,
        _ => unreachable!(),
    };
    let proof_a = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared_a.block().hash(), 0, &signer).unwrap());
    let proof_b = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared_b.block().hash(), 0, &signer).unwrap());
    chain_a.commit_finalised(prepared_a, proof_a).unwrap();
    assert!(matches!(
        chain_b.commit_finalised(prepared_b, proof_b),
        Err(myelin_session::SessionError::Store(StoreError::Conflict(_)))
    ));
}

#[test]
fn network_sequences_and_queues_are_durable_and_gap_free() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (consensus, _) = consensus();
    let binding = NetworkBinding {
        session_id: [3; 32],
        consensus_module_commitment: myelin_session::FinalityVerifier::descriptor(consensus.as_ref()).commitment(),
    };
    let chain = SessionChain::create(config(&consensus), consensus, TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let signer = NetworkSigner::new("authority", [7; 32]).unwrap();

    let block_candidate = MessageType::new(MessageClass::BlockCandidate, 1, 0);
    let finality_proof = MessageType::new(MessageClass::FinalityProof, 1, 0);
    let first = queue_outbound(&*store, &FixedClock(10), &signer, binding, "peer-b", block_candidate, vec![1]).unwrap();
    let second = queue_outbound(&*store, &FixedClock(11), &signer, binding, "peer-b", finality_proof, vec![2]).unwrap();
    let other_peer = queue_outbound(&*store, &FixedClock(12), &signer, binding, "peer-c", block_candidate, vec![3]).unwrap();
    assert_eq!(first.envelope.sequence, 0);
    assert_eq!(second.envelope.sequence, 1);
    assert_eq!(other_peer.envelope.sequence, 0);
    assert_eq!(store.pending_outbound([3; 32], "peer-b", 10).unwrap(), vec![first.clone(), second.clone()]);
    assert_eq!(store.pending_outbound([3; 32], "peer-c", 10).unwrap(), vec![other_peer]);

    assert!(matches!(store.enqueue_inbound(&second), Err(myelin_session_network::NetworkStoreError::Conflict(_))));
    assert_eq!(store.enqueue_inbound(&first).unwrap(), EnqueueStatus::Enqueued);
    assert_eq!(store.enqueue_inbound(&first).unwrap(), EnqueueStatus::Duplicate);
    let equivocation =
        myelin_session_network::DurableEnvelope::new(signer.sign(binding, "peer-b", 0, 10, block_candidate, vec![9]).unwrap());
    assert!(matches!(store.enqueue_inbound(&equivocation), Err(myelin_session_network::NetworkStoreError::Conflict(_))));
    assert_eq!(store.enqueue_inbound(&second).unwrap(), EnqueueStatus::Enqueued);
    assert_eq!(store.enqueue_inbound(&first).unwrap(), EnqueueStatus::Duplicate);
    assert_eq!(store.pending_inbound([3; 32], 10).unwrap(), vec![first.clone(), second.clone()]);

    let wrong_binding = NetworkBinding { session_id: [3; 32], consensus_module_commitment: [8; 32] };
    let wrong_module =
        myelin_session_network::DurableEnvelope::new(signer.sign(wrong_binding, "peer-z", 0, 10, block_candidate, vec![9]).unwrap());
    assert!(matches!(store.enqueue_outbound(&wrong_module), Err(myelin_session_network::NetworkStoreError::Conflict(_))));

    store.acknowledge_outbound([3; 32], first.message_id).unwrap();
    store.acknowledge_inbound([3; 32], first.message_id).unwrap();
    assert_eq!(store.enqueue_inbound(&first).unwrap(), EnqueueStatus::Duplicate);
    assert_eq!(store.pending_outbound([3; 32], "peer-b", 10).unwrap(), vec![second.clone()]);
    assert_eq!(store.pending_inbound([3; 32], 10).unwrap(), vec![second]);
    drop(chain);
    drop(store);
    let reopened = RocksSessionStore::open(directory.path()).unwrap();
    assert_eq!(reopened.enqueue_inbound(&first).unwrap(), EnqueueStatus::Duplicate);
}

#[test]
fn rolling_checkpoint_avoids_full_snapshot_in_every_historical_block() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        RocksSessionStore::open_with_options(
            directory.path(),
            RocksSessionStoreOptions { archival_checkpoint_interval: 2, ..RocksSessionStoreOptions::default() },
        )
        .unwrap(),
    );
    let (consensus, signer) = consensus();
    let mut chain =
        SessionChain::create(config(&consensus), consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let engine = match consensus.selected_consensus() {
        myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) => engine,
        _ => unreachable!(),
    };
    for height in 0..3 {
        let prepared = chain
            .prepare_block(BlockRequest {
                transactions: vec![transaction(height as u8)],
                frame_input: frame_input(height),
                scheduler_commitment: [5; 32],
                data_commitments: vec![],
                timestamp_ms: height + 1,
                local_now_ms: height + 1,
                successor: None,
                handoffs: vec![],
                consume_handoffs: vec![],
            })
            .unwrap();
        let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), height, &signer).unwrap());
        chain.commit_finalised(prepared, proof).unwrap();
    }

    let page = store.load_chain_page([3; 32], 0, 10).unwrap();
    assert!(!page[0].state_snapshot.is_empty());
    assert!(page[1].state_snapshot.is_empty());
    assert!(!page[2].state_snapshot.is_empty());
    let checkpoint = store.load_checkpoint([3; 32]).unwrap();
    assert_eq!(checkpoint.finalised_height, Some(2));
    assert_eq!(checkpoint.state_root, chain.head().state_root);
}

#[test]
fn successor_is_created_atomically_and_seals_the_predecessor() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (consensus, signer) = consensus();
    let source_config = config(&consensus);
    let mut source =
        SessionChain::create(source_config.clone(), consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let input = frame_input(0);
    let transactions = vec![transaction(1)];
    let post_root = expected_root([1; 32], 0, &input, &transactions, &[]);
    let mut target_config = source_config.clone();
    target_config.session_id = [4; 32];
    target_config.initial_state_root = post_root;
    target_config.initial_input_position = 1;
    target_config.initial_logical_time = 1;
    target_config.initial_timestamp_ms = 10;
    target_config.predecessor = Some(myelin_session::PredecessorReference {
        session_id: source_config.session_id,
        final_height: 0,
        final_state_root: post_root,
        application_profile_commitment: source_config.application_profile.commitment(),
    });
    let prepared = source
        .prepare_block(BlockRequest {
            transactions: transactions.clone(),
            frame_input: input,
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 10,
            local_now_ms: 10,
            successor: Some(target_config.clone()),
            handoffs: vec![],
            consume_handoffs: vec![],
        })
        .unwrap();
    assert_eq!(prepared.successor().unwrap().target_config, target_config);
    let engine = match consensus.selected_consensus() {
        myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) => engine,
        _ => unreachable!(),
    };
    let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), 0, &signer).unwrap());
    let sealed_head = source.commit_finalised(prepared, proof).unwrap();
    assert_eq!(sealed_head.sealed_by_successor, Some([4; 32]));
    assert!(source
        .prepare_block(BlockRequest {
            transactions: vec![transaction(2)],
            frame_input: frame_input(1),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 11,
            local_now_ms: 11,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![],
        })
        .is_err());

    let target_genesis = store.load_genesis([4; 32]).unwrap();
    assert_eq!(target_genesis.config, target_config);
    assert_eq!(target_genesis.state_snapshot, post_root);
    let target = SessionChain::recover(
        [4; 32],
        &application_profile(),
        consensus,
        TestExecutor::from_snapshot(&target_genesis.state_snapshot),
        store,
    )
    .unwrap();
    assert_eq!(target.head().state_root, post_root);
    assert_eq!(target.head().input_position, 1);
    assert_eq!(target.head().timestamp_ms, 10);
}

#[test]
fn handoff_is_source_committed_target_bound_expiring_and_single_use() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (consensus, signer) = consensus();
    let mut source =
        SessionChain::create(config(&consensus), consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let mut target_config = config(&consensus);
    target_config.session_id = [4; 32];
    let mut target =
        SessionChain::create(target_config, consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let mut wrong_config = config(&consensus);
    wrong_config.session_id = [5; 32];
    let wrong_target =
        SessionChain::create(wrong_config, consensus.clone(), TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let prepared = source
        .prepare_block(BlockRequest {
            transactions: vec![transaction(1)],
            frame_input: frame_input(0),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 10,
            local_now_ms: 10,
            successor: None,
            handoffs: vec![
                HandoffIntent {
                    target: HandoffTarget::Session([4; 32]),
                    expires_at_ms: 100,
                    payload: b"player-state".to_vec(),
                    authorization: b"signed-inventory".to_vec(),
                    evidence_requirement: None,
                },
                HandoffIntent {
                    target: HandoffTarget::Session([4; 32]),
                    expires_at_ms: 100,
                    payload: b"evidence-gated".to_vec(),
                    authorization: b"signed-gated".to_vec(),
                    evidence_requirement: Some(EvidenceRequirement {
                        message_id: [91; 32],
                        pipeline_commitment: [92; 32],
                        minimum_stage_index: 1,
                    }),
                },
                HandoffIntent {
                    target: HandoffTarget::Session([4; 32]),
                    expires_at_ms: 15,
                    payload: b"expired".to_vec(),
                    authorization: b"signed-expired".to_vec(),
                    evidence_requirement: None,
                },
            ],
            consume_handoffs: vec![],
        })
        .unwrap();
    let valid_id = prepared.issued_handoffs()[0].id();
    let expired_id = prepared.issued_handoffs()[1].id();
    let gated_id = prepared.issued_handoffs()[2].id();
    let proof = FinalityProof::ProofOfAuthority(match consensus.selected_consensus() {
        myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) => {
            engine.seal_from_signer(prepared.block().hash(), 0, &signer).unwrap()
        }
        _ => unreachable!(),
    });
    source.commit_finalised(prepared, proof).unwrap();
    assert!(!store.load_handoff(valid_id).unwrap().unwrap().is_consumed());

    assert!(wrong_target
        .prepare_block(BlockRequest {
            transactions: vec![transaction(2)],
            frame_input: frame_input(0),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 20,
            local_now_ms: 20,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![valid_id],
        })
        .is_err());
    assert!(target
        .prepare_block(BlockRequest {
            transactions: vec![transaction(2)],
            frame_input: frame_input(0),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 20,
            local_now_ms: 20,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![expired_id],
        })
        .is_err());
    assert!(target
        .prepare_block(BlockRequest {
            transactions: vec![transaction(2)],
            frame_input: frame_input(0),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 20,
            local_now_ms: 20,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![gated_id],
        })
        .is_err());

    let prepared = target
        .prepare_block(BlockRequest {
            transactions: vec![transaction(3)],
            frame_input: frame_input(0),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 20,
            local_now_ms: 20,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![valid_id],
        })
        .unwrap();
    assert_eq!(prepared.consumed_handoffs()[0].id(), valid_id);
    let proof = FinalityProof::ProofOfAuthority(match consensus.selected_consensus() {
        myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) => {
            engine.seal_from_signer(prepared.block().hash(), 0, &signer).unwrap()
        }
        _ => unreachable!(),
    });
    let target_head = target.commit_finalised(prepared, proof).unwrap();
    let stored = store.load_handoff(valid_id).unwrap().unwrap();
    assert_eq!(stored.consumed_by_session, Some([4; 32]));
    assert_eq!(stored.consumed_at_height, Some(0));
    assert_eq!(stored.consumed_by_block_hash, Some(target_head.block_hash));
    assert!(target
        .prepare_block(BlockRequest {
            transactions: vec![transaction(4)],
            frame_input: frame_input(1),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 21,
            local_now_ms: 21,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![valid_id],
        })
        .is_err());
}

#[test]
fn durable_network_queue_quota_is_released_by_batch_acknowledgement() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        RocksSessionStore::open_with_options(
            directory.path(),
            RocksSessionStoreOptions {
                max_network_queue_messages: 2,
                max_network_queue_bytes: 1_000_000,
                ..RocksSessionStoreOptions::default()
            },
        )
        .unwrap(),
    );
    let (consensus, _) = consensus();
    let binding = NetworkBinding {
        session_id: [3; 32],
        consensus_module_commitment: myelin_session::FinalityVerifier::descriptor(consensus.as_ref()).commitment(),
    };
    let _chain = SessionChain::create(config(&consensus), consensus, TestExecutor { root: [1; 32] }, Arc::clone(&store)).unwrap();
    let signer = NetworkSigner::new("authority", [7; 32]).unwrap();
    let message_type = MessageType::new(MessageClass::Consensus, 1, 0);
    let first = queue_outbound(&*store, &FixedClock(10), &signer, binding, "peer-b", message_type, vec![1]).unwrap();
    let second = queue_outbound(&*store, &FixedClock(11), &signer, binding, "peer-b", message_type, vec![2]).unwrap();
    assert!(matches!(
        queue_outbound(&*store, &FixedClock(12), &signer, binding, "peer-b", message_type, vec![3]),
        Err(myelin_session_network::NetworkError::Store(myelin_session_network::NetworkStoreError::ResourceLimit(_)))
    ));
    store.acknowledge_outbound_batch([3; 32], &[first.message_id, second.message_id]).unwrap();
    queue_outbound(&*store, &FixedClock(13), &signer, binding, "peer-b", message_type, vec![3]).unwrap();
}
