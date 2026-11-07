use myelin_session::HandoffImportKind;
#[allow(dead_code)]
mod support;
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session::{
    EvidencePipelineDescriptor, EvidenceReceipt, EvidenceRecord, EvidenceRequirement, EvidenceStage, FinalisedBlockRecord,
    HandoffImportPolicy, HandoffImportVerifier, HandoffProofBundle, Hash32, OutboxMessage, SessionConfig, SessionError, SessionStore,
};
use myelin_session::{FinalityVerifier, HandoffImportRegistry, HandoffInboxRecord, SessionChain};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::{RemoteInboxLimits, RocksSessionStore};
use myelin_session_transfer::*;
use secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use std::sync::Arc;
use support::*;

struct IntakeVerifier {
    policy: TransferPolicy,
    source_policy: TransferPolicy,
    evidence_key: XOnlyPublicKey,
}
impl IntakeVerifier {
    fn for_fixture(f: &Fixture) -> Self {
        Self {
            source_policy: stored_state(&f.store, f.source.config().session_id).policy().clone(),
            policy: stored_state(&f.store, f.target.config().session_id).policy().clone(),
            evidence_key: Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[19; 32]).unwrap())
                .x_only_public_key()
                .0,
        }
    }
    fn stage_commitment(&self) -> Hash32 {
        *blake3::hash(&self.evidence_key.serialize()).as_bytes()
    }
}
fn rejected(message: impl ToString) -> SessionError {
    SessionError::Continuity(message.to_string())
}
fn evidence_digest(message: &OutboxMessage, stage: &EvidenceStage, previous: Option<&EvidenceReceipt>) -> Hash32 {
    let mut hash = blake3::Hasher::new();
    hash.update(b"remote-proof-test:external-review:v1");
    hash.update(&message.commitment());
    hash.update(&stage.verifier_commitment);
    hash.update(stage.name.as_bytes());
    if let Some(previous) = previous {
        hash.update(&previous.evidence);
    }
    *hash.finalize().as_bytes()
}
impl HandoffImportVerifier for IntakeVerifier {
    fn verify_application(
        &self,
        source: &SessionConfig,
        target: &SessionConfig,
        handoff: &myelin_session::SessionHandoff,
    ) -> myelin_session::Result<HandoffImportKind> {
        let verifier = TransferImportVerifier::new(
            self.policy.clone(),
            vec![self.source_policy.clone()],
            std::sync::Arc::new(|_, _, _, _| Err(rejected("no evidence adapter"))),
        )
        .map_err(rejected)?;
        verifier.verify_application(source, target, handoff)
    }
    fn verify_evidence(
        &self,
        message: &OutboxMessage,
        stage: &EvidenceStage,
        previous: Option<&EvidenceReceipt>,
        evidence: &[u8],
    ) -> myelin_session::Result<()> {
        if stage.verifier_commitment != self.stage_commitment() {
            return Err(rejected("unknown external reviewer"));
        }
        let signature = Signature::from_slice(evidence).map_err(rejected)?;
        Secp256k1::verification_only()
            .verify_schnorr(&signature, &Message::from_digest(evidence_digest(message, stage, previous)), &self.evidence_key)
            .map_err(rejected)
    }
}

#[test]
fn source_proof_uses_independent_genesis_consensus_application_and_exact_target_pins() {
    let mut f = Fixture::new(TransferKind::CopyableReceipt);
    let intent = f.intent(1, 0, 1);
    let request = issue_request(&f.source, vec![intent]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, request);
    let proof = HandoffProofBundle::export(f.store.as_ref(), ids[0], f.target.config()).unwrap();
    let policy = HandoffImportPolicy::new(f.source.config(), f.target.config(), 1_000_000).unwrap();
    let adapter = IntakeVerifier::for_fixture(&f);
    let verify = |proof: &HandoffProofBundle, policy: &HandoffImportPolicy| {
        proof.verify(policy, f.source.config(), f.target.config(), f.verifier.as_ref(), &adapter)
    };
    let original = verify(&proof, &policy).unwrap();
    let bytes = proof.encode(1_000_000).unwrap();
    let decoded = HandoffProofBundle::decode(&bytes, bytes.len()).unwrap();
    assert_eq!(verify(&decoded, &policy).unwrap().inbox_key(), original.inbox_key());
    assert_eq!(original.handoff().id(), ids[0]);
    assert!(f.store.load_handoff(ids[0]).unwrap().unwrap().is_available());
    assert!(stored_state(&f.store, f.target.config().session_id).receipts().is_empty());
    for case in 0..9 {
        let mut changed = proof.clone();
        let mut pins = policy.clone();
        match case {
            0 => pins.source_genesis[0] ^= 1,
            1 => pins.target_genesis[0] ^= 1,
            2 => changed.target_genesis[0] ^= 1,
            3 => changed.handoff_id[0] ^= 1,
            4 => changed.source_config[10] ^= 1,
            5 => {
                let mut block = FinalisedBlockRecord::decode(&changed.issuing_block).unwrap();
                block.block.timestamp_ms += 1;
                changed.issuing_block = block.encode().unwrap();
            }
            6 => {
                let mut block = FinalisedBlockRecord::decode(&changed.issuing_block).unwrap();
                block.issued_handoffs[0].payload[0] ^= 1;
                changed.issuing_block = block.encode().unwrap();
            }
            7 => changed.format_version += 1,
            8 => {
                let mut record = FinalisedBlockRecord::decode(&changed.issuing_block).unwrap();
                record.frame_input.payload[0] ^= 1;
                changed.issuing_block = record.encode().unwrap();
            }
            _ => unreachable!(),
        }
        assert!(verify(&changed, &pins).is_err(), "mutation {case}");
    }
    assert!(proof.encode(bytes.len() - 1).is_err());
    assert!(HandoffProofBundle::decode(&bytes, bytes.len() - 1).is_err());
    assert!(HandoffProofBundle::decode(&[0; 64], 0).is_err());
    let mut appended = bytes;
    appended.push(0);
    assert!(HandoffProofBundle::decode(&appended, 1_000_000).is_err());
}

#[test]
fn required_remote_evidence_reverifies_actual_stage_signatures_and_committed_message() {
    let mut f = Fixture::new(TransferKind::CopyableReceipt);
    let intent = f.intent(1, 0, 1);
    let request = issue_request(&f.source, vec![intent]);
    commit(&mut f.source, &f.verifier, &f.validator, request);
    let message = f.store.load_block(f.source.config().session_id, 0).unwrap().outbox[0].clone();
    let adapter = IntakeVerifier::for_fixture(&f);
    let descriptor = EvidencePipelineDescriptor::new(
        "independent-review",
        &message.topic,
        vec![
            EvidenceStage { name: "retrieved".into(), verifier_commitment: adapter.stage_commitment() },
            EvidenceStage { name: "reviewed".into(), verifier_commitment: adapter.stage_commitment() },
        ],
    )
    .unwrap();
    let key = Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[19; 32]).unwrap());
    let mut evidence: Option<EvidenceRecord> = None;
    for i in 0..2 {
        let previous = evidence.as_ref().and_then(|r| r.receipts.last());
        let signature = Secp256k1::new()
            .sign_schnorr_no_aux_rand(&Message::from_digest(evidence_digest(&message, &descriptor.stages[i], previous)), &key)
            .serialize()
            .to_vec();
        let prior = previous.map_or([0; 32], |r| r.commitment(descriptor.commitment(), message.commitment()));
        let receipt = EvidenceReceipt::new(&descriptor, i, prior, signature).unwrap();
        let next = EvidenceRecord::append(evidence.as_ref(), f.source.config().session_id, &message, &descriptor, receipt).unwrap();
        f.store.compare_and_set_evidence(evidence.as_ref().map(|r| r.revision), &next).unwrap();
        evidence = Some(next);
    }
    let _evidence = evidence.unwrap();
    let source_policy = stored_state(&f.store, f.source.config().session_id).policy().clone();
    let intent = signer()
        .sign(
            payload(
                f.source.config(),
                &source_policy,
                TransferTarget::Session {
                    session_id: f.target.config().session_id,
                    genesis_commitment: f.target.config().commitment(),
                },
                2,
                1,
                1,
            ),
            100,
            Some(EvidenceRequirement { message_id: message.id, pipeline_commitment: descriptor.commitment(), minimum_stage_index: 1 }),
        )
        .unwrap();
    let request = issue_request(&f.source, vec![intent]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, request);
    let proof = HandoffProofBundle::export(f.store.as_ref(), ids[0], f.target.config()).unwrap();
    let policy = HandoffImportPolicy::new(f.source.config(), f.target.config(), 1_000_000).unwrap();
    let verify = |p: &HandoffProofBundle| p.verify(&policy, f.source.config(), f.target.config(), f.verifier.as_ref(), &adapter);
    verify(&proof).unwrap();
    let mut missing = proof.clone();
    missing.evidence = None;
    assert!(verify(&missing).is_err());
    let mut forged = proof.clone();
    let record = &mut forged.evidence.as_mut().unwrap().record;
    record.receipts[1].evidence[0] ^= 1;
    // Shape, receipt count, descriptor and previous-stage commitment still match.
    record.validate_for(f.source.config().session_id, &message, &descriptor).unwrap();
    assert!(verify(&forged).is_err());
    let mut forged_message = proof.clone();
    let evidence = forged_message.evidence.as_mut().unwrap();
    let mut block = FinalisedBlockRecord::decode(&evidence.message_block).unwrap();
    block.outbox[0].payload.push(1);
    evidence.message_block = block.encode().unwrap();
    assert!(verify(&forged_message).is_err());
}

#[test]
fn independent_store_inbox_consumes_atomically_and_recovers_without_local_source_history() {
    let mut source = Fixture::new(TransferKind::CopyableReceipt);
    let source_cfg = source.source.config().clone();
    let source_genesis = source.store.load_genesis(source_cfg.session_id).unwrap();
    let source_policy = stored_state(&source.store, source_cfg.session_id).policy().clone();
    let target_state = stored_state(&source.store, source.target.config().session_id);
    let target_signer = CommitteeSigner::new("independent-target", [23; 32]).unwrap();
    let target_finality = Arc::new(
        RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
            authorities: vec![Authority { id: target_signer.validator_id().into(), public_key: target_signer.ckb_public_key() }],
        }))
        .unwrap(),
    );
    let target_cfg = config(target_finality.as_ref(), 24, &target_state);
    assert_ne!(source.verifier.descriptor().commitment(), target_finality.descriptor().commitment());
    let target_policy = target_state.policy().clone();
    let source_id = source_cfg.session_id;
    let bindings = move || {
        SourceBindings::from_configs(&target_policy, |id| {
            if id == source_id {
                Ok(source_genesis.config.clone())
            } else {
                Err(Error::Source("unknown configured source".into()))
            }
        })
        .unwrap()
    };
    let policy = HandoffImportPolicy::new(&source_cfg, &target_cfg, 1_000_000).unwrap();
    let application = Arc::new(
        TransferImportVerifier::new(
            target_state.policy().clone(),
            vec![stored_state(&source.store, source_cfg.session_id).policy().clone()],
            Arc::new(|_, _, _, _| Err(rejected("no evidence provider configured"))),
        )
        .unwrap(),
    );
    let mut registry = HandoffImportRegistry::default();
    let policy_id = registry
        .register(
            policy,
            source_cfg.clone(),
            target_cfg.clone(),
            myelin_session::RemoteFinalityVerifiers { source: source.verifier.clone(), target: target_finality.clone() },
            application,
        )
        .unwrap();
    let registry = Arc::new(registry);
    let directory = tempfile::tempdir().unwrap();
    let limits = RemoteInboxLimits { max_entries: 1, max_bytes: 4_000_000 };
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap().with_handoff_imports(registry.clone(), limits).unwrap());
    let executor = TransferExecutor::new(&target_cfg, target_state.clone(), bindings()).unwrap();
    let mut target = SessionChain::create(target_cfg.clone(), target_finality.clone(), executor, store.clone()).unwrap();
    let intent = signer()
        .sign(
            payload(
                &source_cfg,
                &source_policy,
                TransferTarget::Session { session_id: target_cfg.session_id, genesis_commitment: target_cfg.commitment() },
                1,
                0,
                1,
            ),
            100,
            None,
        )
        .unwrap();
    let request = issue_request(&source.source, vec![intent]);
    let ids = commit(&mut source.source, &source.verifier, &source.validator, request);
    let bundle = HandoffProofBundle::export(source.store.as_ref(), ids[0], &target_cfg).unwrap();
    let record = HandoffInboxRecord { policy: policy_id, proof: bundle };
    let imported = store.import_handoff(&record).unwrap();
    assert!(imported.inserted);
    assert!(imported.handoff.is_available());
    assert!(!store.import_handoff(&record).unwrap().inserted);
    assert!(store.load_genesis(source_cfg.session_id).is_err());
    assert!(store.load_block(source_cfg.session_id, 0).is_err());
    assert!(stored_state(&store, target_cfg.session_id).receipts().is_empty());
    let restored =
        TransferExecutor::restore(&target_cfg, &store.load_checkpoint(target_cfg.session_id).unwrap().state_snapshot, bindings())
            .unwrap();
    let mut other = SessionChain::recover(
        target_cfg.session_id,
        &target_cfg.application_profile,
        target_finality.clone(),
        restored,
        store.clone(),
    )
    .unwrap();
    let request = intake_request(&target, ids.clone());
    let command = request.frame_input.commands[0].clone();
    let first = target.prepare_block(request).unwrap();
    let mut request = intake_request(&other, ids.clone());
    request.timestamp_ms += 1;
    request.local_now_ms += 1;
    let second = other.prepare_block(request).unwrap();
    let second_proof = proof(&target_finality, &target_signer, &second);
    let first_proof = proof(&target_finality, &target_signer, &first);
    target.commit_finalised(first, first_proof).unwrap();
    assert!(other.commit_finalised(second, second_proof).is_err());
    let head = target.head().clone();
    let original_receipt = store.load_command_receipt(target_cfg.session_id, command.id).unwrap().unwrap();
    assert!(store.import_handoff(&record).unwrap().handoff.is_consumed());
    assert_eq!(stored_state(&store, target_cfg.session_id).receipts().len(), 1);
    let intent = signer()
        .sign(
            payload(
                &source_cfg,
                &source_policy,
                TransferTarget::Session { session_id: target_cfg.session_id, genesis_commitment: target_cfg.commitment() },
                2,
                1,
                1,
            ),
            100,
            None,
        )
        .unwrap();
    let request = issue_request(&source.source, vec![intent]);
    let more = commit(&mut source.source, &source.verifier, &source.validator, request);
    let extra = HandoffInboxRecord {
        policy: policy_id,
        proof: HandoffProofBundle::export(source.store.as_ref(), more[0], &target_cfg).unwrap(),
    };
    assert!(store.import_handoff(&extra).is_err());
    assert!(store.load_handoff(more[0]).unwrap().is_none());
    assert_eq!(store.load_head(target_cfg.session_id).unwrap(), head);
    // Destroy the producing store. Recovery and application replay can use only
    // the target's durable proof and independently configured trust.
    drop(source);
    drop(other);
    drop(target);
    drop(store);
    let unconfigured = RocksSessionStore::open(directory.path()).unwrap();
    assert!(unconfigured.load_handoff(ids[0]).is_err());
    drop(unconfigured);
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap().with_handoff_imports(registry, limits).unwrap());
    let restored =
        TransferExecutor::restore(&target_cfg, &store.load_checkpoint(target_cfg.session_id).unwrap().state_snapshot, bindings())
            .unwrap();
    let target =
        SessionChain::recover(target_cfg.session_id, &target_cfg.application_profile, target_finality, restored, store.clone())
            .unwrap();
    assert_eq!(target.head(), &head);
    assert_eq!(store.load_command_receipt(target_cfg.session_id, command.id).unwrap(), Some(original_receipt));
    let replay_config = target_cfg.clone();
    target.replay_range(0, 0, move |bytes| TransferExecutor::restore(&replay_config, bytes, bindings()).map_err(rejected)).unwrap();
    assert!(target.prepare_block(intake_request(&target, ids)).is_err());
    assert_eq!(stored_state(&store, target_cfg.session_id).receipts().len(), 1);
}

#[test]
fn conflicting_valid_source_signatures_quarantine_intake_and_survive_restart() {
    let mut first = Fixture::new(TransferKind::CopyableReceipt);
    let mut fork = Fixture::new(TransferKind::CopyableReceipt);
    assert_eq!(first.source.config(), fork.source.config());
    let cfg = first.target.config().clone();
    let initial = stored_state(&first.store, cfg.session_id);
    let mut registry = HandoffImportRegistry::default();
    let policy = HandoffImportPolicy::new(first.source.config(), &cfg, 1_000_000).unwrap();
    let policy_id = registry
        .register(
            policy,
            first.source.config().clone(),
            cfg.clone(),
            myelin_session::RemoteFinalityVerifiers { source: first.verifier.clone(), target: first.verifier.clone() },
            Arc::new(IntakeVerifier::for_fixture(&first)),
        )
        .unwrap();
    let registry = Arc::new(registry);
    let directory = tempfile::tempdir().unwrap();
    let limits = RemoteInboxLimits { max_entries: 4, max_bytes: 4_000_000 };
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap().with_handoff_imports(registry.clone(), limits).unwrap());
    let target = create_chain(cfg.clone(), initial, first.verifier.clone(), store.clone()).unwrap();
    let head = target.head().clone();
    let intent = first.intent(1, 0, 1);
    let request = issue_request(&first.source, vec![intent]);
    let ids = commit(&mut first.source, &first.verifier, &first.validator, request);
    let record =
        HandoffInboxRecord { policy: policy_id, proof: HandoffProofBundle::export(first.store.as_ref(), ids[0], &cfg).unwrap() };
    store.import_handoff(&record).unwrap();
    // Both branches have valid owner authorization and the configured source's
    // real PoA signature. They differ at the same canonical source height.
    let intent = fork.intent(2, 0, 1);
    let request = issue_request(&fork.source, vec![intent]);
    let other = commit(&mut fork.source, &fork.verifier, &fork.validator, request);
    let conflict =
        HandoffInboxRecord { policy: policy_id, proof: HandoffProofBundle::export(fork.store.as_ref(), other[0], &cfg).unwrap() };
    registry.verify(&conflict).unwrap();
    let error = store.import_handoff(&conflict).err().unwrap();
    assert!(error.to_string().contains("quarantined"));
    assert!(target.prepare_block(intake_request(&target, ids.clone())).is_err());
    assert_eq!(store.load_head(cfg.session_id).unwrap(), head);
    assert!(stored_state(&store, cfg.session_id).receipts().is_empty());
    drop(target);
    drop(store);
    let store = RocksSessionStore::open(directory.path()).unwrap().with_handoff_imports(registry, limits).unwrap();
    assert!(store.import_handoff(&record).is_err());
    assert!(store.load_handoff_import(ids[0]).is_err());
    assert_eq!(store.load_head(cfg.session_id).unwrap(), head);
    assert!(stored_state(&store, cfg.session_id).receipts().is_empty());
}
