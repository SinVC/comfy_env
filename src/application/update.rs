use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::{UpdateInspectArgs, UpdatePromoteArgs, UpdateResolveArgs, UpdateRunArgs};
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, ProjectRoot, TxId, TxStatus};
use crate::platform;
use crate::safety_guards;
use crate::state_ledger::{self, OpKind, PromotionRecord, TransactionRecord};
use crate::toml_support::{self, GroupEditMode};

pub fn cmd_update_run(args: UpdateRunArgs, root: &ProjectRoot) -> AppResult<()> {
    let (config, uv) = super::configured_uv(root)?;
    let torch = toml_support::read_dependency_group(&root.pyproject_toml(), "torch")?;
    if !["torch", "torchvision", "torchaudio"].iter().all(|name| {
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
    let base_hashes = truth_hashes(root.as_path())?;
    let stage = dependency_sync::create_staged_workdir(root)?;
    let stage_path = stage.persist();
    let id = state_ledger::next_tx_id()?;
    let candidate = root
        .as_path()
        .join(&config.runtime.candidate_root)
        .join(&id.0);
    let mut tx = TransactionRecord {
        id: id.0,
        kind: "core_update".into(),
        status: Some(TxStatus::Running),
        subject: "core".into(),
        created_at: Some(state_ledger::now_string()?),
        candidate_env: platform::to_state_path(&candidate),
        staged_workdir: platform::to_state_path(&stage_path),
        observed_requirements: specs.clone(),
        base_truth_sha256: Some(base_hashes),
        source_requirements_path: Some(platform::to_state_path(&requirements)),
        source_requirements_sha256: Some(source_hash),
        ..Default::default()
    };
    state_ledger::save_transaction(root, &tx)?;
    let locked = match stage_core(root, &uv, &stage_path, &specs, &[], &mut tx) {
        Ok(locked) => locked,
        Err(error) => {
            mark_failed(root, &mut tx)?;
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

fn stage_core(
    root: &ProjectRoot,
    uv: &UvClient,
    stage: &Path,
    specs: &[String],
    pins: &[String],
    tx: &mut TransactionRecord,
) -> Result<bool> {
    toml_support::rewrite_dependency_group(
        &stage.join("pyproject.toml"),
        "core",
        GroupEditMode::Replace,
        specs,
    )?;
    if !pins.is_empty() {
        toml_support::rewrite_dependency_group(
            &stage.join("pyproject.toml"),
            "overrides",
            GroupEditMode::UpsertExact,
            pins,
        )?;
        toml_support::upsert_uv_overrides(&stage.join("pyproject.toml"), pins)?;
    }
    let lock = uv.lock(stage, root)?;
    if !lock.success {
        tx.conflict_report = Some(state_ledger::write_conflict(root, tx, &lock.stderr)?);
        tx.status = Some(TxStatus::NeedsResolution);
        tx.updated_at = Some(state_ledger::now_string()?);
        state_ledger::save_transaction(root, tx)?;
        return Ok(false);
    }
    tx.staged_truth_sha256 = Some(truth_hashes(stage)?);
    tx.conflict_report = None;
    state_ledger::save_transaction(root, tx)?;
    Ok(true)
}

pub fn cmd_update_inspect(args: UpdateInspectArgs, root: &ProjectRoot) -> AppResult<()> {
    let tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    println!(
        "Transaction: {}\nKind: {}\nSubject: {}\nStatus: {}",
        tx.id,
        tx.kind,
        tx.subject,
        tx.status.unwrap_or(TxStatus::Running)
    );
    println!(
        "Source requirements: {}\nSource SHA256: {}",
        tx.source_requirements_path.as_deref().unwrap_or("-"),
        tx.source_requirements_sha256.as_deref().unwrap_or("-")
    );
    println!(
        "Started: {}\nEnded: {}\nAdded: {}\nRemoved: {}\nCore impact: {}",
        tx.created_at.as_deref().unwrap_or("-"),
        tx.updated_at.as_deref().unwrap_or("-"),
        tx.diff.added.join(", "),
        tx.diff.removed.join(", "),
        tx.core_impact.join(", ")
    );
    println!(
        "Run exit code: {:?}\nTimed out: {}",
        tx.logs.run_exit_code, tx.logs.timed_out
    );
    for (name, path) in [
        ("Candidate env", &tx.candidate_env),
        ("Staged workdir", &tx.staged_workdir),
    ] {
        let annotation = if platform::from_state_path(path).exists() {
            ""
        } else if matches!(tx.status, Some(TxStatus::Promoted | TxStatus::Aborted)) {
            " (cleaned)"
        } else {
            " (missing)"
        };
        println!("{name}: {path}{annotation}");
    }
    println!("Stdout: {}\nStderr: {}", tx.logs.stdout, tx.logs.stderr);
    if tx.status == Some(TxStatus::NeedsResolution) {
        if let Some(conflict) = state_ledger::load_conflict(root, &TxId(tx.id.clone()))? {
            println!("Conflict: {}\n{}", conflict.summary, conflict.input_hint);
        }
    }
    if let Some(promotion) = tx.promotion {
        println!(
            "Promotion: {}\nOperation: {}\nReason: {}",
            promotion.status,
            promotion.op_id.unwrap_or_default(),
            promotion.reason.unwrap_or_default()
        );
    }
    Ok(())
}

pub fn cmd_update_abort(args: UpdateInspectArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    if !matches!(
        tx.status,
        Some(
            TxStatus::Completed
                | TxStatus::Failed
                | TxStatus::NeedsResolution
                | TxStatus::Resolved
                | TxStatus::PromoteFailed
        )
    ) {
        return Err(AppError::usage(format!(
            "update {} cannot be aborted from {:?}",
            tx.id, tx.status
        )));
    }
    let config = super::configured_uv(root)?.0;
    super::tx::remove_tx_artifact(
        &tx.candidate_env,
        &root.as_path().join(&config.runtime.candidate_root),
    )?;
    super::tx::remove_tx_artifact(&tx.staged_workdir, &root.work_dir())?;
    tx.status = Some(TxStatus::Aborted);
    tx.updated_at = Some(state_ledger::now_string()?);
    state_ledger::save_transaction(root, &tx)?;
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
        pins.extend(
            fs::read_to_string(&path)?
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned),
        );
    }
    if pins.is_empty() {
        return Err(AppError::usage("at least one resolution pin is required"));
    }
    for pin in &pins {
        if !toml_support::is_exact_pin(pin) {
            return Err(AppError::usage(format!("invalid resolution pin: {pin}")));
        }
    }
    verify_baseline(root, &tx)?;
    let stage = checked_stage(root, &tx)?;
    let (config, uv) = super::configured_uv(root)?;
    for pin in pins {
        let name = toml_support::normalize_package_name(&pin);
        tx.resolution_pins
            .retain(|old| toml_support::normalize_package_name(old) != name);
        tx.resolution_pins.push(pin);
    }
    let specs = tx.observed_requirements.clone();
    let merged_pins = tx.resolution_pins.clone();
    state_ledger::save_transaction(root, &tx)?;
    let locked = match stage_core(root, &uv, &stage, &specs, &merged_pins, &mut tx) {
        Ok(locked) => locked,
        Err(error) => {
            mark_failed(root, &mut tx)?;
            return Err(error.into());
        }
    };
    if !locked {
        println!("Update still needs resolution: {}", tx.id);
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
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_core_update(&tx)?;
    if !matches!(
        tx.status,
        Some(TxStatus::Completed | TxStatus::Resolved | TxStatus::Failed)
    ) {
        return Err(AppError::usage(format!(
            "update {} is not promotable",
            tx.id
        )));
    }
    if tx.status == Some(TxStatus::Failed)
        && (!args.allow_failed_run
            || !tx.candidate_ready
            || tx.logs.run_exit_code.unwrap_or(0) == 0)
    {
        return Err(AppError::usage(
            "failed update requires an observed nonzero run and --allow-failed-run",
        ));
    }
    if !tx.candidate_ready {
        return Err(AppError::usage(
            "update has no synced candidate environment",
        ));
    }
    verify_baseline(root, &tx)?;
    let stage = checked_stage(root, &tx)?;
    let staged_hashes = tx
        .staged_truth_sha256
        .as_ref()
        .ok_or_else(|| AppError::usage("update has no locked snapshot; run a fresh update"))?;
    if &truth_hashes(&stage)? != staged_hashes {
        return Err(AppError::usage(
            "staged update changed after observation; run a fresh update",
        ));
    }
    if !tx.core_impact.is_empty() && !args.approve_core {
        return Err(AppError::usage(format!(
            "core impact requires --approve-core: {}",
            tx.core_impact.join(", ")
        )));
    }
    let (config, uv) = super::configured_uv(root)?;
    let op = safety_guards::op_begin(OpKind::CorePromote, &tx.id, root)?;
    let mutation = (|| -> Result<()> {
        dependency_sync::promote_staged_path(&stage, root)?;
        uv.sync_prod(root, &config).context("prod sync failed")?;
        let smoke =
            safety_guards::run_smoke_test(&root.as_path().join(&config.runtime.prod_env), &config)?;
        if !smoke.passed {
            bail!("smoke test failed: {}", smoke.detail);
        }
        tx.status = Some(TxStatus::Promoted);
        tx.updated_at = Some(state_ledger::now_string()?);
        tx.promotion = Some(PromotionRecord {
            status: TxStatus::Promoted,
            reason: args.reason.clone(),
            op_id: Some(op.0.clone()),
            error: None,
        });
        state_ledger::save_transaction(root, &tx)?;
        safety_guards::op_finalize(&op, true, None, root)
    })();
    if let Err(error) = mutation {
        let failure = super::rollback_failed_operation(root, &op, &config, &uv, error);
        tx.status = Some(TxStatus::PromoteFailed);
        tx.promotion = Some(PromotionRecord {
            status: TxStatus::PromoteFailed,
            reason: args.reason,
            op_id: Some(op.0),
            error: Some(failure.to_string()),
        });
        tx.updated_at = Some(state_ledger::now_string()?);
        state_ledger::save_transaction(root, &tx)?;
        return Err(failure);
    }
    if !args.keep_artifacts {
        for (path, base) in [
            (
                &tx.candidate_env,
                root.as_path().join(&config.runtime.candidate_root),
            ),
            (&tx.staged_workdir, root.work_dir()),
        ] {
            if let Err(error) = super::tx::remove_tx_artifact(path, &base) {
                eprintln!("warning: failed to clean {path}: {error:#}");
            }
        }
    }
    super::prune_history(root, &config);
    println!("Update promoted: {}", tx.id);
    Ok(())
}

fn ensure_core_update(tx: &TransactionRecord) -> AppResult<()> {
    if tx.kind != "core_update" {
        return Err(AppError::usage(format!(
            "transaction {} is not a core update",
            tx.id
        )));
    }
    Ok(())
}

fn truth_hashes(dir: &Path) -> Result<BTreeMap<String, String>> {
    ["pyproject.toml", "uv.lock"]
        .into_iter()
        .map(|name| Ok((name.to_owned(), crate::fs_support::sha256(&dir.join(name))?)))
        .collect()
}

fn verify_baseline(root: &ProjectRoot, tx: &TransactionRecord) -> AppResult<()> {
    let expected = tx
        .base_truth_sha256
        .as_ref()
        .ok_or_else(|| AppError::usage("update has no truth baseline; run a fresh update"))?;
    if &truth_hashes(root.as_path())? != expected {
        return Err(AppError::usage(
            "project truth changed after staging; run a fresh update",
        ));
    }
    Ok(())
}

fn checked_stage(root: &ProjectRoot, tx: &TransactionRecord) -> AppResult<PathBuf> {
    let stage = platform::from_state_path(&tx.staged_workdir)
        .canonicalize()
        .with_context(|| format!("staged workdir missing: {}", tx.staged_workdir))?;
    let base = root.work_dir().canonicalize()?;
    if stage == base || !stage.starts_with(base) {
        return Err(AppError::usage("staged workdir is outside state/work"));
    }
    Ok(stage)
}

fn mark_failed(root: &ProjectRoot, tx: &mut TransactionRecord) -> Result<()> {
    tx.status = Some(TxStatus::Failed);
    tx.updated_at = Some(state_ledger::now_string()?);
    state_ledger::save_transaction(root, tx)
}
