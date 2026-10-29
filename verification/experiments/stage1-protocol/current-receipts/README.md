# 当前完整回执与 Snapshot 资源的连接候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包直接使用既有 `receipts::action_version/receipt_validate` 透明根，对当前原
`ActionIdentity::validate_version` 与 `WorkReceipt::validate_for` 抽取。不修改正确版
Rust，也不复制回执算法。当前资源类型有 Sandbox、Vm、Workspace、Snapshot、NetworkSlot；
旧冻结生成体和原证明保留在原四种资源的适用范围。

`FullReceiptContract` 继续使用原公共 `SandboxModel.Foundations.Receipts.checkReceipt`：
动作版本、回执版本、全文身份、可选观察资源的检查保持首错顺序。完整身份保存版本、
owner、sandbox、operation、epoch、generation、logical_action、attempt 及资源种类/编号。
没有观察时不调用 resource getter；有观察时使用同一给定字典并保留失败，不能据此
把 Unknown 或明确失败改成成功工作。这里的工作协议版本仍为 1，与生命周期请求版本 4 分开。

正确版包含任意值对应、接受充要条件、字段表示单射、原错误/失败传播及合法 attempt 0。
新增 Snapshot 正常观察、九项身份字段拒绝和版本首错见证复用同一规格。真实 owner-only
错误版只比较 owner，实际接受 Snapshot 动作与 Workspace 回执；同一个完整合同明确拒绝。
两版 Definitions/Values 和公共纯规格相同，错误证明不导入正确版实现定理。

## 唯一公共规格与待采用改动

`shared-model-candidate.patch` 只交 V0/主控协调，不在本分支修改公共文件。
它将原 Outcome/回执纯规格移入独立模块，并给同一个 ResourceKind 增加 Snapshot。
原声明名称、字段、checkReceipt、旧生成体对应和旧证明主体保留；没有另造一份协议规格。
候选补丁使用零上下文格式；核对 inventory 的原输入后用 `git apply --unidiff-zero`，不能忽略准入失配。
分离纯规格是为了避免同时导入旧、新生成 WorkOutcome 时的全局 discriminant 实例重名。
固定工具及两组原生成体不改，不用公理、改生成代码或换工具规避重名。

函数级外部绑定仅复用原 String 全文比较、Option.as_ref/is_some_and 三项逐字函数体；
避免导入会同时带入旧生成业务类型的整个 FunsExternal。两版固定库源码和逐符号体均需核对。
`ResourceGetterExact` 仍由具体观察提供者建立，普通值函数不证明观察真实、授权、资源存在、
已持久接受或已完成工作。没有新增路径、map、Serde、SQL 或 HostEvidence 可信合同。

本包仍是未采用公共模块候选的局部开发交付。源码基线/生成接口见 inventory，最小请求见
adoption-request；原源/IR/生成物、私有配置、完整文件内核/同服务诊断和失败沿 #635 交接。
当前没有本包已登记的正式 CLI，不能跑旧 verify-stage1-protocol.sh 报告覆盖本包。
公共 Lake/总根/登记/冻结/覆盖由主控统一，普通及正确/错误/恢复原生由 ning 验收。

V3 的原 probes_match、实际 HostEvidence/进程来源、完整 Ready、JSON 字节、SQL/SHA、
事务未知提交/重启和资源事实仍分别待证。本候选不追加到主控正在接收的冻结批次，
不能据此关闭完整 V0 或 V3。
