use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::cli::{TxInspectArgs, TxPromoteArgs, TxRunArgs};
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, ProjectRoot, TxId, TxStatus};
use crate::platform;
use crate::runtime_executor::{self, RunOutcome};
use crate::safety_guards;
use crate::source_integration;
use crate::state_ledger::{self, OpKind, PackageDiff, PromotionRecord, TransactionRecord};
use crate::toml_support::{self, GroupEditMode, RuntimeConfig};

pub fn cmd_tx_run(args: TxRunArgs, root: &ProjectRoot) -> AppResult<()> {
    super::node::validate_node_id(&args.node_id)?;
    let (config, uv) = super::configured_uv(root)?;
    let plugin = state_ledger::load_plugins(root)?
        .into_iter()
        .find(|p| p.id == args.node_id)
        .ok_or_else(|| AppError::usage(format!("node not registered: {}", args.node_id)))?;
    if state_ledger::load_transactions(root)?.iter().any(|tx| {
        tx.node_id.as_deref() == Some(&args.node_id) && tx.status == Some(TxStatus::Running)
    }) {
        return Err(AppError::usage(format!(
            "running transaction already exists for {}",
            args.node_id
        )));
    }
    let comfy = config
        .paths
        .comfyui_dir
        .as_ref()
        .ok_or_else(|| AppError::usage("config.toml is missing paths.comfyui_dir"))?;
    let source = source_integration::plugin_install_path(comfy, &plugin.id);
    if !source.is_dir() {
        return Err(AppError::usage(format!(
            "node source missing: {}",
            source.display()
        )));
    }
    let requirements = read_plugin_requirements(&source.join("requirements.txt"))?;
    let id = state_ledger::next_tx_id()?;
    let candidate = root
        .as_path()
        .join(&config.runtime.candidate_root)
        .join(&id.0);
    let stage = dependency_sync::create_staged_workdir(root)?;
    let stage_path = stage.persist();
    let mut tx = TransactionRecord {
        id: id.0.clone(),
        kind: "plugin".into(),
        status: Some(TxStatus::Running),
        subject: plugin.id.clone(),
        node_id: Some(plugin.id.clone()),
        created_at: Some(state_ledger::now_string()?),
        candidate_env: platform::to_state_path(&candidate),
        staged_workdir: platform::to_state_path(&stage_path),
        observed_requirements: requirements,
        ..Default::default()
    };
    state_ledger::save_transaction(root, &tx)?;
    let requirements = tx.observed_requirements.clone();
    let locked = match stage_plugin(
        &stage_path,
        &plugin.group,
        &requirements,
        &[],
        &uv,
        root,
        &mut tx,
    ) {
        Ok(locked) => locked,
        Err(error) => {
            tx.status = Some(TxStatus::Failed);
            tx.updated_at = Some(state_ledger::now_string()?);
            state_ledger::save_transaction(root, &tx)?;
            return Err(error.into());
        }
    };
    if !locked {
        println!("Transaction needs resolution: {}", tx.id);
        return Ok(());
    }
    observe_candidate(
        root,
        &config,
        &uv,
        &mut tx,
        &stage_path,
        args.timeout.unwrap_or(config.tx.timeout_seconds),
    )?;
    println!("Transaction {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}

pub(crate) fn stage_plugin(
    stage: &Path,
    group: &str,
    requirements: &[String],
    pins: &[String],
    uv: &UvClient,
    root: &ProjectRoot,
    tx: &mut TransactionRecord,
) -> Result<bool> {
    toml_support::rewrite_dependency_group(
        &stage.join("pyproject.toml"),
        group,
        GroupEditMode::Replace,
        requirements,
    )?;
    if !pins.is_empty() {
        toml_support::rewrite_dependency_group(
            &stage.join("pyproject.toml"),
            "overrides",
            GroupEditMode::UpsertExact,
            pins,
        )?;
    }
    let lock = uv.lock(stage, root)?;
    if !lock.success {
        tx.conflict_report = Some(state_ledger::write_conflict(root, tx, &lock.stderr)?);
        tx.status = Some(TxStatus::NeedsResolution);
        tx.updated_at = Some(state_ledger::now_string()?);
        state_ledger::save_transaction(root, tx)?;
        return Ok(false);
    }
    Ok(true)
}

pub(crate) fn observe_candidate(
    root: &ProjectRoot,
    config: &RuntimeConfig,
    uv: &UvClient,
    tx: &mut TransactionRecord,
    stage: &Path,
    timeout: u32,
) -> Result<()> {
    let candidate = platform::from_state_path(&tx.candidate_env);
    let logs = root.state_dir().join("logs");
    fs::create_dir_all(&logs)?;
    let stdout = logs.join(format!("{}.stdout.log", tx.id));
    let stderr = logs.join(format!("{}.stderr.log", tx.id));
    tx.logs.stdout = platform::to_state_path(&stdout);
    tx.logs.stderr = platform::to_state_path(&stderr);
    let observation = (|| -> Result<()> {
        tx.pre_freeze = uv.pip_freeze(&root.as_path().join(&config.runtime.prod_env))?;
        let sync = uv.sync(stage, &candidate)?;
        ensure!(
            sync.success,
            "candidate sync failed: {}",
            sync.summary_line()
        );
        tx.candidate_ready = true;
        let comfy = config
            .paths
            .comfyui_dir
            .as_ref()
            .context("config.toml is missing paths.comfyui_dir")?;
        let outcome =
            runtime_executor::run_candidate(&candidate, comfy, timeout, &stdout, &stderr)?;
        tx.post_freeze = uv.pip_freeze(&candidate)?;
        tx.diff = freeze_diff(&tx.pre_freeze, &tx.post_freeze);
        tx.core_impact = core_impact(&tx.diff, config);
        match outcome {
            RunOutcome::Passed => {
                tx.status = Some(TxStatus::Completed);
                tx.logs.run_exit_code = Some(0);
            }
            RunOutcome::Failed(code) => {
                tx.status = Some(TxStatus::Failed);
                tx.logs.run_exit_code = Some(code);
            }
            RunOutcome::TimedOut => {
                tx.status = Some(TxStatus::Completed);
                tx.logs.timed_out = true;
                tx.logs.run_exit_code = Some(-1);
            }
        }
        Ok(())
    })();
    if let Err(error) = &observation {
        tx.status = Some(TxStatus::Failed);
        let _ = crate::fs_support::atomic_write(&stderr, format!("{error:#}\n").as_bytes());
    }
    tx.updated_at = Some(state_ledger::now_string()?);
    state_ledger::save_transaction(root, tx)?;
    observation
}

pub(crate) fn freeze_diff(before: &[String], after: &[String]) -> PackageDiff {
    let parse = |items: &[String]| -> BTreeMap<String, String> {
        items
            .iter()
            .map(|spec| (toml_support::normalize_package_name(spec), spec.clone()))
            .filter(|(name, _)| !name.is_empty())
            .collect()
    };
    let left = parse(before);
    let right = parse(after);
    PackageDiff {
        added: right
            .iter()
            .filter(|(name, spec)| left.get(*name) != Some(*spec))
            .map(|(_, spec)| spec.clone())
            .collect(),
        removed: left
            .iter()
            .filter(|(name, spec)| right.get(*name) != Some(*spec))
            .map(|(_, spec)| spec.clone())
            .collect(),
    }
}

pub(crate) fn core_impact(diff: &PackageDiff, config: &RuntimeConfig) -> Vec<String> {
    let cores: BTreeSet<_> = config
        .policy
        .core_packages
        .iter()
        .map(|name| toml_support::normalize_package_name(name))
        .collect();
    diff.added
        .iter()
        .chain(&diff.removed)
        .map(|spec| toml_support::normalize_package_name(spec))
        .filter(|name| cores.contains(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn read_plugin_requirements(path: &Path) -> AppResult<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path)?;
    let mut specs = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        let value = line.split(" #").next().unwrap_or("").trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        let name = toml_support::normalize_package_name(value);
        if value.starts_with('-')
            || value.starts_with('.')
            || value.contains('\\')
            || !toml_support::is_package_name(&name)
        {
            return Err(AppError::usage(format!(
                "unsupported requirement at {}:{}: {value}",
                path.display(),
                index + 1
            )));
        }
        specs.push(value.to_string());
    }
    Ok(specs)
}

pub fn cmd_tx_inspect(args: TxInspectArgs, root: &ProjectRoot) -> AppResult<()> {
    let tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_plugin_tx(&tx)?;
    println!(
        "Transaction: {}\nKind: {}\nSubject: {}\nStatus: {}",
        tx.id,
        tx.kind,
        tx.subject,
        tx.status.unwrap_or(TxStatus::Running)
    );
    println!(
        "Started: {}\nEnded: {}",
        tx.created_at.as_deref().unwrap_or("-"),
        tx.updated_at.as_deref().unwrap_or("-")
    );
    println!(
        "Added: {}\nRemoved: {}\nCore impact: {}",
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
        let label = if platform::from_state_path(path).exists() {
            ""
        } else if matches!(tx.status, Some(TxStatus::Promoted | TxStatus::Aborted)) {
            " (cleaned)"
        } else {
            " (missing)"
        };
        println!("{name}: {path}{label}");
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

pub fn cmd_tx_abort(args: TxInspectArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_plugin_tx(&tx)?;
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
            "transaction {} cannot be aborted from {:?}",
            tx.id, tx.status
        )));
    }
    let config = super::configured_uv(root)?.0;
    remove_tx_artifact(
        &tx.candidate_env,
        &root.as_path().join(&config.runtime.candidate_root),
    )?;
    remove_tx_artifact(&tx.staged_workdir, &root.work_dir())?;
    tx.status = Some(TxStatus::Aborted);
    tx.updated_at = Some(state_ledger::now_string()?);
    state_ledger::save_transaction(root, &tx)?;
    println!("Transaction aborted: {}", tx.id);
    Ok(())
}

pub fn cmd_tx_promote(args: TxPromoteArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_plugin_tx(&tx)?;
    if !matches!(
        tx.status,
        Some(TxStatus::Completed | TxStatus::Resolved | TxStatus::Failed)
    ) {
        return Err(AppError::usage(format!(
            "transaction {} is not promotable",
            tx.id
        )));
    }
    if tx.status == Some(TxStatus::Failed)
        && (!args.allow_failed_run
            || !tx.candidate_ready
            || tx.logs.run_exit_code.unwrap_or(0) == 0)
    {
        return Err(AppError::usage(
            "failed transaction requires an observed nonzero run and --allow-failed-run",
        ));
    }
    let (config, uv) = super::configured_uv(root)?;
    let mut plugins = state_ledger::load_plugins(root)?;
    let plugin = plugins
        .iter()
        .find(|p| tx.node_id.as_deref() == Some(&p.id))
        .cloned()
        .ok_or_else(|| AppError::usage("transaction plugin is no longer registered"))?;
    let mut plan = tx.observed_requirements.clone();
    let names: BTreeSet<_> = plan
        .iter()
        .map(|s| toml_support::normalize_package_name(s))
        .collect();
    for spec in &tx.diff.added {
        if !names.contains(&toml_support::normalize_package_name(spec)) {
            plan.push(spec.clone());
        }
    }
    let stage = dependency_sync::create_staged_workdir(root)?;
    let locked = stage_plugin(
        stage.path(),
        &plugin.group,
        &plan,
        &tx.resolution_pins.clone(),
        &uv,
        root,
        &mut tx,
    )?;
    if !locked {
        println!("Transaction needs resolution: {}", tx.id);
        return Ok(());
    }
    let pre = uv.pip_freeze(&root.as_path().join(&config.runtime.prod_env))?;
    fs::create_dir_all(root.as_path().join(&config.runtime.candidate_root))?;
    let probe = tempfile::Builder::new()
        .prefix("gov-probe-")
        .tempdir_in(root.as_path().join(&config.runtime.candidate_root))?;
    let probe_env = probe.path().join("env");
    let sync = uv.sync(stage.path(), &probe_env)?;
    if !sync.success {
        return Err(AppError::runtime(format!(
            "promotion candidate sync failed: {}",
            sync.summary_line()
        )));
    }
    let post = uv.pip_freeze(&probe_env)?;
    let impact = core_impact(&freeze_diff(&pre, &post), &config);
    if !impact.is_empty() && !args.approve_core {
        return Err(AppError::usage(format!(
            "core impact requires --approve-core: {}",
            impact.join(", ")
        )));
    }
    let op = safety_guards::op_begin(OpKind::PluginPromote, &tx.id, root)?;
    let mutation = (|| -> Result<()> {
        dependency_sync::promote_workdir(&stage, root)?;
        uv.sync_prod(root, &config).context("prod sync failed")?;
        let smoke =
            safety_guards::run_smoke_test(&root.as_path().join(&config.runtime.prod_env), &config)?;
        if !smoke.passed {
            bail!("smoke test failed: {}", smoke.detail);
        }
        if let Some(entry) = plugins.iter_mut().find(|p| p.id == plugin.id) {
            entry.managed_deps = plan.clone();
            entry.updated_at = state_ledger::now_string()?;
        }
        state_ledger::save_plugins(root, &plugins)?;
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
            if let Err(error) = remove_tx_artifact(path, &base) {
                eprintln!("warning: failed to clean {path}: {error:#}");
            }
        }
    }
    super::prune_history(root, &config);
    println!("Transaction promoted: {}", tx.id);
    Ok(())
}

pub(crate) fn remove_tx_artifact(recorded: &str, base: &Path) -> Result<()> {
    let path = platform::from_state_path(recorded);
    if !path.exists() {
        return Ok(());
    }
    let path = path.canonicalize()?;
    let base = base.canonicalize()?;
    ensure!(
        path != base && path.starts_with(&base),
        "transaction artifact is outside its root: {}",
        path.display()
    );
    fs::remove_dir_all(&path).with_context(|| format!("failed to remove {}", path.display()))
}

fn ensure_plugin_tx(tx: &TransactionRecord) -> AppResult<()> {
    if tx.kind != "plugin" {
        return Err(AppError::usage(format!(
            "transaction {} is not a plugin transaction",
            tx.id
        )));
    }
    Ok(())
}
