# install

**Implementation target:** [src/application/install.rs](../../src/application/install.rs)

## `install torch`

### Synopsis

```
gov install torch --index-url <url> [--torch <spec>] [--torchvision <spec>] [--torchaudio <spec>]
```

### Purpose

Install PyTorch family packages into the `dependency-groups.torch` group with a specific index URL (e.g., CUDA-specific wheels).

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `--index-url` | Yes | PyTorch wheel index URL |
| `--torch` | No | Exact torch version spec (e.g., `torch==2.3.0`) |
| `--torchvision` | No | Exact torchvision version spec |
| `--torchaudio` | No | Exact torchaudio version spec |

### Preconditions

- `config.toml` must exist (run `init` first)
- `uv` must be available

### Reads

- `config.toml` — python version, prod env path
- `pyproject.toml` — current dependency groups

### Writes

- `pyproject.toml` — adds/updates `dependency-groups.torch` entries
- `uv.lock` — re-locked after changes
- `.venv-prod/` — synced

### Success Path

```
1. Derive index name from URL (e.g., "pytorch-cu121" from URL pattern)
2. Copy truth into a temporary workdir.
3. Add the three selected torch specs together, binding them to the requested named index in the staged project.
4. Lock the staged project once, then start the install_torch operation and promote its truth.
5. Sync prod env via [Prod Sync Protocol]
6. Smoke test: import torch, torchvision, torchaudio in prod env Python
7. op_finalize(success)
```

### Failure Path

```
IF staging or lock fails: discard the workdir before touching production.
IF uv sync fails: op_restore → re-sync prod → op_finalize(failed)
IF smoke test fails: op_restore → re-sync prod → op_finalize(failed)
```

### Platform Notes

- Smoke test uses `venv_python()` to locate Python in prod env
- Index URL handling is platform-agnostic

---

## `install`

### Synopsis

```
gov install [--requirements-file <path>]
```

### Purpose

Import ComfyUI core requirements into `dependency-groups.core`. Filters out torch family packages (managed separately by `install torch`).

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `--requirements-file` | No | Path to requirements.txt (default: `<comfyui_dir>/requirements.txt`) |

### Preconditions

- `config.toml` must exist
- Torch group must be populated (run `install torch` first)
- Requirements file must exist and be readable

### Reads

- `config.toml` — comfyui_dir, python, prod env
- Requirements file — dependency specs
- `pyproject.toml` — current torch group (to filter)

### Writes

- `pyproject.toml` — replaces `dependency-groups.core` content
- `uv.lock` — re-locked
- `.venv-prod/` — synced

### Success Path

```
1. Read requirements file
2. Filter out torch family packages (torch, torchvision, torchaudio)
3. Copy truth to a temporary workdir.
4. Replace the staged core group once with all filtered requirements.
5. Lock the workdir.
6. Start the install_core operation and promote the staged truth.
7. Sync prod env via [Prod Sync Protocol]
8. Smoke test via [Smoke Test Protocol]
9. op_finalize(success)
```

### Failure Path

```
IF torch group not ready: exit with precondition error (no mutation)
IF staging or lock fails: discard the workdir before touching production.
IF promotion, sync or smoke fails: op_restore → re-sync prod → op_finalize(failed)
```

### Platform Notes

- Requirements file path uses platform-native path resolution
- Torch family filtering is case-insensitive after normalization
