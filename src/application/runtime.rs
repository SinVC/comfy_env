use std::path::PathBuf;

use crate::cli::RunArgs;
use crate::dependency_sync::UvClient;
use crate::domain::{AppError, AppResult, ProjectRoot};
use crate::runtime_executor::{self, StopOutcome};
use crate::state_ledger::OpKind;
use crate::toml_support::{self, RuntimeConfig};

pub fn cmd_run(args: RunArgs, root: &ProjectRoot) -> AppResult<i32> {
    let guard = runtime_executor::try_lock_production(&root.pid_file())?;
    runtime_executor::ensure_prod_stopped(&root.pid_file())?;
    crate::safety_guards::ensure_production_recoverable(root)?;
    let config = toml_support::read_config(&root.config_toml())?
        .ok_or_else(|| AppError::usage("config.toml is required"))?;
    let comfyui_dir = configured_entry(root, &config)?;
    let env = root.as_path().join(&config.runtime.prod_env);
    if !env.is_dir() {
        return Err(AppError::runtime(format!(
            "production environment not found: {}; run gov init first",
            env.display()
        )));
    }

    if args.sync || config.run.sync_before_run {
        if !root.pyproject_toml().is_file() || !root.uv_lock().is_file() {
            return Err(AppError::usage(
                "startup sync requires pyproject.toml and uv.lock",
            ));
        }
        let python = config
            .runtime
            .python
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::usage("config.toml is missing runtime.python"))?;
        super::production::commit(
            root,
            OpKind::RuntimeSync,
            "run",
            None,
            |_| Ok(()),
            |write| write.sync(&config, &UvClient::new(python)),
        )?;
        super::prune_history(root, &config);
    }
    let mut extra_args = config.run.extra_args.to_args();
    extra_args.extend(args.args);
    Ok(runtime_executor::run_prod(
        &env,
        &comfyui_dir,
        &extra_args,
        &root.pid_file(),
        guard,
    )?)
}

pub(super) fn configured_entry(root: &ProjectRoot, config: &RuntimeConfig) -> AppResult<PathBuf> {
    let comfy = root.as_path().join(
        config
            .paths
            .comfyui_dir
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?,
    );
    let main = comfy.join("main.py");
    if !main.is_file() {
        return Err(AppError::usage(format!(
            "ComfyUI entry not found: {}",
            main.display()
        )));
    }
    Ok(comfy)
}

pub fn cmd_stop(root: &ProjectRoot) -> AppResult<()> {
    let message = match runtime_executor::stop(&root.pid_file(), 30)? {
        StopOutcome::Terminated => "ComfyUI stopped.",
        StopOutcome::AlreadyStopped => "ComfyUI process not running. Removed stale PID file.",
        StopOutcome::NotRunning => "No running process.",
    };
    println!("{message}");
    Ok(())
}
