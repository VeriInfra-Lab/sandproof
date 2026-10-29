# Host24 同步调用方与 Destroy 候选

[组件入口](../README.md) · [范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [任务 #635](https://github.com/VeriInfra-Lab/sandproof/issues/635)

本候选固定基线为 `f3627b7bb9995251abe31e2bd2bf6252daa1f2c8`，按
[#635 的接线审查](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-6489420997)
采用最小同步拆分。`record_lifecycle_intent` 在原 WorldTransaction 内调用唯一的
`record_using`；Replay 仍提前返回，新操作仍按原故障点和提交顺序处理。Destroy
保留释放中拒绝、原 `terminate_local` 及随后查询的当前操作号。

公开的旧三文件提案漏记了 `codec::StandardInboxSave` 及其 `connection` 字段的
两行 `pub(super)` 可见性。原已编译的私有视图包含这两行，本候选将其一并纳入，
复用原标准保存实例的方法体。该修正没有更换 SQL、JSON、SHA 或保存算法；旧提案
及其证据保留原版本，不称三文件提案本身已经编译通过。

17 份调用方模板中16份保持原字节，正常读取fixture只修正两处23→24的手写证明
字面值；`FullFlowContract` 保持原字节。它们复用完整 load、独立认证、全文查找、
record、save 和 state 规格。`roots.rs.in` 只透明调用实际通用入口。
`candidate-inputs.json` 是候选记录，既不是正式冻结，也不是完整验收回执。

相邻 `inbox-load` 的五份模板仅将实际格式常量 23 改为 24；完整合同结构和首错顺序
保持原样。新增格式见证保留 helper 的准确边界：旧版本且收件表、SourceWork 表均
缺席时返回 None；不能改称所有旧版本均拒绝。真实错误版仅将已有收件表分支的
版本守卫留在 23，结论仍为同一 `FullLoadContract` 的否定。

旧 Host23 调用方开发证明保持原适用范围。当前 Host24 调用方及三个真实错误版已在
固定原工具下重新编译/抽取，经原严格真实文件检查器完成开发检查，见输入记录和
[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-6491475311)；
标准实例的方法转发审查也不等于 Serde、SHA、SQL 或事务已机器证明。普通、原生及
错误版断言交 ning；公共 Lake/总根、登记、输入清单、冻结、覆盖及正式封口由验收
窗口统一维护。当前登记尚未更新，运行旧根不能报告本目录目标通过。

[合同与表示关系](contracts.md)列出主要定理、完整状态、效果前后态和具体提供者边界。
`native_common.rs.in` 是交ning的三条真实ControllerJournal断言候选，尚未原生编译/执行；
它补充现有套件，不替换原断言。开发归档与各结果摘要见 `candidate-inputs.json`，
这些记录不充当原生或正式封口。

V7 提供独立认证来源，V1/V2 提供当前意图及耐久发送；V4/V5/V6 的本地接受、未知
提交恢复、材料引用、真实文件、VM 及物理终止事实仍按具名义务交付。manifest现版
准入和真实材料、
JSON/TLS 字节链、持久收件与完整 V3 继续开放。
