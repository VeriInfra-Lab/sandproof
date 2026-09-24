# 系统一致性审计：用状态机检查 E2B 的方法

状态：方法定义完成；映射表与 findings ledger 尚未填充。本文回答一个问题：如何把"用 Lean 状态机找 E2B 与规范矛盾之处"从逐个阅读源码的启发式工作，变成可枚举、可覆盖记账、对新 commit 可复跑的系统流程。

与 [第一次审计](e2b-bug-finding.md) 的关系：那次审计给出了审计尺子的雏形（每条 Lean 性质翻译成源码危险形状），并用两个历史修复做了演示。本文把尺子组织成完整流程，并明确"已知 issue 只用于校准尺子，不用作发现目标"。

## 1. "与状态机矛盾"的精确定义

Lean 定理都是蕴含式：`Reachable world → ProtocolSecurity world` 和 `Step world event next → ProtocolStepSecurity world event next`。它说的是"如果事件以模型要求的 guard 被接纳，则安全性质成立"。E2B 不被要求实现这些 guard——这正是审计要检查的对象。

因此需要一个固定的观察层：

1. **状态映射 `obs`**：Lean `World` 的每个字段 → E2B 中可观察的存储与键（Redis 键、PG 表、进程内 map、Firecracker 状态）。
2. **事件映射 `evt`**：E2B 的每个生命周期相关副作用点 → 一个 `Event` 构造子，或显式声明"无对应事件"。
3. **角色映射**：每个副作用点的执行者是 controller 角色（E2B api 服务）还是 worker 角色（orchestrator 节点 / fcgate）。模型中 guard 分布在两侧，审计必须在正确的位置找检查。

固定这两份映射后，**conformance failure** 才有定义：

> 存在一个 E2B 可实现的历史，其经 `obs`/`evt` 投影后，满足模型前提（投影为合法 `Reachable` 世界与合法 `Step`），但违反某个已证明定理的结论。

不满足这个定义的发现按 [evaluation.md §6](evaluation.md) 分类：Contract difference、Unsupported configuration、Inconclusive。这个区分是纪律性的核心：**先固定映射，再谈违反；不能先看到坏结果，再临时构造映射。**

审计问题因此分成两类，且只有两类：

- **A 类（事件映射级）**：E2B 有生命周期相关副作用，但任何 `Step` 构造子都不允许它，或它绕开了所有映射事件。
- **B 类（历史级）**：在固定映射下，E2B 能实现违反定理结论的历史。

A 类的处置优先级：先判断是否为模型缺口（真实合同中存在的动作，模型没建模），再判断是否为 bug 候选。模型缺口的正确产出是扩展 `Event`/`Step` 并补证明，而不是硬套现有性质。

## 2. 四份工作产物

| 产物 | 内容 | 形式 |
|---|---|---|
| `obs-map` | World 字段 → E2B 可观察点；不可观察字段显式声明 | JSON，`references/conformance-maps/` |
| `evt-map` | 副作用点（文件+函数+行） → Event 构造子 + 角色 + store 类型 | JSON，同上 |
| guard 表 | 每个 `Step` 构造子的完整 guard 清单 → E2B 检查位置与原子域 | JSON，同上 |
| findings ledger | 每个 diff 的最小历史、可构造性证据、判定 | JSON + 报告 |

guard 表可以从 `Step.lean` 直接机械导出（每个构造子的前提清单），不需要人工再总结一遍。E2B 侧的检查位置与原子域必须逐点记录，因为同一 guard 在 Lua 脚本里和在锁外 Go 代码里的安全含义完全不同。

## 3. Layer A：副作用点的机械枚举

枚举必须按副作用类别做，而不是按业务流程读代码。基于 pin 到 `82f6529` 的本地树，类别与范围：

| 类别 | 机械模式（grep/AST 层面） | 覆盖组件 |
|---|---|---|
| 控制面存储写 | Redis 脚本调用、`Store`/`Delete`/`Upsert` 方法、PG 写、进程内 map 写 | `sandbox-catalog`、`sandbox/storage/redis`、`db`、`orchestrator/pkg/sandbox/map.go`、`edge` 路由 |
| 节点级生命周期 RPC | orchestrator 与 fcgate 的 gRPC 调用点（Pause/Create/Checkpoint/Delete/Stop） | `api/internal/orchestrator/*.go`、`api/internal/fcgate`、`orchestrator/pkg/server/sandboxes.go` |
| 路由与代理准入 | 路由表删除、live map 删除、连接池分组、token 发放 | `proxy`、`edge`、`sandbox_envd_secret.go` |
| 异步后台动作 | goroutine/队列里脱离请求 context 的写与删除 | `evictor`、snapshot 上传、pause 后台停止 |
| 用户可见能力 admit | exec/file/ingress 的数据面入口与 phase 检查 | client-proxy、节点 sandbox proxy |

完整性的声称标准：每类有一个可验证的机械模式集合；每个组件被标记为 `enumerated` / `partial` / `out-of-scope`（含理由）。没有这个标记，"系统性"就没有可检验的含义。

**显式超出范围**（首次声明，避免静默遗漏）：`sandbox_fork.go`（snapshot fork 未建模）、`sandbox_network_update.go`（动态网络策略，对应 `7faa59e` 修复的领域，首版固定静态 policy）、auto-resume（有意关闭）、`sandbox_timeout.go` 的计时语义（模型只有 `rpcTimeout` 抽象）。它们的状态是 Unsupported configuration，不是 Conforms。

## 4. Layer B：单点 guard-diff

对每个映射到 `Step` 构造子 `c` 的副作用点，从 guard 表取下 guard 集合 `G(c)`，在 E2B 侧抽取实际检查 `A(p)`，计算 `G(c) \ A(p)`。`Step.lean` 的 guard 库存按构造子分类包括：

| 构造子 | 必备 guard（摘要） |
|---|---|
| `acceptCreate/acceptPause/acceptResume/acceptKill` | controllerUp、phase 前提、terminal 未置、**fresh operation key**、authorized actor、（resume）`usableSnapshot` 七元组 |
| `dispatchEffect` / `acknowledgeEffect` | committed 前置 / dispatched 前置 + `commandResultPresent`（精确身份的结果已存在） |
| `installFence`（两种） | inflight + dispatched、phase 前提、`fenceCanAdvance`（epoch 单调、terminal 不倒退） |
| `dispatchPrepare`/`prepareVm` | fenced 精确 `(sid, epoch, false)`、fresh VM/slot、**restoreFrom 原子选定**且 `usableSnapshot` |
| `activateVm` | prepared 状态、slot 已 configured、`noExecutingVm` 互斥、fenced |
| `applyFilePolicy` | fenced、starting、live；receipt 精确 `(sid, generation, workspace, policy)` |
| `publishRunning` | VM executing、slot 七元组就绪、`noActiveSlot`、policy receipt 精确、**policy effect 已精确 ack**、当前 operation 在账本中且无结果 |
| `publishPaused` / `publishDestroyed` | snapshot durable+immutable+绑定 / terminal fence、`noExecutingVm`、`noActiveSlot`、operation 匹配账本 |
| `cleanupVm`/`cleanupSlot` | inflight + dispatched、身份精确三元组、stopped/inactive、`notCurrent` |
| `rpcTimeout` | 只能标记匹配的未完成账本记录 |

guard-diff 有两个独立的失败方向，都要查：

1. **缺失**：E2B 没有对应检查。如修复前的 evictor 不携带 `ExpectExecutionID`（E27）。
2. **错位**：检查存在，但位置错误。这细分为两个尺子：
   - **原子性尺子**（`E2BAudit.lean` 形式化过）：检查与副作用必须在同一原子域。对每个 store 分类并发域——Redis Lua/server-side compare-and-delete（原子）、PG 事务（原子）、进程内 mutex（只保护本进程）、锁外序列（split）。`GET → 比较 → SET/DEL` 的 split 形状即为候选，无论比较逻辑本身多正确（E25）。
   - **时点尺子**：模型在 effect 执行点检查 guard，不在接受点。E2B 在请求入口检查、在副作用点不复查的路径是候选（如 `startRemoving` 在锁外做事后写回）。

判定为 guard-diff 不等于 bug：升级路径见 Layer E。

## 5. Layer C：字段写审计

Layer B 检查"事件被正确接纳"；Layer C 检查"字段从不被非法写"。对每个单调/稳定字段，枚举其在 `obs-map` 下**全部**写点，逐点检查是否被对应性质承认：

| 字段 | Lean 性质 | 审计问题 |
|---|---|---|
| terminal | `step_never_clears_terminal`、`TerminalConsistent` | running store 的所有 upsert 路径中，是否存在把已 terminal 的记录写回非 terminal 状态的路径（含 kill 后迟到 callback、pause refusal 恢复路径）？ |
| generation/ExecutionID | `step_generationMonotone`、stale admission | 所有携带 ExecutionID 的写入是否从当前记录读取并核对？哪些路径用 `sid` 单独寻址？ |
| epoch/fence | `step_workerFenceMonotone`、`FenceWellFormed` | fence 安装点是否检查了 epoch 单调？是否存在 fence 被更旧值覆盖的窗口？ |
| operation 身份/结果 | `OperationIdentityStable`、`OperationResultStable` | 同一 operation key 下，kind/epoch/input 是否可能被重写？timeout、context 取消、重试是否会覆盖已持久化结果（对照 E28/E29 修复的形状）？ |
| effect 结果 | `EffectResultStable`、`EffectAdmissionSafe` | 每个 RPC dispatch 前是否有持久记录？ack 是否按精确 EffectId 匹配，而非按"当前 phase 看起来合理"？ |
| manifest/workspace 绑定 | `SnapshotManifestStable`、`WorkspaceIdentityStable`、`PausedSnapshotValid` | 恢复路径是否按 manifest+workspace+来源 generation 核对，还是只按可变名字查找？ |
| 稳定态静默 | `StableStatesQuiescent` | paused/destroyed 后是否仍有路径引入新流量或执行（对照 H4 类问题，但由字段审计而非 issue 触发）？ |

Layer C 与 Layer B 的分工：Layer B 问"这个动作缺不缺 guard"，Layer C 问"不管哪个动作干的，这个字段的值是否可能倒退"。修复历史中的 `eef64c7` 两个尺子都命中（决策不携带 ID = B 类缺失；running 状态被旧决策改写 = C 类倒退）。

## 6. Layer D：准入审计

用户可见能力（`exec`/`fileRead`/`fileWrite`/`ingress`/`egress`）的 admit 点映射到数据面：client-proxy → 节点 proxy → envd，以及 token 生成与 auto-resume 入口。对照性质：

- `EventAuthorized`：每个 admit 点是否验证调用者与 owner 的关系；
- `noFileAuthorityOutsideRunning` / `staleGenerationCannotAccessWorkspace`：非 Running 或旧 generation 的 admit 是否被拒绝；auto-resume 路径作为显式的 `acceptResume` 映射单独审计，不得混入静默唤醒；
- `EgressPolicyEnforced`：egress 的 allowlist 检查点与生效点的时序（动态更新不在范围，但静态 check-act split 仍在范围内）；
- `RunningRequiresAppliedFilePolicy`：数据面放行文件操作前，policy receipt 是否已是当前 generation。

## 7. Layer E：历史构造与证据升级

guard-diff / 字段写 diff 只是候选。升级到 Violates 需要：

1. 在模型中写出最小事件历史（对照 `Scenarios.lean` 的构造方式），确认它违反哪条已证明定理；
2. 检查可构造性：真实 API 序列、真实并发域、real-time 约束、E2B 现有其他保护是否已经覆盖（这常常反驳候选）；
3. 按 [evaluation.md](evaluation.md) 记录证据：原始 HTTP/gRPC 状态、ExecutionID/LifecycleID、节点、时间线；单看迟到 stdout、404、进程短暂共存不构成判定；
4. delta debugging 缩到最小历史。

分类不变式：**Conforms 需要正面记录等价机制**（例如 E2B 用 Lua 原子性替代我们模型的单步原子性——机制不同但 guard 等价），而不是"没找到问题"。每个映射点最终必须是五类之一，无默认。

## 8. Layer F：校准

方法本身的灵敏度需要被校准，否则覆盖记账只是形式：

- **负例控制**：对修复前提交 `84362fc6`（E25 所在）跑 Layer B/C，必须命中非原子 compare-and-delete 形状；对 `eef64c7` 之前的历史跑 Layer C，必须命中 stale 决策。跑不中说明枚举模式或映射表有漏，先修尺子再审计。
- **正例控制**：`Scenarios.lean` 已证明模型允许正常生命周期；Layer B/C 不得把正常路径判为 violation。
- **模型内灵敏度**：`Counterexamples.lean` 已机器证明每个关键 guard 删除后对应性质失败，即尺子在模型侧是完备敏感的；模型侧不需要再校准，需要校准的是 E2B 侧的枚举与映射。

## 9. 覆盖记账与复跑

每轮审计 = 固定新 commit → 更新三份映射 → 跑 Layer A–E → 更新 findings ledger。组件级覆盖矩阵（类别 × 组件 × Layer 状态）随代码变化增量维护：E2B 新增的副作用点必须落入 `evt-map` 或被声明 out-of-scope，两者都不做即审计不完整。

ledger 条目最小字段：

```json
{
  "id": "F-001",
  "evt_map_ref": "catalog_redis.go:DeleteSandbox",
  "layer": "B",
  "guard_diff": ["notCurrent 检查与删除非原子"],
  "lean_property": "ReclaimedResourcesNotCurrent",
  "minimal_history": "GET(exec1) → Store(exec2) → DEL",
  "constructible": {"api": true, "concurrency": "split redis ops", "other_guards": null},
  "verdict": "violates | conforms | contract_difference | inconclusive | unsupported | model_gap",
  "evidence": ["E25", "E26"]
}
```

## 10. 方法不能发现什么

沿用 [state-machine.md §9](state-machine.md#9-lean-证明到哪里为止) 的边界：文件内容与 ACL 忠实性（E2B issue #884 类）、guest 内进程与 TCP 状态（infra issue #3292 类）、journal 断电原子性、KVM/内核正确性、liveness（永久 pending，E28/E29 类需要额外的审计观察层才能筛查）。这些字段在 `obs-map` 中显式标记为不可观察，性质依赖它们时自动判定为 Inconclusive，而不是假装覆盖。

## 11. 与 Lean 证明的关系

本方法不修改规范来源。Layer B 的 guard 表从 `Step.lean` 机械导出；Layer C 的字段清单从 `Safety.lean` 的单调/稳定性质导出；升级路径中的历史违反必须引用具体定理名。若发现模型缺口（A 类），正确的动作是回到 `Types.lean`/`Step.lean`/`Safety.lean`/`Proofs.lean` 扩展并重新证明，而不是在审计报告中绕开规范。
