# System Key Flows

## Metadata

- Status: Active
- Last Reviewed: 2026-04-07

## Hero Flow

- Hero flow: onboard one plugin safely into production.
  - Register source with `node add`.
  - Observe dependency effect in a candidate transaction with `tx run`.
  - Inspect diff and core impact with `tx inspect`.
  - Promote into local truth and `.venv-prod` with `tx promote`.

## SKF List

- SKF-001: Plugin onboarding to production
- SKF-002: Conflict-driven re-promotion
- SKF-003: Reversible removal and undo
- SKF-004: Runtime bootstrap and core dependency upgrade
- SKF-005: Exact environment handoff

## SKF Details

### SKF-001

- Module Boundaries:
  - Application Core -> Source Integration -> State Ledger -> Dependency Sync -> Runtime Executor -> Safety Guards
- Key Contracts:
  - CLI command contract
  - Transaction record contract
  - Operation backup contract
- Failure & Recovery Posture:
  - `tx run` may end `failed` but still records diff.
  - `tx promote` blocks on core impact unless approved.
  - Failed prod sync or smoke restores pre-op files before exit.

### SKF-002

- Module Boundaries:
  - Application Core -> Dependency Sync -> State Ledger -> Safety Guards
- Key Contracts:
  - Conflict report contract
  - Resolution pin contract
- Failure & Recovery Posture:
  - Promote lock failure writes `state/conflicts/<txid>.json` and moves transaction to `needs_resolution`.
  - `resolve` merges new pins, retries workdir lock, and either marks `resolved` or re-emits conflict state.

### SKF-003

- Module Boundaries:
  - Application Core -> State Ledger -> Dependency Sync -> Safety Guards -> Source Integration
- Key Contracts:
  - Operation metadata contract
  - Undo hash guard contract
- Failure & Recovery Posture:
  - `node remove` always starts with an op backup.
  - `undo` refuses to run if current local truth hashes do not match the target op's post hashes.
  - `--purge-code` deletes source trees outside the undoable contract.

### SKF-004

- Module Boundaries:
  - Application Core -> Dependency Sync -> State Ledger -> Safety Guards -> Runtime Executor
- Key Contracts:
  - CLI command contract
  - Transaction record contract (`kind=core_update`)
  - Operation metadata contract
- Failure & Recovery Posture:
  - `init` must persist `paths.comfyui_dir` and `runtime.python` before later commands assume them.
  - `install torch` must happen before `install`.
  - `update run` stages a workdir from `requirements.txt`; `update promote` only promotes that staged snapshot.
  - Failed sync or smoke restores pre-op files before exit.

### SKF-005

- Module Boundaries:
  - Application Core -> Dependency Sync -> Source Integration -> State Ledger -> Safety Guards -> Runtime Executor
- Key Contracts:
  - Environment handoff contract
  - Bundle manifest and checksum contract
  - Operation backup contract
- Failure & Recovery Posture:
  - `env export` fails closed on missing truth files, missing plugin source directories, or non-empty output directory.
  - `env import` validates manifest integrity and runtime compatibility before mutating root truth.
  - Failed import restores pre-op truth and any `custom_nodes` directories that were overwritten or purged.

## Integration Contracts

| Contract | Owner Module | Consumer Module | Link |
|---|---|---|---|
| CLI command contract | Application Core | Local operator | [application-core/contracts.md](../application-core/contracts.md) |
| Transaction and operation records | State Ledger | Application Core, Safety Guards | [state-ledger/contracts.md](../subsystems/state-ledger/contracts.md) |
| Shared IDs and statuses | Shared docs | All modules | [shared-types.md](../contracts/shared-types.md) |
