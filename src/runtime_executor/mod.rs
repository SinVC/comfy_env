use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::platform::ProcessIdentity;
use crate::{fs_support, platform};

#[derive(Debug, Clone, Copy)]
pub enum RunOutcome {
    Passed,
    Failed(i32),
    TimedOut,
    Interrupted,
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
    ensure_prod_stopped(pid_path)?;
    fs_support::remove_file_if_exists(pid_path)?;
    before_launch()?;

    let python = platform::venv_python(env);
    #[cfg(unix)]
    let mut command = Command::new(&python);
    #[cfg(windows)]
    let gate_dir = tempfile::tempdir_in(pid_path.parent().context("PID file has no parent")?)?;
    #[cfg(windows)]
    let gate = gate_dir.path().join("ready");
    #[cfg(windows)]
    let mut command = {
        let owner = ProcessIdentity::capture(std::process::id())?;
        let mut launcher = Command::new(std::env::current_exe()?);
        launcher
            .arg(PROD_LAUNCHER)
            .arg(&gate)
            .arg(serde_json::to_string(&owner)?)
            .arg(&python);
        launcher
    };
    command
        .arg(comfyui_dir.join("main.py"))
        .args(args)
        .current_dir(comfyui_dir);
    let interrupts = platform::RunInterrupts::install()?;
    #[cfg(unix)]
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", python.display()))?;
    #[cfg(windows)]
    let mut process = platform::CandidateProcess::spawn(&mut command)
        .with_context(|| format!("failed to start {}", python.display()))?;
    #[cfg(windows)]
    let child = &mut process.child;
    let pid = child.id();
    interrupts.set_child(pid);
    let record = (|| -> Result<ProcessIdentity> {
        let identity = ProcessIdentity::capture(pid)?;
        fs_support::atomic_write(pid_path, &serde_json::to_vec(&identity)?)?;
        #[cfg(windows)]
        if let Err(error) = fs_support::atomic_write(&gate, &[1]) {
            fs_support::remove_file_if_exists(pid_path)?;
            return Err(error);
        }
        Ok(identity)
    })();
    let identity = match record {
        Ok(identity) => identity,
        Err(error) => {
            let _ = child.kill();
            child
                .wait()
                .context("failed to reap unrecorded ComfyUI process")?;
            return Err(error);
        }
    };
    drop(guard);

    let status = child.wait().context("failed to wait for ComfyUI")?;
    #[cfg(windows)]
    process
        .terminate()
        .context("failed to close the ComfyUI process tree")?;
    drop(interrupts);
    let _guard = lock_pid_file(pid_path)?;
    if matches!(read_record(pid_path)?, Some(PidRecord::Verified(record)) if record == identity) {
        fs_support::remove_file_if_exists(pid_path)?;
    }
    Ok(platform::exit_code(status))
}

pub fn stop(pid_path: &Path, grace_seconds: u32) -> Result<StopOutcome> {
    if !pid_path.try_exists()? {
        return Ok(StopOutcome::NotRunning);
    }
    let _guard = lock_pid_file(pid_path)?;
    let Some(record) = read_record(pid_path)? else {
        return Ok(StopOutcome::NotRunning);
    };
    let outcome = if record.is_alive()? {
        let PidRecord::Verified(identity) = record else {
            bail!(
                "legacy PID is still alive but its identity is unverified; verify and stop it manually"
            );
        };
        platform::terminate_process(&identity, grace_seconds)
            .with_context(|| format!("failed to stop ComfyUI (PID {})", identity.pid))?;
        StopOutcome::Terminated
    } else {
        StopOutcome::AlreadyStopped
    };
    fs_support::remove_file_if_exists(pid_path)?;
    Ok(outcome)
}

enum PidRecord {
    Verified(ProcessIdentity),
    Legacy(u32),
}

impl PidRecord {
    fn pid(&self) -> u32 {
        match self {
            Self::Verified(identity) => identity.pid,
            Self::Legacy(pid) => *pid,
        }
    }

    fn is_alive(&self) -> Result<bool> {
        match self {
            #[cfg(unix)]
            Self::Verified(identity) => identity.is_alive(),
            #[cfg(windows)]
            Self::Verified(identity) => identity.candidate_is_alive(),
            Self::Legacy(pid) => Ok(platform::check_process_alive(*pid)?),
        }
    }
}

pub fn ensure_prod_stopped(path: &Path) -> Result<()> {
    if let Some(record) = read_record(path)?
        && record.is_alive()?
    {
        bail!(
            "ComfyUI is already running (PID {}); run gov stop first",
            record.pid()
        );
    }
    Ok(())
}

pub fn runtime_status(path: &Path) -> Result<Option<(u32, bool)>> {
    read_record(path)?
        .map(|record| Ok((record.pid(), record.is_alive()?)))
        .transpose()
}

fn read_record(path: &Path) -> Result<Option<PidRecord>> {
    let Some(text) = fs_support::read_text_if_exists(path)? else {
        return Ok(None);
    };
    if let Ok(pid) = text.trim().parse::<u32>()
        && pid > 0
    {
        return Ok(Some(PidRecord::Legacy(pid)));
    }
    let identity: ProcessIdentity = serde_json::from_str(&text).with_context(|| {
        format!(
            "invalid process record at {}; verify ComfyUI is stopped",
            path.display()
        )
    })?;
    ensure!(
        identity.pid > 0 && !identity.creation.is_empty(),
        "invalid ComfyUI process identity"
    );
    Ok(Some(PidRecord::Verified(identity)))
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

const CANDIDATE_LAUNCHER: &str = "--internal-candidate-launcher";
#[cfg(windows)]
const PROD_LAUNCHER: &str = "--internal-prod-launcher";

pub fn launcher_entry(mut args: impl Iterator<Item = OsString>) -> Option<i32> {
    args.next();
    let launcher = args.next()?;
    #[cfg(windows)]
    if launcher == PROD_LAUNCHER {
        return Some(run_windows_prod_launcher(args).unwrap_or_else(|error| {
            eprintln!("gov: production launcher failed: {error:#}");
            125
        }));
    }
    if launcher != CANDIDATE_LAUNCHER {
        return None;
    }
    let mut gate = [0];
    if std::io::stdin().read_exact(&mut gate).is_err() || gate != [1] {
        return Some(125);
    }
    let mut command = Command::new(args.next()?);
    command.args(args).stdin(Stdio::null());
    #[cfg(unix)]
    let result = {
        use std::os::unix::process::CommandExt;
        Err(command.exec())
    };
    #[cfg(windows)]
    let result = command.status().map(platform::exit_code);
    Some(result.unwrap_or_else(|error| {
        eprintln!("gov: process launcher failed: {error}");
        125
    }))
}

#[cfg(windows)]
fn run_windows_prod_launcher(mut args: impl Iterator<Item = OsString>) -> Result<i32> {
    let gate = std::path::PathBuf::from(args.next().context("missing production start gate")?);
    let owner = args.next().context("missing production owner identity")?;
    let owner: ProcessIdentity = serde_json::from_str(
        owner
            .to_str()
            .context("invalid production owner identity")?,
    )?;
    platform::keep_launcher_alive_on_break()?;
    loop {
        match fs::read(&gate) {
            Ok(bytes) if bytes == [1] => break,
            Ok(_) => bail!("invalid production start gate"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(owner.is_alive()?, "production owner exited before launch");
        std::thread::sleep(Duration::from_millis(20));
    }
    let program = args.next().context("missing production interpreter")?;
    Ok(platform::exit_code(
        Command::new(program).args(args).status()?,
    ))
}

pub fn run_candidate(
    env: &Path,
    comfyui_dir: &Path,
    timeout_seconds: u32,
    stdout: &Path,
    stderr: &Path,
    probe_report: &Path,
    record: impl FnOnce(ProcessIdentity) -> Result<()>,
) -> Result<RunOutcome> {
    let python = platform::venv_python(env);
    ensure!(
        python.is_file(),
        "candidate Python missing: {}",
        python.display()
    );
    let out_file = File::create(stdout)?;
    let err_file = File::create(stderr)?;
    let interrupts = platform::RunInterrupts::install()?;
    let mut process = platform::CandidateProcess::spawn(
        Command::new(std::env::current_exe()?)
            .arg(CANDIDATE_LAUNCHER)
            .arg(python)
            .args(["-u", "-c", include_str!("probe.py")])
            .arg(comfyui_dir.join("main.py"))
            .arg(probe_report)
            .current_dir(comfyui_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .context("failed to start candidate launcher")?;

    let mut relays = Vec::with_capacity(2);
    let (send, receive) = std::sync::mpsc::channel();
    let observation = (|| -> Result<Option<RunOutcome>> {
        let out = process
            .child
            .stdout
            .take()
            .context("missing child stdout")?;
        let err = process
            .child
            .stderr
            .take()
            .context("missing child stderr")?;
        let out_send = send.clone();
        relays.push(std::thread::Builder::new().spawn(move || {
            let _ = out_send.send(copy_stream(out, out_file, false));
        })?);
        relays.push(std::thread::Builder::new().spawn(move || {
            let _ = send.send(copy_stream(err, err_file, true));
        })?);
        record(ProcessIdentity::capture(process.child.id())?)?;
        release_launcher(&mut process)?;
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds.into());
        loop {
            if interrupts.interrupted() {
                return Ok(Some(RunOutcome::Interrupted));
            }
            if let Ok(result) = receive.try_recv() {
                result.context("candidate log relay failed")?;
            }
            if process.has_exited()? {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Ok(Some(RunOutcome::TimedOut));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();

    let status = reap(&mut process);
    let deadline = Instant::now() + Duration::from_secs(2);
    for relay in relays {
        while !relay.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        ensure!(
            relay.is_finished(),
            "candidate log pipe did not close after termination"
        );
        relay
            .join()
            .map_err(|_| anyhow::anyhow!("candidate log relay panicked"))?;
    }
    let status = status?;
    let outcome = observation?;
    for result in receive.try_iter() {
        result.context("candidate log relay failed")?;
    }
    if let Some(outcome) = outcome {
        return Ok(outcome);
    }
    match status.code() {
        Some(0) => Ok(RunOutcome::Passed),
        Some(code) => Ok(RunOutcome::Failed(code)),
        None => bail!("candidate process terminated without an exit code"),
    }
}

pub fn run_tracked_command(
    command: &Command,
    timeout: Option<Duration>,
    mut record: impl FnMut(Option<ProcessIdentity>) -> Result<()>,
) -> Result<crate::domain::CmdResult> {
    let mut launcher = Command::new(std::env::current_exe()?);
    launcher
        .arg(CANDIDATE_LAUNCHER)
        .arg(command.get_program())
        .args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        launcher.current_dir(dir);
    }
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            launcher.env(name, value);
        } else {
            launcher.env_remove(name);
        }
    }
    launcher
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let interrupts = platform::RunInterrupts::install()?;
    let mut process = platform::CandidateProcess::spawn(&mut launcher)?;
    let mut readers = Vec::with_capacity(2);
    let result = (|| -> Result<()> {
        readers.push(read_output(
            process.child.stdout.take().context("missing stdout")?,
        )?);
        readers.push(read_output(
            process.child.stderr.take().context("missing stderr")?,
        )?);
        record(Some(ProcessIdentity::capture(process.child.id())?))?;
        release_launcher(&mut process)?;
        let start = Instant::now();
        loop {
            ensure!(!interrupts.interrupted(), "external command interrupted");
            if process.has_exited()? {
                return Ok(());
            }
            if let Some(timeout) = timeout {
                ensure!(
                    start.elapsed() < timeout,
                    "external command timed out after {}s",
                    timeout.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    let status = reap(&mut process)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut output = Vec::with_capacity(2);
    for reader in readers {
        while !reader.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        ensure!(
            reader.is_finished(),
            "external command output pipe did not close"
        );
        output.push(
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("output reader panicked"))??,
        );
    }
    record(None)?;
    result?;
    let stderr = output.pop().context("missing stderr output")?;
    let stdout = output.pop().context("missing stdout output")?;
    Ok(crate::domain::CmdResult::from_output(Output {
        status,
        stdout,
        stderr,
    }))
}

fn read_output(
    mut source: impl Read + Send + 'static,
) -> Result<std::thread::JoinHandle<std::io::Result<Vec<u8>>>> {
    Ok(std::thread::Builder::new().spawn(move || {
        let mut output = Vec::new();
        source.read_to_end(&mut output)?;
        Ok(output)
    })?)
}

fn release_launcher(process: &mut platform::CandidateProcess) -> Result<()> {
    let mut gate = process
        .child
        .stdin
        .take()
        .context("missing process start gate")?;
    gate.write_all(&[1])?;
    Ok(())
}

fn reap(process: &mut platform::CandidateProcess) -> Result<std::process::ExitStatus> {
    drop(process.child.stdin.take());
    let terminated = process
        .terminate()
        .context("failed to terminate process tree");
    if terminated.is_err() {
        let _ = process.child.kill();
    }
    let status = process.child.wait().context("failed to reap process");
    terminated?;
    status
}

pub fn node_load_results(log: &Path) -> Result<Vec<(String, bool)>> {
    let text = String::from_utf8_lossy(&fs::read(log)?).into_owned();
    let mut summary = false;
    let mut results = Vec::new();
    for line in text.lines().map(str::trim) {
        if line == "Import times for custom nodes:" {
            summary = true;
            continue;
        }
        if !summary {
            continue;
        }
        let Some((seconds, suffix)) = line.split_once(" seconds") else {
            summary = false;
            continue;
        };
        if seconds.parse::<f64>().is_err() {
            summary = false;
            continue;
        }
        if let Some(path) = suffix.strip_prefix(" (IMPORT FAILED): ") {
            results.push((path.to_owned(), false));
        } else if let Some(path) = suffix.strip_prefix(": ") {
            results.push((path.to_owned(), true));
        } else {
            summary = false;
        }
    }
    Ok(results)
}

fn copy_stream(mut source: impl Read, mut file: File, error: bool) -> Result<()> {
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
