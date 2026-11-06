use crate::{Cancellation, FailureKind, OperationalLimits, RunMeasurements, RunReport, WorkerError, WorkerProgram};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::OpenOptionsExt,
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::Instant,
};

pub(crate) struct Executable(File);

impl Executable {
    pub(crate) fn load(path: &Path, expected: [u8; 32]) -> Result<Self, WorkerError> {
        let load = || -> io::Result<Self> {
            let mut input = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
            if !input.metadata()?.is_file() || input.metadata()?.len() > 256 * 1024 * 1024 {
                return Err(io::Error::other("worker executable must be a regular file of at most 256 MiB"));
            }
            // SAFETY: the literal is NUL-terminated, and the returned descriptor
            // is owned exactly once by File. The memfd remains pinned until exec.
            let fd =
                unsafe { libc::memfd_create(c"myelin-worker".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING | libc::MFD_EXEC) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let mut hash = blake3::Hasher::new();
            let mut buffer = [0u8; 65536];
            let mut total = 0usize;
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                if total == 0 && (count < 4 || &buffer[..4] != b"\x7fELF") {
                    return Err(io::Error::other("worker must be an ELF executable"));
                }
                total = total.checked_add(count).ok_or_else(|| io::Error::other("executable size overflow"))?;
                if total > 256 * 1024 * 1024 {
                    return Err(io::Error::other("worker executable grew beyond its bound"));
                }
                hash.update(&buffer[..count]);
                file.write_all(&buffer[..count])?;
            }
            if total == 0 || hash.finalize().as_bytes() != &expected {
                return Err(io::Error::other("worker executable digest mismatch"));
            }
            // SAFETY: fcntl operates on our live descriptor. All content/size
            // changes and future seal changes are prohibited before sharing it.
            let result = unsafe {
                libc::fcntl(fd, libc::F_ADD_SEALS, libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL)
            };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(file))
        };
        load().map_err(|e| WorkerError::new(FailureKind::Artifact, e.to_string()))
    }
}

// Keep the leader unreaped while terminating the group, so its PID cannot be
// reused between observation and killpg. No Child::wait/try_wait is used.
struct ChildGuard {
    child: Child,
    pid: libc::pid_t,
    reaped: bool,
}
impl ChildGuard {
    fn exited(&self) -> io::Result<bool> {
        // SAFETY: initialized siginfo is writable and waitid targets only our
        // child. WNOWAIT leaves ownership of reaping and rusage to finish().
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result =
            unsafe { libc::waitid(libc::P_PID, self.pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { info.si_pid() } != 0)
    }
    fn finish(&mut self) -> io::Result<(ExitStatus, libc::rusage)> {
        // SAFETY: process_group(0) created a group whose leader remains our
        // unreaped child. A negative pid addresses that group, never all processes.
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

fn nonblocking(fd: &impl AsRawFd) -> io::Result<()> {
    // SAFETY: only the descriptor's flags change; the owning pipe stays alive.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn pump(reader: &mut impl Read, target: &mut Vec<u8>, cap: usize) -> Result<bool, FailureKind> {
    let mut chunk = [0u8; 8192];
    // Bound work per iteration so a continuously writing worker cannot starve
    // cancellation/deadline checks. Never allocate an attacker-declared length.
    for _ in 0..8 {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if count > cap.saturating_sub(target.len()) {
                    return Err(FailureKind::OutputLimit);
                }
                target.extend_from_slice(&chunk[..count]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(FailureKind::Transport),
        }
    }
    Ok(false)
}

pub(crate) fn run(
    program: &WorkerProgram,
    packet: &[u8],
    limits: &OperationalLimits,
    cancellation: &Cancellation,
) -> Result<(Vec<u8>, RunReport), WorkerError> {
    if !cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        return Err(WorkerError::new(FailureKind::UnsupportedPlatform, "single-process filter is supported on Linux x86_64/aarch64"));
    }
    let start = Instant::now();
    let mut command = Command::new(format!("/proc/self/fd/{}", program.executable.0.as_raw_fd()));
    command
        .args(program.args.as_ref())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let address_space = limits.address_space_bytes;
    let cpu = limits.cpu_seconds;
    // SAFETY: the closure runs after fork and performs only fixed-size stack
    // construction and async-signal-safe kernel calls; no locks or allocation.
    unsafe {
        command.pre_exec(move || {
            set_limit(libc::RLIMIT_AS, address_space, address_space)?;
            set_limit(libc::RLIMIT_CPU, cpu, cpu + 1)?;
            set_limit(libc::RLIMIT_CORE, 0, 0)?;
            single_process_filter()
        });
    }
    let child = command.spawn().map_err(|e| WorkerError::new(FailureKind::Spawn, e.to_string()))?;
    let pid = child.id() as libc::pid_t;
    let mut child = ChildGuard { child, pid, reaped: false };
    let mut stdin = child.child.stdin.take();
    let mut stdout = child.child.stdout.take().expect("piped stdout");
    let mut stderr = child.child.stderr.take().expect("piped stderr");
    let setup = || {
        nonblocking(stdin.as_ref().expect("piped stdin"))?;
        nonblocking(&stdout)?;
        nonblocking(&stderr)
    };
    let mut failure = setup().err().map(|_| FailureKind::Transport);
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let mut rss_peak: Option<u64> = None;
    let mut rss_samples = 0u64;
    let mut position = 0usize;
    let mut out_eof = false;
    let mut err_eof = false;
    while failure.is_none() {
        if cancellation.is_cancelled() {
            failure = Some(FailureKind::Cancelled);
            break;
        }
        if start.elapsed() >= limits.wall_time {
            failure = Some(FailureKind::Deadline);
            break;
        }
        let exited = match child.exited() {
            Ok(exited) => exited,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                failure = Some(FailureKind::Transport);
                break;
            }
        };
        if exited {
            stdin = None;
        } else if let Some(rss) = sample_rss(pid) {
            rss_peak = Some(rss_peak.unwrap_or(0).max(rss));
            rss_samples += 1;
        }
        if let Some(pipe) = &mut stdin {
            let end = packet.len().min(position.saturating_add(65536));
            match pipe.write(&packet[position..end]) {
                Ok(count) => {
                    position += count;
                    if position == packet.len() {
                        stdin = None;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => stdin = None,
                Err(_) => {
                    failure = Some(FailureKind::Transport);
                    break;
                }
            }
        }
        if !out_eof {
            match pump(&mut stdout, &mut output, limits.output_bytes) {
                Ok(eof) => out_eof = eof,
                Err(kind) => failure = Some(kind),
            }
        }
        if !err_eof && failure.is_none() {
            match pump(&mut stderr, &mut diagnostics, limits.stderr_bytes) {
                Ok(eof) => err_eof = eof,
                Err(kind) => failure = Some(kind),
            }
        }
        if failure.is_some() || (exited && out_eof && err_eof) {
            break;
        }
        let mut fds = [
            libc::pollfd { fd: stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd), events: libc::POLLOUT, revents: 0 },
            libc::pollfd { fd: if out_eof { -1 } else { stdout.as_raw_fd() }, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: if err_eof { -1 } else { stderr.as_raw_fd() }, events: libc::POLLIN, revents: 0 },
        ];
        let timeout = limits.wall_time.saturating_sub(start.elapsed()).as_millis().min(5) as i32;
        // SAFETY: poll borrows the live, fixed-size descriptor array.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) } < 0
            && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
        {
            failure = Some(FailureKind::Transport);
        }
    }
    drop(stdin);
    drop(stdout);
    drop(stderr);
    let (status, usage) = child.finish().map_err(|e| WorkerError::new(FailureKind::Transport, e.to_string()))?;
    if failure.is_none() && !status.success() {
        failure = Some(if status.signal() == Some(libc::SIGXCPU) { FailureKind::CpuLimit } else { FailureKind::ProcessExit });
    }
    let report = RunReport {
        artifact_hash: program.digest,
        request_hash: *blake3::hash(packet).as_bytes(),
        limits: limits.clone(),
        measurements: RunMeasurements {
            pid: pid as u32,
            wall_time_us: start.elapsed().as_micros().min(u64::MAX as u128) as u64,
            user_cpu_us: micros(usage.ru_utime),
            system_cpu_us: micros(usage.ru_stime),
            lifetime_peak_rss_bytes: (usage.ru_maxrss.max(0) as u64).saturating_mul(1024),
            observed_post_exec_peak_rss_bytes: rss_peak,
            rss_samples,
        },
        exit_code: status.code(),
        signal: status.signal(),
        failure,
        stderr: diagnostics,
    };
    if let Some(kind) = failure {
        return Err(WorkerError {
            kind,
            message: format!("process exited {:?}, signal {:?}", report.exit_code, report.signal),
            report: Some(Box::new(report)),
        });
    }
    Ok((output, report))
}

fn micros(time: libc::timeval) -> u64 {
    (time.tv_sec.max(0) as u64).saturating_mul(1_000_000).saturating_add(time.tv_usec.max(0) as u64)
}

fn set_limit(resource: libc::__rlimit_resource_t, soft: u64, hard: u64) -> io::Result<()> {
    let limit = libc::rlimit { rlim_cur: soft as libc::rlim_t, rlim_max: hard as libc::rlim_t };
    // SAFETY: setrlimit reads our fixed-size initialized value.
    if unsafe { libc::setrlimit(resource, &limit) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn single_process_filter() -> io::Result<()> {
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc00000b7;
    const fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }
    const LOAD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    const EQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
    let mut filter = [
        instruction(LOAD, 0, 0, 4), // seccomp_data.arch
        instruction(EQ, 1, 0, ARCH),
        instruction(RET, 0, 0, libc::SECCOMP_RET_KILL_PROCESS),
        instruction(LOAD, 0, 0, 0), // seccomp_data.nr
        instruction((libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K) as u16, 0, 1, 0x40000000),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32), // reject x32 ABI
        instruction(EQ, 0, 1, libc::SYS_clone as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        instruction(EQ, 0, 1, libc::SYS_clone3 as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        #[cfg(target_arch = "x86_64")]
        instruction(EQ, 0, 1, libc::SYS_fork as u32),
        #[cfg(target_arch = "x86_64")]
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        #[cfg(target_arch = "x86_64")]
        instruction(EQ, 0, 1, libc::SYS_vfork as u32),
        #[cfg(target_arch = "x86_64")]
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        instruction(EQ, 0, 1, libc::SYS_setsid as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        instruction(EQ, 0, 1, libc::SYS_setpgid as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        instruction(EQ, 0, 1, libc::SYS_unshare as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        instruction(EQ, 0, 1, libc::SYS_setns as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ERRNO | libc::EPERM as u32),
        instruction(RET, 0, 0, libc::SECCOMP_RET_ALLOW),
    ];
    let descriptor = libc::sock_fprog { len: filter.len() as u16, filter: filter.as_mut_ptr() };
    // SAFETY: prctl reads the initialized BPF array synchronously. no_new_privs
    // makes the filter unprivileged and prevents privilege gain across exec.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } < 0
        || unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &descriptor) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn single_process_filter() -> io::Result<()> {
    Err(io::Error::from_raw_os_error(libc::ENOTSUP))
}

fn sample_rss(pid: libc::pid_t) -> Option<u64> {
    let mut status = String::new();
    File::open(format!("/proc/{pid}/status")).ok()?.take(65536).read_to_string(&mut status).ok()?;
    let value = status.lines().find_map(|line| line.strip_prefix("VmHWM:"))?;
    value.split_whitespace().next()?.parse::<u64>().ok()?.checked_mul(1024)
}
