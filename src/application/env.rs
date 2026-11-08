use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::cli::{EnvExportArgs, EnvImportArgs};
use crate::dependency_sync::UvClient;
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
    fs::create_dir(&args.output_dir)
        .with_context(|| format!("failed to create {}", args.output_dir.display()))?;
    let result = export_into(&args.output_dir, root, &config, &uv, &sources, ids);
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
) -> Result<()> {
    fs_support::copy_file(&root.pyproject_toml(), &output.join("pyproject.toml"))?;
    fs_support::copy_file(&root.uv_lock(), &output.join("uv.lock"))?;
    fs_support::copy_file(&root.plugins_registry(), &output.join("state/plugins.json"))?;
    let exported = uv.export_pylock(root.as_path())?;
    ensure!(
        exported.success,
        "uv export failed: {}",
        exported.summary_line()
    );
    fs_support::atomic_write(&output.join("pylock.toml"), exported.stdout.as_bytes())?;
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
    let python = UvClient::resolve_python_minor(&args.python)?;
    let project = toml_support::read_project(&args.bundle_dir.join("pyproject.toml"))?;
    let requires = project
        .get("project")
        .and_then(|item| item.get("requires-python"))
        .and_then(|item| item.as_str())
        .context("bundle project.requires-python missing")?;
    ensure!(
        python_satisfies(&python, requires)?,
        "bundle requires-python is {requires}, but target python resolves to {python}"
    );
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
    verify_environment_markers(&project)?;
    verify_bundle(&args.bundle_dir, &manifest)?;
    let plugins: Vec<PluginRecord> =
        serde_json::from_slice(&fs::read(args.bundle_dir.join("state/plugins.json"))?)?;
    let ids: BTreeSet<_> = plugins.iter().map(|plugin| plugin.id.clone()).collect();
    ensure!(
        ids.len() == plugins.len() && ids.iter().cloned().collect::<Vec<_>>() == manifest.node_ids,
        "bundle plugin list differs from manifest"
    );
    for plugin in &plugins {
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
    let nodes_target = args.comfyui_dir.join("custom_nodes");
    if let Ok(metadata) = fs::symlink_metadata(&nodes_target) {
        ensure!(
            !metadata.file_type().is_symlink(),
            "target custom_nodes cannot be a link"
        );
    }
    let bundle = fs::canonicalize(&args.bundle_dir)?;
    let nodes_guard = if nodes_target.exists() {
        fs::canonicalize(&nodes_target)?
    } else {
        nodes_target.clone()
    };
    ensure!(
        !bundle.starts_with(&nodes_guard),
        "bundle cannot reside in target custom_nodes"
    );
    let uv = UvClient::new(&python);
    let check = uv.lock_check(&args.bundle_dir)?;
    ensure!(
        check.success,
        "bundle lock check failed: {}",
        check.summary_line()
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
    let op = safety_guards::op_begin(OpKind::EnvImport, &platform::to_state_path(&bundle), root)?;
    let result = (|| -> Result<RuntimeConfig> {
        safety_guards::backup_import_config(&op, root)?;
        safety_guards::backup_import_nodes(&op, &args.comfyui_dir.join("custom_nodes"), root)?;
        for (from, to) in [
            (
                args.bundle_dir.join("pyproject.toml"),
                root.pyproject_toml(),
            ),
            (args.bundle_dir.join("uv.lock"), root.uv_lock()),
            (
                args.bundle_dir.join("state/plugins.json"),
                root.plugins_registry(),
            ),
        ] {
            fs_support::atomic_write(&to, &fs::read(from)?)?;
        }
        toml_support::set_import_config(&root.config_toml(), &args.comfyui_dir, &python)?;
        let config =
            toml_support::read_config(&root.config_toml())?.context("imported config missing")?;
        uv.sync_prod(root, &config)
            .context("production sync failed")?;
        sync_custom_nodes(&args.bundle_dir, &args.comfyui_dir, &plugins)?;
        let smoke =
            safety_guards::run_smoke_test(&root.as_path().join(&config.runtime.prod_env), &config)?;
        ensure!(smoke.passed, "smoke test failed: {}", smoke.detail);
        safety_guards::op_finalize(&op, true, None, root)?;
        Ok(config)
    })();
    let config = match result {
        Ok(config) => config,
        Err(error) => bail!(
            "{}",
            rollback_import(
                root,
                &op,
                old_config.as_ref(),
                old_lock,
                &old_env_path,
                error
            )
        ),
    };
    super::prune_history(root, &config);
    println!("Environment imported: {}", bundle.display());
    Ok(())
}

fn rollback_import(
    root: &ProjectRoot,
    op: &OpId,
    old_config: Option<&RuntimeConfig>,
    old_lock: bool,
    old_env: &Path,
    error: anyhow::Error,
) -> AppError {
    let mut message = format!("env import failed: {error:#}");
    if let Err(restore) = safety_guards::op_restore(op, root) {
        message.push_str(&format!("; restore failed: {restore:#}"));
    }
    if old_lock {
        if let Some(config) = old_config {
            let uv = UvClient::new(config.runtime.python.as_deref().unwrap_or_default());
            if let Err(sync) = uv.sync_prod(root, config) {
                message.push_str(&format!("; recovery sync failed: {sync:#}"));
            }
        } else {
            message.push_str("; recovery sync unavailable: previous config missing");
        }
    } else if old_env.exists() {
        if let Err(remove) = fs_support::remove_tree(old_env) {
            message.push_str(&format!("; failed to remove new environment: {remove:#}"));
        }
    }
    if let Err(record) = safety_guards::op_finalize(op, false, Some(message.clone()), root) {
        message.push_str(&format!("; failed to record operation: {record:#}"));
    }
    AppError::runtime(message)
}

fn sync_custom_nodes(bundle: &Path, comfy: &Path, plugins: &[PluginRecord]) -> Result<()> {
    let target = comfy.join("custom_nodes");
    fs::create_dir_all(&target)?;
    for entry in fs::read_dir(&target)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            eprintln!(
                "warning: preserving linked custom_nodes entry: {}",
                entry.path().display()
            );
            continue;
        }
        fs_support::remove_tree(&entry.path())?;
    }
    for plugin in plugins {
        let destination = target.join(&plugin.id);
        if destination.symlink_metadata().is_ok() {
            eprintln!(
                "warning: preserving linked custom_nodes entry: {}",
                destination.display()
            );
            continue;
        }
        fs_support::copy_tree(
            &bundle.join("custom_nodes").join(&plugin.id),
            &destination,
            &[],
        )?;
    }
    Ok(())
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
    ensure!(
        actual == manifest.files.sha256,
        "bundle manifest checksum mismatch or payload file list changed"
    );
    Ok(())
}

fn collect_hashes(base: &Path, dir: &Path, hashes: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "linked bundle entry is not supported: {}",
            path.display()
        );
        if kind.is_dir() {
            collect_hashes(base, &path, hashes)?;
        } else if kind.is_file() && path != base.join("manifest.json") {
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

fn python_satisfies(python: &str, requires: &str) -> Result<bool> {
    let mut parts = python.split('.');
    let major: u32 = parts.next().context("invalid Python minor")?.parse()?;
    let minor: u32 = parts.next().context("invalid Python minor")?.parse()?;
    let version = (major, minor);
    for clause in requires.split(',') {
        let clause = clause.trim();
        let (op, raw) = ["==", ">=", "<=", "~=", ">", "<"]
            .into_iter()
            .find_map(|op| clause.strip_prefix(op).map(|raw| (op, raw.trim())))
            .context("unsupported requires-python constraint")?;
        let numeric = raw.trim_end_matches(".*");
        let values: Vec<u32> = numeric
            .split('.')
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()?;
        ensure!(
            values.len() >= 2,
            "unsupported requires-python constraint: {clause}"
        );
        let bound = (values[0], values[1]);
        let matches = match op {
            "==" => version == bound,
            ">=" => version >= bound,
            ">" => version > bound,
            "<=" => version <= bound,
            "<" => version < bound,
            "~=" => version >= bound && version.0 == bound.0,
            _ => false,
        };
        if !matches {
            return Ok(false);
        }
    }
    Ok(true)
}

fn verify_environment_markers(project: &toml_edit::DocumentMut) -> Result<()> {
    let markers = project
        .get("tool")
        .and_then(|item| item.get("uv"))
        .and_then(|item| item.get("environments"));
    let Some(markers) = markers else {
        return Ok(());
    };
    let values = markers
        .as_array()
        .context("tool.uv.environments must be an array")?;
    if values.is_empty() {
        return Ok(());
    }
    ensure!(
        values
            .iter()
            .any(|item| item.as_str().is_some_and(marker_matches_host)),
        "bundle platform incompatible: expected sys_platform={} and platform_machine={}",
        sys_platform(),
        platform_machine()
    );
    Ok(())
}

fn marker_matches_host(marker: &str) -> bool {
    marker.split(" or ").any(|alternative| {
        alternative.split(" and ").all(|condition| {
            let Some((name, value)) = condition.trim().split_once("==") else {
                return false;
            };
            let value = value.trim().trim_matches(['\'', '"']);
            match name.trim() {
                "sys_platform" => value == sys_platform(),
                "platform_machine" => value == platform_machine(),
                _ => false,
            }
        })
    })
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
