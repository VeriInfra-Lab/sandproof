# 研究计划：Generation-Scoped Sandbox Lifecycle Authority

> 历史研究计划。2026-09-25 起当前目标是 E2B 核心兼容沙箱，按[项目蓝图](docs/project-blueprint.md)先完成 V1 功能与测试，再集中形式化验证。本文的研究论证继续保留，但“E2B 仅作审计对象”和旧开发顺序不再定义当前工程任务。现有 Rust 对应证明状态见[证明覆盖记录](docs/proof-coverage.md)。

版本：0.7，2026-09-20。状态：Lean 协议安全、可执行纯状态处理函数、规范分支反向覆盖及条件化最终完成已机器证明；尚无生产 controller refinement 或 Firecracker/E2B 实机结论。

## 1. 研究问题

> 在单台 Linux/KVM 主机上，面对并发生命周期请求、重复或迟到命令、RPC 结果未知、controller 崩溃和旧资源延迟清理，能否用 generation-scoped authority、持久 operation/effect ledger 和 worker fence，保证 sandbox 在 create、pause、resume、kill 交接中不双跑、不错误换绑 workspace/policy/manifest、不从终态复活？

暂题：**Generation-Scoped Authority for Crash-Tolerant Sandbox Lifecycle Control**。

中心命题是：**snapshot 保存数据与计算状态；它不保存执行权，也不保存永久文件访问权。** 每次 resume 的权限来自 controller 当前持久记录，并绑定新的 generation。

## 2. 固定边界

| 维度 | 首版选择 |
|---|---|
| 运行时 | 上游 Firecracker v1.16.1、Linux x86_64、KVM |
| 部署 | 单节点、一个 controller、一个 worker；controller 可 crash/restart |
| 对象 | 多 tenant/sandbox；每个 sandbox owner 不变 |
| 资源 | 每代一个 VM、独占 workspace、独占 network slot、immutable durable snapshot |
| API 文件权限 | 唯一构造 `WorkspaceApiAccess.managedReadWrite` |
| 下游文件策略 | 不透明 `FilePolicyRef`；不解释路径、chmod、UID/GID 或 ACL |
| snapshot | 不透明 `SnapshotManifestRef` 绑定来源 generation 与 workspace |
| 网络 | `denyAll` 或静态 L3/L4 endpoint allowlist |
| 故障 | 重复/乱序/迟到、回执丢失、timeout、controller crash/recover、迟到 cleanup |

不进入首篇：多节点调度和迁移、HA 共识、共享卷、路径和文件内容模型、POSIX ACL 语义、host mount、DNS/SNI 动态策略、snapshot fork/clone、容器逃逸/KVM 证明、恶意宿主、计费、完整 E2B 兼容，以及不要求底层最终响应的无条件完成性。

Docker 只用于离线制作 rootfs，不作为运行时。固定 Firecracker 是为了保留 freeze、snapshot、restore、activate 与物理执行权交接的真实语义。

## 3. E2B 的角色

E2B 是现实系统证据和后续 conformance 对照，不是直接照搬的 specification。源码阅读固定在 `e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`，见[源码分析](docs/e2b-code-audit.md)。

评估会分别陈述本研究的 StrictLocal 契约、E2B 公开源码能支持的契约、以及云端部署中无法从公开代码推出的事实。返回语义不同首先记为 contract difference；只有固定版本、配置和观察映射后仍无合法历史，才能称 conformance failure。审计按[系统性 conformance 审计方法](docs/conformance-method.md)执行：固定 obs/evt 映射与 guard 表，按副作用枚举、guard-diff、字段写审计、准入审计四层做覆盖记账，已知修复历史只用于校准尺子，不用作发现目标。

## 4. 规范结构

Lean 是唯一权威来源：

1. `Types`：分离的身份类型、phase、authority、operation、effect、policy 和 manifest。
2. `State`：无界 World 及纯更新函数。
3. `Step`：每个异步协议动作及其 guard。
4. `Safety`：状态、准入和跨步骤性质。
5. `Proofs`：初始、保持、可达性与直接协议定理。
6. `Scenarios`：正常路径、重试查询、崩溃重放、迟到 cleanup。
7. `Counterexamples`：绕过关键 guard 后的最小坏状态。
8. `Controller / WorkerController`：请求、具体动作、调度、执行端动作和稳定状态发布的确定性处理函数。
9. `HandlerCompleteness / SchedulerCompleteness / WorkerCompleteness`：规范允许分支的反向覆盖。
10. `ControllerLiveness`：真实处理函数上的弱公平和条件化最终完成。

```lean
Step : World → Event → World → Prop
Reachable : World → Prop
```

标识、资源数量和轨迹长度没有预设上限。旧 Python BFS 已删除，避免有限枚举被误解为正式证明。

## 5. 本轮建议的分析结论

讨论提出的六组补充总体合理，并且不会扩大到 Linux 内部证明。实现采用以下收缩：

- managed 文件 API 固定 read-write；复杂下游权限只用 `FilePolicyRef` 表示身份；
- snapshot 只增加 `SnapshotManifestRef`，不建模文件内容与 ACL；
- operation ID 作用域固定为 `(SandboxId, OperationId)`；
- operation 明确区分 pending、结果未知、补偿中、成功与失败；失败补偿和 kill 取代旧操作都有合法转移；
- effect 账本保存已提交、已派发、结果未知和带成功/失败结果的已确认状态；
- EffectId 由逻辑动作身份和明确尝试编号组成；超时保持原编号，只有当前尝试明确可重试失败后才能产生严格连续的新编号；
- cleanup 精确绑定 generation 和资源 ID；
- 历史性质写成 `Step before event after` 定理，不伪装成单个 World 的字段；
- `inflight` 仅保留为消息可能重复到达的传输历史，执行资格完全来自持久 effect ledger。

`AppliedFilePolicy` 可以在非 Running 状态暂时保留旧代 receipt，但它不授予任何 authority。发布 Running 时必须匹配当前记录并有精确 ack，所以旧 receipt 无法使新 generation 运行。

## 6. 已证明的规格

顶层可达状态定理：

```lean
theorem reachable_protocol_secure {world : World}
    (reachable : Reachable world) : ProtocolSecurity world

theorem step_protocol_secure
    (transition : Step world event next) :
    ProtocolStepSecurity world event next
```

第一个定理组合原有 `Safe`、policy receipt 合法性、Running 精确 policy receipt、operation ledger 键值合法性、当前 operation 的账本存在性、effect identity，以及 reclaimed 资源永不为 current；第二个汇总任意合法一步的用户、effect 与 cleanup 准入、状态保持、单调性以及 operation/effect 结果稳定。原有 `Safe` 覆盖资源归属、workspace 双向绑定、当前 VM/slot 身份、单执行 VM、单活动 slot、snapshot durability/immutability、authority 形状、fence 合法、terminal/phase、Paused/Starting 来源和当前 operation/phase 一致性。

独立定理覆盖：

| 规格组 | 已证明结果 |
|---|---|
| Workspace/API | `step_workspaceIdentityStable`、`step_workspaceApiAccessStable`、`reachable_runningWorkspaceAuthorityExact`、`noFileAuthorityOutsideRunning`、`staleGenerationCannotAccessWorkspace` |
| File policy | `reachable_appliedFilePolicyValid`、`step_filePolicyStable`、`reachable_runningRequiresAppliedFilePolicy`、`staleFilePolicyCommandRejected`、resume 后重新应用当前 policy |
| Manifest | `step_snapshotManifestStable`、Paused/Starting source 绑定、`snapshotCarriesNoAuthority`、resume 使用精确 selected manifest |
| Operation | `reachable_operationLedgerValid`、`reachable_currentOperationInLedger`、`operationRetryIdempotent`、`queryReturnsCommittedResult`、`step_operationIdentityStable`、`step_operationResultStable`；新操作必须使用 fresh `(sid,id)`，已有 kind/epoch/input 不可改写，timeout 只能改匹配的未完成账本 |
| Effect | `effectCommittedBeforeDispatch`、结果未知后的安全重发、迟到/重复/冲突回执、命令后置状态检查、`step_ackCannotChangeAnotherEffect`、`step_effectResultStable` |
| Cleanup | `cleanupVmTargetsExactResource`、`cleanupSlotTargetsExactResource`、两个单步非 current 定理，以及覆盖任意可达状态的 `reachable_reclaimedResourcesNotCurrent` |
| 单调性 | epoch、generation、worker fence、terminal、workspace、API access、file policy 和 manifest 的逐步单调/稳定定理 |

正常场景覆盖 create、policy apply/ack、Running、manifest seal/commit/stop/ack、Paused、Resume、operation retry/query、effect commit 后 controller crash/recover、结果未知后的重发与迟到回执、冲突回执拒绝、外部失败证据补偿、kill 原子终结旧 operation，以及旧代清理晚于新代运行。Mutant 覆盖缺少 policy guard、旧代 policy、错误 manifest、未提交派发、清理当前资源和 fence 回退。

所有证明使用 Lean 4.30.0，源码没有 `sorry`、`admit` 或新增 `axiom`。

## 7. 清楚的证明边界

Lean 证明的是 controller/worker 协议中使用的身份和顺序。它没有证明：

- Linux 确实按 `FilePolicyRef` 设置 mode、owner、ACL；
- storage 确实按 manifest 保存和恢复内容及元数据；
- Firecracker/KVM 确实按 stop/freeze/activate 回执语义工作；
- journal 的真实断电原子性；
- Rust controller 的实现分支都对应 `Step`；
- 网络栈、内核、编译器或硬件没有 bug；
- 在适配器永久不响应或安全清理永久失败时，请求仍最终完成。

这些内容被拆成窄 adapter contract。Lean 验证传给 adapter 的 sid、epoch、generation、resource、workspace、policy 和 manifest 身份正确；具体执行由固定版本审计、refinement 和 failpoint 实验支撑。

## 8. 下一阶段：生产实现 refinement

Lean 内的纯状态处理核心已经完成，并且每个成功分支携带合法 `Step` 证明，规范允许分支也有反向覆盖。下一步不再另写一套业务状态机，而是从该核心生成/包装生产实现，或实现受限的纯 Rust 核心：

```text
reduce(state, event) -> Result<(state', effects), Reject>
```

它不直接执行 I/O。使用 Aeneas 翻译安全关键 Rust 子集，并证明每个成功分支模拟一个合法 `Step`，Reject 不产生未持久 effect。journal/outbox、worker fence、Firecracker、storage、policy、network 和 cleanup 留在明确 adapter 边界。

详细合同见[refinement 计划](docs/refinement-plan.md)。如果 Aeneas 对必要子集支持不足，继续缩小 reducer，不再维护另一份手写 specification。

## 9. 实验计划

固定一台 Linux/KVM 主机、kernel/rootfs/config hash，按顺序验证：

- operation、outbox 和 ack 写入前后 crash；
- apply-policy 新旧 generation 乱序；
- snapshot 数据写入、seal 与 manifest commit 间掉电；
- activate/freeze/stop 与 worker fence 的不可取消窗口；
- 旧 VM/slot cleanup 与新 generation 并发；
- create×kill、resume×kill、pause×kill、resume×resume 和 exec×pause/kill。

结果按完整 history 分为 Conforms、Violates、Inconclusive、Contract difference 或 Unsupported configuration，见[实验协议](docs/evaluation.md)。

## 10. 里程碑

| 阶段 | 交付物 | 当前状态 |
|---|---|---|
| G0 | 固定范围、E2B 源码基线、Lean World/Step | 完成 |
| G1 | 状态/准入/历史 safety、正常场景、mutant | 完成 |
| G2 | Lean 可执行核心、Rust reducer、Aeneas translation、simulation | Lean 核心完成；Rust/refinement 未开始 |
| G3 | journal/worker/storage/FC adapter 实验 | 未开始 |
| G4 | 固定 E2B profile 的 conformance histories | 未开始 |
| G5 | 可复现 artifact 与论文结果 | 未开始 |

停止条件保持严格：如果实现无法证明模拟 `Step`，或 worker fence 只能由一个笼统的“实现正确”假设承担，就不宣称 verified controller。
