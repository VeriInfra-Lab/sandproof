# 研究计划：沙箱生命周期中的执行权、资源与精化

版本：0.2，2026-09-07。状态：Lean 参数化规范已经建立，核心不变量已开始机器证明；尚无 production controller 或 Firecracker 实机结论。

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
| 生命周期 | create、managed exec、pause、resume、kill、operation timeout/query 语义 |
| 快照 | 全量内存 + VM state + 独立磁盘副本；本地持久介质与原子 manifest |
| 网络 | `denyAll` 或静态 L3/L4 allowlist；管理入口与 guest 业务网络在模型中分开 |
| 权限 | Exec、FileRead/Write、Ingress/Egress、Resume 均绑定 sid；数据面再绑定 generation 与资源 |
| 故障 | 命令重复/乱序/延迟、回执丢失、RPC timeout、controller crash/recover |

不进入首篇：多节点调度与迁移、HA 共识、共享卷、POSIX ACL、host mount、DNS/SNI 动态策略、snapshot fork/clone API、容器逃逸或 KVM 证明、恶意宿主、配额计费、完整 E2B 功能兼容。

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

该层级符合 SOLID 中对单一职责、依赖方向和接口隔离的要求，又没有引入类层级、框架或通用 DSL。正式关系为：

```lean
Step : World → Event → World → Prop
Reachable : World → Prop
```

World 的映射和自然数身份均不设有限上界。Python BFS 与旧报告已删除，避免有限枚举被误解为最终证明。

## 5. 当前证明状态

已经由 Lean 4.30.0 检查，且没有 `sorry`、`admit` 或新增 `axiom`：

| 定理 | 含义 |
|---|---|
| `reachable_unique_execution` | 任意可达 World 中，同一 sid 至多一个 executing VM |
| `reachable_durable_snapshots_are_immutable` | 任意可达 World 中，Durable snapshot 必为 immutable |
| `reachable_terminal_consistent` | terminal 对象只能处于 Revoking/Destroyed |
| `step_never_clears_terminal` | 任意合法一步都不能清除已有 terminal tombstone |
| `reachable_core_safe` | 汇总前三项状态不变量 |

完整 `Safe` 已写成正式目标，但资源归属、capability 形状、worker fence、有效权限的运行时依据、稳定态静止和 Paused snapshot 绑定仍待逐项证明。本阶段宁可明确显示证明缺口，也不通过缩弱定义或假设把它隐藏掉。

## 6. 下一阶段：先闭合 Safe，再写 controller

按依赖顺序完成：

1. 证明资源表的 owner/sid 归属在每个 Step 后保持。
2. 证明 capability 的 phase/generation/resource 形状。
3. 证明当前 Pausing/Revoking fence 清除 effective authority。
4. 证明 data-plane authority 必有 executing VM 与 active/configured network slot。
5. 证明 Paused/Destroyed quiescence 与 Paused snapshot binding。
6. 得到 `Reachable world → Safe world` 的总定理。
7. 增加正常 create→pause→resume→kill 的构造性可达 trace，排除“全拒绝所以安全”。

若某个义务不能成立，先修改状态机和 guard；不在证明中加入针对实现缺陷的无依据假设。

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
| G1 安全闭合 | `Reachable → Safe` 与正常 trace | 每项不变量逐 Step 保持；mutation 能破坏相应 guard |
| G2 reducer 精化 | Rust reducer + Aeneas + Lean simulation | 每个 reducer 分支对应一个 Step 或明确 Reject/stutter |
| G3 runtime contract | journal/worker/storage/FC adapters | 关键 fence 与 durability 假设可由故障注入支持 |
| G4 E2B 对照 | 固定 SHA/profile 的 histories | 区分契约差异、未知与可复现违反 |
| G5 artifact | 构建、证明、trace、机器配置 | 可重建，证明与未证明范围逐项列出 |

如果 G1 无法闭合，就继续收缩状态机；如果 worker fence 只能靠“实现会正确”这一大假设，就停止宣称 verified controller；如果 Aeneas 不支持必要 Rust 子集，就进一步缩小 reducer，而不是换回手写双份 specification。
