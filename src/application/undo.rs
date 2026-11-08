use anyhow::Context;

use crate::cli::UndoArgs;
use crate::domain::{AppError, AppResult, OpId, OpStatus, ProjectRoot};
use crate::safety_guards::{self, DriftCheck};
use crate::state_ledger::{self, OpKind};

pub fn cmd_undo(args: UndoArgs, root: &ProjectRoot) -> AppResult<()> {
    let (config, uv) = super::configured_uv(root)?;
    let target_id = OpId(args.op_id);
    let reference = format!("undo:{}", target_id.0);
    if super::production::resume_link(root, OpKind::ManualUndo, &reference)? {
        return Ok(());
    }
    let target = state_ledger::load_operation(root, &target_id)?;

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

    let link = crate::state_ledger::OperationLink::Undo {
        op_id: target_id.clone(),
    };
    let source_touched = root
        .ops_dir()
        .join(&target_id.0)
        .join("backup/custom_node.path")
        .try_exists()?;
    safety_guards::verify_backup(&target_id, root, source_touched)?;
    super::production::commit(
        root,
        OpKind::ManualUndo,
        &reference,
        Some(link),
        |op| safety_guards::backup_undo_custom_node(&OpId(op.op_id.clone()), &target_id, root),
        |write| {
            if source_touched {
                write.touch_source()?;
            }
            safety_guards::op_restore(&target_id, root, source_touched)?;
            write.sync(&config, &uv).context("during undo")
        },
    )?;
    super::prune_history(root, &config);
    println!("Undo completed: {}", target_id.0);
    Ok(())
}
