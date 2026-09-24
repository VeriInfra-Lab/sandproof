# 控制核心证明覆盖记录

更新日期：2026-09-22。

本轮整套检查通过：78 个 Rust 测试、静态检查、重新抽取一致性、所有
项目证明模块编译、146 项定理依赖审计和证明占位符扫描均通过。
完整日志：`/private/tmp/worker-final-five-review.log`，退出状态为 0。

本表记录真实 Rust 经 Charon/Aeneas 抽取后的函数，与 Lean 可执行状态机
之间已经编译通过的对应证明。它不是代码覆盖率，也不把模型侧自带的
合法步骤证明当作 Rust 实现已经得到证明。

## 当前可复核的覆盖

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

### 本轮模型纠正

激活分支原有缩进导致两类拒绝原因反转：身份不符返回“状态不符”，
尚未准备好返回“身份不符”。完整拒绝对应证明发现了该问题。本轮仅纠正
模型的拒绝标签和归属，不改任何接受条件、成功状态或 Rust 生产逻辑。
新增回归测试覆盖所有者、沙箱、代数三种身份不符，以及身份正确但状态
不对的情形；身份检查仍优先于状态检查。修正后完整分支证明通过。

## 当前阶段仍须完成

2026-09-22 后续第一批：`WorkflowReceiptProgress.lean` 已补齐创建、暂停、
恢复正常阶段的当前尝试回执推进证明，成功与明确失败均覆盖，并连接模型的
可执行处理函数。旧尝试回执不算作当前命令的进展；销毁、补偿阶段及等待期间
条件保持尚未包含。容量实现核查与剩余义务见 [本批复核](liveness-capacity-review.md)。
该批整套验收通过：78 项 Rust 测试、提取一致性、全部项目证明模块、151 项
实际定理依赖审计及其他既有检查；未改动 Rust 运行行为。

2026-09-23 后续批次：`WorkflowCleanupReceipt.lean` 已证明销毁和失败补偿阶段的
当前尝试有效回执使剩余工作量下降，包括有限停止回执扫描不会倒退；
`WorkflowReceiptProgress.lean` 与清理模块均已把实际回执事件接到无限执行轨迹。
两者共用当前回执与重复回执的区分引理。仍须证明等待期间的条件保持、
执行节点动作、工作项选择及最终完成性整体组合。结构审查与已处理的重复
引理、文档漂移见 [架构复核](architecture-review-2026-09-23.md)。
本批整套验收通过：78 项 Rust 测试、全部项目证明模块、提取一致性、159 项
实际定理依赖审计及占位符扫描；Rust 运行行为未改动。

- 有限机器的计数器、存储容量边界与长期执行假设的统一表述。
- 最终完成性中的内部条件：资源预留条件持续有效、工作项选择、剩余
  进展分支和所有非当前资源的最终回收。

`WorkflowLiveness.lean` 中的 `ReservationGuardsPersistUnderNoRankDrop`、
`WorkflowProtocolClosure.select`、`WorkflowServiceProgress.residual`
仍是内部证明义务，不能包装成外部系统必须提供的保证。

## 检查方式与限制

`scripts/verify-controller.sh` 执行 Rust 格式检查、测试、静态检查，重做
抽取并检查生成文件未过期，编译每个项目证明文件，扫描未完成证明和
自定义公理。`verification/ProofAudit.lean` 另外检查已列出的真实定理及其
传递依赖，只允许 Lean 的标准逻辑基础，防止依赖中的未证明假设绕过
源码扫描。本轮 Rust 测试增至 78 个，新增激活拒绝原因及顺序的回归测试。
本轮增量是剩余五条工作命令、十一条命令统一组合、实际控制器全输入对应
与有限执行安全证明；只纠正上述模型拒绝原因，没有新增生产功能。

检查脚本现在枚举所有项目证明文件，防止未被默认入口导入的文件逃过
编译。它仍不能通过“所有文件能编译”推断“所有输入已被证明”；上表
与剩余清单明确区分二者。本轮整体定理实际枚举公开输入类型；容量前提与最终完成性义务仍单独列出。

## 后续阶段

数据库、Redis、真实时钟和任务循环、CubeSandbox 请求适配、真实资源
重新核对、部署与运维，按照用户本轮决定列为后续工作。保留它们的
模型合同，不将它们计为已经实现或验证的生产组件。
