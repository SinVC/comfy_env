use std::fs;

use anyhow::{Context, ensure};

use crate::cli::InitArgs;
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, ProjectRoot};
use crate::state_ledger::OpKind;
use crate::{fs_support, platform, safety_guards, state_ledger, toml_support};

pub fn cmd_init(args: InitArgs, root: &ProjectRoot) -> AppResult<()> {
    let config_path = root.config_toml();
    let existing =
        config_path.is_file() && root.pyproject_toml().is_file() && root.uv_lock().is_file();
    if !config_path.try_exists()? && (args.comfyui_dir.is_none() || args.python.is_none()) {
        return Err(AppError::usage(
            "first init requires --comfyui-dir and --python",
        ));
    }

    let (mut config_doc, config) = toml_support::config_for_init(&config_path)?;
    if existing
        && config
            .runtime
            .python
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(AppError::usage(
            "existing project has no configured Python for recovery",
        ));
    }
    if !existing && root.as_path().join(&config.runtime.prod_env).try_exists()? {
        return Err(AppError::usage(
            "cannot initialize over an existing production environment without recoverable truth",
        ));
    }
    let comfyui_dir = args
        .comfyui_dir
        .as_deref()
        .or(config.paths.comfyui_dir.as_deref())
        .filter(|path| platform::is_absolute(path))
        .ok_or_else(|| AppError::usage("--comfyui-dir must be an absolute path"))?;
    let selector = args
        .python
        .as_deref()
        .or(config.runtime.python.as_deref())
        .ok_or_else(|| AppError::usage("--python is required"))?;
    let uv_version = UvClient::version()?;
    ensure_uv_success(&uv_version, "version")?;
    let python = UvClient::resolve_python_minor(selector)?;
    let uv = UvClient::new(&python);
    let marker = format!(
        "sys_platform == '{}' and platform_machine == '{}'",
        platform::sys_platform(),
        platform::platform_machine()
    );
    toml_support::configure_init(&mut config_doc, comfyui_dir, &python)?;
    if args.cpu {
        toml_support::configure_cpu(&mut config_doc, &config)?;
    }
    let project_path = root.pyproject_toml();
    let project_doc = toml_support::project_for_init(&project_path, &python, &marker)?;
    let stage = dependency_sync::stage_init(root, &project_doc.to_string())?;
    ensure_uv_success(&uv.lock(stage.path())?, "lock")?;

    let state = root.state_dir();
    for path in [
        root.transactions_dir(),
        state.join("logs"),
        state.join("conflicts"),
        root.work_dir(),
        root.ops_dir(),
        root.as_path().join(&config.runtime.candidate_root),
    ] {
        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create {}", path.display()))?;
    }
    let write_truth = || -> anyhow::Result<()> {
        fs_support::atomic_write(&config_path, config_doc.to_string().as_bytes())?;
        dependency_sync::promote_workdir(&stage, root)?;
        if !root.plugins_registry().try_exists()? {
            state_ledger::save_plugins(root, &[])?;
        }
        Ok(())
    };
    super::production::commit(
        root,
        if existing {
            OpKind::Reinit
        } else {
            OpKind::Init
        },
        &python,
        None,
        |record| safety_guards::backup_config(record, root),
        |write| {
            write_truth()?;
            write.sync(&config, &uv)
        },
    )?;
    super::prune_history(root, &config);
    println!("Initialized production environment for Python {python}.");
    Ok(())
}

fn ensure_uv_success(result: &crate::domain::CmdResult, command: &str) -> anyhow::Result<()> {
    ensure!(
        result.success,
        "uv {command} exited {}: {}",
        result.exit_code,
        result.summary_line()
    );
    Ok(())
}
