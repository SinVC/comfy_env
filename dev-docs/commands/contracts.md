# Command Behavioral Contracts

Implementation entry points are linked at the top of each command doc. Start from the one command you are changing, then follow only the module references that command uses.

Language-agnostic behavioral specifications for every `gov` command. These describe *what* each command does, not *how* it's implemented.

**Document hierarchy**: architecture.md (system rules) > commands/*.md (command behavior) > modules/*.md (implementation). Conflicts resolve upward. See [architecture.md §Document Hierarchy](../architecture.md#document-hierarchy).

- For system-level definitions (state machines, types, error taxonomy, cross-platform rules), see [../architecture.md](../architecture.md).
- For module-level implementation specs (the building blocks commands use), see [../modules/](../modules/).

## How to Use

- **Implementers**: These specs are the authority for Rust command implementations. If the code disagrees with this doc, fix the code.
- **Reviewers**: Verify that implementations satisfy every step in the success and failure paths.
- **AI agents**: Read the relevant command spec before implementing or modifying a command.
- **AI agents**: Avoid loading unrelated command docs unless the task actually spans multiple command groups.

## Shared Protocol Reference

Individual command docs reference these protocols by name (e.g. "Execute [Staged Workdir Protocol]"). Each protocol is defined in the module spec that owns it:

| Protocol | Definition | Module |
|----------|-----------|--------|
| Staged Workdir | [dependency-sync.md §Staged Workdir](../modules/dependency-sync.md#2-staged-workdir-protocol) | `dependency_sync/` |
| Prod Sync | [dependency-sync.md §Prod Sync](../modules/dependency-sync.md#3-prod-sync-protocol) | `dependency_sync/` |
| Lock Conflict Detection | [dependency-sync.md §Lock Conflict](../modules/dependency-sync.md#4-lock-conflict-detection) | `dependency_sync/` |
| Backup/Restore/Finalize | [safety-guards.md §Op Lifecycle](../modules/safety-guards.md#1-backuprestorefinalize-operation-lifecycle) | `safety_guards/` |
| Undo Drift Guard | [safety-guards.md §Drift Guard](../modules/safety-guards.md#2-undo-drift-guard) | `safety_guards/` |
| Core Impact Gate | [safety-guards.md §Core Gate](../modules/safety-guards.md#3-core-impact-gate) | `safety_guards/` |
| Smoke Test | [safety-guards.md §Smoke Test](../modules/safety-guards.md#4-smoke-test) | `safety_guards/` |

## Command Index

| Group | Commands | Spec |
|-------|----------|------|
| Bootstrap | `init` | [init.md](init.md) |
| Install | `install`, `install torch` | [install.md](install.md) |
| Pins | `pin add`, `pin list`, `pin remove` | [pin.md](pin.md) |
| Nodes | `node add`, `node remove` | [node.md](node.md) |
| Transactions | `tx run`, `tx inspect`, `tx abort`, `tx promote` | [tx.md](tx.md) |
| Conflict resolution | `resolve` | [resolve.md](resolve.md) |
| Updates | `update run`, `update inspect`, `update abort`, `update promote`, `update resolve` | [update.md](update.md) |
| Environment | `env export`, `env import` | [env.md](env.md) |
| Operations | `op list`, `op inspect`, `op recover`, `undo` | [ops.md](ops.md) |
| Runtime | `run`, `stop` | [runtime.md](runtime.md) |
| Status | `status`, `help` | [status.md](status.md) |
