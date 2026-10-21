# V3 高级协议与持久收件

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。状态：第一阶段实施与证明计划；代码定位基线 `56b14d64`。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实际差距与所有权

[sandbox-protocol 内部生命周期](../../sandbox-protocol/src/internal/lifecycle.rs) 当前只有 Create/Renew/Destroy。本阶段必须版本化加入 unload/resume/查询所需绑定，沿 [sandbox-host-service](../../sandbox-host-service/src/lib.rs) 的原收件/映射连接 V4/V5；协议接受不等于 VM 动作成功。

规范化消息至少绑定 deployment、认证中心/固定节点、owner、session、operation key、全文输入、期望版本、源/目标运行代及 capture。外部用户不指定宿主路径/PID、manifest 成功标志或任意命令；当前模板/配置来源仍受原合同约束。

## 2. 状态与义务

| 状态/事件 | 验收义务 |
|---|---|
| 原始字节→结构→合法请求 | 真实 schema/全部字段/版本/限额/错误顺序对应；解析接受不等于业务授权 |
| 收件、重复、冲突 | 在原 SQLite 事务绑定全文与本地身份；同键重放不生成另一份映射，不同参数明确拒绝 |
| 高级意图→本地操作 | unload/resume 使用同一会话、原来源与当前版本；不绕过 V4 检查，不转成裸宿主命令 |
| 宿主结果→中心事实 | Accepted、SnapshotCommitted、Unloaded、Ready、StateLost、终止/释放保持原类别与原身份 |
| 确认、窗口退休 | 只确认已耐久保存的完整结果；关闭窗口仍保留未结责任，旧消息不能影响复用位置 |
| 乱序、重启、终止竞争 | 旧意图不能复活已终止代；有效终止前已发出的动作保持原责任，不从消息丢失推物理不存在 |

中心要求与宿主应用分开允许暂时不一致；模型不强制“收到 HTTP 就已经 applied”。所有字段以实际 Rust 表示关系连接，不能把整个 serde 或内部网络层当业务正确。

## 3. 实现与验收

V0 提供数值/JSON/TLS 结果基础；V1/V2 提供当前授权意图和耐久发送责任；V4/V5 提供本地接受/回执；V6 提供真实资源证据。必须补原 codec、映射函数、事务回调与恢复入口的生成体/绑定。

验收：全部新消息合法往返，字段变异/非法输入/冲突拒绝，重复和乱序回执，收件提交未知，两管理进程独立重启，destroy 与旧 resume 同时到达。保留真实 mTLS/限额检查；模板分发、跨节点发现不扩入本任务。负责 LC-I3/4 的邻接和 LC-P2/3/8/9，不能单独声称系统唯一运行。
