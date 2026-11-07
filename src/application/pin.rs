use anyhow::{Context, Result, bail};

use crate::cli::PinMutateArgs;
use crate::dependency_sync;
use crate::domain::{AppError, AppResult, ProjectRoot};
use crate::safety_guards;
use crate::state_ledger::OpKind;
use crate::toml_support::{self, GroupEditMode};

const TORCH_FAMILY: [&str; 3] = ["torch", "torchvision", "torchaudio"];
const RECOMMENDED_PINS: [&str; 2] = ["numpy", "transformers"];

pub fn cmd_pin_add(args: PinMutateArgs, root: &ProjectRoot) -> AppResult<()> {
    for spec in &args.specs {
        if !toml_support::is_exact_pin(spec) {
            return Err(AppError::usage(format!("invalid pin format: {spec}")));
        }
        let name = toml_support::normalize_package_name(spec);
        reject_torch_family(&name)?;
        if !RECOMMENDED_PINS.contains(&name.as_str()) {
            eprintln!("WARNING: pinning non-recommended package: {spec}");
        }
    }
    apply_pins(
        root,
        &args.specs,
        OpKind::PinAdd,
        GroupEditMode::UpsertExact,
        "pin add",
    )?;
    println!("Pins added.");
    Ok(())
}

pub fn cmd_pin_list(root: &ProjectRoot) -> AppResult<()> {
    let pins = toml_support::read_dependency_group(&root.pyproject_toml(), "overrides")?;
    if pins.is_empty() {
        println!("No pins in overrides group.");
        return Ok(());
    }

    for pin in pins {
        println!("{pin}");
    }
    Ok(())
}

pub fn cmd_pin_remove(args: PinMutateArgs, root: &ProjectRoot) -> AppResult<()> {
    let packages = &args.specs;
    for package in packages {
        if !toml_support::is_package_name(package) {
            return Err(AppError::usage(format!("invalid package name: {package}")));
        }
        reject_torch_family(&toml_support::normalize_package_name(package))?;
    }
    apply_pins(
        root,
        packages,
        OpKind::PinRemove,
        GroupEditMode::RemoveNames,
        "pin remove",
    )?;
    println!("Pins removed.");
    Ok(())
}

fn apply_pins(
    root: &ProjectRoot,
    specs: &[String],
    kind: OpKind,
    mode: GroupEditMode,
    action: &str,
) -> AppResult<()> {
    let (config, uv) = super::configured_uv(root)?;
    let staged = dependency_sync::create_staged_workdir(root)?;
    toml_support::rewrite_dependency_group(&staged.pyproject_toml(), "overrides", mode, specs)?;
    let lock = uv.lock(staged.path(), root)?;
    if !lock.success {
        return Err(AppError::runtime(format!(
            "{action} failed during lock: {}",
            lock.summary_line()
        )));
    }

    let op_id = safety_guards::op_begin(kind, &specs.join(", "), root)?;
    let mutation = (|| -> Result<()> {
        dependency_sync::promote_workdir(&staged, root)?;
        uv.sync_prod(root, &config)
            .with_context(|| format!("prod sync failed during {action}"))?;
        let smoke =
            safety_guards::run_smoke_test(&root.as_path().join(&config.runtime.prod_env), &config)?;
        if !smoke.passed {
            bail!("smoke test failed during {action}: {}", smoke.detail);
        }
        safety_guards::op_finalize(&op_id, true, None, root)
    })();
    if let Err(error) = mutation {
        return Err(super::rollback_failed_operation(
            root, &op_id, &config, &uv, error,
        ));
    }
    super::prune_history(root, &config);
    Ok(())
}

fn reject_torch_family(name: &str) -> AppResult<()> {
    if TORCH_FAMILY.contains(&name) {
        return Err(AppError::usage(
            "torch-family packages are managed by 'gov install torch'",
        ));
    }
    Ok(())
}
