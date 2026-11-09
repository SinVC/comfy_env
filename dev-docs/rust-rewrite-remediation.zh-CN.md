# gov Rust 重写整改与交接方案

## 交接起点

本方案整合 Slice 1–6 的三轮静态审查，基线为 feat/rust-rewrite 的 62f236115fb400ff936dff3d4366fda51786bdad。审查和方案编写期间未改动 Rust 实现、未运行测试，也未安装 ComfyUI。新会话应先核对 HEAD 与工作区，保留已有改动，再按批次实施。

可将下面这段直接发给新会话。

> 在 comfy_env 当前工作区实施 dev-docs/rust-rewrite-remediation.zh-CN.md。先读 AGENTS.md 和 dev-docs/architecture.md，再按 A–F 顺序分批完成。每批先明确命令契约，再做最小实现并验收。生产写入按串行执行，不加项目级写锁。测试编写和执行均交给无继承上下文的 GPT-6.1 sol 独立测试子代理，只提供公开输入输出和验收契约，禁止其读取业务源码。使用临时项目、虚拟 ComfyUI 目录和小依赖包。最终报告实现结果、测试结果和未覆盖项。

### 已确定的边界

- 保持 Rust 单二进制为入口，uv、Git 和 Python 继续通过子进程调用。先暂存并锁定，再写生产真相。
- 同一项目的生产写入按串行使用。现有 PID 文件锁继续协调运行时启动、停止和清理。生产进程占用 venv 以及跨命令的顺序性漂移仍需处理。例如观察插件后执行 pin，再尝试晋升旧候选。
- 保留 bundle v1 的同平台、同架构和精确恢复语义。未出现在 bundle 中的普通节点目录属于清理范围。顶层普通文件、symlink 和 junction 属于保留范围。跨平台导入在写目标前拒绝。
- 持久字段只做追加式演进。旧记录仍能查询和安全清理。缺少晋升或恢复所需证据时，给出重跑或恢复指引。通过 CLI 与账本接口操作持久状态。旧记录夹具优先由基线二进制在临时项目中生成。
- 审查中排除的方案包括替换标准库 File::lock()、因调用方无控制台而修改 FreeConsole()、项目级写锁、原地编辑生产文件试解、以裸包名和版本比较 uv.lock 代替核心审批，以及只同步纳管节点的导入模式。

| 批次 | 优先级 | 交付 |
| --- | --- | --- |
| A | P1 | 进程身份、生产环境占用门禁、候选超时和陈旧事务回收 |
| B | P1 | 可判定的候选观察与一致的锁定快照晋升 |
| C | P1 | 可恢复的生产提交、重新 init、节点分组唯一性 |
| D | P2 | 普通 pin 和强制覆盖的管理契约 |
| E | P2 | bundle 准入、节点移动与回滚、文件树操作 |
| F | 后置 | 合并已验证的重复逻辑与诊断清理 |

## A　运行时身份与中断恢复

**入口**　[src/runtime_executor/mod.rs](../src/runtime_executor/mod.rs)、[src/platform](../src/platform)、[src/application/runtime.rs](../src/application/runtime.rs)、[src/application/tx.rs](../src/application/tx.rs)、[src/application/update.rs](../src/application/update.rs)。

1. 为 gov 启动的生产进程记录 PID 和宿主进程创建身份。由平台层查询和验证。Windows 的终止操作使用已验证的同一进程句柄。继续读取旧数字 PID 记录。旧 PID 已消失时可清理，仍存活但身份无法证明时，提示操作者核对并拒绝发送信号。保持先尝试优雅停止、超时后强制终止。
2. 在修改生产配置、真相、源码或 venv 的命令首次写入前检查生产进程。覆盖 pin、install、node remove、两个 promote、undo、env import、重新 init，以及批次 C 的生产恢复入口。检测到 gov 启动的生产进程仍存活时提示先 stop。由 run 自己执行的启动前同步复用现有启动检查。这项门禁依赖 gov 的运行记录，范围限于由 gov 管理的生产进程。
3. 为候选进程设置可清理的 Linux 进程组或 Windows Job Object。在候选代码可能派生后代前建立进程约束。超时、Ctrl+C、启动后设置失败和日志错误共用终止和回收路径。继承输出管道的后代不能让日志收尾无限阻塞。日志仍同步显示并落盘，明确区分正常退出、超时、中断和观察基础设施错误。
4. 在进入可中断阶段前持久化事务所有者身份。需要跨调用回收候选时，记录足以识别该次执行的身份。候选仍活动时拒绝清理它的路径。确认所有者和候选均已结束后，允许现有 tx abort 和 update abort 回收陈旧 running 记录。旧记录没有身份时，提供明确的恢复分支，要求操作者先核实候选已退出。保留事务 ID、日志和最终状态。回收允许重试。
5. 让暂存目录先受 RAII 管理，事务记录保存成功后才转移清理责任。观察初始化失败也应进入可检查和清理的终态。删除候选或暂存目录前继续校验路径归属。
6. 更新 [runtime.md](commands/runtime.md)、[runtime-executor.md](modules/runtime-executor.md)、[architecture.md](architecture.md) 与两条 abort 的契约。

**验收**　身份不匹配的进程不会收到 stop 信号。生产进程运行时写入在备份和生产变更前被拒绝，停止后可正常执行。候选后代保留管道时仍能有界退出。中断或强杀后可以回收旧事务，同节点能再运行，生产文件哈希不变。

## B　候选结果与快照晋升

**入口**　[src/application/tx.rs](../src/application/tx.rs)、[src/application/resolve.rs](../src/application/resolve.rs)、[src/application/update.rs](../src/application/update.rs)、[src/state_ledger/mod.rs](../src/state_ledger/mod.rs)。

1. 把候选就绪、运行结果、包快照、核心影响和日志归为同一次观察。每次重新暂存或 resolve 都先使旧观察失效。同步失败、启动错误、目标节点加载失败和真实非零退出应可区分。现有 allow-failed-run 只适用于本次候选同步成功后记录的真实非零退出，并仍须通过快照完整性与核心审批。旧 ready 或退出码不能跨重试复用。
2. 用明确的启动探针替代默认等满超时。支持该参数的 ComfyUI 可用 --quick-test-for-ci 在绑定服务端口前完成启动。插件事务还要判定目标节点确实加载。退出 0 和泛化日志搜索都不足以作通过证据。核心更新检查启动完成，并报告观察范围内的节点加载失败。对不支持探针或无法判定加载结果的版本，给出可诊断的失败结果。超时仍是执行上限。虚拟 ComfyUI 只模拟公开探针契约。
3. 在 run 或 resolve 锁定成功后保存生产真相基线、暂存真相哈希及其候选观察。观察完成时核验暂存真相未改变。候选同步完成后、启动前采集实际包集，结束后再次采集。如果候选运行自行改变包集，记录偏离并阻止按原快照晋升。确需采用这些依赖时，将明确声明纳入暂存，重新锁定并观察。节点组只写声明的依赖，传递依赖留在 uv.lock，freeze 差异留作审计。
4. 两种 promote 共用基线、快照、观察资格和审批检查。基线变化、暂存真相缺失或哈希不匹配时拒绝，并告知重新 run 或 resolve。确认实际生产包集仍与观察基线相符，再用本次候选差异和当前 policy.core_packages 计算审批。保留 --approve-core、--reason、--allow-failed-run 和 --keep-artifacts 的可见语义。
5. promote 使用已观察的锁定结果，保留本地 wheel 索引在复制锁文件时所需的等价路径换算。求解、冲突处理和重新观察留在 run 或 resolve。移除晋升时的二次求解和探针 venv。缺少证据的旧事务仍可 inspect 或 abort，晋升时要求重跑。
6. 更新 [architecture.md](architecture.md)、[tx.md](commands/tx.md)、[resolve.md](commands/resolve.md)、[update.md](commands/update.md) 和 [runtime-executor.md](modules/runtime-executor.md)，覆盖原先的超时成功、observed additions 及晋升时重新求解条款。

**验收**　目标节点导入失败但 ComfyUI 返回 0 时仍不具备晋升资格。使用基线二进制生成曾非零退出、后来进入锁冲突的旧事务。由新二进制重新观察并触发同步失败，验证旧退出码不能放行晋升。实际依赖与锁一致，传递依赖不写进节点组。候选自行安装的包只出现在偏离记录。真相、暂存内容、实际生产包集或核心策略变化时，给出相应的重跑或审批结果。有效晋升执行已锁定的生产同步，无额外求解或第二个候选环境。

## C　生产提交、恢复与重新初始化

**入口**　[src/application/mod.rs](../src/application/mod.rs)、[src/safety_guards/mod.rs](../src/safety_guards/mod.rs)、[src/state_ledger/mod.rs](../src/state_ledger/mod.rs)、[src/application/init.rs](../src/application/init.rs)、[src/application/node.rs](../src/application/node.rs)。

1. 提取共享生产提交步骤。命令各自完成验证和暂存。共享步骤负责确认备份、登记恢复依据、提交和结案。生产同步、Torch 冒烟、普通冒烟和命令特定写入由调用方选择。undo、env import 和 init 保留各自所需的恢复数据。
2. 以操作账本的持久状态为提交依据。在备份完成后、首次可能改变生产前，记录恢复依据和关联事务 ID。通过追加字段区分已准备、可能已写入、已提交和恢复失败。按实际可能触碰的资源恢复。两份真相文件分别原子写入，中途终止由恢复协议处理。
3. 同步、必要冒烟及生产元数据写入全部成功后，先持久化操作成功，再更新 tx 的 promoted 或原 op 的 undone 引用。关联记录更新失败时保留已提交结果和补记依据。重试仅补记关联，避免重复同步或撤销已提交操作。inspect 可显示关联操作已撤销，保留事务曾晋升的历史。后续生产写入遇到需要恢复的记录时，给出明确指引。
4. **拟新增的恢复入口为 gov op recover <op_id>。** 未提交且可能已写入时，从备份恢复并用旧配置同步。已提交但缺关联记录时补记。旧 running 记录缺少阶段证据或备份不全时保留现场，并报告缺少的证据。恢复失败时保留备份及错误链，修复原因后可重试。重复调用不重复应用生产变更。可预见的普通失败仍在原命令中即时恢复并结案。状态和查询命令应能显示待恢复操作，修剪不得清除仍需要恢复的备份。
5. **重新 init 采用完整暂存和恢复。** 从旧配置和项目生成新草案，先锁定，再备份旧配置及真相，提交并同步。同步失败用旧 Python 配置恢复旧环境。首次 init 可以保留可重试的引导语义。将首次 init 与重新 init 的失败契约分开写清，已有可用环境在重试失败后仍可用。
6. **保证节点依赖组归属唯一。** node add 和 env import 按实际归一化的组名校验。foo_bar 和 foo-bar 这类组合在克隆、替换和生产写入前拒绝。对已有冲突记录，阻止会覆盖或移除共享组的操作，并报出冲突双方。保留旧记录和组名，依赖归属由操作者决定。
7. 更新 [ops.md](commands/ops.md)、[init.md](commands/init.md)、[node.md](commands/node.md)、[application.md](modules/application.md) 和 [state-ledger.md](modules/state-ledger.md)，包括 op recover 的成功、拒绝和重试条件。

**验收**　生产同步或冒烟故障能恢复原真相、配置、注册表和实际包集。第一次生产文件写入后中断可通过恢复入口修复。已提交操作只补记账本时不重复同步。恢复失败保留两次错误及备份。重新 init 因 Python、锁定或同步失败后原项目仍可运行。归一化冲突在源码和生产真相变更前拒绝。

## D　可撤销的 pin 与强制覆盖

**入口**　[src/application/pin.rs](../src/application/pin.rs)、[src/application/resolve.rs](../src/application/resolve.rs)、[src/application/update.rs](../src/application/update.rs)、[src/toml_support/mod.rs](../src/toml_support/mod.rs)。

- **拟定公开契约。** 普通 --pin 只增加约束。插件和核心 resolve 均用显式 --override-requirements 选择 uv 强制覆盖。原 update resolve --pin 默认强制的行为将改变。在帮助与迁移说明给出旧调用的等价新写法。普通 pin 解决不了冲突时，保留冲突记录并提示显式模式。强制覆盖仍需重新观察并遵守核心审批。
- pin add 可用同一显式模式管理强制覆盖。普通 pin 继续写 dependency-groups.overrides。为保留现有依赖引入行为，强制模式可同时写依赖组及 tool.uv.override-dependencies，两处作为一项变更一起暂存、锁定、晋升和恢复。
- pin list 继续支持逐行输出精确 pin，并显示独立生效的强制覆盖。可用 --details 区分普通、强制及旧记录不一致。明确成对管理的 pin 删除时一并移除。已有孤儿或两处版本不一致时，用显式强制模式按规范包名操作。带 marker、作用域或其他语法的手写覆盖保留。
- 节点移除不按包名删除全局 pin。Torch 家族继续由专用安装流程治理。保留非目标 TOML 内容。更新 [ADR-001](adr/001-global-pin-management.md)、[pin.md](commands/pin.md)、[resolve.md](commands/resolve.md) 和 [update.md](commands/update.md)，说明两种模式及历史数据处理。

**验收**　真实不兼容约束通过普通 pin 仍冲突，显式强制覆盖才允许重新观察。新旧强制条目均可列出并删除。删除后真实 uv 解析不再沿用强制版本，其他范围或其他包的覆盖保持原样。pin 变更可由原有 undo 流程撤销。

## E　导入恢复和文件树操作

**入口**　[src/application/env.rs](../src/application/env.rs)、[src/fs_support/mod.rs](../src/fs_support/mod.rs)、[src/source_integration/mod.rs](../src/source_integration/mod.rs)、[src/safety_guards/mod.rs](../src/safety_guards/mod.rs)。

1. 在目标写入前验证 bundle、文件哈希、节点组归属和所选解释器。采用符合 PEP 440 的完整版本匹配和符合 PEP 508 的 marker 判定，可用纯 Rust 规范解析库或经验证等价的 uv 接口。以目标解释器提供 marker 环境。manifest 系统和架构检查、uv lock --check 分别负责 bundle 平台契约和锁一致性。
2. 先列出将替换及移除的普通节点目录、将保留的顶层普通文件和链接。bundle、目标节点目录、治理项目、生产 venv 与恢复区不得发生会导致自复制或自删除的重叠。bundle 节点与保留的普通文件同名时写入前拒绝。与保留链接同名时保持现有跳过并告警的语义，输出须指明该节点的 bundle 源码未替换。
3. 在目标同卷且 ComfyUI 不扫描的位置准备新节点副本和原节点恢复区。逐项移动旧目录，再发布已验证的新目录。每次移动前记录恢复依据。中断后从记录和路径状态判断移动是否已经执行。生产同步在源码替换前失败时保留原树。已替换部分目录时只撤销对应动作。无法安全移动时保留受影响的原目录并恢复此前动作。成功后清理临时恢复区，清理失败应告警并保留可定位路径。env import 成功仍不可用普通 undo 撤销。
4. 将复制、删除和链接识别收拢到 fs_support，保留调用方对链接及 .git 的策略。bundle 快照排除 .git 并拒绝链接，节点源码备份与恢复保留链接。Windows 悬空链接的类型从链接本身读取。需要保持 junction 类型时，移动原目录或使用等价操作。Linux 目录符号链接按链接删除，Windows 按链接类型处理。避免通过链接修改目标权限。
5. 保留已验证的相对 UV_FIND_LINKS 与本地 registry 换算。新增可支持的相对来源应单独验证工作目录语义。无法可靠搬移的引用给出针对性错误。更新 [env.md](commands/env.md)、[fs-support.md](modules/fs-support.md) 和相关 ADR。

**验收**　同平台导入后包集和普通节点目录匹配 bundle，额外普通节点目录被清理，顶层文件和链接被保留。跨平台、校验和错误和不兼容解释器均在写入前失败。patch 约束及带括号的 marker 按规范判定。生产同步失败不重写节点树。目录替换中途故障恢复原条目。Windows 悬空链接、junction 和只读文件，以及 Linux 目录链接均有隔离夹具覆盖。

## F　低风险收敛

- 将配置读取和 UvClient 创建分开，让 node add 和 abort 只依赖用到的配置。复用 inspect、abort 通用步骤及 pins 文件解析，保留插件 resolve 的交互入口。合并 Torch 家族判定，移除重复的 PythonClient。
- 将事务 kind 收敛为序列化值兼容的枚举，在能降低误用的位置采用 ID 类型。将新状态转换集中在相关领域操作中。旧记录缺字段仍能查询，缺少必要证据时拒绝变更。按实际死代码移除宽泛的 allow(dead_code)。
- 批量列表和修剪可对损坏历史记录告警并跳过，单条读取及生产恢复继续严格。保留早期时间戳及备份标记的读取兼容。继续使用 cwd 入口、现有普通解释器健康检查和显式 dispatch。

## 盲测交接与验收矩阵

**测试隔离。** 实施主代理构建可执行文件。测试编写与执行交给独立 **GPT-6.1 sol** 子代理，派生时使用无继承上下文的方式。只提供 gov 二进制绝对路径、公开命令签名、输入输出类型、可观察持久字段和本批验收标准。禁止传递生产实现或 diff，也禁止测试子代理打开或搜索业务源码。只返回通过与失败、复现命令及失败日志。主代理根据结果修实现。

**环境。** Windows 与 Linux 分别使用临时治理项目和自建虚拟 ComfyUI 目录。优先使用 idna、six 或等价小 wheel，通过本地目录或 HTTP 索引验证真实 uv lock、uv sync 及实际 Python 导入状态。Torch 特定用例可用小型同名测试 wheel，避免下载实际 Torch。进程与回滚故障可用外部工具替身、临时进程或可控 smoke 失败制造。真实 uv 的成功及失败路径仍需覆盖。缺少符号链接能力时记录未覆盖，普通目录不算链接覆盖。

| 验收组 | 交给测试子代理的黑盒标准 |
| --- | --- |
| A 运行时 | 正常启动与停止。旧 PID 和身份不匹配的进程存活。生产运行时变更被拒绝且根文件不变。后代继承管道时超时有界。候选被中断后可回收并重跑。 |
| B 观察 | 节点加载成功、目标导入失败但进程退出 0、真实非零、超时和候选同步失败。重试结果独立。候选额外安装的包出现在偏离记录。 |
| B 晋升 | 实际应用锁定快照。真相基线、暂存内容、生产包集和核心策略变化分别得到正确结果。无额外求解或传递依赖写入节点组。 |
| C 恢复 | 锁定失败前后真相一致。生产同步及 smoke 故障可恢复。未完成操作可恢复。已提交操作补记不重复同步。重新 init 失败后原环境可运行。组名冲突在写入前报错。 |
| D pin | 普通与强制模式的真实 uv 行为不同。覆盖可见、可删除、可撤销。旧孤儿可管理，非目标覆盖保留。 |
| E bundle | 同平台成功。跨平台、坏哈希或解释器不兼容时零目标写入。节点替换前与替换中故障分别正确恢复。顶层文件及链接不丢失。 |
| F 兼容 | 无 Python 配置的 node add 或 abort 按自身前置条件运行。旧 JSON 能查询。损坏历史条目不阻断正常列表且不被自动删除。 |

仓库要求的 Cargo 回归也由隔离子代理执行。可让其通过既有 cargo test 入口运行，无需打开业务文件。若测试执行环境要求与源码目录隔离，则由实施代理仅编译测试产物，再交给子代理执行。Linux 另核对 tests/test_gov_cli.sh 的 legacy 基线。如果已更新的产品契约与旧断言不兼容，分别记录实际差异与新契约的验收结果。没有可用 Linux 执行环境时，报告 Windows 结果和 Linux 未验证项。交叉编译不算 Linux 运行验证。

## 完成交付

- 每批实现、对应行为文档及验收结果可追踪。存在中英文镜像的契约应同步更新。先完成 A、B、C 的主要正确性问题，再推进 D、E 和 F。中途交接时记录已完成批次及下一条可执行工作。
- 报告已改命令的成功和失败路径、生产恢复结果、旧记录处理方式及 Windows/Linux 各自覆盖情况。区分通过、失败和未运行。
- 实施前核对工作区，实施后检查 diff。实际生产项目、模板和持久状态不得作为测试夹具。

## 实施与验收记录

| 批次 | 实现与独立验收 |
| --- | --- |
| A | Windows 验证普通停止、忽略 Ctrl+Break 后的强制停止、存活旧数字 PID 拒绝停止和生产写入门禁。两端验证继承输出管道的候选后代不会拖住超时回收。Linux 验证所有者死亡后仍拒绝清理活动候选，候选退出后可重复 abort。Windows 验证 Job Object 随所有者退出终止候选。 |
| B | 两端用真实 uv 验证探针通过、缺少探针、提前零退出、目标加载失败、真实非零、超时、候选包集漂移，以及暂存真相和生产包集漂移。Linux 跟踪到晋升只执行生产包集检查和已锁定的 sync，也验证了晋升时按当前核心包策略要求审批。Windows 用基线二进制生成非零退出后进入锁冲突的事务，新版本重新观察时注入候选同步失败。旧退出码被清空，晋升被拒绝，生产文件与包集保持原样。 |
| C | 两端验证冒烟和同步故障回滚、生产写入中断后恢复原文件与实际包集、恢复失败保留双重错误、重新 init 失败及组名归一化冲突。Linux 验证存活的外部进程会阻止恢复。Windows 在移除 uv 的 PATH 后补记已提交晋升，确认补记不重复同步。 |
| D | 两端用真实 uv 验证普通 pin 保持冲突、显式覆盖后重新观察、列表、成对删除、旧孤儿删除及 undo。带 marker 和作用域的其他覆盖保留。 |
| E | 两端验证同平台导入、Python patch 约束、带括号的 marker、无写入拒绝及冒烟后的节点树恢复。Windows 验证 junction 和悬空 junction、旧 uv 预检兼容及共享句柄造成的中途移动失败。Linux 验证保留符号链接和普通文件。 |
| F | 两端验证损坏历史记录被查询命令警告并跳过，精确读取仍拒绝，以及缺少 Python 配置时 node add 和 abort 可按自身条件运行。 |

独立测试代理在 Windows 和 Ubuntu WSL 各执行 `cargo test`，各 16 项通过。验收使用临时项目、虚拟 ComfyUI、小型 wheel 和真实 uv。最终版本还在两端通过原生导出及导入，核对了真相、节点文件和实际包集。Linux legacy Bash 基线直接运行遇到 CRLF。将测试副本转为 LF 并对接 Rust 二进制后，首个旧帮助输出断言失败，后续断言未执行。

本轮未覆盖 Windows 原生 symlink 创建权限、PID 真实复用，以及 Linux 节点移动中途故障注入。旧退出码与重新同步失败的组合夹具、junction 和目录移动中途回滚已在 Windows 验证。
