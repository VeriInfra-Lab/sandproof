# 证明覆盖与剩余义务

[文档目录](README.md) · [验证设计](formal-verification-design.md) · [本地状态机](state-machine.md) · [证明流程](proof-workflow.md)

## 适用版本与边界

已有纯核心成果基线为 `0f1901a210e24da50d259e07df75e8c2cb9d181f`；生成 Lean 最近一次源码更新为 `8612ef1bf99e1ecbddcbd61e8b5fdeedd69e58fe`。下表记录历史版本的具体对应定理，**不为当前 Rust 服务或当前二进制背书**。新版核心、持久化、寿命、宿主执行及网关已有功能实现与工程测试，但尚未重新完成当前版本的抽取和对应证明。本次仓库整理不修改模型、核心、生成产物或工具版本，不新增证明结论；逐批测试结果查相关 PR，不在这里复制实验日志。

目标仍是分模块建立“Rust 实现 → 抽取的 Lean 语义 → 独立模型 → 安全性质”的验证链；不能每次只运行模型定理便称服务已验证。当前新增义务如下：

| 层次 | 当前实现的待证明连接 |
|---|---|
| 纯控制核心 | IPv4、`AbortPrepare`、`RetireActivation`、`RetiredByStop`、维护状态、结果保留与请求窗口等新类型/转换；公开输入穷尽覆盖及完整状态对应 |
| 持久服务 | 意图、准入、编号/收尾额度和 outbox 原子接受；只在确认提交后派发；不确定提交使缓存失效且恢复同一身份；单写者下缓存与检查点＋连续输入尾/SQL 同一快照对应 |
| 生命周期与授权 | 启动期限和运行寿命的区分、运行起点原子发布、续期/到期竞争、总寿命截顶、归属及令牌撤销、终止抢占；内部恢复和鉴权不能整体假定正确 |
| 宿主执行与回执 | 当前操作/代数/尝试及物理位置 incarnation 绑定；可信停止证据先于逻辑释放；未知结果保留资源占用，迟到回执不恢复旧权限 |
| 精确资源清理 | 启动放行前持久登记目录身份；只消费原设备/inode/创建时间、进程、网络和位置证据；同名替换、缺见证、部分删除及持有者未排空保持未知；已确认 Free 的历史身份不阻止合法位置复用 |
| 有界运行 | 工作流未来转换、有限重试/收尾、数据库与 WAL/temp/检查点、登记和稀疏盘未来增长的合成界；结果到期与闭包退休、单调身份前沿、窗口关闭和旧键拒绝；不以概率碰撞或逻辑释放冒充物理释放 |
| 数据通道 | 单沙箱凭据摘要唯一索引、原代数绑定及跨重启局部拒绝/健康对象隔离；派发与撤销/数据 pin 的次序；有界在途缓冲及并发、持续大输出、上传期限、写半关闭后的迟到拒绝响应、精确停止并回收中继；EOF 不是物理清理证明 |

关系允许证明侧保留抽象历史，而实现删除不再可观察的细节；不能要求压缩前后完整 World 逐字相等，也不能因此遗漏仍可查询的结果或已接受未知工作。有限机器只支持可表示前缀内的安全拒绝与明确前提下的收尾，不承诺无限接单、永久外部故障下完成或任意内存耗尽时可分配。

SQLite 原子事务、Linux/pidfd/文件系统观察、Firecracker/jailer 及密码学原语有明确外部合同；**自有检查次序、见证构造、互斥、恢复、事务连接和授权决定不是外部可信假设**。这些仍需分层对应证明与系统测试。

#262 的接口整理将宿主与数据连接合同及排空/子进程持有者移入 `controller-service/src/application`，网关经 `DataAccessPort` 调用；实际授权、生命周期、恢复和维护编排已迁入 `controller-service/src/application/coordinator`。该基线的本地 HTTP 适配目前仅保留在 `sandbox-api/tests/support/managed.rs` 作为组件夹具，生产 SDK 转换由 `sandbox-api` 的唯一解析/投影与中心 `CenterPort` 连接，不再存在 `sandbox-api/src/managed.rs` 生产模块。应用查询使用数值时间与普通选择条件，SDK 时间/令牌编码不混入控制服务；模板编排单独组织但仍排除证明。此搬迁及可注入可信时钟没有改变纯核心或生成证明，新增应用代码尚无对应证明，不能用旧定理背书新模块。宿主端口已按能力拆分，生产实现位于 `sandbox-node/src/adapter.rs`，物理配置租约位于 `sandbox-node/src/physical_profile.rs`；恢复与执行只共享证据类型，仍须分别证明调用资格、未知观察和回执连接。模块搬迁没有新增 Lean 对应证明。后续对应证明须连接端口调用与原管理锁内的派发重检，以及 `TrackedRelayChild` 的确切回收/未知退出责任；还须证明各检查点读取时钟及同一互斥对象内持久决定/宿主派发/撤销/排空的组合；不能把一次时间快照代替执行后与派发前复核。系统时钟的可信性属于外部前提，应用端口不是可以整体假定正确的授权器。

节点的值判断现集中于 `sandbox-node/src/linux/decisions.rs`（动作字段、资源映射、围栏、槽位坐标、进程身份及命令证据），现场读取在 `identity`/`observation`，执行和补偿在 `execution`/`recovery`，位置退休与精确释放仍由 `host_slots.rs` 负责。纯比较不是授权证书：操作/owner/物理位置核验和原锁内的现场读取顺序仍须连接证明。同步子进程统一使用 `sandbox-runtime::OwnedChild`，异步中继仍独立承担持久身份与排空责任；本次整理未运行抽取或新增 Lean 证明。

## 中心与系统组合：工程实现与待证明连接

目标与模块顺序见[验证总览](formal-verification-design.md)，状态/故障及义务见其V0–V9详细计划，端口语义见[模块合同](module-contracts.md)。#292 复用原本地算法，新增纯中心决策、Redis 单块协调、宿主持久收件和两端网关；结果确认、逐实例保留位置及代次退休、原 SQLite 高级入口已有工程实现。唯一入口迁移代码已集成（#321），不因此认定#292的完整故障与晋升验收已通过。以下P1–P8按总览对应V0–V9，不是新代码已有证明；#101跟踪交付，实际证明批次须冻结同一组合提交。计划拆分未改变下文历史定理的适用版本。

| 工作包 | 待交付连接 | 当前证明状态 |
|---|---|---|
| P1 共享合同/类型 | 全局/本地身份、版本/窗口、完整参数、有限数值与编码，公开输入穷尽 | 身份、意图/事实、确认/窗口、只读资源观察及两份模板描述已实现；尚无对应证明 |
| P2 中心控制 | 唯一固定节点、全局准入/收尾、原意图/终止、查询和退休 | 决策、运行/构建池选择、有限健康否决及确认后窗口关闭已有工程实现；尚未抽取/证明 |
| P3 中心持久协调 | Redis 权威表示、CAS/Lua/耐久确认、缓存/outbox、回执和独立恢复 | 原 CAS 同块保存全局责任及模板/构建引用；只有工程检查，引擎前提和自有连接仍分别待证 |
| P4 内部协议/节点入口 | 双向认证映射、同键全参数去重、终止先到、原映射、确认/窗口 | 原 SQLite 高级入口、同一工作许可、事实提取/续期/确认/有限窗口已有实现；取消、实际执行和退休连接未证明 |
| P5 本地控制/执行 | 当前核心/SQLite 与实际围栏、观察、停止和精确清理 | 下文历史纯核心成果可按版本复用；新增连接未证明 |
| P6 公开入口/两端网关 | 归属/语义、全局查询/路由、本地资格、流完整性、驻留与排空；原销毁等待与退休的交错只读取新鲜耐久匹配终局，期限及未知拒绝不变 | 中心复用原 SDK 解析/投影和唯一流处理，宿主重新检查原本地资格；终局等待的工程回归不等于对应证明，产品迁移和工程验收不增加证明结论 |
| P7 系统组合 | 邻接前提、共同不变量、消息、独立恢复与实际组装精化 | 未交付组合定理 |
| P8 容量/保密/完成性 | 中心预留覆盖节点潜在占用、安全保留/退休、双执行关系及条件推进 | 历史内部义务仍开放；新系统结论未证明 |

新增的 `sandbox-protocol/src/internal.rs` 与 `sandbox-internal` 尚未抽取或建立 Lean 对应证明；本批不修改历史纯核心、独立模型及正式 LLBC/Lean 产物。待证连接包括完整身份/协议/计数器编码与拒绝关系、CA/登记摘要到节点角色的映射、固定端点约束、完整正文结束或原连接取消回收后才退还额度、期限与正文预算及任务回收。密码学/TLS 引擎按明确外部原语合同使用，自有登记核验、检查顺序、错误处理和额度连接不能整体视为可信黑箱。只读探测没有耐久结果或执行权限，后续持久映射/高级操作仍需独立连接；组件测试通过不新增系统组合、保密或完成性定理。

本次唯一入口迁移还增加下列连接义务：静态注册与原恢复块一致且不重置代次/高水位；健康/物理空闲的有限否决不能解除责任；GLOBAL 描述到仅改变路径的 LOCAL 描述及原租户映射的准确对应；确认未到不压缩、该实例原责任及引用结算后才退休其固定位置；调用取消不提前退还实际阻塞工作许可；缓存与构建池各自独占、共同峰值仍计入原物理 profile。协议 2 的 `retention_slot` 与该位置的 `request_window` 分开建模，最近完整终局回执、单调拒绝前沿及永久全局/本地编号仍需证明；旧请求/迟到确认不能触及后继实例，长寿命同伴不阻塞另一位置的条件推进，未来每位置回执空间纳入准入预留。这些新增纯转换与 Redis/SQLite 连接只有工程检查，未抽取或证明；历史纯核心结论不覆盖新格式及入口。模板编排/产物服务整体继续排除证明，但核心使用授权、不可变、准确模板及资源描述的前提仍明确。受保护目录账号是可信部署输入，不是可由客户端指定的权限；自有路径/身份/预算检查不因这一输入而自动成立。

需要补建系统状态与历史，不只扩展 World：中心决定/持久缓存、各节点本地核心/持久/真实资源/通道、中心通道、在途消息及独立时钟/故障。模型参数化任意预算内有限静态节点，允许两端滞后、重复/迟到/乱序/丢失及独立崩溃。两节点组件或一台物理机的双服务测试不是一般定理。

`sandbox-center-store` 只有工程测试，尚未抽取或建立对应证明。需连接严格状态编解码、四字段控制块、原字节/修订 CAS、独占写者、持久健康及未知后耐久重确认，再连接未来中心的提交前不派发、额度和原操作身份。`Observed`/`Committed` 类型区分不能代替这组定理；Redis 持久化、可信同机回环/ACL、文件锁和文件系统原语仍需外部合同，自有检查/围栏与连接不移入可信边界。本批不修改历史纯核心或 LLBC/Lean 产物。

新增中心的决策、模型与恢复校验分别位于 `sandbox-center-core`，没有 I/O/VM 依赖，时间是显式输入；序列化、身份字符串/编号解析、整数比值和编码包络仍有自己的对应义务，不宣称能无修改抽取整个服务。异步 HTTPS 复用原 TLS/HTTP 边界，`sandbox-host-service` 在原应用锁前统一限制收件、执行和周期恢复的阻塞任务，并把许可保留到实际任务退出；这一取消/实际执行连接尚未证明。`CenterAuthority` 与脱离借用的 `Dispatch` 需证明成功提交/当前意图/发送顺序及迟到回应隔离；宿主收件需证明原事务、全文/摘要、终止先到、未来 SQL 保留与格式恢复的连接。原子准入映射复用原本地事务；新增义务包括租户映射单射/单调、完整模板及获批形状对应、销毁收件与原核心终止同提交、事实观察与原 Running/Destroyed/ConfirmRelease 的对应，以及确认之前的历史 pin。工程故障测试不等于没有重复 VM 的实现证明，HTTP 收件不授予绕过本地检查的执行许可。修改的 controller-service 格式/容量/恢复胶水没有新增定理；旧纯核心的指定版本结论不背书这些新增组件。所有新模块只有工程检查，实际证明继续归 #101。

查询的调用期间视图与已确认续期顺序、命令句柄/终止范围及 envd 登记丢失、局部故障传播与跨实例推进、零计数键/订阅/池/关联/队列及实际退出计费均为明确义务，精确定义分别见[V7](formal/07-public-api.md)、[V8](formal/08-data-gateway.md)、[V9](formal/09-system-composition.md)。分类为控制器证明、外部条件下证明、工程测试或不承诺不等于完成；尚未闭合的自有连接不能移为外部条件。Start/Connect 无累计输出上限，SDK 客户端 RAM 不属宿主内存结论。

创建/连接等待复用原回复期限，健康否决只重查原会话；迟到观察仅在新鲜持久快照仍为完全相同的原操作时重查，不把旧包当成功。这些连接尚无证明。模板缓存的原身份恢复与库存不可用状态采用工程测试；核心仍须证明缓存失败不授予新执行资格、不能抹除已有实例责任，并连接唯一宿主驱动对原实例的到期/终止/精确收尾。模板排除边界不排除这组局部故障隔离和生命周期连接。

实际 Rust 对应须包含锁/事务/异步/回调及微步骤，固定 Charon/Aeneas 支持子集不覆盖任意 Rust。手写可计算库模型无公理不等于已证明真实库等价；库模型前提、工具/编译器、平台/features/cfg、源码/生成入口及指纹须逐批列明。模型安全、实现精化、系统组合分别验收，有限步安全不直接推出最终完成性。

[#328](https://github.com/ningZhang-cs/sandproof/issues/328) 的查询与模板分发修复不修改已证明纯核心或 LLBC/Lean。当前 `PublicSnapshot` 同时投影原包络的生命周期、隐藏对象及不可变模板绑定，get/list 字段只消费这一视图；后续 V7 对应须连接精确绑定、资源形状与原筛选/排序/分页，而非把模板提供者的另一时刻读取当作同一快照。原 Redis 重确认和未知围栏不变。模板缓存新增按受管修订/摘要的有限轮转提示，只有原可用回执提交产生缓存事实；提示的键清退、有界性及重启不改变责任属于工程检查，模板子系统整体排除边界不变，不新增形式化完成性结论。

## R2 与持续输出的待证明连接

[#290](https://github.com/ningZhang-cs/sandproof/issues/290) 的宿主辅助执行与命令输出策略依[模块合同](module-contracts.md)区分。Start/Connect 不再要求累计输出 `nr ≤ B`；只要求任意有限执行前缀内已写字节不超过已读字节、保持顺序，并分别约束在途队列、单消息、并发与撤销。文件、一元回复和宿主辅助输出仍有各自累计预算。幽灵历史可以增长，不能因此要求实现保存完整历史；执行期限不能替代内存边界。固定 envd、HTTP 库和内核缓冲的可信前提，以及客户端累计输出不在宿主保障内，均须明确。

同步辅助命令新增非阻塞三管道交换、分层期限、累积缓冲前输出检查、启动前额度和固定未知退出回收器。后续局部证明应分别连接额度守恒、错误不产生成功证据、精确进程退出观察、未知保留持久身份及物理资源占用，再连接节点回执与控制器发布。不能把超时视为未执行或已回收，也不能依赖 Drop 的无限等待证明终止；退出仍未知时继续占额，最终完成需要外部恢复/调度前提。这些适配层改动尚未抽取或形式化证明，历史纯核心定理不背书它们；本批未改已证明核心或正式 LLBC/Lean 产物。

#290 R6 的故障投影、恢复样本和后台阶段诊断属于当前应用/宿主实现，只有工程回归，没有新增 Lean 定理。纯核心及生成 Lean 未改变。后续对应证明仍须覆盖原错误分类、局部隔离和共享损坏中止，以及诊断不参与授权、状态转换或资源释放；日志成功不是工作完成证据，不能把本批测试当作当前服务已形式化验证。

## 模板子系统排除边界

按[蓝图](project-blueprint.md)与 [#123](https://github.com/ningZhang-cs/sandproof/issues/123)，整个模板构建与转换子系统采用工程测试，不做形式化证明：第三方构建器、Dockerfile 执行、OCI 获取/解包、镜像制备，以及自有构建编排、查询/别名/登记和发布胶水均排除。

但核心**使用**模板绑定、资源形状记账、恢复匹配与实例精确清理仍在后续验证范围，不能借模板排除条款一起排除。部署前提是产物校验后不可变、交接前关闭写者、路径可信且无不合作写者；ctime 与摘要缓存本身不是这一前提的证明。不宣传用户镜像、供应链或整个沙箱平台都已形式化验证。

## 历史纯核心覆盖

| 输入或能力 | Rust 与模型的对应证明 | 统一入口 |
|---|---|---|
| 初始化 | `initial_world_refines`，完整状态表示 | 初始化函数 |
| 创建、暂停、恢复、销毁、失败补偿 | 五类 `handle…_refines`，全部接受、按顺序拒绝以及完整后继状态；`handleRequest_refines` 合并七类请求 | `handleController_request_refines` |
| 请求重放 | `handleReplay_refines`，所有接受和拒绝分支 | `handleController_replay_refines` |
| 结果查询 | `handleQuery_refines`，所有接受和拒绝分支 | `handleController_query_refines` |
| 执行、文件读写、入口和出口访问 | `handleAccess_refines`，五类输入的全部分支 | `handleController_access_refines` |
| 客户端等待超时 | `handleRpcTimeout_refines`，所有拒绝分支及两处记录同步改为“结果未知” | `handleController_rpcTimeout_refines` |
| 外部命令派发 | `handleEffectDispatch_refines`，全部分支 | `handleController_effectDispatch_refines` |
| 外部命令超时 | `handleEffectTimeout_refines`，全部分支 | `handleController_effectTimeout_refines` |
| 外部命令最终失败 | `handleEffectFinalize_refines`，全部拒绝与成功状态更新 | `handleController_effectFinalize_refines` |
| 明确失败后重试 | `handleEffectRetry_refines`，全部拒绝与成功分支；另证机器编号耗尽时拒绝 | `handleController_effectRetry_refines` |
| 外部命令回执 | `handleEffectReceipt_refines`，缺失、尚未派发、证据不足、成功/失败、相同重放和冲突全部分支 | `handleController_effectReceipt_refines` |
| 外部命令处理整体 | `handleEffect_refines`，派发、超时、回执、重试、最终失败五类输入 | `handleController_effect_refines` |
| 控制器崩溃、恢复、重复或迟到消息观察 | `handleRuntime_refines`，全部分支及接受/拒绝反向覆盖 | `handleController_runtime_refines` |
| 工作执行的旧尝试 | 当前尝试读取、真实向量查找、旧尝试拒绝 | `handleController_worker_stale_rejected` |
| 工作执行的未入队消息 | 当前尝试且未入队时拒绝 | `handleController_worker_unqueued_rejected` |
| 工作执行的未等待结果消息 | `handleWorker_notAwaiting_refines`，全部命令的共同第三项拒绝检查 | 已用于十一条完整工作命令入口 |
| 文件策略工作命令 | `handleWorker_filePolicy_refines`，成功更新及全部内部拒绝；公开组合再覆盖旧尝试和未入队 | `handleController_workerFilePolicy_refines`，已纳入混合执行 |
| 虚拟机回收工作命令 | `handleWorker_cleanupVm_refines`，资源、身份、停止状态、当前指针检查及完整回收更新 | `handleController_workerCleanupVm_refines`，已纳入混合执行 |
| 网络槽回收工作命令 | `handleWorker_cleanupSlot_refines`，资源、身份、激活、已回收、当前指针检查及完整回收更新 | `handleController_workerCleanupSlot_refines`，已纳入混合执行 |
| 围栏工作命令 | `handleWorker_fence_refines`，编号和终止标志匹配、单调推进、启动或撤销阶段、可选网络槽及完整更新 | `handleController_workerFence_refines`，已纳入混合执行 |
| 准备虚拟机工作命令 | `handleWorker_prepare_refines`，完整计划、围栏、资源冲突、恢复来源与快照可用性检查；成功包括两种资源登记、指针更新和三次入队 | `handleController_workerPrepare_refines`，带显式容量条件纳入混合执行 |
| 网络配置工作命令 | `handleWorker_network_refines`，阶段、身份、策略、当前指针、资源归属和围栏的全部顺序检查及完整更新 | `handleController_workerNetwork_refines`，已纳入混合执行 |
| 激活工作命令 | `handleWorker_activate_refines`，身份、已准备、网络槽、围栏、无运行实例的真实有限遍历及完整执行状态更新 | `handleController_worker_refines`，已纳入混合执行 |
| 冻结工作命令 | `handleWorker_freeze_refines`，暂停阶段、身份、当前指针、执行状态及围栏检查，完整冻结更新 | 同上 |
| 封存快照工作命令 | `handleWorker_sealSnapshot_refines`，封存计划、冻结虚拟机、围栏和快照不存在检查，完整快照及索引写入 | 同上 |
| 提交快照工作命令 | `handleWorker_commitSnapshot_refines`，先查快照再查沙箱，检查清单、身份、暂停阶段和本地封存状态；标记持久且不可修改并入队停止命令 | 同上，额外要求一条命令容量 |
| 停止工作命令 | `handleWorker_stop_refines`，暂停时检查持久目标快照，撤销时检查终止围栏，完整停止状态更新 | 同上 |
| 工作命令整体 | `handleWorker_refines` 枚举全部十一种命令，成功、顺序拒绝及完整后继状态 | `handleController_worker_refines`，不要求入队或等待结果前提 |
| 控制核心全输入 | `CoveredInput.ofRust_toRust` 检查实际公开输入类型无遗漏，`handleController_refines` 比较拒绝或成功事件和完整状态 | `controller_execution_refines`、`controller_execution_preserves_safe` 覆盖全部现有输入的有限混合调用 |
| 工作端资源登记与写入 | 虚拟机、网络槽及快照的真实登记、资源表更新；索引表默认值、去重插入及实际循环终止均已证明 | 相应工作命令成功分支实际使用，不单独计作命令覆盖 |
| 稳定状态发布的更新动作 | 四类 `generated_publish…_refines`，核对完整后继状态、操作记录及结果；当前尝试成功检查亦有完整证明 | 四类完整入口及统一组合均已接通 |
| 销毁完成、失败完成发布入口 | `handlePublishDestroyed_refines`、`handlePublishFailure_refines`：真实提取函数的全部成功／拒绝分支，比较拒绝原因、成功事件及完整后继状态 | `handleController_publishDestroyed_refines`、`handleController_publishFailure_refines`；已纳入连续执行组合 |
| 暂停完成发布入口 | `handlePublishPaused_refines`：快照完整性、当前尝试回执、资源关闭、操作账本的全部成功／拒绝分支及完整后继状态 | `handleController_publishPaused_refines`；已纳入连续执行组合 |
| 运行完成发布入口 | `handlePublishRunning_refines`：虚拟机、网络槽、文件策略、当前尝试回执及账本的全部成功／拒绝分支与完整后继状态 | `handleController_publishRunning_refines`；已纳入连续执行组合 |
| 四类稳定状态发布整体 | `handlePublication_refines`，覆盖运行、暂停、销毁、失败四类完整入口 | `handleController_publication_refines` |
| 发布前的资源关闭与停止回执检查 | `generated_indexedVmsClosed_refines`、`generated_indexedSlotsClosed_refines`、`generated_indexedStopsSucceeded_refines`，包含实际有限遍历的终止与结果对应 | 已组合进销毁完成、失败完成入口；暂停完成复用资源关闭检查及指定停止命令的当前尝试成功检查 |
| 调度命令安排六类入口 | 准备、封存、提交、停止、虚拟机回收、网络槽回收的全部成功／顺序拒绝路径及完整后继状态 | 六个统一控制入口定理及混合执行组合已接通 |
| 调度资源预留两类入口 | 启动资源、暂停快照的全部顺序拒绝与成功更新，包括编号递增、计划字段和资源列表 | 已接通统一入口及混合执行 |
| 调度整体 | `handleScheduler_refines` 实际枚举全部八类输入 | `handleController_scheduler_refines` |
| 状态观察和轨迹监控 | `Runtime.lean`、`Monitor.lean` 中独立观察函数的对应证明 | 独立只读入口 |

派发和超时证明需要有限存储可容纳更新的条件，以及命令索引完整性。
生命周期请求要求计数器和存储满足 `HandlerMachineHeadroom`，并显式
要求 `RequestCapacity`：暂停最多新增两条命令，其余四类最多一条。
入队、命令索引、操作账本、操作计划和完整后继状态都包含在对应关系中。
补偿还证明真实证据检查与模型一致，不把“已请求补偿”直接当作失败证据。
这些是有限机器内的条件化对应证明，不包含生命周期计数器耗尽时与无界
模型仍有相同结果的断言。
访问证明需要权限形状不变量；模型已经证明可达状态满足该不变量。
客户端等待超时证明需要沙箱与操作记录存储的容量条件，并证明其他状态
全部保持不变；它不会生成执行命令或把未知结果解释为成功、失败。
重试成功证明要求当前尝试已明确失败、失败可重试、仍有额度、操作尚未
终结、新编号未使用且未到机器上限。真实整数运算被证明为编号加一、
额度减一；命令记录、当前尝试、索引和队列的更新全部纳入状态对应。
`handleEffectRetry_counterExhausted` 单独证明到达上限时拒绝而不发生回绕；
它是有限机器的额外拒绝，不被伪装成无界模型中的相同结果。
这些前提必须随定理一起阅读，不能省略后声称任意损坏状态也正确。
回执证明检查所有十一种命令的成功证据；只有当前尝试成功才更新逻辑
动作的最终结果。相同回执重放保持完整状态不变，冲突回执被拒绝。
`generated_effectSucceeded_refines` 证明发布所用的成功检查只读取当前
尝试，不能由旧尝试的成功代替。四类发布更新证明需要对应存储容量；
“运行中”更新还需要网络槽已列入所属沙箱的资源清单。这是已有资源的
索引条件，可由资源索引完整性及资源查找结果推出，不是对任意损坏索引
的保证。更新动作证明不等同于整个发布入口的证明。
资源关闭与停止回执检查已覆盖真实向量遍历：资源查找缺失、资源尚未
关闭、停止回执不匹配都会得到与模型相同的结果。停止回执按沙箱、资源
代数和虚拟机身份匹配，并读取该命令的当前尝试。遍历终止由剩余元素数
严格减少证明；这不是整个操作在公平调度下最终完成的证明。
控制器崩溃/恢复在这里表示纯状态转换，不是数据库落盘和真实进程重启。

统一结果关系 `ControllerResultRep` 比较拒绝原因，或成功事件和完整后继
状态。`controllerResultRep_accepted_is_legal` 与
`controllerResultRep_accepted_preserves_safe` 负责把已取得的对应关系
传递为合法模型步骤及安全性质；它们仍需要具体输入的对应证明。

### 已覆盖入口的连续组合

`CoveredExecution.lean` 明确枚举全部请求、请求重放、查询、访问、客户端等待超时、
命令派发、命令超时、回执、重试、最终失败、四类稳定发布、控制器运行状态、全部调度及全部工作命令，共十七组已完整证明的输入。
其中“全部请求”含五类生命周期请求和重放、查询；后两者保留独立入口，
因此十七组并不是十七类互不重叠的业务操作，更不是完成百分比。调度组含八类输入，工作命令组合并此前六个临时分组并覆盖全部十一种命令。
`handleCovered_refines` 将它们组合成一个统一对应定理；权限形状、资源
索引和命令索引条件由模型可达性推出，不是交给调用者的新证明义务。

`covered_execution_refines` 和 `covered_execution_preserves_safe` 进一步
证明：从一个对应于可达模型状态的 Rust 状态出发，这些输入任意有限次
混合执行，接受事件形成合法模型轨迹，最终状态仍对应且安全。拒绝时
保留输入状态；这只是纯处理核心的调用规则，不是已实现的服务主循环。

这里仍要求每次调用满足 `HandlerMachineHeadroom` 和输入所需的额外
容量，没有证明容量永不耗尽；这一限制没有因为完成全部命令覆盖而消失。
调度输入的 `SchedulerCapacity` 区分：六类命令安排最多新增一条命令，
暂停快照预留不入队，启动资源预留额外要求所查询计划的停止列表和关闭列表可追加。
两类预留还使用共享的存储、分配编号容量条件，不把机器耗尽伪装成无界模型拒绝。
四类发布入口不入队命令，不另加命令容量要求。
文件策略工作命令不入队新命令，只需策略存储空间。
两类回收同样不入队新命令，使用既有 `HandlerMachineHeadroom` 中的资源表、
索引表和所查询索引列表容量；这些是共同充分条件，并未声称已经最小化。
成功更新同时核对资源状态和所属沙箱的资源索引，未忽略登记操作。
围栏和网络配置不新增命令，沿用共享存储与索引容量条件。
准备虚拟机另外要求 `WorkerPrepareCapacity`：最多三条新命令所需的容量，
以及资源索引表长度加二不超过机器上限。这是两次索引写入的保守充分界，
第二次实际上替换同一键；当前未证明只剩一个空位时同样可组合，不能将该界
说成运行代码的必要条件。它是新增入口明确列出的证明范围限制，不改变业务模型。
十一条工作命令的公开定理不要求调用者提供队列成员证明；真实 Rust 队列检查与证明级
`workerEnvelope` 对应，先判断旧尝试，再判断是否入队。该映射是证明中的
输入解释，不是新增部署代码，也不是把是否入队交给外部保证。
激活、冻结、封存和停止不新增命令；提交快照要求一条新停止命令的空间。
`WorkerCapacity` 对十一种命令分别表达这些条件。快照更新证明明确展示中间
状态，证明快照表及索引写入不消耗命令队列的余量，不假设修改后满足全部无关容量条件。

`ControllerComplete.lean` 用实际 Rust 公开输入类型，而不是手选子集，
给出 `handleController_refines`。其 `BoundedControllerExecution` 和
`controller_execution_preserves_safe` 覆盖全部现有输入的任意有限混合执行。
每一步仍要求显式机器容量，初态对应可达模型状态；不是容量耗尽、损坏状态、
真实 I/O 或无限执行最终完成的证明。

## 最终完成性：已完成与剩余

`WorkflowReceiptProgress.lean` 覆盖创建、暂停和恢复正常阶段的当前尝试有效回执；`WorkflowCleanupReceipt.lean` 覆盖销毁与失败补偿，包括停止回执有限扫描不倒退。成功和明确失败均可降低剩余工作量，两类模块已接到实际无限事件轨迹。旧尝试回执不算当前尝试进展，观察到回执不等于它必将到达。

仍未闭合：

1. `ReservationGuardsPersistUnderNoRankDrop`：等待期间预留条件与执行前提持续成立。
2. `WorkflowProtocolClosure.select`：区分可执行的控制器工作与等待外部响应，不能要求所有未完成状态都有立即可执行的控制器动作。
3. `WorkflowServiceProgress.residual`：消除剩余内部进展前提，并组合执行节点动作、当前尝试回执、重试耗尽与状态发布。
4. 所有已接受操作（包括被销毁取代的操作）的终局稳定，以及符合条件的非当前资源最终回收。

这些是内部证明义务，不能换个名称当作环境保证。公平调度不能代替外部响应；外部最终返回可以是失败，不保证操作最终成功。永久外部故障下不承诺无条件完成。

## 长期容量：当前实现与证明限制

显式完整容量配置已经接通逻辑收尾预留、有限物理位置、检查点、分批退休和请求/构建结果保留；见[模块合同](module-contracts.md)。这是实现事实与工程验收范围，不是新增形式化结论。

历史定理仍逐步要求 `HandlerMachineHeadroom`、`VecHeadroom` 等容量前提；没有证明新准入、维护、所有后续动作始终保持这些前提，也没有证明 SQLite 页/WAL/temp、辅助库、宿主登记及实际内存/磁盘的组合界。物理观察未知时不得释放占用；新请求不能消耗已接受工作的逻辑收尾额度。编号耗尽须拒绝且不回绕，不能把无界自然数结论直接用于机器整数。

## 结构维护风险

- 完成性证明应按进度量、具体动作、时间组合分层，避免继续向大文件追加特殊情况。
- 请求/调度按真实功能族逐步拆模块；拒绝顺序、完整后继状态、公开输入穷尽覆盖不能变弱。
- 通配公开导出和多个 `insert_unique` 实现应在实际功能修改时梳理；先确认顺序语义，再重新抽取与完整验收，不能仅为整理目录批量搬迁。
- 模型不依赖 Rust 抽取物；对应证明单向依赖模型。扩展仍使用显式输入枚举，不用开放插件绕过穷尽检查。

## 验收与新增服务

### Rust 来宾服务的新增局部核心（#311，抽取试验通过，未证明）

[sandproof-guest-core](../sandbox-guest-core/src/lib.rs) 新增命令订阅的身份/驻留/完成判定，以及单文件接收、短写、结束和失败计费。该版本的 Rust 定向测试、固定 SDK 协议样例及固定 Charon/Aeneas 抽取试验通过；未启动服务，未执行生成物的 Lean 编译或对应证明，历史控制核心定理不覆盖它。

可用 `bash scripts/try-guest-extraction.sh` 重现：直接抽取两个类型的 14 个选定方法，包括初始化、输出/写入、结束与失败，产物及前后源码指纹放隔离 artifacts，不修改正式控制器生成物。[清单检查](../tools/check_guest_extraction.py)逐项核对实际方法名、局部性和透明函数体，拒绝空抽取、缺项及不透明自有依赖。当前还有一个标准库 `Option::is_some_and` 不透明依赖；它的外部定义模板未导入正式库，不能当成已成立的公理。试验通过只表示选定 Rust 主体能生成 Lean 表达。

后续仍需证明驻留计数与实际缓冲/释放的连接、原进程/订阅身份、退出与 EOF/排空的先后、部分写入与失败结果，并落实标准库依赖的定义/连接；端口观察来源、公开 PID 歧义、真实分配、认证和协议/系统调用适配均不能整体假定正确。自有代码的实际证明纳入 #101，固定 envd 外部合同在现役模板被正式替换前仍保留。

新 [Linux 进程适配](../sandbox-guest-runtime/src/process.rs)已把原 `CommandStream` 接到真实 wait、管道与消费确认，并实现单次私有 worker 放行及 pidfd 控制。该适配及 [worker](../sandbox-guest-runtime/src/worker.rs)只有组件测试，未抽取或证明；纯核心源码未变，原抽取试验不能背书异步与系统调用。待证连接包括：容量/公开身份先于放行、取消后的原进程责任、私有协议未知结果、真实 PID/创建证据与稳定句柄、stdin 部分效果、管道消费与实际分配/释放。内核、Tokio 与 exec 外部前提须逐项登记；自有状态和协议逻辑不能整体列为可信假设。

新增的[命令 JSON/Connect 编解码](../sandbox-guest-runtime/src/protocol.rs)只有固定 SDK 样例、畸形输入及真实进程组合测试，未抽取或证明。请求存在性/选择器、帧边界、字节保真、预算到实际分配、End 到真实观察及传输完成的连接均为 #101 后续义务；Serde/base64/HTTP 库的使用不使自有适配逻辑自动成为可信前提。原纯核心和已证明控制器源码未变，旧证明不覆盖该新增模块。

新增的[初始化、鉴权与 MMDS 组件](../sandbox-guest-service/src/lib.rs)只有会话及真实 HTTP/TCP 组件测试，未抽取或证明。#101 待证义务包括：元数据来源与实例绑定、令牌/defaults 原子发布、许可持有与实际派发的顺序、取消/超时的未知边界、重复初始化收敛、正文准入与实际资源释放。SHA-512、常量时间库、HTTP/异步库及内核是逐项外部前提，不覆盖自有头部判定、元数据解析、锁/预算与连接收尾。当前还未接到正式命令/文件路由；原已证明 Rust 与纯核心源码未变，已有定理或抽取记录不能背书此模块。

来宾[账户解析](../sandbox-guest-runtime/src/accounts.rs)、[授权上下文组合](../sandbox-guest-service/src/execution.rs)和 worker 的数值身份切换是新增自有逻辑，尚未抽取或证明。待证义务包括账户记录到数值身份、环境与身份的分离、原许可到真实派发、保存 ID/附加组设置次序、部分切换失败不得 exec、取消后实际阻塞工作与额度的对应。当前只增加组件验收入口，不把 Linux setresuid/setresgid、文件解析或线程池整段当作自有代码已正确的假设；外部内核合同和单线程 worker 前提须单列。纯核心和历史控制器 Rust 未改，旧定理仍不覆盖新代码。

新增[命令所有权、执行期限与订阅](../sandbox-guest-service/src/command.rs)及 Linux 端口的独立控制句柄/失败排空路径尚未抽取或证明。#101 待证连接包括：初始订阅先于放行、授权到派发、原绝对期限与 pidfd、任务/请求取消的区分、未知结果不重执行、输出与终局顺序、慢订阅拒绝、实际持有块与字节/订阅/记录额度的共同释放、任务丢失后的未决责任和容量保留。只增加真实进程组件证据，未改变已证明控制器 Rust 或纯来宾核心，原抽取和定理不覆盖新异步编排；公开 PID/tag 和持久终局保留仍是待接入义务。

[准备对象](../sandbox-guest-runtime/src/process/preparation.rs)及[准备管理任务](../sandbox-guest-service/src/command/preparation/owner.rs)进一步接通取消、实际 wait 到原额度归还，以及原身份登记确认到执行所有权移交；只有组件测试，未抽取或证明。同步 spawn 到持有 child、借用握手取消、不可重复放行、期限/确认竞争、未知清理保留、任务丢失隔离和跨任务原额度移交都是新增自有义务，不能整体列为外部前提。Linux 进程句柄/wait、Tokio 取消和文件系统操作仍须逐项外部合同；测试中的正常目录回收不证明故障文件系统清理。原已证明 Rust 与纯核心未变，历史结论不覆盖这批编排。

来宾 [PTY 内核端口](../sandbox-guest-runtime/src/pty.rs)、worker session/controlling terminal 设置、原身份 resize 和双输入通道连接只有工程测试，尚未抽取或证明。原纯来宾核心的 `OutputMode::Pty` 未改，但旧抽取不能证明新 master/slave FD 所有权、异步就绪与挂断判定、EIO/EOF 到真实 wait 的连接、取消部分效果或数值身份切换顺序。初始/更新尺寸的固定 uint16 转换和实际终端 line discipline 也须在 #101 区分自有判定与外部内核前提；组件及固定 SDK 回调通过不代表正式来宾服务或系统证明通过。

文件[一元编解码](../sandbox-guest-runtime/src/file_protocol.rs)及[授权路径解析](../sandbox-guest-service/src/files.rs)尚未抽取或证明。共享 JSON 解析/有界输出和 Basic/路径辅助函数供命令、文件两端复用，工程回归核对原命令行为；纯来宾核心未修改。字段缺省/精度、编码预算、账号选择、路径展开、授权快照到实际 I/O 的连接均须在 #101 后续证明，固定 SDK 与合成属性通过不证明真实文件操作、DAC、整个服务或系统正确。

新增[真实一元文件端口](../sandbox-guest-runtime/src/filesystem/mod.rs)、账户显示反查及[阻塞文件所有权](../sandbox-guest-service/src/file_io.rs)只有工程回归与固定参考对照，尚未抽取或证明。链接解析与实际 inode/FD、Go 属性对应、遍历/暂存/结果预算、非原子副作用与错误、原授权到派发/完成、取消后容量及结果保留均是新增自有义务，不整体列为外部前提。系统调用、目录变更、xattr、线程池和序列化的实际连接另须最小外部合同。纯来宾核心和历史已证明 Rust 未改，旧定理不覆盖这些新增实现；组件对照也不证明 POSIX/ACL 隔离或整套部署系统。

新增[流式文件 I/O](../sandbox-guest-runtime/src/filesystem/transfer.rs)和[传输所有权](../sandbox-guest-service/src/file_transfer.rs)已有单文件 Linux 组件及原 SDK 测试路由检查，未抽取或证明。原 `Transfer` 的输入/实际写入计费被接到真实 FD，但纯来宾核心源码未改，原抽取试验不覆盖这个连接。待证义务包括短写量到真实副作用、完整输入/元数据到成功、取消/期限与晚到回复、块释放到背压/容量、原授权到实际阻塞工作结束，以及打开后 FD 与可变路径的区别。xattr、内核读写、异步通道/取消和内存分配须逐项最小外部合同；历史控制器定理不能背书新增适配或整个 HTTP 服务。

[顺序批量编排](../sandbox-guest-service/src/file_transfer/batch.rs)和一次账户/defaults 快照解析另增加请求级授权、容量及期限共享、排他 part 借用、路径判重、累计正文/结果预算、失败后不可继续和正常结束证据的自有连接。实现已有 Linux 批量回归，尚未证明；单文件测试或原纯核心结论不覆盖该组合，HTTP/MIME 解析及正式服务仍是后续义务。

[增量 multipart/MIME 解码](../sandbox-guest-runtime/src/multipart/mod.rs)已有本地可移植测试和固定 Go 标准库对照，未抽取或证明。新增义务包括跨块边界与字节前缀、原始/解码计费、UTF-8/续段参数、错误后不可复用以及有效结束到批次成功的连接。明确拒绝 Go 可误作 EOF 成功的 MIME 头截断，不能沿用纯 `Transfer` 或历史控制器定理覆盖解析器。其与 HTTP 传输、账户授权、实际文件副作用及请求总预算的组合见下项，尚未证明。

[multipart 到上传批次的连接](../sandbox-guest-service/src/file_transfer/batch/multipart.rs)已实现，真实文件/固定 SDK 组合已有原提交 Linux 组件证据，后续候选仍须重跑。它增加原授权/容量/期限跨 parser、HTTP frame 和实际 I/O 的所有权链；End 与 HTTP EOF/长度的联合判据、QP 错误前缀、未知字段/尾部预算以及取消后的资源保留均是新的自有义务。该组合未抽取或证明，之前解码器/单文件/批次测试和 `Transfer` 抽取不能代为背书；HTTP 库的先行分配和响应序列化/发送仍须最小外部合同及独立计费。

[gzip 编解码组件](../sandbox-guest-runtime/src/gzip/mod.rs)新增头部预检、DEFLATE/CRC32 外部库连接、逐段结束与真实源 EOF、编码/展开预算及失败前缀义务。已有本地组件和固定 Go 对照，现已连接下述 HTTP/文件批次，未抽取或证明；`flate2`/`zlib-rs`/`crc32fast` 及调用者缓冲/期限/授权所有权仍须外部合同和组合连接。原纯来宾核心及历史已证明 Rust 未改变，旧定理不覆盖本模块或依赖实现。

[HTTP gzip worker](../sandbox-guest-service/src/upload_body/mod.rs)与[文件连接](../sandbox-guest-service/src/file_transfer/batch/gzip.rs)复用原批次，新增压缩头到文件打开的先后关系、压缩长度/展开预算、阻塞解压/实际文件两个任务的原授权守卫、Bytes 最后副本释放到下一读取、取消后的实际退出以及 raw/multipart 不同 metadata 完成点。共享 HTTP 帧校验重构和该所有权连接均未抽取/证明；可移植测试已有本地运行，Linux 授权/文件和合成压缩 POST＋固定 SDK 回读待验证，不能用组件测试或纯 `Transfer` 定理代替组合义务。

[gzip 下载连接](../sandbox-guest-service/src/file_transfer/gzip_download.rs)和[阻塞编码连接](../sandbox-guest-service/src/upload_body/encode.rs)增加同一原授权/容量到实际 open/read/销毁、实际 EOF 而非初始长度、源缓冲与编解码预算、响应 Bytes 最后副本释放及错误不伪装成功的义务。与上传共用的 worker/取消重构及 FD 打开辅助也未抽取或证明。原纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不背书这些连接；可移植测试与 Linux 交叉检查不等于 Linux 组合或系统证明通过。

[内容编码选择](../sandbox-guest-runtime/src/file_encoding/mod.rs)和[HTTP 字段连接](../sandbox-guest-service/src/file_encoding.rs)增加原字段字节/重复值、Go 浮点及排序、预算分配和条件回退/Vary 阶段义务，尚未抽取或证明。固定 Go 函数的有限对照是工程证据，不证明解析器对全部输入等价，也不证明请求鉴权/stat/open 顺序或完整 HTTP 服务。纯来宾核心、历史 Rust 与 LLBC/Lean 未改；所移植 Go 排序/浮点辅助和 Rust 十进制转换仍需对应及最小库合同。

[下载条件/范围](../sandbox-guest-runtime/src/file_download/mod.rs)与[响应计划](../sandbox-guest-service/src/file_download.rs)增加日期/时区转换、条件优先级、区间整数边界、MIME 线长、头变换和阶段执行义务，尚未抽取或证明。固定 Go 日期/ServeContent 的有限原函数对照只验证所列样例；计划中的文件区间不是已完成读取的证据，seek/read 错误、原授权/容量及响应发送的组合另由执行器承担，仍待完整验收。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不背书该计划器或第三方 `time` 库。

[范围正文执行器](../sandbox-guest-service/src/file_download/stream.rs)、[Linux FD 连接](../sandbox-guest-service/src/file_transfer/identity_download.rs)及[共用阻塞正文任务](../sandbox-guest-service/src/owned_body.rs)新增条件提前结束、准备期/正文期 seek、按计划读取与短读错误、MIME 分段、HEAD 抑制、原期限及最后字节副本到容量释放的义务。共用任务从 gzip 所有权代码抽出，旧路径回归通过仅属工程证据；Linux mtime/同一 FD/原 AuthPermit 组合尚待实跑。这些模块未抽取或证明，不能用纯 `Transfer` 或原历史版本定理覆盖新增执行连接；纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改。

[下载 MIME](../sandbox-guest-runtime/src/file_mime/mod.rs)新增不可变数据库来源/优先级、Unicode 查表、原始 filename 编码、512 字节签名顺序及配置计费义务；[identity 执行](../sandbox-guest-service/src/file_download/stream.rs)新增条件先于嗅探、忽略读取错误但保留前缀、回绕失败与阻塞取消期间原守卫保留的组合义务。有限固定 Go 对照及本地组件通过只属工程证据，Linux 固定路径/原 FD 和模板来源验证尚待实跑。这些模块未抽取或证明；现有 multipart 参数解析仅开放 crate 内复用，算法未改，纯来宾核心、历史已证明 Rust 与 LLBC/Lean 均未改，旧定理不覆盖新增路径。

[协商下载](../sandbox-guest-service/src/file_download/stream/negotiated.rs)与[Linux 延迟打开](../sandbox-guest-runtime/src/filesystem/transfer.rs)新增 stat/选择/open 拒绝顺序、旧 metadata 与后续路径变化、单 worker 身份及期限、Vary 阶段、准备期错误响应和 gzip HEAD 消费/发送时机义务。可移植有限故障对照不代表真实 Linux 或完整端点验收；这些新增及重构代码均未抽取或证明。纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新的文件连接。

[下载错误格式化](../sandbox-guest-service/src/file_download/stream/errors.rs)及结构化 Linux errno 新增操作阶段/状态对应、可信解析路径、Go JSON 转义与精确线长、初始范围失败前头部回退，以及错误正文继续持有原授权/容量的义务。有限原 Go 受控错误对照只属工程证据；Linux 真实 errno 与 NUL 适配回归尚待实跑。修改的来宾 runtime/service 均未抽取或证明，纯来宾核心、历史已证明 Rust 和 LLBC/Lean 未改，旧定理不覆盖新增错误响应。

[上传请求连接](../sandbox-guest-service/src/file_transfer/upload.rs)、[可移植头决策](../sandbox-guest-runtime/src/file_upload.rs)及账户池原期限/所有权重构新增编码预检与账户/metadata/类型的拒绝顺序、单次准入、阻塞查询取消后守卫保持和 raw 实际 EOF 的组合义务。共享 metadata 验证已替换真实写入中的相同校验，MIME 大小写辅助仅开放 crate 内复用；这些来宾实现均未抽取或证明。有限固定 Go 头部对照只属工程证据，Linux 文件/账户取消组合待实跑。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不覆盖本候选的新增连接。

[上传成功回执](../sandbox-guest-service/src/upload_response.rs)及原 BatchReply 的响应连接新增完成文件效果与响应交付的分离、原槽位/期限延续、最后字节副本释放、精确编码预算和 Go HTTP 成功头对应义务。下载错误转义/计数抽为共用内部函数，UploadResult 只迁移到可移植类型并保留旧路径重导出；原错误对照须一起重跑。这些来宾实现均未抽取或证明，固定 Go 回环 HTTP 只属有限工程证据；Linux 文件池/SDK 连接待实跑。纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新回执连接。

[上传详细错误](../sandbox-guest-runtime/src/filesystem/transfer/upload.rs)及父目录准备重构新增实际操作阶段/路径/errno/xattr 名的准确记录、粗分类兼容、部分写入计数及失败后不重放的连接义务。原输入/预算拒绝仍独立，固定 processFile 的有限故障状态对照只属工程证据，Linux 真实路径与 FD 错误待实跑；文件 I/O 错误响应现接下述组件，其余拒绝映射仍待补。这些来宾模块未抽取或证明，纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不覆盖本候选。

[上传错误发送](../sandbox-guest-service/src/upload_response/errors.rs)及请求入口新增实际故障经原 worker/批次传递、阶段/errno/属性名与固定 JSON 对应、原声明长度诊断、编码上界与原槽位/期限延续的义务。错误发送失败不回滚文件，实际 I/O 与响应保留分别持有原责任；这些组合均未抽取或证明，原 Go 受控错误和 HTTP 对照只属工程证据，Linux 文件/SDK 连接待实跑。纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖本候选。

### 当前串行实现的模块交接

#262 B8 的代码定位基线为 `beb46514cc1b99d1731616e9d5bd8f4dd143270a`。以下是实际实现的输入、输出和待证明连接，不是新增规约或已完成定理；后续 #101 证明批次必须重新冻结实际源码、抽取产物和工具链。本批保持原管理锁、单控制进程租约、派发顺序和 SQLite，不改产品行为。跨实例有限并行另列 [P2 待办 #284](https://github.com/ningZhang-cs/sandproof/issues/284)，不作为本次重构的前置条件。

| 局部边界与实际入口 | 输入、输出及当前责任 | 后续对应证明需要连接的事实 |
|---|---|---|
| 准入与持久决定：[admission.rs](../controller-service/src/controller/admission.rs) 的 `admit_create_with_options` / `admit_template_create`；[transaction.rs](../controller-service/src/controller/transaction.rs) 的 `WorldTransaction` | 输入原用户、创建参数、模板绑定、可信时间和容量限制；接受时保存请求、资源计划、寿命、预算及工作记录，拒绝/提交未知分别返回错误。`commit_with` 在调用提交前使缓存失效，仅确认提交成功后发布候选状态 | 核对 SQL、检查点＋输入尾、核心状态与 outbox 的表示关系；拒绝不产生外部工作，不确定提交不允许使用候选缓存或创建替代身份。编号高水位与业务接受分别核对，不能要求拒绝时所有持久表逐字不变 |
| 生命周期、寿命与撤销：[lifecycle.rs](../controller-service/src/application/coordinator/lifecycle.rs) 的 `create_locked` / `kill_locked` / `expire_due_report`；[admission.rs](../controller-service/src/controller/admission.rs) 的 `publish_running_at` / `set_lifetime` / `accept_expiry` | 创建接受后才绑定原物理位置、登记原凭据并执行；宿主就绪后采样运行起点。终止意图提交后撤销原数据通道，提交未知也保守撤销但不派发未确认动作；到期时重新读取时钟与寿命 | 原管理锁内持久决定、宿主派发、时间重读、撤销和返回结果的次序；接受之后的错误不能伪装成可安全重新创建的拒绝；终止后旧激活不得恢复 Running 或访问权限 |
| 工作领取与回执：[controller/mod.rs](../controller-service/src/controller/mod.rs) 的 `dispatch_next_for` / `claim_next` / `complete`；[workflow.rs](../controller-service/src/workflow.rs) 的 `drain` | `claim_next` 从原 outbox 选择已提交且核心预检允许的动作，持久领取后调用宿主；传输失败保留未知。`complete` 接收绑定动作身份的宿主回执，核对证据、重复/冲突、当前终止状态并原子应用资源更新与回执 | 派发提交先于副作用；回执不能先于领取；同一回执重放不重复推进，旧激活回执不可复活终止实例。底层 `Journal::dispatch_next` 只是通用记录原语，正式核心连接以 `ControllerJournal` 的资格检查和 `complete` 为准 |
| 恢复与失败隔离：[recovery.rs](../controller-service/src/application/coordinator/recovery.rs) 的 `recover_locked` / `matches_current_reclaim` / `bind_recovery_slots`；[controller/mod.rs](../controller-service/src/controller/mod.rs) 的 `matches_current_*` / `reconcile_observed` | 从原持久快照、未知动作、密封资源计划和精确宿主观察恢复；先核对当前操作/尝试/物理位置，再执行命令专属补偿或只读观察。单实例观察失败保留该实例未知，共享存储不可信则拒绝继续 | 恢复不重新派发未知 PrepareVm；可收敛的初始化/停止补偿只处理原身份，不等于任意重试安全；局部故障隔离不吞掉全局状态损坏。缺失绑定安装资格仍需原位置与前任身份核验，不是新建凭据的许可 |
| 宿主判断、执行与证据：[decisions.rs](../sandbox-node/src/linux/decisions.rs)、[identity.rs](../sandbox-node/src/linux/identity.rs) 的 `validate_action` / `require_fence` / `require_slot`；[execution.rs](../sandbox-node/src/linux/execution.rs) 的 `execute`；[observation.rs](../sandbox-node/src/linux/observation.rs) 的 `observe_action` | 值判断比较动作字段、围栏、位置坐标、进程身份及结果形状；节点另读取当前占用、归属和现场证据。只读观察不启动/停止/清理 VM；执行和命令专属恢复使用原身份 | 纯布尔比较不是执行证书；必须证明现场读取、判断与使用的顺序，及自有见证构造如何满足 `HostEvidence::validates`。不能将整个 NodeHost、观察器或恢复器作为外部正确假设 |
| 终局发布、清理与容量：[publication.rs](../controller-core/src/publication.rs) 的 `handle_publish_destroyed`；[workflow.rs](../controller-service/src/workflow.rs) 的 `finish_accepted_kill_v1` / `cleanup_destroyed_resources`；[host_slots.rs](../sandbox-node/src/host_slots.rs) 的 `release_host_slot`；[recovery.rs](../controller-service/src/application/coordinator/recovery.rs) 的 `release_reclaimed_slots` | 终止计划的可信停止/关闭回执满足核心前置条件后发布 Destroyed，随后继续物理清理。物理位置先持久记录 Releasing 和原退休清单，精确清理并观察确认后才可 Free；数据持有者未排空仍保留占用 | Destroyed、物理清理完成、位置 Free、逻辑退休及结果过期是不同事实。停止证据不能直接证明目录已删；未知不释放位置或预算，旧清理不能影响复用位置。已接受工作的收尾预留和真实字节边界仍需组合证明 |
| 数据权限与在途持有者：[data_access.rs](../controller-service/src/application/coordinator/data_access.rs) 的 `data_permit` / `spawn_data_relay`；[data_credentials.rs](../controller-service/src/application/coordinator/data_credentials.rs)；当前[中心网关](../sandbox-center-service/src/gateway.rs)及共享传输 | 原管理锁内查可信凭据索引、核对归属/寿命/VM/代数并登记撤销、排空及宿主闸门；真正启动中继前再次核对。中心只使用同次已确认全局视图，转发不自行授予宿主权限或拼接命令；B8 历史基线不覆盖新网关 | 许可不是永久授权；旧许可在撤销、到期或身份改变后不能启动连接。自有闸门登记、子进程回收、字节/协议结束和持有者排空须单独连接，不把 EOF 或 HTTP 成功当作物理退出证据 |

局部证明按上述责任建立前置条件、返回关系和后继状态关系，然后组合“创建已受理 → 结果未知 → 接受终止 → 精确停止/清理 → 发布终局”。组合必须包含工作调用资格、持久提交和物理观察之间的关系，不能只重新运行独立模型定理；模板构建/管理仍排除，但核心使用模板绑定与资源形状仍在范围内。有限执行安全对应不能直接推出无限轨迹的最终完成性；后者仍需要内部进度量、调度及外部响应条件。

### 固定工具链的局部抽取评估

以下评估绑定上述代码基线和[固定工具链](../verification/README.zh-CN.md)。真实 Rust 文件未改写，未升级工具；输出保存在仓库外的独立临时目录，没有覆盖正式 LLBC/Lean 产物。采用 `--preset=aeneas --error-on-warnings --sysroot default`，Cargo 使用 `--lib --locked --offline`；Aeneas 使用 `-backend lean -abort-on-error -warnings-as-errors -split-files -emit-json`。入口是 `charon cargo --start-from <实际函数> --dest-file <独立输出> -- --manifest-path <原 crate/Cargo.toml> --lib --locked --offline`，随后转换同一 LLBC，不手写一份替代 Rust 实现。

| 选定真实函数 | 已实测层次与阻碍 | 下一步，不是已完成证明 |
|---|---|---|
| `controller-core` 的 `crate::effect::handle_effect_receipt` 与 `crate::publication::handle_publish_destroyed` | Charon 成功；固定 Aeneas 生成两入口的 Lean 函数及依赖。生成器仍输出标准库外部函数的公理模板，未导入或提交它；未完成外部函数可计算连接、Lean 编译或与独立模型的对应证明 | 在证明批次核对新枚举/状态与标准库连接，重新抽取正式入口，补齐接受、按序拒绝和完整后继状态对应；临时生成成功不背书历史定理或整个服务 |
| `controller-service` 的 `crate::application::coordinator::recovery::can_install_host_slot_binding` | Charon 成功，Aeneas 在 `recovery.rs` 的 `record.terminal && record.phase == Revoking && record.current_vm.is_none()` 分支合并处报 `Could not match the contexts`。额外显式 `--include sandbox_controller_core` 展开自有核心依赖后仍在同处失败 | 后续以同一规则评估等价的局部 Rust 控制流整理，先保留所有边界回归，再重新抽取；不能将自有核心查询保持不透明或升级工具来冒充完整连接。本批只登记阻碍，不改恢复语义 |
| `sandbox-node` 的 `crate::linux::decisions::fence_can_advance` | 最初 macOS 原目标缺少真实 Linux 模块，跨目标又因 SQLite 的 C 编译器缺失阻断。R3 在原服务库分离共享宿主合同与默认 `durable` 实现；节点独立依赖树不再包含 SQLite。使用固定 Charon、原函数、`--targets x86_64-unknown-linux-gnu` 和原始 Linux cfg，已完成 LLBC 抽取及固定 Aeneas 的单函数 Lean 翻译；未复制算法，临时产物不进入正式生成目录 | 该函数的生成 Lean 尚未编译或完成对应定理；节点其他函数、自有见证核验、恢复与服务组合义务仍需逐项抽取验证。正式服务器仍启用同一持久化实现，不能将宿主端口、自有恢复或见证整体当作可信黑箱 |

本批是代码接口交接和抽取可行性评估，**没有新增 Lean 定理，也没有执行完整 `verify-controller.sh` 证明验收**。已有历史证明结论及开放义务不变；抽取成功、环境未就绪、翻译失败、局部证明与完整证明验收必须分别报告。

### 首条跨模块证明批次：待执行义务

R3/R5 的结构整理不新增定理：共享宿主合同只是原声明的唯一归属，`durable` 默认生产实现不变；见证构造仍是本项目代码而非新增假设。令牌 SHA-512 改用既有 Rust 库，摘要格式由回归及真实 SDK/KVM 验收检查，但该密码原语与授权逻辑的合同连接尚未证明。中继别名删除不改变原资源资格或排空算法。核心三处前插去重暂不合并，以免仅为小型复用改变历史抽取函数和证明接口；既有核心源码及证明结论不因此扩展到服务整理。

首批链路为“中心耐久创建 → 原节点就绪 → 丢回执 → 中心重启 → 原身份恢复 → 销毁及精确清理”。先完成 #292 合同、功能/故障/实机验收，再冻结同一真实提交和抽取物；本链尚未实现对应闭合证明。下列本地义务是链路中的必要部分，还须纳入上表 P1–P7 的中心/协议/确认连接；详细批次和闭合判据只在[证明工作流](proof-workflow.md#首条跨模块链路)维护。

- **类型与状态对应：**先核对当前 Rust 与生成 Lean 的差异，包括 IPv4、`AbortPrepare`、`RetireActivation`、`RetiredByStop`。补齐模型与抽取后的对应关系；缩小首批行为范围不等于可以遗漏新增枚举或降低整套检查。
- **持久接受与派发：**证明准入、编号/收尾预留、意图和 outbox 的事务组合；提交前无外部派发，拒绝保持持久状态，提交不确定不生成第二个身份。SQLite 的原子提交是外部原语合同，事务顺序和恢复决定是本项目需要验证的逻辑。
- **执行资格与真实观察：**证明当前操作、尝试、代数、终止围栏和密封资源计划的检查一致；未知不是无副作用失败；终止初始化须引用同一资源的可信停止回执，迟到回执不能恢复权限。Linux/pidfd 的物理观察准确性仍需明确合同和系统测试支持。
- **恢复、授权与收尾：**证明局部观察失败保留该实例的未知状态和预算，不阻止独立实例到期收尾；共享状态不可信则拒绝继续。覆盖重放、凭据恢复资格及网关撤销登记的次序，不把自有恢复循环、互斥锁和授权决定整体假定正确。
- **验收与剩余范围：**按证明工作流读取实时目标、局部试验、重新抽取并运行完整验收；分别报告本链的安全对应、仍需的外部合同，以及内部进展/长期容量义务。首批局部定理不等于全服务、无限运行或完整最终完成性证明。

`scripts/verify-controller.sh` 检查 Rust、重新抽取一致性、所有项目证明文件和未完成证明；`verification/ProofAudit.lean` 审查真实定理传递依赖，只允许 Lean 标准逻辑基础。编译文件数、抽取函数数、局部成功或 CI 绿灯都不是全输入证明覆盖。

`controller-service` 已接入纯控制核心的创建、销毁、状态发布与真实宿主回执；`sandbox-api`、`sandbox-node` 和 `sandbox-server` 新增持久寿命/配额、鉴权、网关、实际磁盘余量检查和实机测试。这些新增服务代码**没有** Rust→Lean 对应证明；现有定理只覆盖原先明确列出的纯核心输入，不自动扩展到 SQLite、时钟、令牌、Linux 网络空间、HTTP 转发或 Firecracker。特别是自有授权、持久化决策、worker 资格和恢复逻辑不能整体放入“适配器正确”假设；需要逐项列出外部原语合同并继续做对应证明。未知回执对账、重启恢复、完整容量保证和长期完成性仍是开放义务。
