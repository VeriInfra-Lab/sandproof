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

## 7. 原应用层的接收与启动入口

Host 传输只调用 `ControllerApplication::record_lifecycle_intent` 和 `lifecycle_work_state`，它们在原应用锁下访问同一个 Journal；不向 HTTP 侧暴露私有状态，也不另开数据库。传输仍须提供经独立认证得到的完整上下文。Destroy 的确认接收同时撤销原数据路由；提交结果未知时保守撤销现有订阅，不据此释放资源或认为终止已经完成。

`advance_lifecycle_create_with_verification` 使用原应用的时钟、部署上限、file-policy 和模板检查，复用原物理存储、配额、启动凭据、outbox 与恢复工作流。旧内部 Create 与 V3 共用原准入准备逻辑；Host 侧不能自行构造一份 `InternalAdmission` 绕过应用检查。已有本地映射的重试不重新解析模板别名、分配对象或生成另一份启动凭据；容量拒绝与材料/提供方失败保持原错误分类，不能由本返回值推断中心可释放配额。

`establish_lifecycle_host_epoch` 只调用原 Host 的 `HostCapacity::observe_host_epoch`。该能力默认拒绝，必须由真实受保护提供方实现；HTTP 请求不能注入观察闭包或以摘要存在代替核对。Journal 仍在 SQL 事务外调用提供方，再复核原配置并确认提交。缺少本 manager 的新核对时，应用恢复不仅阻止新启动派发，也把已派发而未知的启动对象保留为未解决，不能通过 reclaim/observe 继续启动。终止、停止和清理仍走原责任，不等待启动资格。

这些入口返回原请求顺序和映射数据；当前 Ready 的专门观察见下节，公开字段由原传输层显式投影，Unloaded 仍须独立证据。真实环境提供方、捕获接受与材料登记、卸载专用物理释放仍需各自接通；不能把完整请求接收、运行态逻辑值或核心 Paused 拼成已完成卸载。应用接线和新增恢复门控的机器证明同样待补。

`lifecycle_pending_creates` 为原后台工作选择提供 V3 输入：在原应用锁和同一读快照内核对持久 Node 配置，只返回尚未被新意图/终止覆盖的 Create，以及其中尚未映射或原生仍在 Starting 的记录。列表上限是已配置 `max_sessions`，每项保留原 owner/session/操作全文；重开从原 inbox 重建，不建立内存权威队列。列表本身不允许创建 VM，原 `advance_lifecycle_create_with_verification` 仍需重新核对请求、环境、模板、配额和寿命。已 Running 的记录不反复排入准入；终止/补偿沿原独立恢复路径处理，不能据列表缺席推断释放。工作选择每轮须给其它候选机会，单项 Busy/材料暂缺不得中止其它项的推进或改成 NotAccepted；实际工作门、模板来源、时钟和有限等待的接线与进展仍由原 worker 提供。未准入且期限已过的记录也不能因扫描而删除或退额度，须由原规定终止/收尾责任处理。

`advance_recorded_lifecycle_create_with_verification` 只处理原 inbox 已接收并保存的全文请求。它先在原锁下核对 Node 与原记录，取回原接收上下文，再释放读锁并调用同一个应用推进入口；后者重新取得原锁并复核当前完整请求。此上下文不是重新认证网络请求的凭据，不允许调用者用本地 Node 身份接收新正文。

## 8. 历史 Create 结果与当前只读 Ready

`observe_lifecycle_create` 返回两类独立信息：`historical_result` 保留原操作的结果，`readiness` 表示本次查询还能否观察到该 Create 对应的运行事实。后续 Unload/Destroy 已被接受时返回 `NotCurrent`，不把历史成功改成失败，也不把它继续用于当前 Running。原生终态/非运行态返回 `NotRunning`；期限已到返回 `Expired`。原映射、启动、源身份、凭据或 Host 观察不足时返回具名 `Pending`，不能推断停机、释放位置或生成另一份凭据。跨模块的公开状态规则统一见[模块合同](../module-contracts.md)。

第一份读快照核对原接收正文、当前会话版本、原本地映射、操作、epoch/generation/VM、寿命、未解决效果、原 CreateOptions 和凭据索引。五项启动动作及其已提交回执均来自原 outbox；身份、原计划、当前尝试、资源和回执须一致。原 Prepare/Activate 保存的源进程身份须一致。寿命事实复用原 `internal_inbox` 的 Ready 投影，不在查询时重新计算开始、结束或最大结束时间。

关闭 SQL 读事务后，应用保留原所有者锁，按原凭据核对和 `WorkRecovery::observe` 观察这五个原动作。此调用不得执行、认领、修复或撤销资源。Host 提供方负责这些操作确实只读、有限等待、对应原动作的当前事实；Prepare/Activate 的当前 PID、进程开始和 boot 须符合原源身份。凭据在观察前后均核对。调用期间期限可以到达，观察完成后仍须用当前受保护时钟检查。

最后重读同一权威并逐项比较正文、版本、映射、原生状态、寿命、原动作与回执字节、选项和凭据。不能仅比较原输入序号：寿命或凭据 SQL 更新可以不改变该序号。内容变化不发布这份旧观察；新意图、终止和到期按最新事实返回。数据库外部变化仍服从原存储身份拒绝规则。只有全部条件仍成立才返回 `Observed`，附原动作身份与回执摘要；这个原生快照依据不是新分配的物理观察序号。

本地 Running 已提交、Ready 尚未发布时 manager 可以重启。该记录无需重新准入，故不列入本地 pending Create；中心原操作的 `ready=None` 责任仍须恢复并继续查询同一请求，不能把接收回执当作查询结束。局部 SQLite 重启样例只能检查本地这一半；中心持久重派发、原 worker 的公平性、真实 Host 的只读事实和受保护配置仍有各自的验收责任。单纯抽取一个纯检查函数也不能证明上述 SQL、外部观察或完整服务接线。

## 9. 显式 Source18 只读责任

`enable_lifecycle_source_observations` 是原完整 FirstUnload/V3 库的受保护配置入口。它保留格式 17 的 Inbox JSON，在同一原 SQLite 事务内登记格式 18、有界配置、空 SourceWork 表及足额后续记录空间。打开数据库不自动启用或降级；17 出现 18 专属表、18 缺表/形状不符/摘要不符/超预算/身份不符均拒绝。共享 Host 16、SQLite user_version 5、core checkpoint 2 不因这项本地功能而改变。真实旧读者须以格式 17 正例和格式 18 拒绝样例实测。

`prepare_lifecycle_source_work` 只在原已接收的当前 Unload、第一代原 Create 已成功且未终止/到期时保存只读问题。它绑定原完整 Unload 键及版本、原 Create/本地映射/VM/运行代/epoch、原寿命、源 PID/开始时间/boot、原五项启动动作和回执摘要、原选项与凭据、提前保存的 HostBootstrap 引用及观察上限。ID 复用原类型与分配：本地 VM 的第一个合法编号是 0，不套用外部正计数的要求。同一个工作只保存一次，不新分配操作编号或重复扣预留；返回的历史工作不构成新的执行许可。

应用 `observe_lifecycle_source_work` 首先从原库核对已提交问题与当前完整绑定，在没有 SQL 事务时调用应用持有的 `HostCapacity::observe_lifecycle_source`，再在原事务内重读当前全部条件并保存原始结果。该 Host 能力默认拒绝。提供方仅允许针对同一原进程做有界 GET 与只读文件读取，不得 Pause、捕获、Stop、修复、改权限或换进程；它须落实配置给出的执行文件、内核、磁盘和配置字节上限。调用期间仍持有原应用锁，不引入新锁框架。

结果是原 GET /vm/config 字节、执行文件与内核 SHA-256、内核长度、运行盘长度，以及精确工作/资源/源身份。配置 Vec 在 SQL 读取前受总记录上限约束，反序列化后再核对每项上限；字节数组 JSON 的最坏编码占用也进入预留。原始设备 JSON 未解释为完整兼容，运行盘长度不证明活盘内容未变化；这些字段既不证明 RAM/设备/磁盘已捕获，也不证明 VMM 的 CPU/XSAVE、实际资源预留或未来持续不变。HostBootstrap 仍只有启动用途。

固定 Firecracker 的 GET /vm/config 不返回全部真实启动项：启动后的 cpu_config、logger、metrics 可为 None，serial 跳过序列化，PCI 和 boot_timer 不在返回结构。`cpu-config:null` 或字段缺失不能解释为“没有自定义 CPU”或“功能已禁用”，也不能补默认值后给出兼容结论。本批 raw 样例故意只给部分字段；保存并重放这些字节不推进原运行阶段。后续完整源接受必须另绑原真实 launch/参数、唯一 API 写者责任及实际禁用事实。

未知、等待方取消或读取后未保存时，原责任仍可枚举，不能据此推断资源消失。未知提交查询同一原键，恢复以实际提交行为为准，不换工作。终止、新版本/代、寿命或其它原绑定变化使迟到结果拒绝；原问题保留为历史数据，不得变成捕获或释放许可。保存过的结果重放为历史字节，不冒充本次新采样；回收只随原会话终止和后续明确的原材料/责任清理规则处理，本批不自动删除或归还位置。实际 Node 提供方、完整兼容校验、物理预算、Capture/Stop/Release 和 Unloaded 接线及本连接的机器证明分别仍有义务。
