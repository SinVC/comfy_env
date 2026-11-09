# 05 数据与状态契约

## 1. 本地真相层与运行状态边界

本地真相文件（不进入 Git）：

1. `pyproject.toml`
2. `uv.lock`

模板/范例文件（进入 Git）：

1. `pyproject.toml.template`
2. `state/plugins.json.example`
3. `config.toml.template`

本地运行状态（不进入 Git）：

1. `state/plugins.json`
2. `state/ops/`
3. `state/transactions/`
4. `state/logs/`
5. `state/conflicts/`
6. `state/work/`

## 2. plugins.json v2 结构

`state/plugins.json` 顶层为数组，每个插件项字段：

1. `id`
2. `git_url`
3. `ref`
4. `install_relpath`（相对 `comfyui_dir`）
5. `group`（通常 `node-<id>`）
6. `managed_deps`（冗余缓存）
7. `enabled`
8. `created_at`
9. `updated_at`

语义约束：

1. `managed_deps` 与实际删除逻辑不一致时，以 `pyproject.toml` 的 `dependency-groups.<group>` 为权威。
2. `path` / `last_txid` / `last_snapshot` 已废弃。

## 3. 事务 JSON 结构

`state/transactions/<txid>.json` 核心字段：

1. 基础：`txid`, `node_id`, `started_at`, `ended_at`, `status`
2. 环境：`candidate_env`, `staged_workdir`
3. 差异：`pre_freeze`, `post_freeze`, `diff.added`, `diff.removed`, `core_impact`
4. 日志：`logs.stdout`, `logs.stderr`, `logs.run_exit_code`
5. 冲突与晋升：`conflict_report`, `resolution_pins`, `promotion_plan`, `promotion`

`promotion` 关键子字段：

1. `status`
2. `reason`
3. `op_id`
4. `pre_op_id`
5. `error`

语义约束：

1. `candidate_env` 与 `staged_workdir` 记录原始 artifact 路径，即使成功 `promote` 或 `abort` 后目录已被清理，也不会从事务 JSON 中抹除。
2. inspect 层应根据文件系统实际存在性，把缺失路径显示为 `(cleaned)` 或 `(missing)`，但不改写 JSON。

## 4. Operation 元数据结构

`state/ops/<op_id>/meta.json`：

1. `op_id`
2. `kind`（如 `promote` / `remove` / `manual` / `env_import`）
3. `ref`
4. `status`（`running` / `success` / `failed` / `undone`）
5. `started_at`
6. `ended_at`
7. `files`：每个文件包含 `pre_sha256` / `post_sha256`
8. `backup_dir`
9. `undoable`
10. `note`

备份目录固定为：

1. `state/ops/<op_id>/backup/pyproject.toml`
2. `state/ops/<op_id>/backup/uv.lock`
3. `state/ops/<op_id>/backup/plugins.json`

## 5. 状态机

事务允许状态集合：

1. `running`
2. `completed`
3. `failed`
4. `aborted`
5. `needs_resolution`
6. `resolved`
7. `promoted`
8. `promote_failed`

常见流转：

1. `running -> completed|failed|needs_resolution`
2. `running -> completed` 既包括进程自然退出 0，也包括达到观察 timeout 后被系统收束。
3. `completed -> promoted|needs_resolution|aborted`
4. `failed -> promoted|promote_failed|needs_resolution|aborted`（promote 需 `--allow-failed-run`）
5. `needs_resolution -> resolved|needs_resolution|aborted`
6. `resolved -> promoted|needs_resolution|promote_failed`

## 6. 向后兼容规则

1. 新字段只能追加，不得无迁移重命名旧字段。
2. 解析方必须容忍未知字段。
3. 修改字段语义时必须更新文档和迁移说明。

## 7. 环境重建约定

`init` / `promote` / `remove` / `undo` 的 prod 同步使用：

1. `uv sync --locked --exact --all-groups`
2. `dependency-groups` 中受管依赖属于重建范围。

## 8. Bundle 交付物契约

环境迁移 bundle 是交付物，不是本地真相层。v1 目录结构至少包含：

1. `manifest.json`
2. `pyproject.toml`
3. `uv.lock`
4. `pylock.toml`
5. `state/plugins.json`
6. `custom_nodes/<node_id>/`
7. `audit/prod-freeze.txt`
8. `audit/export-summary.json`

语义约束：

1. `manifest.json` 记录 bundle 兼容性与完整性元数据，包括 Python / 平台摘要、节点列表和关键文件 SHA256。
2. `pyproject.toml + uv.lock` 仍是导入后的本地依赖真相。
3. `pylock.toml` 是标准化导出物与审计辅助，不替代导入后的本地真相。
4. `custom_nodes/<node_id>/` 保存运行态源码快照：保留工作树中的已修改文件和未跟踪文件，但过滤 `.git/` 目录与 `.git` 指针文件。
5. `paths.comfyui_dir` 不属于 bundle 真相；目标机路径必须由 `env import --comfyui-dir` 指定。
6. `env import` 默认执行 exact restore；导入后目标机的 `custom_nodes/*` 目录集合应与 bundle registry 一致。
7. v1 不支持 partial import、nodes-only import 或按节点依赖增量导入。
