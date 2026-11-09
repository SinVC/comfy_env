# 12 Gov Pin Notes

本文记录 Bash 版的 uv 命令与实现锚点。Rust 版在暂存目录一次编辑 overrides 数组，再调用 uv 锁定。当前流程见 [pin 命令契约](../dev-docs/commands/pin.md)。

## 1. 目标

`gov pin` 提供独立于事务冲突解决流程的全局精确版本 pin 管理能力。它允许用户直接维护共享的 `dependency-groups.overrides`，从而影响：

1. 后续 `uv lock` 的求解结果。
2. prod 环境的 exact sync 结果。
3. `gov resolve` / `gov update resolve` 与手动 pin 的共同求解面。

## 2. 当前实现概览

### 2.1 底层权威

- `pyproject.toml` 中的 `dependency-groups.overrides` 是 pin 的声明态 source of truth。
- `uv.lock` 和 `.venv-prod` 都由该真相经过显式 lock + sync 推导而来。
- `pin` 不引入来源归属模型；手动 pin 与事务 resolution pin 共享同一个 `overrides` group。

### 2.2 三条写入路径

当前会写入 `dependency-groups.overrides` 的路径有三类：

1. `gov pin add` / `gov pin remove`
   - 在 staged workdir 中调用 `uv` 修改 `pyproject.toml`；`pin add` 先 remove 当前目标包名再 add 目标 exact specs，`pin remove` 直接调用 `uv remove`。
   - 修改成功后再显式执行 `lock_project_exact`。
   - lock 成功后才会晋升到 root truth，并继续 prod sync + smoke test。
2. `gov resolve <txid>`
   - 把 `resolution_pins` 合并进事务 staging truth，再 lock。
3. `gov update resolve <txid>`
   - 把参数化 pin 合并进核心升级事务的 staged workdir，再 lock。

## 3. 命令行为

### 3.1 `gov pin add <pkg==version>...`

- 只接受精确版本 spec，不支持范围约束。
- 不接受 `torch`、`torchvision`、`torchaudio`；torch-family 由 `gov install torch` 单独治理。
- 对同名包执行 upsert：同一命令内按规范化包名 last-wins 去重；已有 override 会先按包名移除，再写回目标 exact spec。
- 对非推荐关键包输出 warning；当前推荐集合为 `numpy`、`transformers`。
- group 变更阶段先对当前已存在的目标包执行 `uv remove --group overrides --python "$py" --frozen`，再执行 `uv add --group overrides --python "$py" --frozen`。
- 后续仍保留 `gov` 自己的显式 `lock -> copy truth -> prod sync -> smoke test -> op record` 流程。

### 3.2 `gov pin list`

- 逐行读取当前 root `pyproject.toml` 中的 `dependency-groups.overrides`。
- 输出的是声明态 pin，不是环境实际安装版本。
- 若 `pyproject.toml` 尚未初始化或该组为空，输出 `No pins in overrides group.`。
- 若 `pyproject.toml` 存在但 TOML 非法，会直接返回 parse error。

### 3.3 `gov pin remove <pkg>...`

- 只接受包名，不接受带版本的 spec。
- 匹配按规范化包名进行，大小写以及 `-` / `_` / `.` 差异会被折叠。
- 在 staged workdir 中直接调用 `uv remove --group overrides --python "$py" --frozen`。
- 缺包报错、解析失败和原子失败语义都由 `uv` 原生负责。

## 4. 事务与回滚语义

- `gov pin add/remove` 都走 staged workdir，不直接改 root truth。
- 只有 staged `pyproject.toml` 和 `uv.lock` 都准备好后，才会通过 `apply_staged_pin_change` 晋升到 root truth。
- 若 prod sync 失败：
  - 恢复备份的 `pyproject.toml` 和 `uv.lock`。
  - 再次把 prod 环境同步回恢复后的 root truth。
- 若 smoke test 失败：
  - 同样恢复 root truth，并把 prod 环境回滚到恢复后的状态。
- 成功路径会生成可撤销的 operation，保持与其他破坏性依赖变更一致。

## 5. 为什么委托给 uv

`dependency-groups` 是 TOML 结构，不适合继续通过“解析 TOML 语义后再手写文本替换”的方式局部修改。`gov pin` 当前把这部分委托给 `uv`，主要是为了获得：

1. 对 quoted key 的稳定处理，例如 `"overrides" = []`。
2. 对 inline table / 其他 group 内容的保真，避免不相关条目被字符串化。
3. 对合法字符串内容中 `]` 等字符的稳健处理，避免数组边界误判。

这并不意味着所有 `dependency-groups` 写入路径都已经完全去除手写重写逻辑；当前只收敛了 `pin add/remove`。

## 6. 关键实现锚点

- CLI 入口：[bin/gov](../bin/gov)
- `cmd_pin_add`
- `cmd_pin_list`
- `cmd_pin_remove`
- `write_group_deps`
- `write_pin_specs_last_wins`
- `write_group_names_matching_specs`
- staged apply 与回滚：`apply_staged_pin_change`
- 事务 resolution 仍使用的共享 pin helper：`append_pins_to_overrides_group`

## 7. 测试关注点

`tests/test_gov_cli.sh` 当前覆盖了以下 pin 相关场景：

1. `pin list` 在未 init 和空组时返回 `No pins in overrides group.`。
2. `pin add` 的新增、替换、非法 spec、torch-family 拒绝、warning、lock/sync/smoke 失败回滚。
3. `pin remove` 的成功删除、缺失包报错、torch-family 拒绝。
4. `quoted "overrides"` key 只保留一次，不会重复生成 assignment。
5. `overrides` 中包含合法字符串依赖且字符串内容带 `]` 时，`pin add/remove` 不会破坏 TOML，且文件仍可被 `tomllib` 解析。

## 8. 当前边界

- 不区分 pin 来源归属。
- 不支持 `pin --no-sync` 之类的部分提交模式。
- 不自动推荐具体版本；用户必须显式给出 `pkg==version`。
- `install torch` 和其他 `dependency-groups` 重写路径仍可能使用独立逻辑，它们不在本文档覆盖的这次 pin 收敛范围内。
