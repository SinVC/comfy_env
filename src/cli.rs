use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
    Init(InitArgs),
    Install(InstallArgs),
    Pin(PinArgs),
    Node(NodeArgs),
    Tx(TxArgs),
    Update(UpdateArgs),
    Resolve(ResolveArgs),
    Env(EnvArgs),
    Op(OpArgs),
    Undo(UndoArgs),
    Run(RunArgs),
    Stop,
    Status,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long)]
    pub comfyui_dir: Option<PathBuf>,
    #[arg(long)]
    pub python: Option<String>,
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
    List,
    Remove(PinMutateArgs),
}

#[derive(Debug, Args)]
pub struct PinMutateArgs {
    #[arg(required = true)]
    pub specs: Vec<String>,
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
    Abort(TxInspectArgs),
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
    Abort(UpdateInspectArgs),
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
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    pub tx_id: String,
    #[arg(long = "pin")]
    pub pins: Vec<String>,
    #[arg(long)]
    pub pins_file: Option<PathBuf>,
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
    #[arg(long)]
    pub sync: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}
