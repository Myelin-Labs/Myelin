//! Adversarial application-policy checks through real live/replay/store paths.
//! This small ledger is a test policy, not the shared-store cancellation/refund
//! reference application required by #11.
use super::*;
use myelin_session::{
    EffectAuthorizationScope, FinalisedBlockRecord, ProtocolEffectsContext, ProtocolEffectsPolicy, SessionError, SessionHandoff,
};
use secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};

const POLICY: [u8; 32] = [73; 32];
const OBJECT: [u8; 32] = [74; 32];
const NONCE: [u8; 32] = [75; 32];

fn actor() -> [u8; 32] {
    keypair().x_only_public_key().0.serialize()
}
fn keypair() -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[3; 32]).unwrap())
}
fn request() -> BlockRequest {
    BlockRequest {
        transactions: vec![],
        frame_input: frame_input(0),
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: 10,
        local_now_ms: 10,
        successor: None,
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
fn scope() -> EffectAuthorizationScope {
    EffectAuthorizationScope { actor: actor(), object: OBJECT, object_version: 0, nonce: NONCE }
}
fn signed(genesis: [u8; 32], scope: &EffectAuthorizationScope, digest: [u8; 32]) -> Vec<u8> {
    let mut bytes = genesis.to_vec();
    bytes.extend(scope.actor);
    bytes.extend(scope.object);
    bytes.extend(scope.object_version.to_le_bytes());
    bytes.extend(scope.nonce);
    bytes.extend(Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(digest), &keypair()).serialize());
    bytes
}
fn decode_auth(bytes: &[u8]) -> Result<([u8; 32], EffectAuthorizationScope, Signature), String> {
    if bytes.len() != 200 {
        return Err("invalid owner authorization encoding".into());
    }
    Ok((
        bytes[0..32].try_into().unwrap(),
        EffectAuthorizationScope {
            actor: bytes[32..64].try_into().unwrap(),
            object: bytes[64..96].try_into().unwrap(),
            object_version: u64::from_le_bytes(bytes[96..104].try_into().unwrap()),
            nonce: bytes[104..136].try_into().unwrap(),
        },
        Signature::from_slice(&bytes[136..200]).map_err(|e| e.to_string())?,
    ))
}
fn verify(scope: &EffectAuthorizationScope, digest: [u8; 32], sig: &Signature) -> Result<(), String> {
    let pk = XOnlyPublicKey::from_slice(&scope.actor).map_err(|e| e.to_string())?;
    Secp256k1::verification_only()
        .verify_schnorr(sig, &Message::from_digest(digest), &pk)
        .map_err(|_| "invalid owner signature".into())
}
fn payload(recipient: [u8; 32], amount: u64) -> Vec<u8> {
    let mut bytes = OBJECT.to_vec();
    bytes.extend(recipient);
    bytes.extend(amount.to_le_bytes());
    bytes
}
fn parse_payload(bytes: &[u8]) -> Result<([u8; 32], u64), String> {
    if bytes.len() != 72 || bytes[..32] != OBJECT {
        return Err("wrong transfer object/payload".into());
    }
    let amount = u64::from_le_bytes(bytes[64..72].try_into().unwrap());
    if amount == 0 {
        return Err("zero transfer".into());
    }
    Ok((bytes[32..64].try_into().unwrap(), amount))
}
fn intent(cfg: &SessionConfig, recipient: [u8; 32], amount: u64) -> HandoffIntent {
    let mut intent = HandoffIntent {
        target: HandoffTarget::Session([4; 32]),
        expires_at_ms: 100,
        payload: payload(recipient, amount),
        authorization: vec![],
        evidence_requirement: None,
    };
    let scope = scope();
    intent.authorization = signed(cfg.commitment(), &scope, scope.handoff_digest(cfg.commitment(), &intent).unwrap());
    intent
}
fn lock_command(req: &mut BlockRequest, amount: u64) {
    req.frame_input.payload = amount.to_le_bytes().to_vec();
    req.frame_input.payload.extend(NONCE);
}

#[derive(Clone)]
struct Ledger {
    owner: [u8; 32],
    available: u64,
    locked: u64,
    version: u64,
    last_nonce: [u8; 32],
    source_anchor: [u8; 32],
    // Only the deliberately bad producer uses false. Replay restores true.
    check_policy: bool,
}
impl Ledger {
    fn source() -> Self {
        Self { owner: actor(), available: 100, locked: 0, version: 0, last_nonce: [0; 32], source_anchor: [0; 32], check_policy: true }
    }
    fn restore(bytes: &[u8]) -> myelin_session::Result<Self> {
        if bytes.len() != 120 {
            return Err(SessionError::Execution("invalid ledger snapshot".into()));
        }
        Ok(Self {
            owner: bytes[..32].try_into().unwrap(),
            available: u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
            locked: u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
            version: u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            last_nonce: bytes[56..88].try_into().unwrap(),
            source_anchor: bytes[88..120].try_into().unwrap(),
            check_policy: true,
        })
    }
}
impl TransitionExecutor for Ledger {
    fn state_root(&self) -> [u8; 32] {
        *blake3::hash(&self.snapshot().unwrap()).as_bytes()
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        let mut b = self.owner.to_vec();
        b.extend(self.available.to_le_bytes());
        b.extend(self.locked.to_le_bytes());
        b.extend(self.version.to_le_bytes());
        b.extend(self.last_nonce);
        b.extend(self.source_anchor);
        Ok(b)
    }
    fn execute_block(
        &mut self,
        _: u64,
        input: &FrameInput,
        consumed: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        if !txs.is_empty() {
            return Err("test policy accepts only ledger commands".into());
        }
        if input.payload.len() == 40 {
            let amount = u64::from_le_bytes(input.payload[..8].try_into().unwrap());
            self.available = self.available.checked_sub(amount).ok_or("insufficient inventory")?;
            self.locked = self.locked.checked_add(amount).ok_or("inventory overflow")?;
            self.version = self.version.checked_add(1).ok_or("version overflow")?;
            self.last_nonce = input.payload[8..].try_into().unwrap();
        }
        for handoff in consumed {
            let (_, amount) = parse_payload(&handoff.payload)?;
            self.available = self.available.checked_add(amount).ok_or("inventory overflow")?;
        }
        Ok(ExecutionOutcome {
            ordered_cell_tx_commitments: vec![],
            data_commitments: vec![],
            outbox: vec![PendingOutboxMessage { topic: "reference-policy/state".into(), payload: self.state_root().to_vec() }],
            cycles: 0,
        })
    }
    fn execute_block_with_context(
        &mut self,
        c: &ProtocolEffectsContext,
        input: &FrameInput,
        consumed: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        if !self.check_policy {
            return self.execute_block(c.height, input, consumed, txs);
        }
        c.validate().map_err(|e| e.to_string())?;
        if c.policy != (ProtocolEffectsPolicy::Application { policy_hash: POLICY })
            || c.pre_state_root != self.state_root()
            || c.input_root != input.root()
        {
            return Err("wrong application effect policy/context".into());
        }
        if let Some(target) = c.successor().map_err(|e| e.to_string())? {
            if !c.issued_handoffs.is_empty() || !consumed.is_empty() {
                return Err("unsupported mixed migration".into());
            }
            let (genesis, auth, sig) = decode_auth(&input.payload)?;
            if genesis != c.source_genesis_commitment || auth.actor != self.owner || auth.object_version != self.version {
                return Err("wrong migration owner/version/genesis".into());
            }
            verify(&auth, auth.successor_digest(genesis, &target).map_err(|e| e.to_string())?, &sig)?;
            return self.execute_block(c.height, input, consumed, txs);
        }
        if c.issued_handoffs.len() > 1 {
            return Err("reference policy only issues one transfer at a time".into());
        }
        let mut debit = 0;
        for intent in &c.issued_handoffs {
            let (genesis, auth, sig) = decode_auth(&intent.authorization)?;
            if genesis != c.source_genesis_commitment
                || auth.actor != self.owner
                || auth.object != OBJECT
                || auth.object_version != self.version
                || auth.nonce == self.last_nonce
            {
                return Err("wrong source owner/object/version/nonce/genesis".into());
            }
            verify(&auth, auth.handoff_digest(genesis, intent).map_err(|e| e.to_string())?, &sig)?;
            let (_, amount) = parse_payload(&intent.payload)?;
            debit = amount;
            if input.payload.len() != 40 || input.payload[8..] != auth.nonce {
                return Err("missing source lock/nonce command".into());
            }
        }
        for h in consumed {
            let intent = HandoffIntent {
                target: h.target.clone(),
                expires_at_ms: h.expires_at_ms,
                payload: h.payload.clone(),
                authorization: h.authorization.clone(),
                evidence_requirement: h.evidence_requirement.clone(),
            };
            let (genesis, auth, sig) = decode_auth(&h.authorization)?;
            if genesis != self.source_anchor {
                return Err("untrusted source genesis".into());
            }
            verify(&auth, auth.handoff_digest(genesis, &intent).map_err(|e| e.to_string())?, &sig)?;
            if parse_payload(&h.payload)?.0 != self.owner {
                return Err("wrong recipient".into());
            }
        }
        let before = self.clone();
        let outcome = self.execute_block(c.height, input, consumed, txs)?;
        if !c.issued_handoffs.is_empty()
            && (self.available.checked_add(debit) != Some(before.available) || before.locked.checked_add(debit) != Some(self.locked))
        {
            return Err("handoff did not lock the exact source amount".into());
        }
        Ok(outcome)
    }
}
fn ledger_config(v: &RegisteredFinalityVerifier, ledger: &Ledger) -> SessionConfig {
    let mut cfg = config(v);
    cfg.initial_state_root = ledger.state_root();
    cfg.protocol_effects = ProtocolEffectsPolicy::Application { policy_hash: POLICY };
    cfg
}

#[test]
fn owner_authorization_and_source_disposition_fail_closed_without_durable_effects() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, _) = consensus();
    let ledger = Ledger::source();
    let cfg = ledger_config(&v, &ledger);
    let chain = SessionChain::create(cfg.clone(), v, ledger, store.clone()).unwrap();
    let mut valid = request();
    valid.handoffs.push(intent(&cfg, [8; 32], 10));
    lock_command(&mut valid, 10);
    assert!(chain.prepare_block(valid.clone()).is_ok());
    for mutation in 0..13 {
        let mut req = valid.clone();
        match mutation {
            0 => req.handoffs[0].payload[32] ^= 1,       // recipient
            1 => req.handoffs[0].payload[64] ^= 1,       // amount
            2 => req.handoffs[0].payload[0] ^= 1,        // object
            3 => req.handoffs[0].authorization[96] ^= 1, // object version
            4 => req.handoffs[0].expires_at_ms += 1,
            5 => req.handoffs[0].target = HandoffTarget::Session([9; 32]),
            6 => req.handoffs[0].authorization = vec![1; 200],
            7 => req.frame_input.payload = vec![0], // no debit command
            8 => lock_command(&mut req, 9),         // wrong debit
            9 => req.handoffs.push(req.handoffs[0].clone()),
            10 => req.handoffs[0].authorization[0] ^= 1,   // source genesis
            11 => req.handoffs[0].authorization[104] ^= 1, // nonce
            12 => {
                req.handoffs[0].evidence_requirement =
                    Some(EvidenceRequirement { message_id: [1; 32], pipeline_commitment: [2; 32], minimum_stage_index: 0 })
            }
            _ => unreachable!(),
        }
        assert!(chain.prepare_block(req).is_err(), "mutation {mutation} accepted");
        assert_eq!(store.load_head(cfg.session_id).unwrap(), *chain.head());
        assert_eq!(store.load_checkpoint(cfg.session_id).unwrap().finalised_height, None);
        assert!(store.pending_outbox(cfg.session_id, 10).unwrap().is_empty());
    }
}

#[test]
fn valid_lock_and_target_credit_replay_after_restart_with_identical_effect_context() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, signer) = consensus();
    let ledger = Ledger::source();
    let cfg = ledger_config(&v, &ledger);
    let mut source = SessionChain::create(cfg.clone(), v.clone(), ledger, store.clone()).unwrap();
    let mut req = request();
    req.handoffs.push(intent(&cfg, [8; 32], 10));
    lock_command(&mut req, 10);
    let prepared = source.prepare_block(req).unwrap();
    let handoff_id = prepared.issued_handoffs()[0].id();
    let proof = seal(&v, &signer, &prepared);
    source.commit_finalised(prepared, proof).unwrap();
    let record = store.load_block(cfg.session_id, 0).unwrap();
    let context_hash = record.effects_context(&cfg).unwrap().commitment().unwrap();
    assert_eq!(record.block.data_commitments[record.block.data_commitments.len() - 2], context_hash);
    let after = Ledger::restore(&store.load_checkpoint(cfg.session_id).unwrap().state_snapshot).unwrap();
    assert_eq!((after.available, after.locked), (90, 10));
    let target_ledger = Ledger { owner: [8; 32], available: 0, source_anchor: cfg.commitment(), ..Ledger::source() };
    let mut target_cfg = ledger_config(&v, &target_ledger);
    target_cfg.session_id = [4; 32];
    let mut target = SessionChain::create(target_cfg.clone(), v.clone(), target_ledger, store.clone()).unwrap();
    let mut req = request();
    req.frame_input.payload = vec![0];
    req.consume_handoffs.push(handoff_id);
    let prepared = target.prepare_block(req).unwrap();
    let proof = seal(&v, &signer, &prepared);
    target.commit_finalised(prepared, proof).unwrap();
    assert_eq!(Ledger::restore(&store.load_checkpoint([4; 32]).unwrap().state_snapshot).unwrap().available, 10);
    assert!(store.load_handoff(handoff_id).unwrap().unwrap().is_consumed());
    source.replay_range(0, 0, Ledger::restore).unwrap();
    target.replay_range(0, 0, Ledger::restore).unwrap();
    drop(source);
    drop(target);
    drop(store);
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let checkpoint = store.load_checkpoint(cfg.session_id).unwrap();
    let recovered = SessionChain::recover(
        cfg.session_id,
        &cfg.application_profile,
        v,
        Ledger::restore(&checkpoint.state_snapshot).unwrap(),
        store.clone(),
    )
    .unwrap();
    recovered.replay_range(0, 0, Ledger::restore).unwrap();
    assert_eq!(store.load_block(cfg.session_id, 0).unwrap().effects_context(&cfg).unwrap().commitment().unwrap(), context_hash);
    // Reusing the old authorized intent after the version/nonce was consumed fails.
    let mut retry = request();
    retry.frame_input.start_position = 1;
    retry.frame_input.end_position = 2;
    retry.frame_input.logical_time_start = 1;
    retry.frame_input.logical_time_end = 2;
    retry.handoffs.push(intent(&cfg, [8; 32], 10));
    lock_command(&mut retry, 10);
    assert!(recovered.prepare_block(retry).is_err());
}

#[test]
fn source_replay_rejects_a_signed_structurally_valid_but_unauthorized_declaration() {
    for missing_lock in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (v, signer) = consensus();
        let mut ledger = Ledger::source();
        let cfg = ledger_config(&v, &ledger);
        ledger.check_policy = false;
        let mut chain = SessionChain::create(cfg.clone(), v.clone(), ledger, store.clone()).unwrap();
        let mut req = request();
        req.handoffs.push(intent(&cfg, [8; 32], 10));
        req.frame_input.payload = vec![0];
        if !missing_lock {
            lock_command(&mut req, 10);
            req.handoffs[0].authorization[199] ^= 1;
        }
        let prepared = chain.prepare_block(req).unwrap();
        let proof = seal(&v, &signer, &prepared);
        chain.commit_finalised(prepared, proof).unwrap();
        store.load_block(cfg.session_id, 0).unwrap().validate_resources(&cfg).unwrap();
        let err = chain.replay_range(0, 0, Ledger::restore).unwrap_err().to_string();
        assert!(err.contains(if missing_lock { "missing source lock" } else { "invalid owner signature" }), "{err}");
    }
}

#[derive(Clone)]
struct NoPolicy(TestExecutor);
impl TransitionExecutor for NoPolicy {
    fn state_root(&self) -> [u8; 32] {
        self.0.state_root()
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        self.0.snapshot()
    }
    fn execute_block(&mut self, h: u64, f: &FrameInput, c: &[SessionHandoff], t: &[Vec<u8>]) -> Result<ExecutionOutcome, String> {
        self.0.execute_block(h, f, c, t)
    }
}

#[test]
fn missing_executor_policy_and_disabled_genesis_reject_source_effects() {
    for disabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (v, _) = consensus();
        let mut cfg = config(&v);
        if disabled {
            cfg.protocol_effects = ProtocolEffectsPolicy::Disabled;
        }
        let chain = SessionChain::create(cfg.clone(), v, NoPolicy(TestExecutor { root: [1; 32] }), store).unwrap();
        let mut req = request();
        req.handoffs.push(intent(&cfg, [8; 32], 10));
        let err = chain.prepare_block(req).err().unwrap().to_string();
        assert!(err.contains(if disabled { "disabled by genesis" } else { "no source issuance" }), "{err}");
    }
}

#[test]
fn metadata_source_cannot_be_promoted_by_application_intake() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, signer) = consensus();
    let cfg = config(&v);
    let mut source = SessionChain::create(cfg.clone(), v.clone(), TestExecutor { root: [1; 32] }, store.clone()).unwrap();
    let mut req = request();
    req.handoffs.push(intent(&cfg, [8; 32], 10));
    let p = source.prepare_block(req).unwrap();
    let id = p.issued_handoffs()[0].id();
    let proof = seal(&v, &signer, &p);
    source.commit_finalised(p, proof).unwrap();
    let mut target_cfg = cfg.clone();
    target_cfg.session_id = [4; 32];
    target_cfg.protocol_effects = ProtocolEffectsPolicy::Application { policy_hash: POLICY };
    let target = SessionChain::create(target_cfg, v, NoPolicy(TestExecutor { root: [1; 32] }), store.clone()).unwrap();
    let mut req = request();
    req.consume_handoffs.push(id);
    assert!(target.prepare_block(req).err().unwrap().to_string().contains("authority metadata"));
    assert!(!store.load_handoff(id).unwrap().unwrap().is_consumed());
    let stored = store.load_handoff(id).unwrap().unwrap();
    let source_record = store.load_block(cfg.session_id, 0).unwrap();
    myelin_session::validate_handoff_source(&cfg, &source_record, &stored.handoff).unwrap();
    let mut changed_source = cfg.clone();
    changed_source.protocol_effects = ProtocolEffectsPolicy::Application { policy_hash: POLICY };
    assert!(myelin_session::validate_handoff_source(&changed_source, &source_record, &stored.handoff).is_err());
}

#[test]
fn successor_authorization_binds_full_target_configuration_and_replays() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, signer) = consensus();
    let ledger = Ledger::source();
    let cfg = ledger_config(&v, &ledger);
    let mut source = SessionChain::create(cfg.clone(), v.clone(), ledger, store.clone()).unwrap();
    let mut target = cfg.clone();
    target.session_id = [4; 32];
    target.initial_input_position = 1;
    target.initial_logical_time = 1;
    target.initial_timestamp_ms = 10;
    target.predecessor = Some(myelin_session::PredecessorReference {
        session_id: cfg.session_id,
        final_height: 0,
        final_state_root: cfg.initial_state_root,
        application_profile_commitment: cfg.application_profile.commitment(),
    });
    let mut req = request();
    req.frame_input.payload = signed(cfg.commitment(), &scope(), scope().successor_digest(cfg.commitment(), &target).unwrap());
    req.successor = Some(target);
    for mutation in 0..7 {
        let mut changed = req.clone();
        let target = changed.successor.as_mut().unwrap();
        match mutation {
            0 => target.consensus_config_commitment[0] ^= 1,
            1 => target.consensus_module_commitment[0] ^= 1,
            2 => target.application_profile.program_digest[0] ^= 1,
            3 => target.application_profile.logical_time_policy_hash[0] ^= 1,
            4 => target.commit_limits.max_outbox_bytes -= 1,
            5 => target.protocol_effects = ProtocolEffectsPolicy::Disabled,
            6 => target.max_future_drift_ms += 1,
            _ => unreachable!(),
        }
        assert!(source.prepare_block(changed).err().unwrap().to_string().contains("invalid owner signature"));
    }
    let p = source.prepare_block(req).unwrap();
    let proof = seal(&v, &signer, &p);
    source.commit_finalised(p, proof).unwrap();
    source.replay_range(0, 0, Ledger::restore).unwrap();
    assert_eq!(source.head().sealed_by_successor, Some([4; 32]));
}

#[test]
fn effect_policy_version_and_context_cannot_be_reinterpreted_on_import() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, signer) = consensus();
    let cfg = config(&v);
    let mut chain = SessionChain::create(cfg.clone(), v.clone(), TestExecutor { root: [1; 32] }, store.clone()).unwrap();
    let p = chain.prepare_block(request()).unwrap();
    let proof = seal(&v, &signer, &p);
    chain.commit_finalised(p, proof).unwrap();
    let record = store.load_block(cfg.session_id, 0).unwrap();
    let bytes = record.encode().unwrap();
    assert_eq!(&bytes[4..6], &myelin_session::RECORD_FORMAT_VERSION.to_le_bytes());
    assert_eq!(FinalisedBlockRecord::decode_for_config(&bytes, &cfg).unwrap(), record);
    let mut changed = cfg.clone();
    changed.protocol_effects = ProtocolEffectsPolicy::Application { policy_hash: POLICY };
    assert_ne!(cfg.commitment(), changed.commitment());
    assert!(FinalisedBlockRecord::decode_for_config(&bytes, &changed).is_err());
    let mut changed = record.clone();
    changed.frame_input.payload.push(0);
    assert!(changed.validate_resources(&cfg).is_err());
    let mut changed = record;
    changed.block.version = 1;
    assert!(changed.validate_resources(&cfg).is_err());
    let mut downgraded = bytes;
    downgraded[4..6].copy_from_slice(&8u16.to_le_bytes());
    assert!(FinalisedBlockRecord::decode(&downgraded).is_err());
}
