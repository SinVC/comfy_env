use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::ProcessIdentity;

#[cfg(target_os = "linux")]
fn process_stat(pid: u32) -> io::Result<Option<(bool, u32, String)>> {
    let stat = match std::fs::read(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ESRCH) =>
        {
            return Ok(None);
        }
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

#[cfg(target_os = "linux")]
mod production {
    use super::*;
    use anyhow::{Result, ensure};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::atomic::AtomicI32;

    static STOP_SIGNAL: AtomicI32 = AtomicI32::new(0);
    const GRACE: Duration = Duration::from_secs(30);
    const FORCE: Duration = Duration::from_secs(5);
    const POLL: Duration = Duration::from_millis(25);

    pub fn install_supervisor_signals() -> io::Result<()> {
        extern "C" fn request_stop(signal: libc::c_int) {
            let _ = STOP_SIGNAL.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
        }
        for signal in [libc::SIGTERM, libc::SIGINT] {
            if unsafe { libc::signal(signal, request_stop as *const () as libc::sighandler_t) }
                == libc::SIG_ERR
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub fn supervise_production(command: &mut Command) -> Result<i32> {
        let group = std::process::id();
        ensure!(
            unsafe { libc::getpgrp() } == group as libc::pid_t,
            "production launcher is not its process group leader"
        );
        let signal = STOP_SIGNAL.load(Ordering::Relaxed);
        if signal != 0 {
            return Ok(128 + signal);
        }
        let mut child = command.spawn()?;
        let (status, signal) = loop {
            let status = child.try_wait()?;
            let signal = STOP_SIGNAL.load(Ordering::Relaxed);
            if status.is_some() || signal != 0 {
                break (status, signal);
            }
            std::thread::sleep(POLL);
        };
        if !group_members(group)?.is_empty() {
            let signal = if signal == 0 { libc::SIGTERM } else { signal };
            // The supervisor keeps the group ID reserved and handles this signal itself.
            send_signal(-(group as libc::pid_t), signal)?;
            let deadline = Instant::now() + GRACE;
            while Instant::now() < deadline && !group_members(group)?.is_empty() {
                std::thread::sleep(POLL);
            }
            let deadline = Instant::now() + FORCE;
            loop {
                let members = group_members(group)?;
                if members.is_empty() {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "production process group did not exit"
                );
                for pid in members {
                    signal_verified(pid, libc::SIGKILL, || {
                        Ok(process_stat(pid)?
                            .is_some_and(|(alive, pgid, _)| alive && pgid == group))
                    })?;
                }
                std::thread::sleep(POLL);
            }
        }
        Ok(exit_code(match status {
            Some(status) => status,
            None => child.wait()?,
        }))
    }

    pub fn production_group_alive(identity: &ProcessIdentity) -> Result<bool> {
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        if identity.creation.split_once(':').map(|(id, _)| id) != Some(boot.trim()) {
            return Ok(false);
        }
        if let Some((alive, pgid, started)) = process_stat(identity.pid)? {
            if identity.creation != format!("{}:{started}", boot.trim()) {
                return Ok(false);
            }
            if alive {
                ensure!(
                    pgid == identity.pid,
                    "recorded supervisor is not a process group leader"
                );
                return Ok(true);
            }
        }
        Ok(!group_members(identity.pid)?.is_empty())
    }

    pub fn interrupt_supervisor(identity: &ProcessIdentity) -> io::Result<()> {
        signal_supervisor(identity, libc::SIGINT).map(|_| ())
    }

    pub fn stop_supervised(identity: &ProcessIdentity, grace_seconds: u32) -> Result<()> {
        if signal_supervisor(identity, libc::SIGTERM)? {
            let deadline =
                Instant::now() + Duration::from_secs(grace_seconds.into()) + FORCE + POLL * 4;
            while Instant::now() < deadline && production_group_alive(identity)? {
                std::thread::sleep(POLL);
            }
        }
        ensure!(
            !production_group_alive(identity)?,
            "production process group is still active; verify the recorded supervisor and its group"
        );
        Ok(())
    }

    fn signal_supervisor(identity: &ProcessIdentity, signal: libc::c_int) -> io::Result<bool> {
        signal_verified(identity.pid, signal, || {
            Ok(
                process_creation(identity.pid)?.as_ref() == Some(&identity.creation)
                    && process_stat(identity.pid)?.is_some_and(|(_, pgid, _)| pgid == identity.pid),
            )
        })
    }

    fn signal_verified(
        pid: u32,
        signal: libc::c_int,
        verify: impl FnOnce() -> io::Result<bool>,
    ) -> io::Result<bool> {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return missing_or_error();
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd as _) };
        if !verify()? {
            return Ok(false);
        }
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
            return missing_or_error();
        }
        Ok(true)
    }

    fn missing_or_error() -> io::Result<bool> {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        Err(error)
    }

    fn group_members(group: u32) -> io::Result<Vec<u32>> {
        let mut members = Vec::new();
        for entry in std::fs::read_dir("/proc")? {
            if let Ok(pid) = entry?.file_name().to_string_lossy().parse()
                && pid != group
                && process_stat(pid)?.is_some_and(|(alive, pgid, _)| alive && pgid == group)
            {
                members.push(pid);
            }
        }
        Ok(members)
    }

    #[cfg(test)]
    #[test]
    fn stale_supervisor_identity_is_not_signaled() {
        let mut identity = ProcessIdentity::capture(std::process::id()).unwrap();
        identity.creation.push('0');
        assert!(!production_group_alive(&identity).unwrap());
        assert!(!signal_supervisor(&identity, libc::SIGTERM).unwrap());
    }
}

#[cfg(target_os = "linux")]
pub use production::{
    install_supervisor_signals, interrupt_supervisor, production_group_alive, stop_supervised,
    supervise_production,
};

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
