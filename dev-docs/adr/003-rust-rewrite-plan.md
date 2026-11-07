# ADR-003: Rust Rewrite Plan

## Metadata

- Status: Accepted
- Date: 2026-04-19
- Supersedes: Original draft ADR-003, all prior Bash-centric migration strategies

## Premise Constraints

The following premises are non-negotiable. Every design decision in this document must satisfy all of them:

1. **`gov` (Rust binary) is the sole entry point.** No Bash wrapper, no shell dispatcher, no shim.
2. **Bash is fully eliminated from the product runtime.** `bin/gov` is retained only as a development reference artifact, never distributed or executed by users.
3. **Windows is a first-class target**, equal to Linux. Users must not need Bash, coreutils, or any Unix-specific tooling to use `gov`.
4. **The rewrite replaces Bash entirely**, not "adds a Rust backend to Bash".
5. **During migration, unimplemented commands return an explicit error** (`error: command '<name>' is not yet implemented`), never fall back to Bash.

## Context

`comfy_env` is a sidecar governance CLI for ComfyUI dependency management. The current implementation is a 5285-line Bash script (`bin/gov`) with 54 inline Python heredoc snippets and 27 `cmd_*` command entries.

Structural problems:

1. **Not cross-platform.** Hard dependency on bash, coreutils, Unix signals, Unix path semantics.
2. **Dual-language boundary friction.** 54 heredoc boundaries passing data via stdout/exit code. No type checking, poor IDE support.
3. **TOML editing is a core capability masquerading as a scripting hack.**
4. **Recovery semantics enforced by convention, not by code.**

## Decision

Rewrite `comfy_env` as a **Rust single-binary CLI** targeting **Linux + Windows**.

### Key Decisions

1. **Single crate** to start. Module boundaries preserved, no workspace split until concrete need emerges.
2. **Rust `gov` is the only entry point from the first release.** Unimplemented commands print an error and exit non-zero.
3. **Vertical-slice migration.** Each slice delivers a complete command group. No horizontal "read commands first, write commands later" phasing.
4. Implement platform behavior with its first consumer. Pin needs a venv Python locator and a bounded smoke command. Candidate runs add live log capture, and run/stop add runtime process management.
5. **State file schema compatibility.** Existing schemas preserved. New fields are append-only and optional.
6. **External tool boundary unchanged.** `uv`, `git`, and Python remain external CLI calls.
7. **Command surface stable.** `init`, `install`, `pin`, `node`, `tx`, `update`, `env`, `op`, `undo`, `run`, `stop`, `help` — no renames.

## Rationale

### Why Rust

- `toml_edit`: purpose-built for format-preserving TOML editing — the project's highest-frequency file operation.
- `Result<T, E>` + `?`: compiler-enforced error handling. A missed error check in this tool can corrupt a production venv.
- Single static binary: zero runtime dependencies for end users on both platforms.
- Cross-compilation: one-time CI cost, not recurring burden.

### Why not Python

The tool manages `pyproject.toml`, `uv.lock`, and `.venv-*`. A Python implementation creates namespace confusion, bootstrap paradox, and `uv` operation pollution risk.

### Why not Go

`go-toml/v2` offers best-effort format preservation that would cause ongoing friction on the project's most frequent file operation. Go's `if err != nil` is discipline-based, not compiler-enforced.

## Non-Goals

- No concurrent transactions, file lock servers, or background task systems in v1.
- No ComfyUI runtime supervisor (remains start/stop with PID tracking).
- No `state/plugins.json` promotion to dependency truth source.
- No custom dependency resolution — continue delegating to `uv lock` / `uv sync`.

## Architecture

### Module Layout

Single crate, `src/` organized by subsystem:

```
src/
├── main.rs
├── cli.rs                  # clap command definitions and dispatch
├── application/            # command implementations
│   ├── init.rs
│   ├── install.rs
│   ├── pin.rs
│   ├── node.rs
│   ├── tx.rs
│   ├── update.rs
│   ├── env.rs
│   ├── op.rs
│   ├── runtime.rs          # run / stop
│   └── undo.rs
├── domain/                 # core types, state enums, newtypes
├── state_ledger/           # transaction / operation / plugin / conflict CRUD
├── safety_guards/          # backup, restore, undo drift guard, core impact gate
├── dependency_sync/        # uv CLI wrapper, staged workdir, freeze, lock check
├── source_integration/     # git clone/checkout, plugin path mapping
├── runtime_executor/       # process launch, PID, timeout, log capture
├── platform/               # cross-platform abstractions (see §Platform Abstraction)
├── toml_support/           # config.toml and pyproject.toml format-preserving editing
└── fs_support/             # atomic write, hashing, dir copy, temp workdir, path normalization
```

### Core Types

```rust
// Strong newtypes — not bare strings
TxId, OpId, NodeId, GroupName, PythonMinor

// Domain records
RuntimeConfig, PluginRecord, TransactionRecord,
OperationRecord, ConflictReport, BundleManifest,
PromotionPlan

// State enums — not string conventions
TxKind     { Plugin, CoreUpdate }
TxStatus   { Running, Completed, Failed, NeedsResolution, Resolved,
             Promoted, PromoteFailed, Aborted }
OpStatus   { Running, Success, Failed, Undone }
RunOutcome { Passed, Failed(i32), TimedOut }
```

### Error Model

- **Command layer**: `anyhow::Result` with `.context()` for user-facing messages.
- **Critical mutation paths** (promote, undo, env import): return a thin typed boundary indicating:
  - Whether side effects were produced
  - Whether restore is needed
  - Whether restore succeeded
- Add typed boundaries reactively when a concrete path demands it. No pre-designed four-layer error taxonomy.

### Logging

Thin abstraction over stderr + `state/logs/*` file output. Logs are audit artifacts, not debug output. Interface:

- `warn()`, `info()` → stderr
- `audit()` → `state/logs/*` file
- Command summary, key action recording, restore-happened flag

Initial implementation can be simple; interface must not be hardcoded to `eprintln!`.

### External Command Boundary

Structured wrappers returning typed results:

```rust
struct CmdResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    command_summary: String,
    log_path: Option<PathBuf>,
}
```

Clients: `UvClient`, `GitClient`, `PythonClient`, `RuntimeClient`.

All clients use the platform-abstracted process runner (see §Platform Abstraction). No direct `Command::new()` in application code.

## Platform Abstraction

The platform module owns behavior that differs between systems. Add capabilities in the slice that uses them.

### Venv Python Locator

The location of the Python executable inside a virtualenv differs by platform:

| Platform | Path |
|----------|------|
| Linux | `<venv>/bin/python` |
| Windows | `<venv>/Scripts/python.exe` |

A single function `venv_python(venv_root: &Path) -> PathBuf` centralizes this. All code that needs "the Python in this venv" calls this function. No hardcoded `bin/python` anywhere.

### Process Runner

Unified process execution with:

- Configurable timeout (using `std::time::Duration`, not Unix `timeout` command)
- stdout/stderr capture to both memory and log files
- Exit code extraction
- Cross-platform child process termination on timeout:
  - Linux: SIGTERM → wait → SIGKILL
  - Windows: `TerminateProcess`

### Signal / Termination Semantics

For `gov stop`:

| Aspect | Linux | Windows |
|--------|-------|---------|
| Graceful stop | SIGTERM | `GenerateConsoleCtrlEvent(CTRL_C_EVENT)` or `TerminateProcess` |
| Forced stop | SIGKILL after timeout | `TerminateProcess` |
| Child tree | `kill(-pid)` process group | Job objects or tree kill |
| PID validity | `/proc/<pid>` or `kill -0` | `OpenProcess` + `GetExitCodeProcess` |

Product semantics (user-facing contract):

- `gov stop` attempts graceful shutdown with a 30-second timeout, then forces termination.
- If the process is already gone, `gov stop` succeeds silently and cleans up the PID file.
- Stale PID files are detected and cleaned on both platforms.

### `gov run` Behavior

`gov run` does **not** use Unix `exec` semantics. On both platforms:

- `gov run` spawns ComfyUI as a child process.
- `gov run` writes the PID file, then waits for the child to exit.
- `gov run` returns the child's exit code.
- `gov run` cleans up the PID file on exit.

This is a deliberate simplification from the Bash version's `exec` behavior, chosen for cross-platform consistency. The user experience is identical: `gov run` blocks until ComfyUI exits.

### Path Normalization

- All paths stored in state files use **forward slashes** (`/`) regardless of platform, for portability.
- `--comfyui-dir` accepts platform-native paths and normalizes to absolute form via `std::fs::canonicalize`.
- On Windows, drive letters and UNC paths are accepted. Case-insensitive comparison is **not** performed — paths are compared after canonicalization.
- `fs_support/` provides `normalize_path()` used consistently across all file I/O.

### Atomic File Writes

- Linux: temp file → fsync → rename → fsync parent dir
- Windows: temp file → `FlushFileBuffers` → `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING`
- Both paths abstracted behind a single `atomic_write()` function in `fs_support/`.

### Unix/GNU Dependency Replacement Map

Every Unix/GNU tool dependency in the current Bash implementation is explicitly replaced:

| Unix dependency | Rust replacement |
|----------------|-----------------|
| `bash` | eliminated — Rust binary is the entry point |
| `mktemp` | `tempfile::NamedTempFile` / `tempfile::TempDir` |
| `sha256sum` | `sha2` crate |
| `cmp` (file comparison) | byte-level comparison in `fs_support` |
| `timeout` | `std::time::Duration` + child process timeout in process runner |
| `uuidgen` / `/dev/urandom` | `getrandom` crate + custom timestamp-hex ID format |
| `/proc/<pid>` | platform-abstracted PID check (see Signal/Termination) |
| `kill` / signals | platform-abstracted termination (see Signal/Termination) |
| `bash -lc` (smoke test) | structured command execution (see §Smoke Test) |
| `date -u` | `time` crate |
| `find` / `cp -r` / `rm -rf` | `walkdir` + `fs_extra` or `std::fs` recursive operations |
| `sed` / `tr` | string operations in Rust |

## Smoke Test Model

`smoke_test_cmd` in `config.toml` changes from a shell string to a **structured command**:

### New config.toml format

```toml
[tx]
timeout_seconds = 120

# Old (Bash era, no longer supported):
# smoke_test_cmd = "python -c 'import torch; print(torch.__version__)'"

# New (Rust era):
[tx.smoke_test]
program = "python"
args = ["-c", "import torch; print(torch.__version__)"]
```

If `[tx.smoke_test]` is absent, the default smoke test is:

```
program = "python"   # resolved via venv Python locator
args = ["-c", "import sys; print(sys.version)"]
```

The `program` field undergoes venv Python resolution: if `program` is `"python"`, it resolves to the platform-appropriate venv Python path. Other programs are looked up on `PATH`.

**Migration**: if the tool encounters an old-format `smoke_test_cmd` string key, it prints a deprecation warning and ignores it.

## Bundle Cross-Platform Rules

### Default: cross-platform import is rejected

When `env import` encounters a bundle, it checks:

1. `requires-python` compatibility (must match)
2. `sys_platform` in `[tool.uv].environments` (must match current platform)
3. `platform_machine` in `[tool.uv].environments` (must match current architecture)
4. Manifest checksum verification
5. `uv lock --check` against imported `pyproject.toml` + `uv.lock`

**Check order is fixed**: Python version → platform → architecture → checksums → lock check. The first failure stops the import and reports a specific error.

If `sys_platform` does not match (e.g., bundle from Linux, importing on Windows), the import is **rejected** with:

```
error: bundle platform mismatch
  bundle:  sys_platform == 'linux' and platform_machine == 'x86_64'
  current: sys_platform == 'win32' and platform_machine == 'AMD64'
hint: cross-platform import is not supported in v1
```

A future `--force-platform` flag may override this, but is not implemented in v1.

### Bundle path storage

Paths inside `manifest.json` use forward slashes regardless of export platform. On import, paths are converted to platform-native form.

### custom_nodes handling

- `env import` removes `custom_nodes/*` directories not present in the bundle (same as current behavior).
- On Windows, if a `custom_nodes/*` entry is a junction or symlink, `env import` skips deletion and warns. It does not follow or delete link targets.
- Backup of existing `custom_nodes` uses `fs_support::dir_copy()` which handles symlinks by copying the link, not the target.

## TOML Editing Rules

First-class capability via `toml_edit`:

- `config.toml`: point updates to `paths.comfyui_dir`, `runtime.python`, `[tx.smoke_test]`, etc.
- `pyproject.toml`: point updates to `project.requires-python`, `[tool.uv].environments`, and precise maintenance of all `dependency-groups.*` entries.
- Principle: **modify only the target node; do not reformat the file.**

## Dependencies

First-batch Rust dependencies:

```
clap, serde, serde_json, toml_edit, anyhow, sha2, tempfile, time, getrandom
```

Add later only when needed: `thiserror` (when typed errors emerge), `tracing` / `tracing-subscriber` (when log abstraction outgrows the thin wrapper), `walkdir` / `fs_extra` (if `std::fs` recursive operations prove insufficient).

Not needed: `uuid` (timestamp + random hex format is sufficient).

## Migration Plan

### Slice 0: Scaffold and Cross-Platform Infrastructure

Start with the CLI surface, config and truth-file reads, platform paths and subprocess probes used by status. Unimplemented commands return a named error. Add the ledger and backup operations with pin/undo, log streaming with candidate runs, and directory snapshots with the first command that restores them.

**Completion criteria**: help and read-only status run on the target platforms. Pending command groups return their migration error.

### Slice 1: `pin add` / `pin list` / `pin remove` / `undo` / `op list` / `op inspect`

Exercises: TOML editing, `uv lock`/`uv sync`, staged workdir, op audit, backup/restore, undo drift guard.

**Completion criteria**: these commands work on both Linux and Windows. `tests/test_gov_cli.sh` pin/undo sections pass on Linux against Rust binary.

### Slice 2: `install torch` / `install` / `status`

Exercises: managed dependency groups (core/torch), prod sync, smoke test (structured model), read-only state reporting.

**Completion criteria**: these commands work on both platforms. Smoke test uses structured `[tx.smoke_test]` format.

### Slice 3: `node add` / `node remove` / `tx run` / `tx inspect` / `tx abort` / `tx promote` / `resolve`

Exercises: complete plugin transaction lifecycle, candidate env, conflict artifacts, core impact gate, cross-platform candidate process execution.

**Completion criteria**: full plugin hero flow works on both platforms.

### Slice 4: `update run` / `update inspect` / `update abort` / `update promote` / `update resolve`

Exercises: core update transaction lifecycle.

**Completion criteria**: core update flow works on both platforms.

### Slice 5: `env export` / `env import`

Exercises: bundle manifest, checksum, platform compatibility rejection, cross-platform path normalization, custom_nodes cleanup (with symlink/junction safety on Windows).

**Completion criteria**: export/import works on both platforms. Cross-platform import correctly rejected.

### Slice 6: `run` / `stop` / `help` / `init`

Exercises: ComfyUI process lifecycle, PID tracking, platform-specific termination, graceful shutdown, stale PID cleanup. `init` deferred here because it creates config.toml and pyproject.toml from templates — straightforward but depends on full TOML and config infrastructure being battle-tested.

**Completion criteria**: all commands implemented. Full test suite passes on both platforms.

## Testing Strategy

### Layer 1: Existing Shell Black-Box Tests (Linux-Only Oracle)

`tests/test_gov_cli.sh` is preserved as a **Linux behavioral baseline only**. It is not the cross-platform oracle.

Support configurable binary path so CI runs the same tests against the Rust binary on Linux.

### Layer 2: Rust Unit and Fixture Tests

- TOML edit before/after comparison fixtures
- State enum transition validation
- Hash, ID generation, path normalization (including Windows paths)
- Bundle manifest verification
- Venv Python locator correctness per platform
- Structured smoke test command construction

### Layer 3: Cross-Platform Integration Tests (Hard Requirement)

Platform-neutral Rust integration tests with fake `uv`/`git`/`python` that run on both Linux and Windows CI.

### Layer 4: Windows CI End-to-End Tests (Hard Requirement)

The following commands **must** pass in Windows CI before Windows support is claimed:

- `init`
- `pin add` / `pin remove`
- `install`
- `status`
- `tx run`
- `tx promote`
- `undo`
- `env import`
- `run` / `stop`

### High-Risk Paths Requiring Dedicated Tests

- `pin add/remove` — TOML editing correctness
- `tx promote` / `update promote` — guarded mutation + restore-before-return
- `node remove` — plugin cleanup + dependency removal
- `undo` — drift guard + backup restoration
- `env import` — checksum verification + platform compatibility rejection + symlink safety

## Acceptance Criteria

1. **Rust `gov` is the sole entry point.** No Bash wrapper, dispatcher, or shim exists in the distributed product.
2. **Bash is fully eliminated.** `bin/gov` is not executed by users. No Python heredocs in the runtime path.
3. **All commands implemented in Rust.** The full command surface (`init`, `install`, `pin`, `node`, `tx`, `update`, `env`, `op`, `undo`, `run`, `stop`, `help`) is functional.
4. **Linux behavioral compatibility.** `tests/test_gov_cli.sh` passes against the Rust binary on Linux.
5. **Windows full command support.** All commands work on Windows. Windows CI passes the minimum test set defined in §Testing Strategy Layer 4.
6. **Recovery semantics verified.** `pin add/remove`, `tx promote`, `undo`, `env import` have restore-before-return behavior validated by dedicated tests.
7. **TOML editing verified.** `pyproject.toml` and `config.toml` editing has fixture tests covering key scenarios.
8. **Cross-platform bundle rules enforced.** `env import` correctly rejects platform-mismatched bundles.
9. **No Unix-specific runtime dependencies.** The binary does not shell out to bash, coreutils, or any Unix-specific commands.

## Risks

1. Attempting "perfect architecture" and "perfect behavioral compatibility" simultaneously will stretch the migration. Prefer shipping slices over perfecting abstractions.
2. `run`/`stop` process semantics diverge between platforms. The product semantics are defined in §Platform Abstraction; implementation details may need iteration.
3. If typed error boundaries proliferate before real pain emerges, they become organizational overhead. Add them reactively.
4. Windows CI infrastructure may require non-trivial setup (installing `uv`, `git`, Python). Budget for this in Slice 0.

## Documents Requiring Updates

The following existing documents contain Unix/Bash assumptions that conflict with this ADR. They have been archived to `dev-docs-old/` and will be replaced by new Rust-era documentation as needed:

- `architecture-haiku.md` — references `bin/gov`, Bash module cards
- `subsystems/runtime-executor/spec.md` — Unix signals, `/proc`
- `subsystems/dependency-sync/spec.md` — Bash heredoc patterns
- `application-core/spec.md` — `cmd_*()` dispatch model
- `conventions/code-map.md` — Bash function organization
