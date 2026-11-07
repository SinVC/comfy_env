use anyhow::Context;

use crate::cli::UndoArgs;
use crate::domain::{AppError, AppResult, OpId, OpStatus, ProjectRoot};
use crate::safety_guards::{self, DriftCheck};
use crate::state_ledger::{self, OpKind};

pub fn cmd_undo(args: UndoArgs, root: &ProjectRoot) -> AppResult<()> {
    let (config, uv) = super::configured_uv(root)?;
    let target_id = OpId(args.op_id);
    let mut target = state_ledger::load_operation(root, &target_id)?;

    if target.status != OpStatus::Success || !target.undoable {
        return Err(AppError::usage(format!(
            "operation {} is not undoable",
            target_id.0
        )));
    }

    match safety_guards::check_undo_drift(&target, root)? {
        DriftCheck::Clean => {}
        DriftCheck::Drifted(files) => {
            return Err(AppError::usage(format!(
                "hash drift detected: {}",
                files.join(", ")
            )));
        }
    }

    let undo_op =
        safety_guards::op_begin(OpKind::ManualUndo, &format!("undo:{}", target_id.0), root)?;

    let undo_result = (|| -> anyhow::Result<()> {
        safety_guards::op_restore(&target_id, root)?;
        uv.sync_prod(root, &config)
            .context("prod sync failed during undo")?;
        safety_guards::op_finalize(&undo_op, true, None, root)?;
        state_ledger::mark_operation_undone(root, &mut target, &undo_op)
    })();

    match undo_result {
        Ok(()) => {
            super::prune_history(root, &config);
            println!("Undo completed: {}", target_id.0);
            Ok(())
        }
        Err(error) => Err(super::rollback_failed_operation(
            root, &undo_op, &config, &uv, error,
        )),
    }
}
