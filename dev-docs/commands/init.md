# init

**Implementation target:** [src/application/init.rs](../../src/application/init.rs)

## Synopsis

```
gov init --comfyui-dir <abs-path> --python <python-spec>
```

## Purpose

Bootstrap a new `comfy_env` project or re-initialize an existing one. Creates config, project truth files, directory layout, and production venv.

## Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `--comfyui-dir` | Yes (first run) | Absolute path to ComfyUI installation |
| `--python` | Yes (first run) | Python version selector (e.g., `3.11`, `python3.11`, path to interpreter) |

On re-init, flags are optional — existing values are preserved if not overridden.

## Preconditions

- `uv` must be available on PATH
- `--comfyui-dir` must be an absolute path (platform-appropriate check)
- If no `config.toml` exists, both flags are required

## Reads

- `config.toml` (if exists — merge with overrides)
- `config.toml.template` (if config.toml doesn't exist — use as base)
- `pyproject.toml.template` (if pyproject.toml doesn't exist)

## Writes

- `config.toml` — created or updated with resolved values
- `pyproject.toml` — created from template if missing; updated with `requires-python` and `[tool.uv].environments`
- Directory layout: `state/`, `state/transactions/`, `state/logs/`, `state/conflicts/`, `state/work/`, `state/ops/`, `cache/`, `.venv-candidate/`
- `state/plugins.json` — created as `[]` if missing

## Success Path

```
1. Resolve Python selector to canonical minor line:
   - If selector is already "X.Y" format: use directly
   - Else: run `uv python find --no-python-downloads <selector>`
     → extract minor version from resolved interpreter
2. Write config.toml:
   - Merge flags with existing config (or template defaults)
   - Validate comfyui_dir is absolute
   - Write all sections: paths, runtime, tx, policy, ops, run
3. Ensure pyproject.toml exists:
   - If missing: copy from pyproject.toml.template
4. Update pyproject.toml runtime constraints:
   - Set `project.requires-python` = `==<major>.<minor>.*`
   - Detect current platform marker (sys_platform + platform_machine)
   - Set `[tool.uv].environments` = [<current_marker>]
5. Create directory layout (state dirs, cache dirs)
6. Initialize plugins registry (state/plugins.json = [] if missing)
7. Lock project: `uv lock --python <minor>`
8. Sync prod env: `uv sync --python <minor> --locked --all-groups`
   with UV_PROJECT_ENVIRONMENT=<prod_env_path>
```

## Failure Path

- Python resolution failure → exit before any file writes
- Config validation failure (non-absolute path) → exit before writes
- `uv lock` failure → exit (config and pyproject already written, but no venv)
- `uv sync` failure → exit (truth files valid, venv incomplete — re-run init to retry)

## Platform Notes

- Python path resolution uses `uv python find` which is cross-platform
- Environment marker detection uses host platform introspection
- `--comfyui-dir` absolute path check must handle drive letters on Windows
