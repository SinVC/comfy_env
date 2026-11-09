# Comfy Env Architecture Haiku

## Metadata

- Status: Active
- Last Reviewed: 2026-04-17

## 1. System Identity / Goals / Non-Goals / Assumptions

- System Identity: `comfy_env` is a sidecar governance CLI that controls how ComfyUI custom-node dependency changes are observed, promoted, rolled back, and run.
- Goals:
  - Make local dependency state reproducible from `pyproject.toml` and `uv.lock`.
  - Make runtime prerequisites explicit through `gov init --comfyui-dir --python`.
  - Force plugin dependency changes through an observable candidate transaction before production promotion.
  - Handle ComfyUI core requirements through `install` and `update` flows without bypassing local truth.
  - Allow direct exact-version compatibility pins through shared `dependency-groups.overrides` without waiting for a conflict-resolution flow.
  - Hand off a verified environment through `env export` and `env import` without re-cloning plugins or redefining local truth ownership.
  - Preserve auditable state for transactions, operations, conflicts, and plugin registration.
  - Prefer automatic recovery before leaving local truth in a partially changed state.
- Non-Goals:
  - It does not manage ComfyUI application logic or node business behavior.
  - It does not provide concurrent multi-user coordination or remote orchestration.
  - It does not restore purged plugin source code during `undo`.
  - It does not preserve source-machine `comfyui_dir` as bundle truth or ship VCS admin metadata as part of source handoff.
- Assumptions:
  - `uv`, `git`, and Python are available on the host.
  - `comfy_env` and `ComfyUI` run in sidecar layout, with `paths.comfyui_dir` pointing at the managed ComfyUI root.
  - Local state files are trusted as single-user workstation state, not shared source-of-truth in Git.

## 2. Scope Boundary & Context

- Actors:
  - Local operator running `bin/gov`.
  - ComfyUI runtime launched by `gov run` or exercised indirectly by `gov tx run`.
  - External plugin Git repositories.
- External Dependencies:
  - `uv` for lock/sync/add/remove/freeze.
  - `git` for clone/checkout.
  - Host OS process control for timeout and signals.
  - ComfyUI entrypoint at `main.py`.
- Trust Boundaries:
  - `pyproject.toml`, `uv.lock`, and `state/*` are local truth inside the sidecar.
  - Plugin repositories and ComfyUI source tree are external inputs.
  - `uv` solver output is trusted only after lock/sync succeeds.
- C4 L1 Summary:
  - `application-core` accepts CLI commands and orchestrates use cases.
  - Peer subsystems own state semantics and safety rules.
  - Infrastructure adapters translate those use cases into `uv`, `git`, filesystem, and process operations.

## 3. Architectural Invariants

1. All production dependency mutations are anchored in local truth files first, then applied to `.venv-prod`.
2. A transaction is the only supported observation unit for plugin dependency impact before promotion.
3. Destructive state changes (`promote`, `remove`, `undo`) must create or use operation backups before finalizing.
4. Lock conflicts must surface as explicit conflict artifacts and a resolvable transaction state, not silent partial success.
5. Core package drift is policy-gated and cannot promote without explicit approval.
6. `state/plugins.json` is registry metadata; dependency-group content in `pyproject.toml` remains authoritative for actual dependency removal.
7. `env export` / `env import` bundles are transport artifacts; after import, local truth remains root `pyproject.toml + uv.lock`, while `paths.comfyui_dir` stays target-local configuration.
8. Bundle source snapshots represent runtime working trees and must exclude VCS admin metadata such as `.git/`.

## 4. Top-Level Decomposition

- Application Core:
  - [Governance CLI Orchestrator](./application-core/spec.md): command dispatch plus surface use cases for bootstrap/install, global override pin management, plugin lifecycle, plugin tx, core update tx, audit/undo, and runtime control.
- Peer Subsystems:
  - [State Ledger](./subsystems/state-ledger/spec.md): durable local records for transactions, operations, plugin registry, and conflict artifacts (I1).
  - [Safety Guards](./subsystems/safety-guards/spec.md): core-impact gate, backup discipline, rollback posture, undo hash guard (I5).
- Infrastructure Adapters:
  - [Dependency Sync](./subsystems/dependency-sync/spec.md): `uv` lock/sync/add/remove/freeze, `pylock.toml` export, bundle compatibility checks, and environment materialization (I2).
  - [Source Integration](./subsystems/source-integration/spec.md): plugin clone/checkout, install-path mapping, and runtime snapshot export/import for `custom_nodes` (I3).
  - [Runtime Executor](./subsystems/runtime-executor/spec.md): candidate/prod Python execution, ComfyUI launch, timeout, PID, logs (I4).

## 5. Contracts & Interfaces Index

| Contract | Owner | Consumer | Link |
|---|---|---|---|
| CLI command and use-case contract | Application Core | Local operator | [application-core/contracts.md](./application-core/contracts.md) |
| Local state record contract | State Ledger | Application Core, Safety Guards, Adapters | [state-ledger/contracts.md](./subsystems/state-ledger/contracts.md) |
| Cross-module compatibility index | Architecture docs | All modules | [inter-subsystem-contracts.md](./contracts/inter-subsystem-contracts.md) |
| Shared identifiers and state enums | Shared docs | All modules | [shared-types.md](./contracts/shared-types.md) |

## 6. Key Flows Index

- Hero Flow:
  - [SKF-001 plugin onboarding to production](./key-flows/system.md#skf-001)
- SKF List:
  - [SKF-001](./key-flows/system.md#skf-001) `node add -> tx run -> inspect -> promote`
  - [SKF-002](./key-flows/system.md#skf-002) `promote lock conflict -> resolve -> re-promote`
  - [SKF-003](./key-flows/system.md#skf-003) `node remove/undo with backup restore`
  - [SKF-004](./key-flows/system.md#skf-004) `init -> install torch -> install -> update run -> update promote`
  - [SKF-005](./key-flows/system.md#skf-005) `env export -> env import`
- Module KF Index:
  - Application Core: `core#KF-001..011` in [application-core/spec.md](./application-core/spec.md#key-flows--failure-recovery)
  - State Ledger: `state-ledger#KF-001..003` in [state-ledger/spec.md](./subsystems/state-ledger/spec.md#key-flows--failure-recovery)
  - Safety Guards: `safety-guards#KF-001..003` in [safety-guards/spec.md](./subsystems/safety-guards/spec.md#key-flows--failure-recovery)
- UC Index:
  - [UC-001 Manage plugin through transaction](./application-core/use-cases/UC-001-manage-plugin-through-transaction.md)
  - [UC-002 Remove plugin with reversible state](./application-core/use-cases/UC-002-remove-plugin-with-reversible-state.md)
  - [UC-003 Undo successful operation](./application-core/use-cases/UC-003-undo-successful-operation.md)
  - [UC-008 Install managed runtime dependencies](./application-core/use-cases/UC-008-install-managed-runtime-dependencies.md)
  - [UC-009 Transactional update of ComfyUI core requirements](./application-core/use-cases/UC-009-transactional-update-of-comfyui-core-requirements.md)

## 7. Data Sovereignty & Integration

- Source of Truth by Domain:
  - Dependency truth: [`pyproject.toml`, `uv.lock`](./data/spec.md)
  - Plugin registry truth: [`state/plugins.json`](./subsystems/state-ledger/contracts.md)
  - Transaction truth: [`state/transactions/*.json`](./subsystems/state-ledger/contracts.md)
  - Operation backup truth: [`state/ops/<op_id>/`](./subsystems/state-ledger/contracts.md)
  - Runtime liveness hint: `state/comfyui.pid` in [operations/spec.md](./operations/spec.md)
  - Transfer artifact: `env export` bundle (`manifest.json`, `pylock.toml`, `state/plugins.json`, `custom_nodes/*`, audit files)
- Integration Boundaries:
  - `git` only touches plugin source directories.
  - `uv` owns lock calculation and virtualenv synchronization.
  - ComfyUI runtime is an external executable target; `comfy_env` only launches, times out, and stops it.

## 8. Cross-Cutting Policies Index

- Policy Index Links:
  - [Core Impact Gate](./policies/core-impact-gate.md)
  - [Rollback Safety](./policies/rollback-safety.md)

## 9. NFR

- Reliability: failure paths prefer restore + re-sync over partial success; see [nfr/spec.md](./nfr/spec.md).
- Performance: lock/sync and runtime startup are host-bound, so the system optimizes for correctness over latency; see [nfr/spec.md](./nfr/spec.md).
- Security: local-only trust, no secret distribution, limited to shelling out into known tools; see [nfr/spec.md](./nfr/spec.md).
- Observability: transactions, operations, conflict reports, and logs are first-class artifacts; see [operations/spec.md](./operations/spec.md).

## 10. Glossary / ADR Index / Open Questions / Risk Register

- Glossary:
  - Transaction: candidate execution record before promotion.
  - Operation: backup-protected state mutation with optional undo.
  - Promote: applying transaction-derived dependency changes to local truth and `.venv-prod`.
- ADR Index:
  - [ADR-001 Global Pin Management via `dependency-groups.overrides`](./adr/001-global-pin-management.md)
  - [ADR-002 重写语言选择讨论稿](./adr/002-rewrite-language-selection.md)
  - [ADR-003 Rust 重写方案](./adr/003-rust-rewrite-plan.md)
- Open Questions:
  - The codebase is a single shell entrypoint; future extraction into separate scripts may require re-drawing module boundaries from logical to physical modules.
  - Plugin `resolve` still uses interactive stdin while `update resolve` is parameterized; the long-term convergence path is still open.
- Risk Register:
  - No explicit file locking, so overlapping local invocations can race on the same state files.
  - `undo` protects local truth files but intentionally excludes purged source trees.
  - `env import` is an exact-restore flow and will remove bundle-external `custom_nodes/*` directories unless they are restored during rollback.

## Module Cards

### Governance CLI Orchestrator

**Type**: Application Core

**Role / Responsibility**
- Owns command routing and the nine user-visible surfaces S1-S9.
- Does NOT own tool-specific semantics or record schema internals.

**Owned Data / Source of Truth**
- In-memory command intent only.

**Boundary & Dependency Rules**
- Allowed: consume State Ledger, Safety Guards, and adapters.
- Forbidden: bypass backup/restore or treat plugin source as dependency truth.

**Implements / Consumes**
- Implements CLI command ports in `bin/gov`.
- Consumes local record contracts plus `uv`, `git`, and ComfyUI adapters.

**Orchestration Style**
- Single-command imperative flows with explicit failure branches.

**Key Behaviors (Index)**
- `core#KF-001..011`

**Drill-down**
- [Spec](./application-core/spec.md)
- [Contracts](./application-core/contracts.md)

### State Ledger

**Type**: Peer Subsystem

**Role / Responsibility**
- Owns record schemas and lifecycle for plugin, transaction, operation, and conflict artifacts.

**Owned Data / Source of Truth**
- `state/plugins.json`, `state/transactions/*`, `state/ops/*`, `state/conflicts/*`

**Boundary & Dependency Rules**
- Allowed: local persistence and normalization.
- Forbidden: external tool execution.

**Implements / Consumes**
- Implements shared local artifact contracts.
- Consumes filesystem plus timestamp/hash helpers.

**Orchestration Style**
- Persist explicit lifecycle state; fail closed on missing critical records.

**Key Behaviors (Index)**
- `state-ledger#KF-001..003`

**Drill-down**
- [Spec](./subsystems/state-ledger/spec.md)
- [Contracts](./subsystems/state-ledger/contracts.md)

### Safety Guards

**Type**: Peer Subsystem

**Role / Responsibility**
- Owns approval, backup, restore, and drift-check rules around destructive mutation.

**Owned Data / Source of Truth**
- Guard decisions derived from transaction diff, op backup hashes, and command flags.

**Boundary & Dependency Rules**
- Allowed: inspect ledger state and invoke restore.
- Forbidden: tolerate drift or skip backup on destructive paths.

**Implements / Consumes**
- Implements core-impact gating and undo safety.
- Consumes State Ledger data plus Dependency Sync outcomes.

**Orchestration Style**
- Preflight gate, then guarded mutation or rollback.

**Key Behaviors (Index)**
- `safety-guards#KF-001..003`

**Drill-down**
- [Spec](./subsystems/safety-guards/spec.md)

### Dependency Sync

**Type**: Infrastructure Adapter

**Role / Responsibility**
- Owns translation into `uv` lock/sync/add/remove/freeze behavior.

**Owned Data / Source of Truth**
- No domain truth; consumes local truth files and env paths.

**Boundary & Dependency Rules**
- Allowed: invoke `uv`.
- Forbidden: decide policy approval.

**Implements / Consumes**
- Implements lock, sync, freeze, and workdir mutation ports.
- Consumes plans and dependency-group inputs.

**Orchestration Style**
- Batch-transform a workdir, then materialize env state.

**Key Behaviors (Index)**
- `dependency-sync#KF-001..009`

**Drill-down**
- [Spec](./subsystems/dependency-sync/spec.md)

### Source Integration

**Type**: Infrastructure Adapter

**Role / Responsibility**
- Owns plugin clone/checkout and install-path mapping into `custom_nodes`.

**Owned Data / Source of Truth**
- No domain truth; consumes plugin registry fields and external Git refs.

**Boundary & Dependency Rules**
- Allowed: invoke `git` and compute install paths.
- Forbidden: treat cloned source as dependency truth.

**Implements / Consumes**
- Implements clone, checkout, and path-resolution ports.
- Consumes `paths.comfyui_dir` plus registry metadata.

**Orchestration Style**
- Materialize source tree, then hand off to ledger-backed flows.

**Key Behaviors (Index)**
- `source-integration#KF-001..005`

**Drill-down**
- [Spec](./subsystems/source-integration/spec.md)

### Runtime Executor

**Type**: Infrastructure Adapter

**Role / Responsibility**
- Owns candidate/prod process execution, timeout, logs, PID write, and stop signals.

**Owned Data / Source of Truth**
- Runtime logs and `state/comfyui.pid`.

**Boundary & Dependency Rules**
- Allowed: spawn and signal processes.
- Forbidden: mutate dependency truth except through requested sync paths.

**Implements / Consumes**
- Implements candidate run, prod run, and stop ports.
- Consumes env paths, `main.py`, and run config.

**Orchestration Style**
- Prepare env path, execute, capture output, or terminate.

**Key Behaviors (Index)**
- `runtime-executor#KF-001..003`

**Drill-down**
- [Spec](./subsystems/runtime-executor/spec.md)
