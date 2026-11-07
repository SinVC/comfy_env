use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::cli::{InstallArgs, InstallTorchArgs};
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, ProjectRoot};
use crate::safety_guards;
use crate::state_ledger::OpKind;
use crate::toml_support::{self, GroupEditMode, RuntimeConfig};

const TORCH_FAMILY: [&str; 3] = ["torch", "torchvision", "torchaudio"];

pub fn cmd_install(args: InstallArgs, root: &ProjectRoot) -> AppResult<()> {
    let (config, uv) = super::configured_uv(root)?;
    let torch = toml_support::read_dependency_group(&root.pyproject_toml(), "torch")?;
    if !TORCH_FAMILY.iter().all(|name| {
        torch
            .iter()
            .any(|spec| toml_support::normalize_package_name(spec) == *name)
    }) {
        return Err(AppError::usage(
            "torch group is not ready; run 'gov install torch' first",
        ));
    }

    let requirements = args.requirements_file.unwrap_or_else(|| {
        config
            .paths
            .comfyui_dir
            .as_ref()
            .map(|dir| dir.join("requirements.txt"))
            .unwrap_or_else(|| root.as_path().join("requirements.txt"))
    });
    let specs = read_core_requirements(&requirements)?;
    ensure_uv_available()?;
    let staged = dependency_sync::create_staged_workdir(root)?;
    toml_support::rewrite_dependency_group(
        &staged.pyproject_toml(),
        "core",
        GroupEditMode::Replace,
        &specs,
    )?;
    lock_stage(&uv, &staged, root, "install")?;

    promote_install(
        root,
        &config,
        &uv,
        &staged,
        OpKind::InstallCore,
        &requirements.display().to_string(),
        false,
    )?;
    println!("Core requirements installed ({} packages).", specs.len());
    Ok(())
}

pub fn cmd_install_torch(args: InstallTorchArgs, root: &ProjectRoot) -> AppResult<()> {
    if args.index_url.trim().is_empty() || args.index_url.chars().any(char::is_whitespace) {
        return Err(AppError::usage(
            "--index-url must be a non-empty URL without whitespace",
        ));
    }
    let specs = [
        ("torch", args.torch),
        ("torchvision", args.torchvision),
        ("torchaudio", args.torchaudio),
    ]
    .into_iter()
    .map(|(name, requested)| match requested {
        Some(spec)
            if toml_support::is_exact_pin(&spec)
                && toml_support::normalize_package_name(&spec) == name =>
        {
            Ok(spec)
        }
        Some(spec) => Err(AppError::usage(format!(
            "expected an exact {name} pin (for example {name}==1.0), got '{spec}'"
        ))),
        None => Ok(name.to_string()),
    })
    .collect::<AppResult<Vec<_>>>()?;

    let (config, uv) = super::configured_uv(root)?;
    ensure_uv_available()?;
    let staged = dependency_sync::create_staged_workdir(root)?;
    toml_support::rewrite_dependency_group(
        &staged.pyproject_toml(),
        "torch",
        GroupEditMode::UpsertExact,
        &specs,
    )?;
    toml_support::configure_torch_index(&staged.pyproject_toml(), &args.index_url)?;
    lock_stage(&uv, &staged, root, "install torch")?;

    promote_install(
        root,
        &config,
        &uv,
        &staged,
        OpKind::InstallTorch,
        &args.index_url,
        true,
    )?;
    println!("Torch family installed from {}.", args.index_url);
    Ok(())
}

fn read_core_requirements(path: &Path) -> AppResult<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read requirements file {}", path.display()))?;
    let mut specs = Vec::new();
    for (offset, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let comment = line
            .char_indices()
            .find(|(i, ch)| *ch == '#' && line[..*i].ends_with(char::is_whitespace))
            .map(|(i, _)| i);
        let spec = line[..comment.unwrap_or(line.len())].trim();
        if spec.is_empty() {
            continue;
        }
        if spec.starts_with('-') || spec.starts_with('.') || spec.contains('\\') {
            return Err(AppError::usage(format!(
                "unsupported requirements directive at {}:{}: {spec}",
                path.display(),
                offset + 1
            )));
        }
        let name = toml_support::normalize_package_name(spec);
        if !toml_support::is_package_name(&name) {
            return Err(AppError::usage(format!(
                "invalid requirement at {}:{}: {spec}",
                path.display(),
                offset + 1
            )));
        }
        if !TORCH_FAMILY.contains(&name.as_str()) {
            specs.push(spec.to_string());
        }
    }
    Ok(specs)
}

fn ensure_uv_available() -> AppResult<()> {
    let version = UvClient::version().context("uv is required")?;
    if !version.success {
        return Err(AppError::usage(format!(
            "uv is unavailable: {}",
            version.summary_line()
        )));
    }
    Ok(())
}

fn lock_stage(
    uv: &UvClient,
    staged: &dependency_sync::StagedWorkdir,
    root: &ProjectRoot,
    action: &str,
) -> AppResult<()> {
    let lock = uv.lock(staged.path(), root)?;
    if !lock.success {
        return Err(AppError::runtime(format!(
            "{action} failed during lock: {}",
            lock.summary_line()
        )));
    }
    Ok(())
}

fn promote_install(
    root: &ProjectRoot,
    config: &RuntimeConfig,
    uv: &UvClient,
    staged: &dependency_sync::StagedWorkdir,
    kind: OpKind,
    reference: &str,
    torch_smoke: bool,
) -> AppResult<()> {
    let op_id = safety_guards::op_begin(kind, reference, root)?;
    let mutation = (|| -> Result<()> {
        dependency_sync::promote_workdir(staged, root)?;
        uv.sync_prod(root, config)
            .context("production sync failed")?;
        let env = root.as_path().join(&config.runtime.prod_env);
        let smoke = if torch_smoke {
            safety_guards::run_torch_smoke_test(&env, config)?
        } else {
            safety_guards::run_smoke_test(&env, config)?
        };
        if !smoke.passed {
            bail!("smoke test failed: {}", smoke.detail);
        }
        safety_guards::op_finalize(&op_id, true, None, root)
    })();
    if let Err(error) = mutation {
        return Err(super::rollback_failed_operation(
            root, &op_id, config, uv, error,
        ));
    }
    super::prune_history(root, config);
    Ok(())
}
