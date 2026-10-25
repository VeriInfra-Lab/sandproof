# V3 高级协议与持久收件

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实际差距与所有权

[当前生命周期协议](../../sandbox-protocol/src/lifecycle.rs)已表达 Create/Renew/Unload/Resume/Destroy。正式收件、查询与原 SQLite 映射已连接；Resume 已沿原收件、`admit_lifecycle_resume` 与后台 `advance_recorded_lifecycle_resume` 接入。沿 [sandbox-host-service](../../sandbox-host-service/src/lib.rs) 连接 V4/V5；协议接受不等于 VM 动作成功，完整收件与恢复调用对应仍未闭合。

规范化消息至少绑定 deployment、认证中心/固定节点、owner、session、operation key、全文输入、期望版本、源/目标运行代及 capture。外部用户不指定宿主路径/PID、manifest 成功标志或任意命令；当前模板/配置来源仍受原合同约束。

首批证明固定于 `a9c9967889c4cf298e4378c7106948018835fc64`，从已解码且按生成类型表示的值开始：复用原 `ReplayContract`、`ValidationModel`，连接完整请求重放、请求接受与六类首错，以及 Accepted/Recorded 的完整封套关系。协议版本为 4；正常和错误 Rust 使用同一规格，重新抽取和验收后才迁移适用版本。此切片交给 V1 的是值规格和经实际检查的引理，不要求 V1 等待网络与持久收件证明。

独立认证上下文的连接来源由 V7/连接适配提供；V1/V2 提供当前意图与耐久发送，V4/V5 提供本地接受、耐久结果及调用次序，V6 提供文件/VM 的实际事实。全文相等、slot 形状和版本关系都不提供这些前提。Accepted 的首错依次为 center、部署、内层请求首错、retention slot、接受版本；retention window 不要求等于请求 key window。Recorded 只按完整 intent、版本下界和 current 一致性短路，不检查 terminal 真值。

完整 manifest 任意文件集校验、JSON 字节到值的实际接线、持久收件事务、网络外壳及系统组合另行验收。保留仍适用的 carrier 值往返、manifest 正常见证与局部循环证据；它们不证明任意文件集、哈希计算、材料完整或恢复资格。首批不认领这些后续义务完成，具体候选检查状态仍由覆盖记录和本批 PR 记录。

终局确认与关闭的独立值切片复用完整 intent 和既有版本/retention 合同：终局匹配只核对全文与 Destroy/版本/slot 形状，
不重新认证或 validate；确认与关闭另外要求 Cleaned，关闭回执要求完整 terminal 及精确 retention window。
这些守卫不能签发耐久确认或物理清理事实；续租确认的库表示连接、实际 journal 确认/退休事务和未结责任仍另行验收。
对应入口和边界见[终局值组件](../../verification/experiments/stage1-protocol/result-confirmation/README.md)。

运行期限与恢复来源的[事实叶子合同](../../verification/experiments/stage1-protocol/observation-bindings/README.md)另作值切片：
对应 Create 的原 checked_add、Resume 的来源代/capture/完整摘要与时间字段，以及恢复后续租的非负期限和精确转换。
这些叶子不重做完整观测封套校验，不核实 material_bytes，也不以字段比较代替来源选择、续租单调政策或实际材料/运行事实。
完整 Resume/Create/Unload 观测分支、持久收件和系统组合仍须分别连接并验收。


完整 Unload 结果与 outcome 投影的[值合同](../../verification/experiments/stage1-protocol/unload-observation/README.md)另作切片：
匹配只涉及完整原请求、版本/retention 形状及三类结果各自字段，不重做协议/center 校验或核实 digest。
耐久材料保管、原候选退出或同一 VM 继续由各提供者独立建立，不从字段匹配或 Pending 推断。
