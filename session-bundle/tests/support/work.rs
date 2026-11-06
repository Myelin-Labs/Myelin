use super::*;
use std::io::{BufRead, BufReader, Write};

// Finality work does not execute the program. A small pinned artifact keeps
// these scheduling tests independent of the debug test executable's size.
pub(super) fn work_transfer() -> Fixture {
    let mut f = Fixture::with_program(TransferKind::Scarce, BlobId::of(&vec![42; 3 * 65536]).0);
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    let req = intake_request(&f.target, ids);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    f
}
pub(super) fn small_plan(f: &Fixture, input: &Path) -> ExportPlan {
    let mut plan = plan(f, input);
    let old = plan.sessions[0].execution.program;
    let bytes = vec![42; 3 * 65536];
    let id = BlobId::of(&bytes);
    let path = input.join("program");
    fs::write(&path, bytes).unwrap();
    let artifact = plan.artifacts.iter_mut().find(|a| a.id == old).unwrap();
    artifact.id = id;
    artifact.path = path;
    for s in &mut plan.sessions {
        s.execution.program = id;
        s.execution.replayer = id;
    }
    plan
}
fn paused(outcome: ArchiveOutcome) -> Box<PausedArchive> {
    match outcome {
        ArchiveOutcome::Paused(cursor) => cursor,
        _ => panic!("expected a retained cursor"),
    }
}
fn slice(steps: u64) -> ArchiveBudget {
    ArchiveBudget { max_steps: steps, ..ArchiveBudget::default() }
}

#[test]
fn verify_retains_global_progress_across_budget_cancel_and_deadline_pauses() {
    let f = work_transfer();
    let dir = tempfile::tempdir().unwrap();
    let plan = small_plan(&f, dir.path());
    let archive = dir.path().join("archive");
    let manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
    let trusted = anchors(&manifest);
    let task = ArchiveTask::verify(Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap()), trusted).unwrap();
    let cancelled = RecoveryCancellation::default();
    cancelled.cancel();
    let cursor = paused(task.advance(slice(0), &cancelled).unwrap());
    assert_eq!(cursor.reason(), ArchivePauseReason::Cancelled);
    assert_eq!(cursor.progress().total.steps, 0);
    let token = RecoveryCancellation::default();
    let cursor = paused(cursor.resume(ArchiveBudget { max_elapsed_ms: 0, ..slice(10) }, &token).unwrap());
    assert_eq!(cursor.reason(), ArchivePauseReason::Deadline);
    let cursor = paused(cursor.resume(ArchiveBudget { max_io_bytes: 0, ..slice(100_000) }, &token).unwrap());
    assert_eq!(cursor.reason(), ArchivePauseReason::Bytes);
    assert_eq!(cursor.progress().total.io_bytes, 0);
    let mut previous = cursor.progress().total;
    let mut outcome = cursor.resume(slice(17), &token).unwrap();
    let mut stages = BTreeSet::new();
    let mut pauses = 0;
    loop {
        match outcome {
            ArchiveOutcome::Paused(cursor) => {
                assert_eq!(cursor.reason(), ArchivePauseReason::Steps);
                assert_eq!(cursor.progress().step.steps, 17);
                assert_eq!(cursor.progress().total.steps, previous.steps + 17);
                assert!(cursor.progress().total.io_bytes >= previous.io_bytes);
                stages.insert(format!("{:?}", cursor.progress().stage));
                previous = cursor.progress().total;
                pauses += 1;
                assert!(pauses < 1000, "work must progress, not rescan the inventory");
                outcome = cursor.resume(slice(17), &token).unwrap();
            }
            ArchiveOutcome::Complete { output, usage } => {
                assert!(usage.steps >= previous.steps);
                let ArchiveOutput::Verified { store, report } = *output else {
                    panic!("wrong operation");
                };
                assert!(matches!(report.stages.finality, VerificationStage::Verified));
                assert!(matches!(report.stages.host_replay, VerificationStage::NotRun));
                assert_eq!(store.load_head(f.source.config().session_id).unwrap(), *f.source.head());
                assert_eq!(store.load_head(f.target.config().session_id).unwrap(), *f.target.head());
                break;
            }
        }
    }
    assert!(pauses > 1);
    assert!(stages.contains("Inventory") && stages.contains("Finality"));
}

#[test]
fn inventory_rejects_an_already_hashed_prefix_changed_while_paused() {
    let f = work_transfer();
    let dir = tempfile::tempdir().unwrap();
    let plan = small_plan(&f, dir.path());
    let archive = dir.path().join("archive");
    let manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
    let program = plan.sessions[0].execution.program;
    let task = ArchiveTask::inventory(Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap()), anchors(&manifest)).unwrap();
    let token = RecoveryCancellation::default();
    let mut cursor = paused(task.advance(slice(1), &token).unwrap());
    for _ in 0..1000 {
        if cursor.progress().position.as_deref() == Some(program.hex().as_str()) && cursor.progress().step.io_bytes == 65536 {
            // The first chunk has already entered the retained hash state.
            fs::OpenOptions::new().write(true).open(archive.join("blobs").join(program.hex())).unwrap().write_all(&[99]).unwrap();
            error(cursor.resume(ArchiveBudget::default(), &token), ErrorCode::SourceChanged);
            return;
        }
        cursor = paused(cursor.resume(slice(1), &token).unwrap());
    }
    panic!("never paused in the streamed program");
}

#[test]
fn export_keeps_manifest_unpublished_until_resumption_and_rechecks_the_source() {
    for advance_source in [false, true] {
        let mut f = work_transfer();
        let dir = tempfile::tempdir().unwrap();
        let plan = small_plan(&f, dir.path());
        let archive = dir.path().join("archive");
        let token = RecoveryCancellation::default();
        let task = ArchiveTask::export(f.store.clone(), plan, archive.clone()).unwrap();
        let mut cursor = paused(task.advance(slice(0), &token).unwrap());
        assert!(!archive.exists());
        for _ in 0..2000 {
            assert!(!archive.join("manifest.json").exists());
            if cursor.progress().stage == ArchiveStage::Publish {
                break;
            }
            cursor = paused(cursor.resume(slice(1), &token).unwrap());
        }
        assert_eq!(cursor.progress().stage, ArchiveStage::Publish);
        // One final check contains both heads/checkpoints and the shared claim.
        // Undersized grants must expose that requirement before any prefix is
        // reread; otherwise a caller can loop forever despite a stable source.
        let cursor = paused(cursor.resume(slice(1), &token).unwrap());
        let required = cursor.progress().next_operation.expect("publication reservation must be explicit");
        assert_eq!(required.steps, 5);
        assert_eq!(required.io_bytes, 5 * myelin_session::bounded_codec::MAX_RECORD_BYTES as u64);
        let previous = cursor.progress().total;
        let cursor = paused(cursor.resume(slice(required.steps - 1), &token).unwrap());
        assert_eq!(cursor.reason(), ArchivePauseReason::Steps);
        assert_eq!(cursor.progress().step.steps, 0);
        assert_eq!(cursor.progress().step.io_bytes, 0);
        assert_eq!(cursor.progress().total.steps, previous.steps);
        let cursor = paused(
            cursor
                .resume(
                    ArchiveBudget { max_steps: required.steps, max_io_bytes: required.io_bytes - 1, ..ArchiveBudget::default() },
                    &token,
                )
                .unwrap(),
        );
        assert_eq!(cursor.reason(), ArchivePauseReason::Bytes);
        assert_eq!(cursor.progress().step.steps, 0);
        assert_eq!(cursor.progress().step.io_bytes, 0);
        assert!(!archive.join("manifest.json").exists());
        let final_budget = ArchiveBudget { max_steps: required.steps, max_io_bytes: required.io_bytes, ..ArchiveBudget::default() };
        if advance_source {
            let req = issue_request(&f.source, vec![f.intent(2, 1, 1)]);
            commit(&mut f.source, &f.verifier, &f.validator, req);
            error(cursor.resume(final_budget, &token), ErrorCode::SourceChanged);
            assert!(!archive.join("manifest.json").exists());
        } else {
            let ArchiveOutcome::Complete { output, .. } = cursor.resume(final_budget, &token).unwrap() else {
                panic!("export must finish");
            };
            let ArchiveOutput::Exported(manifest) = *output else {
                panic!("wrong operation");
            };
            assert_eq!(fs::read(archive.join("manifest.json")).unwrap(), manifest.encode().unwrap());
            let (_, report) = open(&archive, &anchors(&manifest)).unwrap();
            assert!(matches!(report.stages.finality, VerificationStage::Verified));
        }
    }
}

#[test]
fn work_cli_accepts_multiple_grants_in_one_process_and_emits_no_early_verified_report() {
    use std::process::{Command, Stdio};
    let f = work_transfer();
    let dir = tempfile::tempdir().unwrap();
    let plan = small_plan(&f, dir.path());
    let archive = dir.path().join("archive");
    let manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
    let trust = dir.path().join("trust.json");
    fs::write(&trust, serde_json::to_vec(&anchors(&manifest)).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .arg("work")
        .arg("verify")
        .arg(&archive)
        .arg(trust)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for (i, grant) in [slice(0), slice(37), ArchiveBudget::default()].into_iter().enumerate() {
        writeln!(input, "{}", serde_json::json!({"budget":grant,"cancelled":false})).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let result: serde_json::Value = serde_json::from_str(&line).unwrap();
        if i < 2 {
            assert_eq!(result["status"], "paused");
            assert!(result.get("report").is_none());
            assert!(child.try_wait().unwrap().is_none());
        } else {
            assert_eq!(result["status"], "complete");
            assert_eq!(result["report"]["stages"]["finality"], "verified");
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}
