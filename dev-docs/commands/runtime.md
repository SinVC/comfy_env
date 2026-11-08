# runtime (run / stop)

**Implementation target:** [src/application/runtime.rs](../../src/application/runtime.rs)

## `run`

### Synopsis

```
gov run [--sync] [-- <args...>]
```

### Purpose

Launch ComfyUI from the production environment. Optionally sync prod env before launching.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `--sync` | No | Run `uv sync` before launching |
| `-- <args>` | No | Arguments passed through to ComfyUI `main.py` |

### Preconditions

- `config.toml` must exist
- Prod env must exist (`.venv-prod/`)
- ComfyUI `main.py` must exist at `<comfyui_dir>/main.py`
- No other instance should be running (check PID file)

### Reads

- `config.toml` — comfyui_dir, prod_env, run.extra_args, run.sync_before_run
- `state/comfyui.pid` — check for existing process

### Writes

- `state/comfyui.pid` — write PID of launched process
- `state/comfyui.pid.lock` — coordinate startup, stop, and PID cleanup
- `.venv-prod/` — synced if `--sync` or `sync_before_run` is configured

### Success Path

```
1. Check for existing PID file:
   - IF PID file exists AND process is alive: exit with "already running" error
   - IF PID file exists AND process is dead: clean up stale PID file
2. IF --sync OR config run.sync_before_run:
   - Sync prod env via [Prod Sync Protocol]
3. Resolve Python: venv_python(prod_env_path)
4. Build command: [venv_python, <comfyui_dir>/main.py] + extra_args + passthrough args
5. Spawn ComfyUI as child process
6. Write PID → state/comfyui.pid
7. Wait for child process to exit
8. Clean up PID file
9. Return child's exit code
```

### Compatibility Notes (Bash → Rust)

- **Bash era**: used `exec` to replace the shell process with ComfyUI. After exec, `gov` no longer existed as a process.
- **Rust era**: spawns ComfyUI as a child process, writes PID, waits for exit. This enables PID tracking and cleanup on both platforms.

### Platform Notes

- PID file contains the child process PID (integer as text)
- Process liveness check:
  - Linux: check `/proc/<pid>` or `kill -0 <pid>`
  - Windows: `OpenProcess` with existence check
- Extra args from config and CLI are concatenated
- `run.extra_args` accepts a TOML array of strings for exact arguments, or a legacy whitespace-separated string

---

## `stop`

### Synopsis

```
gov stop
```

### Purpose

Stop a running ComfyUI process launched by `gov run`.

### Preconditions

- `state/comfyui.pid` should exist (if not, report no process)

### Reads

- `state/comfyui.pid` — PID of running process

### Writes

- `state/comfyui.pid` — deleted after successful stop

### Success Path

```
1. Read PID from state/comfyui.pid
2. IF PID file missing: print "no running process", exit success
3. Check if process is alive:
   - IF not alive: clean up stale PID file, print "process not running", exit success
4. Request graceful termination:
   - Linux: send SIGTERM
   - Windows: platform-native graceful termination
5. Wait up to 30 seconds for process to exit
6. IF process still alive after timeout:
   - Forced termination:
     - Linux: send SIGKILL
     - Windows: TerminateProcess
7. Clean up PID file
8. Report success
```

### Product Semantics

The user-visible contract is: **"stop" means the process will be terminated, first gracefully then forcefully.** The 30-second timeout is the grace window. The exact mechanism is platform-specific but the outcome is guaranteed: after `gov stop` returns, the process is no longer running.

### Platform Notes

- Process tree: ideally terminate the entire process group/tree, not just the root PID
- Stale PID detection must be robust on both platforms
- PID file is always cleaned up, even if the process was already dead
