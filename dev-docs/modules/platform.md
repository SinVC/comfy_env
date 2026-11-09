# platform/

**Implementation target:** [src/platform/](../../src/platform/)

## Responsibility

Cross-platform abstractions that isolate the rest of the codebase from OS-specific differences. Every other module calls into `platform/` rather than using platform-conditional code directly.

## Capabilities

### 1. Venv Python Locator

```rust
fn venv_python(venv_root: &Path) -> PathBuf
```

| Platform | Returns |
|----------|---------|
| Linux | `<venv_root>/bin/python` |
| Windows | `<venv_root>/Scripts/python.exe` |

**Contract**: The returned path must exist and be executable after a successful `uv sync`. Commands never construct venv Python paths themselves.

### 2. Process Liveness Check

```rust
fn is_process_alive(pid: u32) -> bool
```

| Platform | Implementation |
|----------|---------------|
| Linux | Check `/proc/<pid>/` existence or `kill(pid, 0)` |
| Windows | `OpenProcess` and check whether the process handle is signaled |

**Contract**: Returns `true` only if the process is still running. Used by `run` (stale PID detection) and `stop`.

### 3. Process Termination

```rust
fn terminate_process(pid: u32, grace_seconds: u32) -> TerminateResult
```

Product semantics (both platforms):
1. Request graceful shutdown
2. Wait up to `grace_seconds`
3. If still alive: forced termination

| Platform | Graceful | Forced |
|----------|----------|--------|
| Linux | `SIGTERM` | `SIGKILL` |
| Windows | Send Ctrl+Break to the launched console process group | `TerminateProcess` |

**Contract**: After this function returns `Ok`, the process is guaranteed to be dead. Returns error only if PID doesn't exist or permissions are insufficient.

### 4. Absolute Path Check

```rust
fn is_absolute(path: &Path) -> bool
```

| Platform | Logic |
|----------|-------|
| Linux | Starts with `/` |
| Windows | Has drive letter (`C:\...`) or UNC prefix (`\\...`) |

**Contract**: Used to validate `--comfyui-dir` and other user-provided paths. Rust's `Path::is_absolute()` already handles this, but this documents the expected behavior.

### 5. Path Normalization for State Files

```rust
fn to_state_path(path: &Path) -> String
```

Converts a platform-native path to forward-slash form for storage in JSON state files. State files always use forward slashes regardless of host OS.

```rust
fn from_state_path(s: &str) -> PathBuf
```

Converts a forward-slash state path back to platform-native `PathBuf`.

## Path Format Rules (Consolidated)

| Context | Format | Example (Windows) |
|---------|--------|-------------------|
| State files (JSON) | Forward slashes | `custom_nodes/my-plugin` |
| Bundle manifest | Forward slashes | `custom_nodes/my-plugin/init.py` |
| OS filesystem calls | Platform-native | `custom_nodes\my-plugin` |
| `uv` / `git` CLI args | Platform-native | `C:\Users\...\.venv-prod` |
| `UV_PROJECT_ENVIRONMENT` env var | Platform-native | `C:\Users\...\.venv-prod` |
| `config.toml` values | Platform-native | `C:\Users\ComfyUI` |

**Rule**: Forward slashes only in serialized state/manifest. Everything passed to OS, `uv`, or `git` uses native paths.

## Dependencies

- `std` for path handling and process launching
- `libc` on Unix and `windows-sys` on Windows for native process control

## Used By

- `dependency_sync/` — venv Python for `uv` invocations
- `runtime_executor/` — process launch, PID check, termination
- `safety_guards/` — smoke test Python resolution
- `state_ledger/` — path normalization in JSON records
- `application/` (all commands that accept paths)
