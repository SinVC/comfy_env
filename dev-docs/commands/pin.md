# pin

**Implementation target:** [src/application/pin.rs](../../src/application/pin.rs)

## `pin add`

### Synopsis

```
gov pin add <pkg==version>...
```

### Purpose

Add exact-version override pins to `dependency-groups.overrides`. uv resolves this group together with the other groups. An incompatible requirement produces a lock conflict.

### Arguments

- One or more `pkg==version` specs (positional, space-separated)

### Preconditions

- `config.toml` must exist
- Each spec must match `<name>==<version>` format
- Package must not be torch family (torch, torchvision, torchaudio) — rejected with error
- Warns (but proceeds) for non-recommended packages (numpy, transformers are recommended)

### Reads

- `config.toml` — python, prod env
- `pyproject.toml` — current overrides group

### Writes

- `pyproject.toml` — upserts entries in `dependency-groups.overrides`
- `uv.lock` — re-locked
- `.venv-prod/` — synced

### Success Path

```
1. Validate all specs are pkg==version format
2. Reject torch family packages
3. Warn for non-recommended packages
4. Copy root truth to a temporary workdir.
5. Edit the overrides array once. The last spec for each normalized name wins.
6. Lock the workdir with `uv lock --python <py>`.
7. After a successful lock, start the backup operation and promote both staged files.
8. Sync prod, run the smoke command and finalize the operation.
```

### Failure Path

```
IF validation fails: exit before mutation
IF staging or lock fails: release the temporary workdir and return the error before starting a production operation.
IF promotion fails partway: restore the backup, sync prod and record the failure.
IF sync fails: op_restore → re-sync prod → op_finalize(failed)
IF smoke test fails: op_restore → re-sync prod → op_finalize(failed)
```

### Platform Notes

- Package name normalization: lowercase, replace `[-_.]` with `-`

---

## `pin list`

### Synopsis

```
gov pin list
```

### Purpose

Display current entries in `dependency-groups.overrides`.

### Preconditions

- A missing `pyproject.toml` or overrides group produces the empty-list message.

### Reads

- `pyproject.toml` — overrides group entries

### Writes

Nothing.

### Success Path

```
1. Read dependency-groups.overrides from pyproject.toml
2. Print each entry to stdout, one per line
3. If empty: print informational message
```

---

## `pin remove`

### Synopsis

```
gov pin remove <pkg>...
```

### Purpose

Remove packages from `dependency-groups.overrides`.

### Arguments

- One or more package names (positional, normalized before matching)

### Preconditions

- `config.toml` must exist
- Each package must currently exist in overrides group
- Package must not be torch family

### Reads

- `config.toml` — python, prod env
- `pyproject.toml` — current overrides group

### Writes

- `pyproject.toml` — removes entries from `dependency-groups.overrides`
- `uv.lock` — re-locked
- `.venv-prod/` — synced

### Success Path

```
1. Normalize package names
2. Reject torch family
3. Copy truth to a temporary workdir. Remove requested names from the staged array in one edit. If any are missing, reject the whole request.
4. Lock the workdir.
5. After a successful lock, start the backup operation and promote both staged files.
6. Sync prod, run the smoke command and finalize the operation.
```

### Failure Path

```
IF package not in overrides: exit with error (no mutation)
IF staging or lock fails: release the temporary workdir and return the error before starting a production operation.
IF promotion fails partway: restore the backup, sync prod and record the failure.
IF sync/smoke fails: op_restore → re-sync prod → op_finalize(failed)
```
