use anyhow::Context;

use crate::cli::{PinListArgs, PinMutateArgs};
use crate::dependency_sync;
use crate::domain::{AppError, AppResult, ProjectRoot, TORCH_FAMILY};
use crate::state_ledger::{OpKind, TransactionRecord};
use crate::toml_support::{self, unscoped_exact_pin};

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
    apply_pins(root, &args, OpKind::PinAdd, false, "pin add")?;
    println!("Pins added.");
    Ok(())
}

pub fn cmd_pin_list(args: PinListArgs, root: &ProjectRoot) -> AppResult<()> {
    let project = toml_support::read_project(&root.pyproject_toml())?;
    let pins = toml_support::dependency_group(&project, "overrides")?;
    let forced = toml_support::uv_overrides(&project)?;
    if pins.is_empty() && forced.is_empty() {
        println!("No pins in overrides group.");
        return Ok(());
    }

    for pin in &pins {
        let kind = match unscoped_exact_pin(pin) {
            Some(exact)
                if forced
                    .iter()
                    .any(|spec| unscoped_exact_pin(spec).as_ref() == Some(&exact)) =>
            {
                "forced"
            }
            Some((name, _))
                if forced.iter().any(|spec| {
                    unscoped_exact_pin(spec).is_some_and(|(other, _)| other == name)
                }) =>
            {
                "inconsistent"
            }
            _ => "constraint",
        };
        print_pin(pin, args.details, kind);
    }
    for spec in &forced {
        let kind = match unscoped_exact_pin(spec) {
            Some(exact)
                if pins
                    .iter()
                    .any(|pin| unscoped_exact_pin(pin).as_ref() == Some(&exact)) =>
            {
                continue;
            }
            Some((name, _))
                if pins
                    .iter()
                    .any(|pin| unscoped_exact_pin(pin).is_some_and(|(other, _)| other == name)) =>
            {
                "inconsistent override"
            }
            Some(_) => "forced only",
            None => "manual override",
        };
        print_pin(spec, args.details, kind);
    }
    Ok(())
}

fn print_pin(spec: &str, details: bool, kind: &str) {
    if details {
        println!("{spec}\t{kind}");
    } else {
        println!("{spec}");
    }
}

pub fn cmd_pin_remove(args: PinMutateArgs, root: &ProjectRoot) -> AppResult<()> {
    let packages = &args.specs;
    for package in packages {
        if !toml_support::is_package_name(package) {
            return Err(AppError::usage(format!("invalid package name: {package}")));
        }
        reject_torch_family(&toml_support::normalize_package_name(package))?;
    }
    apply_pins(root, &args, OpKind::PinRemove, true, "pin remove")?;
    println!("Pins removed.");
    Ok(())
}

fn apply_pins(
    root: &ProjectRoot,
    args: &PinMutateArgs,
    kind: OpKind,
    remove: bool,
    action: &str,
) -> AppResult<()> {
    let (config, uv) = super::configured_uv(root)?;
    let staged = dependency_sync::create_staged_workdir(root)?;
    let specs = &args.specs;
    let force = if args.override_requirements {
        specs
            .iter()
            .map(|spec| toml_support::normalize_package_name(spec))
            .collect()
    } else {
        Vec::new()
    };
    toml_support::edit_pins(&staged.pyproject_toml(), specs, &force, remove)?;
    let lock = uv.lock(staged.path())?;
    if !lock.success {
        return Err(AppError::runtime(format!(
            "{action} failed during lock: {}",
            lock.summary_line()
        )));
    }

    super::production::commit(
        root,
        kind,
        &specs.join(", "),
        None,
        |_| Ok(()),
        |write| {
            dependency_sync::promote_workdir(&staged, root)?;
            write
                .sync(&config, &uv)
                .with_context(|| format!("prod sync failed during {action}"))?;
            write
                .smoke(&config, false)
                .with_context(|| format!("smoke test failed during {action}"))
        },
    )?;
    super::prune_history(root, &config);
    Ok(())
}

pub(super) fn reject_torch_family(name: &str) -> AppResult<()> {
    if TORCH_FAMILY.contains(&name) {
        return Err(AppError::usage(
            "torch-family packages are managed by 'gov install torch'",
        ));
    }
    Ok(())
}

pub(super) fn read_pins_file(path: &std::path::Path) -> AppResult<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

pub(super) fn merge_resolution_pins(
    tx: &mut TransactionRecord,
    pins: Vec<String>,
    force: bool,
) -> AppResult<()> {
    if pins.is_empty() {
        return Err(AppError::usage("at least one resolution pin is required"));
    }
    for pin in &pins {
        if !toml_support::is_exact_pin(pin) {
            return Err(AppError::usage(format!("invalid resolution pin: {pin}")));
        }
        reject_torch_family(&toml_support::normalize_package_name(pin))?;
    }
    for pin in pins {
        let name = toml_support::normalize_package_name(&pin);
        tx.resolution_pins
            .retain(|old| toml_support::normalize_package_name(old) != name);
        tx.resolution_pins.push(pin);
        tx.resolution_overrides.retain(|old| old != &name);
        if force {
            tx.resolution_overrides.push(name);
        }
    }
    Ok(())
}

pub(super) fn apply_resolution_pins(
    path: &std::path::Path,
    tx: &TransactionRecord,
) -> AppResult<()> {
    if !tx.resolution_pins.is_empty() {
        toml_support::edit_pins(path, &tx.resolution_pins, &tx.resolution_overrides, false)?;
    }
    Ok(())
}
