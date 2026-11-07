# env (Environment Export/Import)

**Implementation target:** [src/application/env.rs](../../src/application/env.rs)

## `env export`

### Synopsis

```
gov env export <output_dir>
```

### Purpose

Export the current environment as a portable bundle for transfer to another machine. The bundle contains all truth files, plugin source snapshots, and a verification manifest.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<output_dir>` | Yes | Directory to create the bundle in (must not already exist) |

### Preconditions

- `pyproject.toml` and `uv.lock` must exist
- `config.toml` and `state/plugins.json` must exist
- All registered plugins must have their source directories present
- Output directory must not already exist

### Reads

- `pyproject.toml`, `uv.lock` — truth files
- `state/plugins.json` — plugin registry
- `custom_nodes/*/` — plugin source directories
- `config.toml` — for runtime metadata in audit files

### Writes

- `<output_dir>/pyproject.toml` — copy of truth
- `<output_dir>/uv.lock` — copy of truth
- `<output_dir>/pylock.toml` — generated via `uv export --format pylock.toml --locked --all-groups`
- `<output_dir>/state/plugins.json` — copy of registry
- `<output_dir>/custom_nodes/<node_id>/` — source snapshots (without `.git/`)
- `<output_dir>/manifest.json` — checksums + platform info
- `<output_dir>/audit/prod-freeze.txt` — `uv pip freeze` output from prod env
- `<output_dir>/audit/export-summary.json` — audit metadata

### Success Path

```
1. Verify preconditions (truth files exist, all plugins have source dirs)
2. Create output_dir
3. Copy truth files: pyproject.toml, uv.lock
4. Export pylock.toml: `uv export --format pylock.toml --locked --all-groups`
5. Copy state/plugins.json
6. For each registered plugin:
   - Copy custom_nodes/<node_id>/ to bundle, EXCLUDING .git/ directories
7. Capture prod freeze: `uv pip freeze` in prod env → prod-freeze.txt
8. Write audit/export-summary.json:
   - timestamp, python version, platform info, plugin count, package count
9. Write manifest.json:
   - SHA256 checksum for each payload file, excluding manifest.json itself
   - requires-python value
   - sys_platform, platform_machine
   - tool.uv.environments marker
```

### Failure Path

```
IF truth files missing: exit with precondition error
IF plugin source dir missing: exit with error listing missing plugins
IF uv export fails: clean up partial bundle, exit
```

### Platform Notes

- `.git/` exclusion: skip directory entirely during copy (not just hide files)
- Manifest records platform markers for import-time compatibility checking

---

## `env import`

### Synopsis

```
gov env import <bundle_dir> --comfyui-dir <abs-path> --python <python-spec>
```

### Purpose

Import an environment from an exported bundle. This is an **exact restore** — it overwrites local truth, recreates the prod environment, and synchronizes custom_nodes.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<bundle_dir>` | Yes | Path to exported bundle |
| `--comfyui-dir` | Yes | Target ComfyUI directory (absolute) |
| `--python` | Yes | Python version for target |

### Preconditions

- Bundle directory must exist and contain `manifest.json`
- Manifest checksums must verify (all files intact)
- Platform compatibility must pass (see rules below)
- `--comfyui-dir` must be absolute

### Reads

- `<bundle_dir>/manifest.json` — checksums + platform markers
- `<bundle_dir>/pyproject.toml`, `<bundle_dir>/uv.lock` — bundled truth
- `<bundle_dir>/state/plugins.json` — bundled registry
- `<bundle_dir>/custom_nodes/*/` — plugin snapshots

### Writes

- `config.toml` — created/updated with new paths + python
- `pyproject.toml` — overwritten from bundle
- `uv.lock` — overwritten from bundle
- `state/plugins.json` — overwritten from bundle
- `.venv-prod/` — recreated via sync
- `custom_nodes/*/` — synchronized with bundle (extras removed, bundle contents restored)
- `state/ops/<op_id>/` — operation record

### Design Note: No Core Impact Gate

`env import` does not apply the core impact gate. Unlike `tx promote` / `update promote` which compute a diff and gate on core packages, import is an **exact restore** — the user explicitly chose to adopt the bundle's entire dependency set. The platform compatibility check is the safety gate for import.

```
1. Check requires-python: target python must satisfy bundled requires-python
2. Check sys_platform: MUST match exactly
3. Check platform_machine: MUST match exactly
4. IF any mismatch: exit with "platform incompatible" error BEFORE any mutation
```

**Cross-platform import is rejected by default.** A Linux bundle cannot be imported on Windows, and vice versa.

### Success Path

```
1. Load the manifest and resolve the requested Python minor.
2. Check requires-python, platform and architecture, then verify payload checksums.
3. Run `uv lock --check --python <py>` in the bundle directory.
4. Begin the env_import operation. Back up config.toml (including prior absence) and the custom_nodes entries that will be replaced or removed, along with the usual truth files and registry.
5. Write target config, root truth and registry, then sync prod with the locked project.
6. Replace the affected custom_nodes trees and remove extras. Preserve linked entries as specified in ADR-003.
7. Run the smoke command and finalize the import with undoable=false.
```

### Failure Path

```
IF manifest checksum mismatch: exit before mutation
IF platform incompatible: exit before mutation
IF lock check fails: exit before mutation
IF any target write, sync or smoke step fails: restore config, truth, registry and affected node trees. If a previous production lock existed, re-sync its environment using the previous config. On a fresh import, remove the environment created by this operation. Record any recovery error along with the import failure.
```

### Platform Notes

- custom_nodes sync: on Windows, handle read-only file attributes before deletion (Git sets some files read-only)
- Bundle paths in manifest use forward slashes; convert to platform-native for file operations
