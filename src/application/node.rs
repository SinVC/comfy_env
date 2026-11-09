use crate::cli::{NodeAddArgs, NodeRemoveArgs};
use crate::dependency_sync;
use crate::domain::{AppError, AppResult, NodeId, OpId, ProjectRoot};
use crate::safety_guards;
use crate::source_integration::{self, GitClient};
use crate::state_ledger::{self, OpKind, PluginRecord};
use crate::toml_support;

pub fn cmd_node_add(args: NodeAddArgs, root: &ProjectRoot) -> AppResult<()> {
    let config = super::configured(root)?;
    let comfy = config
        .paths
        .comfyui_dir
        .as_deref()
        .filter(|path| !path.as_os_str().is_empty())
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
    let group = format!("node-{}", toml_support::normalize_package_name(&id));
    validate_group_owner(&plugins, &group, &id)?;
    let project = toml_support::read_project(&root.pyproject_toml())?;
    if let Some(existing) = toml_support::existing_group_name(&project, &group)? {
        return Err(AppError::usage(format!(
            "dependency group {existing} already exists without a registered owner"
        )));
    }
    let target = source_integration::plugin_install_path(comfy, &NodeId(id.clone()));
    if crate::fs_support::metadata_if_exists(&target)?.is_some() {
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
    let nodes = comfy.join("custom_nodes");
    let stage_parent = if crate::fs_support::metadata_if_exists(&nodes)?
        .is_some_and(|metadata| crate::fs_support::is_link(&metadata))
    {
        nodes
            .canonicalize()?
            .parent()
            .ok_or_else(|| AppError::usage("linked custom_nodes has no parent for staging"))?
            .to_path_buf()
    } else {
        comfy.to_path_buf()
    };
    let stage = tempfile::Builder::new()
        .prefix(".gov-node-")
        .tempdir_in(stage_parent)?;
    let checkout = stage.path().join(&id);
    let result = GitClient::clone(&args.git_url, &checkout)?;
    if !result.success {
        return Err(AppError::runtime(format!(
            "git clone failed: {}",
            result.summary_line()
        )));
    }
    if let Some(reference) = &args.ref_name {
        let result = GitClient::checkout(&checkout, reference)?;
        if !result.success {
            return Err(AppError::runtime(format!(
                "git checkout failed: {}",
                result.summary_line()
            )));
        }
    }
    let now = state_ledger::now_string()?;
    plugins.push(PluginRecord {
        id: id.clone(),
        enabled: true,
        git_url: args.git_url,
        git_ref: args.ref_name.unwrap_or_else(|| "HEAD".to_string()),
        install_relpath: format!("custom_nodes/{id}"),
        group,
        managed_deps: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    });
    super::production::commit(
        root,
        OpKind::NodeAdd,
        &id,
        None,
        |op| safety_guards::backup_absent_node(&OpId(op.op_id.clone()), &target, root),
        |write| {
            std::fs::create_dir_all(&nodes)?;
            write.touch_source()?;
            crate::fs_support::move_entry(&checkout, &target)?;
            state_ledger::save_plugins(root, &plugins)
        },
    )?;
    super::prune_history(root, &config);
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
    validate_group_owner(&plugins, &plugin.group, &plugin.id)?;
    let comfy = config
        .paths
        .comfyui_dir
        .as_ref()
        .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?;
    let source = source_integration::plugin_install_path(comfy, &NodeId(plugin.id.clone()));
    if args.purge_code && !source.is_dir() {
        return Err(AppError::usage(format!(
            "node source missing: {}",
            source.display()
        )));
    }
    let staged = dependency_sync::create_staged_workdir(root)?;
    let project = toml_support::read_project(&staged.pyproject_toml())?;
    if let Some(group) = toml_support::existing_group_name(&project, &plugin.group)? {
        toml_support::remove_dependency_group(&staged.pyproject_toml(), &group)?;
    }
    let lock = uv.lock(staged.path(), root)?;
    if !lock.success {
        return Err(AppError::runtime(format!(
            "node remove lock failed: {}",
            lock.summary_line()
        )));
    }
    super::production::commit(
        root,
        OpKind::NodeRemove,
        &plugin.id,
        None,
        |op| {
            if args.purge_code {
                safety_guards::backup_custom_node(
                    &crate::domain::OpId(op.op_id.clone()),
                    &source,
                    root,
                )?;
            }
            Ok(())
        },
        |write| {
            dependency_sync::promote_workdir(&staged, root)?;
            write.sync(&config, &uv)?;
            write.smoke(&config, false)?;
            plugins.retain(|entry| entry.id != plugin.id);
            state_ledger::save_plugins(root, &plugins)?;
            if args.purge_code {
                write.touch_source()?;
                source_integration::remove_plugin_source(&source)?;
            }
            Ok(())
        },
    )?;
    super::prune_history(root, &config);
    println!("Node removed: {}", args.node_id);
    Ok(())
}

pub(crate) fn validate_group_owner(
    plugins: &[PluginRecord],
    group: &str,
    id: &str,
) -> AppResult<()> {
    let name = toml_support::normalize_package_name(group);
    if name.is_empty() || matches!(name.as_str(), "core" | "torch" | "overrides") {
        return Err(AppError::usage(format!(
            "node {id} has an invalid or reserved dependency group: {group}"
        )));
    }
    if let Some(other) = plugins.iter().find(|plugin| {
        plugin.id != id && toml_support::normalize_package_name(&plugin.group) == name
    }) {
        return Err(AppError::usage(format!(
            "dependency group {name} is shared by nodes {id} and {}",
            other.id
        )));
    }
    Ok(())
}

pub(crate) fn validate_node_id(id: &str) -> AppResult<()> {
    if !toml_support::is_package_name(id) || id == "." || id == ".." {
        return Err(AppError::usage(format!("invalid node ID: {id}")));
    }
    Ok(())
}
