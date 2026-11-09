# source_integration/

**Implementation target:** [src/source_integration/](../../src/source_integration/)

## Responsibility

All interactions with `git` and plugin source directory management. Handles cloning, checkout, path mapping, and source tree operations for custom_nodes.

## Capabilities

### 1. GitClient

```rust
struct GitClient;
```

#### Clone

```rust
fn clone(&self, url: &str, target: &Path) -> Result<CmdResult>
```
Runs: `git clone <url> <target>`

#### Checkout

```rust
fn checkout(&self, repo_path: &Path, ref_spec: &str) -> Result<CmdResult>
```
Runs: `git -C <repo_path> checkout <ref_spec>`

### 2. Plugin Path Mapping

```rust
fn plugin_install_path(comfyui_dir: &Path, node_id: &NodeId) -> PathBuf
```
Returns: `<comfyui_dir>/custom_nodes/<node_id>`

```rust
fn node_id_from_url(git_url: &str) -> NodeId
```
Derives node_id from git URL basename (strips `.git` suffix, normalizes).

### 3. Plugin Source Operations

```rust
fn snapshot_plugin(src: &Path, dst: &Path) -> Result<()>
```
Copies plugin directory to destination, **excluding `.git/` directories at any depth**. The exclusion rule is: skip any directory named exactly `.git` during recursive copy. All other files and directories are copied as-is.

Used by `env export` to create portable bundle snapshots.

```rust
fn apply_plugin_snapshots(bundle_dir: &Path, target_custom_nodes: &Path, registered_ids: &[NodeId]) -> Result<()>
```
Restores plugin directories from a bundle into `custom_nodes/`:
1. Remove directories in `target_custom_nodes/` that are NOT in `registered_ids` (cleanup untracked plugins)
2. For each registered plugin in bundle: copy `bundle_dir/custom_nodes/<id>/` → `target_custom_nodes/<id>/`

Used by `env import`.

**Failure responsibility**: If `apply_plugin_snapshots` fails partway (e.g., copy error on 3rd of 5 plugins), the **command layer** is responsible for calling `safety_guards::op_restore` which restores the pre-import custom_nodes backup. This module does not self-restore.

**`registered_ids` source**: During import, this comes from the **bundle's** `plugins.json`, not the current root registry. The bundle defines what should exist after import.

```rust
fn remove_plugin_source(path: &Path) -> Result<()>
```
Removes plugin directory. Handles read-only files (Git on Windows sets some files read-only). Used by `node remove --purge-code`.

```rust
fn list_custom_nodes(comfyui_dir: &Path) -> Result<Vec<PathBuf>>
```
Lists all directories under `<comfyui_dir>/custom_nodes/`.

## Dependencies

- `std::process::Command` (git calls)
- `fs_support/` — directory copy (with .git exclusion), directory remove

## Used By

- `application/node.rs` — clone, checkout, remove
- `application/env.rs` — snapshot for export, restore for import
- `application/tx.rs` — reads plugin install path for requirements detection
