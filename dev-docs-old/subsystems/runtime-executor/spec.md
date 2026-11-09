# Runtime Executor Spec

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Scope & External System Profile

This adapter owns process execution semantics for candidate and production environments. It runs Python from the chosen virtualenv, targets `ComfyUI/main.py`, captures logs, and manages the optional PID file used by `gov stop`.

The external systems are:

- host process scheduler and signals
- optional `timeout` utility
- the ComfyUI Python entrypoint

## Data Mapping (Port/API/Event)

- Input ports:
  - candidate and prod env paths
  - `main.py` path
  - timeout seconds
  - run args and config-derived extra args
  - PID file path
- Output ports:
  - stdout/stderr log files for candidate runs
  - run exit code inside transaction JSON
  - `state/comfyui.pid`

## Error Translation (Infra -> Domain/Application)

- missing `main.py` becomes explicit command failure or a transaction `failed` run
- timed or non-zero candidate execution becomes `status=failed` while still preserving the transaction record
- stale PID file becomes cleanup plus a non-error stop result

## Integration Behaviors / Key Flows

- `runtime-executor#KF-001` Timed candidate execution
  - run `main.py` inside the candidate env, optionally through `timeout`, and capture logs
- `runtime-executor#KF-002` Foreground prod execution
  - optionally re-sync prod, write PID, then `exec` the ComfyUI process
- `runtime-executor#KF-003` Graceful then forced stop
  - send `SIGTERM`, wait up to 30 seconds, then `SIGKILL` if still alive

## Runtime / Connectivity Constraints

- Candidate run depends on the candidate env already being synced.
- `gov run` replaces the shell process with `exec`, so control returns only when ComfyUI exits.
- PID tracking works only for processes started via `gov run`.

## Schema / DDL

- Not applicable. This adapter owns runtime behavior, not persistent data models.

## Code Anchors

| Doc ID | path | symbol | line |
|---|---|---|---|
| RE-001 | `bin/gov` | `prod_env_path` | 135 |
| RE-002 | `bin/gov` | `candidate_root_path` | 141 |
| RE-003 | `bin/gov` | `tx_timeout_seconds` | 151 |
| RE-004 | `bin/gov` | `cmd_tx_run` | 1484 |
| RE-005 | `bin/gov` | `cmd_run` | 1990 |
| RE-006 | `bin/gov` | `cmd_stop` | 2068 |
