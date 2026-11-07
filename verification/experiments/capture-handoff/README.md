# 原操作计划中的捕获交接切片

[V4 详细计划](../../../docs/formal/04-local-core.md) · [证明工作流](../../../docs/proof-workflow.md) · [覆盖记录](../../../docs/proof-coverage.md#current-status)

本组件使用固定原 Charon/Aeneas，从真实 `controller-core` 抽取暂停补偿和迟到
`CommitSnapshot` 的有限调用链。Lean 证明直接引用实际生成函数，不另写业务解释器。
当前六字段调用方的入口是 `caller_replay.py`；历史四字段值层的 `replay.py`、
`Values.lean`、`DropMaterial.lean` 和 `ReviveEligibility.lean` 保留其原版本身份，
不作为当前调用链已通过的依据。实际通过的版本、环境与公共接线见覆盖记录。

## 检查范围和调用方责任

| 实际入口或性质 | 本切片的结论 | 仍须由后续提供的条件或连接 |
|---|---|---|
| `CaptureHandoff` 的十二个值方法 | 失败保留材料责任，撤销不能被迟到成功恢复，已撤销状态不能获准停止原 VM；有正常接受见证 | 字段表示逻辑材料责任，不代表物理文件引用、映射、预算或退出证据；值层历史只覆盖列出的转换 |
| `handle_compensation` 的普通 Pause 分支 | 取消、超时分别与 Pending、OutcomeUnknown 组合；精确说明原记录和计划的更新，保留原材料责任 | 不覆盖其它操作种类的补偿，也不证明整个驱动能完成卸载 |
| `current_capture` | 对应真实函数检查的 Pause 种类、原计划键、operation、epoch 和 generation | 不宣称该函数检查了 VM 或完整 target；这些条件由各自实际入口负责 |
| 迟到 `commit` 成功反推 | 从实际成功返回推出 generation、完整 target、原快照 owner/sandbox/source generation/workspace/manifest/LocalSealed，以及实际 `observe_bundle` 成功所需条件 | 原计划行及撤销状态须绑定实际调用；有限容器写入需要相应容量条件。Unknown、work=true、source=false 不是成功反推的输入假设；NotRegistered 未被排除 |
| 迟到 `commit` 的材料登记 | 原计划和原快照的精确更新；原资源索引成员保留并加入当前快照；其它 World 字段不变；撤销保持且不能 Stop | 不等于 SQLite 已持久化、真实 bundle 完整或 VM 已停止 |
| `record_inner` 与外层 `record` 成功反推及组合 | 原 logical effect、原计划、current operation 等身份检查来自真实执行；外层从原 World 查询终态 Revoking 行，再接入上述材料结论 | 这是有限同步入口的实现证明，不能替代 V9 合法状态可达性、之前 Freeze/Seal 的材料来源或全生命周期组合 |
| 正常成功见证 | 在明确的接口输入和容量条件下确实可以成功，不把成功返回本身作为见证前提 | 接口域见证不是完整系统从初态可达的证明 |

共同性质位于 `SandboxModel/Lifecycle/CaptureHandoff/Conditions.lean`，不依赖生成类型。
`FailurePreservesUses` 与 `LateSuccessSafe` 描述材料责任和撤销；
`RejectsMismatchedTarget` 要求不匹配的完整目标被优先拒绝为 PlanMismatch。
当前正确版和真实忽略 target 错误版使用同一个 `RejectsMismatchedTarget`。

## 真实抽取和依赖边界

`regenerate.sh caller` 直接选择生产 `controller-core` 的局部根，包括实际外层
`late_capture::record` 和 `handle_compensation`。旧的外部包装根可能存在于 LLBC，
却被原 Aeneas 的未使用函数处理过滤；当前入口同时核对 translation JSON 与最终
Lean 函数体，不能仅用 LLBC 中“存在 record”声明接通。

生成的 Types、Funs 和 translation JSON 必须与已提交版本逐字节一致。
`FunsExternal` 仅接入已有审核的十一项纯 Std 模型：Option、String、Vec 和 Bool，
不替代业务查询或处理函数。外部模板出现其它符号即拒绝。
`Option.and_then` 使用真实抽取体，克隆、查询和有限写入分别通过实际生成体的引理连接。
这仍沿用原翻译工具和库模型的信任标准，不宣称翻译器本身已经形式化验证。

Pause 分支有两处同名局部变量由 `capture` 改为 `handoff`，避免原 Lean 后端把局部值与
`capture` 命名空间混淆。未改动业务条件、更新或调用顺序；新源码必须重新抽取并接受
本候选普通检查，不能用旧源码结果自动背书。生成物不手工修补。

正确和错误生成环境具有同名枚举辅助实例，各自在独立导入闭包检查。
当前形式入口检查十四个调用方证明模块及总审计，并使用项目已有的本地导入解析器
展开全部真实依赖；每份文件都接受完整服务诊断、指纹检查和它自身的精确审计。
缺文件、依赖失败、超时、stale、partial、错误、警告或缺审计均拒绝。
公共 Lake、总根、登记、冻结和覆盖记录由统一集成负责人维护，本组件不自行改写。

## 正反例和复现

`native.py` 在普通测试环境执行正确版及四种真实 Rust 错误版：普通 Pause 错走终止
补偿、失败提前清空材料责任、迟到成功恢复资格、忽略原 target。
每版须真实编译成功并产生当前副本的新测试二进制，随后原断言失败；编译失败不算拒绝。
当前形式入口另从真实忽略 target 的 Rust 变异抽取，并证明同一 target 性质不成立。
该形式反例检查首个拒绝：错误实现继续查询而返回 ResourceNotFound，违反必须先返回
PlanMismatch 的规则。它不宣称是 V9 可达状态；原生终态场景另外检查实际材料接纳。
本批不迁移历史两份四字段形式反例，也不宣称当前四个原生错误版都有形式反例。

从最终精确提交导出同一归档并记录 SHA256，两机只交接源码和原始证据，不复制 Cargo
target。使用已有批准的私有 wrapper、固定工具/库及离线依赖；普通检查在 ning，形式检查
在证明环境。外层调度已经持有重任务锁时，下面 `--lock` 使用独立组件锁，不能再次锁住
同一个全局锁。每次使用新输出目录，保留旧失败记录。

普通检查：

```sh
export CARGO_HOME=/private/approved-cargo-home
python3 verification/experiments/capture-handoff/native.py \
  --archive /private/capture-source.tar.gz --archive-sha256 SOURCE_SHA256 \
  --head EXACT_COMMIT --output /private/native-capture \
  --lock /private/coordination/capture-native-component.lock
```

包含 workspace/抽取 crate 格式检查、核心全特性测试、两者严格 Clippy 和四个真实错误版。

当前调用方形式检查：

```sh
python3 verification/experiments/capture-handoff/caller_replay.py \
  --archive /private/capture-source.tar.gz --archive-sha256 SOURCE_SHA256 \
  --head EXACT_COMMIT --output /private/formal-capture-caller \
  --lock /private/coordination/capture-formal-component.lock \
  --tools /private/fixed-tools --packages /private/fixed-lake-packages
```

入口重新抽取正确和忽略 target 版，核对真实根、生成物、外部模板和固定工具，运行
原内核构建、同性质反例和全部本地闭包文件的真实服务检查，转发原始审计输出。
`result.json` 保存 head、归档摘要、实际根、闭包指纹、命令/日志摘要、形式反例及真实
诊断；只有全部成功才写 `complete=true`。这只是本组件结果，不能代替公共完整验收。

## 范围外

完整 worker 抽取、非 Pause 补偿、普通非终态 Commit/Stop 派发、SQLite 原子提交、
真实 bundle/文件引用、原工作退出、未知提交对账、继续原 VM 的驱动进展，以及 Resume/GC
和 V9 全局不变量不由本切片完成。停留在 Pausing 不能满足完整进展目标。
这些业务义务不能通过额外外部假设或本值层定理消除。
