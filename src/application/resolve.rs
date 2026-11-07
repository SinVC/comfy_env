use crate::cli::ResolveArgs;
use crate::domain::{AppError, AppResult, ProjectRoot, TxId, TxStatus};
use crate::{dependency_sync, platform, state_ledger, toml_support};
use std::io::{self, Write};

pub fn cmd_resolve(args: ResolveArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    if tx.kind != "plugin" || tx.status != Some(TxStatus::NeedsResolution) {
        return Err(AppError::usage(
            "resolve requires a plugin transaction needing resolution",
        ));
    }
    let (config, uv) = super::configured_uv(root)?;
    let plugin = state_ledger::load_plugins(root)?
        .into_iter()
        .find(|p| tx.node_id.as_deref() == Some(&p.id))
        .ok_or_else(|| AppError::usage("transaction plugin is no longer registered"))?;
    let mut pins = args.pins;
    if let Some(file) = args.pins_file {
        pins.extend(
            std::fs::read_to_string(file)?
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned),
        );
    }
    if pins.is_empty() {
        if let Some(conflict) = state_ledger::load_conflict(root, &TxId(tx.id.clone()))? {
            println!("{}", conflict.summary);
        }
        println!("Enter pkg==version pins, one per line; empty line to finish:");
        loop {
            print!("> ");
            io::stdout().flush()?;
            let mut line = String::new();
            if io::stdin().read_line(&mut line)? == 0 || line.trim().is_empty() {
                break;
            }
            pins.push(line.trim().to_string());
        }
    }
    if pins.is_empty() {
        return Err(AppError::usage("at least one resolution pin is required"));
    }
    for pin in &pins {
        if !toml_support::is_exact_pin(pin) {
            return Err(AppError::usage(format!("invalid resolution pin: {pin}")));
        }
    }
    for pin in pins {
        let name = toml_support::normalize_package_name(&pin);
        tx.resolution_pins
            .retain(|old| toml_support::normalize_package_name(old) != name);
        tx.resolution_pins.push(pin);
    }
    let old_stage = platform::from_state_path(&tx.staged_workdir);
    let stage = dependency_sync::create_staged_workdir(root)?;
    let stage_path = stage.persist();
    tx.staged_workdir = platform::to_state_path(&stage_path);
    state_ledger::save_transaction(root, &tx)?;
    let locked = super::tx::stage_plugin(
        &stage_path,
        &plugin.group,
        &tx.observed_requirements.clone(),
        &tx.resolution_pins.clone(),
        &uv,
        root,
        &mut tx,
    )?;
    if old_stage != stage_path && old_stage.exists() {
        if let Err(error) =
            super::tx::remove_tx_artifact(&platform::to_state_path(&old_stage), &root.work_dir())
        {
            eprintln!("warning: failed to clean old stage: {error}");
        }
    }
    if !locked {
        println!("Transaction still needs resolution: {}", tx.id);
        return Ok(());
    }
    super::tx::observe_candidate(
        root,
        &config,
        &uv,
        &mut tx,
        &stage_path,
        config.tx.timeout_seconds,
    )?;
    if tx.status == Some(TxStatus::Completed) {
        tx.status = Some(TxStatus::Resolved);
        state_ledger::save_transaction(root, &tx)?;
    }
    println!("Transaction {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}
