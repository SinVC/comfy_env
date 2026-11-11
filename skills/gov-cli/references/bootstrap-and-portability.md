# Environment Bootstrap & Portability

Use this workflow to initialize a new governance project, reconfigure Python or PyTorch, import initial ComfyUI core dependencies, or migrate a complete environment across identical OS/architecture hosts.

> **Preconditions**: ComfyUI must be stopped (`gov stop`) and no operations may be pending recovery (`gov op recover`).

---

## Scenario A: Fresh Setup or Reconfiguration (`init` -> `install torch` -> `install`)

Standard initialization sequence must run in order:

```bash
gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12
gov install torch --index-url https://download.pytorch.org/whl/cu124
gov install
```

### 1. `gov init`
Initializes or reconfigures `config.toml`, `pyproject.toml`, `uv.lock`, `state/`, and `.venv-prod`.

```text
gov init [--comfyui-dir <COMFYUI_DIR>] [--python <PYTHON>]
```

- **First run**: Both `--comfyui-dir` (absolute path) and `--python` (`<major>.<minor>`, patch version, executable name, or path) are required. Refuses if `.venv-prod` already exists without recoverable `config.toml`, `pyproject.toml`, and `uv.lock`.
- **Re-initialization**: If `config.toml`, `pyproject.toml`, and `uv.lock` exist, omitted flags reuse existing values.
- **Binding**: Resolves patch versions/names/paths via `uv python find --no-python-downloads`, writes `requires-python = "==<major>.<minor>.*"`, and binds `[tool.uv].environments` to the host OS/arch marker.
- **Rollback & Undo**: Stages and locks before writing root files. Rolls back files and `.venv-prod` on failure. **Not undoable via `gov undo`**.

### 2. `gov install torch`
Configures the PyTorch wheel index and installs `torch`, `torchvision`, and `torchaudio` into `dependency-groups.torch`.

```text
gov install torch --index-url <INDEX_URL> [--torch <TORCH>] [--torchvision <TORCHVISION>] [--torchaudio <TORCHAUDIO>]
```

- `--index-url`: Required; no whitespace allowed. Registered under `[tool.uv.index]` and bound to all three Torch packages via `[tool.uv.sources]`.
- `--torch`, `--torchvision`, `--torchaudio`: Optional exact pins (e.g., `torch==2.5.1`). Omitted packages default to unpinned names.
- **Verification**: Locks in staging, syncs `.venv-prod`, and runs `import torch, torchvision, torchaudio` smoke test. Rolls back on failure. **Undoable via `gov undo`**.

### 3. `gov install`
Imports ComfyUI core requirements into `dependency-groups.core` and syncs `.venv-prod`.

```text
gov install [--requirements-file <REQUIREMENTS_FILE>]
```

- **Precondition**: `config.toml` initialized and all three Torch-family packages present in `dependency-groups.torch` (run `gov install torch` first).
- `--requirements-file`: Defaults to `<comfyui_dir>/requirements.txt`.
- **Behavior**: Strips `torch`, `torchvision`, and `torchaudio` automatically, replaces `dependency-groups.core`, locks in staging, syncs `.venv-prod`, and runs the configured smoke test. Rolls back on failure. **Undoable via `gov undo`**.
- *(Note: For subsequent core updates when custom nodes are already installed, prefer the sandboxed `gov update run` workflow in `node-and-tx-workflow.md`.)*

---

## Scenario B: Portable Environment Bundles (`env export` / `env import`)

### 1. `gov env export`
Exports locked dependencies, plugin registry, and custom-node working trees into a new bundle directory.

```text
gov env export <OUTPUT_DIR>
```

- **Preconditions**: `<OUTPUT_DIR>` must not exist and must lie outside all node source directories. Requires valid `config.toml`, `pyproject.toml`, `uv.lock` (must be up-to-date for `uv export --format pylock.toml --locked --all-groups`), `state/plugins.json`, and every registered `custom_nodes/<node_id>` directory.
- **Bundle contents**:
  - `pyproject.toml`, `uv.lock`, `pylock.toml`, `state/plugins.json`, `manifest.json` (v1 metadata + SHA-256 checksums)
  - `custom_nodes/<node_id>/` (working-tree snapshots including uncommitted/untracked files, excluding `.git`; fails if any node contains symlinks/junctions)
  - `audit/prod-freeze.txt`, `audit/export-summary.json`

### 2. `gov env import`
Restores an exported bundle onto the current machine as an exact replacement of the governance environment and managed `custom_nodes`.

```text
gov env import <BUNDLE_DIR> --comfyui-dir <COMFYUI_DIR> --python <PYTHON>
```

- **Strict Pre-Flight Checks**:
  - Same OS and CPU architecture (`sys_platform`, `platform_system`, `platform_machine`) as the bundle. Cross-platform import is rejected.
  - Checksums, `manifest.json` v1, `uv lock --check`, and `uv sync --dry-run` against the target `--python` (resolved via `uv python find --no-python-downloads`).
  - Refuses if `.venv-prod` exists without an existing `pyproject.toml` and `uv.lock`.
- **Custom Nodes Directory Handling**:
  - Replaces managed `custom_nodes/<node_id>` directories and removes unlisted **ordinary directories** in `custom_nodes/` (backed up during import for failure rollback).
  - Preserves top-level regular files, symlinks, and junctions: aborts before writing if a bundle node collides with a regular file; skips the bundled node with a warning if it collides with a symlink/junction.
- **Rollback & Undo**: Automatically rolls back truth, `.venv-prod`, and node directories if import or smoke test fails. **Not undoable via `gov undo`** (use `gov op recover` only if interrupted mid-write).
