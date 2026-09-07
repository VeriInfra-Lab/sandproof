# 状态机：generation-scoped capability lifecycle

本文描述当前 Lean 实现。规范的核心不是一个生命周期枚举，而是 **沙箱逻辑状态、资源事实、执行权限和 worker fence 的乘积状态**。快照保存计算状态；是否还能执行由当前 controller 记录和 worker fence 决定，旧快照不能携带永久执行权。

## 1. 固定范围

首版固定：单台 Linux/KVM 主机、一个 controller、一个 worker、Firecracker v1.16.1；支持多个 tenant 和多个 sandbox。每个 sandbox 有独占 workspace、VM、snapshot 和 network slot。

纳入 create、pause、resume、kill、managed exec、文件读写、业务 ingress/egress、RPC timeout、controller crash/recover，以及命令重复或迟到。

暂不纳入多节点迁移、共享卷、POSIX ACL、host mount、独立 snapshot/fork API、DNS/SNI 动态策略、任意 egress proxy、恶意宿主、资源配额、计费和 liveness。网络策略只有 `denyAll` 或固定 Endpoint 列表；Endpoint 是 address/prefix、TCP/UDP 与 port 的 L3/L4 条目。

## 2. 为什么身份和代必须分开

Lean 为 tenant、sandbox、VM、filesystem、snapshot、network slot 和 operation 使用不同类型。所有集合都用函数映射表示，没有固定数量上界。

每个 sandbox 同时带两个单调量：

- `epoch`：一次生命周期意图的版本。worker 只接受与当前 `(epoch, terminal)` 一致的副作用。
- `generation`：一次可运行实例的代。resume 会增加 generation；exec、文件和网络 capability 都绑定 generation 与具体资源。

`terminal` 是 tombstone。kill 接受时立即置为 true；后续 fence 和资源清理可以异步完成，但任何转移都不能再清除它。

## 3. World

`World` 是参数化全局状态：

| 字段 | 含义 |
|---|---|
| `sandboxes` | owner、phase、terminal、epoch、generation、资源引用、网络策略、desired/effective capability、当前 operation |
| `vms` | VM 的 owner、sandbox、generation 与 prepared/executing/frozen/stopped 状态 |
| `filesystems` | 独占 workspace 的 owner、sandbox 与 generation |
| `snapshots` | owner、sandbox、source generation、workspace、LocalSealed/Durable 与 immutable |
| `netSlots` | owner、sandbox、generation、VM、策略、configured/active |
| `workerFences` | worker 已安装的 `(epoch, terminal)` |
| `inflight` | 可重复、延迟、乱序交付的 command 集合 |
| `controllerUp` | controller crash/recover 的易失状态 |

`desired` 表示 controller 要达到的权限，`effective` 表示当前实际可用的权限。权限不是任意布尔集合，而是只有三种构造：`closed`、绑定 sid/generation/VM/workspace/network slot/policy 的 `running`，以及绑定 sid/snapshot 的 `paused`。因此“不合法的权限组合”无法靠一串 `if/else` 拼出来。

pause/kill 接受时先关闭 desired；worker 安装撤销 fence 后才清除 effective，因此模型保留了真实的撤销窗口。启动 fence 与撤销 fence 是两个不同的 Step：启动 fence 只登记 epoch，撤销 fence 才有权关闭 effective 和网络槽。这个区分阻止迟到的 create/resume fence 在 Running 中误撤销当前权限。

## 4. Phase 与 capability

```mermaid
stateDiagram-v2
    [*] --> Starting: create accepted
    Starting --> Running: VM executing + network configured + publish
    Running --> Pausing: pause accepted; epoch++
    Pausing --> Paused: fence + frozen + durable snapshot + stopped
    Paused --> Starting: resume accepted; epoch++, generation++
    Starting --> Revoking: kill accepted
    Running --> Revoking: kill accepted
    Pausing --> Revoking: kill accepted
    Paused --> Revoking: kill accepted
    Revoking --> Destroyed: terminal fence + quiescence
    Destroyed --> Destroyed: stale delivery / observation
```

Running 的 capability 包含当前 generation/VM 的 exec，当前 generation/workspace 的 file read/write，以及当前 generation/VM/network slot/policy 的 ingress/egress。Paused 只保留绑定 durable snapshot 的 resume capability。Starting 与 Destroyed 没有 effective capability；Pausing/Revoking 在 fence 生效前可暂时保留旧 data-plane capability。

## 5. 异步转移

规范使用：

```lean
inductive Step : World → Event → World → Prop
inductive Reachable : World → Prop
```

每个构造子都是一个带 guard 的原子协议事件。主要路径如下：

| 路径 | 分解的事件 | 关键 guard / 发布条件 |
|---|---|---|
| create | AcceptCreate → InstallFence → DispatchPrepare → PrepareVm → ConfigureNetwork → ActivateVm → PublishRunning | sid/workspace/VM/slot 新鲜；worker fence 当前；激活前同 sid 无 executing VM；发布前同 sid 无旧 active slot |
| pause | AcceptPause → InstallFence → FreezeVm → SealSnapshot → CommitSnapshot → StopVm → PublishPaused | fence 先关闭权限；snapshot durable+immutable 且绑定 owner/sid/generation/workspace；无 executing VM 和 active slot |
| resume | AcceptResume → InstallFence → create 的准备/激活路径 | snapshot 必须 durable、immutable 且归属正确；新 epoch/generation；旧命令不能匹配新记录 |
| kill | AcceptKill → InstallFence → StopVm → PublishDestroyed | terminal 在接受时写入；Destroyed 仅在 terminal fence、无 executing VM、无 active slot 后发布 |

controller crash 只改变 `controllerUp`，不会抹掉持久状态、worker fence 或在途命令。RPC timeout 只把当前 operation 标为 `outcomeUnknown`，不反向修改 VM、快照或 phase。重复/迟到命令可以 stutter；真正产生副作用的 worker 事件仍要重新核对 epoch、generation、资源身份和 fence。

文件与网络操作是自循环事件：它们不改变 World，但只有 owner/systemController 获得正确 capability 时才能发生。Egress 还要求目标属于当前静态 allowlist；`denyAll` 不产生合法 egress 转移。

## 6. 安全目标

[`Safety.lean`](../SandboxModel/Safety.lean) 中的完整 `Safe` 包含十六项条件：

1. 所有 VM、workspace、snapshot、network slot 的 owner/sandbox 归属一致。
2. 每个 sandbox 记录指向的 workspace 真实存在，并反向绑定同一个 owner/sid。
3. `currentVm` 与 `netSlot` 即使在权限关闭窗口也必须存在且绑定当前 owner/sid/generation；slot 还必须绑定 current VM 与固定 policy。
4. 同一 sandbox 至多一个 executing VM。
5. 同一 sandbox 至多一个 active network slot。
6. 每个 active slot 都是其 sandbox 当前记录中的 configured slot，不允许旧路由悬空。
7. Durable snapshot 必为 immutable。
8. effective authority 与 phase、generation、具体资源完全匹配。
9. worker fence 来自现存 sandbox，epoch 不超前，terminal fence 不会凭空出现。
10. Pausing/Revoking 的当前 worker fence 会关闭 effective authority。
11. 非关闭、非 Paused 的 effective authority 有 executing VM 与 configured/active network slot 作为事实依据。
12. Paused/Destroyed 中无 executing VM 与 active slot。
13. terminal 为真当且仅当 phase 是 Revoking/Destroyed。
14. Paused 指向归属、代、workspace、durability 都正确的 snapshot。
15. resume 后 Starting 中的 `restoreFrom` 必须属于同 owner/sid/workspace、durable+immutable，且来源代正好是当前代的前一代。
16. 当前 operation 的 epoch、kind、pending/unknown/succeeded 结果与 phase 一致。

`emptyWorld_safe` 证明初始状态满足全部条件；`step_preserves_safe` 证明任意合法一步同时保持全部条件；最终由可达性归纳得到：

```lean
theorem reachable_safe {world : World} (reachable : Reachable world) : Safe world
```

这不是有限场景搜索。`World` 中的标识、资源数量和轨迹长度都没有预设上限。转移级定理另外证明：带 actor 的事件经过 owner/system 授权、egress 目标属于静态 allowlist、文件事件使用归属正确的 workspace、RPC timeout 只把 pending operation 写成 `outcomeUnknown`、Destroyed 不会离开 Destroyed，已有 terminal tombstone 也不会被任何一步清除。`Scenarios.lean` 还构造了完整正常轨迹；`Counterexamples.lean` 证明删除 VM/网络互斥、静止发布或恢复源身份检查会分别打破对应性质。所有证明均不含 `sorry`、`admit` 或新增 `axiom`。

## 7. 用简单比喻理解边界

可以把一个 sandbox 想成一间实验室：

- `SandboxRecord` 是前台登记簿；VM、workspace、snapshot 和 network slot 是实际钥匙与设备。
- `generation` 是本轮钥匙的批次号。恢复后会换一批钥匙，旧钥匙不能打开新设备。
- `effective authority` 是眼下真正能开门的钥匙，不是前台“希望门关上”的记录。
- `worker fence` 是门卫已经执行到哪个命令的盖章。只有撤销盖章完成，系统才能把实验室发布为 Paused 或 Destroyed。

Lean 已证明：只要每一次真实动作都确实属于 `Step` 允许的动作，上述十六条规则永远不会被打破。Lean 没有证明真实门卫一定照做；把 controller、worker、Firecracker 和 Linux 的动作对应到 `Step`，属于后续 refinement 与 adapter 验证。

具体边界如下：

| 模型内已证明 | 模型外仍需验证 |
|---|---|
| 任意多个 tenant/sandbox、任意有限长度的合法 Step 历史 | controller/worker 代码是否只产生这些 Step |
| 重复/迟到命令、RPC outcome unknown、controller crash/recover | host/worker 永久故障后的活性与最终恢复 |
| owner/sid/generation/resource 绑定与静态 L3/L4 策略 | KVM/Firecracker、Linux 内核、nftables/netlink 没有 bug |
| workspace 是私有资源，Committed snapshot 不可变 | POSIX ACL、路径穿越、symlink/hardlink、host mount 的具体实现 |
| pause/kill 完成时无 executing VM 与 active slot | 内存清零、磁盘安全擦除、侧信道与恶意宿主 |
| 单节点 controller/worker 协议安全 | 多节点调度、迁移、共识、共享卷、动态 DNS/SNI 策略 |

“任意历史”也有限定：`Reachable` 表示任意**有限**步数。证明是 safety，即坏状态永远到不了；它不保证某个请求一定最终完成，也不保证系统不会一直等待。

## 8. 设计上刻意保留的窗口

- `AcceptPause` 或 `AcceptKill` 与 `InstallFence` 分开。在 fence 生效前旧 VM 可能继续执行，所以此时不能发布 Paused/Destroyed。
- `SealSnapshot` 与 `CommitSnapshot` 分开。本地封口不等于持久且不可变。
- `ActivateVm` 与 `PublishRunning` 分开。VM 可能已经执行而回执未知；timeout 不能把对象伪装回 Paused。
- controller 判断命令 current 还不够。worker 必须在实际副作用处检查 fence；否则旧命令即使不回调也能启动 VM。

这些窗口让规范能够表达真正危险的交错，同时把首篇工作限制在一个可证明的单节点协议内。
