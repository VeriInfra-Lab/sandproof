# Sandbox Lifecycle Refinement

本仓库研究单节点 Firecracker 沙箱在 `create → pause → resume → kill` 过程中的控制协议。目标是在命令重复或迟到、RPC 结果未知、controller 崩溃恢复和旧资源延迟清理时，仍保证旧 generation 不能重新获得执行、文件或网络权限。

研究范围固定为单台 Linux/KVM 主机、一个 controller、一个 worker、Firecracker v1.16.1，以及每个 sandbox 独占的 workspace 和网络槽。模型支持多个 tenant 和 sandbox，但不处理多节点迁移、共享卷、动态 DNS/SNI 策略、恶意宿主或活性证明。

## 当前结果

Lean 4 是唯一规范来源：

- [`Types.lean`](SandboxModel/Types.lean)：身份、phase、authority、operation、effect、manifest 和命令类型。
- [`State.lean`](SandboxModel/State.lean)：controller、worker、资源、operation ledger 与 effect ledger 的持久状态。
- [`Step.lean`](SandboxModel/Step.lean)：所有合法异步协议步骤及其 guard。
- [`Safety.lean`](SandboxModel/Safety.lean)：状态、准入和跨步骤安全性质。
- [`Proofs.lean`](SandboxModel/Proofs.lean)：对任意可达状态和任意合法一步的证明。
- [`Scenarios.lean`](SandboxModel/Scenarios.lean)：正常生命周期、崩溃重放和迟到清理场景。
- [`Counterexamples.lean`](SandboxModel/Counterexamples.lean)：删除关键 guard 后的机器检查反例。

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
- effect 必须先持久提交再派发，ack 必须对应精确 EffectId，确认结果不会回退；
- snapshot 绑定不透明 `SnapshotManifestRef`，resume 使用被选中的同一个 manifest；
- cleanup 命令精确绑定 sid、generation 和 VM/slot ID；全局不变量保证任意时刻所有 reclaimed VM/slot 都不是 current，迟到清理不能回收或复活当前资源；
- epoch、generation、worker fence、workspace、file policy 和 snapshot manifest 满足相应单调或稳定性质。

证明针对无界标识集合和任意有限长度的合法历史，不是有限状态枚举；源码中没有 `sorry`、`admit` 或新增 `axiom`。

## 文件权限的边界

模型有意区分三个概念：

1. `WorkspaceApiAccess.managedReadWrite`：调用者能否通过受管 API 请求文件读写。
2. `FilePolicyRef`：controller 要求 Linux/worker 应用的具体权限配置之不透明身份。
3. `SnapshotManifestRef`：snapshot 数据和权限元数据清单之不透明身份。

Lean 证明 controller 和 worker 在生命周期交接中使用了正确的身份、generation、策略与 manifest。它不解释路径、文件内容、mode bits、UID/GID、ACL、symlink 或 ext4 行为。`applyPolicy(policy)` 与 `restore(manifest, workspace)` 是否忠实实现这些引用，是明确列出的 adapter 契约，需要用固定实现审计和故障实验验证。

因此，当前成果的准确名称是“已完整证明的 Lean 抽象协议安全模型”，还不是“已验证的 E2B、Firecracker、Linux 或 controller 实现”。实现到规范的 refinement 是下一阶段。

## 验证

项目固定 Lean 4.30.0。完整检查只需：

```sh
lake build
```

构建不依赖 Python，也没有保留旧的有限 BFS 模型。

## 文档

1. [研究计划](RESEARCH_PLAN.zh-CN.md)
2. [状态机与清晰边界](docs/state-machine.md)
3. [E2B 源码分析](docs/e2b-code-audit.md)
4. [Lean 到 Rust/adapter 的 refinement 路线](docs/refinement-plan.md)
5. [场景、反例与实验协议](docs/evaluation.md)
6. [固定源码证据](references/code-evidence.md)

E2B 阅读基线为 `e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`。本项目目前没有声称发现 E2B 漏洞，也没有声称验证整个 E2B 或 Firecracker。
