use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use wait_timeout::ChildExt;

use anyhow::{Context, Result};

use crate::domain::{OpId, OpStatus, ProjectRoot};
use crate::platform;
use crate::state_ledger::{self, OpKind, OperationRecord, TrackedFileHashes};
use crate::toml_support::RuntimeConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftCheck {
    Clean,
    Drifted(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmokeResult {
    pub passed: bool,
    pub detail: String,
}

pub fn op_begin(kind: OpKind, reference: &str, root: &ProjectRoot) -> Result<OpId> {
    let op_id = state_ledger::next_op_id()?;
    let backup_dir = root.ops_dir().join(&op_id.0).join("backup");
    std::fs::create_dir_all(&backup_dir)
        .with_context(|| format!("failed to create {}", backup_dir.display()))?;

    let mut files = BTreeMap::new();
    for (name, path) in tracked_files(root) {
        let backup = backup_dir.join(name);
        let marker = backup_dir.join(format!("{name}.exists"));
        crate::fs_support::backup_with_marker(&path, &backup, &marker)?;
        files.insert(
            name.to_string(),
            TrackedFileHashes {
                pre_sha256: crate::fs_support::sha256_if_exists(&path)?,
                post_sha256: None,
            },
        );
    }

    let record = OperationRecord {
        op_id: op_id.0.clone(),
        kind,
        reference: reference.to_string(),
        status: OpStatus::Running,
        started_at: state_ledger::now_string()?,
        ended_at: None,
        files,
        backup_dir: crate::platform::to_state_path(&backup_dir),
        undoable: false,
        note: None,
        undo_reference: None,
    };
    state_ledger::save_operation(root, &record)?;

    Ok(op_id)
}

pub fn op_finalize(
    op_id: &OpId,
    success: bool,
    note: Option<String>,
    root: &ProjectRoot,
) -> Result<()> {
    let mut record = state_ledger::load_operation(root, op_id)?;
    record.status = if success {
        OpStatus::Success
    } else {
        OpStatus::Failed
    };
    record.undoable = success;
    record.ended_at = Some(state_ledger::now_string()?);
    record.note = note;

    if success {
        for (name, path) in tracked_files(root) {
            if let Some(file) = record.files.get_mut(name) {
                file.post_sha256 = crate::fs_support::sha256_if_exists(&path)?;
            }
        }
    }

    state_ledger::save_operation(root, &record)
}

pub fn op_restore(op_id: &OpId, root: &ProjectRoot) -> Result<()> {
    let backup_dir = root.ops_dir().join(&op_id.0).join("backup");
    for (name, path) in tracked_files(root) {
        let backup = backup_dir.join(name);
        let marker = backup_dir.join(format!("{name}.exists"));
        let marker = if marker.try_exists()? {
            marker
        } else {
            backup_dir.join(format!("{name}.present"))
        };
        crate::fs_support::restore_with_marker(&backup, &path, &marker)?;
    }

    Ok(())
}

pub fn check_undo_drift(record: &OperationRecord, root: &ProjectRoot) -> Result<DriftCheck> {
    let mut mismatches = Vec::new();

    for (name, path) in tracked_files(root) {
        let Some(file) = record.files.get(name) else {
            anyhow::bail!("missing post-operation file record for {name}");
        };
        let current = crate::fs_support::sha256_if_exists(&path)?;
        if current.as_deref() != file.post_sha256.as_deref().filter(|hash| !hash.is_empty()) {
            mismatches.push(name.to_string());
        }
    }

    if mismatches.is_empty() {
        Ok(DriftCheck::Clean)
    } else {
        Ok(DriftCheck::Drifted(mismatches))
    }
}

pub fn run_smoke_test(env_path: &Path, config: &RuntimeConfig) -> Result<SmokeResult> {
    let (program, args) = match &config.tx.smoke_test {
        Some(smoke) => {
            let program = if smoke.program == "python" {
                platform::venv_python(env_path)
                    .to_string_lossy()
                    .to_string()
            } else {
                smoke.program.clone()
            };
            (program, smoke.args.clone())
        }
        None => (
            platform::venv_python(env_path)
                .to_string_lossy()
                .to_string(),
            vec![
                "-c".to_string(),
                "import sys; print(sys.version)".to_string(),
            ],
        ),
    };

    execute_smoke(&program, &args, config.tx.timeout_seconds)
}

pub fn run_torch_smoke_test(env_path: &Path, config: &RuntimeConfig) -> Result<SmokeResult> {
    let program = platform::venv_python(env_path)
        .to_string_lossy()
        .into_owned();
    let args = ["-c", "import torch, torchvision, torchaudio"];
    execute_smoke(&program, &args, config.tx.timeout_seconds)
}

fn execute_smoke(
    program: &str,
    args: &[impl AsRef<std::ffi::OsStr>],
    timeout_seconds: u32,
) -> Result<SmokeResult> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(std::io::stderr())
        .spawn()
        .with_context(|| format!("failed to run smoke test `{program}`"))?;
    let timeout = Duration::from_secs(timeout_seconds.into());
    if let Some(status) = child.wait_timeout(timeout)? {
        return Ok(SmokeResult {
            passed: status.success(),
            detail: status.to_string(),
        });
    }
    child.kill()?;
    child.wait()?;
    Ok(SmokeResult {
        passed: false,
        detail: format!("timed out after {}s", timeout.as_secs()),
    })
}

fn tracked_files(root: &ProjectRoot) -> [(&'static str, PathBuf); 3] {
    [
        ("pyproject.toml", root.pyproject_toml()),
        ("uv.lock", root.uv_lock()),
        ("plugins.json", root.plugins_registry()),
    ]
}
