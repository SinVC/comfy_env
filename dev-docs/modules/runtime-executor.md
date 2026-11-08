# runtime_executor/

**Implementation target:** [src/runtime_executor/](../../src/runtime_executor/)

## Responsibility

Manages ComfyUI process lifecycle: launching (in candidate or prod environments), capturing output, enforcing timeouts, PID tracking, and process termination.

## Capabilities

### 1. Candidate Run (Transaction Execution)

```rust
fn run_candidate(
    env_path: &Path,
    comfyui_dir: &Path,
    timeout_seconds: u32,
    log_stdout: &Path,
    log_stderr: &Path,
) -> Result<RunOutcome>
```

1. Resolve Python: `venv_python(env_path)`
2. Build command: `[python, <comfyui_dir>/main.py]`
3. Spawn child process
4. Pipe stdout → `log_stdout` file and mirror it to the parent process stdout in real time
5. Pipe stderr → `log_stderr` file and mirror it to the parent process stderr in real time
6. Wait with timeout:
   - If exits before timeout: return `RunOutcome::Passed` or `RunOutcome::Failed(exit_code)`
   - If timeout expires: kill child, return `RunOutcome::TimedOut`

**Contract**: Used by `tx run` and `update run`. The candidate environment is isolated — this never touches prod env or truth files. `RunOutcome::TimedOut` means the observation window ended and the process was deliberately terminated; command layers decide how that maps into transaction status.

### 2. Prod Run (ComfyUI Launch)

```rust
fn run_prod(
    env_path: &Path,
    comfyui_dir: &Path,
    args: &[OsString],
    pid_path: &Path,
    before_launch: impl FnOnce() -> Result<()>,
) -> Result<i32>
```

1. Acquire the PID lock and check for an existing process
2. Run the pre-launch hook for optional sync, then resolve Python with `venv_python(env_path)`
3. Build command: `[python, <comfyui_dir>/main.py] + args`
4. Spawn child process
5. Write child PID to `pid_path` and release the PID lock
6. Wait for child exit
7. Reacquire the PID lock and remove the PID file if it still belongs to this run
8. Return child exit code

**Contract**: Blocks until ComfyUI exits. PID file exists only while process is running. Differs from Bash era (`exec` replaced the process) — Rust spawns a child to enable PID tracking.

### 3. Stop

```rust
fn stop(pid_path: &Path, grace_seconds: u32) -> Result<StopOutcome>
```

1. Read PID from file
2. If file missing: return `StopOutcome::NotRunning`
3. Check liveness via `platform::is_process_alive`
4. If dead: clean up stale PID file, return `StopOutcome::AlreadyStopped`
5. Call `platform::terminate_process(pid, grace_seconds)`
6. Clean up PID file
7. Return `StopOutcome::Terminated`

### 4. RunOutcome

```rust
enum RunOutcome {
    Passed,           // exit code 0
    Failed(i32),      // non-zero exit code
    TimedOut,         // killed after observation timeout
}
```

**Command-layer mapping**:

- `tx run` / `update run` should persist the real `run_exit_code` for audit.
- `TimedOut` should normally map to transaction `status=completed`, not `failed`, because the bounded observation finished successfully even though the process did not exit on its own.

### 5. StopOutcome

```rust
enum StopOutcome {
    Terminated,       // process was killed
    AlreadyStopped,   // stale PID, cleaned up
    NotRunning,       // no PID file
}
```

## Log File Conventions

All logs go to `state/logs/`. Naming patterns:

| Scenario | stdout | stderr |
|----------|--------|--------|
| Candidate run (`tx run`, `update run`) | `state/logs/<txid>.stdout.log` | `state/logs/<txid>.stderr.log` |
| Lock conflict | — | `state/conflicts/<txid>.lock.log` |

Candidate run output is dual-channel by contract: operators see it live in the terminal, and the same bytes are written to `state/logs/` for audit.

Prod run (`gov run`) does NOT write to `state/logs/` — ComfyUI output goes to the terminal (inherited stdout/stderr from the parent process).

## Dependencies

- `platform/` — venv_python, is_process_alive, terminate_process
- `std::process::Command` — process spawning
- `std::time::Duration` — timeout enforcement

## Used By

- `application/tx.rs` — candidate run during `tx run`
- `application/update.rs` — candidate run during `update run`
- `application/runtime.rs` — prod run and stop
