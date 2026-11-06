mod support;
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session::*;
use myelin_session_authority::*;
use myelin_session_journal::*;
use std::path::Path;
use support::*;

fn signer(n: u8) -> AuthoritySigner {
    AuthoritySigner::new([n; 32]).unwrap()
}
fn authority(root: &Path) -> SessionAuthority<Inventory> {
    let (v, _) = consensus();
    let cfg = config(&v, 1, 100);
    let finality = CommitteeSigner::new("journal-validator", [7; 32]).unwrap();
    let consensus = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: finality.validator_id().into(), public_key: finality.ckb_public_key() }],
    });
    let scope = CommandScope::from_config(&cfg);
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
fn cfg() -> SessionConfig {
    let (v, _) = consensus();
    config(&v, 1, 100)
}
fn journal(root: &Path, a: &SessionAuthority<Inventory>) -> CommandJournal {
    CommandJournal::open_fenced(root.join("journal"), &cfg(), JournalLimits::default(), a.policy().clone()).unwrap()
}
fn rotate(a: &SessionAuthority<Inventory>, owner: u8) -> WriterGrant {
    let epoch = a.current_grant().unwrap().map_or(0, |g| g.epoch);
    a.grant(signer(8).authorize_grant(a.policy(), epoch, signer(owner).public_key(), &a.head().unwrap()).unwrap()).unwrap()
}
fn request_at(head: &SessionHead, commands: Vec<OriginalCommand>) -> BlockRequest {
    BlockRequest {
        transactions: vec![],
        frame_input: FrameInput {
            commands: commands.iter().map(|c| c.identity.clone()).collect(),
            start_position: head.input_position,
            end_position: head.input_position + commands.len() as u64,
            logical_time_start: head.logical_time,
            logical_time_end: head.logical_time + 1,
            payload: serde_json::to_vec(&commands).unwrap(),
        },
        scheduler_commitment: [20; 32],
        data_commitments: vec![],
        timestamp_ms: head.timestamp_ms + 1,
        local_now_ms: head.timestamp_ms + 1,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    }
}
fn reserve(j: &CommandJournal, a: &SessionAuthority<Inventory>) -> Reservation {
    j.reserve(&a.head().unwrap(), 32, 100_000, &Auth, |c, _| BuildDecision::Ready(serde_json::to_vec(c).unwrap())).unwrap().unwrap()
}
fn begin(
    j: &CommandJournal,
    a: &SessionAuthority<Inventory>,
    grant: &WriterGrant,
    key: u8,
    commands: Vec<OriginalCommand>,
) -> (Reservation, FencedSubmission, BlockRequest) {
    for command in &commands {
        j.enqueue(command.clone(), &Auth).unwrap();
    }
    let reservation = reserve(j, a);
    let request = request_at(&a.head().unwrap(), commands);
    let auth = signer(key).authorize_commit(a.policy(), grant, &a.head().unwrap(), &request).unwrap();
    let binding = j.begin_fenced_commit(reservation.token, a.policy().clone(), grant.clone(), auth, &request).unwrap();
    (reservation, binding, request)
}

#[test]
fn only_later_epoch_missing_evidence_releases_exact_unknown_submission() {
    let dir = tempfile::tempdir().unwrap();
    let a = authority(dir.path());
    let g1 = rotate(&a, 10);
    let j = journal(dir.path(), &a);
    let original = command(&cfg(), 30, 25);
    let (r, binding, request) = begin(&j, &a, &g1, 10, vec![original.clone()]);
    j.mark_unknown(r.token).unwrap();
    let same = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
    j.reconcile_fenced(r.token, &same, result).unwrap();
    assert!(matches!(
        j.entry(original.identity.id).unwrap().unwrap().status,
        EntryStatus::Reserved { phase: ReservationPhase::Unknown, .. }
    ));
    assert!(j.release_unsubmitted(r.token).is_err());
    drop(j);
    let j = journal(dir.path(), &a);
    assert_eq!(j.active_reservation().unwrap().unwrap().fenced_submission.as_ref(), Some(&binding));
    let g2 = rotate(&a, 11);
    let fenced = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
    let restored = j.reconcile_fenced(r.token, &fenced, result).unwrap();
    assert_eq!(restored[0].status, EntryStatus::Queued);
    assert_eq!(restored[0].original, original);
    assert!(matches!(a.commit(binding.authorization.clone(), request, 1), Err(AuthorityError::Fenced(_))));
    let (retry, next, request) = begin(&j, &a, &g2, 11, vec![original.clone()]);
    assert!(j.reconcile_fenced(r.token, &fenced, result).is_err());
    assert!(j.reconcile_fenced(retry.token, &fenced, result).is_err());
    a.commit(next.authorization.clone(), request, 1).unwrap();
    let included = a.reconcile_submission(&next.authorization, &next.commands).unwrap();
    j.reconcile_fenced(retry.token, &included, |_, _| Ok(None)).unwrap();
    assert!(j.acknowledge(original.identity.id).is_err());
    j.reconcile_fenced(retry.token, &included, result).unwrap();
    assert_eq!(j.acknowledge(original.identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
    assert_eq!(j.fenced_recoveries(0, 10).unwrap().len(), 3);
    assert_eq!(a.head().unwrap().finalised_height, Some(0));
    drop(j);
    let j = journal(dir.path(), &a);
    assert!(matches!(j.enqueue(original, &Auth).unwrap().status, EntryStatus::Committed { acknowledged: true, .. }));
    j.reconcile_fenced(retry.token, &included, |_, _| panic!("retained result is immutable")).unwrap();
}
fn restored_status(j: &CommandJournal, id: Hash32) -> EntryStatus {
    j.entry(id).unwrap().unwrap().status
}

#[test]
fn canonical_batch_records_recover_success_rejection_and_lost_response() {
    let dir = tempfile::tempdir().unwrap();
    let a = authority(dir.path());
    let g = rotate(&a, 10);
    let j = journal(dir.path(), &a);
    let commands = vec![command(&cfg(), 30, 25), command(&cfg(), 31, 200)];
    let (r, binding, request) = begin(&j, &a, &g, 10, commands.clone());
    a.commit(binding.authorization.clone(), request, 1).unwrap();
    drop(j);
    let j = journal(dir.path(), &a);
    let proof = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
    assert_eq!(proof.blocks.len(), 1);
    assert!(FinalisedBlockRecord::decode(&proof.blocks[0]).unwrap().state_snapshot.is_empty());
    let roundtrip: CommandReconciliation = serde_json::from_slice(&serde_json::to_vec(&proof).unwrap()).unwrap();
    j.reconcile_fenced(r.token, &roundtrip, result).unwrap();
    assert_eq!(j.acknowledge(commands[0].identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
    assert_eq!(j.acknowledge(commands[1].identity.id).unwrap(), BusinessOutcome::Rejected(b"insufficient balance".to_vec()));
    let (replica, _) = open_chain(&dir.path().join("replica"));
    assert!(j.reconcile(&replica, result).is_err());
    let mut other = a.policy().clone();
    other.certificate_key = signer(12).public_key();
    drop(j);
    assert!(CommandJournal::open_fenced(dir.path().join("journal"), &cfg(), JournalLimits::default(), other).is_err());
    let j = journal(dir.path(), &a);
    assert!(matches!(j.enqueue(commands[0].clone(), &Auth).unwrap().status, EntryStatus::Committed { acknowledged: true, .. }));
    // Runtime validation may reuse unchanged in-memory evidence, but reopening
    // must verify the signed record again even with a freshly valid checksum.
    drop(j);
    let path = dir.path().join("journal");
    let db = rocksdb::DB::open_default(&path).unwrap();
    let original = db.get(b"myelin-command-journal").unwrap().unwrap();
    let mut body: serde_json::Value = serde_json::from_slice(&original[37..]).unwrap();
    let byte = body["reconciliations"][0]["certificate"]["signature"][0].as_u64().unwrap();
    body["reconciliations"][0]["certificate"]["signature"][0] = serde_json::json!(byte ^ 1);
    let body = serde_json::to_vec(&body).unwrap();
    let mut forged = b"MJNL\x02".to_vec();
    forged.extend_from_slice(blake3::hash(&body).as_bytes());
    forged.extend_from_slice(&body);
    db.put(b"myelin-command-journal", forged).unwrap();
    drop(db);
    assert!(CommandJournal::open_fenced(&path, &cfg(), JournalLimits::default(), a.policy().clone()).is_err());
    let db = rocksdb::DB::open_default(&path).unwrap();
    db.put(b"myelin-command-journal", original).unwrap();
    drop(db);
    assert_eq!(journal(dir.path(), &a).fenced_recoveries(0, 256).unwrap().len(), 1);
}

#[test]
fn foreign_modified_or_wrongly_ordered_evidence_cannot_change_journal() {
    let dir = tempfile::tempdir().unwrap();
    let a = authority(dir.path());
    let g = rotate(&a, 10);
    let j = journal(dir.path(), &a);
    let commands = vec![command(&cfg(), 30, 25), command(&cfg(), 31, 5)];
    let (r, binding, request) = begin(&j, &a, &g, 10, commands);
    a.commit(binding.authorization.clone(), request, 1).unwrap();
    let proof = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
    let before = j.entries(0, 10).unwrap();
    for change in 0..7 {
        let mut bad = proof.clone();
        match change {
            0 => bad.policy[0] ^= 1,
            1 => bad.submission_digest[0] ^= 1,
            2 => bad.observed_epoch += 1,
            3 => bad.observations.swap(0, 1),
            4 => bad.blocks[0][10] ^= 1,
            5 => bad.observations[0].status = ObservedStatus::Missing,
            _ => bad.signature[0] ^= 1,
        }
        assert!(j.reconcile_fenced(r.token, &bad, result).is_err());
        assert_eq!(j.entries(0, 10).unwrap(), before);
    }
    let mut foreign = binding.authorization.clone();
    foreign.owner = signer(12).public_key();
    assert!(a.reconcile_submission(&foreign, &binding.commands).is_err());
    assert!(a.reconcile_submission(&binding.authorization, &[]).is_err());
    let repeated = vec![binding.commands[0].clone(); 2];
    assert!(a.reconcile_submission(&binding.authorization, &repeated).is_err());
}

#[test]
fn mismatched_candidate_cannot_establish_a_fenced_submission() {
    let dir = tempfile::tempdir().unwrap();
    let a = authority(dir.path());
    let g = rotate(&a, 10);
    let j = journal(dir.path(), &a);
    let first = command(&cfg(), 30, 25);
    let second = command(&cfg(), 31, 5);
    j.enqueue(first.clone(), &Auth).unwrap();
    j.enqueue(second.clone(), &Auth).unwrap();
    let r = reserve(&j, &a);
    assert!(j.begin_commit(r.token, &a.head().unwrap()).is_err());
    let bad = request_at(&a.head().unwrap(), vec![second.clone(), first.clone()]);
    let auth = signer(10).authorize_commit(a.policy(), &g, &a.head().unwrap(), &bad).unwrap();
    assert!(j.begin_fenced_commit(r.token, a.policy().clone(), g.clone(), auth, &bad).is_err());
    let mut valid = request_at(&a.head().unwrap(), vec![first, second]);
    let auth = signer(10).authorize_commit(a.policy(), &g, &a.head().unwrap(), &valid).unwrap();
    valid.frame_input.payload.push(1);
    assert!(j.begin_fenced_commit(r.token, a.policy().clone(), g, auth, &valid).is_err());
    assert_eq!(j.active_reservation().unwrap().unwrap().phase, ReservationPhase::Held);
}

#[test]
fn mixed_conflict_and_missing_observations_preserve_exact_remaining_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let a = authority(dir.path());
    let g = rotate(&a, 10);
    let j = journal(dir.path(), &a);
    let first = command(&cfg(), 30, 25);
    let second = command(&cfg(), 31, 1);
    let (r, binding, _) = begin(&j, &a, &g, 10, vec![first, second.clone()]);
    let competing = request_at(&a.head().unwrap(), vec![command(&cfg(), 30, 10)]);
    let auth = signer(10).authorize_commit(a.policy(), &g, &a.head().unwrap(), &competing).unwrap();
    a.commit(auth, competing, 1).unwrap();
    let g2 = rotate(&a, 11);
    let proof = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
    j.reconcile_fenced(r.token, &proof, |_, _| panic!("different payload's result must not be returned")).unwrap();
    assert!(matches!(restored_status(&j, [30; 32]), EntryStatus::IdentityConflict(_)));
    assert_eq!(restored_status(&j, [31; 32]), EntryStatus::Queued);
    let (retry, binding, request) = begin(&j, &a, &g2, 11, vec![second]);
    a.commit(binding.authorization.clone(), request, 2).unwrap();
    j.reconcile_fenced(retry.token, &a.reconcile_submission(&binding.authorization, &binding.commands).unwrap(), result).unwrap();
    assert_eq!(j.acknowledge([31; 32]).unwrap(), BusinessOutcome::Succeeded(89u64.to_le_bytes().to_vec()));
}

#[test]
fn newer_writer_commit_racing_missing_observation_never_creates_a_second_charge() {
    for _ in 0..8 {
        let dir = tempfile::tempdir().unwrap();
        let a = authority(dir.path());
        let g1 = rotate(&a, 10);
        let j = journal(dir.path(), &a);
        let cmd = command(&cfg(), 30, 25);
        let (r, old, _) = begin(&j, &a, &g1, 10, vec![cmd.clone()]);
        let g2 = rotate(&a, 11);
        let request = request_at(&a.head().unwrap(), vec![cmd.clone()]);
        let auth = signer(11).authorize_commit(a.policy(), &g2, &a.head().unwrap(), &request).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let proof = std::thread::scope(|scope| {
            let observation = scope.spawn(|| {
                barrier.wait();
                a.reconcile_submission(&old.authorization, &old.commands).unwrap()
            });
            let commit = scope.spawn(|| {
                barrier.wait();
                a.commit(auth, request, 1).unwrap()
            });
            commit.join().unwrap();
            observation.join().unwrap()
        });
        j.reconcile_fenced(r.token, &proof, result).unwrap();
        if restored_status(&j, [30; 32]) == EntryStatus::Queued {
            let (retry, binding, request) = begin(&j, &a, &g2, 11, vec![cmd]);
            assert!(a.commit(binding.authorization.clone(), request, 2).is_err());
            let proof = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
            j.reconcile_fenced(retry.token, &proof, result).unwrap();
        }
        assert_eq!(j.acknowledge([30; 32]).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
        assert_eq!(a.head().unwrap().finalised_height, Some(0));
    }
}

#[test]
fn fenced_journal_crash_child() {
    let Ok(root) = std::env::var("MYELIN_FENCED_JOURNAL_ROOT") else { return };
    let point = std::env::var("MYELIN_FENCED_JOURNAL_POINT").unwrap();
    let root = Path::new(&root);
    let stop = |name: &str| {
        if name == point {
            std::process::exit(75)
        }
    };
    let a = authority(root);
    let g1 = rotate(&a, 10);
    let j = journal(root, &a);
    let cmd = command(&cfg(), 30, 25);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    let r = reserve(&j, &a);
    let request = request_at(&a.head().unwrap(), vec![cmd.clone()]);
    let auth = signer(10).authorize_commit(a.policy(), &g1, &a.head().unwrap(), &request).unwrap();
    stop("before-binding");
    let binding = j.begin_fenced_commit(r.token, a.policy().clone(), g1, auth, &request).unwrap();
    stop("after-binding");
    if point == "after-original-commit" {
        a.commit(binding.authorization.clone(), request, 1).unwrap();
        stop("after-original-commit");
    }
    let g2 = rotate(&a, 11);
    stop("after-rotation");
    let proof = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
    stop("after-observation");
    j.reconcile_fenced(r.token, &proof, result).unwrap();
    stop("after-release");
    let (retry, binding, request) = begin(&j, &a, &g2, 11, vec![cmd]);
    stop("after-rebind");
    a.commit(binding.authorization.clone(), request, 1).unwrap();
    stop("after-retry-commit");
    j.reconcile_fenced(retry.token, &a.reconcile_submission(&binding.authorization, &binding.commands).unwrap(), result).unwrap();
    stop("after-result");
    j.acknowledge([30; 32]).unwrap();
    stop("after-ack");
    panic!("unknown crash point");
}

#[test]
fn crashes_around_binding_fence_absence_retry_result_and_ack_recover_once() {
    for point in [
        "before-binding",
        "after-binding",
        "after-original-commit",
        "after-rotation",
        "after-observation",
        "after-release",
        "after-rebind",
        "after-retry-commit",
        "after-result",
        "after-ack",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fenced_journal_crash_child", "--nocapture"])
            .env("MYELIN_FENCED_JOURNAL_ROOT", dir.path())
            .env("MYELIN_FENCED_JOURNAL_POINT", point)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(75), "{point}: {}", String::from_utf8_lossy(&output.stderr));
        for _ in 0..2 {
            let a = authority(dir.path());
            let j = journal(dir.path(), &a);
            let cmd = command(&cfg(), 30, 25);
            j.enqueue(cmd.clone(), &Auth).unwrap();
            if let Some(r) = j.active_reservation().unwrap() {
                if r.phase == ReservationPhase::Held {
                    j.release_unsubmitted(r.token).unwrap();
                } else {
                    let binding = r.fenced_submission.unwrap();
                    if a.head().unwrap().finalised_height.is_none()
                        && a.current_grant().unwrap().unwrap().epoch <= binding.authorization.epoch
                    {
                        rotate(&a, 12);
                    }
                    let proof = a.reconcile_submission(&binding.authorization, &binding.commands).unwrap();
                    j.reconcile_fenced(r.token, &proof, result).unwrap();
                }
            }
            if restored_status(&j, [30; 32]) == EntryStatus::Queued {
                let grant = rotate(&a, 12);
                let (r, binding, request) = begin(&j, &a, &grant, 12, vec![cmd.clone()]);
                a.commit(binding.authorization.clone(), request, 1).unwrap();
                j.reconcile_fenced(r.token, &a.reconcile_submission(&binding.authorization, &binding.commands).unwrap(), result)
                    .unwrap();
            }
            assert_eq!(j.acknowledge([30; 32]).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
            assert_eq!(j.enqueue(cmd, &Auth).unwrap().original, command(&cfg(), 30, 25));
            assert_eq!(a.head().unwrap().finalised_height, Some(0));
            a.replay_range(0, 0, |bytes| {
                Ok(Inventory { scope: CommandScope::from_config(&cfg()), balance: u64::from_le_bytes(bytes.try_into().unwrap()) })
            })
            .unwrap();
        }
    }
}
