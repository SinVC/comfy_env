# 04 CLI 命令参考

> 本文记录 Bash 版 `bin/gov` 的命令行为（Batch-4）。Rust 分支的可用命令见 [README](../README.md)，pin、审计及撤销的当前契约见 [pin](../dev-docs/commands/pin.md) 和 [ops](../dev-docs/commands/ops.md)。

## 1. 首装与核心依赖

### `gov init --comfyui-dir <abs-path> --python <python-spec>`

用途：幂等初始化本地治理配置，写入 `paths.comfyui_dir` 和规范化后的 `runtime.python`，同步收紧 `pyproject.toml` 的 Python / 平台约束，并创建/同步最小 prod 环境。

关键行为：

1. 首次初始化时，两个参数都必填。
2. 已有 `config.toml` 时，未传参数沿用现值，传入参数覆盖现值。
3. 若 `pyproject.toml` 缺失，则从 `pyproject.toml.template` 生成。
4. `--python` 若传入纯 `<major>.<minor>`，会直接按该 minor 线写入；若传入 patch 版本或解释器选择器，则会调用 `uv python find --no-python-downloads` 解析到本机可执行解释器，再规范化为 minor 线写入 `runtime.python`。例如 `3.13.12` 最终会写成 `3.13`。
5. `init` 会把 `pyproject.toml` 中的 `project.requires-python` 收紧为 `==<major>.<minor>.*`。
6. `init` 会按当前主机自动写入 `[tool.uv].environments`，把 lock 收敛到单一目标平台。

### `gov install torch --index-url <url> [--torch <torch==version>] [--torchvision <torchvision==version>] [--torchaudio <torchaudio==version>]`

用途：先安装受治理的 `torch/torchvision/torchaudio`，写入 `dependency-groups.torch`。

关键行为：

1. 内部使用 `uv add --group torch ... --index <derived-name>=<url>`。
2. 若传入 `--torch/--torchvision/--torchaudio`，对应 flag 只接受匹配自身包名的精确版本 spec，如 `--torch torch==2.11.1`。
3. 若提供精确版本 flag，CLI 会先用 `uv add` 建立 torch source/index 绑定，再把 `dependency-groups.torch` 重写为目标 spec 集合后重新 lock。
4. 成功后会同步 prod，并执行 `import torch, torchvision, torchaudio` smoke test。
5. 会生成可撤销的 operation。

### `gov install [--requirements-file <path>]`

用途：把 ComfyUI 核心依赖导入到 `dependency-groups.core` 并同步 prod。

关键行为：

1. 默认读取 `${paths.comfyui_dir}/requirements.txt`。
2. 若 torch 还未通过 `gov install torch` 安装，会直接阻断。
3. 会过滤 `torch`、`torchvision`、`torchaudio`，避免双重权威。
4. 会生成可撤销的 operation。

### `gov pin add <pkg==version>...`

用途：把一个或多个精确版本 pin 写入 `dependency-groups.overrides`，重新 lock、同步 prod，并执行 smoke test。

关键行为：

1. 只接受 `pkg==version` 格式，不支持范围约束。
2. 对同名包执行 upsert：同一命令内按 last-wins 去重；已有 override 会先按包名移除，再写回新的 exact spec，不会累积重复 pin。
3. 不接受 `torch`、`torchvision`、`torchaudio`；这三者由 `gov install torch` 单独治理。
4. 会对非推荐关键包输出警告；推荐包集合当前为 `numpy`、`transformers`。
5. 在 staged workdir 中先对当前已存在的目标包执行 `uv remove --group overrides --python "$py" --frozen`，再执行 `uv add --group overrides --python "$py" --frozen` 写回目标 exact specs，只有显式 lock 成功后才晋升到 root truth。
6. sync 或 smoke test 失败会恢复 `pyproject.toml` 和 `uv.lock`，并把 prod 环境重新同步回恢复后的状态。
7. 会生成可撤销的 operation。

### `gov pin list`

用途：逐行列出当前 `dependency-groups.overrides` 中声明的 pin。

关键行为：

1. 输出的是声明态 source of truth，不是已安装版本探针。
2. 无 pin 时输出 `No pins in overrides group.`。
3. 若 `pyproject.toml` 存在但 TOML 非法，会直接返回 parse error。

### `gov pin remove <pkg>...`

用途：从 `dependency-groups.overrides` 中移除一个或多个包的 pin，重新 lock、同步 prod，并执行 smoke test。

关键行为：

1. 参数只接受包名，不接受带版本的 spec。
2. 匹配按规范化包名进行，`-` / `_` / `.` 与大小写差异会被折叠。
3. 不接受 `torch`、`torchvision`、`torchaudio`；这三者由 `gov install torch` 单独治理。
4. 删除阶段直接委托 `uv remove --group overrides --python "$py" --frozen`；缺包、解析失败和原子性都由 `uv` 原生判定。
5. 会生成可撤销的 operation。

## 2. 核心依赖升级事务

### `gov update run [--requirements-file <path>] [--timeout <seconds>]`

用途：为核心依赖升级创建独立事务，写入 `kind=core_update` 记录。

关键行为：

1. 会过滤 `torch`、`torchvision`、`torchaudio`，并继续事务；torch-family 本身不是 `update run` 的失败原因。
2. candidate 运行是有界观察：到达 timeout 后会收束进程，但事务仍应记为 `completed`，同时保留真实 `run_exit_code`。
3. candidate 的 stdout/stderr 会同时输出到终端和 `state/logs/`。

### `gov update inspect <txid>`

用途：查看核心依赖升级事务摘要，包括 requirements 来源、staged workdir、diff、冲突和日志。

### `gov update resolve <txid> [--pin <pkg==version>]... [--pins-file <path>]`

用途：用参数化 pin 修复 `update run` 或 `update promote` 的 lock 冲突。

### `gov update promote <txid> [--approve-core --reason "..."] [--allow-failed-run] [--keep-artifacts]`

用途：把核心依赖升级事务的 staged snapshot 晋升到 prod；成功后默认清理 candidate env 与 staged workdir，传 `--keep-artifacts` 可保留。

### `gov update abort <txid>`

用途：删除该核心升级事务的 candidate env 与 staged workdir，并标记为 `aborted`。

## 3. 插件事务

### `gov node add <git_url> [--ref <sha/tag>] [--id <node_id>]`

用途：克隆插件并写入 `state/plugins.json`。

### `gov node remove <node_id> [--purge-code]`

用途：移除插件注册与依赖组，并执行依赖 GC。

### `gov tx run <node_id> [--timeout <seconds>]`

用途：在 candidate 环境运行 ComfyUI 并记录插件事务。

关键行为：

1. candidate 运行是有界观察：到达 timeout 后会收束进程，但事务仍应记为 `completed`，同时保留真实 `run_exit_code`。
2. candidate 的 stdout/stderr 会同时输出到终端和 `state/logs/`。

### `gov tx inspect <txid>`

用途：查看插件事务摘要。

### `gov tx abort <txid>`

用途：删除插件事务的 candidate env 与 staged workdir，并标记为 `aborted`。

### `gov tx promote <txid> [--approve-core --reason "..."] [--allow-failed-run] [--keep-artifacts]`

用途：把插件事务晋升到 prod；成功后默认清理 candidate env 与 staged workdir，传 `--keep-artifacts` 可保留。

### `gov resolve <txid>`

用途：交互输入 `pkg==version`，修复插件事务冲突并重试 lock。

## 4. 环境导出导入

### `gov env export <output_tar>`

用途：导出当前已验证环境的单文件 `.tar` bundle，用于跨机器整体恢复。

关键行为：

1. 要求 `pyproject.toml`、`uv.lock`、`state/plugins.json`、`config.toml` 存在。
2. 使用 `uv export --format pylock.toml --locked --all-groups` 导出 `pylock.toml`；不会自动 re-lock。
3. 从 `state/plugins.json` 枚举节点，并把每个节点当前运行态源码快照导出到 bundle 的 `custom_nodes/<node_id>/`。
4. 节点快照保留工作树中的已修改文件和未跟踪文件，但会过滤 `.git/` 目录与 `.git` 指针文件，不把 Git 管理元数据带入 bundle。
5. tar 内部的 `bundle/` 至少包含：`manifest.json`、`pyproject.toml`、`uv.lock`、`pylock.toml`、`state/plugins.json`、`audit/prod-freeze.txt`、`audit/export-summary.json`。
6. `paths.comfyui_dir` 只用于定位当前源码目录，不会被当成迁移真相写回目标机。

### `gov env import <bundle_tar> --comfyui-dir <abs-path> --python <python-spec>`

用途：从单文件 `.tar` bundle 整体恢复 root truth、插件注册和 `custom_nodes` 源码，并重建 `.venv-prod`。

关键行为：

1. 只支持 `.tar` bundle；archive 内必须只有一个顶层 `bundle/` 目录。
2. 先安全解包到 staging 目录，再校验 `manifest.json` 与关键文件 SHA256，并校验 bundle 的 Python / 平台约束是否与目标机兼容，然后才进入 staging 恢复。
3. 导入后的本地依赖真相仍然是 `pyproject.toml + uv.lock`；`pylock.toml` 仅作为交付物与审计文件保留。
4. `--comfyui-dir` 和 `--python` 始终来自目标机 CLI 参数，不从 bundle 恢复；其中 `--python` 若是纯 minor 线则直接使用，否则会先通过 `uv python find --no-python-downloads` 解析目标机解释器，再规范化为 minor 线参与 lock/sync 并写回配置。
5. 导入默认执行 exact restore：覆盖目标 root truth、prod env、插件注册，并清理 bundle 外的 `custom_nodes/*` 目录，使目标机与 bundle 一致。
6. 失败会恢复 root truth，并恢复本次导入覆盖或清理过的节点目录。

## 5. 操作与运行时

### `gov op list`

用途：列出操作记录。

### `gov op inspect <op_id>`

用途：查看 `state/ops/<op_id>/meta.json`。

### `gov undo <op_id>`

用途：撤销指定成功 operation。

### `gov status`

用途：查看当前配置、prod 环境、事务总量和待处理升级事务。

### `gov run [--sync] [-- <args...>]`

用途：在 `.venv-prod` 中启动 ComfyUI。

### `gov stop`

用途：停止由 `gov run` 启动的 ComfyUI。
