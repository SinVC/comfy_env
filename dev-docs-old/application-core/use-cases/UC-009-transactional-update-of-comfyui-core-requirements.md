# UC-009 Transactional Update Of ComfyUI Core Requirements

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Goal / Actor / Trigger

- Goal: update ComfyUI base requirements through an observable staged transaction before mutating production truth.
- Actor: local operator.
- Trigger: `gov update run`, followed by `gov update inspect`, optional `gov update resolve`, and `gov update promote` or `gov update abort`.

## Preconditions / Postconditions

- Preconditions:
  - `gov init --comfyui-dir --python` has completed.
  - `gov install torch` and `gov install` have completed at least once.
- Postconditions:
  - Success path: a `kind=core_update` transaction is recorded and a successful promote updates `pyproject.toml`, `uv.lock`, and `.venv-prod`.
  - Conflict path: the transaction remains in `needs_resolution` with a conflict artifact.

## Main Path

1. Read `${comfyui_dir}/requirements.txt` (or the supplied override path).
2. Build a staged workdir by rewriting `dependency-groups.core`.
3. Lock the staged workdir with `runtime.python`.
4. Sync the staged workdir into a candidate env and run ComfyUI.
5. Record a `kind=core_update` transaction including requirements path, hash, staged workdir, and diff.
6. Promote the staged snapshot into root truth only after approval.

## Alternative / Failure Paths

1. If the staged lock fails, write a conflict report and persist a `needs_resolution` transaction.
2. `update resolve` accepts only parameterized pins (`--pin`, `--pins-file`), merges them into `overrides`, and retries lock.
3. If prod sync or smoke fails during `update promote`, restore pre-op truth and mark the transaction `promote_failed`.
4. `update abort` deletes the candidate env and staged workdir, then marks the transaction `aborted`.

## Acceptance Checks

1. `update inspect <txid>` shows `kind: core_update`.
2. A successful `update promote` yields `status=promoted` and an undoable `update_promote` operation.
3. `undo <op_id>` restores the prior core dependency truth after a successful promote.
