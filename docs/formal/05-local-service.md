# V5 本地持久服务：SQLite、寿命、编排与恢复

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：详细证明计划；拟定定理尚待建立。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

把 V4 的本地决定、期限、身份、额度和 outbox 放进同一提交，并在重启后安全地继续原操作。它与 V6 的分界是“何时允许执行和相信结果”与“实际如何产生/观察资源效果”；两者分开不等于宿主节点被拆成两套控制器。

## 1 实现与状态所有权

| 代码 | 入口/责任 |
|---|---|
| [journal.rs](../../controller-service/src/journal.rs)、[controller/mod.rs](../../controller-service/src/controller/mod.rs) | `ControllerJournal::apply/dispatch_next/complete/reconcile_observed`，原身份、投影、计划/outbox |
| [transaction.rs](../../controller-service/src/controller/transaction.rs)、[storage.rs](../../controller-service/src/controller/storage.rs) | WorldTransaction、CommittedWorld、事务顺序、SQL-only写入与缓存失效 |
| [checkpoint.rs](../../controller-service/src/controller/checkpoint.rs)及 [recovery](../../controller-service/src/controller/checkpoint/recovery.rs)、[retirement](../../controller-service/src/controller/checkpoint/retirement.rs) | 版本化检查点＋连续尾、切换、释放见证与历史退休 |
| [lifecycle.rs](../../controller-service/src/application/coordinator/lifecycle.rs)、[recovery.rs](../../controller-service/src/application/coordinator/recovery.rs)、[maintenance.rs](../../controller-service/src/application/coordinator/maintenance.rs) | 真实寿命、发布/销毁、原证据恢复、引用/位置结算 |
| [内部高级入口](../../controller-service/src/application/coordinator/internal_lifecycle.rs)、[driver.rs](../../sandbox-host-service/src/driver.rs) | V3 收件映射后原算法启用，原事实保存与提取 |
| [host.rs](../../controller-service/src/application/host.rs)、[relay_child.rs](../../controller-service/src/application/relay_child.rs) | 执行端口合同及真实子进程许可/排空的协调侧；实际观察归 V6，传输归 V8 |

拥有 `Ji=(已提交SQL、检查点/连续尾、核心缓存、派发记录、回执、寿命、凭据/路由投影、引用、事务和易失锁状态)`。V3 独立认领 inbox 的业务转换，但用本模块同一事务原语。二者的联合事务必须一次对应，不分成两个假原子提交。

## 2 事件与中间状态

决定计算 → 开事务/验证候选 → 写输入、完整投影/额度/期限/动作 → 提交 → 发布缓存 → claim原工作 → 执行/观察 → 保存回执与核心更新 → 重查期限/资格发布。每一箭头之间允许崩溃；事务内部按 SQLite 合同恢复提交前或提交后，不把 VM 纳入 SQLite 事务。

`CommitUnknown` 保留写前/写后可能，失效候选缓存并恢复原身份；只允许已确认提交派发。到期候选在事务内重查最新期限，SQL-only 写入同样参与失效。凭据读取失败按实例拒绝，真正共享存储损坏才全局拒绝；不存在/不可观察分别处理。

## 3 拟定定理

| 定理 | 义务 |
|---|---|
| `sqlite_world_representation` | Decode(有效检查点,连续已提交尾)=K；同一提交全部投影/动作/寿命一致，缓存仅来自相应提交 |
| `local_accept_before_effect` | 每个宿主效果有此前精确本地准入责任；claim/回执不丢计划，失败提交不派发 |
| `local_commit_unknown_recovery` | 任意声明崩溃点恢复某合法提交并保留可能外部效果，不新建替代实例或假失败 |
| `lifetime_refines` | 原发布起点、默认/拒绝/截顶、重放、续期/到期事务竞争与持久时间下界符合 D4 |
| `receipt_to_publication` | V6原事实进入正确尝试并耐久后，重查全部前提才发布运行或销毁；最终成功响应有相应耐久结果 |
| `checkpoint_retirement_refines` | 原/新完整表示之一可恢复，连续性/模式/高水位保留；记录移除不遗失未清理责任 |
| `local_orchestration_refines_progress` | 真实锁/扫描/driver/回调对应微步骤；已启用工作被取出，引用能按无环顺序解除，局部故障不无限阻断独立实例 |

主要 S4/S6/S8/S10/S13/S14，L1/L2局部提供方、DS3/DS5/DS7。V4 的安全输入判定只是其中一环；`ConfirmRelease` 需 V6/V8 实际见证，不能凭 SQL 行变成 Free。

## 4 抽取与批次

首批先抽取可顺序表示的计划/寿命/恢复决定，再建立真实 rusqlite 调用与事务微步骤的对应。数据库引擎为 A4，**自有SQL、编解码、事务范围和缓存逻辑不排除**。不支持的 async/锁封装写明确接口模型与调用路径证明；无法建立的连接保留未解，而非把整个 ControllerJournal 放入可信边界。

第一链路：原准入/提交→工作派发→启动丢回执→恢复原观察→稳定发布→终止/停止/精确清理。下面 F2 每个故障点都属于该链路，不必首批同时覆盖所有查询/文件接口。

扩展：完整寿命与查询、检查点/退休/完整容量、故障域与跨实例推进。复用 [事务测试](../../controller-service/src/controller/cache_tests.rs)、[检查点测试](../../controller-service/src/controller/checkpoint/tests.rs) 和 [系统故障入口](../../tests/system/center_host_faults.py)。

依赖 V0/V4、A4/A8；V3 提供认证原映射；V6 提供真实证据；V8 提供原连接/辅助执行排空；V9 以共同不变量连接，不互相假定对方已完成清理。开工/完成沿用 [V0](00-foundations.md#5-开工与完成条件)。

## 精确条款

以下条款为本模块的规范定义；跨模块条款通过前提提供方连接，不将下游自有义务假定为已证。

### B5 持久服务还需要哪些状态

`J` 至少包含：数据库已提交版本、持久核心表示、操作与工作日志、检查点及其后的日志、身份高水位、创建参数、不可变模板绑定、寿命记录、资源与未来清理预算、凭据索引、内存缓存所对应的提交版本、恢复阶段、物理释放见证和阻止退休的引用。

事务可处于 `Idle / Executing / CommitUnknown / Committed / Aborted`。`CommitUnknown` 是调用方不知道提交结果，不是数据库允许“提交一半”。外部动作不与 SQLite 共享原子事务。

### C7 未知结果不能用重试掩盖

只允许两种推进未知工作的办法：

1. 观察原资源，获得足以确认该工作后果的证据。
2. 对特定命令证明重复执行保持相同效果，且不会重复非幂等副作用，再重放同一身份的动作。

“幂等”必须是逐命令定理：执行两次与一次在允许观察上等价。它不能自动用于用户命令、追加文件或镜像中的启动程序。初始化连接可以重试，不表示其中的任意启动命令可以重跑。

未知激活在原虚拟机已被精确停止后，可以被 `RetiredByStop` 覆盖退休。这证明“这次未知激活不再能使实例运行”，不证明我们知道它历史上究竟有没有成功。

核心的工作结果检查使用逻辑资源状态。例如逻辑成功结果尚未出现，是某些失败回执的核心检查条件；它不自动证明物理副作用从未发生。真实适配器必须给出独立的明确失败证据；观察不到或回执丢失只能产生未知，不能借用逻辑记录缺失来伪造失败。

### D4 寿命和时间

寿命不是一个 `timeout` 字段，而是几种不同期限。令 `a` 为创建准入时间，`s` 为首次持久发布运行时间，`d` 为获批可用时长，`D_start` 为启动等待上限，`D_max` 为实例总寿命上限：

```text
启动截止 = a + D_start
首次运行发布要求：now < 启动截止
初始可用截止 = s + d
总截止 = s + D_max
```

创建和连接省略寿命使用固定合同的 300 秒；创建必须通过单次和总寿命上限，不静默缩短。连接只延长原实例，设置寿命允许缩短。对已经通过对应前置检查的请求：

```text
connect：deadline' = max(deadline, min(now + d, 总截止))
set_timeout：deadline' = min(now + 有符号时长, 总截止)
```

非正设置结果可立即到期；算术在足够宽的域中计算，转换回持久类型时检查范围。创建或连接的参数拒绝、已到期、正在撤销、外租户与不存在的错误区别由模块合同决定。

到期扫描读取的实例列表只是候选。真正接受到期销毁时，必须在事务内重新比较最新截止，避免续期后仍被旧扫描结果杀死。重启加载已经提交的期限，不根据新配置重新延长。

持久时间下界只防止逻辑时间倒退，不能证明物理时钟正确。若只假设逻辑时钟单调，能证明“观察到到期后不再授予许可”；若要证明“现实时间截止后最多 Δ 毫秒关闭”，还需要时钟误差、扫描延迟、调度及撤销耗时的上界。没有这些上界，就不能把条件最终完成性写成实时承诺。

### F1 提交点决定责任何时成立

本节本地 `Accepted_node(op)` 表示完整输入、计划、预算和期限的本地事务已提交；中心 `Accepted_center(op)` 由 C10 的中心耐久准入定义，两者不能合并为一个跨库提交点。原局部公式中 `Accepted(op)` 按本地责任解释；系统接受与节点责任连接见 F7。对任意宿主派发事件：

```text
Dispatch(x) ⇒ 之前存在 DurableCommitted(x, exact_plan)
```

对创建、销毁等需要持久终局结果的生命周期操作，其完成响应必须满足：

```text
SuccessReply(op,result) ⇒ 之前存在 DurableFinal(op,result)
```

这不要求为每次查询或命令输出建立终局账本。查询响应对应某个允许的已提交视图；续期成功对应期限提交；文件和命令结果遵守附录 E的数据与端点合同，不能把它们误归为控制器持久生命周期结果。

响应丢失不会撤销提交。提交返回错误但结果不明时，状态是“提交前或提交后，尚待核实”，不能立即分配第二组资源。

### F2 必须覆盖的崩溃位置

| 崩溃位置 | 恢复必须允许的真实情况 | 合法推进 |
|---|---|---|
| 意图事务提交前 | 没有接受该操作 | 不派发；按原身份检查是否实际已提交 |
| 提交后、派发前 | 有持久责任，没有物理效果 | 派发原尝试 |
| 宿主登记后、资源创建中 | 有部分目录、磁盘或网络 | 按原登记观察，继续可安全重放的步骤或精确清理 |
| 虚拟机已启动、回执未写入 | 实际运行，账本仍未知 | 观察原虚拟机，确认原结果或停止；不新建替代实例 |
| 回执已存、运行发布前 | 所需事实可能已有持久证据 | 重查发布前提和期限，发布原实例或进入补偿 |
| 终止提交后、停止前 | 新许可关闭，原进程可能仍运行 | 安装终止围栏，排空和停止原资源 |
| 已删除资源、释放记录未提交 | 物理已无，但额度仍占用 | 原身份范围内确认不存在，再提交释放；不误删同名新对象 |
| 检查点或退休处理中 | 旧表示或新表示之一仍有效 | 选择完整有效表示恢复，不拼接两个版本 |

### F3 数据库表示关系

定义：

```text
Decode(检查点, 连续的已提交日志尾部) = K
Cache.version = Commit.version ⇒ Cache.state = Decode(Commit)
```

需要证明日志顺序、缺口检查、模式版本、错误分支、检查点切换和退休保持这一关系。序列化只证明编码可读不够，还要证明它保存的是对应提交的完整状态。

重启不能混用新数据库核心与旧缓存凭据、旧宿主路线或重新解析后的模板别名。全局存储损坏可以导致拒绝启动；单个实例凭据读取失败应只拒绝该实例的数据授权，而不是让其他健康实例统一失效。
