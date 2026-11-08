use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::domain::{OpId, OpStatus, ProjectRoot};
use crate::platform;
use crate::state_ledger::{
    self, OpKind, OperationLink, OperationPhase, OperationRecord, OperationRecovery,
    TrackedFileHashes,
};
use crate::toml_support::RuntimeConfig;

pub mod node_swap;

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

pub fn op_begin(
    kind: OpKind,
    reference: &str,
    link: Option<OperationLink>,
    root: &ProjectRoot,
) -> Result<OperationRecord> {
    crate::runtime_executor::ensure_prod_stopped(&root.pid_file())?;
    ensure_production_recoverable(root)?;
    let owner = platform::ProcessIdentity::capture(std::process::id())?;
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
        recovery: Some(OperationRecovery {
            phase: OperationPhase::Prepared,
            owner,
            active_process: None,
            source_touched: false,
            sync: None,
            link,
            linked: false,
            apply_error: None,
            restore_error: None,
        }),
    };
    state_ledger::save_operation(root, &record)?;

    Ok(record)
}

pub fn ensure_production_recoverable(root: &ProjectRoot) -> Result<()> {
    if let Some(op) = state_ledger::operations_for_recovery(root)?
        .into_iter()
        .find(|op| op.needs_recovery())
    {
        anyhow::bail!(
            "operation {} needs recovery; run gov op recover {}",
            op.op_id,
            op.op_id
        );
    }
    Ok(())
}

pub fn op_finalize(
    record: &mut OperationRecord,
    success: bool,
    note: Option<String>,
    root: &ProjectRoot,
) -> Result<()> {
    record.status = if success {
        OpStatus::Success
    } else {
        OpStatus::Failed
    };
    record.undoable = success;
    if matches!(record.kind, OpKind::EnvImport | OpKind::Reinit) {
        record.undoable = false;
    }
    record.ended_at = Some(state_ledger::now_string()?);
    record.note = note;

    if let Some(recovery) = &mut record.recovery {
        recovery.phase = if success {
            OperationPhase::Committed
        } else {
            OperationPhase::Restored
        };
    }
    if success {
        for (name, path) in tracked_files(root) {
            if let Some(file) = record.files.get_mut(name) {
                file.post_sha256 = crate::fs_support::sha256_if_exists(&path)?;
            }
        }
        if let Some(file) = record.files.get_mut("config.toml") {
            file.post_sha256 = crate::fs_support::sha256_if_exists(&root.config_toml())?;
        }
    }

    state_ledger::save_operation(root, record)
}

pub fn op_restore(op_id: &OpId, root: &ProjectRoot, source_touched: bool) -> Result<()> {
    let backup_dir = root.ops_dir().join(&op_id.0).join("backup");
    verify_backup(op_id, root, source_touched)?;
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

    let source_path_file = backup_dir.join("custom_node.path");
    if source_touched && source_path_file.exists() {
        let target = crate::platform::from_state_path(&std::fs::read_to_string(&source_path_file)?);
        crate::source_integration::remove_plugin_source(&target)?;
        let exists =
            crate::fs_support::read_text_if_exists(&backup_dir.join("custom_node.exists"))?
                .unwrap_or_else(|| "1".to_string());
        match exists.trim() {
            "1" => crate::source_integration::copy_plugin_source(
                &backup_dir.join("custom_node"),
                &target,
            )?,
            "0" => {}
            other => anyhow::bail!("invalid custom node existence marker: {other}"),
        }
    }

    if source_touched {
        node_swap::restore(root, op_id)?;
    }

    let config_marker = backup_dir.join("config.toml.exists");
    if config_marker.exists() {
        crate::fs_support::restore_with_marker(
            &backup_dir.join("config.toml"),
            &root.config_toml(),
            &config_marker,
        )?;
    }

    Ok(())
}

pub fn backup_config(record: &mut OperationRecord, root: &ProjectRoot) -> Result<()> {
    let backup = root.ops_dir().join(&record.op_id).join("backup");
    crate::fs_support::backup_with_marker(
        &root.config_toml(),
        &backup.join("config.toml"),
        &backup.join("config.toml.exists"),
    )?;
    record.files.insert(
        "config.toml".into(),
        TrackedFileHashes {
            pre_sha256: crate::fs_support::sha256_if_exists(&root.config_toml())?,
            post_sha256: None,
        },
    );
    state_ledger::save_operation(root, record)
}

pub fn verify_backup(op_id: &OpId, root: &ProjectRoot, source_touched: bool) -> Result<()> {
    let record = state_ledger::load_operation(root, op_id)?;
    let backup = root.ops_dir().join(&op_id.0).join("backup");
    if source_touched
        && record
            .recovery
            .as_ref()
            .is_some_and(|recovery| recovery.source_touched)
    {
        match record.kind {
            OpKind::EnvImport => node_swap::verify_journal(root, op_id)?,
            OpKind::NodeRemove | OpKind::ManualUndo => ensure!(
                backup.join("custom_node.path").is_file(),
                "missing custom node recovery evidence"
            ),
            _ => {}
        }
    }
    let mut names = tracked_files(root).map(|(name, _)| name).to_vec();
    if record.files.contains_key("config.toml") {
        names.push("config.toml");
    }
    for name in names {
        let marker = backup.join(format!("{name}.exists"));
        let marker = if marker.try_exists()? {
            marker
        } else {
            backup.join(format!("{name}.present"))
        };
        let present = std::fs::read_to_string(&marker)
            .with_context(|| format!("missing backup marker {}", marker.display()))?;
        let actual = match present.trim() {
            "1" => Some(crate::fs_support::sha256(&backup.join(name))?),
            "0" => None,
            _ => anyhow::bail!("invalid backup marker {}", marker.display()),
        };
        if record.recovery.is_some() {
            let expected = record
                .files
                .get(name)
                .with_context(|| format!("missing backup evidence for {name}"))?;
            ensure!(
                actual == expected.pre_sha256,
                "backup checksum mismatch for {name}"
            );
        }
    }
    if source_touched && backup.join("custom_node.path").try_exists()? {
        let exists = crate::fs_support::read_text_if_exists(&backup.join("custom_node.exists"))?;
        match exists.as_deref().map(str::trim).unwrap_or("1") {
            "1" => ensure!(
                backup.join("custom_node").is_dir(),
                "missing custom node backup"
            ),
            "0" => {}
            _ => anyhow::bail!("invalid custom node backup marker"),
        }
    }
    Ok(())
}
pub fn backup_custom_node(op_id: &OpId, source: &Path, root: &ProjectRoot) -> Result<()> {
    let backup = root.ops_dir().join(&op_id.0).join("backup");
    crate::source_integration::copy_plugin_source(source, &backup.join("custom_node"))?;
    crate::fs_support::atomic_write(
        &backup.join("custom_node.path"),
        crate::platform::to_state_path(source).as_bytes(),
    )
}

pub fn backup_undo_custom_node(undo_id: &OpId, target_id: &OpId, root: &ProjectRoot) -> Result<()> {
    let original = root
        .ops_dir()
        .join(&target_id.0)
        .join("backup")
        .join("custom_node.path");
    if !original.exists() {
        return Ok(());
    }
    let source = crate::platform::from_state_path(&std::fs::read_to_string(&original)?);
    let backup = root.ops_dir().join(&undo_id.0).join("backup");
    if source.exists() {
        crate::source_integration::copy_plugin_source(&source, &backup.join("custom_node"))?;
        crate::fs_support::atomic_write(&backup.join("custom_node.exists"), b"1")?;
    } else {
        crate::fs_support::atomic_write(&backup.join("custom_node.exists"), b"0")?;
    }
    crate::fs_support::atomic_write(
        &backup.join("custom_node.path"),
        crate::platform::to_state_path(&source).as_bytes(),
    )
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

pub fn run_smoke_test(
    env_path: &Path,
    config: &RuntimeConfig,
    record: impl FnMut(Option<platform::ProcessIdentity>) -> Result<()>,
) -> Result<SmokeResult> {
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

    execute_smoke(&program, &args, config.tx.timeout_seconds, record)
}

pub fn run_torch_smoke_test(
    env_path: &Path,
    config: &RuntimeConfig,
    record: impl FnMut(Option<platform::ProcessIdentity>) -> Result<()>,
) -> Result<SmokeResult> {
    let program = platform::venv_python(env_path)
        .to_string_lossy()
        .into_owned();
    let args = ["-c", "import torch, torchvision, torchaudio"];
    execute_smoke(&program, &args, config.tx.timeout_seconds, record)
}

fn execute_smoke(
    program: &str,
    args: &[impl AsRef<std::ffi::OsStr>],
    timeout_seconds: u32,
    record: impl FnMut(Option<platform::ProcessIdentity>) -> Result<()>,
) -> Result<SmokeResult> {
    let mut command = Command::new(program);
    command.args(args);
    let result = crate::runtime_executor::run_tracked_command(
        &command,
        Some(Duration::from_secs(timeout_seconds.into())),
        record,
    )
    .with_context(|| format!("smoke test failed running `{program}`"))?;
    for output in [&result.stdout, &result.stderr] {
        if !output.is_empty() {
            eprintln!("{output}");
        }
    }
    Ok(SmokeResult {
        passed: result.success,
        detail: format!("exit code {}", result.exit_code),
    })
}

fn tracked_files(root: &ProjectRoot) -> [(&'static str, PathBuf); 3] {
    [
        ("pyproject.toml", root.pyproject_toml()),
        ("uv.lock", root.uv_lock()),
        ("plugins.json", root.plugins_registry()),
    ]
}
