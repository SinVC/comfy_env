# UC-008 Install Managed Runtime Dependencies

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Goal / Actor / Trigger

- Goal: install ComfyUI runtime prerequisites into local truth in a controlled order.
- Actor: local operator.
- Trigger: `gov install torch`, then `gov install`.

## Preconditions / Postconditions

- Preconditions:
  - `gov init --comfyui-dir --python` has completed.
  - `runtime.python` is present in `config.toml`.
- Postconditions:
  - Success path: `dependency-groups.torch` and `dependency-groups.core` are reflected in `pyproject.toml`, `uv.lock`, and `.venv-prod`.
  - Failure path: local truth is restored from operation backup.

## Main Path

1. Install `torch/torchvision/torchaudio` into `dependency-groups.torch` from a user-supplied index URL.
2. Sync `.venv-prod` and run a torch import smoke test.
3. Read `${comfyui_dir}/requirements.txt` (or the supplied override path).
4. Filter out torch-family requirements and write the remainder into `dependency-groups.core`.
5. Sync `.venv-prod` again and run the configured smoke test.

## Alternative / Failure Paths

1. If torch is not installed first, `gov install` blocks before mutating local truth.
2. If `uv` lock or sync fails, the command restores pre-op files and marks the operation failed.
3. If smoke fails, the command restores pre-op files and marks the operation failed.

## Acceptance Checks

1. `gov status` shows `torch_ready: yes` after `gov install torch`.
2. `gov status` shows `core_ready: yes` after `gov install`.
3. `op list` shows `install_torch` / `install_core` operations as undoable on success.
