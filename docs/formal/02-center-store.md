# V2 Redis 持久协调与恢复

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#634](https://github.com/VeriInfra-Lab/sandproof/issues/634)。状态：第一阶段实施与证明计划；代码定位基线 `56b14d64`。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 状态与真实边界

对应 [sandbox-center-store](../../sandbox-center-store/src/lib.rs)，调用 V1 决定、V3 发送/回执。保留原一个权威控制块、原字节/修订 CAS、耐久确认、写者约束与失败围栏，不新增并行状态库。存储引擎保证是 A 类条件；实际编码、Lua 参数和业务提交逻辑必须对应。

模型分开：内存候选、已提交 Redis 状态、待发送意图、发送/回执、结果确认。明确未提交、确认提交、可能提交但未知。Redis 与宿主 SQLite 不同事务；中心失联不允许重新创建替代 VM。

首个显式生命周期 profile 直接扩展原 `CenterState`，沿原 `AuthorityState`、`CenterAuthority` 和 `RedisLedger` 保存；不新增权威 key、outbox key 或内存替代账本。新增未来容量同时计入原包络预算，`BusinessEnvelope` 仍保存同一个中心投影。普通初始化必须明确选择此 profile，原 `open/attach/recover` 不把缺失/损坏记录当新状态。

| 实际接口 | 同一原提交保存/读取什么 | 限制 |
|---|---|---|
| `accept_lifecycle` | 完整提交参数、原分配/预留、规范请求、owner/固定 node、接受版本与待发送责任同一次 CAS；确认后才返回意图 | 自有决定和 codec 尚须实现对应；成功组件测试不是整个提交图已证 |
| 原键重放 | 返回原规范请求；即使不改业务状态仍经原 reaffirm 确认当前权威 | Redis revision 可增加，接受版本/编号/原时间不增加或重算 |
| `lifecycle_dispatch_batch` | 从原完整包络耐久确认后枚举 v3 意图；旧 `dispatch_batch` 排除受管会话 | 只提供可派发工作，不证明实际后台 driver 已接通或最终完成 |
| `observe_lifecycle_ready/query_lifecycle` | 全文绑定的原 Ready 与当前 owner/session 查询经原提交/确认 | 认证和原 Host 事实由 V3/V5/V6 提供；本片不构造 Unloaded |
| 原 `recover` | 所有未知提交投影先围栏，重连、读原块并耐久 reaffirm；随后按原键取回原意图 | 不能另分配会话、换 node、释额或只信缓存；完整 manager HTTP 重启仍另验收 |

固定窗口、退休/恢复边界和内部终止尚未完成的部分沿 V1 计划保留；无默认启用、无不受限留史。新 profile 内部版本与原外层格式分别检查，不将 checkpoint schema version 2 称为中心 V2 已完成。

## 2. 必须证明

- 原请求键/全文、固定节点、预留、意图和 outbox 责任同一原提交；提交确认前没有外部派发。所有可能发送均有可重建责任。
- 按实际版本/参数验证原 READ/CAS/健康检查和 Lua 分支；网络错误不等于 CAS 失败，未知提交围栏新写并读回原身份，不重新接受同键。
- 正常恢复打开同一原账本/格式，保留 writer 约束；不能缺库自动初始化，缓存不能代替权威。
- 原宿主事实只有全文匹配并在中心耐久保存后才能确认；确认丢失可重发，不能让宿主提前遗忘唯一结果。
- 新操作窗口关闭、结果保留、拒绝前沿和预留结算有原事务关系；不因单纯 TTL 删除未定责任，不无限保留全部历史掩盖缺失退休算法。
- 待发送/确认/结算工作在重启后仍可枚举和启用；实际 driver 不依赖原 HTTP 请求存活。外部最终可用时原操作可推进。

## 3. 实现连接与验收

V1 给纯决定，V0 给编码/结果合同，V3 给消息匹配，V5 给本地持久事实。逐实际 callback、codec、Lua 文本/参数/返回、连接和错误分支连接；机械连接 B 不包含整个存储模块。沿既有固定工具抽取同步业务，SQL/Lua 剩余对应有明确提供者。

验收包含提交前/后崩溃、提交成功回包丢失、CAS 冲突、相同键重放、不同全文拒绝、宿主已卸载/激活而中心未知、结果确认丢失。正确版本满足 LC-P2/3/8/9 的持久部分；“未知即新派发身份/释额”错误版必须被同性质拒绝。普通真实 Redis 组件与故障测试不能代替生成函数对应或 KVM 闭环。
