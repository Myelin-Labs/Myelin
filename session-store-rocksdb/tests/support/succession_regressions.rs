//! A signed reward ledger exercises actual state conversion, not a supplied-root
//! acceptance stub. Conversion retains balances/nullifiers and resets only score.
use super::*;
use myelin_session::{
    CommandIdentity, FinalisedBlockRecord, MigrationArtifact, PredecessorReference, ProtocolEffectsContext, ProtocolEffectsPolicy,
    SessionError, SessionHandoff, SuccessorState,
};
use secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, SecretKey};
use std::collections::BTreeSet;

const POLICY: [u8; 32] = [83; 32];
// Versioned migration bytecode: magic, version, output codec, reset-score flag.
const MIGRATION: &[u8] = b"MIGR\x01\x02\x01";
const DECLARATION: &[u8] = b"reward ledger: identical balances, score and claimed-reward interpretation";
const VECTORS: &[u8] = b"0,0,5,1;9,3,14,4;duplicate-reward-rejected";
fn digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}
fn owner() -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[42; 32]).unwrap())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ledger {
    codec: u8,
    balance: u64,
    score: u64,
    claimed: BTreeSet<[u8; 32]>,
    corrupt_migration: bool,
}
impl Ledger {
    fn initial() -> Self {
        Self { codec: 1, balance: 100, score: 7, claimed: BTreeSet::from([[17; 32]]), corrupt_migration: false }
    }
    fn restore(bytes: &[u8]) -> myelin_session::Result<Self> {
        let offset = match bytes.first() {
            Some(1) => 1,
            Some(2) if bytes.get(1) == Some(&0) => 2,
            _ => return Err(SessionError::Execution("unknown ledger codec".into())),
        };
        if bytes.len() < offset + 16 || !(bytes.len() - offset - 16).is_multiple_of(32) {
            return Err(SessionError::Execution("invalid ledger snapshot".into()));
        }
        let mut claimed = BTreeSet::new();
        for hash in bytes[offset + 16..].chunks_exact(32) {
            if !claimed.insert(hash.try_into().unwrap()) {
                return Err(SessionError::Execution("duplicate nullifier".into()));
            }
        }
        let ledger = Self {
            codec: bytes[0],
            balance: u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()),
            score: u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap()),
            claimed,
            corrupt_migration: false,
        };
        if ledger.snapshot().unwrap() != bytes {
            return Err(SessionError::Execution("noncanonical snapshot".into()));
        }
        Ok(ledger)
    }
    fn claim(&mut self, id: [u8; 32]) -> Result<(), String> {
        if !self.claimed.insert(id) {
            return Err("reward already claimed".into());
        }
        self.balance = self.balance.checked_add(5).ok_or("balance overflow")?;
        self.score = self.score.checked_add(1).ok_or("score overflow")?;
        Ok(())
    }
    fn migrate(&self, artifact: &[u8], inputs: &[u8]) -> Result<Self, String> {
        if artifact != MIGRATION || inputs != [1, 2] || self.codec != 1 {
            return Err("unsupported migration program or inputs".into());
        }
        let mut next = self.clone();
        next.codec = artifact[5];
        if artifact[6] == 1 {
            next.score = 0;
        }
        if self.corrupt_migration {
            next.balance += 1;
        }
        Ok(next)
    }
    fn check_vectors() -> Result<(), String> {
        for (balance, score, expected_balance, expected_score) in [(0, 0, 5, 1), (9, 3, 14, 4)] {
            let mut a = Self::initial();
            a.balance = balance;
            a.score = score;
            a.claim([18; 32])?;
            if (a.balance, a.score) != (expected_balance, expected_score) || a.claim([18; 32]).is_ok() {
                return Err("compatibility vector failed".into());
            }
        }
        Ok(())
    }
}
fn authorization(genesis: [u8; 32], root: [u8; 32], op: &[u8], target: Option<&SessionConfig>) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"succession-test-owner-authorization");
    h.update(&genesis);
    h.update(&root);
    h.update(&(op.len() as u64).to_le_bytes());
    h.update(op);
    h.update(&target.map(SessionConfig::commitment).unwrap_or([0; 32]));
    *h.finalize().as_bytes()
}
impl TransitionExecutor for Ledger {
    fn state_root(&self) -> [u8; 32] {
        digest(&self.snapshot().unwrap())
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        let mut out = vec![self.codec];
        if self.codec == 2 {
            out.push(0);
        }
        out.extend(self.balance.to_le_bytes());
        out.extend(self.score.to_le_bytes());
        for hash in &self.claimed {
            out.extend(hash);
        }
        Ok(out)
    }
    fn execute_block(&mut self, _: u64, _: &FrameInput, _: &[SessionHandoff], _: &[Vec<u8>]) -> Result<ExecutionOutcome, String> {
        Err("signed context required".into())
    }
    fn execute_block_with_context(
        &mut self,
        c: &ProtocolEffectsContext,
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        c.validate().map_err(|e| e.to_string())?;
        if c.policy != (ProtocolEffectsPolicy::Application { policy_hash: POLICY })
            || c.input_root != input.root()
            || c.pre_state_root != self.state_root()
            || !handoffs.is_empty()
            || !c.issued_handoffs.is_empty()
            || !txs.is_empty()
            || input.payload.len() < 65
        {
            return Err("unsupported ledger context".into());
        }
        let target = c.successor().map_err(|e| e.to_string())?;
        let split = input.payload.len() - 64;
        let (op, signature) = input.payload.split_at(split);
        let signature = Signature::from_slice(signature).map_err(|e| e.to_string())?;
        Secp256k1::verification_only()
            .verify_schnorr(
                &signature,
                &Message::from_digest(authorization(c.source_genesis_commitment, c.pre_state_root, op, target.as_ref())),
                &owner().x_only_public_key().0,
            )
            .map_err(|_| "invalid session-bound owner signature")?;
        match (op, target) {
            ([0], Some(_)) => {}
            (op, None) if op.len() == 33 && op[0] == 1 => self.claim(op[1..].try_into().unwrap())?,
            _ => return Err("unsupported ledger operation".into()),
        }
        Ok(ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![], outbox: vec![], cycles: 0 })
    }
    fn prepare_successor_state(
        &self,
        source: &SessionConfig,
        target: &SessionConfig,
        _: &FrameInput,
    ) -> Result<SuccessorState, String> {
        if source.application_profile == target.application_profile {
            return Ok(SuccessorState::Continue);
        }
        let mut compatible = source.application_profile.clone();
        compatible.program_digest = [21; 32];
        if target.application_profile == compatible && target.protocol_effects == source.protocol_effects {
            Self::check_vectors()?;
            return Ok(SuccessorState::Compatible {
                policy_hash: POLICY,
                declaration_hash: digest(DECLARATION),
                vectors_hash: digest(VECTORS),
            });
        }
        let mut migrated = source.application_profile.clone();
        migrated.program_digest = [22; 32];
        migrated.state_codec_hash = [23; 32];
        if target.application_profile == migrated && target.protocol_effects == source.protocol_effects {
            let inputs = vec![1, 2];
            let next = self.migrate(MIGRATION, &inputs)?;
            return Ok(SuccessorState::Migrate {
                artifact: MigrationArtifact {
                    format_version: 1,
                    program_digest: digest(MIGRATION),
                    input_schema_hash: digest(b"source-codec,target-codec"),
                },
                inputs,
                state_root: next.state_root(),
                snapshot: next.snapshot()?,
            });
        }
        Err("unsupported rule upgrade".into())
    }
}
fn cfg(v: &RegisteredFinalityVerifier, ledger: &Ledger) -> SessionConfig {
    let mut c = config(v);
    c.initial_state_root = ledger.state_root();
    c.protocol_effects = ProtocolEffectsPolicy::Application { policy_hash: POLICY };
    c.application_profile.command_identity_required = true;
    c
}
fn target(source: &SessionConfig, ledger: &Ledger, kind: u8) -> SessionConfig {
    let mut c = source.clone();
    c.session_id = [4; 32];
    c.initial_input_position = 1;
    c.initial_logical_time = 1;
    c.initial_timestamp_ms = 10;
    c.predecessor = Some(PredecessorReference {
        session_id: source.session_id,
        final_height: 0,
        final_state_root: ledger.state_root(),
        application_profile_commitment: source.application_profile.commitment(),
    });
    if kind == 1 {
        c.application_profile.program_digest = [21; 32];
    }
    if kind == 2 {
        c.application_profile.program_digest = [22; 32];
        c.application_profile.state_codec_hash = [23; 32];
        c.initial_state_root = ledger.migrate(MIGRATION, &[1, 2]).unwrap().state_root();
    }
    c
}
fn request(source: &SessionConfig, ledger: &Ledger, target: Option<SessionConfig>, claim: Option<[u8; 32]>) -> BlockRequest {
    let mut op = vec![0];
    if let Some(id) = claim {
        op = vec![1];
        op.extend(id);
    }
    let sig = Secp256k1::new().sign_schnorr_no_aux_rand(
        &Message::from_digest(authorization(source.commitment(), ledger.state_root(), &op, target.as_ref())),
        &owner(),
    );
    op.extend(sig.serialize());
    let mut input = frame_input(source.initial_input_position);
    input.commands = vec![CommandIdentity { id: [92; 32], payload_hash: digest(&op) }];
    input.payload = op;
    BlockRequest {
        transactions: vec![],
        frame_input: input,
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: 10,
        local_now_ms: 10,
        successor: target,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    }
}
fn seal<E: TransitionExecutor>(
    v: &RegisteredFinalityVerifier,
    signer: &CommitteeSigner,
    p: &myelin_session::PreparedBlock<E>,
) -> FinalityProof {
    let myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) = v.selected_consensus() else { unreachable!() };
    FinalityProof::ProofOfAuthority(engine.seal_from_signer(p.block().hash(), p.block().number, signer).unwrap())
}

#[test]
fn same_codec_program_and_policy_changes_require_explicit_successor_rules() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, _) = consensus();
    let c = config(&v);
    let chain = SessionChain::create(c.clone(), v, TestExecutor { root: [1; 32] }, store.clone()).unwrap();
    let input = frame_input(0);
    let mut next = c.clone();
    next.session_id = [4; 32];
    next.initial_state_root = expected_root([1; 32], 0, &input, &[], &[]);
    next.initial_input_position = 1;
    next.initial_logical_time = 1;
    next.initial_timestamp_ms = 10;
    next.predecessor = Some(PredecessorReference {
        session_id: c.session_id,
        final_height: 0,
        final_state_root: next.initial_state_root,
        application_profile_commitment: c.application_profile.commitment(),
    });
    for change in 0..3 {
        let mut target = next.clone();
        match change {
            0 => target.application_profile.program_digest = [20; 32],
            1 => target.application_profile.entropy_policy_hash = [20; 32],
            _ => target.protocol_effects = ProtocolEffectsPolicy::Disabled,
        }
        let req = BlockRequest {
            transactions: vec![],
            frame_input: input.clone(),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 10,
            local_now_ms: 10,
            successor: Some(target),
            handoffs: vec![],
            consume_handoffs: vec![],
            handoff_actions: vec![],
        };
        assert!(chain.prepare_block(req).err().unwrap().to_string().contains("explicit compatibility"));
    }
    assert!(store.load_genesis([4; 32]).is_err());
    assert_eq!(store.load_head(c.session_id).unwrap().finalised_height, None);
}

#[test]
fn compatible_and_incompatible_upgrades_roundtrip_replay_and_recover() {
    for kind in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (v, signer) = consensus();
        let ledger = Ledger::initial();
        let c = cfg(&v, &ledger);
        let next = target(&c, &ledger, kind);
        let mut chain = SessionChain::create(c.clone(), v.clone(), ledger.clone(), store.clone()).unwrap();
        let req = request(&c, &ledger, Some(next.clone()), None);
        let prepared = chain.prepare_block(req.clone()).unwrap();
        let proof = seal(&v, &signer, &prepared);
        chain.commit_finalised(prepared, proof).unwrap();
        let record = store.load_block(c.session_id, 0).unwrap();
        assert_eq!(&record.encode().unwrap()[4..6], &myelin_session::RECORD_FORMAT_VERSION.to_le_bytes());
        assert_eq!(FinalisedBlockRecord::decode_for_config(&record.encode().unwrap(), &c).unwrap(), record);
        assert_eq!(record.encoded_size_with_limit(u64::MAX).unwrap(), record.encode().unwrap().len() as u64);
        chain.replay_range(0, 0, Ledger::restore).unwrap();
        let genesis = store.load_genesis(next.session_id).unwrap();
        let converted = Ledger::restore(&genesis.state_snapshot).unwrap();
        assert_eq!(converted.balance, ledger.balance);
        assert_eq!(converted.claimed, ledger.claimed);
        assert_eq!(converted.score, if kind == 2 { 0 } else { ledger.score });
        let successor =
            SessionChain::recover(next.session_id, &next.application_profile, v.clone(), converted.clone(), store.clone()).unwrap();
        // The new command index is empty; retained economic nullifiers still reject
        // a freshly signed duplicate award under the new session namespace.
        assert!(successor
            .prepare_block(request(&next, &converted, None, Some([17; 32])))
            .err()
            .unwrap()
            .to_string()
            .contains("reward already claimed"));
        let mut reused = request(&next, &converted, None, Some([18; 32]));
        reused.frame_input.payload = req.frame_input.payload;
        reused.frame_input.commands[0].payload_hash = digest(&reused.frame_input.payload);
        assert!(successor.prepare_block(reused).err().unwrap().to_string().contains("invalid session-bound"));
        assert!(chain.prepare_block(request(&c, &ledger, Some(next.clone()), None)).is_err());
        drop(successor);
        drop(chain);
        drop(store);
        let reopened = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let recovered = SessionChain::recover(c.session_id, &c.application_profile, v.clone(), ledger, reopened.clone()).unwrap();
        assert_eq!(recovered.head().sealed_by_successor, Some(next.session_id));
        SessionChain::recover(next.session_id, &next.application_profile, v, converted, reopened).unwrap();
    }
}

#[test]
fn migration_root_receipt_inputs_and_resource_tampering_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, signer) = consensus();
    let ledger = Ledger::initial();
    let c = cfg(&v, &ledger);
    let next = target(&c, &ledger, 2);
    let mut chain = SessionChain::create(c.clone(), v.clone(), ledger.clone(), store.clone()).unwrap();
    let mut wrong = next.clone();
    wrong.initial_state_root[0] ^= 1;
    assert!(chain.prepare_block(request(&c, &ledger, Some(wrong), None)).is_err());
    let mut small = next.clone();
    small.commit_limits.max_snapshot_bytes = 1;
    assert!(chain.prepare_block(request(&c, &ledger, Some(small), None)).is_err());
    let p = chain.prepare_block(request(&c, &ledger, Some(next), None)).unwrap();
    let proof = seal(&v, &signer, &p);
    chain.commit_finalised(p, proof).unwrap();
    let record = store.load_block(c.session_id, 0).unwrap();
    for change in 0..6 {
        let mut bad = record.clone();
        let receipt = &mut bad.successor.as_mut().unwrap().transition;
        match change {
            0 => receipt.source_genesis_commitment[0] ^= 1,
            1 => receipt.input_root[0] ^= 1,
            2 => receipt.source_snapshot_hash[0] ^= 1,
            3 => {
                if let SuccessorState::Migrate { artifact, .. } = &mut receipt.state {
                    artifact.format_version = 2;
                }
            }
            4 => {
                if let SuccessorState::Migrate { state_root, .. } = &mut receipt.state {
                    state_root[0] ^= 1;
                }
            }
            _ => bad.block.version = 3,
        }
        assert!(bad.validate_resources(&c).is_err());
    }
    // Even an internally consistent, signed producer's altered conversion is
    // checked again by the approved executor during replay (separate test below).
    assert!(chain
        .replay_range(0, 0, |bytes| {
            let mut l = Ledger::restore(bytes)?;
            l.corrupt_migration = true;
            Ok(l)
        })
        .is_err());
}

#[test]
fn rotated_authority_cannot_reinterpret_or_finalize_the_old_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (old, old_signer) = consensus();
    let new_signer = CommitteeSigner::new("new-authority", [8; 32]).unwrap();
    let new = Arc::new(
        RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
            authorities: vec![Authority { id: new_signer.validator_id().into(), public_key: new_signer.ckb_public_key() }],
        }))
        .unwrap(),
    );
    let ledger = Ledger::initial();
    let c = cfg(&old, &ledger);
    let mut next = target(&c, &ledger, 0);
    let authority = config(&new);
    next.consensus_config_commitment = authority.consensus_config_commitment;
    next.consensus_module_commitment = authority.consensus_module_commitment;
    next.consensus_wal_schema_hash = authority.consensus_wal_schema_hash;
    let mut source = SessionChain::create(c.clone(), old.clone(), ledger.clone(), store.clone()).unwrap();
    let p = source.prepare_block(request(&c, &ledger, Some(next.clone()), None)).unwrap();
    let wrong = seal(&new, &new_signer, &p);
    assert!(source.commit_finalised(p, wrong).is_err());
    let p = source.prepare_block(request(&c, &ledger, Some(next.clone()), None)).unwrap();
    let proof = seal(&old, &old_signer, &p);
    source.commit_finalised(p, proof).unwrap();
    assert!(SessionChain::recover(c.session_id, &c.application_profile, new.clone(), ledger.clone(), store.clone()).is_err());
    assert!(SessionChain::recover(next.session_id, &next.application_profile, old.clone(), ledger.clone(), store.clone()).is_err());
    let mut successor =
        SessionChain::recover(next.session_id, &next.application_profile, new.clone(), ledger.clone(), store.clone()).unwrap();
    // Isolate session binding: same state root, reward and operation shape,
    // with no successor target in either signed reward request.
    let old_reward = request(&c, &ledger, None, Some([18; 32]));
    let mut replayed = request(&next, &ledger, None, Some([18; 32]));
    replayed.frame_input.payload = old_reward.frame_input.payload;
    replayed.frame_input.commands[0].payload_hash = digest(&replayed.frame_input.payload);
    assert!(successor.prepare_block(replayed).err().unwrap().to_string().contains("invalid session-bound"));
    let p = successor.prepare_block(request(&next, &ledger, None, Some([18; 32]))).unwrap();
    let wrong = seal(&old, &old_signer, &p);
    assert!(successor.commit_finalised(p, wrong).is_err());
    let p = successor.prepare_block(request(&next, &ledger, None, Some([18; 32]))).unwrap();
    let proof = seal(&new, &new_signer, &p);
    successor.commit_finalised(p, proof).unwrap();
    source.replay_range(0, 0, Ledger::restore).unwrap();
    successor.replay_range(0, 0, Ledger::restore).unwrap();
}

#[test]
fn signed_forged_migration_inputs_pass_structural_audit_but_fail_execution_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path().join("honest")).unwrap());
    let (v, signer) = consensus();
    let ledger = Ledger::initial();
    let c = cfg(&v, &ledger);
    let next = target(&c, &ledger, 2);
    let mut source = SessionChain::create(c.clone(), v.clone(), ledger.clone(), store.clone()).unwrap();
    let initial = source.head().clone();
    let p = source.prepare_block(request(&c, &ledger, Some(next), None)).unwrap();
    let proof = seal(&v, &signer, &p);
    source.commit_finalised(p, proof).unwrap();
    let mut record = store.load_block(c.session_id, 0).unwrap();
    let mut head = source.head().clone();
    let declaration = record.successor.as_mut().unwrap();
    let old_commitment = declaration.commitment();
    let SuccessorState::Migrate { inputs, .. } = &mut declaration.transition.state else { unreachable!() };
    inputs[1] = 3;
    let commitment = declaration.commitment();
    *record.block.data_commitments.iter_mut().find(|h| **h == old_commitment).unwrap() = commitment;
    let myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) = v.selected_consensus() else { unreachable!() };
    record.proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(record.block.hash(), 0, &signer).unwrap());
    head.block_hash = record.block.hash();
    let forged = Arc::new(RocksSessionStore::open(dir.path().join("forged")).unwrap());
    SessionChain::create(c.clone(), v.clone(), ledger.clone(), forged.clone()).unwrap();
    forged.commit_block(&initial, &head, &record).unwrap();
    let chain = SessionChain::recover(c.session_id, &c.application_profile, v, ledger, forged).unwrap();
    assert!(chain.replay_range(0, 0, Ledger::restore).err().unwrap().to_string().contains("successor transition mismatch"));
}

#[test]
fn migration_crash_child() {
    let Some(path) = std::env::var_os("MYELIN_SUCCESSION_CRASH_TEST_PATH") else { return };
    let store = Arc::new(RocksSessionStore::open(path).unwrap());
    let (v, signer) = consensus();
    let ledger = Ledger::initial();
    let c = cfg(&v, &ledger);
    let next = target(&c, &ledger, 2);
    let mut source = SessionChain::create(c.clone(), v.clone(), ledger.clone(), store).unwrap();
    let p = source.prepare_block(request(&c, &ledger, Some(next), None)).unwrap();
    if std::env::var("MYELIN_SUCCESSION_CRASH_TEST_PHASE").unwrap() == "after" {
        let proof = seal(&v, &signer, &p);
        source.commit_finalised(p, proof).unwrap();
    }
    std::process::exit(73);
}

#[test]
fn migration_process_exit_preserves_atomic_source_seal_and_unique_target() {
    for phase in ["before", "after"] {
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "succession_regressions::migration_crash_child", "--nocapture"])
            .env("MYELIN_SUCCESSION_CRASH_TEST_PATH", dir.path())
            .env("MYELIN_SUCCESSION_CRASH_TEST_PHASE", phase)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(73), "{output:?}");
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (v, signer) = consensus();
        let ledger = Ledger::initial();
        let c = cfg(&v, &ledger);
        let next = target(&c, &ledger, 2);
        let mut source =
            SessionChain::recover(c.session_id, &c.application_profile, v.clone(), ledger.clone(), store.clone()).unwrap();
        if phase == "before" {
            assert_eq!(source.head().finalised_height, None);
            assert!(store.load_genesis(next.session_id).is_err());
            let p = source.prepare_block(request(&c, &ledger, Some(next.clone()), None)).unwrap();
            let proof = seal(&v, &signer, &p);
            source.commit_finalised(p, proof).unwrap();
        }
        assert_eq!(source.head().sealed_by_successor, Some(next.session_id));
        assert_eq!(store.load_chain_page(c.session_id, 0, 2).unwrap().len(), 1);
        let genesis = store.load_genesis(next.session_id).unwrap();
        assert_eq!(Ledger::restore(&genesis.state_snapshot).unwrap(), ledger.migrate(MIGRATION, &[1, 2]).unwrap());
        assert!(source.prepare_block(request(&c, &ledger, Some(next), None)).is_err());
        source.replay_range(0, 0, Ledger::restore).unwrap();
    }
}
