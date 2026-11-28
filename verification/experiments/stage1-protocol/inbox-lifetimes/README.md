# 收件寿命校验候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**最终统一候选的抽取与整套原生、Lean 验收尚待完成**；局部结果只按原回执的精确提交保留，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。
[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

旧证明起于 a9c 的 renewal_confirmation::validate_lifetimes，从已解码 Session/Lifetime
值开始。原函数在固定 Aeneas 下触发循环提前返回限制；[生产候选补丁](production-candidate.patch)
仅将原循环及收尾原样移入辅助函数，保留此前的开始时间、首条 Create、整数转换、
checked_add 与上限检查。该辅助函数已被当前生产采用，验证副本不再移动业务代码。
本包直接检查当前实现的合同；有限原版/候选/恢复版对照不被称为一般等价证明。

[共同规格](InboxLifetimesModel.lean.in)逐条记录前驱 deadline、前一接受版本及最后一次 applied
deadline；[循环证明](InboxLifetimesLoop.lean.in)覆盖任意有限列表，[调用组合](InboxLifetimes.lean.in)
连接整个候选的结果，并给出接受/拒绝充要条件。所有拒绝在真实 API 中都为 CorruptStore；
规格中的内部原因名不能当作额外可观察错误码或已证明的执行轨迹。

版本不匹配且前一版本为正时，仅断开记忆的 deadline 前驱；是否属于合法压缩空洞，
另由原 follows_retained 校验。未 applied 的记录仍推进版本，不新增 metadata 必空条件。
没有任何 applied 时不要求最终 deadline 等于初始值。此函数不另检查封套、timeout 正数、
授权、实际删除、资源存在、当前意图或事务提交，也不自行加强成连续版本/必定延长。

[正常输入](InboxLifetimesValues.lean.in)复用现有完整 ValidEnvelope；正反视图使用同一
FullLifetimeContract、Model 和 Values。三个真实 Rust 错误版分别忽略前驱不等、
在 applied 空洞处保留旧前驱、忽略最终 deadline；必须实际编译、抽取并到达同合同反例，
编译失败和超时不算性质拒绝。错误视图只导入共同规格、值及有限循环展开引理。

Option::and_then 由固定 Charon/Aeneas 透明抽取，包含真实 FnOnce 调用。其余逐符号
绑定复用既有 Foundations、SqliteStatements 的 u64→i64 转换、TLS 的 Slice.first，
以及 #718 的借用 Vec 候选。BorrowedVecBinding、FirstBinding、ConversionBinding
在组合定理中具名；[核对器](library_binding.py)核对既有模型原文和两版固定库源。
可计算模型不是标准库实现已证明，借用 Vec 的 V0 调用登记仍待采用。
本路径忽略 TryFromIntError 的内容，不扩展为 Debug/Display 或任意错误观察的合同。

实际 codec 在存在 applied 记录时，通过原 local sandbox/owner 和 namespace 调用
read_lifetime，再运行该校验。原对象、同一事务、持久读取、提交未知/重启及
确认/压缩事务保持性由 V3/V4/V5 继续连接；V1/V2、V6、V7 的业务、资源与认证条件仍各归提供者。
完整 Inbox.validate、JSON/TLS/handler 和完整 V3 不由本切片完成。

## 检查

```sh
python3 verification/experiments/stage1-protocol/inbox-lifetimes/native.py --output artifacts/inbox-lifetimes/native
python3 verification/experiments/stage1-protocol/inbox-lifetimes/formal.py --variant correct --output artifacts/inbox-lifetimes/correct
python3 verification/experiments/stage1-protocol/inbox-lifetimes/formal.py --variant skip-predecessor --output artifacts/inbox-lifetimes/wrong-previous
python3 verification/experiments/stage1-protocol/inbox-lifetimes/formal.py --variant keep-gap-predecessor --output artifacts/inbox-lifetimes/wrong-gap
python3 verification/experiments/stage1-protocol/inbox-lifetimes/formal.py --variant skip-final-deadline --output artifacts/inbox-lifetimes/wrong-final
```

依工作流持有共享重锁，使用固定工具及各副本独立 .lake/build。正式模式增加
--require-native 并提供 ning 的有效普通/native 回执；公共入口登记与完整套件由主控
同批执行和内容复用。prepare-only 只准备抽取视图，不表示 Lean 或正式验收通过。
原始输入、完整 LLBC、原样生成体、实际文件/依赖检查、审计及失败日志保存在 artifacts，
不以只运行旧根替代本包的四个视图。
