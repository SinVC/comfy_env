pub mod env;
pub mod init;
pub mod install;
pub mod node;
pub mod ops;
pub mod pin;
mod production;
pub mod resolve;
pub mod runtime;
pub mod status;
pub mod tx;
pub mod undo;
pub mod update;

use crate::cli::{
    Cli, Commands, EnvSubcommand, InstallSubcommand, NodeSubcommand, OpSubcommand, PinSubcommand,
    TxSubcommand, UpdateSubcommand,
};
use crate::dependency_sync::UvClient;
use crate::domain::{AppError, AppResult, OpId, ProjectRoot};
use crate::toml_support::{self, RuntimeConfig};

pub fn dispatch(cli: Cli) -> AppResult<i32> {
    let root = ProjectRoot::discover()?;
    if matches!(
        &cli.command,
        Commands::Init(_)
            | Commands::Install(_)
            | Commands::Undo(_)
            | Commands::Node(_)
            | Commands::Pin(crate::cli::PinArgs {
                command: PinSubcommand::Add(_) | PinSubcommand::Remove(_)
            })
            | Commands::Tx(crate::cli::TxArgs {
                command: TxSubcommand::Promote(_)
            })
            | Commands::Update(crate::cli::UpdateArgs {
                command: UpdateSubcommand::Promote(_)
            })
            | Commands::Env(crate::cli::EnvArgs {
                command: EnvSubcommand::Import(_)
            })
    ) {
        crate::runtime_executor::ensure_prod_stopped(&root.pid_file())?;
    }

    match cli.command {
        Commands::Init(args) => init::cmd_init(args, &root),
        Commands::Install(args) => match args.command {
            Some(InstallSubcommand::Torch(torch_args)) => {
                install::cmd_install_torch(torch_args, &root)
            }
            None => install::cmd_install(args, &root),
        },
        Commands::Pin(args) => match args.command {
            PinSubcommand::Add(add_args) => pin::cmd_pin_add(add_args, &root),
            PinSubcommand::List(list_args) => pin::cmd_pin_list(list_args, &root),
            PinSubcommand::Remove(remove_args) => pin::cmd_pin_remove(remove_args, &root),
        },
        Commands::Node(args) => match args.command {
            NodeSubcommand::Add(add_args) => node::cmd_node_add(add_args, &root),
            NodeSubcommand::Remove(remove_args) => node::cmd_node_remove(remove_args, &root),
        },
        Commands::Tx(args) => match args.command {
            TxSubcommand::Run(run_args) => tx::cmd_tx_run(run_args, &root),
            TxSubcommand::Inspect(inspect_args) => tx::cmd_tx_inspect(inspect_args, &root),
            TxSubcommand::Abort(abort_args) => tx::cmd_tx_abort(abort_args, &root),
            TxSubcommand::Promote(promote_args) => tx::cmd_tx_promote(promote_args, &root),
        },
        Commands::Update(args) => match args.command {
            UpdateSubcommand::Run(run_args) => update::cmd_update_run(run_args, &root),
            UpdateSubcommand::Inspect(inspect_args) => {
                update::cmd_update_inspect(inspect_args, &root)
            }
            UpdateSubcommand::Abort(abort_args) => update::cmd_update_abort(abort_args, &root),
            UpdateSubcommand::Promote(promote_args) => {
                update::cmd_update_promote(promote_args, &root)
            }
            UpdateSubcommand::Resolve(resolve_args) => {
                update::cmd_update_resolve(resolve_args, &root)
            }
        },
        Commands::Resolve(args) => resolve::cmd_resolve(args, &root),
        Commands::Env(args) => match args.command {
            EnvSubcommand::Export(export_args) => env::cmd_env_export(export_args, &root),
            EnvSubcommand::Import(import_args) => env::cmd_env_import(import_args, &root),
        },
        Commands::Op(args) => match args.command {
            OpSubcommand::List => ops::cmd_op_list(&root),
            OpSubcommand::Inspect(inspect_args) => ops::cmd_op_inspect(inspect_args, &root),
            OpSubcommand::Recover(recover_args) => {
                production::recover(&root, &OpId(recover_args.op_id))
            }
        },
        Commands::Undo(args) => undo::cmd_undo(args, &root),
        Commands::Run(args) => return runtime::cmd_run(args, &root),
        Commands::Stop => runtime::cmd_stop(&root),
        Commands::Status => status::cmd_status(&root),
    }
    .map(|()| 0)
}

fn configured_uv(root: &ProjectRoot) -> AppResult<(RuntimeConfig, UvClient)> {
    let config = configured(root)?;
    let python = config
        .runtime
        .python
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::usage("config.toml is missing runtime.python"))?;
    let uv = UvClient::new(python);
    Ok((config, uv))
}

fn configured(root: &ProjectRoot) -> AppResult<RuntimeConfig> {
    toml_support::read_config(&root.config_toml())?
        .ok_or_else(|| AppError::usage("config.toml is required"))
}

fn prune_history(root: &ProjectRoot, config: &RuntimeConfig) {
    if let Err(error) =
        crate::state_ledger::prune_operations(root, config.ops.retention_count.unwrap_or(100))
    {
        eprintln!("warning: failed to prune operation history: {error:#}");
    }
}
