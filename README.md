# Sandbox Lifecycle Refinement

本仓库研究 **Firecracker 沙箱在 create、pause、resume、kill 交接中的执行权安全**。核心问题是：当命令重复或迟到、RPC 结果未知、controller 崩溃时，如何保证旧工作不能重新获得执行权，并且 Paused/Destroyed 的发布有足够的运行时证据。

研究边界固定为单台 Linux/KVM 主机、一个 controller、一个 worker、Firecracker v1.16.1、每个沙箱独占 workspace 与网络槽。网络策略只建模 `deny-all` 或静态 L3/L4 allowlist。首阶段不处理多节点迁移、共享卷、DNS/SNI 策略、恶意宿主、完整 E2B 兼容或活性证明。

## 当前实现

Lean 4 是规范与证明的唯一权威来源：

- [`Types.lean`](SandboxModel/Types.lean)：有类型的身份、生命周期 phase、capability、command 与 event。
- [`State.lean`](SandboxModel/State.lean)：参数化 `World`，包含多租户沙箱、VM、文件系统、快照、网络槽、worker fence 与在途命令。
- [`Step.lean`](SandboxModel/Step.lean)：异步事件关系 `Step : World → Event → World → Prop` 与无界可达性定义。
- [`Safety.lean`](SandboxModel/Safety.lean)：完整安全目标。
- [`Proofs.lean`](SandboxModel/Proofs.lean)：对任意可达状态的归纳证明。

总定理 `reachable_safe` 已由 Lean 机器检查：

```lean
theorem reachable_safe {world : World} (reachable : Reachable world) : Safe world
```

`Safe` 的十二项条件均已证明逐步保持：资源归属、workspace 双向绑定、单 VM 执行、单活动网络槽、快照不可变、权限形状、fence 顺序、fence 后权限关闭、有效权限的真实运行时依据、稳定态静止、terminal 与终态的一致性，以及 Paused 快照绑定。转移级定理还证明所有带 actor 的事件经过 owner/system 授权、egress 只能命中静态 allowlist、文件事件使用归属正确的 workspace、RPC timeout 只写入 `outcomeUnknown`、Destroyed 不会离开 Destroyed，以及 tombstone 不会被清除。证明不含 `sorry`、`admit`、新增 `axiom` 或有限状态枚举。

这个结论只覆盖 Lean 中定义的抽象协议。它尚未证明某个 controller、E2B、Firecracker、Linux/KVM、文件系统或网络栈实现了这些 `Step`；实现到规范的 refinement 与 adapter 实验仍是后续工作。

## 验证

需要 Lean 4.30.0；`lean-toolchain` 会固定版本。

```sh
lake build
```

构建不需要 Python，也没有保留旧的有限 BFS 模型或报告。

## 文档

1. [研究计划](RESEARCH_PLAN.zh-CN.md)
2. [状态机设计](docs/state-machine.md)
3. [E2B 源码分析](docs/e2b-code-audit.md)
4. [Lean → Rust/Aeneas refinement 路线](docs/refinement-plan.md)
5. [实验与判定协议](docs/evaluation.md)
6. [固定源码证据](references/code-evidence.md)

E2B 代码阅读基线为 `e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`。本项目目前没有声称发现 E2B 漏洞，也没有声称验证整个 E2B 或 Firecracker。
