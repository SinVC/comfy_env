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
    if record.kind == OpKind::EnvImport {
        record.undoable = false;
    }
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

    let source_path_file = backup_dir.join("custom_node.path");
    if source_path_file.exists() {
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

    if backup_dir.join("custom_nodes.list.json").exists() {
        restore_import_nodes(op_id, root)?;
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

pub fn backup_import_config(op_id: &OpId, root: &ProjectRoot) -> Result<()> {
    let backup = root.ops_dir().join(&op_id.0).join("backup");
    crate::fs_support::backup_with_marker(
        &root.config_toml(),
        &backup.join("config.toml"),
        &backup.join("config.toml.exists"),
    )
}

pub fn backup_import_nodes(op_id: &OpId, nodes_root: &Path, root: &ProjectRoot) -> Result<()> {
    let backup = root.ops_dir().join(&op_id.0).join("backup");
    let mut names = Vec::new();
    if nodes_root.exists() {
        for entry in std::fs::read_dir(nodes_root)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            crate::fs_support::copy_tree_preserving_links(
                &entry.path(),
                &backup.join("custom_nodes").join(&name),
            )?;
            names.push(name);
        }
    }
    crate::fs_support::atomic_write(
        &backup.join("custom_nodes.root"),
        crate::platform::to_state_path(nodes_root).as_bytes(),
    )?;
    crate::fs_support::atomic_write(
        &backup.join("custom_nodes.list.json"),
        &serde_json::to_vec(&names)?,
    )
}

fn restore_import_nodes(op_id: &OpId, root: &ProjectRoot) -> Result<()> {
    let backup = root.ops_dir().join(&op_id.0).join("backup");
    let nodes_root = crate::platform::from_state_path(&std::fs::read_to_string(
        backup.join("custom_nodes.root"),
    )?);
    let names: Vec<String> =
        serde_json::from_slice(&std::fs::read(backup.join("custom_nodes.list.json"))?)?;
    if nodes_root.exists() {
        for entry in std::fs::read_dir(&nodes_root)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            crate::fs_support::remove_tree(&entry.path())?;
        }
    }
    for name in names {
        crate::fs_support::copy_tree_preserving_links(
            &backup.join("custom_nodes").join(&name),
            &nodes_root.join(name),
        )?;
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
