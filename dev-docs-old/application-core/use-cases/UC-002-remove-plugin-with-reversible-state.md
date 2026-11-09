# UC-002 Remove Plugin With Reversible State

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Goal / Actor / Trigger

- Goal: remove a plugin's managed dependencies from production while preserving an undoable backup of local truth.
- Actor: local operator.
- Trigger: `gov node remove <node_id> [--purge-code]`.

## Preconditions / Postconditions

- Preconditions:
  - Plugin metadata exists in `state/plugins.json`.
  - `uv` is available.
- Postconditions:
  - Success path: local truth is updated, prod is resynced, plugin registry entry is removed, and an undoable operation is finalized.
  - Optional `--purge-code`: plugin source directory is physically removed and stays outside future undo scope.

## Main Path

1. Resolve plugin metadata, group name, and install path.
2. Create an operation backup.
3. Build a throwaway workdir from current local truth.
4. Remove dependency-group entries from the workdir and lock.
5. Copy updated `pyproject.toml` and `uv.lock` back to root.
6. Exact-sync `.venv-prod`.
7. Remove plugin registry entry.
8. Optionally purge plugin source code.
9. Finalize operation as `success`.

## Alternative / Failure Paths

1. If plugin metadata is missing, the command exits before mutation.
2. If workdir lock fails, restore from backup and finalize the operation as failed.
3. If prod sync fails, restore backup, re-sync prod back to pre-op, and finalize as failed.

## Data & Side Effects

- Writes an operation directory under `state/ops`.
- Rewrites local truth and plugin registry on success.
- May delete the plugin source directory on `--purge-code`.

## Referenced Contracts / Flows

- [Application Core Contracts](../contracts.md)
- [State Ledger Contracts](../../subsystems/state-ledger/contracts.md)
- [Rollback Safety Policy](../../policies/rollback-safety.md)

## Acceptance Checks

- `op list` shows a successful, undoable `remove` operation.
- `pyproject.toml` no longer contains the plugin dependency group entries.
- `state/plugins.json` no longer includes the `node_id`.
- If `--purge-code` was used, docs explicitly treat source deletion as non-undoable.
