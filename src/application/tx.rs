use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::cli::{TxAbortArgs, TxInspectArgs, TxPromoteArgs, TxRunArgs};
use crate::dependency_sync::{self, UvClient};
use crate::domain::{AppError, AppResult, GroupName, NodeId, ProjectRoot, TxId, TxKind, TxStatus};
use crate::platform;
use crate::runtime_executor::{self, RunOutcome};
use crate::source_integration;
use crate::state_ledger::{
    self, Observation, ObservationOutcome, OpKind, OperationLink, PackageDiff, TransactionRecord,
};
use crate::toml_support::{self, GroupEditMode, RuntimeConfig};

pub fn cmd_tx_run(args: TxRunArgs, root: &ProjectRoot) -> AppResult<()> {
    super::node::validate_node_id(&args.node_id)?;
    let (config, uv) = super::configured_uv(root)?;
    let plugins = state_ledger::load_plugins(root)?;
    let plugin = plugins
        .iter()
        .find(|p| p.id == args.node_id)
        .ok_or_else(|| AppError::usage(format!("node not registered: {}", args.node_id)))?;
    super::node::validate_group_owner(&plugins, &plugin.group, &plugin.id)?;
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
    let source = source_integration::plugin_install_path(comfy, &NodeId(plugin.id.clone()));
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
    let baseline = truth_hashes(root.as_path())?;
    let stage = dependency_sync::create_staged_workdir(root)?;
    let mut tx = TransactionRecord {
        id: id.0.clone(),
        kind: TxKind::Plugin,
        status: Some(TxStatus::Running),
        subject: plugin.id.clone(),
        node_id: Some(plugin.id.clone()),
        plugin_group: Some(GroupName(plugin.group.clone())),
        created_at: Some(state_ledger::now_string()?),
        candidate_env: platform::to_state_path(&candidate),
        staged_workdir: platform::to_state_path(stage.path()),
        observed_requirements: requirements,
        ..Default::default()
    };
    prepare_transaction(stage.path(), &GroupName(plugin.group.clone()), &tx)?;
    begin_observation(root, &mut tx, baseline)?;
    let stage_path = stage.persist();
    let locked = match lock_transaction(&stage_path, &uv, root, &mut tx) {
        Ok(locked) => locked,
        Err(error) => {
            mark_failed(root, &mut tx, &error)?;
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
        TxStatus::Completed,
    )?;
    println!("Transaction {}: {}", tx.id, tx.status.unwrap());
    Ok(())
}

pub(crate) fn mark_failed(
    root: &ProjectRoot,
    tx: &mut TransactionRecord,
    error: &anyhow::Error,
) -> Result<()> {
    tx.status = Some(TxStatus::Failed);
    tx.updated_at = Some(state_ledger::now_string()?);
    tx.logs.failure = Some(format!("{error:#}"));
    state_ledger::save_transaction(root, tx)
}

pub(crate) fn prepare_transaction(
    stage: &Path,
    group: &GroupName,
    tx: &TransactionRecord,
) -> AppResult<()> {
    let project_path = stage.join("pyproject.toml");
    let project = toml_support::read_project(&project_path)?;
    let group =
        toml_support::existing_group_name(&project, &group.0)?.unwrap_or_else(|| group.0.clone());
    toml_support::rewrite_dependency_group(
        &project_path,
        &group,
        GroupEditMode::Replace,
        &tx.observed_requirements,
    )?;
    super::pin::apply_resolution_pins(&project_path, tx)
}

pub(crate) fn restage_resolution(
    root: &ProjectRoot,
    group: &GroupName,
    tx: &mut TransactionRecord,
) -> AppResult<PathBuf> {
    let _guard = runtime_executor::try_lock_production(&root.pid_file())?;
    let current = state_ledger::load_transaction(root, &TxId(tx.id.clone()))?;
    if current.status != Some(TxStatus::NeedsResolution) || current.attempt != tx.attempt {
        return Err(AppError::usage(
            "transaction changed while preparing resolution; inspect it and retry",
        ));
    }
    let baseline = truth_hashes(root.as_path())?;
    let stage = dependency_sync::create_staged_workdir(root)?;
    prepare_transaction(stage.path(), group, tx)?;
    tx.staged_workdir = platform::to_state_path(stage.path());
    begin_observation(root, tx, baseline)?;
    let stage = stage.persist();
    if let Err(error) = remove_tx_artifact(&current.staged_workdir, &root.work_dir()) {
        eprintln!("warning: failed to clean old stage: {error:#}");
    }
    Ok(stage)
}

pub(crate) fn lock_transaction(
    stage: &Path,
    uv: &UvClient,
    root: &ProjectRoot,
    tx: &mut TransactionRecord,
) -> Result<bool> {
    let lock = uv.lock_tracked(stage, |process| {
        tx.candidate_process = process;
        state_ledger::save_transaction(root, tx)
    })?;
    if !lock.success {
        tx.conflict_report = Some(state_ledger::write_conflict(root, tx, &lock.stderr)?);
        tx.status = Some(TxStatus::NeedsResolution);
        tx.updated_at = Some(state_ledger::now_string()?);
        state_ledger::save_transaction(root, tx)?;
        return Ok(false);
    }
    Ok(true)
}

pub(crate) fn begin_observation(
    root: &ProjectRoot,
    tx: &mut TransactionRecord,
    baseline: BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        truth_hashes(root.as_path())? == baseline,
        "production truth changed while staging; rerun the transaction"
    );
    tx.status = Some(TxStatus::Running);
    tx.owner = Some(platform::ProcessIdentity::capture(std::process::id())?);
    tx.candidate_process = None;
    tx.candidate_ready = false;
    tx.pre_freeze.clear();
    tx.post_freeze.clear();
    tx.diff = PackageDiff::default();
    tx.core_impact.clear();
    tx.logs = Default::default();
    tx.attempt = Some(
        tx.attempt
            .unwrap_or(if tx.created_at.is_some() && tx.updated_at.is_some() {
                1
            } else {
                0
            })
            + 1,
    );
    tx.observation = None;
    tx.updated_at = None;
    tx.staged_truth_sha256 = None;
    tx.base_truth_sha256 = Some(baseline);
    state_ledger::save_transaction(root, tx)
}

pub(crate) fn observe_candidate(
    root: &ProjectRoot,
    config: &RuntimeConfig,
    uv: &UvClient,
    tx: &mut TransactionRecord,
    stage: &Path,
    timeout: u32,
    passed_status: TxStatus,
) -> Result<()> {
    use ObservationOutcome as Outcome;
    let candidate = root
        .as_path()
        .join(platform::from_state_path(&tx.candidate_env));
    let logs = root.state_dir().join("logs");
    let log_id = match tx.attempt {
        Some(attempt) if attempt > 1 => format!("{}.attempt-{attempt}", tx.id),
        _ => tx.id.clone(),
    };
    let stdout = logs.join(format!("{log_id}.stdout.log"));
    let stderr = logs.join(format!("{log_id}.stderr.log"));
    tx.logs.stdout = platform::to_state_path(&stdout);
    tx.logs.stderr = platform::to_state_path(&stderr);
    let mut observation = Observation {
        outcome: Outcome::InfrastructureFailed,
        interpreter: None,
        candidate_before: Vec::new(),
        package_drift: PackageDiff::default(),
        node_failures: Vec::new(),
    };
    let result = (|| -> Result<Outcome> {
        fs::create_dir_all(&logs)?;
        let candidate_root =
            crate::fs_support::resolved_path(&root.as_path().join(&config.runtime.candidate_root))?;
        let candidate_path = crate::fs_support::resolved_path(&candidate)?;
        let prod_path =
            crate::fs_support::resolved_path(&root.as_path().join(&config.runtime.prod_env))?;
        ensure!(
            candidate.is_absolute()
                && candidate_path == candidate_root.join(&tx.id)
                && !candidate_path.starts_with(&prod_path)
                && !prod_path.starts_with(&candidate_path),
            "candidate path is outside the transaction's environment or overlaps production"
        );
        tx.staged_truth_sha256 = Some(truth_hashes(stage)?);
        tx.conflict_report = None;
        tx.pre_freeze = freeze(uv, &root.as_path().join(&config.runtime.prod_env))?;
        state_ledger::save_transaction(root, tx)?;
        observation.outcome = Outcome::SyncFailed;
        let sync = uv.sync_tracked(stage, &candidate, |process| {
            tx.candidate_process = process;
            state_ledger::save_transaction(root, tx)
        })?;
        ensure!(
            sync.success,
            "candidate sync failed: {}",
            sync.summary_line()
        );
        tx.candidate_ready = true;
        observation.outcome = Outcome::InfrastructureFailed;
        let python = platform::venv_python(&candidate);
        let interpreter = platform::PythonClient::identity(&python)?;
        observation.candidate_before = freeze(uv, &candidate)?;
        let comfy = config
            .paths
            .comfyui_dir
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
            .context("config.toml is missing paths.comfyui_dir")?;
        let comfy = root.as_path().join(comfy);
        let probe = tempfile::tempdir_in(&logs)?;
        let report = probe.path().join("result");
        let cpu = config
            .run
            .extra_args
            .to_args()
            .iter()
            .any(|arg| arg == "--cpu");
        let outcome = runtime_executor::run_candidate(
            &candidate,
            &comfy,
            timeout,
            &stdout,
            &stderr,
            &report,
            if cpu { &["--cpu"] } else { &[] },
            |identity| {
                tx.candidate_process = Some(identity);
                state_ledger::save_transaction(root, tx)
            },
        )?;
        tx.candidate_process = None;
        match outcome {
            RunOutcome::Passed => tx.logs.run_exit_code = Some(0),
            RunOutcome::Failed(code) => tx.logs.run_exit_code = Some(code),
            RunOutcome::TimedOut => tx.logs.timed_out = true,
            RunOutcome::Interrupted => {}
        }
        tx.post_freeze = freeze(uv, &candidate)?;
        ensure!(
            platform::PythonClient::identity(&python)? == interpreter,
            "candidate interpreter changed during observation"
        );
        observation.interpreter = Some(interpreter);
        tx.diff = freeze_diff(&tx.pre_freeze, &tx.post_freeze);
        tx.core_impact = core_impact(&tx.diff, config);
        observation.package_drift = freeze_diff(&observation.candidate_before, &tx.post_freeze);
        ensure!(
            tx.staged_truth_sha256.as_ref() == Some(&truth_hashes(stage)?),
            "staged truth changed during the observation; rerun the transaction"
        );
        if tx.post_freeze != observation.candidate_before {
            return Ok(Outcome::PackageDrift);
        }
        let report = crate::fs_support::read_text_if_exists(&report)?;
        let mut loads = runtime_executor::node_load_results(&stdout)?;
        loads.extend(runtime_executor::node_load_results(&stderr)?);
        observation.node_failures = loads
            .iter()
            .filter(|(_, passed)| !passed)
            .map(|(path, _)| path.clone())
            .collect();
        match outcome {
            RunOutcome::TimedOut => return Ok(Outcome::TimedOut),
            RunOutcome::Interrupted => return Ok(Outcome::Interrupted),
            _ => {}
        }
        if report.as_deref() == Some("unsupported") {
            return Ok(Outcome::ProbeUnsupported);
        }
        if report.is_none() {
            bail!("candidate failed before the startup probe could initialize; see logs");
        }
        let target_loaded = if let Some(id) = &tx.node_id {
            let expected = source_integration::plugin_install_path(&comfy, &NodeId(id.clone()))
                .canonicalize()?;
            let mut loaded = false;
            for (path, passed) in &loads {
                if comfy.join(path).canonicalize().ok().as_ref() == Some(&expected) {
                    if !passed {
                        return Ok(Outcome::NodeLoadFailed);
                    }
                    loaded = true;
                }
            }
            loaded
        } else {
            if !observation.node_failures.is_empty() {
                return Ok(Outcome::NodeLoadFailed);
            }
            true
        };
        match outcome {
            RunOutcome::Failed(_) => Ok(Outcome::NonzeroExit),
            RunOutcome::Passed if report.as_deref() != Some("completed") => {
                Ok(Outcome::ProbeIncomplete)
            }
            RunOutcome::Passed if !target_loaded => Ok(Outcome::NodeLoadFailed),
            RunOutcome::Passed => Ok(Outcome::Passed),
            RunOutcome::TimedOut | RunOutcome::Interrupted => unreachable!(),
        }
    })();
    match &result {
        Ok(outcome) => observation.outcome = *outcome,
        Err(error) => tx.logs.failure = Some(format!("{error:#}")),
    }
    tx.status = Some(if observation.outcome == Outcome::Passed {
        passed_status
    } else {
        TxStatus::Failed
    });
    tx.observation = Some(observation);
    tx.updated_at = Some(state_ledger::now_string()?);
    state_ledger::save_transaction(root, tx)?;
    result.map(|_| ())
}

fn freeze(uv: &UvClient, env: &Path) -> Result<Vec<String>> {
    let mut packages = uv.pip_freeze(env)?;
    packages.sort_unstable();
    Ok(packages)
}

pub(crate) fn truth_hashes(dir: &Path) -> Result<BTreeMap<String, String>> {
    ["pyproject.toml", "uv.lock"]
        .into_iter()
        .map(|name| Ok((name.to_owned(), crate::fs_support::sha256(&dir.join(name))?)))
        .collect()
}

pub(crate) fn check_snapshot(
    root: &ProjectRoot,
    config: &RuntimeConfig,
    uv: &UvClient,
    tx: &TransactionRecord,
    allow_failed: bool,
    approve_core: bool,
) -> Result<(PathBuf, UvClient)> {
    const RERUN: &str = "run a fresh transaction or resolve the conflict and observe again";
    let observation = tx
        .observation
        .as_ref()
        .with_context(|| format!("transaction has no complete observation; {RERUN}"))?;
    match (tx.status, observation.outcome) {
        (Some(TxStatus::Completed | TxStatus::Resolved), ObservationOutcome::Passed) => {}
        (Some(TxStatus::Failed), ObservationOutcome::NonzeroExit) if allow_failed => {
            ensure!(
                tx.logs.run_exit_code.is_some_and(|code| code != 0),
                "missing nonzero exit evidence; {RERUN}"
            );
        }
        (Some(TxStatus::Failed), ObservationOutcome::NonzeroExit) => {
            bail!("nonzero candidate exit requires --allow-failed-run")
        }
        _ => bail!(
            "transaction observation {:?} is not promotable; {RERUN}",
            observation.outcome
        ),
    }
    ensure!(
        tx.candidate_ready && tx.candidate_process.is_none(),
        "candidate setup or cleanup is incomplete; {RERUN}"
    );
    ensure!(
        tx.base_truth_sha256.as_ref()
            == Some(
                &truth_hashes(root.as_path())
                    .with_context(|| format!("missing production truth; {RERUN}"))?
            ),
        "production truth changed after observation; {RERUN}"
    );
    let stage = checked_artifact(&tx.staged_workdir, &root.work_dir())
        .with_context(|| format!("staged workdir unavailable; {RERUN}"))?;
    ensure!(
        tx.staged_truth_sha256.as_ref()
            == Some(
                &truth_hashes(&stage).with_context(|| format!("missing staged truth; {RERUN}"))?
            ),
        "staged truth changed after observation; {RERUN}"
    );
    ensure!(
        freeze(uv, &root.as_path().join(&config.runtime.prod_env))? == tx.pre_freeze,
        "production package set changed after observation; {RERUN}"
    );
    ensure!(
        observation.candidate_before == tx.post_freeze,
        "candidate package set drifted; {RERUN}"
    );
    let interpreter = observation
        .interpreter
        .as_ref()
        .with_context(|| format!("transaction has no interpreter evidence; {RERUN}"))?;
    let (pinned_uv, current) = uv
        .resolved_python()
        .with_context(|| format!("could not verify the promotion interpreter; {RERUN}"))?;
    ensure!(
        current == *interpreter,
        "resolved Python interpreter changed after observation; {RERUN}"
    );
    let impact = core_impact(&freeze_diff(&tx.pre_freeze, &tx.post_freeze), config);
    ensure!(
        impact.is_empty() || approve_core,
        "core impact requires --approve-core: {}",
        impact.join(", ")
    );
    Ok((stage, pinned_uv))
}

pub(super) fn apply_snapshot(
    root: &ProjectRoot,
    config: &RuntimeConfig,
    uv: &UvClient,
    tx: &TransactionRecord,
    stage: &Path,
    write: &mut super::production::ProductionWrite<'_>,
) -> Result<()> {
    let env = root.as_path().join(&config.runtime.prod_env);
    let python = platform::venv_python(&env);
    let previous = if env.try_exists()? {
        Some(platform::PythonClient::identity(&python)?)
    } else {
        None
    };
    dependency_sync::promote_staged_path(stage, root)?;
    write.sync_with_previous_python(config, uv, previous)?;
    write.smoke(config, false)?;
    let actual = platform::PythonClient::identity(&python)?;
    ensure!(
        tx.observation
            .as_ref()
            .and_then(|obs| obs.interpreter.as_ref())
            == Some(&actual),
        "production interpreter differs from the observed candidate"
    );
    ensure!(
        freeze(uv, &env)? == tx.post_freeze,
        "production package set differs from the observed candidate"
    );
    Ok(())
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
        let value = super::install::requirement_text(line);
        if value.is_empty() {
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
    inspect_transaction(root, &tx)
}

pub(crate) fn inspect_transaction(root: &ProjectRoot, tx: &TransactionRecord) -> AppResult<()> {
    println!(
        "Transaction: {}\nKind: {}\nSubject: {}\nStatus: {}",
        tx.id,
        tx.kind,
        tx.subject,
        tx.status.unwrap_or(TxStatus::Running)
    );
    if tx.kind == TxKind::CoreUpdate {
        println!(
            "Source requirements: {}\nSource SHA256: {}",
            tx.source_requirements_path.as_deref().unwrap_or("-"),
            tx.source_requirements_sha256.as_deref().unwrap_or("-")
        );
    }
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
    print_observation(tx);
    if tx.status == Some(TxStatus::NeedsResolution) {
        if let Some(conflict) = state_ledger::load_conflict(root, &TxId(tx.id.clone()))? {
            println!("Conflict: {}\n{}", conflict.summary, conflict.input_hint);
        }
    }
    print_promotion(root, tx);
    Ok(())
}

fn print_promotion(root: &ProjectRoot, tx: &TransactionRecord) {
    if let Some(promotion) = &tx.promotion {
        let op_id = promotion.op_id.as_deref().unwrap_or_default();
        println!(
            "Promotion: {}\nOperation: {}\nReason: {}",
            promotion.status,
            op_id,
            promotion.reason.as_deref().unwrap_or_default()
        );
        if let Some(error) = &promotion.error {
            println!("Promotion failure: {error}");
        }
        if !op_id.is_empty()
            && let Ok(operation) =
                state_ledger::load_operation(root, &crate::domain::OpId(op_id.to_owned()))
            && operation.status == crate::domain::OpStatus::Undone
        {
            println!(
                "Promotion operation undone: {}",
                operation.undo_reference.unwrap_or_default()
            );
        }
    }
}

fn print_observation(tx: &TransactionRecord) {
    println!("Stdout: {}\nStderr: {}", tx.logs.stdout, tx.logs.stderr);
    if let Some(observation) = &tx.observation {
        println!(
            "Observation: {:?}\nNode load failures: {}\nCandidate added: {}\nCandidate removed: {}",
            observation.outcome,
            observation.node_failures.join(", "),
            observation.package_drift.added.join(", "),
            observation.package_drift.removed.join(", "),
        );
        if let Some(interpreter) = &observation.interpreter {
            println!("Observed Python: {}", interpreter.base_executable);
        }
    }
    if let Some(error) = &tx.logs.failure {
        println!("Observation error: {error}");
    }
}

pub fn cmd_tx_abort(args: TxAbortArgs, root: &ProjectRoot) -> AppResult<()> {
    let mut tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_plugin_tx(&tx)?;
    abort(root, &mut tx, args.confirm_stopped)?;
    println!("Transaction aborted: {}", tx.id);
    Ok(())
}

pub(crate) fn abort(
    root: &ProjectRoot,
    tx: &mut TransactionRecord,
    confirm_stopped: bool,
) -> AppResult<()> {
    if tx.status == Some(TxStatus::Aborted) {
        return Ok(());
    }
    if let Some(op) = state_ledger::operations_for_recovery(root)?
        .into_iter()
        .find(|op| {
            op.needs_recovery()
                && matches!(op.kind, OpKind::PluginPromote | OpKind::CorePromote)
                && op.reference == tx.id
        })
    {
        return Err(AppError::usage(format!(
            "promotion requires recovery; run gov op recover {}",
            op.op_id
        )));
    }
    if !matches!(
        tx.status,
        Some(
            TxStatus::Running
                | TxStatus::Completed
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
    if tx.status == Some(TxStatus::Running) {
        if let Some(owner) = &tx.owner {
            if owner.is_alive()? {
                return Err(AppError::usage("transaction owner is still running"));
            }
        } else if !confirm_stopped {
            return Err(AppError::usage(
                "old running transaction lacks process identity; verify its owner and candidate have exited, then retry with --confirm-stopped",
            ));
        }
    }
    if let Some(candidate) = &tx.candidate_process
        && candidate.candidate_is_alive()?
    {
        return Err(AppError::usage("candidate process tree is still running"));
    }
    let config = super::configured(root)?;
    remove_candidate(root, &config, tx)?;
    remove_tx_artifact(&tx.staged_workdir, &root.work_dir())?;
    tx.status = Some(TxStatus::Aborted);
    tx.updated_at = Some(state_ledger::now_string()?);
    state_ledger::save_transaction(root, tx)?;
    Ok(())
}

pub fn cmd_tx_promote(args: TxPromoteArgs, root: &ProjectRoot) -> AppResult<()> {
    let tx = state_ledger::load_transaction(root, &TxId(args.tx_id))?;
    ensure_plugin_tx(&tx)?;
    if super::production::resume_link(root, OpKind::PluginPromote, &tx.id)? {
        return Ok(());
    }
    crate::safety_guards::ensure_production_recoverable(root)?;
    let (config, uv) = super::configured_uv(root)?;
    let (stage, uv) = check_snapshot(
        root,
        &config,
        &uv,
        &tx,
        args.allow_failed_run,
        args.approve_core,
    )?;
    let mut plugins = state_ledger::load_plugins(root)?;
    let plugin = plugins
        .iter()
        .find(|p| tx.node_id.as_deref() == Some(&p.id))
        .cloned()
        .ok_or_else(|| AppError::usage("transaction plugin is no longer registered"))?;
    super::node::validate_group_owner(&plugins, &plugin.group, &plugin.id)?;
    if tx.plugin_group.as_ref().map(|group| group.0.as_str()) != Some(plugin.group.as_str()) {
        return Err(AppError::usage(
            "node dependency group changed after observation; run a fresh transaction",
        ));
    }
    let promotion = OperationLink::Promotion {
        tx_id: TxId(tx.id.clone()),
        reason: args.reason,
        keep_artifacts: args.keep_artifacts,
    };
    super::production::commit(
        root,
        OpKind::PluginPromote,
        &tx.id,
        Some(promotion),
        |_| Ok(()),
        |write| {
            apply_snapshot(root, &config, &uv, &tx, &stage, write)?;
            if let Some(entry) = plugins.iter_mut().find(|p| p.id == plugin.id) {
                entry.managed_deps = tx.observed_requirements.clone();
                entry.updated_at = state_ledger::now_string()?;
            }
            state_ledger::save_plugins(root, &plugins)
        },
    )?;
    super::prune_history(root, &config);
    println!("Transaction promoted: {}", tx.id);
    Ok(())
}

pub(crate) fn clean_artifacts(root: &ProjectRoot, config: &RuntimeConfig, tx: &TransactionRecord) {
    if let Err(error) = remove_candidate(root, config, tx) {
        eprintln!("warning: failed to clean {}: {error:#}", tx.candidate_env);
    }
    if let Err(error) = remove_tx_artifact(&tx.staged_workdir, &root.work_dir()) {
        eprintln!("warning: failed to clean {}: {error:#}", tx.staged_workdir);
    }
}

fn remove_candidate(
    root: &ProjectRoot,
    config: &RuntimeConfig,
    tx: &TransactionRecord,
) -> Result<()> {
    if !tx.candidate_env.is_empty() {
        let path = platform::from_state_path(&tx.candidate_env);
        ensure!(
            path.file_name() == Some(std::ffi::OsStr::new(&tx.id)),
            "candidate path does not belong to this transaction"
        );
    }
    remove_tx_artifact(
        &tx.candidate_env,
        &root.as_path().join(&config.runtime.candidate_root),
    )
}

pub(crate) fn remove_tx_artifact(recorded: &str, base: &Path) -> Result<()> {
    if recorded.is_empty()
        || crate::fs_support::metadata_if_exists(&platform::from_state_path(recorded))?.is_none()
    {
        return Ok(());
    }
    let path = checked_artifact(recorded, base)?;
    crate::fs_support::remove_tree(&path)
}

fn checked_artifact(recorded: &str, base: &Path) -> Result<PathBuf> {
    let path = platform::from_state_path(recorded);
    ensure!(!recorded.is_empty(), "invalid transaction artifact path");
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_dir() && !crate::fs_support::is_link(&metadata),
        "transaction artifact is not an ordinary directory: {}",
        path.display()
    );
    let resolved = path.canonicalize()?;
    let base = base.canonicalize()?;
    ensure!(
        resolved != base && resolved.starts_with(&base),
        "transaction artifact is outside its root: {}",
        path.display()
    );
    Ok(path)
}

fn ensure_plugin_tx(tx: &TransactionRecord) -> AppResult<()> {
    if tx.kind != TxKind::Plugin {
        return Err(AppError::usage(format!(
            "transaction {} is not a plugin transaction",
            tx.id
        )));
    }
    Ok(())
}
