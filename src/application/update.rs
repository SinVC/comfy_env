use crate::cli::{
    TxAbortArgs, UpdateInspectArgs, UpdatePromoteArgs, UpdateResolveArgs, UpdateRunArgs,
};
use crate::dependency_sync;
use crate::domain::{
    AppError, AppResult, GroupName, ProjectRoot, TORCH_FAMILY, TxId, TxKind, TxStatus,
};
use crate::platform;
use crate::state_ledger::{self, OpKind, OperationLink, TransactionRecord};
use crate::toml_support;

pub fn cmd_update_run(args: UpdateRunArgs, root: &ProjectRoot) -> AppResult<()> {
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
            .as_ref()
            .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?
            .join("requirements.txt"),
    };
    if !requirements.is_file() {
        return Err(AppError::usage(format!(
            "requirements file missing: {}",
            requirements.display()
        )));
    }
    let requirements = requirements.canonicalize()?;
    let specs = super::install::read_core_requirements(&requirements)?;
    let source_hash = crate::fs_support::sha256(&requirements)?;

    let baseline = super::tx::truth_hashes(root.as_path())?;
    let stage = dependency_sync::create_staged_workdir(root)?;
    let id = state_ledger::next_tx_id()?;
    let candidate = root
        .as_path()
        .join(&config.runtime.candidate_root)
        .join(&id.0);
    let mut tx = TransactionRecord {
        id: id.0,
        kind: TxKind::CoreUpdate,
        status: Some(TxStatus::Running),
        subject: "core".into(),
        created_at: Some(state_ledger::now_string()?),
        candidate_env: platform::to_state_path(&candidate),
        staged_workdir: platform::to_state_path(stage.path()),

        observed_requirements: specs,

        source_requirements_path: Some(platform::to_state_path(&requirements)),
        source_requirements_sha256: Some(source_hash),
        ..Default::default()
    };
    super::tx::begin_observation(root, &mut tx, baseline)?;
    let stage_path = stage.persist();
    let locked = match super::tx::stage_transaction(
        &stage_path,
        &GroupName("core".into()),
        &uv,
        root,
        &mut tx,
    ) {
        Ok(locked) => locked,
        Err(error) => {
            super::tx::mark_failed(root, &mut tx, &error)?;
            return Err(error.into());
        }
    };
    if !locked {
        println!("Update needs resolution: {}", tx.id);
        return Ok(());
    }
    super::tx::observe_candidate(
        root,
        &config,
        &uv,
        &mut tx,
        &stage_path,
        args.timeout.unwrap_or(config.tx.timeout_seconds),
    )?;
    println!("Update {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}

pub fn cmd_update_inspect(args: UpdateInspectArgs, root: &ProjectRoot) -> AppResult<()> {
    let tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    super::tx::inspect_transaction(root, &tx)
}
pub fn cmd_update_abort(args: TxAbortArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    super::tx::abort(root, &mut tx, args.confirm_stopped)?;
    println!("Update aborted: {}", tx.id);
    Ok(())
}

pub fn cmd_update_resolve(args: UpdateResolveArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    if tx.status != Some(TxStatus::NeedsResolution) {
        return Err(AppError::usage("update is not awaiting resolution"));
    }
    let mut pins = args.pins;
    if let Some(path) = args.pins_file {
        pins.extend(super::pin::read_pins_file(&path)?);
    }
    super::pin::merge_resolution_pins(&mut tx, pins, args.override_requirements)?;
    let (config, uv) = super::configured_uv(root)?;

    let old_stage = std::mem::take(&mut tx.staged_workdir);
    let baseline = super::tx::truth_hashes(root.as_path())?;
    let stage = dependency_sync::create_staged_workdir(root)?;
    tx.staged_workdir = platform::to_state_path(stage.path());
    super::tx::begin_observation(root, &mut tx, baseline)?;
    let stage = stage.persist();
    if let Err(error) = super::tx::remove_tx_artifact(&old_stage, &root.work_dir()) {
        eprintln!("warning: failed to clean old stage: {error:#}");
    }
    let locked =
        match super::tx::stage_transaction(&stage, &GroupName("core".into()), &uv, root, &mut tx) {
            Ok(locked) => locked,
            Err(error) => {
                super::tx::mark_failed(root, &mut tx, &error)?;
                return Err(error.into());
            }
        };
    if !locked {
        println!("Update still needs resolution: {}", tx.id);
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
        &stage,
        config.tx.timeout_seconds,
    )?;
    if tx.status == Some(TxStatus::Completed) {
        tx.status = Some(TxStatus::Resolved);
        state_ledger::save_transaction(root, &tx)?;
    }
    println!("Update {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}

pub fn cmd_update_promote(args: UpdatePromoteArgs, root: &ProjectRoot) -> AppResult<()> {
    let tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    if super::production::resume_link(root, OpKind::CorePromote, &tx.id)? {
        return Ok(());
    }
    let (config, uv) = super::configured_uv(root)?;
    let stage = super::tx::check_snapshot(
        root,
        &config,
        &uv,
        &tx,
        args.allow_failed_run,
        args.approve_core,
    )?;
    let promotion = OperationLink::Promotion {
        tx_id: TxId(tx.id.clone()),
        reason: args.reason,
        keep_artifacts: args.keep_artifacts,
    };
    super::production::commit(
        root,
        OpKind::CorePromote,
        &tx.id,
        Some(promotion),
        |_| Ok(()),
        |write| {
            dependency_sync::promote_staged_path(&stage, root)?;
            write.sync(&config, &uv)?;
            write.smoke(&config, false)
        },
    )?;
    super::prune_history(root, &config);
    println!("Update promoted: {}", tx.id);
    Ok(())
}

fn ensure_core_update(tx: &TransactionRecord) -> AppResult<()> {
    if tx.kind != TxKind::CoreUpdate {
        return Err(AppError::usage(format!(
            "transaction {} is not a core update",
            tx.id
        )));
    }
    Ok(())
}
