# Verified sandbox controller 建设路线

> 本文记录证明路线及既有协议阶段。2026-09-25 起当前工程顺序、三人分工和 V1 范围以[项目蓝图](project-blueprint.md)为准：先功能与测试，后集中证明；最终验证目标继续保留，以下证明义务不是 V1 新功能的逐项前置门槛。

## 1. 目标

### 证明路线与部署实现

研究目标包括 Rust 纯控制核心、真实 Rust 抽取结果与 Lean 状态机的
一致性证明，以及既定单节点边界内的安全性、完备性和最终完成性证明。
当前工具链是 Rust → Charon/Aeneas → Lean；尚未使用 Verus。

Sandproof 的事务存储、E2B 兼容接口、运行层和恢复服务按 V1 蓝图实现，
后续集中验证它们遵守合同。已有持久化与外部执行器的模型合同不等于
已经实现的数据库事务、网络发送、真实虚拟机操作或服务重启恢复，
纯核心构建通过也不代表部署系统已经得到形式化验证。

进度应按已经编译通过的定理及其明确前提记录，不使用代码量、测试
通过数或抽取函数数量代替证明覆盖率。

目标不再是用模型声称外部系统存在 bug，而是实现一个在明确边界内完全形式化验证的 sandbox controller。

最终交付物必须同时包含：

1. 唯一规范：Lean `World / Event / Step / Reachable`；
2. 可执行的纯 controller reducer；
3. reducer 每个接受分支到 `Step` 的 soundness 证明；
4. 对规范允许分支的 completeness/refinement 证明；
5. operation/effect journal 和 adapter 窄合同；
6. crash/recovery/fairness 假设下的收敛证明；
7. 可运行的 controller 实体及故障注入测试。

## 2. “完全验证”的边界

边界内：

- 单 controller、单 worker、单节点；
- create / pause / resume / kill / failure compensation；
- operation 幂等、effect outbox、epoch/generation/fence；
- controller crash/recover、消息重复/丢失/迟到；
- VM、workspace、snapshot、network slot 的身份与生命周期；
- managed exec/file/network capability 的准入。

边界外：

- Linux/KVM/Firecracker 内部实现正确性；
- 恶意宿主、内核漏洞和硬件故障；
- 多节点共识、迁移、fork/clone、共享卷；
- 路径级 POSIX/ACL 和文件内容语义；
- DNS/SNI 等动态网络策略。

边界外组件必须通过显式 adapter contract 连接；controller 只依赖返回的结构化 receipt，不在证明中假装知道 adapter 内部行为。

## 3. 信任链

```text
Lean protocol specification
        ↑ sound + complete refinement
Lean executable reducer
        ↑ extraction or proved Rust refinement
controller binary
        ↑ narrow adapter contracts
journal / worker / storage / network / Firecracker
```

不允许另外维护一份手写业务状态机。生产 controller 的每个决策要么由 Lean reducer 直接产生，要么对 Lean reducer 有逐分支 refinement 证明。

## 4. 建设阶段

### V0：主规范闭包

- 所有 lifecycle 转移都在 `Step`中；
- 失败/取消/补偿不能使用旁挂函数；
- stable phase 不能隐藏仍需 stop 的 VM；
- operation 和 effect 都有唯一终局。

当前进度：已封闭稳定状态的资源与成功回执条件；create/resume/pause 的失败关闭补偿已进入主 `Step/Reachable`；外部 effect 失败必须与当前 `sid + epoch` 因果绑定；kill 会原子终结被取代的未完成 operation，并拒绝 terminal 状态中的第二个 kill。逻辑动作现在使用“逻辑动作身份 + 尝试编号”，只有当前尝试明确得到可重试失败后才能创建严格递增的新尝试；超时只进入“结果未知”，不会暗中增加编号。操作计划和有限资源清单已经持久化，计划封存后资源身份与清理范围不可更换。尚未完成独立的用户取消入口。

### V1：可执行 reducer

- 定义 `Request / Observation / Decision / EffectIntent`；
- 实现确定性 `reduce : State → Input → Result Decision Reject`；
- 证明 `reduce_sound`：每个 Accepted decision 都是合法 `Step`；
- 证明 `reduce_complete`：边界内每个 controller-owned `Step` 都能由 reducer 产生；
- 证明确定性、reject 不改状态、决策与 effect 原子持久化。

当前进度：`SandboxModel/Controller.lean` 已实现 create/pause/resume/kill/compensate，请求重放与查询，effect 派发、超时、回执、严格重试与最终失败，后续命令调度，以及 Running/Paused/Destroyed/Failed 的稳定发布。`SandboxModel/WorkerController.lean` 覆盖 fence、VM、网络、文件策略、快照、停止和回收动作。所有成功 `Decision` 都携带一个会在生成代码时擦除的 `Step` 证明。请求/effect 的反向完备性已在 `HandlerCompleteness.lean` 中证明；调度、发布和 worker 的反向完备性分别放在独立模块中。这里的“可执行”仍指已证明的纯状态处理核心，不等于已经存在生产 controller 二进制。

### V2：故障与恢复

- 区分 durable state、volatile state 和 transport state；
- 加入 worker crash/restart、delivery loss/dequeue/redelivery；
- 已有结构化失败回执，并已区分可重试/不可重试失败及精确尝试编号；
- 将 reconciliation 建模为正式转移；
- 证明 crash consistency 和恢复幂等。

### V3：liveness

- 定义 enabled action 和 weak/strong fairness；
- 对每类 operation 定义 ranking/progress measure；
- 证明在 adapter 最终返回、journal 可用、scheduler 公平的假设下，operation 最终 succeeded/failed；
- 证明每个非当前资源最终 reclaimed。

当前进度：已定义允许空转的无限执行轨迹、时间谓词、与真实请求/effect/scheduler/publication/worker 处理函数相连的工作项，以及弱公平调度。操作账本一旦存在便不会消失，最终结果一旦写入便永久保持。`ControllerLiveness.lean` 已证明：若未完成操作总有一个持续可执行的具体工作项，公平调度最终执行它，并且包含适配器响应、重试耗尽和安全清理在内的有限进度量严格下降，则操作最终并永久得到成功或明确失败结果。

这不是“公平调度单独保证成功”。VM、存储、网络适配器永久失败时，调度器无法凭空完成操作。四类操作已有具体有限进度量和若干动作的局部下降证明；当前尝试的有效回执在正常阶段、销毁阶段及失败补偿阶段的下降也已证明。尚须闭合资源预留条件保持、外部等待、执行节点动作和控制器选取下一步的组合，才能得到不依赖内部未证明条件的最终完成性。

### V4：实现 refinement

首选方案是从可执行 Lean reducer 生成或包装生产代码。如果必须手写 Rust，则使用受限的纯 reducer core，通过 Aeneas/Lean 证明其与 Lean reducer 的 forward/backward simulation。I/O shell 只负责事务、调用 adapter 和回传 receipt。

当前进度：已固定 Rust 1.97.0、Charon、Aeneas 与 Lean 4.31.0 的完整
版本链；纯 Rust 核心的全部现有处理入口已能从真实编译器中间表示抽取
为 Lean 并通过编译。Aeneas 需要的少量标准库入口使用可计算定义，属于
需要审查的标准库语义模型，没有保留模板公理。

初始化、状态表示、查找更新、状态复制、运行状态、监控、请求重放、
结果查询、五类访问检查、客户端等待超时、外部命令派发、超时及最终失败处理已具有
对应证明。除独立初始化和监控入口外，上述处理路径已组合到统一控制
入口；工作执行端已证明当前尝试编号、真实队列查找及旧尝试/未入队
拒绝。重试和回执已覆盖全部分支，五类外部命令处理已经组合成统一对应
定理；编号耗尽时拒绝另有机器边界证明。稳定状态发布的四类更新动作及
当前尝试成功检查也已证明；资源关闭、停止回执的实际有限遍历已有终止
和结果对应证明。四类发布入口的全部前置检查、拒绝及成功分支现已完成，
并组合为统一发布定理及统一控制入口证明。
创建、暂停、恢复、销毁和失败补偿现已具有全部接受/拒绝及完整后继状态
对应，并合并为全部七类请求的统一入口证明。入队所需额外容量明确区分
暂停的两条命令与其他生命周期请求的一条命令。
调度全部八类已补齐并接入混合执行；启动资源预留的计划列表容量另外显式列出。
文件策略工作命令也已具有全部拒绝和成功更新的对应证明，并接入混合执行。
虚拟机回收、网络槽回收工作命令现已补齐全部拒绝和成功更新，包含真实资源登记与写入，
并接入统一入口及混合执行。围栏、准备虚拟机、网络配置现也已完成全部分支对应及组合，
激活、冻结、封存快照、提交快照和停止现已补齐，工作命令覆盖 11/11。
准备虚拟机另外显式要求三条命令容量和两次索引写入的保守容量界；
后者尚未最小化，不能声称已覆盖该充分条件之外的极限容量状态。
提交快照另要求一条停止命令的容量。`handleWorker_refines` 合并全部工作命令，
`handleController_refines` 已覆盖实际 Rust 公开输入类型的全部现有输入。
激活证明发现并纠正模型两类拒绝原因写反的问题，接受条件和 Rust 生产行为不变。
当前是明确机器边界下的纯核心一致性证明，不是已验证部署系统；具体定理、前提
和剩余义务见 [证明覆盖记录](proof-coverage.md)。

已覆盖入口还具有有限次混合调用的轨迹对应和安全性传递证明，最新输入
清单统一见 [证明覆盖记录](proof-coverage.md)。
新增的全部请求组包含生命周期操作，并与保留的重放/查询组重叠。
`controller_execution_refines` 与 `controller_execution_preserves_safe` 已覆盖实际公开
输入的任意有限混合执行。每一步仍要求有限机器容量，起点对应可达模型状态；
这不是无限执行最终完成、容量永不耗尽或外部 I/O 的证明。

## 5. 完成标准

下列条件全部满足时，才称为“边界内 fully verified controller”：

- 主规范无旁挂 lifecycle 路径；
- reducer soundness + completeness 通过；
- 生产实现与 reducer refinement 通过；
- safety 对任意有限可达历史成立；
- 在明确 fairness/adapter 假设下，liveness 成立；
- 所有不可证的底层行为都有窄合同和故障实验；
- 没有 `sorry`、`admit` 或项目自定义公理。
