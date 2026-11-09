use crate::cli::ResolveArgs;
use crate::domain::{AppError, AppResult, GroupName, ProjectRoot, TxId, TxKind, TxStatus};
use crate::state_ledger;
use std::io::{self, Write};

pub fn cmd_resolve(args: ResolveArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    if tx.kind != TxKind::Plugin || tx.status != Some(TxStatus::NeedsResolution) {
        return Err(AppError::usage(
            "resolve requires a plugin transaction needing resolution",
        ));
    }
    let (config, uv) = super::configured_uv(root)?;
    let plugins = state_ledger::load_plugins(root)?;
    let plugin = plugins
        .iter()
        .find(|p| tx.node_id.as_deref() == Some(&p.id))
        .ok_or_else(|| AppError::usage("transaction plugin is no longer registered"))?;
    super::node::validate_group_owner(&plugins, &plugin.group, &plugin.id)?;
    tx.plugin_group = Some(GroupName(plugin.group.clone()));
    let mut pins = args.pins;
    if let Some(file) = args.pins_file {
        pins.extend(super::pin::read_pins_file(&file)?);
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
    super::pin::merge_resolution_pins(&mut tx, pins, args.override_requirements)?;
    let stage_path =
        super::tx::restage_resolution(root, &GroupName(plugin.group.clone()), &mut tx)?;
    let locked = match super::tx::lock_transaction(&stage_path, &uv, root, &mut tx) {
        Ok(locked) => locked,
        Err(error) => {
            super::tx::mark_failed(root, &mut tx, &error)?;
            return Err(error.into());
        }
    };
    if !locked {
        println!("Transaction still needs resolution: {}", tx.id);
        if !args.override_requirements {
            println!(
                "Use --override-requirements to explicitly override incompatible requirements."
            );
        }
        return Ok(());
    }
    super::tx::observe_candidate(
        root,
        &config,
        &uv,
        &mut tx,
        &stage_path,
        config.tx.timeout_seconds,
        TxStatus::Resolved,
    )?;
    println!("Transaction {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}
