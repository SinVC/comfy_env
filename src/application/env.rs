use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::cli::{EnvExportArgs, EnvImportArgs};
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, OpId, ProjectRoot};
use crate::fs_support;
use crate::platform::{self, PythonClient, platform_machine, sys_platform};
use crate::safety_guards;
use crate::state_ledger::{self, OpKind, PluginRecord};
use crate::toml_support::{self, RuntimeConfig};

const REQUIRED_FILES: [&str; 6] = [
    "pyproject.toml",
    "uv.lock",
    "pylock.toml",
    "state/plugins.json",
    "audit/prod-freeze.txt",
    "audit/export-summary.json",
];

#[derive(Debug, Serialize, Deserialize)]
struct BundleManifest {
    bundle_format_version: String,
    created_at: String,
    created_by: String,
    python_spec: String,
    python_version: String,
    platform_system: String,
    platform_machine: String,
    node_ids: Vec<String>,
    files: ManifestFiles,
    #[serde(default)]
    requires_python: String,
    #[serde(default)]
    sys_platform: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestFiles {
    sha256: BTreeMap<String, String>,
}

pub fn cmd_env_export(args: EnvExportArgs, root: &ProjectRoot) -> AppResult<()> {
    env_export(args, root).map_err(Into::into)
}

fn env_export(args: EnvExportArgs, root: &ProjectRoot) -> Result<()> {
    let _guard = crate::runtime_executor::try_lock_production(&root.pid_file())?;
    let (config, uv) = super::configured_uv(root)?;
    let comfy = configured_comfy(&config)?;
    ensure!(
        comfy.is_absolute(),
        "configured ComfyUI path must be absolute"
    );
    for path in [
        root.pyproject_toml(),
        root.uv_lock(),
        root.plugins_registry(),
    ] {
        ensure!(path.is_file(), "required file missing: {}", path.display());
    }
    ensure!(
        !args.output_dir.exists(),
        "bundle output already exists: {}",
        args.output_dir.display()
    );
    let plugins = state_ledger::load_plugins(root)?;
    dependency_sync::validate_relocatable_project(&toml_support::read_project(
        &root.pyproject_toml(),
    )?)?;
    let mut sources = Vec::new();
    let mut ids = BTreeSet::new();
    for plugin in &plugins {
        let source = plugin_source(comfy, plugin)?;
        ensure!(
            source.is_dir(),
            "plugin source missing: {}",
            source.display()
        );
        ensure!(
            ids.insert(plugin.id.clone()),
            "duplicate plugin ID: {}",
            plugin.id
        );
        sources.push((plugin.id.clone(), source));
    }
    let output = absolute_path(&args.output_dir)?;
    for (_, source) in &sources {
        ensure!(
            !output.starts_with(fs::canonicalize(source)?),
            "bundle output cannot be inside plugin source"
        );
    }
    // Run the real export once, before creating the destination. In particular,
    // old uv versions may support the project but lack this export format.
    let exported = uv.export_pylock(root.as_path())?;
    if !exported.success
        && exported.stderr.contains("invalid value")
        && exported.stderr.contains("pylock.toml")
    {
        bail!("uv does not support pylock.toml export; use a uv version that supports it");
    }
    ensure!(
        exported.success,
        "uv export failed: {}",
        exported.summary_line()
    );
    fs::create_dir(&args.output_dir)
        .with_context(|| format!("failed to create {}", args.output_dir.display()))?;
    let result = export_into(
        &args.output_dir,
        root,
        &config,
        &uv,
        &sources,
        ids,
        &exported.stdout,
    );
    if let Err(error) = result {
        if let Err(cleanup) = fs_support::remove_tree(&args.output_dir) {
            bail!("export failed: {error:#}; cleanup failed: {cleanup:#}");
        }
        return Err(error.into());
    }
    println!("Environment bundle exported: {}", output.display());
    Ok(())
}

fn export_into(
    output: &Path,
    root: &ProjectRoot,
    config: &RuntimeConfig,
    uv: &UvClient,
    sources: &[(String, PathBuf)],
    ids: BTreeSet<String>,
    pylock: &str,
) -> Result<()> {
    fs_support::copy_file(&root.pyproject_toml(), &output.join("pyproject.toml"))?;
    dependency_sync::copy_lock(root.as_path(), output)?;
    fs_support::copy_file(&root.plugins_registry(), &output.join("state/plugins.json"))?;
    fs_support::atomic_write(&output.join("pylock.toml"), pylock.as_bytes())?;
    for (id, source) in sources {
        fs_support::copy_tree(source, &output.join("custom_nodes").join(id), &[".git"])?;
    }
    let prod_env = root.as_path().join(&config.runtime.prod_env);
    let freeze = uv.pip_freeze(&prod_env)?;
    fs_support::atomic_write(
        &output.join("audit/prod-freeze.txt"),
        freeze.join("\n").as_bytes(),
    )?;
    let now = state_ledger::now_string()?;
    let python = config
        .runtime
        .python
        .as_deref()
        .context("runtime.python missing")?;
    let prod_python = platform::venv_python(&prod_env);
    let python_version = if prod_python.is_file() {
        PythonClient::version_full(&prod_python)?
    } else {
        python.to_string()
    };
    let summary = serde_json::json!({
        "created_at": now, "python_spec": python, "node_ids": ids,
        "node_count": ids.len(), "package_count": freeze.len(),
    });
    fs_support::atomic_write(
        &output.join("audit/export-summary.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    let project = toml_support::read_project(&root.pyproject_toml())?;
    let requires_python = project
        .get("project")
        .and_then(|item| item.get("requires-python"))
        .and_then(|item| item.as_str())
        .context("project.requires-python missing")?;
    let mut hashes = BTreeMap::new();
    collect_hashes(output, output, &mut hashes)?;
    let manifest = BundleManifest {
        bundle_format_version: "1".into(),
        created_at: now,
        created_by: "gov".into(),
        python_spec: python.into(),
        python_version,
        platform_system: platform_system().into(),
        platform_machine: platform_machine().into(),
        node_ids: ids.into_iter().collect(),
        files: ManifestFiles { sha256: hashes },
        requires_python: requires_python.into(),
        sys_platform: sys_platform().into(),
    };
    fs_support::atomic_write(
        &output.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

pub fn cmd_env_import(args: EnvImportArgs, root: &ProjectRoot) -> AppResult<()> {
    env_import(args, root).map_err(Into::into)
}

fn env_import(args: EnvImportArgs, root: &ProjectRoot) -> Result<()> {
    ensure!(
        args.bundle_dir.is_dir(),
        "bundle directory missing: {}",
        args.bundle_dir.display()
    );
    ensure!(
        args.comfyui_dir.is_absolute(),
        "--comfyui-dir must be absolute"
    );
    let manifest_path = args.bundle_dir.join("manifest.json");
    let manifest: BundleManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("bundle manifest missing: {}", manifest_path.display()))?,
    )?;
    ensure!(
        manifest.bundle_format_version == "1",
        "unsupported bundle format version"
    );
    verify_bundle(&args.bundle_dir, &manifest)?;
    let project_bytes = verified_payload(&args.bundle_dir, &manifest, "pyproject.toml")?;
    let project = std::str::from_utf8(&project_bytes)?.parse::<toml_edit::DocumentMut>()?;
    dependency_sync::validate_relocatable_project(&project)?;
    let requires = project
        .get("project")
        .and_then(|item| item.get("requires-python"))
        .and_then(|item| item.as_str())
        .context("bundle project.requires-python missing")?;
    ensure!(
        manifest.requires_python.is_empty() || manifest.requires_python == requires,
        "manifest requires-python differs from project"
    );
    ensure!(
        manifest.sys_platform.is_empty() || manifest.sys_platform == sys_platform(),
        "bundle platform mismatch: bundle={}, current={}",
        manifest.sys_platform,
        sys_platform()
    );
    ensure!(
        manifest.platform_system == platform_system(),
        "bundle platform mismatch: bundle={}, current={}",
        manifest.platform_system,
        platform_system()
    );
    ensure!(
        manifest.platform_machine == platform_machine(),
        "bundle architecture mismatch: bundle={}, current={}",
        manifest.platform_machine,
        platform_machine()
    );

    let registry = verified_payload(&args.bundle_dir, &manifest, "state/plugins.json")?;
    let plugins: Vec<PluginRecord> = serde_json::from_slice(&registry)?;
    let ids: BTreeSet<_> = plugins.iter().map(|plugin| plugin.id.clone()).collect();
    ensure!(
        ids.len() == plugins.len() && ids.iter().cloned().collect::<Vec<_>>() == manifest.node_ids,
        "bundle plugin list differs from manifest"
    );
    let groups = plugins
        .iter()
        .map(|plugin| toml_support::normalize_package_name(&plugin.group))
        .collect::<BTreeSet<_>>();
    for group in toml_support::dependency_group_names(&project)? {
        let normalized = toml_support::normalize_package_name(group);
        ensure!(
            !normalized.starts_with("node-") || groups.contains(&normalized),
            "bundle dependency group {group} has no registered node owner"
        );
    }
    for plugin in &plugins {
        super::node::validate_group_owner(&plugins, &plugin.group, &plugin.id)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let source = plugin_source(&args.comfyui_dir, plugin)?;
        ensure!(
            args.bundle_dir
                .join("custom_nodes")
                .join(&plugin.id)
                .is_dir(),
            "bundle plugin source missing: {}",
            plugin.id
        );
        ensure!(
            source.starts_with(args.comfyui_dir.join("custom_nodes")),
            "invalid plugin destination"
        );
    }
    let bundle = fs::canonicalize(&args.bundle_dir)?;
    let nodes_target = args.comfyui_dir.join("custom_nodes");
    if let Some(metadata) = fs_support::metadata_if_exists(&nodes_target)? {
        ensure!(
            metadata.is_dir() && !fs_support::is_link(&metadata),
            "target custom_nodes must be an ordinary directory"
        );
    }
    let nodes_target = fs_support::resolved_path(&nodes_target)?;
    let project_root = fs::canonicalize(root.as_path())?;
    let state_dir = fs_support::resolved_path(&root.state_dir())?;
    ensure!(
        !overlaps(&bundle, &nodes_target)
            && !project_root.starts_with(&bundle)
            && !project_root.starts_with(&nodes_target)
            && !overlaps(&nodes_target, &state_dir)
            && !overlaps(&bundle, &state_dir),
        "bundle, target custom_nodes and governance paths overlap"
    );
    let old_config = toml_support::read_config(&root.config_toml())?;
    let old_lock = root.uv_lock().exists();
    ensure!(
        !old_lock || root.pyproject_toml().is_file(),
        "previous lock requires pyproject.toml for recovery"
    );
    if let Some(config) = &old_config {
        ensure!(
            config
                .runtime
                .prod_env
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "runtime.prod_env must be a relative path inside project root"
        );
    }
    let old_env_path = root.as_path().join(
        old_config
            .as_ref()
            .map_or(Path::new(".venv-prod"), |c| c.runtime.prod_env.as_path()),
    );
    ensure!(
        old_env_path.starts_with(root.as_path()),
        "production environment must be inside project root"
    );
    ensure!(
        !old_lock
            || old_config
                .as_ref()
                .and_then(|c| c.runtime.python.as_deref())
                .is_some(),
        "previous lock requires a configured Python for recovery"
    );
    ensure!(
        old_lock || !old_env_path.exists(),
        "existing production environment has no previous lock to restore"
    );
    let resolved_env = fs_support::resolved_path(&old_env_path)?;
    ensure!(
        !overlaps(&resolved_env, &nodes_target)
            && !overlaps(&resolved_env, &bundle)
            && !overlaps(&resolved_env, &state_dir)
            && !project_root.starts_with(&resolved_env)
            && resolved_env.starts_with(&project_root),
        "production environment overlaps a protected import path"
    );

    let found = UvClient::find_python(&args.python)?;
    ensure!(
        found.success,
        "target Python lookup failed: {}",
        found.summary_line()
    );
    let interpreter = Path::new(&found.stdout);
    let python_version = PythonClient::version_full(interpreter)?;
    let uv = UvClient::new(&found.stdout);
    let check = uv.lock_check(&args.bundle_dir)?;
    ensure!(
        check.success,
        "bundle lock check failed: {}",
        check.summary_line()
    );
    let probe = tempfile::tempdir()?;
    let check = uv.check_target(&args.bundle_dir, &probe.path().join("env"))?;
    ensure!(
        check.success,
        "target Python {python_version} is incompatible with the bundle requirements or environment markers: {}",
        check.summary_line()
    );

    let nodes = safety_guards::node_swap::NodePlan::inspect(
        &nodes_target,
        plugins.iter().map(|plugin| plugin.id.clone().into()),
    )?;
    let lock = String::from_utf8(verified_payload(&args.bundle_dir, &manifest, "uv.lock")?)?;
    let lock = dependency_sync::relocate_lock_contents(&args.bundle_dir, root.as_path(), lock)?;
    let config = super::production::commit(
        root,
        OpKind::EnvImport,
        &platform::to_state_path(&bundle),
        None,
        |op| {
            safety_guards::backup_config(op, root)?;
            nodes.prepare(
                &bundle.join("custom_nodes"),
                root,
                &OpId(op.op_id.clone()),
                |name, staged| verify_staged_node(&manifest, name, staged),
            )
        },
        |write| {
            fs_support::atomic_write(&root.pyproject_toml(), &project_bytes)?;
            fs_support::atomic_write(&root.plugins_registry(), &registry)?;
            fs_support::atomic_write(&root.uv_lock(), lock.as_bytes())?;
            toml_support::set_import_config(&root.config_toml(), &args.comfyui_dir, &found.stdout)?;
            let config = toml_support::read_config(&root.config_toml())?
                .context("imported config missing")?;
            write.sync(&config, &uv)?;
            write.touch_source()?;
            nodes.publish(root, &write.op_id())?;
            write.smoke(&config, false)?;
            Ok(config)
        },
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    super::prune_history(root, &config);
    println!("Environment imported: {}", bundle.display());
    Ok(())
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}
fn verify_bundle(bundle: &Path, manifest: &BundleManifest) -> Result<()> {
    for required in REQUIRED_FILES {
        ensure!(
            manifest.files.sha256.contains_key(required),
            "manifest missing checksum for {required}"
        );
    }
    let mut actual = BTreeMap::new();
    collect_hashes(bundle, bundle, &mut actual)?;
    actual.remove("manifest.json");
    ensure!(
        actual == manifest.files.sha256,
        "bundle manifest checksum mismatch or payload file list changed"
    );
    Ok(())
}

fn verified_payload(bundle: &Path, manifest: &BundleManifest, name: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(bundle.join(name))?;
    ensure!(
        manifest.files.sha256.get(name) == Some(&fs_support::sha256_bytes(&bytes)),
        "bundle checksum mismatch for {name}"
    );
    Ok(bytes)
}

fn verify_staged_node(
    manifest: &BundleManifest,
    name: &std::ffi::OsStr,
    staged: &Path,
) -> Result<()> {
    let prefix = format!("custom_nodes/{}/", name.to_string_lossy());
    let expected = manifest
        .files
        .sha256
        .iter()
        .filter_map(|(path, hash)| {
            path.strip_prefix(&prefix)
                .map(|path| (path.to_owned(), hash.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::new();
    collect_hashes(staged, staged, &mut actual)?;
    ensure!(
        actual == expected,
        "staged node checksum mismatch for {}",
        name.to_string_lossy()
    );
    Ok(())
}

fn collect_hashes(base: &Path, dir: &Path, hashes: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        ensure!(
            entry.file_name() != ".git",
            "bundle contains Git metadata: {}",
            path.display()
        );
        let metadata = fs::symlink_metadata(&path)?;
        let kind = metadata.file_type();
        ensure!(
            !fs_support::is_link(&metadata),
            "linked bundle entry is not supported: {}",
            path.display()
        );
        if kind.is_dir() {
            collect_hashes(base, &path, hashes)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(base)?;
            hashes.insert(
                platform::to_state_path(relative),
                fs_support::sha256(&path)?,
            );
        } else if !kind.is_file() {
            bail!("special bundle entry is not supported: {}", path.display());
        }
    }
    Ok(())
}

fn plugin_source(comfy: &Path, plugin: &PluginRecord) -> Result<PathBuf> {
    ensure!(
        !plugin.id.is_empty()
            && plugin.id != "."
            && plugin.id != ".."
            && plugin
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "invalid plugin ID: {}",
        plugin.id
    );
    let rel = Path::new(&plugin.install_relpath);
    let parts: Vec<_> = rel.components().collect();
    ensure!(
        parts.len() == 2
            && parts[0] == Component::Normal("custom_nodes".as_ref())
            && parts[1] == Component::Normal(plugin.id.as_ref()),
        "invalid plugin install_relpath: {}",
        plugin.install_relpath
    );
    Ok(comfy.join(rel))
}

fn configured_comfy(config: &RuntimeConfig) -> AppResult<&Path> {
    config
        .paths
        .comfyui_dir
        .as_deref()
        .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().context("path must name a directory")?;
    Ok(fs::canonicalize(parent)?.join(name))
}

fn platform_system() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Darwin"
    } else {
        "Linux"
    }
}
