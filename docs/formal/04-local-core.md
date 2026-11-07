# V4 本地生命周期状态与纯决定

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#636](https://github.com/VeriInfra-Lab/sandproof/issues/636)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 从旧核心到新合同

对应 [controller-core](../../controller-core/src/lib.rs)。原 Pause/Resume 核心已接入多代检查点、服务恢复准入和专用宿主动作；普通 Pause 的补偿保留原 Pausing、VM 和责任，失败继续沿原计划推进。正式链路及有限故障代表已有工程验收，准确版本与范围见[覆盖记录](../proof-coverage.md#current-status)；整体实现对应与全部故障交错证明仍未完成。旧逻辑 manifest 计数不能代替真实材料摘要，保留材料的局部性质也不单独证明失败后已恢复服务。

本模块拥有逻辑 `Session`、`Operation`、`Capture`、运行资格、预留和逻辑工作；V5 提供持久性，V6 提供物理效果。不把持久记录和真实 VMM 状态定义为同一对象。

## 2. 关键投影与不变量

- `Recoverable(c)`：对应会话/源代的完整不可变 bundle 已按合同提交且尚有合法恢复资格/责任；包含内容匹配与外部持久条件，不只是 `stage=Durable`。
- `MayHaveExecuted(i)`：在任何可能激活前先设的单调事实；可保守为真，不可由随后停止/paused/超时改回假。
- `SafeToReuse(slot,old)`：原 VMM/助手与相关工作不再能读写该运行位置或取得新资格；不是只有逻辑 phase 终止。
- `Tracked(effect)`：可能发生的效果有持久计划/身份/额度和恢复责任；V5/V6 分别提供存储/物理连接。
- `RestoreEligibility(c)` 与 `Pins(c)` 独立。资格只产生一个可能执行的后继；资格消费不能隐含清空文件引用。
- `SuccessorAdopted(a,b)`：B 的完整材料、持久登记及当前来源切换已经确认，且精确连接 A 的原恢复后继；暂停失败或提交未知不能建立它。后继继续演进不撤销这项历史事实。
- `MaterialUseEnded(a)`：原 loader、activation/运行候选、capture worker、私有 backing、其它引用和原位置释放均有具名结束依据；V5/V6 提供各自真实连接，不能由当前终止证书或路径缺席推导。

## 3. 必须证明的转换

| 转换 | 规则与错误边界 |
|---|---|
| 接受暂停 | 原 Running、当前版本、关闭新授权、预留捕获与收尾；不能先释放原运行额度 |
| 捕获/提交/卸载 | 同一次冻结截面、指定目标、完整不可变来源先接管，旧 VMM 后退出；真实释放后才 Unloaded |
| 暂停失败 | 普通失败保住唯一原 live/frozen 状态；撤销新 capture 资格且迟到任务失效后才回原 Running；提交未知先对账 |
| 接受恢复 | Unloaded、原 VMM 已退出、当前来源/寿命、唯一 claim 和材料 pin；新运行代不等于新逻辑会话 |
| 装载/激活 | LoadPaused 后先记 MayHaveExecuted 再激活；标记后只继续原候选，不回退重载；原 VMM 非预期死亡报告不可恢复 |
| 发布 | 精确原对象 Ready 与当前资格同时成立；销毁/到期否决新成功发布，不抹掉在途责任 |
| 终止 | 单调撤销未来执行/访问并收尾；显式 destroy 可放弃状态，但不能把普通暂停错误伪装为用户终止 |
| 材料退休 | 终止或后继持久接管两原因沿同一 Held→Retiring→Settled 流程；全部原使用结束后，与 pin 获取同原子决定永久关闭新引用并保存固定计划；旧资格不可复活，未知不退额，后继再演进不使原计划失效 |
| 预算转换 | 运行、保留材料、临时重叠、操作结果及收尾逐分量保持；未知不能释放 |

具体 API 行为唯一以模块合同为准。本模型不重写文件存储或调度器，也不假设所有动作一次成功。

后继回收切片须同时证明安全拒绝与合法接受：未确认后继、任一未决使用或错误身份不能退休；完整接管且全部使用闭合的旧代可以进入退休。Retiring 的原计划在后继保存、续期、管理重启及重复/迟到回执下保持身份和结算唯一性，旧 Resume 仅按原重放规则返回历史结果。该切片不自动加入下述 CaptureHandoff 值方法或旧 Step 的已证闭包；若改变其字段或转换，另行重新抽取和检查受影响引理。

## 4. Rust 对应和反例

对实际新增/重构的决定、回调与补偿生成 Lean，建立字段完整 `R_local`；初态非空、每步保持不变量、拒绝/错误路径保持责任、支持域正常接受。旧 `local_product_subset_closed` 排除 Pause/Resume 的结论不能沿用到新范围；旧定理可按前提重用，覆盖记录必须换精确候选。

至少让三个真实错误版本被同一性质拒绝：暂停捕获失败杀原 VM；激活未知后重载旧 capture；GC 检查后不受保护地删除。负例必须编译且能走到错误，不以抽取失败为安全证明。V4 负责 LC-P1/2/6/7/8/9/10/11 的纯决定，V5/V6 接真实效果，V9 证明全历史。

### 普通暂停登记与 Stop 的函数边界

实际 [worker 入口](../../controller-core/src/worker.rs) 唯一转发到
[`worker::pause_stop::record`](../../controller-core/src/worker/pause_stop.rs)，
不复制一套业务算法。函数正常返回时，`CommitSnapshot`、`StopVm` 由这条路径处理，
其它命令返回 `None`，继续原 worker 分派。机器对应目标是 `record`、`execute`、
`commit_snapshot`、`stop_vm` 的真实生成体及其实际依赖；外层唯一转发和未改路径
另经源码审查、原生连接回归核对，不宣称已机器证明整个 worker 的一般等价。
组件规则与入口见[普通暂停登记与 Stop](../../verification/experiments/pause-stop/README.md)。

该切片需要分别检查以下合同，不以“能成功翻译”替代业务证明：

- 保持原首错顺序：当前尝试、在途队列、`Dispatched` 或 `OutcomeUnknown`，之后才处理迟到捕获和具体命令。缺少逻辑效果行时仍按原实现使用第 0 次尝试；终止后的旧 Commit 保留原迟到处理优先级。
- 普通 Commit 核对实际代码要求的快照、sandbox、epoch、generation、workspace、阶段和完整目标，登记原计划的材料责任并更新 Durable/immutable。只有交接允许停止原 VM 才排入完整身份的 Stop；此时不要求 Commit 成功回执已经到达。
- 登记前已撤销的迟到 Commit 只登记收到的提交观察所指的材料责任，不恢复资格、不新排 Stop。登记后发生普通取消则保持原有效交接，不能一概拒绝后续 Stop。
- 普通 Pausing Stop 必须核对原 VM、状态、非终止记录与栅栏、当前计划、有效交接，以及目标快照身份和 Durable/immutable 字段。Revoking Stop 沿原终止分支使用终止栅栏，不额外要求普通暂停材料。
- 成功事件和完整状态更新与真实函数一致：Commit 只按实际写入改变计划、快照、资源索引，以及条件排队涉及的效果表、键列表、逻辑效果表和在途队列；Stop 只更新 VM 表和资源索引，其余状态保持。正常接受所需的输入和容器容量条件须明确，并给出可满足见证。
- 正错实现使用同一许可性质。错误版只忽略 `may_stop_original` 的布尔结果，保留交接查询、None 拒绝及其它守卫；反例须使其它 Stop 条件全部满足，不能依赖另一道快照检查碰巧拒绝。

分支之间的检查不能混写：普通 Commit 没有单独检查快照 owner 或记录非终止，
后续普通 Stop 才检查这两项，不能由 Commit 成功单独推出全部 Stop 前提。
Stop 明确检查 command.generation 等于 VM.generation；它没有直接比较
VM.generation 与 record.generation，也不额外核对 VM.owner 与记录 owner，
或逻辑效果 origin 与当前操作的全局关系。这些额外关系由实际来源、状态保持或 V9 不变量提供。
Commit 构造的 Stop 携带记录的 generation，组合时须显式连接这一事实。
效果登记使用初始尝试 0，并保留已有逻辑效果行的当前尝试号；不能仅从 Commit 成功
推出新 Stop 必能立即派发，还要满足原尝试、队列和效果状态条件。

合法接受的有限见证必须覆盖首次创建 Stop：输入没有该 Stop 的效果行和逻辑行，
实际 Commit 登记材料后调用原 `commit_effect`/`enqueue`，生成完整命令身份、原操作
origin、初始尝试及重试字段，并把 Stop 放入效果键和在途队列。该具体见证不等于
任意合法 World 都成功；已有行的保留规则也不能代替首次创建的接受见证。

Commit 到后来 Stop 的 generation 连接须明确中间步骤保持 sandbox 和 VM 两张表，
以及后来执行的是 Commit 构造的完整 Stop 命令且真实入口成功。实际派发器如何保持
这两张表仍是自有调用连接义务，不能自动归给 Tokio 的外部保证。
该条件连接不证明中间派发必成功或任务最终运行；后来 Stop 仍检查自己的全部守卫。

本片处理的是纯决定中的登记、许可和状态，不把 `Durable`、逻辑成功回执或
`VmStatus::Stopped` 当作真实落盘、VM 退出或可安全回收的证据。
V5/V6 仍须连接同一持久事务、完整 bundle 和宿主结束事实，V9 仍须连接可达历史、
身份与索引完整性以及条件进展。实际完成程度只在覆盖记录维护。

### 暂停成功发布的函数边界

实际 `handle_publish_paused` 的后续有限切片使用[暂停发布组件](../../verification/experiments/pause-publication/README.md)。
需要分别证明拒绝条件、合法输入能成功、完整返回事件/状态和其它状态不变。
资源索引中 VM/槽已关闭不能替代当前 Stop 命令的成功回执；命令身份包含 sandbox、
epoch、generation 和原 VM，尝试号来自当前逻辑效果行，缺行则沿原实现使用 0。
过去尝试成功不等于当前尝试成功；反例须保留原命令和查询，只绕过该拒绝分支，
并被同一成功前提性质拒绝。

该函数接收的逻辑回执、快照字段与资源索引不自证物理真实性或完整历史。
V5 提供存储/回执的一致查询与提交连接，V6 提供原宿主效果见证，V9 提供可达历史、
身份与索引完整性。缺少这些连接时，仅报告真实纯函数在给定输入域上的结论；
不报告已验证 VM 停机或完整暂停生命周期。

## 5. 首批原核心切片与后续接口

第一批在原 `OperationPlan` 内增加 `capture: Option<CaptureHandoff>`，不建立第二个控制器。该有限记录由原计划的操作键、epoch、源运行代、VM 和目标 snapshot 命名。`CaptureCommit` 区分未登记、结果未知和已确认登记；恢复资格区分捕获中、可恢复和已撤销。捕获工作与卸载来源分别占用预先存在的固定责任槽，撤销资格不清空这两个槽。它们是原核心的逻辑责任，尚不表示真实文件、mmap 或 SQLite 重启恢复已经接通。

普通 Pause 失败保留原 `Pausing`、运行代、VM 和资源索引，仅保持期望访问关闭，不虚构实际权限已经撤销，也不进入通用终止补偿。原 InstallFence 仍能完成实际权限和网络槽撤销。未登记时撤销新 capture 的恢复资格；提交未知时仍保留未知和原工作，不把它降成未提交；已确认登记时保留来源，允许继续原卸载。原派发入口拒绝已撤销的新 Freeze/Seal/Commit，Stop 与发布也沿同一个原计划检查。已经派发的物理工作仍可迟到：真实成功回执按原 worker 观察、Receipt 顺序登记材料责任，但不能把已撤销资格恢复为可恢复、排入 Stop 或发布成功。显式 Kill 独立设置终止并撤销未来资格，仍保留材料责任。

该值方法证明不覆盖已接通的完整产品卸载调用链，不把旧 `ManifestDigest(u64)` 当作内容校验证据，也不声称永久停留在 Pausing 满足进展。当前检查点及原 SQLite 接线见 [V5 检查点接口](05-local-service.md#5-显式卸载检查点)。真实 bundle 校验、同一 SQLite 提交、原工作失效/结束、撤销回执及重启枚举由 V5/V6 连接；主入口的同一性质及允许的真实效果不得换成外部假设。

该切片把值方法与原调用方分开验收：[组件入口和接口责任](../../verification/experiments/capture-handoff/README.md)。公共 `CaptureHandoff/Conditions` 只定义参数化的材料责任保持与撤销后迟到成功安全性质，不依赖具体生成类型；正确 `Values` 与两个真实错误版各在独立导入闭包检查同一性质，正常可接受性另外给出见证。公共根只导入纯条件，组件仍必须检查正确生成体、错误证书、全部实际文件与审计，不能漏验。原 `handle_compensation` 抽取成功只说明工具能生成，不等于其 World/计划身份/持久化对应已经证明；普通 Pause 错走终止补偿的原入口反例也仍需与两个值方法的形式反例区分。

当前继续路径沿原 Pause 计划扩展 `PrepareOriginalContinue` / `PublishOriginalContinued` 输入，复用原 `ActivateVm` 效果；不增加独立 `ContinueOriginalVm` 命令或控制器。输入绑定原操作键、epoch、运行代、VM 与已持久保存的工作结束事实；摘要仅命名该事实，不证明 OS/API 已完成。捕获结束事实使用互斥的 `NeverReleased / NoRequest / Returned`，绑定原身份、具体 Seal 尝试和材料工作；未放行必须在原 SQL 事务中封住将来的放行，`NoRequest` 要求原工作器明确保存未发送事实并退出，`Returned` 要求真实 API 结束与 worker 退出事实，不能由超时或工作器缺席推定。只允许被该事实精确覆盖的旧 Seal `Dispatched / Unknown` 变为 `RetiredByCaptureEnd`；它不是 Host 成功回执，不表示快照成功，也不释放材料。所有 End、Freeze/Commit、资源和当前计划条件先验证，再沿原 Pause 补偿准备继续；不泛用绕过未知动作。原效果派发前的持久责任仍是可能执行标记。已冻结的分支要求捕获资格撤销、提交未知已对账、旧 Freeze/Seal/Commit 工作确定结束，之后只继续原 VM；未冻结的确定失败也必须确认没有可能迟到的冻结效果。未知时保留原责任与关闭访问。继续不改变原运行代、寿命、材料使用责任，也不发布 Pause 成功。正式中心与网关如何接收同一失败继续结果见[模块合同](../module-contracts.md#暂停从活状态交接到完整材料)；本地输入与效果实现、服务持久事实、完整授权接线及故障验收分别记录，不以设计代替实现或证明。
