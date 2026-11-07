# AGENTS.md

## Project Overview

`comfy_env` is a sidecar governance CLI for ComfyUI custom-node dependency changes. Optimize for correctness, auditability, and rollback safety over speed; a broken venv is worse than a slow command. Stage and lock dependency changes before promotion. Plugin and core update transactions also observe a candidate environment.

The CLI is implemented in **Rust** as a single binary `gov`. Target platforms: **Linux + Windows**.

## Tech Stack

- **Language**: Rust (single crate, single binary)
- **CLI**: `clap` with derive macros
- **TOML editing**: `toml_edit` (format-preserving)
- **JSON state**: `serde` + `serde_json`
- **Error handling**: `anyhow` at command layer; typed boundaries on critical mutation paths
- **External tools** (subprocess calls, not embedded): `uv`, `git`, Python
- **Local truth files**: `config.toml`, `pyproject.toml`, `uv.lock`
- **Durable state**: `state/*.json`, `state/transactions/*.json`, `state/ops/*`

## Commands

- Build: `cargo build --release`
- Test: `cargo test`
- Test (legacy Linux behavioral baseline): `bash tests/test_gov_cli.sh`
- Help: `gov help`
- Status: `gov status`
- Hero flow: `gov node add <url>` → `gov tx run` → `gov tx inspect` → `gov tx promote`

## Architecture

- `gov` (Rust binary) is the **sole entry point**. No Bash wrapper, dispatcher, or shim.
- `pyproject.toml` + `uv.lock` are the dependency source of truth; `state/plugins.json` is registry metadata, not dependency truth.
- Candidate transactions are the only supported way to observe dependency impact before promotion into `.venv-prod`.
- Development entry point: start from `dev-docs/architecture.md`, then follow the relevant command and module docs from there.
- Treat `dev-docs/architecture.md` as the authoritative navigation entry for implementation work; `AGENTS.md` is only the quick project brief and constraint summary.

## Coding Conventions

- One module per subsystem under `src/`: `application/`, `domain/`, `state_ledger/`, `safety_guards/`, `dependency_sync/`, `source_integration/`, `runtime_executor/`, `platform/`, `toml_support/`, `fs_support/`.
- Command implementations live in `src/application/<command>.rs`.
- Use newtypes for IDs: `TxId`, `OpId`, `NodeId`, `GroupName`.
- Use enums for state: `TxStatus`, `OpStatus`, `RunOutcome` — never bare strings.
- Error handling: `anyhow::Result` at command layer; typed return values on critical mutation paths that indicate whether side effects occurred and whether restore is needed.
- External tools (`uv`, `git`, `python`) wrapped in client structs returning `CmdResult`. Never call `Command::new()` directly from application code.
- TOML editing via `toml_edit` — modify only the target node, do not reformat the file.
- File writes: temp file → fsync → atomic rename. Never "delete then record".
- No hardcoded `bin/python` — use venv Python locator from `platform/`.
- Paths in state files use forward slashes regardless of platform.

## Boundaries

- Target platforms: Linux + Windows. macOS is not actively tested but should work where possible.
- Do not embed `uv`, `git`, or Python as libraries. Keep them as external CLI dependencies.
- Never edit `*.template`, `state/*`, `.venv-prod/`, or `.venv-candidate/` by hand; use the existing ledger and sync flows.
- Do not bypass transaction, backup, or promote safety paths for prod-affecting changes.
- State file schema changes: append-only optional fields, no renames, read path tolerant of missing optional fields.
- No Unix-specific runtime dependencies. The binary must not shell out to bash, coreutils, or any Unix-only commands.
- Bundle cross-platform import is rejected in v1 (platform mismatch → error).

## Verification

- Run `cargo test` before considering work complete.
- Run `cargo test` on **both Linux and Windows** for cross-platform work.
- Run `bash tests/test_gov_cli.sh` on Linux to verify behavioral compatibility (legacy baseline only, not cross-platform oracle).
- For new commands, exercise one happy path and one failure path on both platforms.
- For dependency-mutation changes, verify real `uv lock` and `uv sync` behavior in a project layout.
- For remove/promote/undo changes, verify backup and restore behavior rather than relying on code inspection alone.

## Sandbox Notes

Use this section to record commands that should be run with escalation immediately in this workspace, without first attempting a non-escalated run.

- `git add` — direct escalation required; sandboxed execution consistently fails with Git index lock or permission errors.
- `git commit` — direct escalation required; sandboxed execution consistently fails with Git index lock or permission errors.

## Migration Status

Vertical slice migration — each slice delivers a complete command group:

- [x] Slice 1 (implemented): `pin add` / `pin list` / `pin remove` / `undo` / `op list` / `op inspect`
- [ ] Slice 2: `install torch` / `install` / `status`
- [ ] Slice 3: `node add` / `node remove` / `tx run` / `tx inspect` / `tx abort` / `tx promote` / `resolve`
- [ ] Slice 4: `update run` / `update inspect` / `update abort` / `update promote` / `update resolve`
- [ ] Slice 5: `env export` / `env import`
- [ ] Slice 6: `run` / `stop` / `help` / `init`

## Read Only When Needed

Do not read every document listed below by default. Read only the document(s) that match the current task.

Recommended read order for implementation work:

1. Start with `dev-docs/architecture.md`.
2. Read the relevant file in `dev-docs/commands/`.
3. Read only the specific file(s) in `dev-docs/modules/` that the command actually depends on.
4. Follow the implementation links in those docs into `src/`.

**System-level design:**
- For system overview, invariants, module map, and cross-platform rules: read `dev-docs/architecture.md`.
- For the rewrite plan and migration strategy: read `dev-docs/adr/003-rust-rewrite-plan.md`.

**Command behavioral contracts (what each command does step-by-step):**
- For the command index and shared protocol reference: read `dev-docs/commands/contracts.md`.
- For a specific command group: read the corresponding file in `dev-docs/commands/` (e.g., `pin.md`, `tx.md`, `env.md`).

**Module specs (how each building block works internally):**
- For command orchestration patterns and standard handler skeleton: read `dev-docs/modules/application.md`.
- For a specific module: read the corresponding file in `dev-docs/modules/` (e.g., `platform.md`, `safety-guards.md`, `dependency-sync.md`).

Legacy reference: read only when current docs and current Rust code are still insufficient to answer the question or recover intended behavior.
- Archived Bash-era design docs: `dev-docs-old\architecture-haiku.md`. This is not active design authority.
- Use old docs only to understand legacy behavior or old implementation intent, not to override `dev-docs/architecture.md`, `dev-docs/commands/`, or `dev-docs/modules/`.
