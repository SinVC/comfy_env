#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use time::{OffsetDateTime, format_description::well_known::Rfc3339, macros::format_description};

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};

use crate::domain::{OpId, OpStatus, ProjectRoot, TxStatus};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginRecord {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TransactionRecord {
    #[serde(default, alias = "txid")]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub status: Option<TxStatus>,
    #[serde(default)]
    pub subject: String,
    #[serde(default, alias = "started_at")]
    pub created_at: Option<String>,
    #[serde(default, alias = "ended_at")]
    pub updated_at: Option<String>,
}

impl TransactionRecord {
    pub fn sort_key(&self) -> Option<&str> {
        self.updated_at
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(self.created_at.as_deref())
            .or(Some(self.id.as_str()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    InstallTorch,
    InstallCore,
    PinAdd,
    PinRemove,
    ManualUndo,
    #[serde(untagged)]
    Historical(String),
}

impl std::fmt::Display for OpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::InstallTorch => "install_torch",
            Self::InstallCore => "install_core",
            Self::PinAdd => "pin_add",
            Self::PinRemove => "pin_remove",
            Self::ManualUndo => "manual_undo",
            Self::Historical(kind) => kind,
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedFileHashes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRecord {
    pub op_id: String,
    pub kind: OpKind,
    #[serde(alias = "ref")]
    pub reference: String,
    pub status: OpStatus,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub files: BTreeMap<String, TrackedFileHashes>,
    pub backup_dir: String,
    #[serde(default)]
    pub undoable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_reference: Option<String>,
}

// Early Rust records used Unix seconds with a millisecond fraction and a Z suffix.
fn operation_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok().or_else(|| {
        let (seconds, millis) = value.strip_suffix('Z')?.split_once('.')?;
        OffsetDateTime::from_unix_timestamp(seconds.parse().ok()?)
            .ok()?
            .replace_millisecond(millis.parse().ok()?)
            .ok()
    })
}

pub fn load_plugins(root: &ProjectRoot) -> Result<Vec<PluginRecord>> {
    let path = root.plugins_registry();
    let Some(text) = crate::fs_support::read_text_if_exists(&path)? else {
        return Ok(Vec::new());
    };

    let records = serde_json::from_str::<Vec<PluginRecord>>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(records)
}

pub fn load_transactions(root: &ProjectRoot) -> Result<Vec<TransactionRecord>> {
    let dir = root.transactions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to inspect {}", dir.display()))?;
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("json")
        ) {
            continue;
        }

        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let record = serde_json::from_str::<TransactionRecord>(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        records.push(record);
    }

    records.sort_by(|left, right| right.sort_key().cmp(&left.sort_key()));
    Ok(records)
}

pub fn next_op_id() -> Result<OpId> {
    let stamp = OffsetDateTime::now_utc().format(format_description!(
        "[year][month][day]T[hour][minute][second]Z"
    ))?;
    let suffix =
        getrandom::u32().map_err(|error| anyhow!("failed to generate operation ID: {error}"))?;
    Ok(OpId(format!("{stamp}-op-{suffix:08x}")))
}

pub fn load_operation(root: &ProjectRoot, op_id: &OpId) -> Result<OperationRecord> {
    ensure!(
        Path::new(&op_id.0).file_name() == Some(std::ffi::OsStr::new(&op_id.0)),
        "invalid operation ID"
    );
    let path = op_meta_path(root, op_id);
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let record = serde_json::from_str::<OperationRecord>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    ensure!(
        record.op_id == op_id.0,
        "operation ID differs from its directory"
    );
    Ok(record)
}

pub fn save_operation(root: &ProjectRoot, op: &OperationRecord) -> Result<()> {
    let path = root.ops_dir().join(&op.op_id).join("meta.json");
    crate::fs_support::atomic_write(&path, &serde_json::to_vec_pretty(op)?)
}

pub fn list_operations(root: &ProjectRoot) -> Result<Vec<OperationRecord>> {
    let dir = root.ops_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut operations = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to inspect {}", dir.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?
            .is_dir()
        {
            continue;
        }
        let path = entry.path().join("meta.json");
        if !path.exists() {
            continue;
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let record = serde_json::from_str::<OperationRecord>(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        ensure!(
            entry.file_name() == std::ffi::OsStr::new(&record.op_id),
            "operation ID differs from its directory: {}",
            path.display()
        );
        operations.push(record);
    }

    operations.sort_by_cached_key(|op| std::cmp::Reverse(operation_time(&op.started_at)));
    Ok(operations)
}

pub fn prune_operations(root: &ProjectRoot, retention_count: usize) -> Result<()> {
    let completed = list_operations(root)?
        .into_iter()
        .filter(|op| op.status != OpStatus::Running);
    for op in completed.skip(retention_count.max(1)) {
        let directory = root.ops_dir().join(op.op_id);
        fs::remove_dir_all(&directory)
            .with_context(|| format!("failed to prune {}", directory.display()))?;
    }
    Ok(())
}

pub fn mark_operation_undone(
    root: &ProjectRoot,
    record: &mut OperationRecord,
    undo_reference: &OpId,
) -> Result<()> {
    record.status = OpStatus::Undone;
    record.undoable = false;
    record.ended_at = Some(now_string()?);
    record.undo_reference = Some(undo_reference.0.clone());
    save_operation(root, record)
}

pub fn now_string() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

fn op_meta_path(root: &ProjectRoot, op_id: &OpId) -> PathBuf {
    root.ops_dir().join(&op_id.0).join("meta.json")
}
