use crate::cli::ResolveArgs;
use crate::domain::{AppError, AppResult, GroupName, ProjectRoot, TxId, TxKind, TxStatus};
use crate::{dependency_sync, platform, state_ledger};
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
    let old_stage = platform::from_state_path(&tx.staged_workdir);
    let baseline = super::tx::truth_hashes(root.as_path())?;
    let stage = dependency_sync::create_staged_workdir(root)?;
    tx.staged_workdir = platform::to_state_path(stage.path());
    super::tx::begin_observation(root, &mut tx, baseline)?;
    let stage_path = stage.persist();
    let locked = super::tx::stage_transaction(
        &stage_path,
        &GroupName(plugin.group.clone()),
        &uv,
        root,
        &mut tx,
    );
    if old_stage != stage_path && old_stage.exists() {
        if let Err(error) =
            super::tx::remove_tx_artifact(&platform::to_state_path(&old_stage), &root.work_dir())
        {
            eprintln!("warning: failed to clean old stage: {error}");
        }
    }
    let locked = match locked {
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
    )?;
    if tx.status == Some(TxStatus::Completed) {
        tx.status = Some(TxStatus::Resolved);
        state_ledger::save_transaction(root, &tx)?;
    }
    println!("Transaction {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}
