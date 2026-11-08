use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_os = "linux"))]
use std::time::{Duration, Instant};

use super::ProcessIdentity;

#[cfg(target_os = "linux")]
fn process_stat(pid: u32) -> io::Result<Option<(bool, u32, String)>> {
    let stat = match std::fs::read(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid /proc process stat");
    let end = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(invalid)?;
    let fields = std::str::from_utf8(&stat[end + 1..])
        .map_err(|_| invalid())?
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    let state = fields.first().ok_or_else(invalid)?;
    let group = fields
        .get(2)
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    let start = fields.get(19).ok_or_else(invalid)?;
    Ok(Some((
        !matches!(*state, "Z" | "X"),
        group,
        (*start).to_owned(),
    )))
}

pub fn process_creation(pid: u32) -> io::Result<Option<String>> {
    #[cfg(target_os = "linux")]
    {
        let Some((_, _, start)) = process_stat(pid)? else {
            return Ok(None);
        };
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        Ok(Some(format!("{}:{start}", boot.trim())))
    }
    #[cfg(not(target_os = "linux"))]
    {
        if check_process_alive(pid)? {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "process identity unavailable",
            ));
        }
        Ok(None)
    }
}

pub fn check_process_alive(pid: u32) -> io::Result<bool> {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return Ok(false);
    };
    if pid <= 0 {
        return Ok(false);
    }
    // kill(pid, 0) also succeeds for zombies, which have already stopped.
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read(format!("/proc/{pid}/stat"))
        && let Some(state) = stat
            .rsplit(|&byte| byte == b')')
            .next()
            .and_then(|tail| tail.get(1))
    {
        return Ok(!matches!(state, b'Z' | b'X'));
    }
    match send_signal(pid, 0) {
        Err(error) if error.raw_os_error() == Some(libc::EPERM) => Ok(true),
        result => result,
    }
}

pub fn terminate_process(identity: &ProcessIdentity, grace_seconds: u32) -> io::Result<()> {
    let pid = identity.pid;
    if !check_process_alive(pid)? || process_creation(pid)?.as_ref() != Some(&identity.creation) {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(error);
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd as _) };
        if process_creation(pid)?.as_ref() != Some(&identity.creation) {
            return Ok(());
        }
        let signal = |signal: libc::c_int| {
            if unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    signal,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            } < 0
            {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
            }
            Ok(())
        };
        let wait = |seconds: u32| {
            let mut poll = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            match unsafe {
                libc::poll(
                    &mut poll,
                    1,
                    seconds.saturating_mul(1000).min(i32::MAX as u32) as i32,
                )
            } {
                -1 => Err(io::Error::last_os_error()),
                0 => Ok(false),
                _ => Ok(true),
            }
        };
        signal(libc::SIGTERM)?;
        if wait(grace_seconds)? {
            return Ok(());
        }
        signal(libc::SIGKILL)?;
        if wait(5)? {
            return Ok(());
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let native_pid = pid as libc::pid_t;
        if !send_signal(native_pid, libc::SIGTERM)? || wait_for_exit(pid, grace_seconds)? {
            return Ok(());
        }
        if !send_signal(native_pid, libc::SIGKILL)? || wait_for_exit(pid, 5)? {
            return Ok(());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "process did not exit after SIGKILL",
    ))
}

fn send_signal(pid: libc::pid_t, signal: libc::c_int) -> io::Result<bool> {
    // pid is positive, so kill targets a single process.
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    Err(error)
}

#[cfg(not(target_os = "linux"))]
fn wait_for_exit(pid: u32, seconds: u32) -> io::Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(seconds.into());
    while check_process_alive(pid)? {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
    Ok(true)
}

pub struct CandidateProcess {
    pub child: Child,
}

impl CandidateProcess {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        command.process_group(0).spawn().map(|child| Self { child })
    }

    pub fn has_exited(&mut self) -> io::Result<bool> {
        // Keep the leader PID reserved until the group has been terminated.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { info.si_pid() } != 0)
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        send_signal(-(self.child.id() as libc::pid_t), libc::SIGKILL).map(|_| ())
    }
}

pub fn candidate_is_alive(identity: &ProcessIdentity) -> anyhow::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        if identity.creation.split_once(':').map(|(id, _)| id) != Some(boot.trim()) {
            return Ok(false);
        }
        for entry in std::fs::read_dir("/proc")? {
            let entry = entry?;
            if let Ok(pid) = entry.file_name().to_string_lossy().parse()
                && let Some((true, group, _)) = process_stat(pid)?
                && group == identity.pid
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    #[cfg(not(target_os = "linux"))]
    identity.is_alive()
}

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

pub struct RunInterrupts(libc::sighandler_t);

impl RunInterrupts {
    pub fn install() -> io::Result<Self> {
        // The foreground child receives terminal SIGINT too. Keep gov alive to
        // reap it and remove its PID file. Exec resets the child's handler.
        extern "C" fn handle_interrupt(_: libc::c_int) {
            INTERRUPTED.store(true, Ordering::Relaxed);
        }
        INTERRUPTED.store(false, Ordering::Relaxed);
        let previous = unsafe {
            libc::signal(
                libc::SIGINT,
                handle_interrupt as *const () as libc::sighandler_t,
            )
        };
        if previous == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(previous))
    }

    pub fn set_child(&self, _pid: u32) {}

    pub fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::Relaxed)
    }
}

impl Drop for RunInterrupts {
    fn drop(&mut self) {
        unsafe { libc::signal(libc::SIGINT, self.0) };
    }
}
