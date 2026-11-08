use crate::cli::OpInspectArgs;
use crate::domain::{AppResult, OpId, ProjectRoot};
use crate::state_ledger;

pub fn cmd_op_list(root: &ProjectRoot) -> AppResult<()> {
    let operations = state_ledger::list_operations(root)?;
    if operations.is_empty() {
        println!("No operations recorded.");
        return Ok(());
    }

    for operation in operations {
        println!(
            "{}\t{}\t{}\t{}\t{}{}",
            operation.op_id,
            operation.kind,
            operation.status,
            display_value(&operation.reference),
            display_value(&operation.started_at),
            if operation.needs_recovery() {
                "\tneeds_recovery"
            } else {
                ""
            },
        );
    }
    Ok(())
}

pub fn cmd_op_inspect(args: OpInspectArgs, root: &ProjectRoot) -> AppResult<()> {
    let operation = state_ledger::load_operation(root, &OpId(args.op_id))?;
    println!("op_id: {}", operation.op_id);
    println!("kind: {}", operation.kind);
    println!("status: {}", operation.status);
    println!("reference: {}", display_value(&operation.reference));
    println!("started_at: {}", display_value(&operation.started_at));
    println!("ended_at: {}", operation.ended_at.as_deref().unwrap_or("-"));
    println!("backup_dir: {}", operation.backup_dir);
    println!("undoable: {}", operation.undoable);
    println!("needs_recovery: {}", operation.needs_recovery());
    if let Some(recovery) = &operation.recovery {
        println!("recovery_phase: {:?}", recovery.phase);
        if let Some(error) = &recovery.apply_error {
            println!("apply_error: {error}");
        }
        if let Some(error) = &recovery.restore_error {
            println!("restore_error: {error}");
        }
    }

    println!("files:");
    for (name, hashes) in operation.files {
        println!(
            "  {} pre={} post={}",
            name,
            hashes.pre_sha256.as_deref().unwrap_or("-"),
            hashes.post_sha256.as_deref().unwrap_or("-"),
        );
    }

    if let Some(note) = operation.note {
        println!("note: {note}");
    }
    if let Some(reference) = operation.undo_reference {
        println!("undo_reference: {reference}");
    }

    Ok(())
}

fn display_value(value: &str) -> &str {
    if value.trim().is_empty() { "-" } else { value }
}
