# UC-001 Manage Plugin Through Transaction

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Goal / Actor / Trigger

- Goal: move one plugin from external source into an observed candidate state and then into production local truth.
- Actor: local operator.
- Trigger: `gov node add`, followed by `gov tx run`, optional `gov tx inspect`, and `gov tx promote` or `gov resolve`.

## Preconditions / Postconditions

- Preconditions:
  - `paths.comfyui_dir` points to a valid ComfyUI tree.
  - `uv`, `git`, and Python are available.
  - The target `node_id` is not already installed.
- Postconditions:
  - Success path: plugin is cloned, transaction is persisted, and a successful promote updates `pyproject.toml`, `uv.lock`, `.venv-prod`, and plugin metadata.
  - Conflict path: transaction remains in `needs_resolution` or `resolved` with conflict artifacts and pins.

## Main Path

1. Clone plugin source into `custom_nodes` and register metadata.
2. Create a candidate env and sync from current lock.
3. Run ComfyUI inside the candidate env and collect pre/post freeze snapshots.
4. Inspect diff and core impact.
5. Generate a promotion plan from transaction diff and plugin requirements.
6. Apply the plan in a workdir, lock, then copy workdir truth back to root.
7. Sync prod and run smoke validation.
8. Finalize transaction and operation as promoted/success.

## Alternative / Failure Paths

1. If `tx run` returns non-zero, the transaction is still recorded as `failed`; promotion requires `--allow-failed-run`.
2. If lock fails during promote, emit a conflict report and move to `needs_resolution`.
3. If `resolve` pins still do not lock, keep `needs_resolution` and refresh the conflict report.
4. If prod sync or smoke fails after backup, restore pre-op truth and mark `promote_failed`.

## Data & Side Effects

- Writes plugin registry, transaction JSON, logs, conflict report, workdir files, and operation metadata.
- Mutates local truth only after lock success in the promote workdir.
- Produces external side effects in `.venv-candidate`, `.venv-prod`, and plugin source clone path.

## Referenced Contracts / Flows

- [Application Core Contracts](../contracts.md)
- [State Ledger Contracts](../../subsystems/state-ledger/contracts.md)
- [System Key Flows](../../key-flows/system.md)

## Acceptance Checks

- Plugin exists under `custom_nodes/<node_id>`.
- `tx inspect <txid>` shows expected diff/core-impact fields.
- Successful promote yields `status=promoted` and a successful `op_id`.
- Failed promote never leaves local truth half-updated without either conflict state or rollback.
