# Code Map

## Metadata

- Status: Active
- Last Reviewed: 2026-03-01

## Primary Entry

- `bin/gov`: single executable shell script containing all current command handlers and helper logic.

## Command Families

- Environment/status:
  - `cmd_init` (line 1258)
  - `cmd_status` (line 1944)
- Node lifecycle:
  - `cmd_node_add` (line 1276)
  - `cmd_node_remove` (line 1369)
- Transactions:
  - `cmd_tx_run` (line 1484)
  - `cmd_tx_inspect` (line 1579)
  - `cmd_tx_abort` (line 1616)
  - `cmd_tx_promote` (line 1772)
  - `cmd_resolve` (line 1634)
- Audit and undo:
  - `cmd_op_list` (line 1032)
  - `cmd_op_inspect` (line 1064)
  - `cmd_undo` (line 1079)
- Runtime:
  - `cmd_run` (line 1990)
  - `cmd_stop` (line 2068)
- Routing:
  - `main` (line 2131)

## Helper Clusters

- State ledger:
  - `write_tx_file`, `tx_update_status`, `tx_set_*`, `op_begin`, `op_finalize`
- Dependency sync:
  - `write_group_deps`, `collect_freeze_file`, `build_workdir_for_tx`, `apply_plan_in_workdir`
- Source integration:
  - `comfyui_dir_path`, `plugin_install_abs_path`, `plugin_get_meta`
- Safety:
  - `core_packages_csv`, `write_conflict_report`

## Directory Ownership

- `state/transactions`: transaction artifacts
- `state/ops`: operation metadata and backups
- `state/conflicts`: conflict reports
- `state/logs`: runtime and lock logs
- `state/work`: temporary workdirs and generated plan files
- `.venv-candidate`: per-transaction candidate envs
- `.venv-prod`: production env
