# node

**Implementation target:** [src/application/node.rs](../../src/application/node.rs)

## `node add`

### Synopsis

```
gov node add <git_url> [--ref <sha|tag|branch>] [--id <node_id>]
```

### Purpose

Register a ComfyUI plugin by cloning its Git repository into `custom_nodes/`. Does not install dependencies — that requires a subsequent `tx run`.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<git_url>` | Yes | Git repository URL |
| `--ref` | No | Git ref to checkout (default: repository default branch) |
| `--id` | No | Override node_id (default: basename of git_url without `.git`) |

### Preconditions

- `config.toml` must exist
- `git` must be available
- `comfyui_dir` must exist
- Node with same id must not already exist in plugins.json

### Reads

- `config.toml` — comfyui_dir
- `state/plugins.json` — check for duplicate id

### Writes

- `custom_nodes/<node_id>/` — cloned source tree
- `state/plugins.json` — appends new plugin record

### Success Path

```
1. Derive node_id from git_url basename (or use --id)
2. Verify node_id not already registered
3. Determine install path: <comfyui_dir>/custom_nodes/<node_id>
4. `git clone <git_url> <install_path>`
5. IF --ref provided: `git -C <install_path> checkout <ref>`
6. Generate group name: "node-<normalized_node_id>"
7. Append to plugins.json:
   {
     id: node_id,
     git_url: git_url,
     ref: ref or "HEAD",
     install_relpath: "custom_nodes/<node_id>",
     group: "node-<normalized>",
     enabled: true,
     managed_deps: [],
     created_at: <utc_timestamp>,
     updated_at: <utc_timestamp>
   }
```

### Failure Path

```
IF duplicate node_id: exit with error (no mutation)
IF the target already exists, preserve its contents. Clean up a partial clone only when this invocation created the directory.
IF git checkout fails: clean up cloned directory, exit
```

### Platform Notes

- `install_relpath` uses forward slashes in plugins.json regardless of platform
- Git clone target path uses platform-native separators for the filesystem operation

---

## `node remove`

### Synopsis

```
gov node remove <node_id> [--purge-code]
```

### Purpose

Remove a plugin's dependency group from `pyproject.toml`, re-lock, sync prod, and update the registry. Optionally delete source code.

### Arguments

| Flag | Required | Description |
|------|----------|-------------|
| `<node_id>` | Yes | Plugin identifier |
| `--purge-code` | No | Also delete `custom_nodes/<node_id>/` directory |

### Preconditions

- `config.toml` must exist
- Plugin must exist in `plugins.json`

### Reads

- `config.toml` — python, prod env, comfyui_dir
- `state/plugins.json` — plugin record (group name, install path)
- `pyproject.toml` — dependency group for the plugin

### Writes

- `pyproject.toml` — removes `dependency-groups.<group_name>` section
- `uv.lock` — re-locked
- `.venv-prod/` — synced
- `state/plugins.json` — removes plugin record
- `custom_nodes/<node_id>/` — deleted if `--purge-code`

### Success Path

```
1. Load plugin record from plugins.json
2. Copy truth to a temporary workdir.
3. Remove the plugin's group in one TOML edit and lock the workdir.
4. After lock succeeds, start the node_remove operation. For --purge-code, include this plugin's source tree in its backup.
5. Apply the removal:
   a. Copy workdir truth → root
   b. Sync prod env via [Prod Sync Protocol]
   c. Smoke test via [Smoke Test Protocol]
   d. Remove plugin entry from plugins.json
   e. IF --purge-code: delete custom_nodes/<node_id>/
   f. op_finalize(success)
```

### Failure Path

```
IF plugin not found: exit with error (no mutation)
IF lock fails: discard the workdir before touching production.
IF promotion, sync, purge or registry update fails: restore the backup, re-sync prod and record the failure.
```

### Platform Notes

- Directory deletion must handle read-only files (Git on Windows sets some files read-only)
- `install_relpath` in plugins.json uses forward slashes regardless of platform
