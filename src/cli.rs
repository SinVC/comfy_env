use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "gov",
    version,
    about = "Governance CLI for ComfyUI dependency transactions",
    subcommand_required = true,
    arg_required_else_help = true,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install or resume a managed ComfyUI workspace
    Setup(SetupArgs),
    /// Bootstrap the project and production environment
    Init(InitArgs),
    /// Install managed ComfyUI or PyTorch dependencies
    Install(InstallArgs),
    /// Manage dependency overrides
    Pin(PinArgs),
    /// Register and remove custom nodes
    Node(NodeArgs),
    /// Observe and promote custom-node dependency transactions
    Tx(TxArgs),
    /// Observe and promote ComfyUI core dependency updates
    Update(UpdateArgs),
    /// Resolve a custom-node dependency conflict
    Resolve(ResolveArgs),
    /// Export or import a managed environment
    Env(EnvArgs),
    /// View the operation ledger
    Op(OpArgs),
    /// Restore the backup for an operation
    Undo(UndoArgs),
    /// Run ComfyUI in the production environment
    Run(RunArgs),
    /// Stop ComfyUI launched by gov run
    Stop,
    /// Report project and runtime status
    Status(StatusArgs),
    /// Verify that the managed ComfyUI environment can start
    Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit machine-readable workspace status
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Run the startup check in CPU mode
    #[arg(long)]
    pub cpu: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SetupMode {
    Fresh,
    Migrate,
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Directory containing bin, ComfyUI, and ComfyUI.gov
    #[arg(long)]
    pub workspace: PathBuf,
    #[arg(long, value_enum, default_value = "fresh")]
    pub mode: SetupMode,
    #[arg(long, default_value = "3.12")]
    pub python: String,
    #[arg(long, default_value = "https://download.pytorch.org/whl/cpu")]
    pub torch_index_url: String,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Absolute path to the ComfyUI directory
    #[arg(long)]
    pub comfyui_dir: Option<PathBuf>,
    /// Python minor version or interpreter selector
    #[arg(long)]
    pub python: Option<String>,
    /// Configure ComfyUI to run in CPU mode
    #[arg(long)]
    pub cpu: bool,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    #[command(subcommand)]
    pub command: Option<InstallSubcommand>,
    #[arg(long)]
    pub requirements_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum InstallSubcommand {
    Torch(InstallTorchArgs),
}

#[derive(Debug, Args)]
pub struct InstallTorchArgs {
    #[arg(long)]
    pub index_url: String,
    #[arg(long)]
    pub torch: Option<String>,
    #[arg(long)]
    pub torchvision: Option<String>,
    #[arg(long)]
    pub torchaudio: Option<String>,
}

#[derive(Debug, Args)]
pub struct PinArgs {
    #[command(subcommand)]
    pub command: PinSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum PinSubcommand {
    Add(PinMutateArgs),
    List(PinListArgs),
    Remove(PinMutateArgs),
}

#[derive(Debug, Args)]
pub struct PinMutateArgs {
    #[arg(required = true)]
    pub specs: Vec<String>,
    /// Manage unscoped exact uv requirement overrides
    #[arg(long)]
    pub override_requirements: bool,
}

#[derive(Debug, Args)]
pub struct PinListArgs {
    /// Label constraints, forced overrides and inconsistent historical entries
    #[arg(long)]
    pub details: bool,
}

#[derive(Debug, Args)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum NodeSubcommand {
    Add(NodeAddArgs),
    Remove(NodeRemoveArgs),
}

#[derive(Debug, Args)]
pub struct NodeAddArgs {
    pub git_url: String,
    #[arg(long = "ref")]
    pub ref_name: Option<String>,
    #[arg(long)]
    pub id: Option<String>,
}

#[derive(Debug, Args)]
pub struct NodeRemoveArgs {
    pub node_id: String,
    #[arg(long)]
    pub purge_code: bool,
}

#[derive(Debug, Args)]
pub struct TxArgs {
    #[command(subcommand)]
    pub command: TxSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum TxSubcommand {
    Run(TxRunArgs),
    Inspect(TxInspectArgs),
    Abort(TxAbortArgs),
    Promote(TxPromoteArgs),
}

#[derive(Debug, Args)]
pub struct TxRunArgs {
    pub node_id: String,
    #[arg(long)]
    pub timeout: Option<u32>,
}

#[derive(Debug, Args)]
pub struct TxInspectArgs {
    pub tx_id: String,
}

#[derive(Debug, Args)]
pub struct TxAbortArgs {
    pub tx_id: String,
    /// Confirm the owner and candidate of an old running transaction have exited
    #[arg(long)]
    pub confirm_stopped: bool,
}

#[derive(Debug, Args)]
pub struct TxPromoteArgs {
    pub tx_id: String,
    #[arg(long)]
    pub approve_core: bool,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub allow_failed_run: bool,
    #[arg(long)]
    pub keep_artifacts: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[command(subcommand)]
    pub command: UpdateSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum UpdateSubcommand {
    Run(UpdateRunArgs),
    Inspect(UpdateInspectArgs),
    Abort(TxAbortArgs),
    Promote(UpdatePromoteArgs),
    Resolve(UpdateResolveArgs),
}

#[derive(Debug, Args)]
pub struct UpdateRunArgs {
    #[arg(long)]
    pub requirements_file: Option<PathBuf>,
    #[arg(long)]
    pub timeout: Option<u32>,
}

#[derive(Debug, Args)]
pub struct UpdateInspectArgs {
    pub tx_id: String,
}

#[derive(Debug, Args)]
pub struct UpdatePromoteArgs {
    pub tx_id: String,
    #[arg(long)]
    pub approve_core: bool,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long)]
    pub allow_failed_run: bool,
    #[arg(long)]
    pub keep_artifacts: bool,
}

#[derive(Debug, Args)]
pub struct UpdateResolveArgs {
    pub tx_id: String,
    #[arg(long = "pin")]
    pub pins: Vec<String>,
    #[arg(long)]
    pub pins_file: Option<PathBuf>,
    /// Force uv to override conflicting requirements (the old default)
    #[arg(long)]
    pub override_requirements: bool,
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    pub tx_id: String,
    #[arg(long = "pin")]
    pub pins: Vec<String>,
    #[arg(long)]
    pub pins_file: Option<PathBuf>,
    /// Force uv to override conflicting requirements
    #[arg(long)]
    pub override_requirements: bool,
}

#[derive(Debug, Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum EnvSubcommand {
    Export(EnvExportArgs),
    Import(EnvImportArgs),
}

#[derive(Debug, Args)]
pub struct EnvExportArgs {
    pub output_dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct EnvImportArgs {
    pub bundle_dir: PathBuf,
    #[arg(long)]
    pub comfyui_dir: PathBuf,
    #[arg(long)]
    pub python: String,
}

#[derive(Debug, Args)]
pub struct OpArgs {
    #[command(subcommand)]
    pub command: OpSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum OpSubcommand {
    List,
    Inspect(OpInspectArgs),
    /// Restore an interrupted production operation or finish its ledger link
    Recover(OpInspectArgs),
}

#[derive(Debug, Args)]
pub struct OpInspectArgs {
    pub op_id: String,
}

#[derive(Debug, Args)]
pub struct UndoArgs {
    pub op_id: String,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Sync the locked production environment before launch
    #[arg(long)]
    pub sync: bool,
    /// Arguments to pass to ComfyUI after --
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<OsString>,
}
