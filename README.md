# Sandbox Lifecycle Refinement

> 新的实现方向是更小的单机 Firecracker 沙箱：指定文件夹只读导入、独立可写工作区、执行命令、受控导出和销毁。正在先建设运行层，随后为这一缩小的范围建立新证明。当前进度与未完成项见[单节点小沙箱开发记录](docs/minimal-sandbox-runtime.md)。下文保留原有暂停/恢复协议研究的说明；其中的定理不能自动覆盖新运行层。

本仓库研究单节点 Firecracker 沙箱在 `create → pause → resume → kill` 过程中的控制协议。目标是在命令重复或迟到、RPC 结果未知、controller 崩溃恢复和旧资源延迟清理时，仍保证旧 generation 不能重新获得执行、文件或网络权限。

研究范围固定为单台 Linux/KVM 主机、一个 controller、一个 worker、Firecracker v1.16.1，以及每个 sandbox 独占的 workspace 和网络槽。模型支持多个 tenant 和 sandbox，但不处理多节点迁移、共享卷、动态 DNS/SNI 策略或恶意宿主；完成性只在明确的公平调度、适配器响应和有限进度条件下成立。

## 当前结果

Rust 纯控制核心已实现；现有公开输入的对应证明，以及在明确机器容量
条件下任意有限次混合执行的安全证明已经通过。新运行层尚未纳入这些证明。
可复核的已证分支、证明前提和剩余工作见
[证明覆盖记录](docs/proof-coverage.md)。数据库、Redis、CubeSandbox 接入
与生产部署已列为后续阶段；当前没有实现这些运行服务。

Lean 4 是唯一规范来源：

- [`Types.lean`](SandboxModel/Types.lean)：身份、phase、authority、operation、effect、manifest 和命令类型。
- [`State.lean`](SandboxModel/State.lean)：controller、worker、资源、operation ledger 与 effect ledger 的持久状态。
- [`Step.lean`](SandboxModel/Step.lean)：所有合法异步协议步骤及其 guard。
- [`Controller.lean`](SandboxModel/Controller.lean)：可执行的请求、具体动作、调度与稳定状态发布处理函数；每个成功决策携带 `Step` 证明。
- [`WorkerController.lean`](SandboxModel/WorkerController.lean)：围栏、虚拟机、网络、文件策略、快照、停止和回收动作的可执行处理函数。
- [`HandlerCompleteness.lean`](SandboxModel/HandlerCompleteness.lean)、[`SchedulerCompleteness.lean`](SandboxModel/SchedulerCompleteness.lean)、[`WorkerCompleteness.lean`](SandboxModel/WorkerCompleteness.lean)：规范允许分支到各处理函数的反向覆盖证明。
- [`PlanProofs.lean`](SandboxModel/PlanProofs.lean)、[`PlanLedgerProofs.lean`](SandboxModel/PlanLedgerProofs.lean)：操作计划、资源身份和操作账本不会分离的保持性证明。
- [`ControllerLiveness.lean`](SandboxModel/ControllerLiveness.lean)：把弱公平条件连接到真实处理函数，并在明确进度合同下证明操作最终且永久得到结果。
- [`Safety.lean`](SandboxModel/Safety.lean)：状态、准入和跨步骤安全性质。
- [`Proofs.lean`](SandboxModel/Proofs.lean)：对任意可达状态和任意合法一步的证明。
- [`Scenarios.lean`](SandboxModel/Scenarios.lean)：正常生命周期、崩溃重放和迟到清理场景。
- [`Counterexamples.lean`](SandboxModel/Counterexamples.lean)：删除关键 guard 后的机器检查反例。
- [`Compensation.lean`](SandboxModel/Compensation.lean)：主状态机内的 failure 补偿合同。

顶层定理已经通过 Lean 检查：

```lean
theorem reachable_protocol_secure {world : World}
    (reachable : Reachable world) : ProtocolSecurity world

theorem step_protocol_secure
    (transition : Step world event next) :
    ProtocolStepSecurity world event next
```

第一个定理汇总所有可达状态的安全，包括 policy receipt、operation/effect 账本身份、当前 operation 的账本存在性，以及已回收资源永不再成为 current；第二个定理汇总每一步的用户准入、effect/cleanup 准入、状态保持、单调性以及 operation/effect 结果稳定性。

它包含原有资源归属、单执行 VM、单活动网络槽、terminal 不可逆、snapshot 和恢复源合法等状态不变量，并加入：

- Running 前必须存在与当前 `sid + generation + workspace + FilePolicyRef` 完全相同的 worker 应用记录和已确认 effect；
- operation 以 `(SandboxId, OperationId)` 为键，重试只能使用相同 kind 与输入，查询返回持久结果；
- operation 的 kind、epoch 和输入在同一账本键下不可改写；最终结果一旦写入，后续任何合法步骤也不能覆盖；
- effect 必须先持久提交再派发；超时只产生“结果未知”，可以安全重发并接收迟到结果；相同回执可重放，冲突回执会被拒绝；
- 成功回执必须有命令后置状态作证；在当前全有或全无的命令抽象下，失败回执必须确认成功后置状态不存在；
- snapshot 绑定不透明 `SnapshotManifestRef`，resume 使用被选中的同一个 manifest；
- cleanup 命令精确绑定 sid、generation 和 VM/slot ID；全局不变量保证任意时刻所有 reclaimed VM/slot 都不是 current，迟到清理不能回收或复活当前资源；
- epoch、generation、worker fence、workspace、file policy 和 snapshot manifest 满足相应单调或稳定性质。
- Paused/Destroyed 发布前，该 sandbox 的所有 VM 必须已 `stopped/reclaimed`，不能隐藏 prepared/frozen 资源；
- create/resume/pause 的外部失败只有在失败 effect 与当前 `sid + epoch` 精确对应时才能进入失败关闭补偿；取消和截止时间到期是单独的本地原因；
- kill 打断未完成操作时，会在同一持久转移中把旧操作终结为 `failed(cancelled)`；terminal revocation 开始后不能再创建第二个 kill 操作；
- create/resume/pause 失败通过 `compensating(failure) → terminal fence → failed(failure)` 进入终态，与正常生命周期共用 `Step/Reachable` 和 safety 证明。

证明针对无界标识集合和任意有限长度的合法历史，不是有限状态枚举；源码中没有 `sorry`、`admit` 或新增 `axiom`。

## 文件权限的边界

模型有意区分三个概念：

1. `WorkspaceApiAccess.managedReadWrite`：调用者能否通过受管 API 请求文件读写。
2. `FilePolicyRef`：controller 要求 Linux/worker 应用的具体权限配置之不透明身份。
3. `SnapshotManifestRef`：snapshot 数据和权限元数据清单之不透明身份。

Lean 证明 controller 和 worker 在生命周期交接中使用了正确的身份、generation、策略与 manifest。它不解释路径、文件内容、mode bits、UID/GID、ACL、symlink 或 ext4 行为。`applyPolicy(policy)` 与 `restore(manifest, workspace)` 是否忠实实现这些引用，是明确列出的 adapter 契约，需要用固定实现审计和故障实验验证。

因此，当前成果的准确名称是“已证明的 Lean 协议模型 + 可执行且带双向覆盖证明的纯状态处理核心”。它仍不是完整 verified controller：每条工作流的具体进度合同、生产实现与该核心的一致性证明，以及底层适配器合同仍是必要阶段。

## 验证

项目固定 Lean 4.31.0，并把可部署控制核心固定在 Rust 1.97.0。仅检查
协议模型可运行：

```sh
lake build
```

包含实际 Rust 抽取、Rust 测试与静态检查的完整验证可运行：

```sh
scripts/bootstrap-aeneas.sh  # 首次准备固定版本工具链
scripts/verify-controller.sh
```

当前 Rust 路线不是重新手写第二份规范：Charon/Aeneas 会把真正编译的
Rust 中间表示转换为 Lean 函数，然后由 `SandboxModel/RustRefinement/`
中的定理与上面的唯一协议规范比较。详细信任链和版本见
[`verification/README.zh-CN.md`](verification/README.zh-CN.md)。

## 文档

1. [项目进展总览](docs/progress-overview.md)
2. [缺口登记表（优先级）](docs/gap-register.md)
3. [研究计划](RESEARCH_PLAN.zh-CN.md)
4. [战略定位：两条轨道与 MBT](docs/strategy.md)
5. [状态机与清晰边界](docs/state-machine.md)
6. [Verified sandbox controller 建设路线](docs/verified-controller-plan.md)
7. [E2B 源码分析](docs/e2b-code-audit.md)
8. [用状态机审计 E2B 的第一次结果](docs/e2b-bug-finding.md)
9. [系统性 conformance 审计方法](docs/conformance-method.md)
10. [模型与 E2B 的系统关联：相关工作与方法](docs/model-based-testing-survey.md)
11. [Trace 格式与映射](docs/trace-format.md)
12. [模型—实现连接的理论：可验证映射契约](docs/mapping-theory.md)
13. [Trace checker：实现与结果](docs/trace-checker.md)
14. [插桩与综合测试](docs/instrumentation-and-testing.md)
15. [模型扩展：accounting / compensation / liveness](docs/model-extensions.md)
16. [Checker 覆盖度](docs/trace-coverage.md)
17. [Issue 语料与相对完备性](docs/issue-study.md)
18. [Issue 语料第一遍判定（待复核）](docs/issue-triage.md)
19. [语料复核：翻译、安全维度与性质对应](docs/corpus-review.md)
20. [Conformance 审计第一轮](docs/e2b-conformance-round1.md)
21. [F-1 复现：kill × 在途 resume](docs/e2b-f1-repro.md)
22. [F-1 issue 草稿](docs/e2b-f1-issue-draft.md)
23. [E2B 第二轮测试发现](docs/e2b-round2-findings.md)
24. [G-1 issue 草稿](docs/e2b-g1-issue-draft.md)
25. [Lean 到 Rust/adapter 的 refinement 路线](docs/refinement-plan.md)
26. [场景、反例与实验协议](docs/evaluation.md)
27. [固定源码证据](references/code-evidence.md)

E2B 阅读基线为 `e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`。本项目不声称验证整个 E2B 或 Firecracker，也不再把本地 harness 行为作为“已确认系统 bug”的结论。相关文档保留为历史实验记录；当前主线是建设边界内 fully verified controller。
