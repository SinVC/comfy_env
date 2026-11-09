# Rollback Safety Policy

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Scope

This policy governs destructive flows that can alter local truth or prod environment state: `tx promote`, `node remove`, and `undo`.

## Rules

- Every destructive flow must start with `op_begin` before copying new truth into root files.
- If a destructive flow fails after backup, the system must attempt `op_restore_backup` before returning control.
- Successful destructive flows must record post hashes through `op_finalize`.
- `undo` must verify current hashes match the target op's post hashes before restore.
- Source deletion from `--purge-code` is explicitly excluded from the rollback contract.

## Control Points

- `cmd_node_remove`
- `cmd_tx_promote`
- `cmd_undo`
- `op_begin`, `op_restore_backup`, `op_finalize`

## Exceptions

- Non-destructive reads (`status`, `tx inspect`, `op list`, `op inspect`) do not create operations.
- `tx run` creates no operation backup because it does not mutate local truth files.

## Verification

- Inspect `state/ops/<op_id>/meta.json` after successful remove/promote/undo.
- Force a lock or sync failure and verify the command restores pre-op truth before exit.
- Attempt `undo` after manually changing local truth and verify it blocks on hash mismatch.
