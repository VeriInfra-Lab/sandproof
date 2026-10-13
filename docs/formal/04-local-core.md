# V4 本地纯控制：生命周期、工作账本与维护

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：详细证明计划；拟定定理尚待建立。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

这一模块只负责本机逻辑状态 K 的允许转换，不操作 SQLite、Linux 或套接字。它保留既有控制算法及历史证明，但必须补齐当前输入的模型/抽取差距；拆开后逻辑规则可以复用，真实证据由相邻模块提供。

## 1 实际入口与版本

| 代码 | 对应入口 |
|---|---|
| [controller.rs](../../controller-core/src/controller.rs) | `handle_controller` 与所有 ControllerInput 的分派 |
| [request.rs](../../controller-core/src/request.rs)、[terminal.rs](../../controller-core/src/terminal.rs) | 生命周期准入与终止 |
| [effect.rs](../../controller-core/src/effect.rs) | 派发、回执、未知、连续新尝试、最终失败 |
| [scheduler.rs](../../controller-core/src/scheduler.rs) | startup、prepare、stop、abort-prepare、精确回收的启用规则 |
| [worker.rs](../../controller-core/src/worker.rs)、[publication.rs](../../controller-core/src/publication.rs) | 逻辑资源更新与运行/销毁/失败发布 |
| [maintenance.rs](../../controller-core/src/maintenance.rs)、[checkpoint.rs](../../controller-core/src/checkpoint.rs)、[network.rs](../../controller-core/src/network.rs) | 安全退休、位置/窗口、检查点、静态 IPv4 判定 |

`World` 是本地逻辑状态。它不能替代中心状态、物理资源或全局网络历史。历史规范/生成文件及已证范围见[覆盖记录](../proof-coverage.md)，不是上述当前源码已全部对应。暂停/快照等历史类型保留但 V1 产品输入不得启用，需证明所选初态/输入的可达子集封闭。

## 2 模型与待证连接

下方 B3/B4/C1/C3/C4/C5/C9 保留完整字段、输入分派、守卫和拒绝定义。证明 `δk(K,input)` 是全函数，每一入口均有初态、成功后继、首个拒绝、修改范围及接受完备性，不只证明更新动作。

| 拟定定理 | 精确义务 |
|---|---|
| `local_product_subset_closed` | 当前初始化/允许输入不进入暂停、恢复、快照分支；不能删掉拒绝输入让定理假成立 |
| `local_step_invariant` | 接受/拒绝维持身份、计划、资源索引、尝试/终局、单调终止和预算一致 |
| `local_handlers_refine_complete` | 当前每种允许输入及其全部拒绝分支，实际生成 Lean 语义精化模型且合法可表示输入被接受 |
| `attempt_retry_contiguous` | 明确可重试失败时才建 current+1，预算扣一次；未知/超时/重发不建编号 |
| `publish_requires_facts` | 必要原尝试成功、计划/围栏/期限满足才发布；逻辑资源值不能代替物理证明 |
| `maintenance_preserves_replay` | 检查点、历史退休、保留结果、请求窗口与位置转换保存防重放及未决责任 |
| `local_enabled_progress` | 阶段、账本与有限预算使后续调度真实启用；其提供的内部推进不是环境公平假设 |

主要性质 S1/S3/S5/S7/S8/S14、F1；S4 的逻辑检查部分。物理成功回执由 V6→V5 提供；实际许可与当前期限由 V5/V8 提供。不得用 `PublishRunning` 输入自身当作就绪证据。

### 网络判断的局部接口

独立模型用自然数地址/前缀定义网段：前缀不超过 32、地址与网络号可表示，网络号低位为零且两者网络前缀相等。必须覆盖非规范网络号、`/0`、`/32` 和超长前缀，不把规范化作为隐藏前提。真实 `Ipv4Cidr::contains`、自有名单遍历和平台常量分别对应这一模型；名单循环的终止和下标加一不溢出也是自有义务。

`NetworkPolicy::destination_allowed` 只判断不可变目标策略，不读取 `World` 或授予当前实例权限。三个模式分别是全部拒绝、旧静态名单的完整 `Endpoint` 相等、IPv4 模式的严格解析和单地址检查。IPv4 顺序固定为平台禁止、用户允许、用户拒绝、默认联网值；平台禁止不可被该模式的用户允许覆盖。旧静态名单保留原完整字段匹配，不能把 IPv4 模式的禁止表定理推广到旧模式；产品子集/构造入口和宿主规则需另行连接。

字符串/地址及克隆库条件见[依赖登记](dependencies.md#a6-网络地址库模型使用登记)。入口 `handle_ingress` 先查记录、再检查租户归属或可信内部身份、再检查实例资格；出口 `handle_egress` 没有租户参数，先查记录、再检查实例资格、最后判断目标。实例资格精确比较生效授权的沙箱、代次、VM、网络位置和完整策略，不把顺序或重复项不同的名单视为同一策略。两入口成功时完整 World 不变，拒绝时没有核心后继状态。

这些读取函数不额外检查 phase、terminal、epoch 或 worker_fences。它们与有效授权的关系须由建立/撤销授权的可达状态不变量及服务调用绑定提供，不能从孤立读取函数推导任意 World 都有终止/围栏保护。出口调用身份绑定、SDK 规范化、策略安装与 V6 真实出口规则仍须连接；当前宿主本地地址排除不能从固定平台表推出。历史规范的入口授权忽略策略相等，只有在有效授权与记录策略一致的条件下才能沿用其结论。实际抽取与证明状态只记[覆盖记录](../proof-coverage.md)。

## 3 开工与可分批任务

第一批先审计实际输入与模型/生成入口集合，登记 NetworkPolicy::Ipv4、AbortPrepare、RetireActivation、维护及格式变化的差距；选创建/工作/运行发布/销毁子集复用版本相符的公共引理。任何依赖字段变更都需更新表示关系，而不是只让旧生成模块编译。

后续分别认领：（1）其余准入/拒绝及当前网络类型；（2）调度/工作/回执/稳定发布的完整产品子集；（3）维护/检查点/保留/复用；（4）内部启用与有限重试推进。按入口小批提交，避免大型展开宏把所有分支混在一起。

最小轨迹：未知prepare→观察/abort收尾、失败新尝试与迟到旧回执、清理延迟到位置被新身份使用。错误分支顺序直接使用现有测试，不新增无限故障矩阵。

依赖 V0；V5 给出调用/持久对应；V6 给出真实事实；V9 连接逻辑预算到系统责任。按 [V0](00-foundations.md#5-开工与完成条件) 与固定工具链交付，不以旧版局部证明背书当前 World。

## 精确条款

以下条款为本模块的规范定义；跨模块条款通过前提提供方连接，不将下游自有义务假定为已证。

### B3 规则核心的字段全集

下表对应 [World 与相关类型](../../controller-core/src/model.rs)及[维护状态](../../controller-core/src/maintenance.rs)。每个字段必须在表示关系中出现；不能只选最容易证明的子集。

| 字段组 | 内容 | 作用 |
|---|---|---|
| `sandboxes` | 所有者、阶段、终止标记、轮次、代次、当前虚拟机、工作区及访问模式、文件策略、网络位置与策略、期望和生效授权、当前操作；另有快照相关可选字段 | 实例身份与授权的主要记录 |
| `vms` | 所有者、沙箱、代次、虚拟机状态 | 逻辑虚拟机资源 |
| `filesystems` | 所有者、沙箱、创建代次 | 工作区归属 |
| `snapshots` | 所有者、来源代次、工作区、清单、阶段、不可变标记 | 既有快照模型；本功能子集证明不可达 |
| `net_slots` | 所有者、沙箱、代次、虚拟机、策略、已配置、已激活、已回收 | 网络资源及策略绑定 |
| `applied_file_policies` | 沙箱、代次、工作区、策略 | 文件策略已应用的逻辑事实，不自动等于宿主权限已正确设置 |
| `worker_fences` | 每个沙箱的控制轮次与终止标记 | 拒绝过期的启动或授权工作 |
| `operations` | 操作键、完整输入、状态与结果 | 请求重放和终局结果 |
| `operation_plans` | 操作键、轮次、代次、资源选择、停止与关闭集合、重试额度、封定标记 | 固定一次操作允许影响的资源 |
| `resource_indexes` | 每个沙箱的虚拟机、网络、快照索引 | 不能漏掉非当前资源的停止和清理 |
| `logical_effects` | 工作来源、当前尝试、剩余重试次数、终局结果 | 工作与重试总账 |
| `effect_keys`、`effects` | 尝试身份索引和各次尝试状态 | 防重复、拒绝错误回执 |
| 四个 `next_*` 计数器 | 虚拟机、网络位置、快照、清单编号 | 有限身份分配 |
| `inflight`、`controller_up` | 在途工作和控制器存活标记 | 派发及核心恢复模型 |
| `maintenance` | 编号高水位、保留结果、时间下界、容量限制、位置记录、准入窗口 | 有界历史和安全复用 |

`maintenance` 内的位置为 `Occupied / Releasing / Free`，即占用、释放中、可复用；`Free` 仍保留上次绑定。窗口记录包括窗口编号、租户、发出时间、到期时间和已发出编号上界。窗口本身不是资源准入的替代品。

### B4 核心状态取值

| 对象 | 全部主要取值 | 本功能子集的约束 |
|---|---|---|
| 沙箱阶段 | `Starting / Running / Pausing / Paused / Revoking / Destroyed` | 启动、运行、撤销、销毁；暂停相关状态不可达 |
| 虚拟机状态 | `Prepared / Executing / Frozen / Stopped / Reclaimed` | 准备、执行、停止、回收；冻结不可达 |
| 操作种类 | `Create / Pause / Resume / Kill` | 创建、销毁；连接和续期不等于创建新虚拟机 |
| 操作状态 | `Pending / OutcomeUnknown / Compensating(f) / Succeeded / Failed(f)` | 等待、未知、为失败收尾、成功、失败；失败是操作结果，不是新增沙箱运行阶段 |
| 授权 | `Closed / Running(binding) / Paused(source)` | 关闭或完整运行绑定；不能仅凭阶段名字授权 |
| 工作状态 | `Committed / Dispatched / OutcomeUnknown / Acknowledged(outcome) / RetiredByStop(coverage)` | 已提交、已派发、未知、已确认、被精确停止证据覆盖并退休 |
| 工作结果 | 成功摘要、明确失败、被停止覆盖退休 | 退休不是伪造一次启动成功或失败 |
| 失败种类 | `Cancelled / DeadlineExceeded / ResourceFailure / PolicyFailure / StorageFailure / InternalFailure` | 是否可重试由明确规则决定，不是所有失败都重试 |

### C1 真实核心输入不能漏项

[统一核心入口](../../controller-core/src/controller.rs)包含九组输入：

| 输入组 | 必须覆盖的内容 |
|---|---|
| `Request` | 创建、暂停、恢复、销毁、重放、查询、补偿；其中暂停、恢复按功能子集证明不可达 |
| `Effect` | 派发、回执、超时、重试、终结失败、用停止证据退休未知激活 |
| `Scheduler` | 启动资源预留、暂停快照预留、准备派发、封存、提交、停止、撤销准备、清理虚拟机、清理网络资源 |
| `Worker` | 执行一个完整尝试键对应的命令 |
| `Publication` | 发布运行、暂停、销毁、失败 |
| `RpcTimeout` | 客户端等待超时的核心语义；不能等同取消成功 |
| `Access` | 执行、文件读、文件写、入口访问、出口访问 |
| `Runtime` | 崩溃、恢复、重复或迟到投递 |
| `Maintenance` | 启用维护、退休、过期结果移除、位置管理、窗口管理 |

还需覆盖公开请求、中心准入/意图/额度、Redis结果、内部高级请求/映射、两端回执/确认、流与取消、各机时钟、SQLite、宿主观察/部分效果、进程退出、断线与独立恢复。Worker 或本地九组输入不能代替中心/通信输入；新增实际入口需逐项穷尽对照。

输出也要带类型，至少区分 `Rejected(reason)`、`Accepted(operation)`、`Pending(operation)`、`Unknown(operation)`、`Completed(result)`。具体接口可以有不同编码，但不能因为它们同用成功状态码就合并含义：续期成功表示期限已提交，不表示到期停机完成；后台命令已接受也不表示执行完成。

### C3 转换关系和拒绝

纯核心规则定义为全函数：

```text
δ(K, input) = Reject(error)
           或 Accept(event, K')
```

每条规则按顺序列出检查 `g1…gn`：第一个失败检查决定拒绝结果；全部通过才做更新。需要证明：

```text
全部检查通过 ⇔ 该规则被接受
拒绝 ⇒ 没有核心后继状态，也不产生宿主工作
接受 ⇒ 指定字段按规则更新，其他字段不变
```

“其他字段不变”叫修改范围条件。例如租户甲的读取请求不能顺便修改租户乙的操作账本。

这只适用于该次纯核心调用。服务为了持久记录到期观察，可能先提交一个时间推进事务，再拒绝后续准入；不能扩大为“每个失败的外部请求都不改任何状态”。

全系统一步 `Σ —a/o→ Σ'` 则包含控制器步、数据库步、宿主步、网络步和崩溃步。一个 Rust 处理函数可能对应多步，不能把整个处理函数假设成原子操作。

### C4 公共检查的精确定义

下列谓词展开为字段比较，而不是“状态合法”这样的占位条件：

```text
Own(u,s)           := sandboxes[s].owner = u
Current(s,e,g)     := sandboxes[s].epoch = e ∧ sandboxes[s].generation = g
Fence(s,e,t)       := worker_fences[s] = (e,t)
VmBound(s,g,v)     := vms[v].sandbox = s ∧ vms[v].generation = g
                     ∧ vms[v].owner = sandboxes[s].owner
SlotBound(s,g,v,n) := net_slots[n] 的 owner、sandbox、generation、vm
                     分别等于 sandboxes[s].owner、s、g、v
CurrentAttempt(x) := logical_effects[x.logical].current_attempt = x.attempt
ExactPlan(x,p)    := x 的完整命令参数与 p 中该动作的身份、策略、资源选择相等
```

再定义 `AttemptExecutable(x)`：存在该尝试记录、其逻辑工作未终结、该尝试是当前尝试，且尝试状态属于该入口允许的执行状态。派发、观察和应用成功各自使用独立的允许状态集合，不能统一替换为同一个“可重试”判断。

例如工作更新入口要求 `x ∈ inflight`，且 `effects[x]` 为 `Dispatched` 或 `OutcomeUnknown`；派发入口则接受 `Committed` 或 `OutcomeUnknown`。后者只是允许重新进入派发协议，不是对任意宿主副作用的重放许可。当前尝试与终局账本之间的关系还需由可达状态不变式连接，不能仅凭单个枚举值证明整体合法。

回执匹配使用完整尝试键，不仅是沙箱编号或结果摘要。旧代资源清理则使用其原绑定，不强制等于沙箱当前代次；否则旧资源会永远无法清理。

### C5 主要转换

| 规则 | 必要检查 | 状态更新与输出 |
|---|---|---|
| 接受创建 | 身份与参数通过；模板绑定有效；不存在冲突的原操作；标识和预算足够；未跨过退休高水位 | 同一事务建立启动状态、关闭授权、完整创建输入、固定资源计划、启动期限及清理预算；只创建原计划工作 |
| 原请求重放 | 操作键、种类和完整输入相同；调用者有权查询 | 返回原结果或原等待状态，不分配新虚拟机；参数不同明确冲突；超过保留期按退休合同处理 |
| 预留和入队 | 当前操作与计划一致；计数器未耗尽；总账和后续收尾额度足够 | 一次分配计划中的原资源身份，建立逻辑工作和初始尝试 |
| 派发 | 原工作已持久提交；当前尝试允许派发；执行前提未失效 | 登记在途身份，再调用宿主；不能先产生物理副作用再补账 |
| 接受回执 | 完整尝试键、来源、状态、结果与所需证据匹配 | 保存该尝试结果；同一终局结果不可被迟到回执改写；推进合法后续工作 |
| 明确失败后重试 | 失败类别允许重试；有重试余额；新尝试编号可表示 | 同一逻辑工作的新尝试编号增加，余额减少；不新建替代沙箱 |
| 超时或丢回执 | 尚无可信终局结果 | 保持原身份，记录结果未知；不因此证明动作没有发生 |
| 发布运行 | 非终止启动状态；计划、虚拟机、网络、文件策略和各项成功证据匹配；启动期限未到 | 同事务发布运行授权、操作成功结果、首次运行时间及寿命；返回该实例而非重建 |
| 接受销毁或到期 | 请求有权，或可信内部到期事件重新核对期限；结果重放规则成立 | 持久终止意图，关闭新访问授权，更新控制轮次和终止围栏计划，安排原资源停止 |
| 发布销毁 | 终止状态和围栏一致；资源索引中虚拟机已关闭；停止证据齐全；网络已关闭 | 发布终局销毁结果；移除当前运行引用；不据此直接声称所有磁盘已物理删除 |
| 创建失败收尾 | 原创建已进入补偿；错误种类和相关证据有效 | 保持终止与关闭授权，停止或撤销原准备工作；满足失败发布条件后保存失败结果 |
| 清理及释放 | 原对象身份一致；不再是当前资源；工作、连接、辅助进程和引用已排空 | 精确清理、保存释放见证、返还相应额度，最后允许位置复用 |
| 历史退休 | 操作终结；资源和引用满足退休条件；保留结果容量可用 | 用有界终局摘要和高水位替代活跃历史；不恢复旧身份的准入资格 |

上表是证明分解，不把整行视为一个真实原子步骤。数据库事务、宿主动作和回执分别建模；只有实际同事务提交的字段才能共享一个提交点。

### C9 完整规则示例 失败后的新尝试

以 `Retry(logical,failure)` 为例，令 `l` 为逻辑工作记录，`a = l.current_attempt`，`x = (logical,a)`。按顺序检查：

| 顺序 | 必须满足 | 否则拒绝 |
|---|---|---|
| 1 | 控制器存活 | `ControllerDown` |
| 2 | 存在逻辑工作 `l` | `LogicalEffectNotFound` |
| 3 | `l.result = None` | `LogicalEffectAlreadyFinal` |
| 4 | `effects[x] = Acknowledged(Failed(observed))` | `CurrentAttemptNotFailed` |
| 5 | `observed = failure` | `FailureMismatch` |
| 6 | `failure` 属于期限、资源、存储或内部失败 | `FailureNotRetryable` |
| 7 | `l.remaining_retries > 0` | `RetryBudgetExhausted` |
| 8 | `a < 2^64 - 1` | `CounterExhausted` |
| 9 | `effects[(logical,a+1)]` 不存在 | `NextAttemptExists` |

全部通过后，令 `y=(logical,a+1)`：

```text
logical_effects'[logical] = l[current_attempt := a+1,
                            remaining_retries := l.remaining_retries-1]
effects'[y] = Committed
effect_keys' = 将 y 不重复地加入原索引
inflight' = 将 y 不重复地加入原在途集合
输出 = StartNextAttempt(x,y)
```

原尝试 `x` 的失败记录保留；其他工作、资源、授权、计划和全局编号不变。拒绝时上述字段全部不变。

这条规则的证明至少包含：接受当且仅当九个检查通过；失败错误是最先失败项；新尝试与原逻辑命令相同但编号不同；重试额度恰好减少一次；不会把未知结果变成可重试失败；计数不回绕；保留状态不变式。

该示例对应[真实重试入口](../../controller-core/src/effect.rs)，但“与代码逐行相同”本身不是安全理由；还需独立证明这些检查足以维护附录 G性质。其余规则应以同样粒度形成定义与定理，而不是用“执行适当检查”代替。
