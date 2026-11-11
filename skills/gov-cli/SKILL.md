---
name: gov-cli
description: Use when running or troubleshooting the `gov` (`gov.exe`) CLI to manage ComfyUI environments, custom nodes, staged transactions (`tx` / `update`), dependency pins/overrides, environment bundles (`env export` / `import`), ComfyUI runtime (`run` / `stop`), or operation rollback/recovery (`undo` / `op recover`) (ComfyUI 环境治理、自定义节点/插件安装、依赖冲突解决、事务晋升、版本锁定、环境导出导入、启停与回滚恢复).
---

# `gov` CLI Operational Guide

`gov` (`gov.exe` on Windows) manages ComfyUI dependencies, custom nodes, sandboxed verification transactions (`.venv-candidate`), and production runtime (`.venv-prod`).

**Working Directory Rule**: Always invoke `gov` from the governance project root (the directory containing `config.toml`, `pyproject.toml`, and `uv.lock`, or their `.template` files before initialization).

## 1. Pre-Flight Check (`gov status`)

Before executing multi-step workflows or troubleshooting errors, inspect current state:

```bash
gov status
```

Key fields returned:
- **Readiness**: `project_root`, `config_ready`, `comfyui_dir`, `python`, `prod_env_exists`, `torch_ready`, `core_ready`, `lock_exists`
- **External tools**: `tool_uv`, `tool_git`, `tool_python` (`uv` is required for all lock/sync/export tasks; `git` is required for `gov node add`)
- **State & blockers**: `runtime_running`, `operations_needing_recovery`, `plugins_registered`, `plugins_enabled`, `transactions_total`, `transactions_pending`, `update_transactions_pending`, `transaction_counts`, `recent_transactions` (up to 8), `notes`

Use `gov help [COMMAND]...` (or `-h` / `--help`, `-V` / `--version`) for syntax checks.

## 2. Global Invariants & Blocking Guards

Every command obeys these rules. Check them before mutating state or when diagnosing exit code `1` (runtime/tool/sync/smoke/rollback failure) or `2` (usage/precondition/policy/guard rejection):

1. **Stop-Before-Write Rule**: Any command that mutates production state fails (exit `2`) if a `gov`-managed ComfyUI process is running (`runtime_running: true`). Run `gov stop` first.
2. **Recovery Gate**: If an interrupted production operation exists (`operations_needing_recovery` is non-empty), **all** production writes and `gov run` are blocked until resolved via `gov op recover <OP_ID>`.
3. **Production Lock**: Commands that mutate production, run recovery, abort transactions, export bundles, or start/stop ComfyUI acquire `state/comfyui.pid.lock` and fail immediately if another `gov` process holds it.
4. **Torch Exclusivity**: `torch`, `torchvision`, and `torchaudio` are managed **exclusively** by `gov install torch`. They are rejected by `gov pin`, `gov resolve`, and `gov update resolve`, and automatically stripped from `requirements.txt` during `gov install` and `gov update run`.
5. **Exact Pin Format & Normalization**:
   - Package and node group names normalize by lowercasing and collapsing runs of `-`, `_`, and `.` into `-` (e.g., `Foo_Bar.Baz` -> `foo-bar-baz`).
   - Pin flags require strict `<package>==<version>` with no whitespace, version ranges, extras, or markers.
6. **Core Impact Gate (`policy.core_packages`)**: Any transaction (`tx` or `update`) whose net diff adds, upgrades, downgrades, or removes a core package (default: `torch`, `torchvision`, `torchaudio`, `xformers`, `triton`, `onnxruntime`, `onnxruntime-gpu`, `numpy`) requires `--approve-core` during `promote`.
7. **Wheel & `requirements.txt` Restrictions**:
   - `UV_FIND_LINKS` accepts comma-separated directories or URLs (relative paths resolve from the invocation working directory). `tool.uv.workspace` and non-relocatable local path/file-URL sources are rejected.
   - In `requirements.txt`, blank lines and `#` comments are ignored; options starting with `-` (e.g., `-r`, `--index-url`), relative paths starting with `.` or containing `\`, and invalid specs are rejected.

## 3. Scenario Routing (Load Only What You Need)

Select the reference document matching the user's goal:

| Scenario / User Intent | Reference Document | Covered Commands |
| :--- | :--- | :--- |
| **Initialize project, configure Torch/Core, or export/import portable bundles** | [`references/bootstrap-and-portability.md`](references/bootstrap-and-portability.md) | `init`, `install torch`, `install`, `env export`, `env import` |
| **Add/remove custom nodes, run candidate transactions, resolve conflicts, or update ComfyUI core** | [`references/node-and-tx-workflow.md`](references/node-and-tx-workflow.md) | `node add`, `node remove`, `tx run/inspect/promote/abort`, `resolve`, `update run/inspect/resolve/promote/abort` |
| **Pin package versions globally or manage forced `uv` overrides** | [`references/dependency-pins-and-overrides.md`](references/dependency-pins-and-overrides.md) | `pin add`, `pin list`, `pin remove` |
| **Start or stop ComfyUI, pass launch args, or check `config.toml` runtime settings** | [`references/runtime-execution.md`](references/runtime-execution.md) | `run`, `stop`, `status`, `config.toml` reference |
| **Undo a previous change, recover from a crashed operation, or audit operation history** | [`references/rollback-and-recovery.md`](references/rollback-and-recovery.md) | `op list`, `op inspect`, `op recover`, `undo` |
