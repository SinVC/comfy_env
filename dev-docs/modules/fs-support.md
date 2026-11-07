# fs_support/

**Implementation target:** [src/fs_support/](../../src/fs_support/)

## Responsibility

Filesystem primitives that ensure data integrity across all write operations. No other module performs raw file writes — they go through `fs_support/`.

## Capabilities

### 1. Atomic File Write

```rust
fn atomic_write(target: &Path, content: &[u8]) -> Result<()>
```

Steps:
1. Write content to a temporary file in the same directory as `target`
2. `fsync` the temporary file (flush to disk)
3. Atomic rename: temp → target
4. `fsync` parent directory (Linux; best-effort on Windows)

**Contract**: Either the full new content is visible at `target`, or the previous content remains. Never a partial write.

### 2. File SHA256

```rust
fn sha256(path: &Path) -> Result<String>
```

Returns lowercase hex-encoded SHA256 digest of file contents. Returns error if file doesn't exist.

**Contract**: Used for pre/post hashes in operations, manifest checksums in bundles, and drift guard checks.

### 3. Directory Copy

```rust
fn copy_dir(src: &Path, dst: &Path, exclude: &[&str]) -> Result<()>
```

Recursively copies `src` to `dst`, skipping directories whose names match any entry in `exclude`.

**Contract**: Used for:
- Bundle export (copy custom_nodes, exclude `.git`)
- Backup creation (copy affected directories)

Platform note: Must handle read-only files (Git on Windows), symlinks (copy the link, not traverse target).

### 4. Directory Remove

```rust
fn remove_dir_all(path: &Path) -> Result<()>
```

Removes directory and all contents. Platform-specific handling:
- Windows: clear read-only attributes before deletion (Git sets some files read-only)
- When reconciling an existing custom_nodes tree, skip linked directories and warn as specified in ADR-003; traversal must stay inside the tree.

### 5. Temporary Workdir

```rust
fn create_workdir(base: &Path) -> Result<TempWorkdir>
```

Creates a temporary directory under `base` (typically `state/work/`). Returns a handle that provides the path. Pin commands own a `TempDir` until they return. A transaction that spans CLI invocations explicitly keeps its workdir and records its path for promote or abort.

**Contract**: Workdirs are used for staged mutations — they hold copies of truth files that get mutated, locked, and tested before being promoted to root.

### 6. Existence Marker

```rust
fn backup_with_marker(src: &Path, backup: &Path, marker: &Path) -> Result<()>
fn restore_with_marker(backup: &Path, dst: &Path, marker: &Path) -> Result<()>
```

Backup: If `src` exists, copy to `backup` and write `"1"` to marker. If `src` doesn't exist, write `"0"` to marker.

Restore: Read marker. If `"1"`, copy backup → dst. If `"0"`, delete dst (file shouldn't exist).

**Contract**: Ensures restore doesn't create files that didn't exist before the operation.

## Dependencies

- `sha2` crate (for SHA256)
- `tempfile` crate (for temp file in atomic write)
- `std::fs` (for copy, rename, remove)

## Used By

- `safety_guards/` — backup/restore operations
- `dependency_sync/` — staged workdir creation
- `state_ledger/` — atomic JSON writes
- `toml_support/` — atomic TOML writes
- `source_integration/` — plugin directory copy/remove
