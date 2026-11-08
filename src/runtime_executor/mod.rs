#![allow(dead_code)]

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

use anyhow::{Context, Result, bail};

use crate::domain::CmdResult;
use crate::{fs_support, platform};

pub struct PythonClient;

#[derive(Debug, Clone, Copy)]
pub enum RunOutcome {
    Passed,
    Failed(i32),
    TimedOut,
}

pub enum StopOutcome {
    Terminated,
    AlreadyStopped,
    NotRunning,
}

pub fn run_prod(
    env: &Path,
    comfyui_dir: &Path,
    args: &[OsString],
    pid_path: &Path,
    before_launch: impl FnOnce() -> Result<()>,
) -> Result<i32> {
    let guard = lock_pid_file(pid_path)?;
    if let Some(pid) = read_pid(pid_path)?
        && platform::check_process_alive(pid).context("failed to check ComfyUI liveness")?
    {
        bail!("ComfyUI is already running (PID {pid})");
    }
    fs_support::remove_file_if_exists(pid_path)?;
    before_launch()?;

    let python = platform::venv_python(env);
    let mut command = Command::new(&python);
    command
        .arg(comfyui_dir.join("main.py"))
        .args(args)
        .current_dir(comfyui_dir);
    platform::configure_prod_process(&mut command);
    let interrupts = platform::RunInterrupts::install()?;
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", python.display()))?;
    let pid = child.id();
    interrupts.set_child(pid);
    if let Err(error) = fs_support::atomic_write(pid_path, format!("{pid}\n").as_bytes()) {
        if let Err(cleanup) = child.kill().and_then(|()| child.wait()) {
            return Err(error.context(format!("failed to clean up child {pid}: {cleanup}")));
        }
        return Err(error);
    }
    drop(guard);

    let status = child.wait().context("failed to wait for ComfyUI")?;
    drop(interrupts);
    let _guard = lock_pid_file(pid_path)?;
    if read_pid(pid_path)? == Some(pid) {
        fs_support::remove_file_if_exists(pid_path)?;
    }
    Ok(platform::exit_code(status))
}

pub fn stop(pid_path: &Path, grace_seconds: u32) -> Result<StopOutcome> {
    if !pid_path.try_exists()? {
        return Ok(StopOutcome::NotRunning);
    }
    let _guard = lock_pid_file(pid_path)?;
    let Some(text) = fs_support::read_text_if_exists(pid_path)? else {
        return Ok(StopOutcome::NotRunning);
    };
    if let Some(pid) = parse_pid(&text)
        && platform::check_process_alive(pid).context("failed to check ComfyUI liveness")?
    {
        platform::terminate_process(pid, grace_seconds)
            .with_context(|| format!("failed to stop ComfyUI (PID {pid})"))?;
        fs_support::remove_file_if_exists(pid_path)?;
        return Ok(StopOutcome::Terminated);
    }
    fs_support::remove_file_if_exists(pid_path)?;
    Ok(StopOutcome::AlreadyStopped)
}

pub fn read_pid(path: &Path) -> Result<Option<u32>> {
    Ok(fs_support::read_text_if_exists(path)?
        .as_deref()
        .and_then(parse_pid))
}

fn parse_pid(text: &str) -> Option<u32> {
    text.trim().parse().ok().filter(|&pid| pid != 0)
}

fn lock_pid_file(pid_path: &Path) -> Result<File> {
    // Lock a separate file whose identity survives PID file replacement.
    let lock_path = pid_path.with_extension("pid.lock");
    if let Some(dir) = lock_path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;
    Ok(file)
}

pub fn run_candidate(
    env: &Path,
    comfyui_dir: &Path,
    timeout_seconds: u32,
    stdout: &Path,
    stderr: &Path,
) -> Result<RunOutcome> {
    let python = crate::platform::venv_python(env);
    let mut child = Command::new(&python)
        .arg(comfyui_dir.join("main.py"))
        .current_dir(comfyui_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {}", python.display()))?;
    let out = child.stdout.take().context("missing child stdout")?;
    let err = child.stderr.take().context("missing child stderr")?;
    let out_path = stdout.to_owned();
    let err_path = stderr.to_owned();
    let out_thread = std::thread::spawn(move || copy_stream(out, &out_path, false));
    let err_thread = std::thread::spawn(move || copy_stream(err, &err_path, true));
    let status = child.wait_timeout(Duration::from_secs(timeout_seconds.into()))?;
    let outcome = match status {
        Some(status) if status.success() => RunOutcome::Passed,
        Some(status) => RunOutcome::Failed(status.code().unwrap_or(-1)),
        None => {
            child.kill()?;
            child.wait()?;
            RunOutcome::TimedOut
        }
    };
    out_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stdout relay panicked"))??;
    err_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stderr relay panicked"))??;
    Ok(outcome)
}

fn copy_stream(mut source: impl Read, path: &Path, error: bool) -> Result<()> {
    let mut file = std::fs::File::create(path)?;
    let mut buffer = [0u8; 4096];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])?;
        if error {
            std::io::stderr().write_all(&buffer[..count])?;
        } else {
            std::io::stdout().write_all(&buffer[..count])?;
        }
    }
    file.sync_all()?;
    Ok(())
}

impl PythonClient {
    pub fn version(python: &Path) -> Result<CmdResult> {
        let output = Command::new(python)
            .arg("--version")
            .output()
            .with_context(|| format!("failed to run `{}`", python.display()))?;

        Ok(CmdResult::from_output(output))
    }
}
