# `gov` CLI 命令参考手册

<p>
  <a href="cli-reference.md"><b>English</b></a> •
  <a href="cli-reference.zh-CN.md"><b>简体中文</b></a>
</p>

`gov`（Windows 下为 `gov.exe`）是一个单二进制 CLI 工具，用于治理 ComfyUI 依赖、管理自定义节点、执行沙盒隔离的依赖验证事务，并托管生产环境运行。请在治理项目根目录（即包含 `config.toml`、`pyproject.toml` 和 `uv.lock`，或首次初始化前包含对应 `.template` 模板的目录）下执行所有命令。

---

## 核心工作流一览

```bash
# 1. 初始化治理配置、绑定 Python 解释器并创建 .venv-prod
gov init --comfyui-dir /abs/path/to/ComfyUI --python 3.12
gov install torch --index-url https://download.pytorch.org/whl/cu124
gov install

# 2. 安全接入并验证自定义节点
gov node add https://github.com/example/custom-node.git
gov tx run custom-node
gov tx inspect <TX_ID>
# （若出现依赖冲突：gov resolve <TX_ID> --pin pkg==1.2.3）
gov tx promote <TX_ID>

# 3. 启动 ComfyUI 或回滚变更
gov run -- --listen 0.0.0.0 --port 8188
gov stop
gov undo <OP_ID>
```

---

## 命令总览

| 分类 | 命令 | 用途 | 修改生产环境 |
| :--- | :--- | :--- | :---: |
| **初始化与基础依赖** | [`gov init`](#gov-init) | 初始化或重新配置项目及 `.venv-prod` | 是 |
| | [`gov install torch`](#gov-install-torch) | 配置并安装 `torch`、`torchvision` 与 `torchaudio` | 是 |
| | [`gov install`](#gov-install) | 将 ComfyUI 核心依赖导入 `dependency-groups.core` | 是 |
| **全局 Pin 与覆盖** | [`gov pin add`](#gov-pin-add) | 添加全局精确版本 pin 或强制覆盖规则 | 是 |
| | [`gov pin list`](#gov-pin-list) | 列出已声明的全局 pin 与覆盖规则 | 否 |
| | [`gov pin remove`](#gov-pin-remove) | 移除全局 pin 或强制覆盖规则 | 是 |
| **自定义节点** | [`gov node add`](#gov-node-add) | 克隆并注册自定义节点 | 是（源码与注册表） |
| | [`gov node remove`](#gov-node-remove) | 注销节点、移除其依赖组并重新同步生产环境 | 是 |
| **插件依赖事务** | [`gov tx run`](#gov-tx-run) | 在 `.venv-candidate` 中暂存、锁定并验证节点依赖 | 否 |
| | [`gov tx inspect`](#gov-tx-inspect) | 查看事务状态、依赖差异、日志与观察结果 | 否 |
| | [`gov resolve`](#gov-resolve) | 为存在冲突的插件事务提供 pin 并重新验证 | 否 |
| | [`gov tx promote`](#gov-tx-promote) | 将已验证通过的插件事务晋升至 `.venv-prod` | 是 |
| | [`gov tx abort`](#gov-tx-abort) | 清理事务的候选环境与暂存目录并标记为中止 | 否 |
| **核心依赖升级** | [`gov update run`](#gov-update-run) | 在 `.venv-candidate` 中暂存、锁定并验证核心依赖升级 | 否 |
| | [`gov update inspect`](#gov-update-inspect) | 查看核心升级事务详情、源文件哈希、差异与日志 | 否 |
| | [`gov update resolve`](#gov-update-resolve) | 为存在冲突的核心升级事务提供 pin 并重新验证 | 否 |
| | [`gov update promote`](#gov-update-promote) | 将已验证通过的核心升级事务晋升至 `.venv-prod` | 是 |
| | [`gov update abort`](#gov-update-abort) | 清理核心升级事务的候选环境与暂存目录并中止 | 否 |
| **环境迁移** | [`gov env export`](#gov-env-export) | 将已验证环境导出为可移植的 Bundle 目录 | 否 |
| | [`gov env import`](#gov-env-import) | 将环境 Bundle 目录精确还原到当前主机 | 是 |
| **操作账本与回滚** | [`gov op list`](#gov-op-list) | 列出历史生产操作记录 | 否 |
| | [`gov op inspect`](#gov-op-inspect) | 查看单条操作的哈希、备份路径与恢复状态 | 否 |
| | [`gov op recover`](#gov-op-recover) | 恢复中断的生产操作或补全账本关联记录 | 是 |
| | [`gov undo`](#gov-undo) | 还原操作前备份并重新同步 `.venv-prod` | 是 |
| **运行时与状态** | [`gov run`](#gov-run) | 在 `.venv-prod` 中启动 ComfyUI | 可选（`--sync`） |
| | [`gov stop`](#gov-stop) | 停止由 `gov run` 启动的 ComfyUI 进程树 | 否 |
| | [`gov status`](#gov-status) | 查看项目就绪状态、运行状态与近期事务摘要 | 否 |
| | [`gov help`](#gov-help) | 打印 CLI 帮助或子命令用法 | 否 |

---

## 通用约定与规则

### 全局选项

- `-h`、`--help` — 打印 `gov` 或任意子命令的用法说明。
- `-V`、`--version` — 打印 `gov` 版本号。

### 退出码

- `0`：命令执行成功（或显示了 `--help` / `--version` / `help`）。
- `1`：运行时或外部工具错误（`uv`、`git` 失败，lock/sync/冒烟测试失败，或回滚失败）。
- `2`：CLI 参数错误、前置条件不满足或策略拦截（例如缺少配置、触发核心包审批门禁、ComfyUI 仍在运行，或存在待恢复操作）。
- **`gov run`**：成功拉起 ComfyUI 后透传其自身退出码（启动前检查失败时返回 `1` 或 `2`）。

### 依赖的外部工具与环境

- **`uv`**：必须位于 `PATH` 中，用于查找 Python 解释器、锁定、同步、导出环境列表及生成 `pylock.toml`。
- **`git`**：必须位于 `PATH` 中，供 `gov node add` 使用。
- **本地 wheel 源**：可通过环境变量 `UV_FIND_LINKS` 指定以逗号分隔的本地目录或 URL（相对路径按执行 `gov` 时的工作目录解析）。由于暂存验证与环境导出会重定位 `pyproject.toml` 和 `uv.lock`，使用 `tool.uv.workspace` 或不可重定位本地路径 / `file:` URL 依赖源的项目将被拒绝。

### 并发控制、运行门禁与恢复阻断

- **生产锁**：修改生产状态、执行恢复、中止事务、导出环境包以及启停 ComfyUI 的命令都会获取 `state/comfyui.pid.lock`；若已有另一个 `gov` 进程持有该锁，命令会立即失败退出。
- **先停机再变更**：只要由 `gov` 托管的 ComfyUI 进程仍在运行，所有修改生产环境的命令都会被拒绝。请先执行 `gov stop`。
- **中断恢复门禁**：若此前有生产操作意外中断并处于待恢复状态，所有后续生产写入命令和 `gov run` 都会被阻断，直到执行 `gov op recover <OP_ID>` 完成恢复。

### 包名规范化、版本格式与 Torch 保护

- **名称规范化**：包名和节点依赖组名均按以下规则规范化——转为小写，并将连续的 `-`、`_`、`.` 折叠为单个 `-`（例如 `Foo_Bar.Baz` 规范化为 `foo-bar-baz`）。
- **精确版本格式**：所有 `--pin` 及 pin 参数仅接受 `<pkg>==<version>` 格式，不允许包含空白字符、版本范围（如 `>=`）、extras 或环境 marker。
- **Torch 家族专属治理**：`torch`、`torchvision` 与 `torchaudio` 仅由 `gov install torch` 统一管理。`gov pin`、`gov resolve` 和 `gov update resolve` 会拒绝这三个包；`gov install` 与 `gov update run` 会自动从 `requirements.txt` 中过滤它们。
- **`requirements.txt` 语法限制**：空行和 `#` 注释会被忽略；以 `-` 开头的指令（如 `-r`、`--index-url`）、以 `.` 开头或包含 `\` 的相对本地路径以及非法包名会被拒绝。

### 核心包审批门禁（`policy.core_packages`）

若事务的净依赖变更涉及 `policy.core_packages`（默认包含 `torch`、`torchvision`、`torchaudio`、`xformers`、`triton`、`onnxruntime`、`onnxruntime-gpu`、`numpy`）中的任意包，在执行 `gov tx promote` 或 `gov update promote` 时必须显式传入 `--approve-core`。

### 事务生命周期与 ID 格式

- **`<TX_ID>` 格式**：`<UTC_TIMESTAMP>-<8_HEX>`（例如 `20260419T120000Z-a1b2c3d4`）。
- **`<OP_ID>` 格式**：`<UTC_TIMESTAMP>-op-<8_HEX>`（例如 `20260419T120000Z-op-e5f6a7b8`）。
- **事务状态**：
  - `running` — 锁定或候选环境观察正在执行中（或执行过程中意外中断）。
  - `needs_resolution` — `uv lock` 因依赖冲突失败；冲突报告保存在 `state/conflicts/<TX_ID>.json`。
  - `completed` — 首次 `tx run` 或 `update run` 通过了候选环境同步和启动探针检查，可以晋升。
  - `resolved` — 经 `resolve` 或 `update resolve` 解决冲突后锁定成功并通过候选观察，可以晋升。
  - `failed` — 发生锁底层错误、候选环境同步失败、超时、中断、探针异常、节点加载失败或运行时包漂移。
  - `promoted` — 已晋升并应用到 `.venv-prod`。
  - `promote_failed` — 晋升失败并已回滚。
  - `aborted` — 已通过 `tx abort` 或 `update abort` 丢弃。

### 相关 `config.toml` 配置项

多数命令行默认值来自 `config.toml`（首次执行 `gov init` 时基于 `config.toml.template` 生成）：

| 配置项 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `paths.comfyui_dir` | — | ComfyUI 安装目录的绝对路径 |
| `runtime.python` | — | Python 选择器（如 `3.12`）或解析后的解释器路径 |
| `runtime.prod_env` | `.venv-prod` | 生产虚拟环境目录（相对于项目根目录） |
| `runtime.candidate_root` | `.venv-candidate` | 各事务候选虚拟环境的父目录 |
| `tx.timeout_seconds` | `120` | 候选启动探针与冒烟测试的默认超时时间（秒） |
| `tx.smoke_test` | Python 版本检查 | 生产同步后执行的 `{ program = "...", args = [...] }` 表；将 `program` 设为 `"python"` 可直接调用 `.venv-prod` 的 Python（`tx.smoke_test_cmd` 已废弃且会被忽略） |
| `policy.core_packages` | Torch 系列、xformers、triton、onnxruntime、numpy | 受 `--approve-core` 门禁保护的核心包列表 |
| `ops.retention_count` | `100` | 历史裁剪时保留的已完成操作最大数量 |
| `run.extra_args` | `[]` | `gov run` 传给 ComfyUI 的额外启动参数（字符串数组或旧版空格分隔字符串） |
| `run.sync_before_run` | `false` | 是否在每次启动 ComfyUI 前强制同步 `.venv-prod` |

---

## 初始化与核心依赖

### `gov init`

初始化或重新配置治理项目、`pyproject.toml`、`uv.lock`、`state/` 目录及 `.venv-prod`。

```text
gov init [--comfyui-dir <COMFYUI_DIR>] [--python <PYTHON>]
```

| 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `--comfyui-dir <COMFYUI_DIR>` | 首次初始化必填 | ComfyUI 安装目录的绝对路径 |
| `--python <PYTHON>` | 首次初始化必填 | Python `<major>.<minor>` 版本号（如 `3.12`）、patch 版本、可执行文件名或解释器路径 |

**行为说明：**
- **首次初始化**：必须同时提供 `--comfyui-dir` 与 `--python`。若 `.venv-prod` 已存在但缺少完整的项目真值文件（`config.toml`、`pyproject.toml`、`uv.lock`），命令将拒绝覆盖。
- **重新初始化**：当 `config.toml`、`pyproject.toml` 与 `uv.lock` 均已存在时，未传入的选项沿用当前配置。
- **Python 与平台绑定**：纯 `<major>.<minor>` 格式会直接使用；传入 patch 版本、可执行文件名或路径时，会调用 `uv python find --no-python-downloads` 查找本机解释器并规范化为 `<major>.<minor>`。同时在 `pyproject.toml` 中将 `requires-python` 设为 `==<major>.<minor>.*`，并将 `[tool.uv].environments` 收敛为当前主机的操作系统与 CPU 架构 marker。
- **安全与回滚**：先在暂存目录完成锁定，再写入项目根目录文件。首次初始化若同步失败，会还原文件并删除新建的虚拟环境；重新初始化若同步失败，会还原原有配置、真值文件和注册表，并使用旧 Python 重新同步 `.venv-prod`。不支持通过 `gov undo` 撤销。

**示例：**

```bash
gov init --comfyui-dir /opt/ComfyUI --python 3.12
gov init --python 3.12.8
```

---

### `gov install torch`

配置 PyTorch 索引源，并将 `torch`、`torchvision` 与 `torchaudio` 安装到 `dependency-groups.torch`。

```text
gov install torch --index-url <INDEX_URL> [--torch <TORCH>] [--torchvision <TORCHVISION>] [--torchaudio <TORCHAUDIO>]
```

| 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `--index-url <INDEX_URL>` | 是 | PyTorch wheel 索引 URL（不能包含空白字符） |
| `--torch <TORCH>` | 否 | `torch` 的精确版本（如 `torch==2.5.1`） |
| `--torchvision <TORCHVISION>` | 否 | `torchvision` 的精确版本（如 `torchvision==0.20.1`） |
| `--torchaudio <TORCHAUDIO>` | 否 | `torchaudio` 的精确版本（如 `torchaudio==2.5.1`） |

**行为说明：**
- 要求已初始化 `config.toml`。
- 在 `[tool.uv.index]` 中注册显式索引条目，并通过 `[tool.uv.sources]` 将 `torch`、`torchvision` 与 `torchaudio` 全部绑定到该索引。未指定版本的包将按无版本约束（如 `torch`）写入。
- 在暂存目录锁定后更新根目录真值、同步 `.venv-prod`，并执行 Torch 冒烟测试（`import torch, torchvision, torchaudio`）。
- 若同步或冒烟测试失败，自动回滚根目录真值并重新同步 `.venv-prod`。会生成可撤销的操作记录。

**示例：**

```bash
gov install torch --index-url https://download.pytorch.org/whl/cu124 --torch torch==2.5.1
```

---

### `gov install`

将 ComfyUI 核心依赖导入 `dependency-groups.core` 并同步 `.venv-prod`。

```text
gov install [--requirements-file <REQUIREMENTS_FILE>]
```

| 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `--requirements-file <REQUIREMENTS_FILE>` | 否 | `requirements.txt` 文件路径（默认为 `<comfyui_dir>/requirements.txt`） |

**行为说明：**
- 要求已初始化 `config.toml`，且 `dependency-groups.torch` 中已包含 `torch`、`torchvision`、`torchaudio`（请先执行 `gov install torch`）。
- 自动过滤 `torch`、`torchvision` 与 `torchaudio`，避免索引源冲突。
- 替换 `dependency-groups.core` 后在暂存目录锁定、提交根目录真值、同步 `.venv-prod` 并执行冒烟测试；失败时自动回滚并重新同步。会生成可撤销的操作记录。

**示例：**

```bash
gov install
gov install --requirements-file /opt/ComfyUI/requirements.txt
```

---

## 全局 Pin 与强制覆盖

### `gov pin add`

全局锁定一个或多个包的精确版本，随后重新锁定、同步 `.venv-prod` 并执行冒烟测试。

```text
gov pin add <SPECS>... [--override-requirements]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<SPECS>...` | 是 | 一个或多个 `<pkg>==<version>` 格式的精确版本声明 |
| `--override-requirements` | 否 | 同时写入 `tool.uv.override-dependencies`，强制覆盖上游冲突的版本约束 |

**行为说明：**
- 拒绝范围约束（如 `>=`）及 Torch 家族包；同一命令中若对同一包传入多个版本，按规范化包名以最后一个为准。
- 当锁定的包不属于常用推荐集合（`numpy`、`transformers`）时，会输出警告提示。
- **普通模式（默认）**：写入 `dependency-groups.overrides`，作为普通依赖约束参与求解；若其他依赖禁止该版本，`uv lock` 会报错失败。若目标包已存在生效中的强制覆盖且版本不同，必须传入 `--override-requirements` 才能修改。
- **强制覆盖模式（`--override-requirements`）**：同时写入 `dependency-groups.overrides` 与 `tool.uv.override-dependencies`，使 `uv` 忽略上游包对该包的冲突版本限制，同时确保该包被安装。
- 在暂存目录锁定成功后才写入生产真值；同步或冒烟测试失败时自动回滚。会生成可撤销的操作记录。

**示例：**

```bash
gov pin add numpy==1.26.4 transformers==4.44.0
gov pin add numpy==1.26.4 --override-requirements
```

---

### `gov pin list`

列出 `pyproject.toml` 中声明的全局 pin 与 `uv` 覆盖条目。

```text
gov pin list [--details]
```

| 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `--details` | 否 | 在每行末尾附加制表符分隔的类型标签 |

**行为说明：**
- 读取 `dependency-groups.overrides` 与 `tool.uv.override-dependencies` 中的声明态配置（而非探测 `.venv-prod` 实际已安装版本）。
- 两者皆空时输出 `No pins in overrides group.`。
- 若两个位置存在完全相同的精确 pin，只输出一次。传入 `--details` 时会标注以下类型：
  - `constraint` — 仅位于 `dependency-groups.overrides`
  - `forced` — 同时且版本一致地存在于 `dependency-groups.overrides` 与 `tool.uv.override-dependencies`
  - `forced only` — 仅存在于 `tool.uv.override-dependencies` 的精确版本
  - `inconsistent` / `inconsistent override` — 同一个包在两处存在不同版本
  - `manual override` — 位于 `tool.uv.override-dependencies` 中的带作用域、带 marker 或非精确版本的手写条目

---

### `gov pin remove`

按包名移除一个或多个全局 pin，随后重新锁定、同步 `.venv-prod` 并执行冒烟测试。

```text
gov pin remove <SPECS>... [--override-requirements]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<SPECS>...` | 是 | 一个或多个包名（不要附加 `==version`） |
| `--override-requirements` | 否 | 允许移除 `tool.uv.override-dependencies` 中孤立或版本不一致的强制覆盖条目 |

**行为说明：**
- 按规范化包名匹配并拒绝 Torch 家族包。若指定的任一包在两处均不存在，整个命令直接报错退出，不做任何修改。
- 成对管理的精确 pin（在 `dependency-groups.overrides` 与 `tool.uv.override-dependencies` 中版本完全匹配）会被一并移除。
- 移除孤立或版本不一致的 `tool.uv.override-dependencies` 条目需显式传入 `--override-requirements`。手写的带 marker、作用域或非精确覆盖条目不会被自动删除。
- 在暂存目录锁定后再提交生产；同步或冒烟测试失败时自动回滚。会生成可撤销的操作记录。

**示例：**

```bash
gov pin remove numpy
gov pin remove numpy --override-requirements
```

---

## 自定义节点管理

### `gov node add`

将自定义节点仓库克隆至 `<comfyui_dir>/custom_nodes/<node_id>` 并注册到 `state/plugins.json`。

```text
gov node add <GIT_URL> [--ref <REF_NAME>] [--id <ID>]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<GIT_URL>` | 是 | Git 仓库 URL |
| `--ref <REF_NAME>` | 否 | 要检出的 Git 分支、标签或 commit SHA（默认为仓库默认 `HEAD`） |
| `--id <ID>` | 否 | 自定义节点 ID（默认为去掉 `.git` 后缀的仓库名） |

**行为说明：**
- 节点 ID 仅允许 ASCII 字母、数字、`.`、`_` 与 `-`（不能为 `.` 或 `..`）。
- 若节点 ID 已注册、目标目录 `custom_nodes/<node_id>` 已存在、`pyproject.toml` 中已存在无归属的同名 `node-*` 组，或发生规范化组名冲突（例如已有 `foo-bar` 占用 `node-foo-bar` 时再添加 `foo_bar`），都会在克隆前拒绝执行。
- 先克隆到 `custom_nodes` 扫描目录之外的临时目录，按需检出 `--ref`，再移入 `custom_nodes/<node_id>` 并更新 `state/plugins.json`。
- **此命令不会安装节点依赖**，请紧接着执行 `gov tx run <node_id>`。不支持通过 `gov undo` 撤销（请使用 `gov node remove --purge-code`）。

**示例：**

```bash
gov node add https://github.com/ltdrdata/ComfyUI-Manager.git --id comfyui-manager
```

---

### `gov node remove`

注销自定义节点、从 `pyproject.toml` 移除其依赖组、重新锁定并同步 `.venv-prod`。

```text
gov node remove <NODE_ID> [--purge-code]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<NODE_ID>` | 是 | 已注册的节点 ID |
| `--purge-code` | 否 | 同时删除 `<comfyui_dir>/custom_nodes/<NODE_ID>` 目录 |

**行为说明：**
- 从 `pyproject.toml` 移除 `dependency-groups.node-<normalized_id>`，在暂存目录重新锁定后更新根目录真值、同步 `.venv-prod`、运行冒烟测试，并从 `state/plugins.json` 中删除该节点记录。
- 不会删除 `dependency-groups.overrides` 中的全局 pin。
- 若该依赖组被多个已注册节点共享，命令会拒绝执行。
- 传入 `--purge-code` 时，会要求 `custom_nodes/<NODE_ID>` 目录必须存在，并在删除前先备份源码；若同步或冒烟测试失败会自动恢复。未传 `--purge-code` 时源码目录保留在磁盘上。会生成可撤销的操作记录。

**示例：**

```bash
gov node remove comfyui-manager --purge-code
```

---

## 插件依赖事务

### `gov tx run`

暂存已注册节点的 `requirements.txt`，求解依赖并在 `.venv-candidate/<TX_ID>` 沙盒中观察 ComfyUI 启动情况。

```text
gov tx run <NODE_ID> [--timeout <SECONDS>]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<NODE_ID>` | 是 | 已注册且在 `custom_nodes/<NODE_ID>` 下存在源码的节点 ID |
| `--timeout <SECONDS>` | 否 | 观察超时上限（秒；默认读取 `config.toml` 的 `tx.timeout_seconds`，即 `120`） |

**行为说明：**
- 若该节点已存在处于 `running` 状态的事务，命令会拒绝执行。
- 读取 `<comfyui_dir>/custom_nodes/<NODE_ID>/requirements.txt`（若不存在该文件，则视为空依赖列表），在生产真值的暂存副本中写入 `dependency-groups.<node_group>` 并执行 `uv lock`。
- **发生锁冲突时**：将冲突详情写入 `state/conflicts/<TX_ID>.json`，事务状态记为 `needs_resolution` 并输出 `<TX_ID>`，随后可通过 `gov resolve <TX_ID>` 解决。
- **锁定成功时**：创建 `.venv-candidate/<TX_ID>` 并同步依赖，随后以 `--quick-test-for-ci` 模式启动 ComfyUI，stdout/stderr 实时输出到终端并写入 `state/logs/`。
- **观察判定规则**：
  - `completed`：候选环境同步成功，ComfyUI 正常执行至原生 `--quick-test-for-ci` 退出分支且退出码为 `0`，目标节点报告加载成功，且运行期间候选环境中的已安装包未发生意外变更。
  - `failed`：候选同步失败、超时、被中断、非零退出、ComfyUI 不支持或未完成 `--quick-test-for-ci` 探针、目标节点加载失败，或发生了运行时包漂移（例如节点启动代码私自执行了 `pip install`）。

**示例：**

```bash
gov tx run comfyui-manager --timeout 180
```

---

### `gov tx inspect`

查看插件事务的详细状态、依赖差异、日志路径与观察证据。

```text
gov tx inspect <TX_ID>
```

| 参数 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<TX_ID>` | 是 | 插件事务 ID |

**行为说明：**
- 输出事务元信息、起止时间、包差异（`Added` / `Removed`）、`Core impact`、退出码、超时标记、候选环境与暂存目录路径（已清理或缺失时会标注 `(cleaned)` 或 `(missing)`）、日志路径、观察结论、节点加载失败列表、候选包漂移情况、冲突摘要与提示（处于 `needs_resolution` 时），以及晋升和撤销关联状态。

---

### `gov resolve`

为处于 `needs_resolution` 状态的插件事务提供 pin 以解决依赖冲突，并重新锁定与观察。

```text
gov resolve <TX_ID> [--pin <PINS>]... [--pins-file <PINS_FILE>] [--override-requirements]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<TX_ID>` | 是 | 处于 `needs_resolution` 状态的插件事务 ID |
| `--pin <PINS>` | 否 | `<pkg>==<version>` 格式的精确 pin（可重复传入） |
| `--pins-file <PINS_FILE>` | 否 | 包含 pin 列表的文件，每行一个 `<pkg>==<version>`（自动忽略空行与 `#` 注释） |
| `--override-requirements` | 否 | 同时将本次传入的 pin 写入 `tool.uv.override-dependencies`，强制覆盖冲突的上游约束 |

**行为说明：**
- 若同时省略 `--pin` 与 `--pins-file`，会先打印冲突摘要，再通过标准输入交互式读取每行一个 `<pkg>==<version>`（输入空行或 EOF 结束）。
- 按规范化包名合并 pin（后传入的值覆盖前者），拒绝 Torch 家族包。
- 基于**当前最新**生产真值重新暂存，叠加该节点记录的依赖与本次 pin（写入 `dependency-groups.overrides`；若指定 `--override-requirements` 则同时写入 `tool.uv.override-dependencies`），然后重新执行 `uv lock`。
- 若依然冲突，状态保持 `needs_resolution` 并更新冲突报告；若锁定成功，则重新同步候选环境并运行启动观察，观察通过后状态转为 `resolved`。在执行 `gov tx promote` 之前，生产环境保持不变。

**示例：**

```bash
gov resolve 20260419T120000Z-a1b2c3d4 --pin numpy==1.26.4 --pin pillow==10.4.0
gov resolve 20260419T120000Z-a1b2c3d4 --pins-file pins.txt --override-requirements
```

---

### `gov tx promote`

将已观察通过的插件事务锁定快照晋升到 `.venv-prod` 与根目录真值文件。

```text
gov tx promote <TX_ID> [--approve-core] [--reason <REASON>] [--allow-failed-run] [--keep-artifacts]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<TX_ID>` | 是 | 已完成观察的插件事务 ID（`completed`、`resolved`，或配合 `--allow-failed-run` 的符合条件 `failed` 事务） |
| `--approve-core` | 条件必填 | 当事务变更了 `policy.core_packages` 中的任意包时必须传入 |
| `--reason <REASON>` | 否 | 随晋升记录保存的可选说明文字 |
| `--allow-failed-run` | 否 | 仅允许晋升候选同步成功且无包漂移、但运行时返回了真实非零退出码的 `failed` 事务 |
| `--keep-artifacts` | 否 | 晋升成功后保留 `.venv-candidate/<TX_ID>` 与暂存目录（默认自动清理） |

**行为说明：**
- 晋升前严格校验当前根目录 `pyproject.toml`/`uv.lock`、暂存真值、`.venv-prod` 实际包集、解析后的 Python 解释器及节点依赖组是否仍与观察快照一致；若期间生产环境发生漂移，命令会拒绝并要求重新执行 `gov tx run` 或 `gov resolve`。
- `--allow-failed-run` 不能绕过同步失败、超时、中断、探针失败、节点加载失败或候选包漂移。
- 备份当前生产真值，直接应用已观察的锁定快照（不再二次求解），同步 `.venv-prod`、运行冒烟测试、校验最终解释器与实际包集、更新 `state/plugins.json`，并按需清理候选与暂存目录。
- 任意一步失败都会自动回滚。会生成可撤销的操作记录。

**示例：**

```bash
gov tx promote 20260419T120000Z-a1b2c3d4
gov tx promote 20260419T120000Z-a1b2c3d4 --approve-core --reason "允许升级 numpy"
```

---

### `gov tx abort`

清理插件事务的候选虚拟环境与暂存目录，并将其状态标记为 `aborted`。

```text
gov tx abort <TX_ID> [--confirm-stopped]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<TX_ID>` | 是 | 插件事务 ID |
| `--confirm-stopped` | 条件必填 | 当早期遗留的 `running` 记录缺少进程身份信息时，在确认进程已退出后需传入此选项 |

**行为说明：**
- 允许从 `running`、`completed`、`failed`、`needs_resolution`、`resolved`、`promote_failed` 状态下中止（对 `aborted` 状态幂等）。已 `promoted` 或存在待恢复晋升操作（需先 `gov op recover`）的事务会被拒绝。
- 对 `running` 状态事务会校验所有者与候选进程树已退出（`--confirm-stopped` 不能覆盖仍在运行中的进程证据）。
- 删除 `.venv-candidate/<TX_ID>` 与暂存目录，但保留事务 JSON 记录与日志文件。

---

## 核心依赖升级

核心依赖升级事务与插件事务共用相同的暂存、观察、冲突解决、审批与回滚机制，操作对象为 `dependency-groups.core`。

### `gov update run`

在 `.venv-candidate/<TX_ID>` 中暂存并验证 ComfyUI 核心依赖升级。

```text
gov update run [--requirements-file <REQUIREMENTS_FILE>] [--timeout <SECONDS>]
```

| 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `--requirements-file <REQUIREMENTS_FILE>` | 否 | `requirements.txt` 文件路径（默认为 `<comfyui_dir>/requirements.txt`） |
| `--timeout <SECONDS>` | 否 | 观察超时上限（秒；默认读取 `config.toml` 的 `tx.timeout_seconds`，即 `120`） |

**行为说明：**
- 要求已初始化 `config.toml`、存在 `<comfyui_dir>/main.py` 且已配置 `dependency-groups.torch`。
- 自动过滤 `torch`、`torchvision` 与 `torchaudio`，在暂存目录替换 `dependency-groups.core`、锁定并执行候选环境 `--quick-test-for-ci` 启动探针。
- 启动探针期间只要有任意自定义节点报告加载失败，即判定观察失败（`failed`）；锁冲突则会记为 `needs_resolution`。

---

### `gov update inspect`

查看 `core_update` 核心升级事务的详细信息。

```text
gov update inspect <TX_ID>
```

输出结构与 [`gov tx inspect`](#gov-tx-inspect) 相同，并额外显示 `Source requirements` 路径与 `Source SHA256`。

---

### `gov update resolve`

使用命令行或文件中的 pin 解决处于 `needs_resolution` 的核心升级事务冲突，并重新进行候选观察。

```text
gov update resolve <TX_ID> [--pin <PINS>]... [--pins-file <PINS_FILE>] [--override-requirements]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<TX_ID>` | 是 | 处于 `needs_resolution` 状态的核心升级事务 ID |
| `--pin <PINS>` | 否 | `<pkg>==<version>` 格式的精确 pin（可重复传入） |
| `--pins-file <PINS_FILE>` | 否 | 每行一个 `<pkg>==<version>` 的 pin 文件 |
| `--override-requirements` | 否 | 同时写入 `tool.uv.override-dependencies`，强制覆盖上游冲突约束 |

**行为说明：**
- 非交互式：必须至少通过 `--pin` 或 `--pins-file` 提供一个 pin。
- 默认情况下 `--pin` 仅向 `dependency-groups.overrides` 添加普通约束（不再像旧版那样默认强制覆盖；如需强制覆盖请传入 `--override-requirements`）。
- 基于当前生产真值重新暂存、锁定并重新观察候选环境，通过后转为 `resolved`。

**示例：**

```bash
gov update resolve 20260419T120000Z-a1b2c3d4 --pin numpy==1.26.4 --override-requirements
```

---

### `gov update promote`

将已观察通过的 `core_update` 事务晋升至 `.venv-prod`。

```text
gov update promote <TX_ID> [--approve-core] [--reason <REASON>] [--allow-failed-run] [--keep-artifacts]
```

与 [`gov tx promote`](#gov-tx-promote) 遵循完全一致的快照校验、基线检查、`--approve-core`、`--allow-failed-run`、生产同步、冒烟测试、自动回滚与产物清理规则。

---

### `gov update abort`

清理核心升级事务的候选虚拟环境与暂存目录，并标记为 `aborted`。

```text
gov update abort <TX_ID> [--confirm-stopped]
```

与 [`gov tx abort`](#gov-tx-abort) 遵循一致的进程校验与清理规则。

---

## 环境导出与导入

### `gov env export`

将当前已锁定的依赖配置、插件注册表以及各自定义节点源码快照导出到一个独立的 Bundle 目录。

```text
gov env export <OUTPUT_DIR>
```

| 参数 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<OUTPUT_DIR>` | 是 | 目标导出目录路径（不能已存在，且不能位于任意节点源码目录内部） |

**行为说明：**
- 要求 `config.toml`（含绝对路径 `comfyui_dir` 与 `runtime.python`）、`pyproject.toml`、`uv.lock`、`state/plugins.json` 以及所有已注册节点的 `custom_nodes/<node_id>` 目录均存在。
- 调用 `uv export --format pylock.toml --locked --all-groups`（若 `uv.lock` 已过期或当前 `uv` 版本不支持导出 `pylock.toml`，命令直接失败且不会自动重锁）。
- 在 `<OUTPUT_DIR>` 中生成以下内容：
  - `pyproject.toml`、`uv.lock`、`pylock.toml`、`state/plugins.json`
  - `custom_nodes/<node_id>/` — 每个已注册节点当前工作树的运行态快照（保留未提交修改与未跟踪文件，自动排除 `.git`，并拒绝目录内的软链接/联接点）
  - `audit/prod-freeze.txt`、`audit/export-summary.json`
  - `manifest.json` — 格式版本 `1` 的清单，记录 Python、操作系统、架构、`node_ids` 及所有文件 SHA-256 校验和
- 若导出过程出错，会自动清理已写入的 `<OUTPUT_DIR>`。

**示例：**

```bash
gov env export /tmp/comfy-bundle
```

---

### `gov env import`

从导出的 Bundle 目录精确还原环境依赖与托管的 `custom_nodes` 到当前机器。

```text
gov env import <BUNDLE_DIR> --comfyui-dir <COMFYUI_DIR> --python <PYTHON>
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<BUNDLE_DIR>` | 是 | 已导出的 Bundle 目录路径 |
| `--comfyui-dir <COMFYUI_DIR>` | 是 | 目标主机 ComfyUI 目录的绝对路径 |
| `--python <PYTHON>` | 是 | 目标主机 Python 选择器（通过 `uv python find --no-python-downloads` 解析） |

**行为说明：**
- **写入前校验**：验证 `manifest.json`（版本 `1`）、全部文件 SHA-256 校验和、目标主机操作系统与 CPU 架构是否完全匹配（`sys_platform`、`platform_system`、`platform_machine`）、所有 `node-*` 依赖组的归属唯一性、路径无重叠，并运行 `uv lock --check` 与针对目标解释器的 `uv sync --dry-run` 兼容性检查。若项目目录下已存在 `.venv-prod` 但缺少 `pyproject.toml` 和 `uv.lock`，为确保可回滚会拒绝导入。
- **精确还原**：替换根目录 `pyproject.toml`、`uv.lock`、`state/plugins.json`，将 `--comfyui-dir` 与解析后的 Python 解释器路径写入 `config.toml`，同步 `.venv-prod`，替换各 `custom_nodes/<node_id>` 目录并执行冒烟测试。
- **`custom_nodes/` 既有条目处理**：
  - 未出现在 Bundle 中的普通节点目录会被**清理删除**（导入期间会先备份以支持失败回滚）。
  - 顶层普通文件、符号链接（symlink）与目录联接点（junction）会被**保留**。若 Bundle 中的节点与顶层普通文件同名，导入会在写入前报错中止；若与符号链接或联接点同名，则保留该链接并输出警告，跳过安装该节点的 Bundle 源码。
- 导入失败时自动回滚真值文件、`.venv-prod` 与已移动的节点目录。**不支持通过 `gov undo` 撤销**（仅在写入中途崩溃中断时使用 `gov op recover`）。

**示例：**

```bash
gov env import /tmp/comfy-bundle --comfyui-dir /opt/ComfyUI --python 3.12
```

---

## 操作账本、恢复与撤销

### `gov op list`

按时间倒序列出所有生产操作记录。

```text
gov op list
```

以制表符分隔输出 `op_id`、`kind`、`status`、`reference`、`started_at`，待恢复的操作会在末尾标注 `needs_recovery`。遇到损坏的历史记录时输出警告并继续列出其余记录。

---

### `gov op inspect`

查看指定操作的详细元数据、文件哈希、备份路径及恢复/撤销状态。

```text
gov op inspect <OP_ID>
```

| 参数 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<OP_ID>` | 是 | 操作 ID |

---

### `gov op recover`

恢复意外中断的生产操作，或补记已提交但缺失的账本关联状态。

```text
gov op recover <OP_ID>
```

| 参数 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<OP_ID>` | 是 | 被标记为需要恢复的操作 ID |

**行为说明：**
- 要求 ComfyUI 已停止（`gov stop`），并确认原操作进程及外部子进程均已退出。
- **提交前中断**（`prepared` / `applying` / 回滚失败）：从已校验的备份恢复真值、配置、注册表及受影响的节点源码操作，并重新同步（或删除本次新建的）`.venv-prod`。
- **已提交但缺少关联引用**：直接将关联事务标记为 `promoted`（或将原操作标记为 `undone`），不会重复执行 `uv sync`。
- 对已完成恢复的操作可安全重复执行。若早期 `running` 记录缺少阶段元数据，会保留其备份并要求人工核查。

---

### `gov undo`

还原一条成功且可撤销的操作前备份，并重新同步 `.venv-prod`。

```text
gov undo <OP_ID>
```

| 参数 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `<OP_ID>` | 是 | 状态为 `success` 且 `undoable: true` 的操作 ID |

**行为说明：**
- **漂移校验**：将当前 `pyproject.toml`、`uv.lock` 与 `state/plugins.json` 的 SHA-256 哈希同该操作记录的 `post_sha256` 比对；若之后发生过其他变更，会拒绝撤销。
- 恢复操作前的真值文件、注册表及（若该操作修改或删除过源码）节点源码，重新同步 `.venv-prod`，并将原操作标记为 `undone`。
- **不可通过 `gov undo` 撤销的操作类型**：`init`、`reinit`、`node add`、`env import` 与 `run --sync`（`runtime_sync`）。

---

## 运行时托管与状态检查

### `gov run`

在 `.venv-prod` 中启动 ComfyUI（`<comfyui_dir>/main.py`）并透传其退出码。

```text
gov run [--sync] [-- <ARGS>...]
```

| 参数 / 选项 | 是否必填 | 说明 |
| :--- | :---: | :--- |
| `--sync` | 否 | 启动 ComfyUI 前对 `.venv-prod` 执行一次锁定同步 |
| `<ARGS>...` | 否 | 在 `--` 之后原样透传给 ComfyUI 的启动参数 |

**行为说明：**
- 要求存在 `config.toml`、`<comfyui_dir>/main.py` 与 `.venv-prod`。若已有 `gov` 启动的 ComfyUI 进程在运行，或存在待 `gov op recover` 的操作，会拒绝启动。
- 当传入 `--sync` 或 `config.toml` 中 `run.sync_before_run = true` 时，先按 `pyproject.toml` + `uv.lock` 同步 `.venv-prod`（该同步会记入 `state/ops/`，不可撤销）。
- 会将 `config.toml` 中的 `run.extra_args` 置于 `--` 后的透传参数之前。
- 将进程身份写入 `state/comfyui.pid`，终端实时输出日志，进程退出后清理 PID 文件并透传 ComfyUI 的退出码。

**示例：**

```bash
gov run --sync -- --listen 0.0.0.0 --port 8188
```

---

### `gov stop`

停止由 `gov run` 启动并记录在 `state/comfyui.pid` 中的 ComfyUI 进程树。

```text
gov stop
```

**行为说明：**
- 发送优雅停止信号（Windows 下为 `Ctrl+Break`，Linux 下为 `SIGTERM`），最多等待 30 秒；若仍未退出则强制终止整个受管进程树/进程组，并移除 `state/comfyui.pid`。
- 若 PID 文件不存在或记录的进程早已退出，将静默清理残留文件并成功返回。
- 若遇到无创建时间等身份信息的旧版纯数字 PID 文件且对应进程仍在运行，为避免误杀无关进程，命令会拒绝发送信号，需人工核验后手动停止。

---

### `gov status`

只读输出当前治理项目的配置就绪度、环境状态、外部工具、插件统计、事务汇总与运行状态。

```text
gov status
```

输出字段包括：`project_root`、`config_ready`、`operations_needing_recovery`（仅存在时输出）、`comfyui_dir`、`python`、`prod_env_exists`、`torch_ready`、`core_ready`、`lock_exists`、`plugins_registered`、`plugins_enabled`、`transactions_total`、`transactions_pending`、`update_transactions_pending`、`transactions_recent`、`runtime_running`、`tool_uv`、`tool_git`、`tool_python`、`transaction_counts`、`recent_transactions`（最多 8 条）与 `notes`。

---

### `gov help`

打印 `gov` 或指定子命令的帮助信息。

```text
gov help [COMMAND]...
```

**示例：**

```bash
gov help
gov help tx promote
```
