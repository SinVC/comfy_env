use anyhow::{Context, Result, bail, ensure};

use crate::dependency_sync::UvClient;
use crate::domain::{AppError, AppResult, OpId, OpStatus, ProjectRoot, TxStatus};
use crate::state_ledger::{
    self, EnvironmentRecovery, OpKind, OperationLink, OperationPhase, OperationRecord,
    PromotionRecord,
};
use crate::toml_support::{self, RuntimeConfig};
use crate::{fs_support, platform, runtime_executor, safety_guards};

pub(super) struct ProductionWrite<'a> {
    root: &'a ProjectRoot,
    operation: &'a mut OperationRecord,
}

impl ProductionWrite<'_> {
    pub(super) fn op_id(&self) -> OpId {
        OpId(self.operation.op_id.clone())
    }

    pub(super) fn touch_source(&mut self) -> Result<()> {
        self.operation.recovery.as_mut().unwrap().source_touched = true;
        state_ledger::save_operation(self.root, self.operation)
    }

    pub(super) fn sync(&mut self, config: &RuntimeConfig, uv: &UvClient) -> Result<()> {
        let path = self.root.as_path().join(&config.runtime.prod_env);
        let existed = path.try_exists()?;
        if !existed {
            checked_new_environment(self.root, &path)?;
        }
        self.operation.recovery.as_mut().unwrap().sync = Some(EnvironmentRecovery {
            path: platform::to_state_path(&path),
            existed,
        });
        state_ledger::save_operation(self.root, self.operation)?;
        uv.sync_prod_tracked(self.root, config, |process| self.record_process(process))
            .context("production sync failed")
    }

    pub(super) fn smoke(&mut self, config: &RuntimeConfig, torch: bool) -> Result<()> {
        let env = self.root.as_path().join(&config.runtime.prod_env);
        let record = |process| self.record_process(process);
        let smoke = if torch {
            safety_guards::run_torch_smoke_test(&env, config, record)?
        } else {
            safety_guards::run_smoke_test(&env, config, record)?
        };
        ensure!(smoke.passed, "smoke test failed: {}", smoke.detail);
        Ok(())
    }

    fn record_process(&mut self, process: Option<platform::ProcessIdentity>) -> Result<()> {
        self.operation.recovery.as_mut().unwrap().active_process = process;
        state_ledger::save_operation(self.root, self.operation)
    }
}

pub(super) fn commit<T>(
    root: &ProjectRoot,
    kind: OpKind,
    reference: &str,
    link: Option<OperationLink>,
    prepare: impl FnOnce(&mut OperationRecord) -> Result<()>,
    apply: impl FnOnce(&mut ProductionWrite<'_>) -> Result<T>,
) -> AppResult<T> {
    let mut operation = safety_guards::op_begin(kind, reference, link, root)?;
    let id = OpId(operation.op_id.clone());
    let result = (|| -> Result<T> {
        prepare(&mut operation)?;
        safety_guards::verify_backup(&id, root, false)?;
        operation.recovery.as_mut().unwrap().phase = OperationPhase::Applying;
        state_ledger::save_operation(root, &operation)?;
        let value = apply(&mut ProductionWrite {
            root,
            operation: &mut operation,
        })?;
        safety_guards::op_finalize(&mut operation, true, None, root)?;
        Ok(value)
    })();
    match result {
        Ok(value) => {
            finish_link(root, &mut operation).with_context(|| {
                format!(
                    "production committed; run gov op recover {} to record its link",
                    id.0
                )
            })?;
            clean_node_recovery(root, &operation);
            Ok(value)
        }
        Err(error) => {
            let mut operation = state_ledger::load_operation(root, &id)
                .with_context(|| format!("{error:#}; unable to read recovery record {}", id.0))?;
            if operation.status == OpStatus::Success {
                return Err(AppError::runtime(format!(
                    "production committed; run gov op recover {}: {error:#}",
                    id.0
                )));
            }
            let message = format!("{error:#}");
            if let Err(restore) = restore(root, &mut operation, Some(message.clone())) {
                return Err(AppError::runtime(format!(
                    "{message}; rollback failed: {restore:#}; run gov op recover {}",
                    id.0
                )));
            }
            Err(AppError::runtime(message))
        }
    }
}

pub(super) fn recover(root: &ProjectRoot, id: &OpId) -> AppResult<()> {
    let mut operation = state_ledger::load_operation(root, id)?;
    if !operation.needs_recovery() {
        println!("No recovery needed for {}.", id.0);
        return Ok(());
    }
    runtime_executor::ensure_prod_stopped(&root.pid_file())?;
    let recovery = operation.recovery.as_ref().ok_or_else(|| {
        AppError::usage("old running operation lacks recovery phase evidence; preserve its backups and verify the project manually")
    })?;
    if recovery.owner.is_alive()? {
        return Err(AppError::usage("operation owner is still running"));
    }
    if let Some(process) = &recovery.active_process
        && process.candidate_is_alive()?
    {
        return Err(AppError::usage(
            "operation's external process tree is still running",
        ));
    }
    if recovery.phase == OperationPhase::Committed {
        finish_link(root, &mut operation)?;
    } else {
        restore(root, &mut operation, None)?;
    }
    println!("Recovery completed: {}", id.0);
    Ok(())
}

pub(super) fn resume_link(root: &ProjectRoot, kind: OpKind, reference: &str) -> AppResult<bool> {
    let pending = state_ledger::operations_for_recovery(root)?
        .into_iter()
        .find(|op| {
            op.kind == kind
                && op.reference == reference
                && op.status == OpStatus::Success
                && op.needs_recovery()
        });
    if let Some(operation) = pending {
        recover(root, &OpId(operation.op_id))?;
        return Ok(true);
    }
    Ok(false)
}

fn restore(
    root: &ProjectRoot,
    operation: &mut OperationRecord,
    error: Option<String>,
) -> Result<()> {
    let id = OpId(operation.op_id.clone());
    let recovery = operation
        .recovery
        .as_mut()
        .context("missing recovery evidence")?;
    if let Some(error) = error {
        recovery.apply_error = Some(error);
        state_ledger::save_operation(root, operation)?;
    }
    let recovery = operation.recovery.as_ref().unwrap();
    let result = if recovery.phase == OperationPhase::Prepared {
        Ok(())
    } else {
        restore_resources(root, &id, operation)
    };
    if let Err(error) = result {
        let recovery = operation.recovery.as_mut().unwrap();
        recovery.phase = OperationPhase::RestoreFailed;
        recovery.restore_error = Some(format!("{error:#}"));
        operation.note = Some(format!(
            "{}; rollback failed: {error:#}",
            recovery
                .apply_error
                .as_deref()
                .unwrap_or("operation interrupted")
        ));
        state_ledger::save_operation(root, operation)
            .context("failed to persist recovery failure")?;
        return Err(error);
    }
    let note = operation
        .recovery
        .as_ref()
        .unwrap()
        .apply_error
        .clone()
        .or_else(|| Some("interrupted operation recovered".into()));
    if operation.recovery.as_ref().unwrap().phase != OperationPhase::Prepared
        && let Some(OperationLink::Promotion { tx_id, reason, .. }) =
            &operation.recovery.as_ref().unwrap().link
    {
        let mark = (|| -> Result<()> {
            let mut tx = state_ledger::load_transaction(root, tx_id)?;
            tx.status = Some(TxStatus::PromoteFailed);
            tx.updated_at = Some(state_ledger::now_string()?);
            tx.promotion = Some(PromotionRecord {
                status: TxStatus::PromoteFailed,
                reason: reason.clone(),
                op_id: Some(id.0.clone()),
                error: note.clone(),
            });
            state_ledger::save_transaction(root, &tx)
        })();
        if let Err(error) = mark {
            eprintln!("warning: could not record promotion failure: {error:#}");
        }
    }
    safety_guards::op_finalize(operation, false, note, root)?;
    clean_node_recovery(root, operation);
    Ok(())
}

fn clean_node_recovery(root: &ProjectRoot, operation: &OperationRecord) {
    if operation.kind == OpKind::EnvImport
        && let Err(error) = safety_guards::node_swap::clean(root, &OpId(operation.op_id.clone()))
    {
        eprintln!("warning: failed to clean node recovery directory: {error:#}");
    }
}

fn restore_resources(root: &ProjectRoot, id: &OpId, operation: &mut OperationRecord) -> Result<()> {
    let recovery = operation.recovery.as_mut().unwrap();
    if let Some(process) = &recovery.active_process {
        ensure!(
            !process.candidate_is_alive()?,
            "operation's external process tree is still running"
        );
    }
    recovery.owner = platform::ProcessIdentity::capture(std::process::id())?;
    recovery.active_process = None;
    let source_touched = recovery.source_touched;
    let sync = recovery.sync.clone();
    state_ledger::save_operation(root, operation)?;
    safety_guards::op_restore(id, root, source_touched)?;
    if let Some(sync) = sync {
        let path = platform::from_state_path(&sync.path);
        if !sync.existed {
            checked_new_environment(root, &path)?;
            fs_support::remove_tree(&path)?;
        } else {
            ensure!(
                root.uv_lock().is_file(),
                "missing previous lock for recovery sync"
            );
            let config = toml_support::read_config(&root.config_toml())?
                .context("missing previous config for recovery sync")?;
            ensure!(
                root.as_path().join(&config.runtime.prod_env) == path,
                "previous configuration has a different production environment"
            );
            let python = config
                .runtime
                .python
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .context("missing previous Python for recovery sync")?;
            let mut write = ProductionWrite { root, operation };
            UvClient::new(python)
                .sync_prod_tracked(root, &config, |process| write.record_process(process))
                .context("recovery sync failed")?;
        }
    }
    Ok(())
}

fn checked_new_environment(root: &ProjectRoot, path: &std::path::Path) -> Result<()> {
    let resolved = fs_support::resolved_path(path)?;
    let project = root.as_path().canonicalize()?;
    let state = fs_support::resolved_path(&root.state_dir())?;
    ensure!(
        resolved != project
            && resolved.starts_with(&project)
            && !resolved.starts_with(&state)
            && !state.starts_with(&resolved),
        "new production environment must be inside the project and outside state: {}",
        path.display()
    );
    Ok(())
}

fn finish_link(root: &ProjectRoot, operation: &mut OperationRecord) -> Result<()> {
    let recovery = operation
        .recovery
        .as_mut()
        .context("missing recovery evidence")?;
    if recovery.linked {
        return Ok(());
    }
    let id = OpId(operation.op_id.clone());
    let cleanup = match &recovery.link {
        Some(OperationLink::Promotion {
            tx_id,
            reason,
            keep_artifacts,
        }) => {
            let mut tx = state_ledger::load_transaction(root, tx_id)?;
            let linked = tx.promotion.as_ref().and_then(|p| p.op_id.as_deref()) == Some(&id.0)
                && tx.status == Some(TxStatus::Promoted);
            if !linked {
                if !matches!(
                    tx.status,
                    Some(TxStatus::Completed | TxStatus::Resolved | TxStatus::Failed)
                ) {
                    bail!(
                        "transaction {} has an unexpected status; preserve the committed operation",
                        tx_id.0
                    );
                }
                tx.status = Some(TxStatus::Promoted);
                tx.updated_at = Some(state_ledger::now_string()?);
                tx.promotion = Some(PromotionRecord {
                    status: TxStatus::Promoted,
                    reason: reason.clone(),
                    op_id: Some(id.0.clone()),
                    error: None,
                });
                state_ledger::save_transaction(root, &tx)?;
            }
            (!keep_artifacts).then_some(tx)
        }
        Some(OperationLink::Undo { op_id }) => {
            let mut target = state_ledger::load_operation(root, op_id)?;
            if target.status != OpStatus::Undone || target.undo_reference.as_deref() != Some(&id.0)
            {
                ensure!(
                    target.status == OpStatus::Success && target.undoable,
                    "undo target {} has an unexpected status; preserve the committed undo",
                    op_id.0
                );
                state_ledger::mark_operation_undone(root, &mut target, &id)?;
            }
            None
        }
        None => return Ok(()),
    };
    recovery.linked = true;
    state_ledger::save_operation(root, operation)?;
    if let Some(tx) = cleanup {
        match toml_support::read_config(&root.config_toml()) {
            Ok(Some(config)) => super::tx::clean_artifacts(root, &config, &tx),
            _ => eprintln!(
                "warning: could not read configuration to clean artifacts for {}",
                tx.id
            ),
        }
    }
    Ok(())
}
