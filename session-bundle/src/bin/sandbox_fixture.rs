//! Adversarial runner for the explicitly enabled local sandbox acceptance lane.
#[cfg(target_os = "linux")]
#[path = "../../tests/support/migration_program.rs"]
mod migration_program;
#[cfg(target_os = "linux")]
mod fixture {
    use myelin_session_bundle::{
        runner::{replay_output, replayer_input},
        BlobId,
    };
    use std::{fs, io::Write};
    pub fn run() {
        let (store, session) = replayer_input().unwrap();
        let inventory = store.manifest().sessions.iter().find(|s| s.session_id == session).unwrap();
        let probe: serde_json::Value =
            serde_json::from_slice(&store.read_artifact(inventory.execution.rules, 64 * 1024).unwrap()).unwrap();
        match probe["mode"].as_str().unwrap() {
            "probe" => {
                // Run the existing isolation probes on a real worker thread:
                // allowing archive threads must not drop the inherited filter.
                std::thread::spawn(move || {
                    let secret = probe["secret"].as_str().unwrap();
                    assert!(fs::read(secret).is_err());
                    assert!(fs::write(secret, b"changed").is_err());
                    assert!(fs::write("/bundle/manifest.json", b"changed").is_err());
                    assert!(fs::write("/tmp/must-not-exist", b"changed").is_err());
                    assert!(fs::write("/dev/shm/must-not-exist", b"changed").is_err());
                    assert!(std::env::var_os("MYELIN_SANDBOX_PRIVATE_SENTINEL").is_none());
                    assert!(fs::read("/bundle/manifest.json").is_ok());
                    assert!(fs::read("/proc/self/exe").is_ok());
                    unsafe {
                        let inherited = probe["fd"].as_i64().unwrap() as i32;
                        assert_eq!(libc::fcntl(inherited, libc::F_GETFD), -1);
                        assert_eq!(libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0), -1);
                        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));
                        assert_eq!(libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0), -1);
                        assert_eq!(libc::syscall(libc::SYS_clone, libc::SIGCHLD, 0, 0, 0, 0), -1);
                        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));
                        let flags = libc::CLONE_VM | libc::CLONE_FS | libc::CLONE_FILES | libc::CLONE_SIGHAND | libc::CLONE_THREAD;
                        for denied in
                            [u64::from(flags as u32) | (1 << 32), (flags | libc::CLONE_NEWUSER) as u64, (flags | libc::SIGCHLD) as u64]
                        {
                            assert_eq!(libc::syscall(libc::SYS_clone, denied, 0, 0, 0, 0), -1);
                            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));
                        }
                        assert_eq!(libc::syscall(libc::SYS_clone3, std::ptr::null::<u8>(), 0), -1);
                        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ENOSYS));
                        assert_eq!(libc::syscall(libc::SYS_unshare, libc::CLONE_NEWUSER), -1);
                        assert_eq!(libc::syscall(libc::SYS_io_uring_setup, 1, std::ptr::null::<u8>()), -1);
                        assert_eq!(libc::fcntl(1, libc::F_SETOWN, 1), -1);
                    }
                    let mut descriptors = vec![];
                    while let Ok(file) = fs::File::open("/dev/null") {
                        descriptors.push(file);
                        assert!(descriptors.len() <= 256, "descriptor limit missing");
                    }
                    assert!(!descriptors.is_empty());
                    drop(descriptors);
                })
                .join()
                .unwrap();
                eprintln!("sandbox probes passed");
                // Deliberately no replay claim: the host must reject this output.
                println!("{{}}");
            }
            "flood" => loop {
                std::io::stdout().write_all(&[b'x'; 8192]).unwrap();
            },
            "loop" | "cpu" => loop {
                std::hint::black_box(42u64.wrapping_mul(17));
            },
            "allocation" => {
                let size = 32usize * 1024 * 1024 * 1024;
                let result = unsafe {
                    libc::mmap(
                        std::ptr::null_mut(),
                        size,
                        libc::PROT_READ | libc::PROT_WRITE,
                        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                        -1,
                        0,
                    )
                };
                assert_eq!(result, libc::MAP_FAILED);
                eprintln!("allocation denied");
                println!("{{}}");
            }
            "wrong-receipt" => {
                let cfg = store.config(session).unwrap().clone();
                let receipt = store
                    .replay_current_process(session, move |bytes, store| {
                        myelin_session_transfer::restore_executor(&cfg, bytes, store)
                            .map_err(|e| myelin_session::SessionError::Replay(e.to_string()))
                    })
                    .unwrap();
                let mut result = replay_output(&store, session, receipt).unwrap();
                result.receipt.as_mut().unwrap().total_input_bytes += 1;
                println!("{}", serde_json::to_string(&result).unwrap());
            }
            "migration" => {
                let artifact: BlobId = serde_json::from_value(probe["artifact"].clone()).unwrap();
                let receipt = store.replay_current_process(session, move |snapshot, store| {
                    let program =
                        store.read_artifact(artifact, 64).map_err(|e| myelin_session::SessionError::Replay(e.to_string()))?;
                    super::migration_program::Converting::restore(snapshot, program, false)
                        .map_err(myelin_session::SessionError::Replay)
                });
                match receipt {
                    Ok(receipt) => println!("{}", serde_json::to_string(&replay_output(&store, session, receipt).unwrap()).unwrap()),
                    Err(error) => {
                        eprintln!("migration replay failed: {error}");
                        std::process::exit(2);
                    }
                }
            }
            "exit" => std::process::exit(71),
            mode => panic!("unknown fixture mode {mode} for {}", BlobId(session).hex()),
        }
    }
}

#[cfg(target_os = "linux")]
fn main() {
    fixture::run();
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("sandbox fixture requires Linux");
    std::process::exit(2);
}
