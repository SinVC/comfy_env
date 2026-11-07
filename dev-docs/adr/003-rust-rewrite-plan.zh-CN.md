# ADR-003: Rust 重写方案

## Metadata (元数据)

- Status: Accepted
- Date: 2026-04-19
- Supersedes: 原始 ADR-003 草稿，以及此前所有以 Bash 为中心的迁移策略

## Premise Constraints (前提约束)

以下前提不可协商。本文档中的每一项设计决策都必须同时满足它们：

1. **`gov`（Rust 二进制）是唯一入口点。** 不允许存在 Bash 包装器、shell 分发器或 shim。
2. **产品运行时彻底消除 Bash。** `bin/gov` 仅保留为开发参考制品，绝不向用户分发，也不会由用户执行。
3. **Windows 是一等目标平台**，与 Linux 地位相同。用户使用 `gov` 时不应需要 Bash、coreutils 或任何 Unix 专属工具。
4. **这次重写是对 Bash 的完全替换**，而不是“给 Bash 增加一个 Rust 后端”。
5. **在迁移期间，尚未实现的命令必须返回显式错误**（`error: command '<name>' is not yet implemented`），绝不能回退到 Bash。

## Context (背景)

`comfy_env` 是一个用于 ComfyUI 依赖管理的 sidecar 治理 CLI。当前实现是一个 5285 行的 Bash 脚本（`bin/gov`），其中包含 54 段内联 Python heredoc 片段和 27 个 `cmd_*` 命令入口。

结构性问题：

1. **不具备跨平台能力。** 强依赖 bash、coreutils、Unix 信号以及 Unix 路径语义。
2. **双语言边界摩擦严重。** 存在 54 处 heredoc 边界，通过 stdout/exit code 传递数据。没有类型检查，IDE 支持也很差。
3. **TOML 编辑本质上是核心能力，却被伪装成脚本技巧。**
4. **恢复语义依赖约定而不是代码来保证。**

## Decision (决策)

将 `comfy_env` 重写为面向 **Linux + Windows** 的 **Rust 单二进制 CLI**。

### Key Decisions (关键决策)

1. **初期采用单 crate。** 保留模块边界，在出现明确需求之前不拆分 workspace。
2. **Rust `gov` 从首个版本开始就是唯一入口点。** 未实现的命令打印错误并以非零状态退出。
3. **采用 vertical-slice migration（纵向切片迁移）。** 每个切片都交付一个完整的命令组。不采用“先做读命令、后做写命令”这种横向分期。
4. 跨平台能力随首次使用它的切片交付。pin 需要定位 venv Python 和有超时的冒烟命令，候选运行需要实时日志，run/stop 负责运行时进程管理。
5. **状态文件 schema 兼容。** 保留现有 schema，新字段只能追加，且必须是可选的。
6. **外部工具边界保持不变。** `uv`、`git` 和 Python 继续作为外部 CLI 调用。
7. **命令面保持稳定。** `init`、`install`、`pin`、`node`、`tx`、`update`、`env`、`op`、`undo`、`run`、`stop`、`help`，不改名。

## Rationale (理由)

### Why Rust (为什么选择 Rust)

- `toml_edit`：专为保留格式的 TOML 编辑而设计，这是本项目最高频的文件操作。
- `Result<T, E>` + `?`：由编译器强制执行错误处理。这个工具如果漏掉一次错误检查，就可能破坏生产环境的 venv。
- 单个静态二进制：两种平台上的终端用户都不需要额外运行时依赖。
- 交叉编译：这是一次性的 CI 成本，不是持续性的负担。

### Why not Python (为什么不选 Python)

该工具要管理 `pyproject.toml`、`uv.lock` 和 `.venv-*`。如果用 Python 实现，会带来命名空间混淆、bootstrap 悖论，以及污染 `uv` 操作边界的风险。

### Why not Go (为什么不选 Go)

`go-toml/v2` 只能做到尽力保留格式，这会在项目最频繁的文件操作上持续带来摩擦。Go 中的 `if err != nil` 属于纪律驱动，而不是编译器强制。

## Non-Goals (非目标)

- v1 不做并发事务、文件锁服务或后台任务系统。
- 不做 ComfyUI 运行时 supervisor，仍然维持 start/stop 加 PID 跟踪的模式。
- 不将 `state/plugins.json` 提升为依赖事实源。
- 不自定义依赖解析，继续委托给 `uv lock` / `uv sync`。

## Architecture (架构)

### Module Layout (模块布局)

采用单 crate，`src/` 按子系统组织：

```text
src/
├── main.rs
├── cli.rs                  # clap 命令定义与分发
├── application/            # 命令实现
│   ├── init.rs
│   ├── install.rs
│   ├── pin.rs
│   ├── node.rs
│   ├── tx.rs
│   ├── update.rs
│   ├── env.rs
│   ├── op.rs
│   ├── runtime.rs          # run / stop
│   └── undo.rs
├── domain/                 # 核心类型、状态枚举、newtypes
├── state_ledger/           # transaction / operation / plugin / conflict CRUD
├── safety_guards/          # backup、restore、undo drift guard、core impact gate
├── dependency_sync/        # uv CLI wrapper、staged workdir、freeze、lock check
├── source_integration/     # git clone/checkout、plugin path mapping
├── runtime_executor/       # 进程启动、PID、timeout、日志捕获
├── platform/               # 跨平台抽象（见 §Platform Abstraction）
├── toml_support/           # config.toml 与 pyproject.toml 的保格式编辑
└── fs_support/             # atomic write、hashing、目录复制、临时工作目录、路径规范化
```

### Core Types (核心类型)

```rust
// 强 newtypes，不使用裸字符串
TxId, OpId, NodeId, GroupName, PythonMinor

// 领域记录
RuntimeConfig, PluginRecord, TransactionRecord,
OperationRecord, ConflictReport, BundleManifest,
PromotionPlan

// 状态枚举，不使用字符串约定
TxKind     { Plugin, CoreUpdate }
TxStatus   { Running, Completed, Failed, NeedsResolution, Resolved,
             Promoted, PromoteFailed, Aborted }
OpStatus   { Running, Success, Failed, Undone }
RunOutcome { Passed, Failed(i32), TimedOut }
```

### Error Model (错误模型)

- **命令层**：使用 `anyhow::Result`，并通过 `.context()` 组织面向用户的消息。
- **关键变更路径**（promote、undo、env import）：返回一个薄的 typed boundary（类型化边界），明确表示：
  - 是否已经产生 side effects（副作用）
  - 是否需要 restore（恢复）
  - restore 是否成功
- 只有在具体路径提出要求时，才响应式地增加 typed boundary。不预先设计四层错误分类体系。

### Logging (日志)

基于 stderr + `state/logs/*` 文件输出的一层薄抽象。日志是审计制品，不是调试输出。接口如下：

- `warn()`、`info()` -> stderr
- `audit()` -> `state/logs/*` 文件
- 命令摘要、关键操作记录、是否发生过 restore 的标记

初始实现可以很简单，但接口不能被硬编码成 `eprintln!`。

### External Command Boundary (外部命令边界)

返回类型化结果的结构化 wrapper：

```rust
struct CmdResult {
    exit_code: i32,
    stdout: String,
    stderr: String,
    command_summary: String,
    log_path: Option<PathBuf>,
}
```

客户端：`UvClient`、`GitClient`、`PythonClient`、`RuntimeClient`。

所有客户端都必须使用平台抽象后的 process runner（见 §Platform Abstraction）。应用层代码中不允许直接调用 `Command::new()`。

## Platform Abstraction (平台抽象)

平台差异归 `platform/` 管理。各切片只实现当前命令需要的能力。

### Venv Python Locator (Venv Python 定位器)

虚拟环境内 Python 可执行文件的位置因平台而异：

| Platform | Path |
|----------|------|
| Linux | `<venv>/bin/python` |
| Windows | `<venv>/Scripts/python.exe` |

通过单一函数 `venv_python(venv_root: &Path) -> PathBuf` 统一封装。所有需要“这个 venv 中的 Python”的代码都调用这个函数。任何地方都不允许硬编码 `bin/python`。

### Process Runner (进程运行器)

统一进程执行能力包括：

- 可配置 timeout（使用 `std::time::Duration`，而不是 Unix 的 `timeout` 命令）
- 同时捕获 stdout/stderr 到内存与日志文件
- 提取退出码
- 超时后的跨平台子进程终止：
  - Linux：SIGTERM -> 等待 -> SIGKILL
  - Windows：`TerminateProcess`

### Signal / Termination Semantics (信号 / 终止语义)

对于 `gov stop`：

| Aspect | Linux | Windows |
|--------|-------|---------|
| Graceful stop | SIGTERM | `GenerateConsoleCtrlEvent(CTRL_C_EVENT)` or `TerminateProcess` |
| Forced stop | SIGKILL after timeout | `TerminateProcess` |
| Child tree | `kill(-pid)` process group | Job objects or tree kill |
| PID validity | `/proc/<pid>` or `kill -0` | `OpenProcess` + `GetExitCodeProcess` |

产品语义，即 user-facing contract（面向用户的契约）如下：

- `gov stop` 先尝试优雅关闭，超时时间为 30 秒，随后强制终止。
- 如果进程已经不存在，`gov stop` 仍然静默成功，并清理 PID 文件。
- 在两种平台上都要检测并清理过期 PID 文件。

### `gov run` Behavior (`gov run` 行为)

`gov run` **不**使用 Unix 的 `exec` 语义。在两种平台上：

- `gov run` 以子进程形式启动 ComfyUI。
- `gov run` 写入 PID 文件，然后等待子进程退出。
- `gov run` 返回子进程的退出码。
- `gov run` 在退出时清理 PID 文件。

这是相对 Bash 版本 `exec` 行为的有意简化，目的是换取跨平台一致性。用户体验保持不变：`gov run` 会一直阻塞，直到 ComfyUI 退出。

### Path Normalization (路径规范化)

- 所有写入状态文件的路径，无论平台如何，都统一使用 **forward slashes（正斜杠）** `/`，以保证可移植性。
- `--comfyui-dir` 接受平台原生路径，并通过 `std::fs::canonicalize` 规范化为绝对路径。
- 在 Windows 上，接受盘符路径和 UNC 路径。不做大小写不敏感比较，路径只在 canonicalize 之后进行比较。
- `fs_support/` 提供 `normalize_path()`，并在所有文件 I/O 路径中统一使用。

### Atomic File Writes (原子文件写入)

- Linux：临时文件 -> fsync -> rename -> fsync 父目录
- Windows：临时文件 -> `FlushFileBuffers` -> 使用 `MOVEFILE_REPLACE_EXISTING` 的 `MoveFileEx`
- 两条路径都封装在 `fs_support/` 中的单一 `atomic_write()` 函数之后。

### Unix/GNU Dependency Replacement Map (Unix/GNU 依赖替换表)

当前 Bash 实现中的每一个 Unix/GNU 工具依赖，都有明确的 Rust 替代方案：

| Unix dependency | Rust replacement |
|----------------|-----------------|
| `bash` | 消除，Rust 二进制即入口点 |
| `mktemp` | `tempfile::NamedTempFile` / `tempfile::TempDir` |
| `sha256sum` | `sha2` crate |
| `cmp`（文件比较） | `fs_support` 中的字节级比较 |
| `timeout` | `std::time::Duration` + process runner 中的子进程超时控制 |
| `uuidgen` / `/dev/urandom` | `getrandom` crate + 自定义 timestamp-hex ID 格式 |
| `/proc/<pid>` | 平台抽象后的 PID 检查（见 Signal/Termination） |
| `kill` / signals | 平台抽象后的终止能力（见 Signal/Termination） |
| `bash -lc`（smoke test） | 结构化命令执行（见 §Smoke Test） |
| `date -u` | `time` crate |
| `find` / `cp -r` / `rm -rf` | `walkdir` + `fs_extra` 或 `std::fs` 递归操作 |
| `sed` / `tr` | Rust 字符串操作 |

## Smoke Test Model (冒烟测试模型)

`config.toml` 中的 `smoke_test_cmd` 将从 shell 字符串改为 **structured command（结构化命令）**：

### New config.toml format (新的 `config.toml` 格式)

```toml
[tx]
timeout_seconds = 120

# Old (Bash era, no longer supported):
# smoke_test_cmd = "python -c 'import torch; print(torch.__version__)'"

# New (Rust era):
[tx.smoke_test]
program = "python"
args = ["-c", "import torch; print(torch.__version__)"]
```

如果不存在 `[tx.smoke_test]`，默认冒烟测试为：

```text
program = "python"   # 通过 venv Python 定位器解析
args = ["-c", "import sys; print(sys.version)"]
```

`program` 字段会经过 venv Python 解析：如果 `program` 是 `"python"`，则会解析为平台对应的 venv Python 路径；其他程序则从 `PATH` 中查找。

**Migration (迁移策略)**：如果工具遇到旧格式的 `smoke_test_cmd` 字符串键，会打印一个 deprecation warning（弃用警告），然后忽略它。

## Bundle Cross-Platform Rules (Bundle 跨平台规则)

### Default: cross-platform import is rejected (默认拒绝跨平台导入)

当 `env import` 遇到一个 bundle 时，会检查：

1. `requires-python` 兼容性（必须匹配）
2. `[tool.uv].environments` 中的 `sys_platform`（必须与当前平台匹配）
3. `[tool.uv].environments` 中的 `platform_machine`（必须与当前架构匹配）
4. manifest 校验和验证
5. 针对导入后的 `pyproject.toml` + `uv.lock` 执行 `uv lock --check`

**Check order (检查顺序)** 固定为：Python 版本 -> 平台 -> 架构 -> 校验和 -> lock 检查。首个失败会立刻中止导入，并报告明确错误。

如果 `sys_platform` 不匹配（例如 bundle 来自 Linux，而当前是在 Windows 上导入），则导入会被**拒绝**，错误如下：

```text
error: bundle platform mismatch
  bundle:  sys_platform == 'linux' and platform_machine == 'x86_64'
  current: sys_platform == 'win32' and platform_machine == 'AMD64'
hint: cross-platform import is not supported in v1
```

未来可能会增加 `--force-platform` 标志用于覆盖这一限制，但 v1 不实现。

### Bundle path storage (Bundle 路径存储)

`manifest.json` 内的路径，无论导出平台是什么，都统一使用正斜杠；导入时再转换为平台原生形式。

### custom_nodes handling (`custom_nodes` 处理)

- `env import` 会删除 bundle 中不存在的 `custom_nodes/*` 目录（与当前行为一致）。
- 在 Windows 上，如果某个 `custom_nodes/*` 条目是 junction 或 symlink，`env import` 会跳过删除并给出警告。它不会跟随或删除链接目标。
- 对现有 `custom_nodes` 的备份通过 `fs_support::dir_copy()` 完成；当遇到 symlink 时，复制的是链接本身，而不是目标。

## TOML Editing Rules (TOML 编辑规则)

通过 `toml_edit` 将其提升为第一等能力：

- `config.toml`：对 `paths.comfyui_dir`、`runtime.python`、`[tx.smoke_test]` 等字段做点状更新。
- `pyproject.toml`：对 `project.requires-python`、`[tool.uv].environments` 以及所有 `dependency-groups.*` 条目做精确维护。
- 原则：**只修改目标节点，不重新格式化整个文件。**

## Dependencies (依赖)

第一批 Rust 依赖：

```text
clap, serde, serde_json, toml_edit, anyhow, sha2, tempfile, time, getrandom
```

后续按需再加：`thiserror`（当 typed errors 真正出现时）、`tracing` / `tracing-subscriber`（当日志抽象超出薄封装能力时）、`walkdir` / `fs_extra`（当 `std::fs` 的递归操作不足时）。

不需要：`uuid`（时间戳 + 随机十六进制格式已经足够）。

## Migration Plan (迁移计划)

### Slice 0: Scaffold and Cross-Platform Infrastructure (Slice 0：脚手架与跨平台基础设施)

先交付 CLI、配置和真相文件读取、平台路径和 status 使用的工具探测。未实现的命令返回包含命令名的错误。pin/undo 引入账本和备份；候选运行引入日志流；需要恢复目录的命令再引入目录快照。

**完成标准**：目标平台可以运行 help 和只读 status，其余尚未实现的命令返回迁移提示。

### Slice 1: `pin add` / `pin list` / `pin remove` / `undo` / `op list` / `op inspect`

覆盖内容：TOML 编辑、`uv lock`/`uv sync`、staged workdir、op 审计、backup/restore、undo drift guard。

**Completion criteria (完成标准)**：这些命令在 Linux 和 Windows 上都可用。`tests/test_gov_cli.sh` 中 pin/undo 部分在 Linux 上针对 Rust 二进制执行并通过。

### Slice 2: `install torch` / `install` / `status`

覆盖内容：受管 dependency groups（core/torch）、生产同步、smoke test（结构化模型）、只读状态报告。

**Completion criteria (完成标准)**：这些命令在两种平台上都可用。smoke test 使用结构化的 `[tx.smoke_test]` 格式。

### Slice 3: `node add` / `node remove` / `tx run` / `tx inspect` / `tx abort` / `tx promote` / `resolve`

覆盖内容：完整插件事务生命周期、candidate env、conflict artifacts、core impact gate、跨平台 candidate 进程执行。

**Completion criteria (完成标准)**：完整插件 hero flow 在两种平台上都可用。

### Slice 4: `update run` / `update inspect` / `update abort` / `update promote` / `update resolve`

覆盖内容：核心更新事务生命周期。

**Completion criteria (完成标准)**：核心更新流程在两种平台上都可用。

### Slice 5: `env export` / `env import`

覆盖内容：bundle manifest、checksum、平台兼容性拒绝、跨平台路径规范化、`custom_nodes` 清理（Windows 上带 symlink/junction 安全处理）。

**Completion criteria (完成标准)**：导出/导入在两种平台上都可用。跨平台导入会被正确拒绝。

### Slice 6: `run` / `stop` / `help` / `init`

覆盖内容：ComfyUI 进程生命周期、PID 跟踪、平台特定终止逻辑、优雅关闭、过期 PID 清理。`init` 被延后到这里，是因为它需要基于模板创建 `config.toml` 和 `pyproject.toml`，虽然逻辑直接，但依赖完整且经过实战检验的 TOML 与配置基础设施。

**Completion criteria (完成标准)**：所有命令全部实现。完整测试套件在 Linux 和 Windows 上都通过。

## Testing Strategy (测试策略)

### Layer 1: Existing Shell Black-Box Tests (Linux-Only Oracle) (第 1 层：现有 Shell 黑盒测试，Linux 专用基线)

`tests/test_gov_cli.sh` 会被保留，**仅作为 Linux 行为基线**。它不是跨平台的 oracle（判定标准）。

支持可配置的二进制路径，以便 CI 在 Linux 上对 Rust 二进制运行同一套测试。

### Layer 2: Rust Unit and Fixture Tests (第 2 层：Rust 单元测试与夹具测试)

- TOML 编辑前后对比的 fixture
- 状态枚举转换验证
- hash、ID 生成、路径规范化（包括 Windows 路径）
- bundle manifest 验证
- 各平台上的 venv Python 定位器正确性
- 结构化 smoke test 命令构造

### Layer 3: Cross-Platform Integration Tests (Hard Requirement) (第 3 层：跨平台集成测试，硬要求)

使用伪造的 `uv`/`git`/`python` 编写平台中立的 Rust 集成测试，并在 Linux 与 Windows CI 上都运行。

### Layer 4: Windows CI End-to-End Tests (Hard Requirement) (第 4 层：Windows CI 端到端测试，硬要求)

在宣称支持 Windows 之前，以下命令**必须**在 Windows CI 中通过：

- `init`
- `pin add` / `pin remove`
- `install`
- `status`
- `tx run`
- `tx promote`
- `undo`
- `env import`
- `run` / `stop`

### High-Risk Paths Requiring Dedicated Tests (需要专门测试的高风险路径)

- `pin add/remove`：TOML 编辑正确性
- `tx promote` / `update promote`：带保护的变更 + restore-before-return
- `node remove`：插件清理 + 依赖移除
- `undo`：drift guard + 备份恢复
- `env import`：校验和验证 + 平台兼容性拒绝 + symlink 安全

## Acceptance Criteria (验收标准)

1. **Rust `gov` 是唯一入口点。** 交付产品中不存在 Bash 包装器、分发器或 shim。
2. **Bash 被彻底消除。** `bin/gov` 不会由用户执行。运行时路径中不存在 Python heredoc。
3. **所有命令都由 Rust 实现。** 完整命令面（`init`、`install`、`pin`、`node`、`tx`、`update`、`env`、`op`、`undo`、`run`、`stop`、`help`）均可用。
4. **Linux 行为兼容。** `tests/test_gov_cli.sh` 在 Linux 上针对 Rust 二进制执行时通过。
5. **Windows 完整命令支持。** 所有命令都能在 Windows 上工作。Windows CI 通过 §Testing Strategy Layer 4 中定义的最小测试集。
6. **恢复语义已验证。** `pin add/remove`、`tx promote`、`undo`、`env import` 的 restore-before-return 行为有专门测试覆盖。
7. **TOML 编辑已验证。** `pyproject.toml` 与 `config.toml` 的编辑具备覆盖关键场景的 fixture 测试。
8. **跨平台 bundle 规则已强制执行。** `env import` 能正确拒绝平台不匹配的 bundle。
9. **不存在 Unix 专属运行时依赖。** 该二进制不会调用 bash、coreutils 或任何 Unix 专属命令。

## Risks (风险)

1. 如果同时追求“完美架构”和“完美行为兼容”，迁移节奏会被拉长。应优先交付切片，而不是先把抽象打磨到完美。
2. `run`/`stop` 的进程语义在不同平台上存在差异。产品语义已经在 §Platform Abstraction 中定义，实现细节仍可能需要迭代。
3. 如果在真实痛点出现之前就让 typed error boundary 大量扩散，它们会变成组织负担。应按需响应式增加。
4. Windows CI 基础设施可能需要非平凡配置（安装 `uv`、`git`、Python）。这部分成本要在 Slice 0 中预留。

## Documents Requiring Updates (需要更新的文档)

以下现有文档包含与本 ADR 冲突的 Unix/Bash 假设。它们已经归档到 `dev-docs-old/`，后续会按需以 Rust 时代的新文档替换：

- `architecture-haiku.md`：引用了 `bin/gov`、Bash 模块卡片
- `subsystems/runtime-executor/spec.md`：依赖 Unix signals、`/proc`
- `subsystems/dependency-sync/spec.md`：使用 Bash heredoc 模式
- `application-core/spec.md`：采用 `cmd_*()` 分发模型
- `conventions/code-map.md`：基于 Bash 函数组织
