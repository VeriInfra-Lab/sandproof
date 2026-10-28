# 原操作计划中的捕获交接切片

[V4 详细计划](../../../docs/formal/04-local-core.md) · [证明工作流](../../../docs/proof-workflow.md)

本组件直接调用 `controller-core/src/capture.rs` 的生产值方法，不使用另一套业务解释器。
`verification/extract-capture-handoff/src/lib.rs` 只提供透明抽取根。
本目录保留的原值方法证明不覆盖后来接通的完整产品卸载或 Resume。两者已接入正式
生命周期；实际实现、有限工程验收及未证明连接统一见[覆盖记录](../../../docs/proof-coverage.md#current-status)。
当前检查点与结构化恢复范围见 [V5](../../../docs/formal/05-local-service.md#5-显式卸载检查点)，不属于本值方法证明。
本组件通过也不表示整个控制器或真实 VM 已验证。

## 接口、性质与调用方责任

| 接口/字段 | 本组件检查的内容 | 本组件之外仍须完成的连接 |
|---|---|---|
| `CaptureCommit` | 未登记、提交未知、确认登记分开；普通失败不把未知改成未登记 | 存储结果必须绑定原计划；未知结果须对账，不能推断没有物理效果 |
| `capture_work`、`unload_source` | 失败保留两项材料责任；资格撤销不释放它们 | 这些是逻辑责任槽，不是完整文件 pin、mmap、预算向量或物理退出证据 |
| `observe_bundle` | 允许记录先前在途操作的迟到成功；已撤销资格保持撤销，并拒绝 Stop | 调用方须核对原 operation/epoch/generation/VM/target，接入真实完整 bundle 及持久结果 |
| `may_stop_original` | 正常登记可以获准停止；已撤销的迟到成功不能获准停止 | 值方法不证明调用方身份检查、SQLite 原子提交、真正 Stop 或后续发布 |
| 原 `handle_compensation` | 重新抽取完整 World 入口，确认工具仍可生成，检查不再新增 `Option.map` 依赖 | 本批不完成该入口的 World 不变量/对应证明；其外部模板绝不作为公理导入 |

共同定义位于 `SandboxModel/Lifecycle/CaptureHandoff/Conditions.lean`，只导入 Aeneas，
不依赖任何生成类型。`FailurePreservesUses` 要求每次失败转换成功返回且材料责任不变；
`LateSuccessSafe` 要求已撤销的状态在记录迟到成功后仍撤销、Stop 为假、已有材料责任保留。
正确版和两个真实错误版使用同一份定义，没有为负例替换成较弱或不同的性质。
正确版另有正常登记并获准 Stop、撤销后仍能登记材料的非空见证。

## 错误版与导入闭包

`native.py` 只在新私有副本中实施三种精确变异：普通 Pause 错走终止补偿、失败提前清空
材料责任、迟到成功恢复已撤销资格。每版必须实际编译成功、生成新测试二进制并触发原断言；
编译失败、缺少预期断言或单纯异常退出都不算拒绝。

后两版还由 `replay.py` 从相同真实 Rust 变异重新抽取，并分别证明共同性质不成立。
对应证书在 `model/DropMaterial.lean`、`model/ReviveEligibility.lean`。
第一种错误版本批只有原入口的原生反例，不宣称有完整 World 的 Lean 反例。

固定 Aeneas 为相同 Rust 枚举生成同名的辅助实例，即使传入不同 namespace 也可能冲突。
因此公共总根只导入纯 `Conditions`；正确 `Values` 和两份负例分别在独立导入闭包检查。
原生成物保持不变。公共根未导入这些实现文件，不构成省略检查的理由。
`replay.py` 要求三份生成库、全部 10 份实际文件的完整服务诊断和 9 个精确审计目标均通过；
缺文件、导入错误、超时、stale、partial、警告或缺审计一律失败。
这些是本组件独立检查要求，不表示当前公共入口已登记该组件。全局 Lake、总根、
组件/原生登记和覆盖记录由统一集成负责；隔离后继的局部成功不能代替共享接线。

## 复现

从已审查的精确提交导出同一归档，记录其提交与 SHA256。只向两台执行环境复制源码归档，
不复制 Cargo target。下面的路径由操作者替换为已有批准的私有目录、离线 Cargo 环境和重锁。
两个组件都要求新输出目录，不覆盖旧失败记录。普通与形式重任务分开执行。

普通测试环境：

```sh
export CARGO_HOME=/private/approved-cargo-home
python3 verification/experiments/capture-handoff/native.py \
  --archive /private/capture-source.tar.gz --archive-sha256 SOURCE_SHA256 \
  --head EXACT_COMMIT --output /private/native-capture \
  --lock /private/coordination/native-heavy.lock
```

包含 workspace/抽取 crate 的格式检查、核心全特性测试、两者严格 Clippy 和三个真实错误版。
原始 stdout/stderr、Cargo 实际二进制及其来源/摘要、编译器信息和结果均保留在输出目录。

形式环境使用现有批准的隔离运行 wrapper 调用以下命令，保持固定工具和库：

```sh
python3 verification/experiments/capture-handoff/replay.py \
  --archive /private/capture-source.tar.gz --archive-sha256 SOURCE_SHA256 \
  --head EXACT_COMMIT --output /private/formal-capture \
  --lock /private/coordination/formal-heavy.lock \
  --tools /private/fixed-tools --packages /private/fixed-lake-packages
```

入口先检查 Charon/Aeneas 二进制摘要；重新生成正确值方法、原 World 入口与两个错误版。
正确生成 Lean 和 translation JSON 必须与归档中的提交版本逐字节一致。三份库只登记在归档
解出的私有 Lake 视图中，使用独立 `.lake/build`，不修改共享源码或清理缓存。
随后执行独立构建/审计，并通过原 `tools/lean_session.py` 检查每份真实文件及其指纹。
所有证据保留，只有完整成功才写 `complete=true`；这仍只是本组件通过，不是共享整套通过。

## 剩余边界

需要继续证明原 World 的计划身份、派发/回执/发布与值方法之间的连接，以及已实现的 SQLite
提交、真实 bundle 和材料引用、原工作撤销/退出、未知提交对账及继续原 VM 路径的对应。
当前六字段 CaptureHandoff 的后继隔离结果另按其精确源码登记；本目录旧结果不自动适用，
也不因产品路径已有工程验收而成为完整实现证明。
停留在 `Pausing` 不满足完整进展目标；本组件不把这些业务义务变成外部假设。
旧模型及旧测试的成功只适用于其原版本，不能替代本候选的检查。
