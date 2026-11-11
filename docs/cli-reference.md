# `gov` CLI Reference

<p>
  <a href="cli-reference.md"><b>English</b></a> •
  <a href="cli-reference.zh-CN.md"><b>简体中文</b></a>
</p>

`gov` (`gov.exe` on Windows) manages ComfyUI dependencies, custom nodes, candidate transactions, and production runtime. In a Release-installed workspace, run `./bin/gov` or `.\bin\gov.exe` from the workspace root. In a manually initialized project, run from the governance directory containing `config.toml`, `pyproject.toml`, and `uv.lock`.

## Workspace installation commands

- `gov setup --workspace <dir> [--mode fresh|migrate] [--python <selector>] [--torch-index-url <url>]` is the Release installer entry point. It uses the two templates beside the extracted executable.
- `gov status --json` reports configuration, declared dependency groups, the production interpreter, pending recovery IDs, and runtime state for automation.
- `gov verify [--cpu]` performs a bounded startup check with custom nodes disabled. It exits with code `3` when the checkout lacks a supported native startup probe.

---

## Quick Workflow Overview

```bash
# 1. Initialize governance config, Python binding, and .venv-prod
gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12
gov install torch --index-url https://download.pytorch.org/whl/cu124
gov install

# 2. Install and verify a custom node safely
gov node add https://github.com/example/custom-node.git
gov tx run custom-node
gov tx inspect <TX_ID>
# (If dependency conflicts occur: gov resolve <TX_ID> --pin pkg==1.2.3)
gov tx promote <TX_ID>

# 3. Launch ComfyUI or roll back a change
gov run -- --listen 0.0.0.0 --port 8188
gov stop
gov undo <OP_ID>
```

---

## Command Index

| Group | Command | Purpose | Modifies Production |
| :--- | :--- | :--- | :---: |
| **Setup & Core** | [`gov init`](#gov-init) | Initialize or re-configure project and `.venv-prod` | Yes |
| | [`gov install torch`](#gov-install-torch) | Configure and install `torch`, `torchvision`, and `torchaudio` | Yes |
| | [`gov install`](#gov-install) | Import ComfyUI core requirements into `dependency-groups.core` | Yes |
| **Pins & Overrides** | [`gov pin add`](#gov-pin-add) | Add exact global version pins or forced overrides | Yes |
| | [`gov pin list`](#gov-pin-list) | List declared global pins and overrides | No |
| | [`gov pin remove`](#gov-pin-remove) | Remove global pins or forced overrides | Yes |
| **Custom Nodes** | [`gov node add`](#gov-node-add) | Clone and register a custom node | Yes (source + registry) |
| | [`gov node remove`](#gov-node-remove) | Unregister a node, remove its dependency group, and re-sync | Yes |
| **Plugin Transactions** | [`gov tx run`](#gov-tx-run) | Stage, lock, and test a node's dependencies in `.venv-candidate` | No |
| | [`gov tx inspect`](#gov-tx-inspect) | View transaction status, diffs, logs, and observation results | No |
| | [`gov resolve`](#gov-resolve) | Resolve lock conflicts for a plugin transaction and re-test | No |
| | [`gov tx promote`](#gov-tx-promote) | Promote an observed plugin transaction to `.venv-prod` | Yes |
| | [`gov tx abort`](#gov-tx-abort) | Discard a transaction's candidate venv and staged files | No |
| **Core Updates** | [`gov update run`](#gov-update-run) | Stage, lock, and test a ComfyUI core requirements update | No |
| | [`gov update inspect`](#gov-update-inspect) | View core update details, source hash, diffs, and logs | No |
| | [`gov update resolve`](#gov-update-resolve) | Resolve lock conflicts for a core update and re-test | No |
| | [`gov update promote`](#gov-update-promote) | Promote an observed core update to `.venv-prod` | Yes |
| | [`gov update abort`](#gov-update-abort) | Discard a core update's candidate venv and staged files | No |
| **Portability** | [`gov env export`](#gov-env-export) | Export a verified environment bundle directory | No |
| | [`gov env import`](#gov-env-import) | Restore an environment bundle directory onto this host | Yes |
| **Operations & Undo** | [`gov op list`](#gov-op-list) | List recorded production operations | No |
| | [`gov op inspect`](#gov-op-inspect) | Inspect operation hashes, backups, and recovery state | No |
| | [`gov op recover`](#gov-op-recover) | Recover an interrupted operation or repair its ledger link | Yes |
| | [`gov undo`](#gov-undo) | Restore pre-operation backup and re-sync `.venv-prod` | Yes |
| **Runtime & Status** | [`gov run`](#gov-run) | Launch ComfyUI inside `.venv-prod` | Optional (`--sync`) |
| | [`gov stop`](#gov-stop) | Stop the ComfyUI process tree started by `gov run` | No |
| | [`gov status`](#gov-status) | Show project readiness, runtime state, and recent transactions | No |
| | [`gov help`](#gov-help) | Print CLI help or subcommand usage | No |

---

## Global Conventions & Rules

### Global Options

- `-h`, `--help` — Print usage information for `gov` or any subcommand.
- `-V`, `--version` — Print the `gov` version.

### Exit Codes

- `0`: Command succeeded (or `--help` / `--version` / `help` displayed).
- `1`: Runtime or external tool failure (`uv`, `git`, sync/lock error, smoke test failure, or rollback error).
- `2`: Invalid CLI usage, unmet precondition, or policy rejection (for example, missing config, core-impact gate violation, live runtime conflict, or pending operation recovery).
- **`gov run`**: Returns ComfyUI's own exit code once launched (returns `1` or `2` if pre-launch checks fail).

### Required External Tools & Environment

- **`uv`**: Required on `PATH` for Python lookup, locking, syncing, freezing, and `pylock.toml` export.
- **`git`**: Required on `PATH` for `gov node add`.
- **Local wheel sources**: `UV_FIND_LINKS` supports comma-separated directories or URLs; relative paths resolve from the working directory where `gov` is invoked. Projects using `tool.uv.workspace` or non-relocatable local path/file-URL sources are rejected because staging and bundles relocate `pyproject.toml` and `uv.lock`.

### Concurrency, Runtime Lock & Recovery Guards

- **Production lock**: Commands that mutate production state, run recovery, abort transactions, export bundles, or start/stop ComfyUI acquire `state/comfyui.pid.lock` and fail immediately if another `gov` process holds it.
- **Stop-before-write rule**: Production-mutating commands fail if a `gov`-managed ComfyUI process is running. Stop it with `gov stop` first.
- **Recovery gate**: If a previous production operation was interrupted and still needs recovery, all subsequent production writes and `gov run` are blocked until you resolve it with `gov op recover <OP_ID>`.

### Package Specs, Normalization & Torch Protection

- **Name normalization**: Package names and node dependency groups are normalized by lowercasing and collapsing runs of `-`, `_`, and `.` to `-` (e.g., `Foo_Bar.Baz` becomes `foo-bar-baz`).
- **Exact pins**: Pin flags require `<package>==<version>` with no whitespace, version ranges, extras, or environment markers.
- **Torch family exclusivity**: `torch`, `torchvision`, and `torchaudio` are managed exclusively by `gov install torch`. They are rejected by `gov pin`, `gov resolve`, and `gov update resolve`, and automatically stripped from `requirements.txt` during `gov install` and `gov update run`.
- **`requirements.txt` restrictions**: Blank lines and `#` comments are ignored. Options starting with `-` (such as `-r` or `--index-url`), relative local paths starting with `.` or containing `\`, and invalid package specs are rejected.

### Core Impact Gate (`policy.core_packages`)

Transactions whose net package diff adds, upgrades, downgrades, or removes any package listed in `policy.core_packages` (default: `torch`, `torchvision`, `torchaudio`, `xformers`, `triton`, `onnxruntime`, `onnxruntime-gpu`, `numpy`) require `--approve-core` during `gov tx promote` or `gov update promote`.

### Transaction Lifecycle & IDs

- **`<TX_ID>` format**: `<UTC_TIMESTAMP>-<8_HEX>` (for example, `20260419T120000Z-a1b2c3d4`).
- **`<OP_ID>` format**: `<UTC_TIMESTAMP>-op-<8_HEX>` (for example, `20260419T120000Z-op-e5f6a7b8`).
- **Transaction statuses**:
  - `running` — Lock or candidate observation is currently executing (or crashed mid-run).
  - `needs_resolution` — `uv lock` failed due to dependency conflicts; conflict report written to `state/conflicts/<TX_ID>.json`.
  - `completed` — Initial `tx run` or `update run` passed candidate sync and startup probe. Ready to promote.
  - `resolved` — `resolve` or `update resolve` locked cleanly and passed candidate observation. Ready to promote.
  - `failed` — Lock infrastructure error, candidate sync failure, timeout, interruption, probe failure, node load failure, or candidate package drift.
  - `promoted` — Applied to `.venv-prod`.
  - `promote_failed` — Promotion failed and rolled back.
  - `aborted` — Discarded via `tx abort` or `update abort`.

### Relevant `config.toml` Settings

Most CLI defaults come from `config.toml` (created from `config.toml.template` on `gov init`):

| Key | Default | Description |
| :--- | :--- | :--- |
| `paths.comfyui_dir` | — | Absolute path to the ComfyUI directory |
| `runtime.python` | — | Python selector (`3.12`) or resolved interpreter path |
| `runtime.prod_env` | `.venv-prod` | Production virtual environment directory (relative to project root) |
| `runtime.candidate_root` | `.venv-candidate` | Parent directory for per-transaction candidate venvs |
| `tx.timeout_seconds` | `120` | Default timeout (seconds) for candidate probes and smoke tests |
| `tx.smoke_test` | Python version check | Optional `{ program = "...", args = [...] }` table run after production syncs; set `program = "python"` to use `.venv-prod`'s Python (`tx.smoke_test_cmd` is deprecated and ignored) |
| `policy.core_packages` | Torch, xformers, triton, onnxruntime, numpy | Packages gated by `--approve-core` |
| `ops.retention_count` | `100` | Maximum completed operations kept during history pruning |
| `run.extra_args` | `[]` | Extra arguments passed to ComfyUI by `gov run` (string array or legacy space-separated string) |
| `run.sync_before_run` | `false` | Always sync `.venv-prod` before launching ComfyUI |

---

## Project Setup & Core Dependencies

### `gov init`

Initialize or re-initialize the governance project, `pyproject.toml`, `uv.lock`, `state/`, and `.venv-prod`.

```text
gov init [--comfyui-dir <COMFYUI_DIR>] [--python <PYTHON>]
```

| Option | Required | Description |
| :--- | :---: | :--- |
| `--comfyui-dir <COMFYUI_DIR>` | First run | Absolute path to the ComfyUI installation directory |
| `--python <PYTHON>` | First run | Python `<major>.<minor>` (e.g., `3.12`), patch version, executable name, or interpreter path |

**Behavior:**
- **First initialization**: Both `--comfyui-dir` and `--python` are required. Fails if `.venv-prod` already exists without recoverable project files (`config.toml`, `pyproject.toml`, and `uv.lock`).
- **Re-initialization**: When `config.toml`, `pyproject.toml`, and `uv.lock` exist, omitted flags reuse the configured values.
- **Python & platform binding**: Pure `<major>.<minor>` selectors are used directly; patch versions, names, or paths are resolved via `uv python find --no-python-downloads` and normalized to `<major>.<minor>`. Writes `requires-python = "==<major>.<minor>.*"` and sets `[tool.uv].environments` to the current host OS/architecture marker.
- **Safety & rollback**: Stages and locks `pyproject.toml` before touching root files. On first init, sync failures restore files and delete the newly created venv. On re-init, sync failures restore the previous config, truth files, and registry, and re-sync `.venv-prod` with the prior Python interpreter. Not undoable via `gov undo`.

**Examples:**

```bash
gov init --comfyui-dir /opt/ComfyUI --python 3.12
gov init --python 3.12.8
```

---

### `gov install torch`

Configure a PyTorch package index and install `torch`, `torchvision`, and `torchaudio` into `dependency-groups.torch`.

```text
gov install torch --index-url <INDEX_URL> [--torch <TORCH>] [--torchvision <TORCHVISION>] [--torchaudio <TORCHAUDIO>]
```

| Option | Required | Description |
| :--- | :---: | :--- |
| `--index-url <INDEX_URL>` | Yes | PyTorch wheel index URL (must contain no whitespace) |
| `--torch <TORCH>` | No | Exact pin for `torch` (e.g., `torch==2.5.1`) |
| `--torchvision <TORCHVISION>` | No | Exact pin for `torchvision` (e.g., `torchvision==0.20.1`) |
| `--torchaudio <TORCHAUDIO>` | No | Exact pin for `torchaudio` (e.g., `torchaudio==2.5.1`) |

**Behavior:**
- Requires an initialized `config.toml`.
- Registers an explicit `[tool.uv.index]` entry for `--index-url` and binds `torch`, `torchvision`, and `torchaudio` to it via `[tool.uv.sources]`. Omitted packages default to unpinned constraints (`torch`, `torchvision`, `torchaudio`).
- Locks in staging, updates root truth, syncs `.venv-prod`, and runs a Torch smoke test (`import torch, torchvision, torchaudio`).
- If sync or smoke test fails, restores root truth and re-syncs `.venv-prod`. Recorded as an undoable operation.

**Example:**

```bash
gov install torch --index-url https://download.pytorch.org/whl/cu124 --torch torch==2.5.1
```

---

### `gov install`

Import ComfyUI's core requirements into `dependency-groups.core` and sync `.venv-prod`.

```text
gov install [--requirements-file <REQUIREMENTS_FILE>]
```

| Option | Required | Description |
| :--- | :---: | :--- |
| `--requirements-file <REQUIREMENTS_FILE>` | No | Path to a `requirements.txt` file (default: `<comfyui_dir>/requirements.txt`) |

**Behavior:**
- Requires an initialized `config.toml` and all three Torch-family packages already present in `dependency-groups.torch` (run `gov install torch` first).
- Automatically filters out `torch`, `torchvision`, and `torchaudio` to avoid index/source conflicts.
- Replaces `dependency-groups.core`, locks in staging, updates root truth, syncs `.venv-prod`, and runs the smoke test. Rolls back on sync/smoke failure. Recorded as an undoable operation.

**Example:**

```bash
gov install
gov install --requirements-file /opt/ComfyUI/requirements.txt
```

---

## Global Pins & Overrides

### `gov pin add`

Pin one or more packages to exact versions globally, then lock, sync `.venv-prod`, and run the smoke test.

```text
gov pin add <SPECS>... [--override-requirements]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<SPECS>...` | Yes | One or more exact pins in `<pkg>==<version>` format |
| `--override-requirements` | No | Also write the pin to `tool.uv.override-dependencies` to force-override conflicting package bounds |

**Behavior:**
- Rejects non-exact specs (e.g., `>=`) and Torch-family packages. If the same package appears multiple times in one command, the last value wins.
- Emits a warning when pinning packages outside the commonly pinned set (`numpy`, `transformers`).
- **Normal mode (default)**: Writes to `dependency-groups.overrides`, acting as a strict constraint; `uv lock` fails if any dependency forbids that version. Refuses to overwrite an active forced override for that package unless `--override-requirements` is passed.
- **Force mode (`--override-requirements`)**: Writes to both `dependency-groups.overrides` and `tool.uv.override-dependencies`, instructing `uv` to ignore conflicting upstream version bounds while keeping the package installed.
- Locks in staging before modifying root files; rolls back on sync or smoke failure. Recorded as an undoable operation.

**Examples:**

```bash
gov pin add numpy==1.26.4 transformers==4.44.0
gov pin add numpy==1.26.4 --override-requirements
```

---

### `gov pin list`

List globally declared pins and `uv` overrides from `pyproject.toml`.

```text
gov pin list [--details]
```

| Option | Required | Description |
| :--- | :---: | :--- |
| `--details` | No | Append tab-separated status labels for each entry |

**Behavior:**
- Reads declarative entries from `dependency-groups.overrides` and `tool.uv.override-dependencies` (not currently installed packages in `.venv-prod`).
- Prints `No pins in overrides group.` if both are empty.
- Matching exact entries present in both sections are printed once. With `--details`, each line includes one of:
  - `constraint` — Present only in `dependency-groups.overrides`
  - `forced` — Identical exact pin in both `dependency-groups.overrides` and `tool.uv.override-dependencies`
  - `forced only` — Exact pin present only in `tool.uv.override-dependencies`
  - `inconsistent` / `inconsistent override` — Same package appears in both sections with different versions
  - `manual override` — Scoped, marker-qualified, or non-exact entry in `tool.uv.override-dependencies`

---

### `gov pin remove`

Remove one or more global pins by package name, then lock, sync `.venv-prod`, and run the smoke test.

```text
gov pin remove <SPECS>... [--override-requirements]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<SPECS>...` | Yes | One or more package names (do not include `==version`) |
| `--override-requirements` | No | Allow removing unmatched or orphaned entries in `tool.uv.override-dependencies` |

**Behavior:**
- Matches by normalized package name and rejects Torch-family packages. Fails without changing anything if any specified package is missing.
- Paired exact pins (matching in `dependency-groups.overrides` and `tool.uv.override-dependencies`) are removed together automatically.
- Orphaned or mismatched entries in `tool.uv.override-dependencies` require `--override-requirements`. Scoped, marker-qualified, or non-exact manual entries in `tool.uv.override-dependencies` are never removed.
- Locks in staging before touching production; rolls back on sync or smoke failure. Recorded as an undoable operation.

**Example:**

```bash
gov pin remove numpy
gov pin remove numpy --override-requirements
```

---

## Custom Nodes

### `gov node add`

Clone a custom node repository into `<comfyui_dir>/custom_nodes/<node_id>` and register it in `state/plugins.json`.

```text
gov node add <GIT_URL> [--ref <REF_NAME>] [--id <ID>]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<GIT_URL>` | Yes | Git repository URL |
| `--ref <REF_NAME>` | No | Git branch, tag, or commit SHA to check out (default: repository `HEAD`) |
| `--id <ID>` | No | Custom node ID (default: repository basename without `.git`) |

**Behavior:**
- Node IDs may contain ASCII letters, digits, `.`, `_`, and `-` (cannot be `.` or `..`).
- Rejects duplicate node IDs, existing directories at `custom_nodes/<node_id>`, unowned `node-*` groups in `pyproject.toml`, and normalized group collisions (e.g., adding `foo_bar` when `foo-bar` already owns group `node-foo-bar`).
- Clones into a temporary directory outside `custom_nodes`, checks out `--ref` if provided, then moves the directory into `custom_nodes/<node_id>` and updates `state/plugins.json`.
- **Does not install dependencies.** Run `gov tx run <node_id>` to stage and verify dependencies. Not undoable via `gov undo` (use `gov node remove --purge-code` instead).

**Example:**

```bash
gov node add https://github.com/ltdrdata/ComfyUI-Manager.git --id comfyui-manager
```

---

### `gov node remove`

Unregister a custom node, remove its dependency group from `pyproject.toml`, re-lock, and sync `.venv-prod`.

```text
gov node remove <NODE_ID> [--purge-code]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<NODE_ID>` | Yes | Registered node ID |
| `--purge-code` | No | Also delete `<comfyui_dir>/custom_nodes/<NODE_ID>` |

**Behavior:**
- Removes `dependency-groups.node-<normalized_id>`, re-locks in staging, updates root truth, syncs `.venv-prod`, runs the smoke test, and removes the record from `state/plugins.json`.
- Does not remove global pins in `dependency-groups.overrides`.
- Refuses if the dependency group is shared by another registered node.
- With `--purge-code`, verifies that `custom_nodes/<NODE_ID>` exists, backs it up before deleting, and restores it if sync or smoke testing fails. Without `--purge-code`, the node directory stays on disk. Recorded as an undoable operation.

**Example:**

```bash
gov node remove comfyui-manager --purge-code
```

---

## Plugin Dependency Transactions

### `gov tx run`

Stage a registered node's `requirements.txt`, solve dependencies, and observe ComfyUI startup inside `.venv-candidate/<TX_ID>`.

```text
gov tx run <NODE_ID> [--timeout <SECONDS>]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<NODE_ID>` | Yes | Registered node ID with source present under `custom_nodes/<NODE_ID>` |
| `--timeout <SECONDS>` | No | Observation timeout in seconds (default: `tx.timeout_seconds` in `config.toml`, `120`) |

**Behavior:**
- Fails if a `running` transaction already exists for `<NODE_ID>`.
- Reads `<comfyui_dir>/custom_nodes/<NODE_ID>/requirements.txt` (an absent `requirements.txt` yields an empty dependency list), writes `dependency-groups.<node_group>` in a staged copy of production truth, and runs `uv lock`.
- **On lock conflict**: Saves conflict details to `state/conflicts/<TX_ID>.json`, marks the transaction `needs_resolution`, and prints `<TX_ID>`. Resolve with `gov resolve <TX_ID>`.
- **On lock success**: Creates `.venv-candidate/<TX_ID>`, syncs dependencies, and launches ComfyUI with `--quick-test-for-ci`, streaming output to the terminal and saving logs under `state/logs/`.
- **Observation result**:
  - `completed`: Candidate sync succeeded, ComfyUI reached its native `--quick-test-for-ci` exit with code `0`, the target node reported successful load, and no unexpected packages were installed/modified inside the candidate venv during the run.
  - `failed`: Candidate sync failed, run timed out or was interrupted, exited nonzero, ComfyUI did not support/complete the `--quick-test-for-ci` probe, target node failed to load, or runtime package drift occurred (e.g., node startup code invoked `pip install`).

**Example:**

```bash
gov tx run comfyui-manager --timeout 180
```

---

### `gov tx inspect`

Display detailed state, diffs, logs, and observation evidence for a plugin transaction.

```text
gov tx inspect <TX_ID>
```

| Argument | Required | Description |
| :--- | :---: | :--- |
| `<TX_ID>` | Yes | Plugin transaction ID |

**Behavior:**
- Prints transaction metadata, timestamps, package diff (`Added` / `Removed`), `Core impact`, exit code, timeout status, candidate/stage artifact paths (annotated `(cleaned)` or `(missing)` if absent), log file paths, observation outcome, node load failures, candidate package drift, conflict summary/hint (when `needs_resolution`), and promotion/undo status.

---

### `gov resolve`

Provide dependency pins to resolve a `needs_resolution` plugin transaction, re-lock, and re-run candidate observation.

```text
gov resolve <TX_ID> [--pin <PINS>]... [--pins-file <PINS_FILE>] [--override-requirements]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<TX_ID>` | Yes | Plugin transaction ID in `needs_resolution` |
| `--pin <PINS>` | No | Exact pin `<pkg>==<version>` (repeatable) |
| `--pins-file <PINS_FILE>` | No | File with one `<pkg>==<version>` pin per line (blank lines and `#` comments ignored) |
| `--override-requirements` | No | Also add pins to `tool.uv.override-dependencies` to override conflicting upstream bounds |

**Behavior:**
- If neither `--pin` nor `--pins-file` is given, prints the conflict summary and reads `<pkg>==<version>` lines interactively from standard input until an empty line or EOF.
- Merges pins by normalized package name (last value wins); rejects Torch-family packages.
- Re-stages from **current** production truth plus the node's recorded requirements and pins, writes pins to `dependency-groups.overrides` (and `tool.uv.override-dependencies` if `--override-requirements` is set), and re-runs `uv lock`.
- If lock still fails, keeps `needs_resolution` with the updated conflict report. If lock succeeds, re-syncs and re-observes the candidate; a passing run sets status to `resolved`. Production remains unchanged until `gov tx promote`.

**Examples:**

```bash
gov resolve 20260419T120000Z-a1b2c3d4 --pin numpy==1.26.4 --pin pillow==10.4.0
gov resolve 20260419T120000Z-a1b2c3d4 --pins-file pins.txt --override-requirements
```

---

### `gov tx promote`

Apply an observed plugin transaction's locked snapshot to `.venv-prod` and root truth files.

```text
gov tx promote <TX_ID> [--approve-core] [--reason <REASON>] [--allow-failed-run] [--keep-artifacts]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<TX_ID>` | Yes | Plugin transaction ID (`completed` or `resolved`, or eligible `failed` with `--allow-failed-run`) |
| `--approve-core` | Conditional | Required when the transaction modifies any package in `policy.core_packages` |
| `--reason <REASON>` | No | Optional note recorded with the promotion |
| `--allow-failed-run` | No | Permit promoting a `failed` transaction whose only failure was a nonzero runtime exit after clean candidate sync |
| `--keep-artifacts` | No | Keep `.venv-candidate/<TX_ID>` and staged workdir after successful promotion |

**Behavior:**
- Verifies that current root `pyproject.toml`/`uv.lock`, staged truth, `.venv-prod` installed packages, resolved Python interpreter, and the node's dependency group still match what was observed. If anything drifted, refuses and instructs you to re-run `gov tx run` or `gov resolve`.
- `--allow-failed-run` cannot bypass sync failures, timeouts, interruptions, probe failures, node load failures, or candidate package drift.
- Backs up current truth, promotes the observed lock directly without re-solving, syncs `.venv-prod`, runs the smoke test, verifies the resulting Python interpreter and package set match the observation, updates `state/plugins.json`, and cleans up candidate/stage artifacts unless `--keep-artifacts` is set.
- Automatically rolls back on failure. Recorded as an undoable operation.

**Examples:**

```bash
gov tx promote 20260419T120000Z-a1b2c3d4
gov tx promote 20260419T120000Z-a1b2c3d4 --approve-core --reason "Allow numpy upgrade"
```

---

### `gov tx abort`

Clean up a plugin transaction's candidate venv and staged workdir and mark it `aborted`.

```text
gov tx abort <TX_ID> [--confirm-stopped]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<TX_ID>` | Yes | Plugin transaction ID |
| `--confirm-stopped` | Conditional | Required for legacy `running` records lacking process identity metadata, after confirming processes exited |

**Behavior:**
- Allowed from `running`, `completed`, `failed`, `needs_resolution`, `resolved`, `promote_failed`, or `aborted` (idempotent). Refused for `promoted` transactions or when a promotion operation still needs `gov op recover`.
- For `running` transactions, checks that owner and candidate processes have exited (`--confirm-stopped` cannot override evidence of a live process).
- Deletes `.venv-candidate/<TX_ID>` and the staged workdir while retaining the transaction JSON and log files.

---

## Core Dependency Updates

Core update transactions use the same staging, observation, conflict, approval, and rollback mechanics as plugin transactions, targeting `dependency-groups.core`.

### `gov update run`

Stage and test an update to ComfyUI's core dependencies inside `.venv-candidate/<TX_ID>`.

```text
gov update run [--requirements-file <REQUIREMENTS_FILE>] [--timeout <SECONDS>]
```

| Option | Required | Description |
| :--- | :---: | :--- |
| `--requirements-file <REQUIREMENTS_FILE>` | No | Path to a `requirements.txt` file (default: `<comfyui_dir>/requirements.txt`) |
| `--timeout <SECONDS>` | No | Observation timeout in seconds (default: `tx.timeout_seconds` in `config.toml`, `120`) |

**Behavior:**
- Requires an initialized `config.toml`, `<comfyui_dir>/main.py`, and populated `dependency-groups.torch`.
- Filters out `torch`, `torchvision`, and `torchaudio`, replaces `dependency-groups.core` in staging, locks, and runs the candidate `--quick-test-for-ci` probe.
- Any custom-node load failure reported during the candidate startup probe fails the observation (`failed`). Lock conflicts produce `needs_resolution`.

---

### `gov update inspect`

Inspect a `core_update` transaction.

```text
gov update inspect <TX_ID>
```

Identical to [`gov tx inspect`](#gov-tx-inspect), and also displays `Source requirements` path and `Source SHA256`.

---

### `gov update resolve`

Resolve a `needs_resolution` core update transaction using explicit pins and re-test in `.venv-candidate/<TX_ID>`.

```text
gov update resolve <TX_ID> [--pin <PINS>]... [--pins-file <PINS_FILE>] [--override-requirements]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<TX_ID>` | Yes | Core update transaction ID in `needs_resolution` |
| `--pin <PINS>` | No | Exact pin `<pkg>==<version>` (repeatable) |
| `--pins-file <PINS_FILE>` | No | File with one `<pkg>==<version>` pin per line |
| `--override-requirements` | No | Force `uv` to override conflicting upstream bounds via `tool.uv.override-dependencies` |

**Behavior:**
- Non-interactive: requires at least one pin supplied via `--pin` and/or `--pins-file`.
- By default, `--pin` adds normal constraints to `dependency-groups.overrides` (unlike legacy behavior where `update resolve` forced overrides by default—pass `--override-requirements` if you need forced overrides).
- Re-stages from current truth, locks, and re-observes the candidate (`resolved` on pass).

**Example:**

```bash
gov update resolve 20260419T120000Z-a1b2c3d4 --pin numpy==1.26.4 --override-requirements
```

---

### `gov update promote`

Promote an observed `core_update` transaction to `.venv-prod`.

```text
gov update promote <TX_ID> [--approve-core] [--reason <REASON>] [--allow-failed-run] [--keep-artifacts]
```

Applies the exact same snapshot, baseline, `--approve-core`, `--allow-failed-run`, production sync, smoke test, rollback, and artifact-cleanup rules as [`gov tx promote`](#gov-tx-promote).

---

### `gov update abort`

Discard a `core_update` transaction's candidate venv and staged workdir and mark it `aborted`.

```text
gov update abort <TX_ID> [--confirm-stopped]
```

Applies the same process-check and cleanup rules as [`gov tx abort`](#gov-tx-abort).

---

## Environment Bundles

### `gov env export`

Export the current locked dependencies, plugin registry, and custom-node source snapshots into a portable bundle directory.

```text
gov env export <OUTPUT_DIR>
```

| Argument | Required | Description |
| :--- | :---: | :--- |
| `<OUTPUT_DIR>` | Yes | Destination directory path (must not already exist; must be outside all node source directories) |

**Behavior:**
- Requires `config.toml` (with absolute `comfyui_dir` and `runtime.python`), `pyproject.toml`, `uv.lock`, `state/plugins.json`, and every registered node's `custom_nodes/<node_id>` directory.
- Runs `uv export --format pylock.toml --locked --all-groups` (fails without re-locking if `uv.lock` is out of date, or if your `uv` version does not support `pylock.toml` export).
- Writes into `<OUTPUT_DIR>`:
  - `pyproject.toml`, `uv.lock`, `pylock.toml`, `state/plugins.json`
  - `custom_nodes/<node_id>/` — working-tree snapshots of each registered node (includes uncommitted edits and untracked files, excludes `.git`, and rejects symlinks/junctions)
  - `audit/prod-freeze.txt`, `audit/export-summary.json`
  - `manifest.json` — format v1 metadata, Python/OS/arch fields, `node_ids`, and SHA-256 checksums for all payload files
- Deletes partial output on failure.

**Example:**

```bash
gov env export /tmp/comfy-bundle
```

---

### `gov env import`

Restore an exported bundle directory onto the current machine as an exact replacement of the governance environment and managed `custom_nodes`.

```text
gov env import <BUNDLE_DIR> --comfyui-dir <COMFYUI_DIR> --python <PYTHON>
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `<BUNDLE_DIR>` | Yes | Path to an exported bundle directory |
| `--comfyui-dir <COMFYUI_DIR>` | Yes | Absolute path to the target ComfyUI directory |
| `--python <PYTHON>` | Yes | Python selector resolved via `uv python find --no-python-downloads` |

**Behavior:**
- **Pre-write verification**: Validates `manifest.json` (version `1`), all SHA-256 checksums, same OS and CPU architecture (`sys_platform`, `platform_system`, `platform_machine`), `node-*` group ownership, non-overlapping paths, `uv lock --check`, and a `uv sync --dry-run` compatibility check using the resolved target Python interpreter. If `.venv-prod` already exists without an existing `uv.lock` and `pyproject.toml`, import refuses so rollback remains possible.
- **Exact restore**: Replaces root `pyproject.toml`, `uv.lock`, `state/plugins.json`, writes `paths.comfyui_dir` and the resolved Python interpreter path into `config.toml`, syncs `.venv-prod`, replaces `custom_nodes/<node_id>` directories, and runs the smoke test.
- **Extra entries in `custom_nodes/`**:
  - Ordinary directories not in the bundle are **removed** (and backed up for rollback during import).
  - Top-level regular files, symlinks, and junctions are **preserved**. If a bundle node collides with a regular file, import aborts before writing; if it collides with a symlink/junction, the link is preserved with a warning and that node's bundled source is skipped.
- Rolls back truth, `.venv-prod`, and any moved node directories on failure. **Cannot be undone with `gov undo`** (use `gov op recover` only if interrupted mid-write).

**Example:**

```bash
gov env import /tmp/comfy-bundle --comfyui-dir /opt/ComfyUI --python 3.12
```

---

## Operation Ledger, Recovery & Undo

### `gov op list`

List recorded operations, newest first.

```text
gov op list
```

Prints tab-separated columns: `op_id`, `kind`, `status`, `reference`, `started_at`, and `needs_recovery` (when recovery is required). Warns and skips damaged historical records.

---

### `gov op inspect`

Show detailed metadata, file hashes, backup path, and recovery/undo status for one operation.

```text
gov op inspect <OP_ID>
```

| Argument | Required | Description |
| :--- | :---: | :--- |
| `<OP_ID>` | Yes | Operation ID |

---

### `gov op recover`

Recover an interrupted production operation or finish recording its transaction/undo link.

```text
gov op recover <OP_ID>
```

| Argument | Required | Description |
| :--- | :---: | :--- |
| `<OP_ID>` | Yes | Operation ID reported as needing recovery |

**Behavior:**
- Requires ComfyUI to be stopped (`gov stop`) and verifies the original operation owner and external subprocesses have exited.
- **Interrupted before commit** (`prepared` / `applying` / failed restore): Restores backed-up truth/config/registry and any moved node sources, and re-syncs (or removes a newly created) `.venv-prod`.
- **Committed, missing ledger link**: Marks the linked transaction as `promoted` (or original operation as `undone`) without re-running `uv sync`.
- Safe to repeat after recovery completes. Historical `running` records missing phase metadata preserve backups and require manual verification.

---

### `gov undo`

Revert a successful undoable operation by restoring its backup and re-syncing `.venv-prod`.

```text
gov undo <OP_ID>
```

| Argument | Required | Description |
| :--- | :---: | :--- |
| `<OP_ID>` | Yes | Operation ID of a `success` operation with `undoable: true` |

**Behavior:**
- **Drift guard**: Compares current SHA-256 hashes of `pyproject.toml`, `uv.lock`, and `state/plugins.json` against the operation's `post_sha256`. Any subsequent modification blocks `undo`.
- Restores backed-up truth, registry, and custom-node source (if the operation modified or purged node source), syncs `.venv-prod`, and marks the original operation `undone`.
- **Excluded from `gov undo`**: `init`, `reinit`, `node add`, `env import`, and `run --sync` (`runtime_sync`).

---

## Runtime & Status

### `gov run`

Start ComfyUI (`<comfyui_dir>/main.py`) using `.venv-prod` and return its exit code.

```text
gov run [--sync] [-- <ARGS>...]
```

| Argument / Option | Required | Description |
| :--- | :---: | :--- |
| `--sync` | No | Run a locked `uv sync` on `.venv-prod` before launching ComfyUI |
| `<ARGS>...` | No | Arguments passed directly to ComfyUI after `--` |

**Behavior:**
- Requires `config.toml`, `<comfyui_dir>/main.py`, and `.venv-prod`. Refuses to launch if another `gov`-managed ComfyUI process is already running or if an operation needs `gov op recover`.
- If `--sync` is passed or `run.sync_before_run = true` in `config.toml`, syncs `.venv-prod` against `pyproject.toml` + `uv.lock` first (recorded in `state/ops/`, not undoable).
- Prepends `run.extra_args` from `config.toml` before any trailing `<ARGS>...`.
- Writes process identity to `state/comfyui.pid`, streams ComfyUI output to the terminal, cleans up the PID file on exit, and forwards ComfyUI's exit code.

**Example:**

```bash
gov run --sync -- --listen 0.0.0.0 --port 8188
```

---

### `gov stop`

Stop the `gov`-managed ComfyUI process tree recorded in `state/comfyui.pid`.

```text
gov stop
```

**Behavior:**
- Attempts graceful termination (`Ctrl+Break` on Windows, `SIGTERM` on Linux), waits up to 30 seconds, force-terminates any remaining processes in the managed tree/group, and removes `state/comfyui.pid`.
- Succeeds cleanly as a no-op if no PID file exists or the recorded process has already exited (cleaning up stale PID files).
- Refuses to signal a live process recorded in a legacy numeric-only PID file whose identity cannot be verified; stop it manually after verifying its identity.

---

### `gov status`

Print a read-only summary of project configuration, environment readiness, tools, plugins, transactions, and runtime status.

```text
gov status
```

Outputs `project_root`, `config_ready`, `operations_needing_recovery` (when non-empty), `comfyui_dir`, `python`, `prod_env_exists`, `torch_ready`, `core_ready`, `lock_exists`, `plugins_registered`, `plugins_enabled`, `transactions_total`, `transactions_pending`, `update_transactions_pending`, `transactions_recent`, `runtime_running`, `tool_uv`, `tool_git`, `tool_python`, `transaction_counts`, `recent_transactions` (up to 8), and `notes`.

---

### `gov help`

Print CLI usage help for `gov` or a specific subcommand.

```text
gov help [COMMAND]...
```

**Examples:**

```bash
gov help
gov help tx promote
```
