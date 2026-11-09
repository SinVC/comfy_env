# UC-003 Undo Successful Operation

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Goal / Actor / Trigger

- Goal: restore local truth to the backup captured before a prior successful destructive operation.
- Actor: local operator.
- Trigger: `gov undo <op_id>`.

## Preconditions / Postconditions

- Preconditions:
  - Target operation exists.
  - Target operation is `status=success` and `undoable=true`.
  - Current hashes of `pyproject.toml`, `uv.lock`, and `state/plugins.json` match the target op's recorded post hashes.
- Postconditions:
  - Success path: backup files are restored, prod is re-synced, the target op is marked `undone`, and a new successful `manual` op records the undo.

## Main Path

1. Load target op metadata.
2. Verify current local truth hashes against target post hashes.
3. Create a new backup for the undo action itself.
4. Restore the target op's backup files.
5. Exact-sync `.venv-prod`.
6. Mark the target op as `undone`.
7. Finalize the undo operation as successful.

## Alternative / Failure Paths

1. If the target op is not undoable, exit without mutation.
2. If hashes diverge, block the undo to avoid overwriting drift.
3. If target backup is missing, restore the undo-op backup and fail the undo.
4. If prod sync fails after restore, roll back using the undo-op backup and fail the undo.

## Data & Side Effects

- Reads and writes operation metadata.
- Restores backed-up `pyproject.toml`, `uv.lock`, and `plugins.json`.
- Rebuilds `.venv-prod`.

## Referenced Contracts / Flows

- [Application Core Contracts](../contracts.md)
- [State Ledger Contracts](../../subsystems/state-ledger/contracts.md)
- [Rollback Safety Policy](../../policies/rollback-safety.md)

## Acceptance Checks

- A successful undo produces a new `manual` operation marked `success`.
- The target operation becomes `status=undone` and `undoable=false`.
- Hash mismatch blocks the undo before any write occurs.
