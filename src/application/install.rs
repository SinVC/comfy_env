use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::cli::{InstallArgs, InstallTorchArgs};
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, ProjectRoot, TORCH_FAMILY};
use crate::state_ledger::OpKind;
use crate::toml_support::{self, GroupEditMode, RuntimeConfig};

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

    let requirements = match args.requirements_file {
        Some(path) => path,
        None => config
            .paths
            .comfyui_dir
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?
            .join("requirements.txt"),
    };
    if !requirements.is_file() {
        return Err(AppError::usage(format!(
            "requirements file missing: {}",
            requirements.display()
        )));
    }
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

pub(crate) fn read_core_requirements(path: &Path) -> AppResult<Vec<String>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read requirements file {}", path.display()))?;
    let mut specs = Vec::new();
    for (offset, raw) in text.lines().enumerate() {
        let spec = requirement_text(raw);
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

pub(super) fn requirement_text(line: &str) -> &str {
    let line = line.trim();
    if line.starts_with('#') {
        return "";
    }
    let comment = line
        .char_indices()
        .find(|(i, ch)| *ch == '#' && line[..*i].ends_with(char::is_whitespace))
        .map(|(i, _)| i);
    line[..comment.unwrap_or(line.len())].trim_end()
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
    super::production::commit(
        root,
        kind,
        reference,
        None,
        |_| Ok(()),
        |write| {
            dependency_sync::promote_workdir(staged, root)?;
            write.sync(config, uv)?;
            write.smoke(config, torch_smoke)
        },
    )?;
    super::prune_history(root, config);
    Ok(())
}
