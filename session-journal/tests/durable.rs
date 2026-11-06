mod support;
use myelin_session::*;
use myelin_session_journal::*;
use myelin_session_producer::{CommitError, ProductionReceipt};
use std::path::Path;
use support::*;

fn journal(path: &Path, config: &SessionConfig) -> CommandJournal {
    CommandJournal::open(path, config, JournalLimits::default()).unwrap()
}
fn reserve(j: &CommandJournal, chain: &Chain) -> Reservation {
    j.reserve(chain.head(), 16, 100_000, &Auth, |c, _| BuildDecision::Ready(serde_json::to_vec(c).unwrap())).unwrap().unwrap()
}

#[test]
fn original_signature_identity_and_result_survive_lost_reply_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let (mut chain, store) = open_chain(&dir.path().join("chain"));
    let j = journal(&dir.path().join("journal"), chain.config());
    let cmd = command(chain.config(), 30, 25);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    assert!(matches!(j.enqueue(command(chain.config(), 30, 26), &Auth), Err(JournalError::IdentityConflict)));
    let mut wrong = cmd.clone();
    wrong.scope.session_id = [2; 32];
    assert!(j.enqueue(wrong, &Auth).is_err());
    let mut forged = cmd.clone();
    forged.authorization[0] ^= 1;
    assert!(j.enqueue(forged, &Auth).is_err());
    let (v, _) = consensus();
    let other = config(&v, 2, 100);
    let other_journal = journal(&dir.path().join("other-journal"), &other);
    let mut retargeted = cmd.clone();
    retargeted.scope = CommandScope::from_config(&other);
    assert!(matches!(other_journal.enqueue(retargeted, &Auth), Err(JournalError::Authentication(_))));
    let r = reserve(&j, &chain);
    j.begin_commit(r.token, chain.head()).unwrap();
    commit(&mut chain, vec![cmd.clone()]);
    // No journal write after durable chain commit: process/HTTP response lost.
    drop(j);
    let j = journal(&dir.path().join("journal"), chain.config());
    assert!(j.release_unsubmitted(r.token).is_err());
    j.reconcile(&chain, |_, _| Ok(None)).unwrap();
    assert!(j.acknowledge(cmd.identity.id).is_err()); // inclusion isn't success
    j.reconcile(&chain, result).unwrap();
    let outcome = j.acknowledge(cmd.identity.id).unwrap();
    assert_eq!(outcome, BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
    let entry = j.enqueue(cmd.clone(), &Auth).unwrap();
    assert!(matches!(entry.status, EntryStatus::Committed { acknowledged: true, .. }));
    assert!(j.reserve(chain.head(), 1, 100_000, &Auth, |_, _| panic!("must not rebuild committed purchase")).unwrap().is_none());
    assert!(chain.prepare_block(request(&chain, vec![cmd])).is_err());
    assert_eq!(store.load_checkpoint(chain.config().session_id).unwrap().state_snapshot, 75u64.to_le_bytes());
}

#[test]
fn stale_prefix_reconstruction_preserves_intents_and_unknown_work() {
    let dir = tempfile::tempdir().unwrap();
    let (chain, _) = open_chain(&dir.path().join("chain"));
    let j = journal(&dir.path().join("journal"), chain.config());
    for n in 30..33 {
        j.enqueue(command(chain.config(), n, 1), &Auth).unwrap();
    }
    let r = j
        .reserve(chain.head(), 3, 1000, &Auth, |c, _| {
            if c.identity.id == [31; 32] {
                BuildDecision::Stale("missing live input".into())
            } else {
                BuildDecision::Ready(vec![7])
            }
        })
        .unwrap()
        .unwrap();
    assert_eq!(r.command_ids, vec![[30; 32]]);
    assert!(matches!(j.entry([31; 32]).unwrap().unwrap().status, EntryStatus::Stale(_)));
    assert_eq!(j.entry([32; 32]).unwrap().unwrap().status, EntryStatus::Queued);
    let mut next = chain.head().clone();
    next.timestamp_ms += 1;
    assert!(j.begin_commit(r.token, &next).is_err());
    j.release_unsubmitted(r.token).unwrap();
    j.retry_stale([31; 32]).unwrap();
    let r = j.reserve(&next, 3, 1000, &Auth, |_, h| BuildDecision::Ready(h.timestamp_ms.to_le_bytes().to_vec())).unwrap().unwrap();
    assert_eq!(r.command_ids, vec![[30; 32], [31; 32], [32; 32]]);
    assert_ne!(r.work[0], vec![7]);
    let s = j.submit(r.token, &next, |_| Err(CommitError::OutcomeUnknown("reply lost".into()))).unwrap();
    assert!(matches!(s, Submission::Unknown(_)));
    drop(j);
    let j = journal(&dir.path().join("journal"), chain.config());
    assert!(j.reconcile(&chain, result).unwrap().is_empty());
    assert_eq!(j.active_reservation().unwrap().unwrap().phase, ReservationPhase::Unknown);
    assert!(j.release_unsubmitted(r.token).is_err());
    assert!(j.retry_stale([30; 32]).is_err());
    assert!(j.reserve(chain.head(), 1, 1000, &Auth, |_, _| panic!()).is_err());
    for entry in j.entries(0, 3).unwrap() {
        assert_eq!(entry.original, command(chain.config(), entry.original.identity.id[0], 1));
    }
}

#[test]
fn definite_rejection_staleness_and_business_rejection_are_different() {
    let dir = tempfile::tempdir().unwrap();
    let (mut chain, _) = open_chain(&dir.path().join("chain"));
    let j = journal(&dir.path().join("journal"), chain.config());
    let cmd = command(chain.config(), 30, 101);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    let r = reserve(&j, &chain);
    assert_eq!(j.submit(r.token, chain.head(), |_| Err(CommitError::StaleCandidate)).unwrap(), Submission::Stale);
    assert_eq!(j.entry(cmd.identity.id).unwrap().unwrap().status, EntryStatus::Queued);
    let r = reserve(&j, &chain);
    let base = chain.head().clone();
    assert!(matches!(
        j.submit(r.token, &base, |_| {
            commit(&mut chain, vec![cmd.clone()]);
            Ok(ProductionReceipt { block_height: chain.head().finalised_height.unwrap(), block_hash: chain.head().block_hash })
        })
        .unwrap(),
        Submission::Committed(_)
    ));
    j.reconcile(&chain, result).unwrap();
    assert_eq!(j.acknowledge(cmd.identity.id).unwrap(), BusinessOutcome::Rejected(b"insufficient balance".to_vec()));
    let cmd = command(chain.config(), 31, 5);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    let r = reserve(&j, &chain);
    assert_eq!(
        j.submit(r.token, chain.head(), |_| Err(CommitError::Rejected("invalid admission".into()))).unwrap(),
        Submission::Rejected("invalid admission".into())
    );
    assert_eq!(j.entry(cmd.identity.id).unwrap().unwrap().status, EntryStatus::Rejected("invalid admission".into()));
    assert!(j.acknowledge(cmd.identity.id).is_err());
}

#[test]
fn committed_identity_conflict_never_returns_another_payloads_result() {
    let dir = tempfile::tempdir().unwrap();
    let (mut chain, _) = open_chain(&dir.path().join("chain"));
    let j = journal(&dir.path().join("journal"), chain.config());
    let cmd = command(chain.config(), 30, 25);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    let r = reserve(&j, &chain);
    j.begin_commit(r.token, chain.head()).unwrap();
    let other = command(chain.config(), 30, 10);
    commit(&mut chain, vec![other]);
    j.reconcile(&chain, |_, _| panic!("conflict has no result for this caller")).unwrap();
    assert!(matches!(j.entry(cmd.identity.id).unwrap().unwrap().status, EntryStatus::IdentityConflict(_)));
    assert!(chain.command_record(&cmd.identity).is_err());
    assert!(j.acknowledge(cmd.identity.id).is_err());
}

#[test]
fn queued_retry_recovers_an_existing_receipt_without_submission() {
    let dir = tempfile::tempdir().unwrap();
    let (mut chain, _) = open_chain(&dir.path().join("chain"));
    let cmd = command(chain.config(), 30, 25);
    commit(&mut chain, vec![cmd.clone()]);
    let j = journal(&dir.path().join("journal"), chain.config());
    j.enqueue(cmd.clone(), &Auth).unwrap();
    j.reconcile(&chain, result).unwrap();
    assert_eq!(j.acknowledge(cmd.identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
}

#[test]
fn profile_scope_limits_and_checksum_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (chain, _) = open_chain(&dir.path().join("chain"));
    let path = dir.path().join("journal");
    let limits = JournalLimits { max_entries: 1, ..JournalLimits::default() };
    let j = CommandJournal::open(&path, chain.config(), limits).unwrap();
    let cmd = command(chain.config(), 30, 25);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    assert!(j.enqueue(command(chain.config(), 31, 25), &Auth).is_err());
    assert_eq!(j.enqueue(cmd, &Auth).unwrap().sequence, 0);
    drop(j);
    assert!(CommandJournal::open(&path, chain.config(), JournalLimits::default()).is_err());
    let mut wrong = chain.config().clone();
    wrong.application_profile.command_identity_required = false;
    assert!(CommandJournal::open(dir.path().join("tape"), &wrong, limits).is_err());
    let db = rocksdb::DB::open_default(&path).unwrap();
    let mut bytes = db.get(b"myelin-command-journal").unwrap().unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    db.put(b"myelin-command-journal", bytes).unwrap();
    drop(db);
    assert!(CommandJournal::open(&path, chain.config(), limits).is_err());
}

#[test]
fn journal_crash_child() {
    let Ok(root) = std::env::var("MYELIN_JOURNAL_CRASH_ROOT") else { return };
    let point: u8 = std::env::var("MYELIN_JOURNAL_CRASH_POINT").unwrap().parse().unwrap();
    let root = Path::new(&root);
    let (mut chain, _) = open_chain(&root.join("chain"));
    let j = journal(&root.join("journal"), chain.config());
    let cmd = command(chain.config(), 30, 25);
    let stop = |n| {
        if point == n {
            std::process::exit(73)
        }
    };
    stop(0);
    j.enqueue(cmd.clone(), &Auth).unwrap();
    stop(1);
    let r = reserve(&j, &chain);
    stop(2);
    let p = chain.prepare_block(request(&chain, vec![cmd.clone()])).unwrap();
    drop(p);
    stop(3);
    j.begin_commit(r.token, chain.head()).unwrap();
    stop(4);
    commit(&mut chain, vec![cmd.clone()]);
    stop(5);
    j.reconcile(&chain, result).unwrap();
    stop(6);
    j.acknowledge(cmd.identity.id).unwrap();
    stop(7);
    // Represents the reply becoming available to the HTTP caller.
    std::fs::write(root.join("reply"), b"75").unwrap();
    stop(8);
    panic!("unknown crash point");
}

#[test]
fn crashes_around_write_reserve_prepare_commit_ack_and_http_response() {
    for point in 0..=8 {
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "journal_crash_child", "--nocapture"])
            .env("MYELIN_JOURNAL_CRASH_ROOT", dir.path())
            .env("MYELIN_JOURNAL_CRASH_POINT", point.to_string())
            .output()
            .unwrap();
        assert_eq!(status.status.code(), Some(73), "{point}: {}", String::from_utf8_lossy(&status.stderr));
        for _ in 0..2 {
            let (mut chain, store) = open_chain(&dir.path().join("chain"));
            let j = journal(&dir.path().join("journal"), chain.config());
            let cmd = command(chain.config(), 30, 25);
            j.enqueue(cmd.clone(), &Auth).unwrap();
            if point < 4 && chain.head().finalised_height.is_none() {
                if let Some(r) = j.active_reservation().unwrap() {
                    j.release_unsubmitted(r.token).unwrap();
                }
                let r = reserve(&j, &chain);
                j.begin_commit(r.token, chain.head()).unwrap();
                commit(&mut chain, vec![cmd.clone()]);
            }
            j.reconcile(&chain, result).unwrap();
            if point == 4 {
                let r = j.active_reservation().unwrap().unwrap();
                assert!(j.release_unsubmitted(r.token).is_err());
                assert!(matches!(chain.command_status(&cmd.identity).unwrap(), CommandStatus::Missing));
                assert_eq!(store.load_checkpoint(chain.config().session_id).unwrap().state_snapshot, 100u64.to_le_bytes());
            } else {
                assert_eq!(j.acknowledge(cmd.identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
                assert_eq!(chain.head().finalised_height, Some(0));
                let scope = CommandScope::from_config(chain.config());
                chain
                    .replay_range(0, 0, move |snapshot| {
                        Ok(Inventory { scope: scope.clone(), balance: u64::from_le_bytes(snapshot.try_into().unwrap()) })
                    })
                    .unwrap();
                assert_eq!(store.load_checkpoint(chain.config().session_id).unwrap().state_snapshot, 75u64.to_le_bytes());
                assert!(matches!(j.enqueue(cmd, &Auth).unwrap().status, EntryStatus::Committed { acknowledged: true, .. }));
            }
        }
    }
}
