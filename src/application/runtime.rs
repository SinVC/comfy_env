use anyhow::Context;

use crate::cli::RunArgs;
use crate::dependency_sync::UvClient;
use crate::domain::{AppError, AppResult, ProjectRoot};
use crate::runtime_executor::{self, StopOutcome};
use crate::toml_support;

pub fn cmd_run(args: RunArgs, root: &ProjectRoot) -> AppResult<i32> {
    let config = toml_support::read_config(&root.config_toml())?
        .ok_or_else(|| AppError::usage("config.toml is required"))?;
    let comfyui_dir = root.as_path().join(
        config
            .paths
            .comfyui_dir
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?,
    );
    let env = root.as_path().join(&config.runtime.prod_env);
    if !env.is_dir() {
        return Err(AppError::runtime(format!(
            "production environment not found: {}; run gov init first",
            env.display()
        )));
    }
    let main = comfyui_dir.join("main.py");
    if !main.is_file() {
        return Err(AppError::runtime(format!(
            "ComfyUI entry not found: {}",
            main.display()
        )));
    }

    let sync = args.sync || config.run.sync_before_run;
    let mut extra_args = config.run.extra_args.to_args();
    extra_args.extend(args.args);
    Ok(runtime_executor::run_prod(
        &env,
        &comfyui_dir,
        &extra_args,
        &root.pid_file(),
        || {
            if sync {
                let python = config
                    .runtime
                    .python
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .context("config.toml is missing runtime.python")?;
                UvClient::new(python).sync_prod(root, &config)?;
            }
            Ok(())
        },
    )?)
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
