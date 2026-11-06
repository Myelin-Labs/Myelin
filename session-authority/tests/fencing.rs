#[path = "../../session-journal/tests/support/mod.rs"]
mod inventory;
use inventory::*;
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session::SessionStore;
use myelin_session_authority::*;
use std::path::Path;

fn signer(n: u8) -> AuthoritySigner {
    AuthoritySigner::new([n; 32]).unwrap()
}
fn open(root: &Path) -> SessionAuthority<Inventory> {
    let (v, _) = consensus();
    let cfg = config(&v, 1, 100);
    let finality = CommitteeSigner::new("journal-validator", [7; 32]).unwrap();
    let consensus = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: finality.validator_id().into(), public_key: finality.ckb_public_key() }],
    });
    let scope = myelin_session_journal::CommandScope::from_config(&cfg);
    SessionAuthority::open(
        myelin_session_authority::CanonicalLedger::open(root.join("canonical")).unwrap(),
        root.join("epochs"),
        AuthorityConfig { session: cfg, consensus, administrator: signer(8).public_key(), effect_domain: [99; 32] },
        AuthorityKeys { finality, certificates: signer(9) },
        Inventory { scope: scope.clone(), balance: 100 },
        |bytes| Ok(Inventory { scope, balance: u64::from_le_bytes(bytes.try_into().unwrap()) }),
    )
    .unwrap()
}
fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}
fn rotate(authority: &SessionAuthority<Inventory>, owner: u8) -> WriterGrant {
    let epoch = authority.current_grant().unwrap().map_or(0, |g| g.epoch);
    authority
        .grant(signer(8).authorize_grant(authority.policy(), epoch, signer(owner).public_key(), &authority.head().unwrap()).unwrap())
        .unwrap()
}
fn add_one(state: &[u8], _: &myelin_session::OutboxMessage) -> std::result::Result<Vec<u8>, String> {
    Ok((u64::from_le_bytes(state.try_into().unwrap()) + 1).to_le_bytes().to_vec())
}

#[test]
fn independent_active_replicas_cannot_fork_canonical_progress_or_external_rights() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let a = open(root);
    let old_grant = rotate(&a, 10);
    drop(a);
    copy_tree(&root.join("canonical"), &root.join("old"));
    copy_tree(&root.join("canonical"), &root.join("new"));
    let (mut old, old_store) = open_chain(&root.join("old"));
    let (new, _) = open_chain(&root.join("new"));
    let a = open(root);
    let old_command = command(old.config(), 30, 25);
    let old_request = request(&old, vec![old_command.clone()]);
    let old_auth = signer(10).authorize_commit(a.policy(), &old_grant, old.head(), &old_request).unwrap();
    let grant = rotate(&a, 11);
    assert_eq!(grant.epoch, 2);
    assert!(matches!(a.commit(old_auth, old_request, 1), Err(AuthorityError::Fenced(_))));
    // An adversarial replica can even use a deliberately provided TEST PoA key.
    // Its locally valid proof still cannot authenticate canonical effects.
    commit(&mut old, vec![old_command]);
    let local_record = old_store.load_block(old.config().session_id, 0).unwrap();
    assert!(a.canonical_receipt(0).is_err());
    assert!(a.effect_certificate(0, local_record.outbox[0].id).is_err());
    let request = request(&new, vec![command(new.config(), 31, 10)]);
    let auth = signer(11).authorize_commit(a.policy(), &grant, new.head(), &request).unwrap();
    let accepted = a.commit(auth, request, 1).unwrap();
    accepted.verify(a.policy()).unwrap();
    assert_ne!(accepted.block_hash, local_record.block.hash());
    let record = a.block(0).unwrap();
    let (certificate, message) = a.effect_certificate(0, record.outbox[0].id).unwrap();
    let mut foreign_domain = a.policy().clone();
    foreign_domain.effect_domain = [98; 32];
    assert!(certificate.verify(&foreign_domain, &message).is_err());
    let sink = CanonicalEffectStore::open(root.join("sink"), a.policy().clone(), 0u64.to_le_bytes().to_vec()).unwrap();
    assert!(sink.apply(&certificate, &local_record.outbox[0], add_one).is_err());
    let mut forged = certificate.clone();
    forged.receipt.block_hash = local_record.block.hash();
    assert!(sink.apply(&forged, &message, add_one).is_err());
    assert_eq!(sink.apply(&certificate, &message, add_one).unwrap(), EffectDisposition::Applied);
    drop(sink);
    let sink = CanonicalEffectStore::open(root.join("sink"), a.policy().clone(), vec![]).unwrap();
    assert_eq!(
        sink.apply(&certificate, &message, |_, _| panic!("duplicate must not mint twice")).unwrap(),
        EffectDisposition::Duplicate
    );
    assert_eq!(sink.state().unwrap(), 1u64.to_le_bytes());
    // Repeated owner changes never invalidate genuine historical effects.
    rotate(&a, 12);
    certificate.verify(a.policy(), &message).unwrap();
    let policy = a.policy().clone();
    drop(a);
    let a = open(root);
    assert_eq!(a.current_grant().unwrap().unwrap().epoch, 3);
    a.canonical_receipt(0).unwrap().verify(&policy).unwrap();
    assert_eq!(a.head().unwrap().state_root, *blake3::hash(&90u64.to_le_bytes()).as_bytes());
}

#[test]
fn grants_bind_admin_epoch_exact_anchor_and_credential_roles() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(dir.path());
    let head = a.head().unwrap();
    let unauthorized = signer(10).authorize_grant(a.policy(), 0, signer(10).public_key(), &head).unwrap();
    assert!(a.grant(unauthorized).is_err());
    let mut wrong = head.clone();
    wrong.state_root = [5; 32];
    assert!(a.grant(signer(8).authorize_grant(a.policy(), 0, signer(10).public_key(), &wrong).unwrap()).is_err());
    assert!(a.grant(signer(8).authorize_grant(a.policy(), 0, signer(9).public_key(), &head).unwrap()).is_err());
    let request = signer(8).authorize_grant(a.policy(), 0, signer(10).public_key(), &head).unwrap();
    let grant = a.grant(request.clone()).unwrap();
    assert_eq!(a.grant(request.clone()).unwrap(), grant);
    rotate(&a, 11);
    assert!(a.grant(request).is_err());
    drop(a);
    let a = open(dir.path());
    assert_eq!(a.current_grant().unwrap().unwrap().epoch, 2);
}

#[test]
fn candidate_mutation_and_stale_backup_are_rejected_before_execution() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(dir.path());
    let grant = rotate(&a, 10);
    drop(a);
    copy_tree(&dir.path().join("canonical"), &dir.path().join("worker"));
    let (worker, _) = open_chain(&dir.path().join("worker"));
    let a = open(dir.path());
    let cmd = command(worker.config(), 30, 25);
    let mut req = request(&worker, vec![cmd.clone()]);
    let auth = signer(10).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
    req.timestamp_ms = 2;
    assert!(matches!(a.commit(auth, req, 2), Err(AuthorityError::Authorization(_))));
    assert!(a.head().unwrap().finalised_height.is_none());
    let mut req = request(&worker, vec![cmd]);
    req.local_now_ms = u64::MAX;
    let auth = signer(10).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
    a.commit(auth, req, 1).unwrap();
    let req = request(&worker, vec![command(worker.config(), 31, 10)]);
    let auth = signer(10).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
    assert!(matches!(a.commit(auth, req, 1), Err(AuthorityError::Fenced(_))));
    assert_eq!(a.head().unwrap().finalised_height, Some(0));
}

#[test]
fn stale_epoch_database_or_missing_history_blocks_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(dir.path());
    rotate(&a, 10);
    drop(a);
    copy_tree(&dir.path().join("epochs"), &dir.path().join("old-epochs"));
    copy_tree(&dir.path().join("canonical"), &dir.path().join("worker"));
    let (worker, _) = open_chain(&dir.path().join("worker"));
    let a = open(dir.path());
    let grant = rotate(&a, 11);
    let req = request(&worker, vec![command(worker.config(), 30, 25)]);
    let auth = signer(11).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
    a.commit(auth, req, 1).unwrap();
    drop(a);
    std::fs::remove_dir_all(dir.path().join("epochs")).unwrap();
    copy_tree(&dir.path().join("old-epochs"), &dir.path().join("epochs"));
    assert!(std::panic::catch_unwind(|| open(dir.path())).is_err());
}

#[test]
fn canonical_rollback_in_the_same_epoch_is_rejected_by_watermark() {
    let dir = tempfile::tempdir().unwrap();
    let a = open(dir.path());
    let grant = rotate(&a, 10);
    drop(a);
    copy_tree(&dir.path().join("canonical"), &dir.path().join("old-canonical"));
    copy_tree(&dir.path().join("canonical"), &dir.path().join("worker"));
    let (worker, _) = open_chain(&dir.path().join("worker"));
    let a = open(dir.path());
    let req = request(&worker, vec![command(worker.config(), 30, 25)]);
    let auth = signer(10).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
    a.commit(auth, req, 1).unwrap();
    drop(a);
    std::fs::remove_dir_all(dir.path().join("canonical")).unwrap();
    copy_tree(&dir.path().join("old-canonical"), &dir.path().join("canonical"));
    assert!(std::panic::catch_unwind(|| open(dir.path())).is_err());
}

#[test]
fn competing_grant_and_commit_have_one_authoritative_order() {
    for _ in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let a = open(dir.path());
        let grant = rotate(&a, 10);
        let (worker, _) = open_chain(&dir.path().join("worker"));
        let cmd = command(worker.config(), 30, 25);
        let req = request(&worker, vec![cmd.clone()]);
        let auth = signer(10).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
        let takeover = signer(8).authorize_grant(a.policy(), 1, signer(11).public_key(), &a.head().unwrap()).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let (write, rotation) = std::thread::scope(|scope| {
            let write = scope.spawn(|| {
                barrier.wait();
                a.commit(auth, req, 1)
            });
            let rotation = scope.spawn(|| {
                barrier.wait();
                a.grant(takeover)
            });
            (write.join().unwrap(), rotation.join().unwrap())
        });
        match (write, rotation) {
            (Ok(receipt), Err(AuthorityError::Fenced(_))) => {
                receipt.verify(a.policy()).unwrap();
                rotate(&a, 11);
            }
            (Err(AuthorityError::Fenced(_)), Ok(grant)) => {
                let req = request(&worker, vec![cmd]);
                let auth = signer(11).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
                a.commit(auth, req, 1).unwrap();
            }
            other => panic!("unexpected ordering: {other:?}"),
        }
        assert_eq!(a.head().unwrap().finalised_height, Some(0));
        assert_eq!(a.current_grant().unwrap().unwrap().epoch, 2);
        assert_eq!(a.head().unwrap().state_root, *blake3::hash(&75u64.to_le_bytes()).as_bytes());
    }
}

#[cfg(feature = "test-fixture")]
#[test]
fn authority_crash_child() {
    let Ok(root) = std::env::var("MYELIN_AUTHORITY_CRASH_ROOT") else { return };
    let root = Path::new(&root);
    let a = open(root);
    let grant = rotate(&a, 11);
    let (worker, _) = open_chain(&root.join("worker"));
    let req = request(&worker, vec![command(worker.config(), 30, 25)]);
    let auth = signer(11).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
    a.commit(auth, req, 1).unwrap();
    let record = a.block(0).unwrap();
    let (cert, message) = a.effect_certificate(0, record.outbox[0].id).unwrap();
    let sink = CanonicalEffectStore::open(root.join("sink"), a.policy().clone(), 0u64.to_le_bytes().to_vec()).unwrap();
    sink.apply(&cert, &message, add_one).unwrap();
    if std::env::var("MYELIN_AUTHORITY_CRASH_POINT").as_deref() == Ok("after-effect") {
        std::process::exit(74);
    }
    panic!("unknown crash point");
}

#[cfg(feature = "test-fixture")]
#[test]
fn takeover_signing_commit_and_effect_crashes_resume_one_canonical_outcome() {
    for point in [
        "before-grant",
        "after-grant",
        "before-intent",
        "after-intent",
        "after-sign",
        "after-canonical-commit",
        "after-watermark",
        "after-receipt",
        "after-effect",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let a = open(dir.path());
        let original_grant = rotate(&a, 10);
        drop(a);
        copy_tree(&dir.path().join("canonical"), &dir.path().join("worker"));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "authority_crash_child", "--nocapture"])
            .env("MYELIN_AUTHORITY_CRASH_ROOT", dir.path())
            .env("MYELIN_AUTHORITY_CRASH_POINT", point)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(74), "{point}: {}", String::from_utf8_lossy(&output.stderr));
        for _ in 0..2 {
            let a = open(dir.path());
            let (worker, _) = open_chain(&dir.path().join("worker"));
            let req = request(&worker, vec![command(worker.config(), 30, 25)]);
            let old_auth = signer(10).authorize_commit(a.policy(), &original_grant, worker.head(), &req).unwrap();
            if point != "before-grant" {
                assert!(matches!(a.commit(old_auth, req, 1), Err(AuthorityError::Fenced(_))));
            }
            let grant = rotate(&a, 12);
            if a.head().unwrap().finalised_height.is_none() {
                let req = request(&worker, vec![command(worker.config(), 30, 25)]);
                let auth = signer(12).authorize_commit(a.policy(), &grant, worker.head(), &req).unwrap();
                a.commit(auth, req, 1).unwrap();
            }
            assert_eq!(a.head().unwrap().finalised_height, Some(0));
            assert_eq!(a.head().unwrap().state_root, *blake3::hash(&75u64.to_le_bytes()).as_bytes());
            let scope = myelin_session_journal::CommandScope::from_config(worker.config());
            a.replay_range(0, 0, move |bytes| {
                Ok(Inventory { scope: scope.clone(), balance: u64::from_le_bytes(bytes.try_into().unwrap()) })
            })
            .unwrap();
            let record = a.block(0).unwrap();
            let (cert, message) = a.effect_certificate(0, record.outbox[0].id).unwrap();
            let sink = CanonicalEffectStore::open(dir.path().join("sink"), a.policy().clone(), 0u64.to_le_bytes().to_vec()).unwrap();
            sink.apply(&cert, &message, add_one).unwrap();
            assert_eq!(sink.apply(&cert, &message, |_, _| panic!("duplicate effect")).unwrap(), EffectDisposition::Duplicate);
            assert_eq!(sink.state().unwrap(), 1u64.to_le_bytes());
        }
    }
}
