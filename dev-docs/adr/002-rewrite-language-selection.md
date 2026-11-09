# ADR-002: 重写语言选择讨论稿

## 项目概况

comfy_env 是 ComfyUI（一个 Python AI 图像生成框架）的**旁路环境治理 CLI**。它不是 ComfyUI 本身的一部分，而是独立于 ComfyUI 之外，管理其 Python 依赖的安装、锁定、回滚和插件事务。核心工作流是：用户通过 CLI 声明要安装哪些插件或更新哪些依赖 → 工具在隔离的候选环境中试运行 → 用户审查差异后决定是否将变更提升到生产 venv。

工具与被管理环境的交互方式是**纯 CLI 调用**：通过 subprocess 调用 `uv`（Python 包管理器）执行 lock/sync/add/remove/freeze，调用 `git` 执行 clone/checkout。工具本身不 import 任何 Python 库，不链接 Python 运行时——它只是编排外部命令并维护状态文件。

被管理的文件包括两类：
- **TOML 文件**（`pyproject.toml` 和 `config.toml`）：需要**保留格式的读写编辑**——向 dependency-groups 添加/删除条目、改写 requires-python 约束、调整 `[tool.uv]` 段。这不是只读解析，是精确编辑后写回，且需要保留注释和原有排版。
- **JSON 文件**（plugins.json、transactions/*.json、ops/*/meta.json、conflicts/*.json）：插件注册表、事务记录、操作审计元数据、冲突报告，属于常规的 CRUD 读写。

## 当前实现状态

当前实现是一个 5285 行的 Bash 脚本（`bin/gov`），内嵌 54 个 Python heredoc 片段。所有 TOML 解析/编辑和 JSON 操作都委托给内嵌 Python（通过 `$PYTHON_BIN -c` 或 `<<'PY'` heredoc 执行）。CLI 分发为 27 个 `cmd_*()` 函数，覆盖 init、插件管理、事务生命周期、依赖锁定、环境导出导入、运行时控制、操作审计与回滚等全部子系统。

该实现存在三个结构性问题：
1. **不兼容 Windows**：依赖 bash、coreutils（date -u、uuidgen、/dev/urandom）、Unix 信号（SIGTERM/SIGKILL），无法在 Windows 上运行。
2. **Bash 内嵌 Python 不优雅**：54 个 heredoc 横跨两种语言的边界传递数据（通过 stdout/exit code），没有类型检查，调试困难，IDE 无法提供完整支持。
3. **分发依赖复杂**：需要用户环境同时具备 bash、python3（带 tomllib）、uv、git 四个前提条件。

项目处于功能基本完整、尚未大规模分发的阶段，是重写的合理窗口。

## 语言选择的关键约束

1. **必须与被管理的 Python 生态完全隔离**。工具管理 `pyproject.toml`、`uv.lock`、`.venv-prod/` 等 Python 项目元素。如果工具本身也是 Python 项目，会产生 pyproject.toml 命名空间冲突（工具自身的依赖 vs 被管理的依赖）、bootstrap 悖论（用 Python 管理 Python 环境）和 `uv` 操作污染风险。**这条约束排除了 Python。**
2. **跨平台**：必须同时支持 Linux 和 Windows，macOS 为可选。
3. **零运行时依赖分发**：最终用户只需要得到一个可执行文件，不需要安装任何运行时。
4. **TOML 保留格式编辑**是高频核心操作，不是附属功能。

## 候选语言分析

在上述约束下，只有编译为静态二进制的系统语言满足条件。候选锁定为 **Go** 和 **Rust**。

### Go

优势：语法简单，团队上手周期 1-2 周；交叉编译零配置（`GOOS=windows go build`）；`os/exec` 子进程调用直观；`encoding/json` 标准库满足全部 JSON 需求。

不足：TOML 保留格式编辑能力有限。`pelletier/go-toml/v2` 可以做到 Marshal/Unmarshal，但对注释保留和局部编辑（只改一个字段而不影响其他部分的格式）是尽力而为，复杂场景下可能丢失注释或改变键的排列顺序。此外，Go 的错误处理依赖 `if err != nil` 纪律，编译器不强制——对于一个环境管理工具，遗漏一个错误检查可能意味着在不该继续的时候继续执行，破坏生产环境。

### Rust

优势：`toml_edit` crate 出自 TOML 规范维护者生态，**保留格式编辑是其设计目标**，可以精确修改单个字段而完整保留注释、空行和原有排版；`Result<T, E>` + `?` 操作符在编译期强制处理每一个可能失败的点，不可能遗漏错误检查；`clap` derive 宏提供声明式 CLI 定义并自动生成帮助文本和 shell 补全；`serde` + `serde_json` 的结构体映射和 Go 的 `encoding/json` 同样成熟；编译后二进制通常更小（3-8 MB vs 8-15 MB）。

不足：学习曲线显著，所有权/借用/生命周期系统需要 4-8 周才能流畅使用；首次编译耗时较长（分钟级）；交叉编译 Linux→Windows 需要配置 mingw 工具链或使用 `cross` 工具，是一次性 CI 成本但比 Go 的体验多一层摩擦。

### 分析侧重

本项目的工作负载特征是：**频繁的 TOML 保留格式编辑 + 大量 subprocess 编排 + JSON 状态 CRUD + 严格的错误处理需求**。不涉及高并发、高性能计算或复杂数据结构。在这个特征下，TOML 编辑能力和错误处理严格性的权重高于语法简洁性和编译速度。交叉编译的额外成本是一次性 CI 配置，而 TOML 格式保留的不足是每次 pyproject.toml 操作都会遇到的持续摩擦。

## 待讨论

以上是基于项目特征的分析。请各方就语言选择发表意见，特别是：
- 对 TOML 保留格式编辑重要性的判断是否准确？
- Go 的 TOML 生态是否有被低估的方案？
- Rust 的学习曲线成本在本项目规模（~5000 行逻辑）下是否值得承受？
- 是否有其他被遗漏的候选方案？
