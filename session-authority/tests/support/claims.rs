use super::*;
#[allow(dead_code)]
#[path = "../../../session-transfer/tests/support/mod.rs"]
mod transfers;
use myelin_session::{HandoffAction, HandoffIntent, HandoffTerminal, Hash32, StoredHandoff};
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;

fn settings() -> [(SessionConfig, TransferState); 2] {
    let (verifier, _) = transfers::consensus();
    let initial = transfers::state(transfers::policy(TransferKind::Scarce));
    let source = transfers::config(&verifier, 3, &initial);
    let mut target_policy = transfers::policy(TransferKind::Scarce);
    target_policy.accepted_sources.push(SourceIdentity::from_config(&source).unwrap());
    let target = TransferState::new(target_policy, vec![]).unwrap();
    [(source, initial), (transfers::config(&verifier, 4, &target), target)]
}
fn open_transfer(
    root: &Path,
    ledger: Arc<CanonicalLedger>,
    cfg: &SessionConfig,
    initial: &TransferState,
) -> myelin_session_authority::Result<SessionAuthority<TransferExecutor>> {
    let sources = SourceBindings::from_configs(initial.policy(), |id| {
        ledger.genesis(id).map(|genesis| genesis.config).map_err(|e| Error::Source(e.to_string()))
    })
    .unwrap();
    let executor = TransferExecutor::new(cfg, initial.clone(), sources.clone()).unwrap();
    let (_, finality) = transfers::consensus();
    let consensus = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: finality.validator_id().into(), public_key: finality.ckb_public_key() }],
    });
    SessionAuthority::open(
        ledger,
        root.join(format!("epochs-{}", cfg.session_id[0])),
        AuthorityConfig { session: cfg.clone(), consensus, administrator: signer(8).public_key(), effect_domain: [99; 32] },
        AuthorityKeys { finality, certificates: signer(9) },
        executor,
        |bytes| TransferExecutor::restore(cfg, bytes, sources).map_err(|e| e.to_string()),
    )
}
async fn start_pair(
    root: &Path,
    ledger: &Arc<CanonicalLedger>,
    configs: &[(SessionConfig, TransferState); 2],
) -> [Server<TransferExecutor>; 2] {
    let source = Arc::new(open_transfer(root, ledger.clone(), &configs[0].0, &configs[0].1).unwrap());
    let target = Arc::new(open_transfer(root, ledger.clone(), &configs[1].0, &configs[1].1).unwrap());
    [start_authority(root, source, NetworkLimits::default()).await, start_authority(root, target, NetworkLimits::default()).await]
}
async fn close<E: TransitionExecutor>(mut server: Server<E>) {
    server.stop.take().unwrap().send(()).unwrap();
    (&mut server.job).await.unwrap().unwrap();
}
fn candidate(
    cfg: &SessionConfig,
    head: &SessionHead,
    commands: Vec<TransferCommand>,
    handoffs: Vec<HandoffIntent>,
    consume_handoffs: Vec<Hash32>,
    handoff_actions: Vec<HandoffAction>,
) -> BlockRequest {
    BlockRequest {
        transactions: vec![],
        frame_input: command_frame(cfg, head.input_position, head.logical_time, commands).unwrap(),
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: head.timestamp_ms + 1,
        local_now_ms: 0,
        successor: None,
        handoffs,
        consume_handoffs,
        handoff_actions,
    }
}
async fn submit(
    client: &mut AuthorityClient,
    grant: &WriterGrant,
    worker: u8,
    cfg: &SessionConfig,
    commands: Vec<TransferCommand>,
    handoffs: Vec<HandoffIntent>,
    consume: Vec<Hash32>,
    actions: Vec<HandoffAction>,
) -> CanonicalReceipt {
    let head = client.observe().await.unwrap().head().unwrap();
    let request = candidate(cfg, &head, commands, handoffs, consume, actions);
    let authorization = signer(worker).authorize_commit(client.policy(), grant, &head, &request).unwrap();
    client.commit(authorization, request).await.unwrap()
}
async fn claim(client: &mut AuthorityClient, id: Hash32, epoch: u64) -> StoredHandoff {
    client.reconcile_claims(vec![id], epoch).await.unwrap().verify(client.policy(), &[id], epoch).unwrap().pop().unwrap().unwrap()
}
fn state(authority: &SessionAuthority<TransferExecutor>) -> TransferState {
    let port = |_: &myelin_session::InspectContext<'_>, bytes: &[u8], _: &[u8]| Ok(bytes.to_vec());
    let receipt = authority.inspect_head(&port, b"state").unwrap();
    let state = TransferState::decode(&receipt.result).unwrap();
    assert_eq!(state.root().unwrap(), receipt.state_root);
    state
}
fn mutations(certificate: &ClaimReconciliation, policy: &FencingPolicy, ids: &[Hash32], epoch: u64) {
    assert!(certificate.verify(policy, ids, epoch + 1).is_err());
    assert!(certificate.verify(policy, &ids.iter().rev().copied().collect::<Vec<_>>(), epoch).is_err());
    let mut other = policy.clone();
    other.certificate_key = signer(12).public_key();
    assert!(certificate.verify(&other, ids, epoch).is_err());
    let mut changed = certificate.clone();
    changed.signature[0] ^= 1;
    assert!(changed.verify(policy, ids, epoch).is_err());
    // Even a correctly signed malformed observation must fail structural checks.
    for mutation in 0..6 {
        let mut changed = certificate.clone();
        match mutation {
            0 => {
                changed.blocks.pop();
            }
            1 => {
                changed.blocks.push(changed.blocks[0].clone());
            }
            2 => {
                changed.sessions[0].head[10] ^= 1;
            }
            3 => {
                changed.observations[0].id[0] ^= 1;
            }
            4 => {
                changed.policy[0] ^= 1;
            }
            _ => {
                let ObservedClaim::Present { record } = &mut changed.observations[0].status else { panic!() };
                let mut stored = StoredHandoff::decode(record).unwrap();
                stored.lifecycle.terminal = HandoffTerminal::Cancelled { at: stored.lifecycle.issued_at.clone() };
                // Canonical envelope encoding rejects this causal inversion itself.
                assert!(stored.encode().is_err());
                continue;
            }
        }
        let secp = secp256k1::Secp256k1::new();
        let key = secp256k1::Keypair::from_secret_key(&secp, &secp256k1::SecretKey::from_slice(&[9; 32]).unwrap());
        changed.signature =
            secp.sign_schnorr_no_aux_rand(&secp256k1::Message::from_digest(changed.digest().unwrap()), &key).as_ref().to_vec();
        assert!(changed.verify(policy, ids, epoch).is_err(), "mutation {mutation}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_claim_terminals_survive_lost_replies_fencing_and_shared_ledger_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_tls(root);
    let configs = settings();
    let source_cfg = &configs[0].0;
    let target_cfg = &configs[1].0;
    let ledger = CanonicalLedger::open(root.join("canonical")).unwrap();
    let [source, target] = start_pair(root, &ledger, &configs).await;
    let duplicate = root.join("duplicate-owner");
    std::fs::create_dir(&duplicate).unwrap();
    let error = open_transfer(&duplicate, ledger.clone(), source_cfg, &configs[0].1).err().unwrap();
    assert!(error.to_string().contains("already has a live authority"));
    let mut source_client = connect(&source.endpoint, root, source.authority.policy()).await;
    let mut target_client = connect(&target.endpoint, root, target.authority.policy()).await;
    let source_grant = rotate(&mut source_client, 10).await;
    let target_grant = rotate(&mut target_client, 10).await;
    let mut ids = vec![];
    for n in 0..3 {
        let intent = transfers::signer()
            .sign(
                transfers::payload(
                    source_cfg,
                    configs[0].1.policy(),
                    TransferTarget::Session { session_id: target_cfg.session_id, genesis_commitment: target_cfg.commitment() },
                    n as u8 + 1,
                    n,
                    20,
                ),
                100,
                None,
            )
            .unwrap();
        let receipt = submit(
            &mut source_client,
            &source_grant,
            10,
            source_cfg,
            vec![TransferCommand::Issue { transfer_id: verify_intent(&intent).unwrap().0 }],
            vec![intent],
            vec![],
            vec![],
        )
        .await;
        let (block, _) = source_client.block(receipt.height).await.unwrap();
        ids.push(block.issued_handoffs[0].id());
    }
    let issued = source_client.reconcile_claims(ids.clone(), source_grant.epoch).await.unwrap();
    assert!(issued
        .verify(source_client.policy(), &ids, source_grant.epoch)
        .unwrap()
        .iter()
        .all(|s| matches!(s.as_ref().unwrap().lifecycle.terminal, HandoffTerminal::Issued)));
    // Copy only stopped stores. Both stale worker copies remain independently
    // open while the canonical services progress under their private signers.
    close(source).await;
    close(target).await;
    drop(ledger);
    copy_tree(&root.join("canonical"), &root.join("worker-old"));
    copy_tree(&root.join("canonical"), &root.join("worker-new"));
    let old = RocksSessionStore::open(root.join("worker-old")).unwrap();
    let new = RocksSessionStore::open(root.join("worker-new")).unwrap();
    let ledger = CanonicalLedger::open(root.join("canonical")).unwrap();
    let [source, target] = start_pair(root, &ledger, &configs).await;
    source_client = connect(&source.endpoint, root, source.authority.policy()).await;
    target_client = connect(&target.endpoint, root, target.authority.policy()).await;
    let current = rotate(&mut source_client, 11).await;
    assert!(issued.verify(source_client.policy(), &ids, current.epoch).is_err());
    let missing = [77; 32];
    assert_eq!(
        source_client
            .reconcile_claims(vec![missing], current.epoch)
            .await
            .unwrap()
            .verify(source_client.policy(), &[missing], current.epoch)
            .unwrap(),
        vec![None]
    );
    assert!(source_client.reconcile_claims(vec![ids[0]; 2], current.epoch).await.is_err());
    assert!(source_client.reconcile_claims(vec![[1; 32]; MAX_RECONCILIATION_CLAIMS + 1], current.epoch).await.is_err());
    assert!(source_client.reconcile_claims(ids.clone(), current.epoch + 1).await.is_err());
    // A real TCP proxy drops the target's committed response. The source's
    // signed terminal observation resolves the claim across session boundaries.
    let lost = proxy(&target.endpoint).await;
    let mut lost_client = connect(&lost.endpoint, root, target.authority.policy()).await;
    let head = lost_client.observe().await.unwrap().head().unwrap();
    let request = candidate(target_cfg, &head, vec![TransferCommand::Intake { handoff_id: ids[0] }], vec![], vec![ids[0]], vec![]);
    let authorization = signer(10).authorize_commit(lost_client.policy(), &target_grant, &head, &request).unwrap();
    let original_commands = request.frame_input.commands.clone();
    lost.drop_responses.store(true, Ordering::SeqCst);
    assert!(matches!(lost_client.commit(authorization.clone(), request).await, Err(ClientError::Transport(_))));
    lost.disconnect().await;
    let consumed = claim(&mut source_client, ids[0], current.epoch).await;
    let HandoffTerminal::Consumed { at } = consumed.lifecycle.terminal.clone() else { panic!() };
    let command_result = target_client.reconcile(authorization.clone(), original_commands.clone()).await.unwrap();
    let verified = command_result.verify(target_client.policy(), &authorization, &original_commands).unwrap();
    let VerifiedObservation::Committed { receipt, .. } = &verified[0] else { panic!() };
    assert_eq!((receipt.session_id, receipt.block_height, receipt.block_hash), (at.session_id, at.height, at.block_hash));
    for worker in [&old, &new] {
        assert!(worker.load_handoff(ids[0]).unwrap().unwrap().is_available());
    }
    let head = source_client.observe().await.unwrap().head().unwrap();
    let cancel = transfers::signer().cancel(source_cfg, &consumed.handoff).unwrap();
    let request = candidate(source_cfg, &head, vec![cancel], vec![], vec![], vec![HandoffAction::Cancel { handoff_id: ids[0] }]);
    let old_authorization = signer(10).authorize_commit(source_client.policy(), &source_grant, &head, &request).unwrap();
    assert!(matches!(
        source_client.commit(old_authorization, request.clone()).await,
        Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Fenced, .. }))
    ));
    let current_authorization = signer(11).authorize_commit(source_client.policy(), &current, &head, &request).unwrap();
    assert!(source_client.commit(current_authorization, request).await.is_err(), "even the new writer cannot cancel a consumed claim");
    submit(
        &mut source_client,
        &current,
        11,
        source_cfg,
        vec![TransferCommand::Retire { handoff_id: ids[0] }],
        vec![],
        vec![],
        vec![HandoffAction::Retire { handoff_id: ids[0], consumed_at: at }],
    )
    .await;
    let pending = claim(&mut source_client, ids[1], current.epoch).await;
    let cancellation = transfers::signer().cancel(source_cfg, &pending.handoff).unwrap();
    submit(
        &mut source_client,
        &current,
        11,
        source_cfg,
        vec![cancellation],
        vec![],
        vec![],
        vec![HandoffAction::Cancel { handoff_id: ids[1] }],
    )
    .await;
    assert!(matches!(claim(&mut source_client, ids[1], current.epoch).await.lifecycle.terminal, HandoffTerminal::Cancelled { .. }));
    submit(
        &mut source_client,
        &current,
        11,
        source_cfg,
        vec![TransferCommand::Reclaim { handoff_id: ids[1] }],
        vec![],
        vec![],
        vec![HandoffAction::Reclaim { handoff_id: ids[1] }],
    )
    .await;
    let certificate = source_client.reconcile_claims(ids.clone(), current.epoch).await.unwrap();
    let outcomes = certificate.verify(source_client.policy(), &ids, current.epoch).unwrap();
    assert!(matches!(outcomes[0].as_ref().unwrap().lifecycle.terminal, HandoffTerminal::Retired { .. }));
    assert!(matches!(outcomes[1].as_ref().unwrap().lifecycle.terminal, HandoffTerminal::Reclaimed { .. }));
    assert!(matches!(outcomes[2].as_ref().unwrap().lifecycle.terminal, HandoffTerminal::Issued));
    mutations(&certificate, source_client.policy(), &ids, current.epoch);
    assert!(
        target_client.reconcile_claims(ids.clone(), target_grant.epoch).await.is_err(),
        "a consuming session cannot impersonate the issuer"
    );
    let source_head = source_client.observe().await.unwrap().head().unwrap();
    let target_head = target_client.observe().await.unwrap().head().unwrap();
    close(source).await;
    close(target).await;
    drop(ledger);
    let ledger = CanonicalLedger::open(root.join("canonical")).unwrap();
    let [source, target] = start_pair(root, &ledger, &configs).await;
    source_client = connect(&source.endpoint, root, source.authority.policy()).await;
    assert_eq!(source.authority.head().unwrap(), source_head);
    assert_eq!(target.authority.head().unwrap(), target_head);
    assert_eq!(source_client.reconcile_claims(ids.clone(), current.epoch).await.unwrap(), certificate);
    let source_state = state(&source.authority);
    let target_state = state(&target.authority);
    let source_account = source_state.account(transfers::ASSET, transfers::signer().actor()).unwrap();
    assert_eq!((source_account.available, source_account.locked), (60, 20));
    assert_eq!(target_state.account(transfers::ASSET, transfers::recipient()).unwrap().available, 20);
    assert_eq!(
        source_account.available
            + source_account.locked
            + target_state.account(transfers::ASSET, transfers::recipient()).unwrap().available,
        100
    );
    // Competing consume/cancel operations use separate session writer locks and
    // the same canonical claim transaction. Exactly one may become canonical.
    let pending = claim(&mut source_client, ids[2], current.epoch).await;
    let cancel = candidate(
        source_cfg,
        &source_head,
        vec![transfers::signer().cancel(source_cfg, &pending.handoff).unwrap()],
        vec![],
        vec![],
        vec![HandoffAction::Cancel { handoff_id: ids[2] }],
    );
    let intake =
        candidate(target_cfg, &target_head, vec![TransferCommand::Intake { handoff_id: ids[2] }], vec![], vec![ids[2]], vec![]);
    let cancel_auth = signer(11).authorize_commit(source_client.policy(), &current, &source_head, &cancel).unwrap();
    let mut target_client = connect(&target.endpoint, root, target.authority.policy()).await;
    let intake_auth = signer(10).authorize_commit(target_client.policy(), &target_grant, &target_head, &intake).unwrap();
    let (cancelled, consumed) = tokio::join!(source_client.commit(cancel_auth, cancel), target_client.commit(intake_auth, intake));
    assert_ne!(cancelled.is_ok(), consumed.is_ok());
    close(source).await;
    close(target).await;
    drop(ledger);
    let ledger = CanonicalLedger::open(root.join("canonical")).unwrap();
    let [source, target] = start_pair(root, &ledger, &configs).await;
    source_client = connect(&source.endpoint, root, source.authority.policy()).await;
    let terminal = claim(&mut source_client, ids[2], current.epoch).await;
    let (command, action) = match terminal.lifecycle.terminal {
        HandoffTerminal::Consumed { at } => {
            assert!(consumed.is_ok());
            (TransferCommand::Retire { handoff_id: ids[2] }, HandoffAction::Retire { handoff_id: ids[2], consumed_at: at })
        }
        HandoffTerminal::Cancelled { .. } => {
            assert!(cancelled.is_ok());
            (TransferCommand::Reclaim { handoff_id: ids[2] }, HandoffAction::Reclaim { handoff_id: ids[2] })
        }
        other => panic!("claim race has no terminal result: {other:?}"),
    };
    submit(&mut source_client, &current, 11, source_cfg, vec![command], vec![], vec![], vec![action]).await;
    let source_state = state(&source.authority);
    let target_state = state(&target.authority);
    let source_account = source_state.account(transfers::ASSET, transfers::signer().actor()).unwrap();
    assert_eq!(source_account.locked, 0);
    assert_eq!(source_account.available + target_state.account(transfers::ASSET, transfers::recipient()).unwrap().available, 100);
    for (server, (cfg, initial)) in [(&source, &configs[0]), (&target, &configs[1])] {
        let sources = SourceBindings::from_configs(initial.policy(), |id| {
            ledger.genesis(id).map(|genesis| genesis.config).map_err(|e| Error::Source(e.to_string()))
        })
        .unwrap();
        let cfg = cfg.clone();
        let head = server.authority.head().unwrap();
        let replay = server
            .authority
            .replay_range(0, head.finalised_height.unwrap(), move |bytes| {
                TransferExecutor::restore(&cfg, bytes, sources).map_err(|e| myelin_session::SessionError::Replay(e.to_string()))
            })
            .unwrap();
        assert_eq!(replay.state_root_after, head.state_root);
    }
    close(source).await;
    close(target).await;
}
