#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

use anyhow::{Context, Result};

use crate::domain::CmdResult;

pub struct PythonClient;

#[derive(Debug, Clone, Copy)]
pub enum RunOutcome {
    Passed,
    Failed(i32),
    TimedOut,
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
