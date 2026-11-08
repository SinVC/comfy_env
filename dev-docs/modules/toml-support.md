# toml_support/

**Implementation target:** [src/toml_support/](../../src/toml_support/)

## Responsibility

Format-preserving reading and editing of `config.toml` and `pyproject.toml`. All TOML mutations go through this module — no other code touches TOML files directly.

## Design Principle

**Modify only the target node; do not reformat the file.** Edits must preserve:
- Comments
- Blank lines
- Key ordering in unmodified sections
- Inline formatting of untouched values

This minimizes audit noise and diff drift.

## Capabilities

### 1. Config Reading

```rust
fn read_config(path: &Path) -> Result<RuntimeConfig>
```

Parses `config.toml` into a typed struct. Tolerant of missing optional fields (uses defaults).

**RuntimeConfig fields:**
- `paths.comfyui_dir: String`
- `runtime.python: String` (canonical minor line)
- `runtime.prod_env: String` (default: ".venv-prod")
- `runtime.candidate_root: String` (default: ".venv-candidate")
- `tx.timeout_seconds: u32` (default: 120)
- `tx.smoke_test: Option<SmokTestConfig>` (program + args)
- `policy.core_packages: Vec<String>`
- `ops.retention_count: u32` (default: 100)
- `run.extra_args: Vec<String>` or a legacy whitespace-separated string
- `run.sync_before_run: bool`

### 2. Config Writing

```rust
fn write_config(path: &Path, config: &RuntimeConfig) -> Result<()>
```

Writes complete config.toml. Used by `init` (creates from scratch or updates existing).

### 3. Config Point Update

```rust
fn update_config_field(path: &Path, key: &str, value: &TomlValue) -> Result<()>
```

Updates a single field in config.toml without reformatting the rest. Used for targeted updates outside of `init`.

### 4. Project File: Read Dependency Group

```rust
fn read_dependency_group(path: &Path, group: &str) -> Result<Vec<String>>
```

Returns the entries of a named dependency group from pyproject.toml. Returns empty vec if group doesn't exist.

### 5. Project File: Rewrite Dependency Group

```rust
fn rewrite_dependency_group(path: &Path, group: &str, mode: GroupEditMode, specs: &[String]) -> Result<()>
```

Modes:
- `Replace` — replace entire group content with `specs`
- `UpsertExact` — for each spec, remove existing entry with same package name, then add
- `RemoveNames` — remove entries whose normalized names match `specs`

**Contract**: Only the target group's array is modified. Other groups, comments, and formatting remain untouched.

### 6. Project File: Remove Dependency Group

```rust
fn remove_dependency_group(path: &Path, group: &str) -> Result<()>
```

Removes the entire `[dependency-groups.<name>]` entry. Used by `node remove`.

### 7. Project File: Update Runtime Constraints

```rust
fn update_project_constraints(path: &Path, python_minor: &str, env_marker: &str) -> Result<()>
```

Updates:
- `project.requires-python` → `==<major>.<minor>.*`
- `[tool.uv].environments` → `[<env_marker>]`

Used by `init` to set Python and platform constraints.

## Data Format: Dependency Group Entries

Entries in `dependency-groups.*` are PEP 508 dependency strings:
- `"numpy==1.26.4"` (exact pin)
- `"transformers>=4.40"` (version range)
- `"torch"` (any version)

Package name normalization: lowercase, replace `[-_.]` with `-`, strip leading/trailing hyphens.

## Dependencies

- `toml_edit` crate (format-preserving TOML manipulation)

## Used By

- `application/init.rs` — config creation, project constraint setup
- `application/install.rs` — torch/core group editing
- `application/pin.rs` — overrides group editing
- `application/node.rs` — plugin group creation/removal
- `dependency_sync/` — reads group content for staged mutations
- `safety_guards/` — reads config for policy
