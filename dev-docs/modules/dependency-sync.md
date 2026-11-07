# dependency_sync/

**Implementation target:** [src/dependency_sync/](../../src/dependency_sync/)

## Responsibility

All interactions with `uv` and the staged workdir pattern. Translates application-level intent ("add this dependency", "lock this workdir", "sync this env") into `uv` subprocess calls with structured results.

## Capabilities

### 1. UvClient

Wraps all `uv` invocations. Never called directly by application code — always through these methods.

```rust
struct UvClient {
    python: String,        // configured Python minor line
}
```

Use uv's configured cache. The CLI does not create or prune a second cache.

#### Lock

```rust
fn lock(&self, project_dir: &Path) -> Result<CmdResult>
```
Runs: `uv lock --python <python>` in `project_dir`.

#### Lock Check

```rust
fn lock_check(&self, project_dir: &Path) -> Result<CmdResult>
```
Runs: `uv lock --check --python <python>` in `project_dir`.
Used by `env import` to verify bundle integrity.

#### Sync

```rust
fn sync(&self, project_dir: &Path, env_path: &Path) -> Result<CmdResult>
```
Runs: `UV_PROJECT_ENVIRONMENT=<env_path> uv sync --python <python> --locked --all-groups` in `project_dir`.

Platform note: `env_path` must be a **platform-native absolute path** (with backslashes on Windows). `uv` expects native paths in environment variables — do not normalize to forward slashes here.

#### Add

```rust
fn add(&self, project_dir: &Path, group: &str, spec: &str, opts: AddOpts) -> Result<CmdResult>
```
Runs: `uv add --group <group> --python <python> [--no-sync|--frozen] <spec> [--index <name>=<url>]`

`AddOpts`:
- `no_sync: bool` — add without syncing
- `frozen: bool` — add without resolving (pin only)
- `index: Option<(name, url)>` — package index

#### Remove

```rust
fn remove(&self, project_dir: &Path, group: &str, pkg: &str, frozen: bool) -> Result<CmdResult>
```
Runs: `uv remove --group <group> --python <python> [--frozen] <pkg>`

#### Freeze

```rust
fn pip_freeze(&self, env_path: &Path) -> Result<Vec<String>>
```
Runs: `uv pip freeze` with the given env. Returns list of `pkg==version` strings.

#### Export

```rust
fn export_pylock(&self, project_dir: &Path) -> Result<CmdResult>
```
Runs: `uv export --format pylock.toml --locked --all-groups`

#### Python Find

```rust
fn python_find(&self, request: &str) -> Result<PathBuf>
```
Runs: `uv python find --no-python-downloads <request>`
Returns resolved interpreter path.

### 2. Staged Workdir Protocol

```rust
fn create_staged_workdir(root: &Path, work_base: &Path) -> Result<StagedWorkdir>
```

1. Create temp directory under `work_base`
2. Copy `pyproject.toml` and `uv.lock` from `root` into workdir. For a local flat wheel index, preserve the directory referenced by a relative `package.source.registry` when moving the lock in either direction.
3. Return a handle owning the workdir. Resolve relative `UV_FIND_LINKS` entries from the project root when locking the staged copy.

```rust
fn promote_workdir(workdir: &StagedWorkdir, root: &Path) -> Result<()>
```

Copy workdir's `pyproject.toml` and `uv.lock` back to root (via atomic writes).

### 3. Prod Sync Protocol

```rust
fn sync_prod(&self, root: &ProjectRoot, config: &RuntimeConfig) -> Result<()>
```

1. Determine prod env path: `<root>/<config.runtime.prod_env>`
2. Call `UvClient::sync(root, prod_env_path)`. Surface spawn errors and nonzero exit codes to the caller, including when resyncing a restored project.

### 4. Lock Conflict Detection

```rust
fn detect_lock_conflict(lock_result: &CmdResult) -> Option<ConflictInfo>
```

If lock exit code is non-zero:
- Capture full stderr as raw log
- Extract package names from stderr lines matching patterns like `package <name>`, `requires <name>`, or version incompatibility messages
- Package detection is best-effort — the raw log is always preserved for human inspection

```rust
struct ConflictInfo {
    raw_log: String,
    detected_packages: Vec<String>,  // best-effort extracted names
}
```

**Contract**: Detection is heuristic. The raw log is the authoritative record. `detected_packages` is a convenience for display — never use it as the sole basis for automated resolution. `uv` error format may change between versions; always fall back gracefully to showing the raw log if parsing extracts nothing.

Used by promote and resolve flows to create conflict artifacts via `state_ledger::write_conflict()`.

## CmdResult

```rust
struct CmdResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    command_summary: String,  // human-readable command description
    log_path: Option<PathBuf>,
}
```

**Contract**: Every `uv` call returns this. Application code checks `exit_code` and decides whether to continue, restore, or create conflict artifacts.

## Dependencies

- `std::process::Command`
- `platform/` — for env vars and path construction
- `fs_support/` — for workdir creation and atomic writes

## Used By

- `application/pin.rs` — staged add/remove + lock + sync
- `application/install.rs` — add to groups + lock + sync
- `application/node.rs` — remove group + lock + sync
- `application/tx.rs` — candidate env creation + sync
- `application/update.rs` — staged core update + lock + sync
- `application/env.rs` — lock check, prod sync
- `application/init.rs` — initial lock + sync
- `safety_guards/` — smoke test requires knowing prod env path
