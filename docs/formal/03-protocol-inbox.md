# V3 高级协议与持久收件

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实际差距与所有权

[当前生命周期协议](../../sandbox-protocol/src/lifecycle.rs)已表达 Create/Renew/Unload/Resume/Destroy。正式收件、查询与原 SQLite 映射已连接；Resume 已沿原收件、`admit_lifecycle_resume` 与后台 `advance_recorded_lifecycle_resume` 接入。沿 [sandbox-host-service](../../sandbox-host-service/src/lib.rs) 连接 V4/V5；协议接受不等于 VM 动作成功，完整收件与恢复调用对应仍未闭合。

规范化消息至少绑定 deployment、认证中心/固定节点、owner、session、operation key、全文输入、期望版本、源/目标运行代及 capture。外部用户不指定宿主路径/PID、manifest 成功标志或任意命令；当前模板/配置来源仍受原合同约束。

首批证明固定于 `a9c9967889c4cf298e4378c7106948018835fc64`，从已解码且按生成类型表示的值开始：复用原 `ReplayContract`、`ValidationModel`，连接完整请求重放、请求接受与六类首错，以及 Accepted/Recorded 的完整封套关系。协议版本为 4；正常和错误 Rust 使用同一规格，重新抽取和验收后才迁移适用版本。此切片交给 V1 的是值规格和经实际检查的引理，不要求 V1 等待网络与持久收件证明。

独立认证上下文的连接来源由 V7/连接适配提供；V1/V2 提供当前意图与耐久发送，V4/V5 提供本地接受、耐久结果及调用次序，V6 提供文件/VM 的实际事实。全文相等、slot 形状和版本关系都不提供这些前提。Accepted 的首错依次为 center、部署、内层请求首错、retention slot、接受版本；retention window 不要求等于请求 key window。Recorded 只按完整 intent、版本下界和 current 一致性短路，不检查 terminal 真值。

完整 manifest 任意文件集校验、JSON 字节到值的实际接线、持久收件事务、网络外壳及系统组合另行验收。保留仍适用的 carrier 值往返、manifest 正常见证与局部循环证据；它们不证明任意文件集、哈希计算、材料完整或恢复资格。首批不认领这些后续义务完成，具体候选检查状态仍由覆盖记录和本批 PR 记录。


访问身份的[结构校验切片](../../verification/experiments/stage1-protocol/access-identity/README.md)对应实际
`LifecycleAccessIdentity::validate`：按协议版本、独立 context 的完整 center/node、两种 deployment 关系、非零 accepted version 顺序拒绝。
这里不查原 journal 的 session/owner/key/incarnation 映射，不授予 Access 权限，也不将外部 incarnation 当作本地 generation。
这些当前身份、阶段、寿命及权限关系仍由 V4/V5/V7 的实际提供者连接。

## 2. 状态与义务
