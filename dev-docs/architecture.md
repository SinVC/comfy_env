# Architecture Overview

## System Identity

`comfy_env` (`gov`) is a sidecar governance CLI that controls how ComfyUI custom-node dependency changes are observed, promoted, rolled back, and run.

- **Language**: Rust, single binary, single crate
- **Platforms**: Linux + Windows
- **External dependencies**: `uv`, `git`, Python (all via subprocess, never embedded)

## Document Hierarchy

When documents conflict, resolve by priority (highest first):

1. **architecture.md** — system-level invariants and rules
2. **commands/*.md** — command-level behavioral contracts
3. **modules/*.md** — module-level implementation specs

Higher-level documents constrain lower-level ones. If a module spec contradicts an invariant in architecture.md, fix the module spec.

## AI Reading Protocol

This doc set is optimized for AI-assisted implementation and review. It is intentionally split so an agent can load only the minimum relevant context for the task at hand.

Read in this order:

1. `architecture.md` for system invariants, module boundaries, and cross-platform rules
2. One relevant file from `commands/` for the command-level behavioral contract
3. Only the module specs referenced by that command contract
4. The direct implementation target links at the top of those command/module docs

Rules:

- Do not preload unrelated command docs or module specs "for background" unless the current task actually crosses those boundaries.
- Prefer direct implementation links over repository-wide exploration when a doc already tells you where the code belongs.
- Treat the docs as the behavioral authority during migration; linked Rust files may still be scaffolded and therefore incomplete.
- Keep context isolated: command docs define what a command must do, module docs define how a subsystem works, and unrelated slices should stay unread.

## Invariants

1. All production dependency mutations are anchored in local truth files (`pyproject.toml`, `uv.lock`) first, then applied to `.venv-prod`.
2. A transaction is the only supported observation unit for plugin dependency impact before promotion.
3. Destructive state changes (`promote`, `remove`, `undo`) must create or use operation backups before finalizing.
4. Lock conflicts must surface as explicit conflict artifacts and a resolvable transaction state, not silent partial success.
5. Core package drift is policy-gated and cannot promote without explicit approval.
6. `state/plugins.json` is registry metadata; dependency-group content in `pyproject.toml` is authoritative for actual dependencies.
7. `env export` / `env import` bundles are transport artifacts; after import, local truth remains `pyproject.toml + uv.lock`.
8. Bundle source snapshots exclude VCS admin metadata (`.git/`).
9. The Rust binary is the sole entry point. No shell wrappers or dispatchers in the product.
10. Cross-platform behavior is defined at the product semantics level, not left as implementation detail.

## Module Map

```
src/
├── main.rs                 → binary entry, panic handler
├── cli.rs                  → clap definitions, command dispatch
├── application/            → command implementations (one file per command group)
├── domain/                 → newtypes, enums, shared domain records
├── state_ledger/           → CRUD for transactions, operations, plugins, conflicts
├── safety_guards/          → backup/restore, drift guard, core impact gate
├── dependency_sync/        → UvClient, staged workdir, freeze, lock check
├── source_integration/     → GitClient, plugin path mapping
├── runtime_executor/       → process execution, PID tracking, log capture
├── platform/               → venv Python locator, process termination, PID check
├── toml_support/           → format-preserving config.toml and pyproject.toml editing
└── fs_support/             → atomic write, hashing, path normalization, dir operations
```

## Domain Model

### State Machines

**Transaction lifecycle:**

Valid transitions:
- `running` → `completed` | `failed` | `needs_resolution`
- `completed` → `promoted` | `promote_failed` | `needs_resolution` | `aborted`
- `failed` → `promoted` | `promote_failed` | `needs_resolution` | `aborted`
- `needs_resolution` → `resolved` | `failed` | `aborted`
- `resolved` → `promoted` | `needs_resolution` | `promote_failed` | `aborted`
- `promote_failed` → `aborted`

Note: `running` → `needs_resolution` occurs when lock fails during `tx run` staging (before ComfyUI executes). `running` → `completed` includes both exit code 0 and bounded observation timeout. `completed` / `resolved` / `failed` → `needs_resolution` occurs when lock fails during plugin promotion. Core-update promotion uses the previously locked snapshot.

**Operation lifecycle:**

```
running → success | failed
success → undone (via undo command)
```

### Type Definitions

| Type | Format | Example |
|------|--------|---------|
| TxId | `<UTC_timestamp>-<8_hex_chars>` | `20260419T120000Z-a1b2c3d4` |
| OpId | `<UTC_timestamp>-op-<8_hex_chars>` | `20260419T120000Z-op-e5f6a7b8` |
| NodeId | Git repo basename or explicit `--id` | `comfyui-manager` |
| GroupName | Normalized: lowercase, hyphens only | `node-comfyui-manager` |
| PythonMinor | `<major>.<minor>` | `3.11` |

### Error Taxonomy

| Category | When | Effect |
|----------|------|--------|
| Usage error | Invalid arguments/flags | Exit before any I/O |
| Precondition error | Missing config, missing files, wrong state | Exit before mutation |
| Policy error | Core impact gate, undo not undoable | Exit before mutation |
| Adapter error | uv/git/python subprocess failure | Discard failed staging. Restore after a production write |
| Restore error | Backup restoration fails | Log prominently, leave state for manual recovery |

## Data Sovereignty

| Domain | Source of Truth | Location |
|--------|----------------|----------|
| Dependencies | `pyproject.toml` + `uv.lock` | project root |
| Configuration | `config.toml` | project root |
| Plugin registry | `state/plugins.json` | state dir |
| Transactions | `state/transactions/*.json` | state dir |
| Operations | `state/ops/<op_id>/meta.json` + `backup/` | state dir |
| Conflicts | `state/conflicts/*.json` | state dir |
| Runtime liveness hint | `state/comfyui.pid` | state dir |
| Logs | `state/logs/*` | state dir |

`state/comfyui.pid` is an operational hint, not a durable source of truth; runtime liveness must always be confirmed against the host process layer.

## External Tool Contracts

| Tool | Used For | Invoked Via |
|------|----------|-------------|
| `uv` | lock, sync, add, remove, pip freeze, export, python find | `UvClient` |
| `git` | clone, checkout | `GitClient` |
| Python | smoke tests, ComfyUI runtime | `PythonClient`, `RuntimeClient` |

All tool invocations go through client structs that return `CmdResult`. No raw `Command::new()` in application code.

## Cross-Platform Rules

- Venv Python: `<venv>/bin/python` (Linux) vs `<venv>/Scripts/python.exe` (Windows); centralized in `venv_python()`, never hardcoded
- Process termination: graceful stop with timeout, then forced termination; Linux uses SIGTERM→SIGKILL, Windows uses platform-native graceful/forced primitives
- Paths in state files: always forward slashes
- Absolute path detection: accounts for drive letters on Windows
- Atomic writes: rename-based on both platforms, platform-specific fsync
- Bundle import: cross-platform import rejected in v1 (sys_platform must match)
- Smoke test: structured command (`program` + `args`), not shell string

## Key Flows

1. **Plugin onboarding**: `node add` → `tx run` → `tx inspect` → `tx promote`
2. **Lock conflict resolution**: promote fails → `resolve` with pins → re-promote
3. **Plugin removal + undo**: `node remove` → `undo` with backup restore
4. **Bootstrap**: `init` → `install torch` → `install` → `update run` → `update promote`
5. **Environment transfer**: `env export` → `env import` (same platform only in v1)
6. **Bundle platform rejection**: `env import` → platform check fails → exit before mutation with explicit mismatch error

## Drill-Down

- **Module specs** (building blocks, independently implementable):
  - [application/](modules/application.md) — command orchestration pattern, standard handler skeleton, restore sequence
  - [platform/](modules/platform.md) — venv Python locator, process control, path normalization
  - [fs_support/](modules/fs-support.md) — atomic writes, hashing, directory operations
  - [toml_support/](modules/toml-support.md) — format-preserving TOML editing
  - [dependency_sync/](modules/dependency-sync.md) — UvClient, staged workdir, prod sync
  - [state_ledger/](modules/state-ledger.md) — transaction/operation/plugin/conflict CRUD + schemas
  - [safety_guards/](modules/safety-guards.md) — backup/restore, drift guard, core impact gate, smoke test
  - [source_integration/](modules/source-integration.md) — GitClient, plugin path mapping
  - [runtime_executor/](modules/runtime-executor.md) — process lifecycle, PID, timeout, logs
- **Command behavioral contracts** (per-command specs): [commands/contracts.md](commands/contracts.md)
- **Rewrite plan and migration strategy**: [adr/003-rust-rewrite-plan.md](adr/003-rust-rewrite-plan.md)
- **Language selection rationale**: [adr/002-rewrite-language-selection.md](adr/002-rewrite-language-selection.md)
