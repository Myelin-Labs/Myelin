use myelin_session::RecoveryCancellation;
use myelin_session_bundle::{
    ArchiveBudget, ArchiveOutcome, ArchiveTask, BackupPlan, BlobId, BundleArchives, BundleDirectory, BundleError, BundleLimits,
    CollectionPlan, ErrorCode, ExportPlan, PackPlan, RetentionPlan, TrustAnchor,
};
use myelin_session_store_rocksdb::{RocksSessionStore, RocksSessionStoreOptions};
use std::{
    fs::OpenOptions,
    io::{BufRead, Read, Write},
    path::Path,
    sync::Arc,
};

fn fail(code: ErrorCode, message: impl Into<String>) -> BundleError {
    BundleError { code, message: message.into(), execution: None, paused: None, progress: None }
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path, cap: u64) -> Result<T, BundleError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let trust = options.open(path).map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
    if !trust.metadata().map_err(|e| fail(ErrorCode::Io, e.to_string()))?.is_file() {
        return Err(fail(ErrorCode::UnsafePath, "JSON input must be a regular file"));
    }
    let mut bytes = vec![];
    trust.take(cap + 1).read_to_end(&mut bytes).map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
    if bytes.len() as u64 > cap {
        return Err(fail(ErrorCode::Limit, "JSON input exceeds byte limit"));
    }
    serde_json::from_slice(&bytes).map_err(|e| fail(ErrorCode::Format, e.to_string()))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    budget: ArchiveBudget,
    cancelled: bool,
}

fn drive(task: ArchiveTask, interactive: bool) -> Result<serde_json::Value, BundleError> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let grant = |input: &mut std::io::StdinLock<'_>| -> Result<Grant, BundleError> {
        if !interactive {
            return Ok(Grant { budget: ArchiveBudget::default(), cancelled: false });
        }
        let mut line = vec![];
        input.take(65537).read_until(b'\n', &mut line).map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
        if line.is_empty() {
            return Err(fail(ErrorCode::Cancelled, "work input closed; process-local cursor discarded"));
        }
        if line.len() > 65536 {
            return Err(fail(ErrorCode::Limit, "work grant exceeds 64 KiB"));
        }
        serde_json::from_slice(&line).map_err(|e| fail(ErrorCode::Format, e.to_string()))
    };
    let token = |cancelled| {
        let t = RecoveryCancellation::default();
        if cancelled {
            t.cancel();
        }
        t
    };
    let first = grant(&mut input)?;
    let mut outcome = task.advance(first.budget, &token(first.cancelled))?;
    loop {
        match outcome {
            ArchiveOutcome::Complete { output, usage } => {
                let report = output.report()?;
                return Ok(if interactive { serde_json::json!({"status":"complete","report":report,"usage":usage}) } else { report });
            }
            ArchiveOutcome::Paused(cursor) => {
                if !interactive {
                    return Err(BundleError {
                        code: ErrorCode::Paused,
                        message: "archive paused; use work to retain and resume the cursor in this process".into(),
                        execution: None,
                        paused: Some(cursor),
                        progress: None,
                    });
                }
                let mut output = std::io::stdout().lock();
                writeln!(output, "{}", serde_json::json!({"status":"paused","reason":cursor.reason(),"progress":cursor.progress()}))
                    .map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
                output.flush().map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
                let next = grant(&mut input).map_err(|mut error| {
                    error.progress = Some(Box::new(cursor.progress().clone()));
                    error
                })?;
                outcome = cursor.resume(next.budget, &token(next.cancelled))?;
            }
        }
    }
}
fn run() -> Result<serde_json::Value, BundleError> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let interactive = args.first().is_some_and(|a| a == "work");
    if interactive {
        args.remove(0);
    }
    let valid = matches!(args.first().map(String::as_str), Some("inspect")) && (2..=3).contains(&args.len())
        || matches!(args.first().map(String::as_str), Some("verify-inventory" | "verify")) && (3..=4).contains(&args.len())
        || matches!(args.first().map(String::as_str), Some("export" | "backup")) && args.len() == 4;
    let valid = valid
        || matches!(args.first().map(String::as_str), Some("replay" | "assemble-files" | "restore")) && (5..=6).contains(&args.len())
        || matches!(args.first().map(String::as_str), Some("pack-files" | "retain")) && args.len() == 3
        || matches!(args.first().map(String::as_str), Some("collect")) && args.len() == 2;
    if !valid || (interactive && matches!(args[0].as_str(), "inspect" | "replay")) {
        return Err(fail(ErrorCode::Format, "usage: myelin-session-bundle [work] inspect <bundle> [limits.json] | verify-inventory|verify <bundle> <external-trust.json> [limits.json] | export <quiescent-rocksdb> <plan.json> <new-directory> | replay <bundle> <external-trust.json> <session-hex> <sandbox-policy.json> [limits.json] | backup <rocksdb> <plan.json> <new-directory> | restore <backup> <manifest-hex> <external-trust.json> <new-directory> [limits.json] | retain <rocksdb> <plan.json> | collect <plan.json> | pack-files <plan.json> <new-directory> | assemble-files <bundle> <external-trust.json> <package-hex> <new-directory> [limits.json]; work supports export, verify, verify-inventory, pack-files, assemble-files, retain, collect, backup and restore; stdin accepts JSON lines {budget:{max_steps,max_io_bytes,max_elapsed_ms},cancelled:bool}"));
    }
    if args[0] == "collect" {
        let path = Path::new(&args[1]);
        let mut plan: CollectionPlan = read_json(path, 8 * 1024 * 1024)?;
        for member in plan.databases.iter_mut().chain(plan.archives.iter_mut().chain(&mut plan.candidates).map(|c| &mut c.path)) {
            if member.is_relative() {
                *member = path.parent().unwrap_or(Path::new(".")).join(&*member);
            }
        }
        return drive(ArchiveTask::collect(plan)?, interactive);
    }
    if args[0] == "backup" {
        if !Path::new(&args[1]).join("CURRENT").is_file() {
            return Err(fail(ErrorCode::MissingArtifact, "backup requires an existing RocksDB store"));
        }
        let path = Path::new(&args[2]);
        let mut plan: BackupPlan = read_json(path, 8 * 1024 * 1024)?;
        for archive in &mut plan.archives {
            if archive.path.is_relative() {
                archive.path = path.parent().unwrap_or(Path::new(".")).join(&archive.path);
            }
        }
        let store = RocksSessionStore::open(&args[1]).map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
        return drive(ArchiveTask::backup(Arc::new(store), plan, args[3].clone().into())?, interactive);
    }
    if args[0] == "restore" {
        let expected: BlobId =
            serde_json::from_value(serde_json::Value::String(args[2].clone())).map_err(|e| fail(ErrorCode::Format, e.to_string()))?;
        let trust: Vec<TrustAnchor> = read_json(Path::new(&args[3]), 1024 * 1024)?;
        let limits: BundleLimits = args.get(5).map(|p| read_json(Path::new(p), 16 * 1024)).transpose()?.unwrap_or_default();
        return drive(ArchiveTask::restore(args[1].clone().into(), expected, trust, limits, args[4].clone().into())?, interactive);
    }
    if args[0] == "retain" {
        if !Path::new(&args[1]).join("CURRENT").is_file() {
            return Err(fail(ErrorCode::MissingArtifact, "retain requires an existing RocksDB store"));
        }
        let path = Path::new(&args[2]);
        let mut plan: RetentionPlan = read_json(path, 8 * 1024 * 1024)?;
        for archive in &mut plan.archives {
            if archive.path.is_relative() {
                archive.path = path.parent().unwrap_or(Path::new(".")).join(&archive.path);
            }
        }
        let archives = Arc::new(BundleArchives::default());
        let store = RocksSessionStore::open_archived(&args[1], RocksSessionStoreOptions::default(), archives.clone())
            .map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
        return drive(
            ArchiveTask::retain(Arc::new(store), archives, plan, Arc::new(myelin_session::RetainUntilTerminal))?,
            interactive,
        );
    }
    if args[0] == "pack-files" {
        let path = Path::new(&args[1]);
        let mut plan: PackPlan = read_json(path, 8 * 1024 * 1024)?;
        for file in &mut plan.files {
            if file.path.is_relative() {
                file.path = path.parent().unwrap_or(Path::new(".")).join(&file.path);
            }
        }
        return drive(ArchiveTask::pack(plan, args[2].clone().into())?, interactive);
    }
    if args[0] == "export" {
        if !Path::new(&args[1]).join("CURRENT").is_file() {
            return Err(fail(ErrorCode::MissingArtifact, "export requires an existing RocksDB store"));
        }
        let plan_path = Path::new(&args[2]);
        let mut plan: ExportPlan = read_json(plan_path, 8 * 1024 * 1024)?;
        for source in &mut plan.artifacts {
            if source.path.is_relative() {
                source.path = plan_path.parent().unwrap_or(Path::new(".")).join(&source.path);
            }
        }
        let store = RocksSessionStore::open(&args[1]).map_err(|e| fail(ErrorCode::Io, e.to_string()))?;
        return drive(ArchiveTask::export(Arc::new(store), plan, args[3].clone().into())?, interactive);
    }
    let limits_index = match args[0].as_str() {
        "inspect" => 2,
        "replay" | "assemble-files" => 5,
        _ => 3,
    };
    let limits: BundleLimits = args.get(limits_index).map(|p| read_json(Path::new(p), 16 * 1024)).transpose()?.unwrap_or_default();
    let directory = Arc::new(BundleDirectory::open(&args[1], limits)?);
    if args[0] == "inspect" {
        return Ok(serde_json::json!({"stage":"unverified-inventory", "manifest":directory.manifest()}));
    }
    let anchors: Vec<TrustAnchor> = read_json(Path::new(&args[2]), 1024 * 1024)?;
    if args[0] == "assemble-files" {
        let root: BlobId =
            serde_json::from_value(serde_json::Value::String(args[3].clone())).map_err(|e| fail(ErrorCode::Format, e.to_string()))?;
        return drive(ArchiveTask::assemble(directory, anchors, root, args[4].clone().into())?, interactive);
    }
    if args[0] == "replay" {
        use myelin_session_bundle::{
            runner::{replay_isolated, Cancellation, SandboxPolicy},
            BlobId,
        };
        let session: BlobId =
            serde_json::from_value(serde_json::Value::String(args[3].clone())).map_err(|e| fail(ErrorCode::Format, e.to_string()))?;
        let policy: SandboxPolicy = read_json(Path::new(&args[4]), 64 * 1024)?;
        return serde_json::to_value(replay_isolated(directory, &anchors, session.0, &policy, &Cancellation::default())?)
            .map_err(|e| fail(ErrorCode::Format, e.to_string()));
    }
    if args[0] == "verify" {
        drive(ArchiveTask::verify(directory, anchors)?, interactive)
    } else {
        drive(ArchiveTask::inventory(directory, anchors)?, interactive)
    }
}
fn main() {
    match run() {
        Ok(report) => println!("{}", serde_json::to_string(&report).expect("report is JSON")),
        Err(error) => {
            eprintln!("{}", serde_json::to_string(&error).expect("error is JSON"));
            std::process::exit(2);
        }
    }
}
