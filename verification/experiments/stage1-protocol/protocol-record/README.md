# 同步调用方与 Destroy 候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [任务 #635](https://github.com/VeriInfra-Lab/sandproof/issues/635)

本目录供主控审阅后续接线，尚未登记为正式检查目标。`production-candidate.patch`
是未应用的三文件生产建议；模板只适用于该私有重构的固定抽取视图，不能背书
当前生产调用方，也没有证明与旧标准事务外壳等价。不能只运行旧检查入口便
报告这些新目标通过。实际开发结果、原始归档和待办只在 #635 维护。

`candidate-inputs.json` 记录模板、真实变异、薄根、全部生成函数/外部符号及原始
生成体摘要，属于候选输入记录，不是正式冻结或准入回执。原始源码基线
`9944f879` 与本审阅分支的基线 `41129b1d` 具有相同源码树；本分支不重复提交 #741，
也不自动跟随后续工程提交。

后续工程提交 `f3627b7b`（#747）把 Host 存储格式从 23 升到 24；该常量通过
`controller-service::LIFECYCLE_STORE_FORMAT_VERSION` 进入实际 load 生成体。
本目录保留的抽取视图仍为 23，不能迁记为 Host24 已证。请求/全文重放的未变
输入按原闭包复用；load/调用方和新增 SourceWork 材料释放关系须按精确新基线
重新核对抽取、标准实例及准入。影响核对见 `upstream-impact.json`。

`InboxProtocolRecordModel` 复用现有 FullLoad、认证、全文查找、FullRecord、
FullSave 和完整状态规格；`FullFlowContract` 对任意已解码值和明确效果提供者，
描述同一效果状态上的实际分支及结果。`InboxProtocolRecord`、Guard、Attach、
AttachTotal、AttachConfig 连接实际抽取的通用调用方，包含失败、越界和配置保持。
正常 Create/Destroy 见证复用同一模型。Destroy 见证让旧 binding.operation=30、
独立查询返回 32，检查实际返回完整状态中的 operation 为 32。

三个 Wrong 模板分别对应真实 Rust 删除独立认证、重放继续执行、写回旧 operation
编号的变异，结论都是 `¬ FullFlowContract record_using`。不替换合同，不以编译或
导入失败作为反例，也不假设保存一定成功。错误视图不得导入正确调用方定理：
三组都排除 Record、Values、DestroyActualValues；旧 operation 组还排除正确的
Attach、AttachTotal、AttachConfig。共享模型及见证保持同一字节内容。

接线采用须先核对生产建议的语义及标准实例关系，再用固定 Charon/Aeneas 重新
生成这些真实根，并通过现有严格逐文件服务检查器核对全部模板及依赖。原生正确/
错误/恢复断言由 ning 执行。公共 Lake/Cargo、根导入、模块登记、原生登记、冻结、
覆盖及共享验收由主控集中协调；当前功能准入和 #743 E5 的公共 `run_formal`
跨报告消费仍须实际完成。未实现公共驱动或没有回执时不得转成正式通过。

假设仅沿现有逐符号库合同、标准 GraphicBytes 合同和解码配置的表示条件。
合成 callbacks 是值见证，不证明标准 Serde、SHA、SQL、终止真实资源或持久提交。
V7 提供独立认证来源；V1/V2 提供当前意图和耐久发送；V4/V5 提供本地接受、结果、
同事务及未知提交/重启事实；V6 提供真实文件、VM、清理与 CoW 事实。完整 manifest、
JSON/TLS 字节接线、持久收件和完整系统结论继续单列。
