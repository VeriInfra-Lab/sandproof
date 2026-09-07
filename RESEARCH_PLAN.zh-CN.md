# 研究计划：沙箱生命周期中的执行权、资源与精化

版本：0.5，2026-09-07。状态：Lean 参数化规范的完整 `Safe`、workspace authority 性质、正常生命周期轨迹与关键规则反例均已机器证明；尚无 production controller、refinement 或 Firecracker 实机结论。

## 1. 收缩后的研究问题

> 在单台 Linux/KVM 主机上，面对并发生命周期请求、重复或迟到命令、RPC 结果未知和 controller 崩溃，能否用 generation-scoped capability 状态机保证沙箱在 pause、resume、kill 交接中不双跑、不从终态复活，并把一个 Rust controller 精化到该规范？

暂题：**Generation-Scoped Authority for Crash-Tolerant Sandbox Lifecycle Control**。

中心抽象是：**快照只保存计算状态；执行许可由快照之外的当前 generation、资源身份和 worker fence 共同决定。** controller 的持久意图和节点上的实际权限不是同一个布尔值，它们之间需要有证据的交接。

## 2. 固定系统边界

| 维度 | 首版选择 |
|---|---|
| 下游 | 上游 Firecracker v1.16.1、Linux x86_64、KVM |
| 部署 | 单节点、一个 controller、一个受控 worker；controller 可 crash/restart |
| 沙箱 | 多 tenant / 多 sandbox；每个 sandbox 一个不可变 owner |
| 资源 | 每代一个 VM、独占 workspace、不可变 durable snapshot、独占 network slot |
| Workspace 权限 | 首版只有封闭类型 `WorkspaceMode.readWrite`；不引入 ACL 或独立读写布尔值 |
| 生命周期 | create、managed exec、pause、resume、kill、operation timeout/query 语义 |
| 快照 | 全量内存 + VM state + 独立磁盘副本；本地持久介质与原子 manifest |
| 网络 | `denyAll` 或静态 L3/L4 allowlist；管理入口与 guest 业务网络在模型中分开 |
| 权限 | Exec、FileRead/Write、Ingress/Egress、Resume 均绑定 sid；数据面再绑定 generation 与资源 |
| 故障 | 命令重复/乱序/延迟、回执丢失、RPC timeout、controller crash/recover |

不进入首篇：多节点调度与迁移、HA 共识、共享卷、路径级权限、文件内容、UID/GID、POSIX ACL、host mount、DNS/SNI 动态策略、snapshot fork/clone API、容器逃逸或 KVM 证明、恶意宿主、配额计费、完整 E2B 功能兼容。

Docker 只可用于离线制作 rootfs，不作为替代运行时。固定 Firecracker 是为了保留 freeze、snapshot、load、activate 与物理执行权交接的语义。

## 3. E2B 在本研究中的位置

E2B 是现实系统证据和后续 conformance 对照，不是直接照搬的 specification。代码阅读基线为 `e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`；已确认其实现具有 execution/lifecycle 身份、原子状态转换、异步 stop、pause/resume 和 snapshot 分层等值得对照的机制，详见[源码分析](docs/e2b-code-audit.md)。

研究必须区分：

- 我们的 StrictLocal 契约；
- E2B 公开源码可支持的契约；
- E2B 云端部署中不可从公开代码推出的事实。

Pause 或 kill 的返回语义不同，首先是 contract difference。只有固定版本、配置和观察映射后仍不存在合法历史，才能报告 conformance failure；还需单独证明影响才能称为安全问题。

## 4. 规范结构

Lean 是唯一权威规范。代码按单一职责拆分：

1. `Types`：领域身份、phase、capability、command、event。
2. `State`：无界 World、资源表与纯状态更新。
3. `Step`：协议 helper 与带 guard 的事件关系。
4. `Safety`：与执行逻辑分离的不变量。
5. `Proofs`：初始成立、逐步保持、可达状态归纳定理。
6. `Scenarios`：正常 create→pause→resume→kill 的构造性可达证明。
7. `Counterexamples`：故意删弱关键条件后的最小反例。

该层级符合 SOLID 中对单一职责、依赖方向和接口隔离的要求，又没有引入类层级、框架或通用 DSL。正式关系为：

```lean
Step : World → Event → World → Prop
Reachable : World → Prop
```

World 的映射和自然数身份均不设有限上界。Python BFS 与旧报告已删除，避免有限枚举被误解为最终证明。

## 5. 当前证明状态

已经由 Lean 4.30.0 检查，且没有 `sorry`、`admit` 或新增 `axiom`：

主定理是：

```lean
theorem reachable_safe {world : World} (reachable : Reachable world) : Safe world
```

它由 `emptyWorld_safe` 和 `step_preserves_safe` 归纳得到。`Safe` 包含十七项顶层条件：四类资源归属、workspace 反向绑定、固定 `readWrite` 模式、当前 VM/slot 的 owner/sid/generation 绑定、单执行 VM、单活动网络槽、活动槽与当前记录绑定、Durable 快照不可变、权限形状、fence 合法顺序、撤销 fence 关闭权限、有效权限对应真实 VM/网络事实、Paused/Destroyed 静止、terminal 与 Revoking/Destroyed 的双向一致性、Paused snapshot 的完整绑定、Starting 恢复源绑定，以及 operation/epoch/phase/result 一致性。

事件策略由另外的逐步定理覆盖：`step_event_authorized`、`step_egress_policy_enforced`、`step_file_event_workspace_bound`、`rpcTimeout_marks_outcome_unknown` 和 `step_keeps_destroyed`。`step_never_clears_terminal` 单独给出 tombstone 的逐步单调性。

侧边讨论提出的 workspace authority 要求已逐项落到命名定理：

| 要求 | 形式化结果 |
|---|---|
| WorkspaceIdentityStable | `step_workspaceIdentityStable`：已有 sandbox 的 workspace 不被任何 Step 换绑 |
| WorkspaceModeStable | `step_workspaceModeStable`，并由 `WorkspaceModeFixed` 将首版限制为 `readWrite` |
| RunningWorkspaceAuthorityExact | `reachable_runningWorkspaceAuthorityExact`：Running 权限精确绑定当前 sid/generation/workspace/mode |
| NoFileAuthorityOutsideRunning | `noFileAuthorityOutsideRunning`；Pausing/Revoking fence 后另由 `reachable_revocationFenceClosesFileAuthority` 关闭 effective authority |
| ResumeReissuesWorkspaceAuthority | `resumeReissuesWorkspaceAuthority`：PublishRunning 从当前 controller 记录构造权限 |
| SnapshotCarriesNoAuthority | `snapshotCarriesNoAuthority`：AcceptResume 只选择 restore source，新 generation 的 desired/effective 均为 closed |
| StaleGenerationCannotAccessWorkspace | `staleGenerationCannotAccessWorkspace`：旧代 read/write 事件不能成为 Step |

这里有意区分 controller admission 和 worker 的物理撤销窗口。进入 Pausing 后，controller 不再接收新的 managed 文件请求；fence 尚未到达时旧 worker 权限可能短暂存在，fence 到达后 Lean 证明 effective authority 必为 closed。

“文件权限不丢失”只在 controller 层证明：workspace 仍是同一个对象，模式仍是 `readWrite`，新代权限由当前记录签发。Linux 层的内容、mode bits、owner 和 ACL 由 storage adapter 契约承担：`restore(manifest, workspace)` 成功时必须忠实实现 manifest 及下游承诺保存的元数据。Lean 只检查交给 adapter 的 snapshot 来源、workspace 和 generation 身份，不证明 ext4、Firecracker 或 Linux 内部行为。

证明过程发现并修正了三组协议/规范缺口：迟到的 create/resume fence 原本可能在 Running 中关闭当前权限；已有 fence 原本可以超前于当前 epoch；Safe 原本只在 effective authority 打开时检查资源，没有直接约束关闭窗口中的 current VM/slot、活动路由和已选恢复源。现在启动 fence 与撤销 fence 是不同转移，`FenceWellFormed` 约束 fence 来源与顺序，新增资源、活动路由和恢复源不变量覆盖关闭窗口。`PublishRunning` 仍必须证明该 sid 没有旧活动槽。

## 6. 当前阶段：冻结系统开发，继续审计规范

`Reachable world → Safe world`、正常 create→pause→resume→kill 构造轨迹以及四个 mutation 反例均已闭合。反例分别显示：删除激活前互斥检查会双跑，删除发布前网络互斥会保留两条活动路由，跳过静止条件会在 VM 仍执行时发布 Paused，只检查 durable 而不检查快照身份会接受跨 sandbox 恢复源。按当前决定，暂不实现 controller。下一批 formal artifact 是逐项审计 `Step` 与 E2B/Firecracker 事件的观察映射。

## 7. Rust 与 Aeneas 精化路线

完整 Safe 闭合后，再实现一个小型 Rust reducer：

```text
reduce(state, event) -> Result<(state', effects), Reject>
```

它只决定持久状态和 effect 描述，不直接做网络、磁盘或 Firecracker I/O。使用 Aeneas 将安全关键 Rust 子集翻译到 Lean，并证明生成定义模拟 `Step`。不追求从 Lean 自动生成整个 controller，也不同时维护 Dafny/TLA+ 的第二份权威规范。

手写 runtime 仍须满足明确 adapter contract：journal 原子持久、提交后派发、精确 VM 身份、worker 在副作用处检查 fence、snapshot bundle 的 durability、stop/wait 的物理含义。Aeneas、Rust/Lean 工具链、Firecracker/Linux primitive 和 adapter contract 都如实列入 TCB。

## 8. 实验计划

在一台受控 Linux/KVM 机器上固定 kernel/rootfs/hash，依次测试：

- create×kill、resume×kill、pause×kill、resume×resume；
- exec 与 pause/kill 的 admission 交界；
- load/activate/freeze/snapshot/stop 每个回执丢失点；
- journal 提交前后 crash；
- snapshot 文件完成与 manifest 提交之间掉电；
- 旧资源 cleanup 与新 generation 并发。

结果按完整 history 判定为 Conforms、Violates、Inconclusive、Contract difference 或 Unsupported configuration。不会从单个 HTTP 状态、迟到输出或两个 PID 共存直接推断漏洞。

## 9. 阶段验收与停止条件

| 阶段 | 交付物 | 通过条件 |
|---|---|---|
| G0 规范 | 当前 Lean World/Step/Safety | 范围、完成点、故障模型可评审；核心定理无占位证明 |
| G1 安全闭合 | `Reachable → Safe`、workspace authority、正常 trace、mutation | 已完成：17 项 Safe、7 组 workspace 定理、完整生命周期轨迹、4 个关键反例及旧代文件访问拒绝实例 |
| G2 reducer 精化 | Rust reducer + Aeneas + Lean simulation | 每个 reducer 分支对应一个 Step 或明确 Reject/stutter |
| G3 runtime contract | journal/worker/storage/FC adapters | 关键 fence 与 durability 假设可由故障注入支持 |
| G4 E2B 对照 | 固定 SHA/profile 的 histories | 区分契约差异、未知与可复现违反 |
| G5 artifact | 构建、证明、trace、机器配置 | 可重建，证明与未证明范围逐项列出 |

如果 G1 无法闭合，就继续收缩状态机；如果 worker fence 只能靠“实现会正确”这一大假设，就停止宣称 verified controller；如果 Aeneas 不支持必要 Rust 子集，就进一步缩小 reducer，而不是换回手写双份 specification。
