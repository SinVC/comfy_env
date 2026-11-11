use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::Output;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{message}")]
    Usage { message: String },
    #[error("{message}")]
    Runtime { message: String },
}

impl AppError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime {
            message: message.into(),
        }
    }

    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Usage { .. } => 2,
            Self::Runtime { .. } => 1,
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self::runtime(format!("{value:#}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::runtime(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct ProjectRoot {
    root: PathBuf,
}

impl ProjectRoot {
    pub fn discover() -> AppResult<Self> {
        let root = std::env::current_dir().map_err(|error| {
            AppError::runtime(format!("failed to resolve project root: {error}"))
        })?;
        let sidecar = root.join("ComfyUI.gov");
        if root
            .join("bin")
            .join(if cfg!(windows) { "gov.exe" } else { "gov" })
            .is_file()
            && ((sidecar.join("config.toml").is_file() && sidecar.join("pyproject.toml").is_file())
                || (sidecar.join("config.toml.template").is_file()
                    && sidecar.join("pyproject.toml.template").is_file()))
        {
            return Ok(Self::at(sidecar));
        }
        Ok(Self::at(root))
    }

    pub(crate) fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn as_path(&self) -> &Path {
        &self.root
    }

    pub fn config_toml(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn pyproject_toml(&self) -> PathBuf {
        self.root.join("pyproject.toml")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn uv_lock(&self) -> PathBuf {
        self.root.join("uv.lock")
    }

    pub fn plugins_registry(&self) -> PathBuf {
        self.state_dir().join("plugins.json")
    }

    pub fn transactions_dir(&self) -> PathBuf {
        self.state_dir().join("transactions")
    }

    pub fn ops_dir(&self) -> PathBuf {
        self.state_dir().join("ops")
    }

    pub fn work_dir(&self) -> PathBuf {
        self.state_dir().join("work")
    }

    pub fn pid_file(&self) -> PathBuf {
        self.state_dir().join("comfyui.pid")
    }

    pub fn prod_env_dir(&self) -> PathBuf {
        self.root.join(".venv-prod")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TxId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OpId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupName(pub String);

pub const TORCH_FAMILY: [&str; 3] = ["torch", "torchvision", "torchaudio"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxKind {
    Plugin,
    CoreUpdate,
    #[serde(untagged)]
    Historical(String),
}

impl Default for TxKind {
    fn default() -> Self {
        Self::Historical(String::new())
    }
}

impl Display for TxKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Plugin => "plugin",
            Self::CoreUpdate => "core_update",
            Self::Historical(value) => value,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxStatus {
    Running,
    Completed,
    Failed,
    NeedsResolution,
    Resolved,
    Promoted,
    PromoteFailed,
    Aborted,
}

impl Display for TxStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::NeedsResolution => "needs_resolution",
            Self::Resolved => "resolved",
            Self::Promoted => "promoted",
            Self::PromoteFailed => "promote_failed",
            Self::Aborted => "aborted",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    Running,
    Success,
    Failed,
    Undone,
}

impl Display for OpStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Undone => "undone",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone)]
pub struct CmdResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CmdResult {
    pub fn from_output(output: Output) -> Self {
        Self {
            success: output.status.success(),
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        }
    }

    pub fn summary_line(&self) -> String {
        if !self.success && !self.stderr.is_empty() {
            return self.stderr.clone();
        }
        let preferred = if self.stdout.is_empty() {
            self.stderr.as_str()
        } else {
            self.stdout.as_str()
        };

        if preferred.is_empty() {
            "no output".to_string()
        } else {
            preferred.lines().next().unwrap_or("no output").to_string()
        }
    }
}
