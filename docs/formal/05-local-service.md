# V5 SQLite、持久引用与恢复编排

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#637](https://github.com/VeriInfra-Lab/sandproof/issues/637)。状态：第一阶段实施与证明计划；代码定位基线 `56b14d64`。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 唯一持久权威

对应 [controller-service](../../controller-service/src/lib.rs) 原 SQLite、工作/结果和恢复入口。snapshot 元数据、唯一认领、持久 pin 与删除意图扩入原权威；不新造第二本业务账。V4 决定是否合法，V6 提供原文件/进程事件。

模型区分已提交状态、内存候选、文件已发布、外部工作已发生和提交结果未知。SQLite 引擎是外部条件；实际 schema、SQL 条件、事务边界、回调、参数和重启枚举是自有义务。

## 2. 事务与责任接口

| 接口（语义名，非已实现 API） | 事务必须绑定的内容 | 失败/未知 |
|---|---|---|
| acceptCapture | 原请求全文、源代、capture、原物理身份、预算、工作 | 未确认不派发；未知查询同一操作 |
| registerBundle | 原捕获计划、完整校验回执、不可变文件身份、pin/资格 | 文件发布不自动变业务成功；孤儿只按原计划对账 |
| claimResume | 当前会话/来源/寿命、唯一操作/新代、资源预留、pin | 同键返回原 claim；其它竞争失败，不复制 claim |
| recordMayExecute | 原 claim/候选和当前终止版本、不可逆标记、激活工作 | 确认提交后才能可能激活；未知不换候选 |
| releasePin / beginRetire | 具名持有者结束证据；同一事务检查新引用关闭、资格/引用状态 | 活跃/未知使用不清零；重启不靠内存引用数 |
| scheduleDelete / settleRelease | 精确对象与原删除责任、实际删除/映射释放结果、计费 | 部分成功可重试，未知仍占账 |

上述接口需映射实际函数/SQL，不要求建立同名壳函数。状态更改与生成待办必须同事务，接受时保留后续记录空间；普通请求不能耗尽内部终止/清理能力。

## 3. 崩溃恢复和进展

每次外部效果之前都已有可枚举的原身份责任。文件 rename 与 SQLite 提交之间崩溃时，原 capture 意图说明文件为何存在；仅扫描到完整文件不能赋予恢复资格。恢复必须核对版本/完整内容/路径身份并按原状态登记或清理。

pin 是具名持久集合，可重建 original loader、candidate、running backing 和 cleanup 责任。未知加载仍持 pin，实际进程消失需精确证明，不用 PID 查询缺席冒充永无迟到 worker。Retiring 后禁止重新取 pin，所有删除者服从同一状态。

恢复驱动独立于 HTTP future。证明待办在条件满足时可被选中、每个本地有限扫描终止、锁/队列不会让停止等待自己的 backing 删除；外部 I/O 永久阻塞不承诺最终完成。锁的机械保证不是这些业务推进的证明。

## 4. 连接与验收

抽取真实同步事务决定、参数构造、回调和结果分类；原异步 SQLite/任务接线仅按已登记窄合同。表约束/受影响行数/比较字段/提交错误顺序全部接实际版本。

覆盖提交前后崩溃、文件发布未登记、capture 已提交但 stop 回执丢失、claim 与 GC 竞争、load 未返回、activate 丢回执、部分删除、ENOSPC、取消等待以及两次 manager 重启。正确版本满足 LC-P3/5/7/10/11/12；“commit unknown 当未提交”“加载返回就释放 pin”的真实错误版本须失败。V6 仍需证明实际文件与进程后果，不能由数据库行自证。

## 5. 显式卸载检查点

`MaintenanceConfig.checkpoint_profile` 明确选择恢复支持范围。默认 `CreateKill` 保留原 schema 1、配置编码和快照拒绝；`FirstUnload` 使用 core schema 2 与服务 envelope 2，支持第一运行代的一次 Pause/Capture/Stop 历史及原 Create/Kill。根 envelope、core schema、不可变部署配置必须一致。Resume 和第二次新 Pause 在分配新身份、产生待办前拒绝；相同操作键仍由原决定检查全文重放。配置不一致不自动迁移或接管旧数据库。

schema 2 仍逐项校验原状态：操作及 epoch、原 VM/网络槽、计划目标与分配上界、快照归属/运行代/workspace/manifest、LocalSealed 与 Durable/immutable 的对应、捕获未知及撤销后的材料责任、效果原计划和尝试记录。快照文件已出现、保留编号或逻辑 manifest 计数都不能单独证明业务已登记。Worker 观察在回执确认前发生，因此 Dispatched/Unknown 和相应已观察状态必须能够持久化；不要求失败或迟到历史假装从未发生。

服务继续使用原 `WorldTransaction`、输入日志、outbox、回执、SQL 投影和 checkpoint 剪枝，不保留全局历史重放作为卸载格式。每个会话的有限状态及责任进入当前检查点，长期保留一个已卸载会话不能钉住其它已结束会话的全部输入。该 profile 在原 Create 预留中加入一次捕获所需的逻辑效果、重试、输入、操作结果和 SQL 空间；这是数据库/核心记录的预算，实际文件、助手与运行资源额度仍须由捕获接受事务绑定。

这层接口只提供结构化恢复支持。公开 Pause/Resume 入口仍拒绝，直至原源身份、完整请求映射、物理预算和工作在同一接受事务中落实。组件测试通过原较低层接受事务与真实 SQLite/outbox/回执检查关闭重开，不手工写入模拟 World，也不声称生产捕获已启用。核心 `Phase::Paused` 不等于对外 `Unloaded`；原 VM 退出、规定运行分量清理、助手排空和原位置归还仍需分别提供证据。结构检查不是到达性、Host 观察真实性或整个 V4/V5 的机器证明；实际通过范围只在 issue/PR 与证明覆盖登记。

## 6. V3 请求到原 Create 的连接

`initialize_lifecycle_inbox` 只配置尚无原生对象、没有旧 inbox 的完整 `FirstUnload` 数据库。它把原接收方、节点、有限会话数量、请求与后续材料记录空间一起登记到原 SQLite；不创建第二个数据库或第二套本地生命周期。该功能的本地 `controller_meta.version` 为 17，旧读者必须拒绝；共享 host 协议版本 16、SQLite `user_version` 5 和 core checkpoint schema 2 是不同层的版本，不相互冒充，也不在启动时自动迁移。

`record_lifecycle_intent` 保存中心已接受的完整请求与顺序。操作键包括 deployment、owner、window 和 sequence；同键须逐项匹配完整内容，不能换会话或请求正文。`admit_lifecycle_create` 复用原 Create 的模板、寿命、配额、计算资源和位置检查，再在原同一事务内写入请求到本地 sandbox/owner/operation 的映射及原待办。中心的 incarnation 与本地 generation 是独立身份，通过保存的映射连接，不把外部编号强转为本地编号。提交结果未知时查询原键；已存在的映射重放不重新分配位置，也不因后来时钟过期而变成新 Create。

Destroy 可以先于 Create 到达，并形成吸收后续 Create 的终止责任；已有本地对象的 Destroy 复用原启动中补偿和清理决定。Unload 目前只登记原请求及其顺序，不代表已接受捕获、生成了停止许可或完成位置归还；Resume 和重复新 Unload 仍拒绝。`LifecycleWorkState` 的返回值是供查询和组装的数据，不能作为 Host 效果或释放许可。

原生 V3 Create 还要求 `establish_host_epoch` 核对配置中提前登记的宿主环境。提供方必须读取原受保护登记并核对实际节点、boot、固定工具及 CPU/内核条件；配置里有一个摘要不表示核对已经发生。服务在没有 SQL 事务时调用提供方，再在原事务内复核并保存完整引用，确认提交后才允许本 manager 实例启动工作。manager 重开后必须重新核对；历史行可用于恢复查询，不能自行允许新 Create 或继续派发原启动 outbox。提供方失败、内容不符或提交结果未知都不能留下可用的旧核对结果；终止与清理待办仍可推进。

这项连接只检查身份、原始检查的复用与提交顺序。提供方陈述是否符合真实硬件与受保护文件仍属于具名依赖；测试替身返回相同字段不提供这项生产保证。真实 Host 提供方、捕获时源进程/配置观察、文件预算及预留、完整 bundle 登记、Stop 与释放接线未完成时，正式卸载入口保持关闭。本连接及新 SQL 恢复检查尚不作为新增 Lean 证明结论。
