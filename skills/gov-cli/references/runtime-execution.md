# Runtime Execution & Configuration (`run`, `stop`, `config.toml`)

Use these commands to start or stop the production ComfyUI instance and inspect runtime or project configuration.

---

## 1. Launching ComfyUI (`gov run`)

Starts `<comfyui_dir>/main.py` inside `.venv-prod` and forwards ComfyUI's exit code.

```text
gov run [--sync] [-- <ARGS>...]
```

- **Preconditions**: Requires `config.toml`, `<comfyui_dir>/main.py`, and `.venv-prod`. Fails (exit `2`) if another `gov`-managed ComfyUI process is running or if an operation needs `gov op recover`.
- `--sync`: Runs a locked `uv sync` on `.venv-prod` against `pyproject.toml` + `uv.lock` before launch (also triggered automatically if `run.sync_before_run = true` in `config.toml`). Recorded in `state/ops/` as `runtime_sync` (**not undoable via `gov undo`**).
- `<ARGS>...`: Arguments after `--` are passed to ComfyUI **after** prepending `run.extra_args` from `config.toml`.
- **Process Tracking**: Records process identity in `state/comfyui.pid`, streams output to the terminal, and removes the PID file on exit.

```bash
gov run --sync -- --listen 0.0.0.0 --port 8188
```

---

## 2. Stopping ComfyUI (`gov stop`)

Stops the `gov`-managed ComfyUI process tree recorded in `state/comfyui.pid`.

```text
gov stop
```

- **Shutdown Sequence**: Sends graceful termination (`Ctrl+Break` on Windows, `SIGTERM` on Linux), waits up to 30 seconds, force-terminates remaining processes in the managed tree/group, and removes `state/comfyui.pid`.
- **No-Op Safety**: Succeeds cleanly if no PID file exists or the process already exited (cleans up stale PID files).
- **Legacy PID Guard**: Refuses to signal a live process recorded in a legacy numeric-only PID file lacking identity metadata; verify and stop such processes manually.

---

## 3. `config.toml` Settings Reference

Created from `config.toml.template` during `gov init`:

| Key | Default | Description |
| :--- | :--- | :--- |
| `paths.comfyui_dir` | — | Absolute path to the ComfyUI installation directory |
| `runtime.python` | — | Python selector (`3.12`) or resolved interpreter path |
| `runtime.prod_env` | `.venv-prod` | Production venv directory (relative to project root) |
| `runtime.candidate_root` | `.venv-candidate` | Parent directory for per-transaction candidate venvs |
| `tx.timeout_seconds` | `120` | Default timeout (seconds) for candidate probes and smoke tests |
| `tx.smoke_test` | Python version check | Optional `{ program = "...", args = [...] }` run after prod syncs (`program = "python"` uses `.venv-prod` Python; `tx.smoke_test_cmd` is deprecated/ignored) |
| `policy.core_packages` | `torch`, `torchvision`, `torchaudio`, `xformers`, `triton`, `onnxruntime`, `onnxruntime-gpu`, `numpy` | Packages requiring `--approve-core` during `promote` |
| `ops.retention_count` | `100` | Maximum completed operations retained during history pruning |
| `run.extra_args` | `[]` | Extra arguments prepended by `gov run` (string array or legacy space-separated string) |
| `run.sync_before_run` | `false` | Always sync `.venv-prod` before launching ComfyUI |
