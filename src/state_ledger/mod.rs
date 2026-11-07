#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use time::{OffsetDateTime, format_description::well_known::Rfc3339, macros::format_description};

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};

use crate::domain::{OpId, OpStatus, ProjectRoot, TxId, TxStatus};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRecord {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub git_url: String,
    #[serde(default, rename = "ref")]
    pub git_ref: String,
    #[serde(default)]
    pub install_relpath: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub managed_deps: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionRecord {
    #[serde(default, rename = "txid", alias = "id")]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub status: Option<TxStatus>,
    #[serde(default)]
    pub subject: String,
    #[serde(default, rename = "started_at", alias = "created_at")]
    pub created_at: Option<String>,
    #[serde(default, rename = "ended_at", alias = "updated_at")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub candidate_env: String,
    #[serde(default)]
    pub staged_workdir: String,
    #[serde(default)]
    pub pre_freeze: Vec<String>,
    #[serde(default)]
    pub post_freeze: Vec<String>,
    #[serde(default)]
    pub diff: PackageDiff,
    #[serde(default)]
    pub core_impact: Vec<String>,
    #[serde(default)]
    pub logs: TransactionLogs,
    #[serde(default)]
    pub conflict_report: Option<String>,
    #[serde(default)]
    pub resolution_pins: Vec<String>,
    #[serde(default)]
    pub promotion: Option<PromotionRecord>,
    #[serde(default)]
    pub observed_requirements: Vec<String>,
    #[serde(default)]
    pub candidate_ready: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransactionLogs {
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub run_exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionRecord {
    pub status: TxStatus,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub op_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictReport {
    pub txid: String,
    pub node_id: String,
    pub created_at: String,
    pub raw_log: String,
    pub summary: String,
    pub detected_packages: Vec<String>,
    pub input_hint: String,
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
    NodeRemove,
    PluginPromote,
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
            Self::NodeRemove => "node_remove",
            Self::PluginPromote => "plugin_promote",
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

pub fn save_plugins(root: &ProjectRoot, plugins: &[PluginRecord]) -> Result<()> {
    crate::fs_support::atomic_write(
        &root.plugins_registry(),
        &serde_json::to_vec_pretty(plugins)?,
    )
}

pub fn next_tx_id() -> Result<TxId> {
    let stamp = OffsetDateTime::now_utc().format(format_description!(
        "[year][month][day]T[hour][minute][second]Z"
    ))?;
    let suffix =
        getrandom::u32().map_err(|error| anyhow!("failed to generate transaction ID: {error}"))?;
    Ok(TxId(format!("{stamp}-{suffix:08x}")))
}

pub fn save_transaction(root: &ProjectRoot, tx: &TransactionRecord) -> Result<()> {
    let id = TxId(tx.id.clone());
    validate_tx_id(&id)?;
    crate::fs_support::atomic_write(
        &root.transactions_dir().join(format!("{}.json", tx.id)),
        &serde_json::to_vec_pretty(tx)?,
    )
}

pub fn load_transaction(root: &ProjectRoot, id: &TxId) -> Result<TransactionRecord> {
    validate_tx_id(id)?;
    let path = root.transactions_dir().join(format!("{}.json", id.0));
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let tx: TransactionRecord = serde_json::from_str(&text)?;
    ensure!(tx.id == id.0, "transaction ID differs from filename");
    Ok(tx)
}

fn validate_tx_id(id: &TxId) -> Result<()> {
    ensure!(
        !id.0.is_empty() && id.0.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "invalid transaction ID"
    );
    Ok(())
}

pub fn write_conflict(root: &ProjectRoot, tx: &TransactionRecord, raw: &str) -> Result<String> {
    let dir = root.state_dir().join("conflicts");
    let log = dir.join(format!("{}.lock.log", tx.id));
    crate::fs_support::atomic_write(&log, raw.as_bytes())?;
    let report = ConflictReport {
        txid: tx.id.clone(),
        node_id: tx.node_id.clone().unwrap_or_default(),
        created_at: now_string()?,
        raw_log: crate::platform::to_state_path(&log),
        summary: raw.lines().take(40).collect::<Vec<_>>().join("\n"),
        detected_packages: detect_conflict_packages(raw),
        input_hint: format!("Use: gov resolve {} --pin pkg==version", tx.id),
    };
    let path = dir.join(format!("{}.json", tx.id));
    crate::fs_support::atomic_write(&path, &serde_json::to_vec_pretty(&report)?)?;
    Ok(crate::platform::to_state_path(&path))
}

fn detect_conflict_packages(raw: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for line in raw.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        for pair in words.windows(2) {
            if matches!(
                pair[0].to_ascii_lowercase().as_str(),
                "package" | "requires"
            ) {
                let token = pair[1].trim_matches(|ch: char| {
                    !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '.'
                });
                let name = crate::toml_support::normalize_package_name(token);
                if crate::toml_support::is_package_name(&name) {
                    names.insert(name);
                }
            }
        }
    }
    names.into_iter().collect()
}

pub fn load_conflict(root: &ProjectRoot, id: &TxId) -> Result<Option<ConflictReport>> {
    validate_tx_id(id)?;
    let path = root
        .state_dir()
        .join("conflicts")
        .join(format!("{}.json", id.0));
    let Some(text) = crate::fs_support::read_text_if_exists(&path)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_str(&text)?))
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
