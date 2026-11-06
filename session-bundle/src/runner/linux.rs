use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{OpenOptionsExt, PermissionsExt},
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

fn io_error(error: impl std::fmt::Display) -> BundleError {
    BundleError::new(ErrorCode::Sandbox, error.to_string())
}
fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u64::MAX as u128) as u64
}
fn readonly(path: &Path, executable: bool) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 })).map_err(io_error)
}

fn launcher(policy: &SandboxPolicy, destination: &Path) -> Result<()> {
    let mut source =
        OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(&policy.launcher).map_err(io_error)?;
    if !source.metadata().map_err(io_error)?.is_file() || source.metadata().map_err(io_error)?.len() > 16 * 1024 * 1024 {
        return Err(io_error("launcher must be a regular executable of at most 16 MiB"));
    }
    let mut output = OpenOptions::new().write(true).create_new(true).open(destination).map_err(io_error)?;
    let mut hash = blake3::Hasher::new();
    let mut bytes = [0; 65536];
    let mut total = 0usize;
    loop {
        let n = source.read(&mut bytes).map_err(io_error)?;
        if n == 0 {
            break;
        }
        total += n;
        if total > 16 * 1024 * 1024 {
            return Err(io_error("launcher grew beyond its limit"));
        }
        hash.update(&bytes[..n]);
        output.write_all(&bytes[..n]).map_err(io_error)?;
    }
    if hash.finalize().as_bytes() != &policy.launcher_digest.0 {
        return Err(io_error("installed launcher differs from the operator-pinned digest"));
    }
    readonly(destination, true)
}

fn target(execution: &ExecutionArtifacts) -> Result<()> {
    let supported =
        [format!("{}-unknown-linux-gnu", std::env::consts::ARCH), format!("{}-unknown-linux-musl", std::env::consts::ARCH)];
    if !supported.contains(&execution.target) {
        return Err(BundleError::new(ErrorCode::UnsupportedPlatform, "replayer target is not this Linux architecture"));
    }
    Ok(())
}

pub(super) fn run(
    directory: &BundleDirectory,
    anchors: &[TrustAnchor],
    inventory: &SessionInventory,
    policy: &SandboxPolicy,
    cancellation: &Cancellation,
) -> Result<(Vec<u8>, RunReport)> {
    target(&inventory.execution)?;
    let start = Instant::now();
    let staging = tempfile::tempdir().map_err(io_error)?;
    let bundle = staging.path().join("bundle");
    fs::create_dir(&bundle).map_err(io_error)?;
    fs::create_dir(bundle.join("blobs")).map_err(io_error)?;
    for blob in &directory.manifest().blobs {
        if cancellation.is_cancelled() {
            return Err(BundleError::new(ErrorCode::Cancelled, "replay cancelled during staging"));
        }
        let path = bundle.join("blobs").join(blob.id.hex());
        let mut output = OpenOptions::new().write(true).create_new(true).open(&path).map_err(io_error)?;
        directory.copy_blob(blob.id, &mut output)?;
        readonly(&path, true)?;
    }
    fs::write(bundle.join("manifest.json"), directory.manifest().encode()?).map_err(io_error)?;
    readonly(&bundle.join("manifest.json"), false)?;
    let run_inputs = staging.path().join("run");
    fs::create_dir(&run_inputs).map_err(io_error)?;
    let trust = serde_json::to_vec(anchors).map_err(io_error)?;
    if trust.len() > 1024 * 1024 {
        return Err(BundleError::new(ErrorCode::Limit, "runner trust file exceeds 1 MiB"));
    }
    fs::write(run_inputs.join("trust.json"), trust).map_err(io_error)?;
    fs::write(run_inputs.join("limits.json"), serde_json::to_vec(directory.limits()).map_err(io_error)?).map_err(io_error)?;
    fs::write(run_inputs.join("session.json"), serde_json::to_vec(&inventory.session_id).map_err(io_error)?).map_err(io_error)?;
    let launcher_path = staging.path().join("launcher");
    launcher(policy, &launcher_path)?;
    let filter_path = staging.path().join("seccomp");
    fs::write(&filter_path, seccomp()).map_err(io_error)?;
    let filter = File::open(filter_path).map_err(io_error)?;
    let filter_fd = filter.as_raw_fd();
    let executable = bundle.join("blobs").join(inventory.execution.replayer.hex());
    let mut header = [0; 20];
    File::open(&executable).map_err(io_error)?.read_exact(&mut header).map_err(io_error)?;
    #[cfg(target_arch = "x86_64")]
    let machine = 62u16;
    #[cfg(target_arch = "aarch64")]
    let machine = 183u16;
    if &header[..6] != b"\x7fELF\x02\x01" || u16::from_le_bytes([header[18], header[19]]) != machine {
        return Err(BundleError::new(ErrorCode::UnsupportedPlatform, "replayer ELF does not match its declared architecture"));
    }
    let mut command = Command::new(&launcher_path);
    command.env_clear().current_dir("/").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    // Require each namespace explicitly. No *-try option can silently weaken
    // filesystem/network/process isolation on a restricted host.
    command.args([
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--disable-userns",
        "--assert-userns-disabled",
        "--new-session",
        "--die-with-parent",
        "--cap-drop",
        "ALL",
        "--clearenv",
    ]);
    command.arg("--ro-bind").arg(&bundle).arg("/bundle");
    command.arg("--ro-bind").arg(&run_inputs).arg("/run");
    command.arg("--ro-bind").arg(&executable).arg("/app/replayer");
    for file in &inventory.execution.runtime.as_ref().expect("checked by caller").files {
        command.arg("--ro-bind").arg(bundle.join("blobs").join(file.blob.hex())).arg(&file.path);
    }
    command
        .args([
            "--proc",
            "/proc",
            "--remount-ro",
            "/proc",
            "--dev",
            "/dev",
            "--remount-ro",
            "/dev",
            "--dir",
            "/tmp",
            "--remount-ro",
            "/",
            "--chdir",
            "/",
            "--seccomp",
        ])
        .arg(filter_fd.to_string())
        .args(["--", "/app/replayer"])
        .arg(BlobId(directory.manifest().commitment()?).hex());
    let cpu = policy.cpu_seconds;
    let memory = policy.address_space_bytes;
    // SAFETY: after fork this performs only async-signal-safe syscalls on fixed
    // data. Mark every inherited descriptor CLOEXEC, retaining only the filter
    // for the trusted launcher (which closes it before application execution).
    unsafe {
        command.pre_exec(move || {
            set_limit(libc::RLIMIT_AS, memory, memory)?;
            set_limit(libc::RLIMIT_CPU, cpu, cpu + 1)?;
            set_limit(libc::RLIMIT_CORE, 0, 0)?;
            set_limit(libc::RLIMIT_FSIZE, 0, 0)?;
            set_limit(libc::RLIMIT_NOFILE, 256, 256)?;
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, libc::CLOSE_RANGE_CLOEXEC) < 0
                || libc::fcntl(filter_fd, libc::F_SETFD, 0) < 0
                || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let preparation_time_us = elapsed(start);
    let result = supervise(command, policy, inventory.execution.replayer, preparation_time_us, cancellation);
    // Private staged artifacts remain alive until the launcher has been reaped.
    drop((filter, staging));
    result
}

fn set_limit(resource: libc::__rlimit_resource_t, soft: u64, hard: u64) -> io::Result<()> {
    let limit = libc::rlimit { rlim_cur: soft as libc::rlim_t, rlim_max: hard as libc::rlim_t };
    // SAFETY: setrlimit reads the initialized stack value.
    if unsafe { libc::setrlimit(resource, &limit) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct ChildGuard {
    child: Child,
    pid: libc::pid_t,
    reaped: bool,
}
impl ChildGuard {
    fn exited(&self) -> io::Result<bool> {
        // SAFETY: waitid writes a valid siginfo for this unreaped child only.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::waitid(libc::P_PID, self.pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { info.si_pid() } != 0)
    }
    fn finish(&mut self) -> io::Result<(ExitStatus, libc::rusage)> {
        // The leader stays unreaped until this point, preventing PID reuse.
        // bubblewrap's die-with-parent and PID namespace tear down its child.
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
        let mut status = 0;
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        loop {
            let result = unsafe { libc::wait4(self.pid, &mut status, 0, &mut usage) };
            if result == self.pid {
                self.reaped = true;
                return Ok((ExitStatus::from_raw(status), usage));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.finish();
        }
    }
}
fn nonblocking(file: &impl AsRawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn drain(input: &mut impl Read, output: &mut Vec<u8>, cap: usize) -> std::result::Result<bool, ErrorCode> {
    let mut bytes = [0; 8192];
    for _ in 0..8 {
        match input.read(&mut bytes) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                if n > cap.saturating_sub(output.len()) {
                    return Err(ErrorCode::OutputLimit);
                }
                output.extend_from_slice(&bytes[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(ErrorCode::Sandbox),
        }
    }
    Ok(false)
}
fn supervise(
    mut command: Command,
    policy: &SandboxPolicy,
    replayer: BlobId,
    preparation_time_us: u64,
    cancellation: &Cancellation,
) -> Result<(Vec<u8>, RunReport)> {
    if cancellation.is_cancelled() {
        return Err(BundleError::new(ErrorCode::Cancelled, "replay cancelled before launch"));
    }
    let start = Instant::now();
    let child = command.spawn().map_err(io_error)?;
    let pid = child.id() as libc::pid_t;
    let mut child = ChildGuard { child, pid, reaped: false };
    let mut stdout = child.child.stdout.take().expect("piped stdout");
    let mut stderr = child.child.stderr.take().expect("piped stderr");
    let setup = || {
        nonblocking(&stdout)?;
        nonblocking(&stderr)
    };
    let mut failure = setup().err().map(|_| ErrorCode::Sandbox);
    let mut output = vec![];
    let mut diagnostics = vec![];
    let (mut out_eof, mut err_eof) = (false, false);
    while failure.is_none() {
        if cancellation.is_cancelled() {
            failure = Some(ErrorCode::Cancelled);
            break;
        }
        if start.elapsed() >= Duration::from_millis(policy.wall_time_ms) {
            failure = Some(ErrorCode::Deadline);
            break;
        }
        let exited = match child.exited() {
            Ok(value) => value,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                failure = Some(ErrorCode::Sandbox);
                break;
            }
        };
        if !out_eof {
            match drain(&mut stdout, &mut output, policy.output_bytes) {
                Ok(eof) => out_eof = eof,
                Err(code) => failure = Some(code),
            }
        }
        if !err_eof && failure.is_none() {
            match drain(&mut stderr, &mut diagnostics, policy.stderr_bytes) {
                Ok(eof) => err_eof = eof,
                Err(code) => failure = Some(code),
            }
        }
        if failure.is_some() || (exited && out_eof && err_eof) {
            break;
        }
        let mut fds = [
            libc::pollfd { fd: if out_eof { -1 } else { stdout.as_raw_fd() }, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: if err_eof { -1 } else { stderr.as_raw_fd() }, events: libc::POLLIN, revents: 0 },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, 5) } < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            failure = Some(ErrorCode::Sandbox);
        }
    }
    drop((stdout, stderr));
    let (status, usage) = child.finish().map_err(io_error)?;
    if failure.is_none() && !status.success() {
        failure = Some(if status.signal() == Some(libc::SIGXCPU) || status.code() == Some(128 + libc::SIGXCPU) {
            ErrorCode::CpuLimit
        } else {
            ErrorCode::Sandbox
        });
    }
    let micros = |t: libc::timeval| (t.tv_sec.max(0) as u64).saturating_mul(1_000_000).saturating_add(t.tv_usec.max(0) as u64);
    let report = RunReport {
        launcher_digest: policy.launcher_digest,
        replayer_digest: replayer,
        policy: policy.clone(),
        preparation_time_us,
        wall_time_us: elapsed(start),
        user_cpu_us: micros(usage.ru_utime),
        system_cpu_us: micros(usage.ru_stime),
        lifetime_peak_rss_bytes: (usage.ru_maxrss.max(0) as u64).saturating_mul(1024),
        exit_code: status.code(),
        signal: status.signal(),
        failure,
        stderr: String::from_utf8_lossy(&diagnostics).into_owned(),
    };
    if let Some(code) = failure {
        let mut error = BundleError::new(code, format!("isolated replayer exited {:?}, signal {:?}", report.exit_code, report.signal));
        error.execution = Some(Box::new(report));
        return Err(error);
    }
    Ok((output, report))
}

fn seccomp() -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7;
    const LD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    const EQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    const AND: u16 = (libc::BPF_ALU | libc::BPF_AND | libc::BPF_K) as u16;
    const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
    let mut bytes = vec![];
    let mut instruction = |code: u16, jt: u8, jf: u8, k: u32| {
        bytes.extend(code.to_ne_bytes());
        bytes.extend([jt, jf]);
        bytes.extend(k.to_ne_bytes());
    };
    instruction(LD, 0, 0, 4);
    instruction(EQ, 1, 0, ARCH);
    instruction(RET, 0, 0, libc::SECCOMP_RET_KILL_PROCESS);
    instruction(LD, 0, 0, 0);
    // Minimal single-process, read-only native runtime surface. All networking,
    // descendants, namespaces, IPC/keyrings, ptrace and io_uring are denied.
    let allowed = [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_pread64,
        libc::SYS_close,
        libc::SYS_lseek,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_statx,
        libc::SYS_openat,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_readlinkat,
        libc::SYS_getdents64,
        libc::SYS_ppoll,
        libc::SYS_pselect6,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_brk,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_set_tid_address,
        libc::SYS_set_robust_list,
        libc::SYS_rseq,
        libc::SYS_prlimit64,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        libc::SYS_futex,
        libc::SYS_futex_waitv,
        libc::SYS_getrandom,
        libc::SYS_getpid,
        libc::SYS_getppid,
        libc::SYS_gettid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_uname,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_yield,
        libc::SYS_getrusage,
        // bubblewrap's namespace init must reap the one application child.
        libc::SYS_wait4,
        libc::SYS_waitid,
        libc::SYS_statfs,
        libc::SYS_fstatfs,
        libc::SYS_tgkill,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_execve,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_arch_prctl,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_open,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_access,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_readlink,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_poll,
        #[cfg(target_arch = "x86_64")]
        libc::SYS_getrlimit,
    ];
    for syscall in allowed {
        instruction(EQ, 0, 1, syscall as u32);
        instruction(RET, 0, 0, libc::SECCOMP_RET_ALLOW);
    }
    // clone3 passes flags through a pointer, which classic BPF cannot inspect.
    // Report it unavailable so libc uses the argument-filtered clone below.
    instruction(EQ, 0, 1, libc::SYS_clone3 as u32);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32);
    // Archive continuations need threads, never independent descendants. Require
    // one thread group, address space, descriptor table and filesystem context;
    // reject namespace, tracing, parent, signal and every unrecognized flag.
    // Filters and process resource limits are inherited by these threads.
    let required = (libc::CLONE_VM | libc::CLONE_FS | libc::CLONE_FILES | libc::CLONE_SIGHAND | libc::CLONE_THREAD) as u32;
    let permitted = required
        | (libc::CLONE_SYSVSEM
            | libc::CLONE_SETTLS
            | libc::CLONE_PARENT_SETTID
            | libc::CLONE_CHILD_CLEARTID
            | libc::CLONE_CHILD_SETTID) as u32;
    instruction(EQ, 0, 12, libc::SYS_clone as u32);
    instruction(LD, 0, 0, 20); // args[0] high word: supported hosts are little endian
    instruction(EQ, 1, 0, 0);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
    instruction(LD, 0, 0, 16); // args[0] low word
    instruction(AND, 0, 0, !permitted);
    instruction(EQ, 1, 0, 0);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
    instruction(LD, 0, 0, 16);
    instruction(AND, 0, 0, required);
    instruction(EQ, 1, 0, required);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ALLOW);
    // Readers may take only nonblocking shared locks on their private archive.
    instruction(EQ, 0, 4, libc::SYS_flock as u32);
    instruction(LD, 0, 0, 24); // args[1], the flock operation
    instruction(EQ, 0, 1, (libc::LOCK_SH | libc::LOCK_NB) as u32);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ALLOW);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
    // Permit descriptor flags and bounded duplication, not fcntl locks or
    // owner-directed asynchronous signals on another process's descriptors.
    instruction(EQ, 1, 0, libc::SYS_fcntl as u32);
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
    instruction(LD, 0, 0, 24); // args[1], the fcntl command
    for command in [libc::F_GETFD, libc::F_SETFD, libc::F_GETFL, libc::F_SETFL, libc::F_DUPFD, libc::F_DUPFD_CLOEXEC] {
        instruction(EQ, 0, 1, command as u32);
        instruction(RET, 0, 0, libc::SECCOMP_RET_ALLOW);
    }
    instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32);
    bytes
}
