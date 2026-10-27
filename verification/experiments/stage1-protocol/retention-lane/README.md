# Retention lane 校验候选

[协议证明入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md)

本包固定生产 `a9c9967889c4cf298e4378c7106948018835fc64`，依赖 #728 的原终态全文匹配及已有 request/accepted 合同。当前生产只读；`prepare.py` 在隔离副本中给 `LifecycleRetentionLane::valid` 的终态编码调用增加窄 `TerminalEncoding` 参数，标准实例仍直接调用原 `serde_json::to_vec`。不是生产已采用或原实现一般等价的声明。

原方法直接抽取在真实 Serde 调用处失败；保留原 LLBC/诊断。固定工具也拒绝了两次 owned Result match 的连接候选。当前候选保持整个编码结果并匹配其借用，其他判断、短路和调用顺序保持原文；`Option::and_then` 同次透明抽取，不作为新增外部模型。原版、候选、恢复版的原生对照只提供回归证据，一般等价仍开放。

`FullLaneContract` 对任意 lane、显式上下文和编码计算精确描述：

- current_window 等于 retired_through 的 checked successor；`u64::MAX` 的 successor 为 None。
- last_retired 为 None 时，且仅当 floor 为零接受；该独立方法不额外检查 slot。
- 有 last 时要求正 floor、Cleaned 标签、完整原终态匹配、完整原 accepted.validate、相同 slot、retention window 等于 floor，然后才编码。
- 编码返回字节长度至多 8192 时接受，编码错误拒绝；外层失败或发散保持不变，前置拒绝不调用编码器。

完整合同从已解码值开始。使用已有 Terminal/Validation 规格，不将 key window 加强为 retention window，也不由 Cleaned 标签推出资源已清理、上下文来源可信或事务已持久接受。空 lane 的任意 slot、不同 key/retention window、最大 floor、8192/8193 字节边界分别给出见证。

五个原有外部函数绑定为 Bool::ne、按字典的 Option::eq、Result::ok、String::eq 和 Vec::is_empty。`library_binding.py` 从既有 A6 文件逐字抽取三个定义及属性，生成独立导入视图，避免无关 InternalError 派生实例重名；它不修改原始 Aeneas Types/Funs，也不修改 LLBC 或新增库合同。正式驱动核对固定源、完整输入、映射、实际透明根/方法、全部真实文件与审计。

六个错误版修改实际 Rust 的后继、状态、上下文、slot、window 或编码上限。使用同一 `FullLaneContract` 和同一反例值，错误视图不导入正确 lane 实现证明。长度错误版通过自定义真实 Rust 编码器返回 8193 字节；它反驳一般编码参数合同，不声称标准 Destroy JSON 实际能达到该长度。

持有共享重锁后串行运行，使用独立 `.lake/build`，保留缓存和失败证据。开发运行与正式准入分开：`native.py --output ...` 保存原版/候选/六个错误版/恢复版的格式、严格 Clippy、实际 fresh 库与测试程序及指定性质失败；`formal.py --variant ... --output ...` 每个视图重新抽取，检查原始生成体和全部依赖文件。正式模式必须加 `--require-native`，由 ning/主控提供同输入有效凭据，开发记录不能替代。

待证：原实现与窄接口候选的一般等价及生产采用；标准 Serde 实例的精确编码/错误/长度连接；独立认证上下文来源（V7）；真实终态与清理事实（V4–V6）；完整 Inbox.validate、成员及状态保持、持久事务/未知提交/重启、实际 JSON/TLS/handler 接线。主控负责公共入口和覆盖记录的候选增量采用，局部成功不表示完整 V3 或全项目验收通过。

[公共登记候选](coordinator-candidate.patch)接 #713–#728 的 16 份父增量，新增本包原生 case 和七个形式视图，保留旧命令。当前公共 `verify-stage1-protocol.sh` 尚未登记本包，不能只跑旧入口就报告本包通过；`verify-controller.sh` 与共享 `proof_native_suite` 由主控同批统一执行与复用。
