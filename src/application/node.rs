use anyhow::{Context, Result, bail};

use crate::cli::{NodeAddArgs, NodeRemoveArgs};
use crate::dependency_sync;
use crate::domain::{AppError, AppResult, ProjectRoot};
use crate::safety_guards;
use crate::source_integration::{self, GitClient};
use crate::state_ledger::{self, OpKind, PluginRecord};
use crate::toml_support;

pub fn cmd_node_add(args: NodeAddArgs, root: &ProjectRoot) -> AppResult<()> {
    let (config, _) = super::configured_uv(root)?;
    let comfy = config
        .paths
        .comfyui_dir
        .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?;
    if !comfy.is_dir() {
        return Err(AppError::usage(format!(
            "ComfyUI directory does not exist: {}",
            comfy.display()
        )));
    }
    let id = args
        .id
        .unwrap_or_else(|| source_integration::node_id_from_url(&args.git_url));
    validate_node_id(&id)?;
    if args
        .ref_name
        .as_deref()
        .is_some_and(|r| r.is_empty() || r.starts_with('-'))
    {
        return Err(AppError::usage("invalid Git ref"));
    }
    let mut plugins = state_ledger::load_plugins(root)?;
    if plugins.iter().any(|plugin| plugin.id == id) {
        return Err(AppError::usage(format!("node already registered: {id}")));
    }
    let target = source_integration::plugin_install_path(&comfy, &id);
    if target.exists() {
        return Err(AppError::usage(format!(
            "node source already exists: {}",
            target.display()
        )));
    }
    let git = GitClient::version()?;
    if !git.success {
        return Err(AppError::runtime(format!(
            "git unavailable: {}",
            git.summary_line()
        )));
    }
    std::fs::create_dir_all(comfy.join("custom_nodes"))?;
    let cloned = (|| -> Result<()> {
        let result = GitClient::clone(&args.git_url, &target)?;
        if !result.success {
            bail!("git clone failed: {}", result.summary_line());
        }
        if let Some(reference) = &args.ref_name {
            let result = GitClient::checkout(&target, reference)?;
            if !result.success {
                bail!("git checkout failed: {}", result.summary_line());
            }
        }
        let now = state_ledger::now_string()?;
        plugins.push(PluginRecord {
            id: id.clone(),
            enabled: true,
            git_url: args.git_url,
            git_ref: args.ref_name.unwrap_or_else(|| "HEAD".to_string()),
            install_relpath: format!("custom_nodes/{id}"),
            group: format!("node-{}", toml_support::normalize_package_name(&id)),
            managed_deps: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        });
        state_ledger::save_plugins(root, &plugins)
    })();
    if let Err(error) = cloned {
        if target.exists() {
            if let Err(cleanup) = source_integration::remove_plugin_source(&target) {
                return Err(AppError::runtime(format!(
                    "{error:#}; clone cleanup failed: {cleanup:#}"
                )));
            }
        }
        return Err(error.into());
    }
    println!("Node registered: {id}");
    Ok(())
}

pub fn cmd_node_remove(args: NodeRemoveArgs, root: &ProjectRoot) -> AppResult<()> {
    validate_node_id(&args.node_id)?;
    let (config, uv) = super::configured_uv(root)?;
    let mut plugins = state_ledger::load_plugins(root)?;
    let plugin = plugins
        .iter()
        .find(|plugin| plugin.id == args.node_id)
        .cloned()
        .ok_or_else(|| AppError::usage(format!("node not registered: {}", args.node_id)))?;
    let comfy = config
        .paths
        .comfyui_dir
        .as_ref()
        .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?;
    let source = source_integration::plugin_install_path(comfy, &plugin.id);
    if args.purge_code && !source.is_dir() {
        return Err(AppError::usage(format!(
            "node source missing: {}",
            source.display()
        )));
    }
    let staged = dependency_sync::create_staged_workdir(root)?;
    toml_support::remove_dependency_group(&staged.pyproject_toml(), &plugin.group)?;
    let lock = uv.lock(staged.path(), root)?;
    if !lock.success {
        return Err(AppError::runtime(format!(
            "node remove lock failed: {}",
            lock.summary_line()
        )));
    }
    let op = safety_guards::op_begin(OpKind::NodeRemove, &plugin.id, root)?;
    let result = (|| -> Result<()> {
        if args.purge_code {
            safety_guards::backup_custom_node(&op, &source, root)?;
        }
        dependency_sync::promote_workdir(&staged, root)?;
        uv.sync_prod(root, &config).context("prod sync failed")?;
        let smoke =
            safety_guards::run_smoke_test(&root.as_path().join(&config.runtime.prod_env), &config)?;
        if !smoke.passed {
            bail!("smoke test failed: {}", smoke.detail);
        }
        plugins.retain(|entry| entry.id != plugin.id);
        state_ledger::save_plugins(root, &plugins)?;
        if args.purge_code {
            source_integration::remove_plugin_source(&source)?;
        }
        safety_guards::op_finalize(&op, true, None, root)
    })();
    if let Err(error) = result {
        return Err(super::rollback_failed_operation(
            root, &op, &config, &uv, error,
        ));
    }
    super::prune_history(root, &config);
    println!("Node removed: {}", args.node_id);
    Ok(())
}

pub(crate) fn validate_node_id(id: &str) -> AppResult<()> {
    if !toml_support::is_package_name(id) || id == "." || id == ".." {
        return Err(AppError::usage(format!("invalid node ID: {id}")));
    }
    Ok(())
}
