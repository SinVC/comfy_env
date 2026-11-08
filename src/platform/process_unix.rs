use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

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

pub fn terminate_process(pid: u32, grace_seconds: u32) -> io::Result<()> {
    if !check_process_alive(pid)? {
        return Ok(());
    }
    let native_pid = pid as libc::pid_t;
    if !send_signal(native_pid, libc::SIGTERM)? || wait_for_exit(pid, grace_seconds)? {
        return Ok(());
    }
    if !send_signal(native_pid, libc::SIGKILL)? || wait_for_exit(pid, 5)? {
        return Ok(());
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

pub fn configure_prod_process(_command: &mut Command) {}

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
        extern "C" fn handle_interrupt(_: libc::c_int) {}
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
}

impl Drop for RunInterrupts {
    fn drop(&mut self) {
        unsafe { libc::signal(libc::SIGINT, self.0) };
    }
}
