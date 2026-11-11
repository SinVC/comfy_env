# Global Dependency Pins & Forced Overrides

Use `gov pin` to lock global package versions directly in production (`pyproject.toml` + `.venv-prod`). For resolving conflicts inside a pending candidate transaction without mutating production prematurely, use `gov resolve` or `gov update resolve` instead.

> **Preconditions**: ComfyUI stopped (`gov stop`), no pending operation recovery (`gov op recover`). `torch`, `torchvision`, and `torchaudio` are rejected (use `gov install torch`).

---

## 1. Normal Constraints vs. Forced Overrides

| Mode | Flag | Target in `pyproject.toml` | Solver Behavior |
| :--- | :--- | :--- | :--- |
| **Normal Constraint** (default) | *(none)* | `dependency-groups.overrides` | Strict constraint; `uv lock` fails if any dependency forbids the pinned version. |
| **Forced Override** | `--override-requirements` | `dependency-groups.overrides` **AND** `tool.uv.override-dependencies` | Forces `uv` to ignore conflicting upstream version bounds while keeping the package installed. |

---

## 2. Commands

### `gov pin add`
Pins one or more packages to exact versions, locks in staging, syncs `.venv-prod`, and runs the smoke test.

```text
gov pin add <SPECS>... [--override-requirements]
```

- `<SPECS>...`: One or more exact `<pkg>==<version>` pins (last value wins if a package is repeated). Warns when pinning packages outside `numpy` or `transformers`.
- **Guard**: Normal mode refuses to overwrite an existing forced override for that package unless `--override-requirements` is passed.
- **Rollback & Undo**: Rolls back on lock/sync/smoke failure. **Undoable via `gov undo`**.

### `gov pin list`
Lists declared global pins and overrides from `pyproject.toml` (reads declarative config, not installed `.venv-prod` packages).

```text
gov pin list [--details]
```

- Prints `No pins in overrides group.` if both sections are empty.
- Identical exact pins in both sections appear once. With `--details`, appends a tab-separated status label:
  - `constraint`: Only in `dependency-groups.overrides`.
  - `forced`: Matching exact pin in both `dependency-groups.overrides` and `tool.uv.override-dependencies`.
  - `forced only`: Exact pin only in `tool.uv.override-dependencies`.
  - `inconsistent` / `inconsistent override`: Same package has different versions across the two sections.
  - `manual override`: Scoped, marker-qualified, or non-exact entry in `tool.uv.override-dependencies`.

### `gov pin remove`
Removes one or more global pins by package name, re-locks in staging, syncs `.venv-prod`, and runs the smoke test.

```text
gov pin remove <SPECS>... [--override-requirements]
```

- `<SPECS>...`: Package names only (**do not** include `==version`). Fails without changes if any named package is missing.
- **Removal Rules**:
  - Paired exact pins (`forced`) are removed from both sections automatically without needing `--override-requirements`.
  - Orphaned (`forced only`) or mismatched (`inconsistent`) entries in `tool.uv.override-dependencies` require `--override-requirements`.
  - `manual override` entries (scoped, marker-qualified, or non-exact) are never removed by `gov pin remove`.
- **Rollback & Undo**: Rolls back on failure. **Undoable via `gov undo`**.
