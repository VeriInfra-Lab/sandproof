# 模型扩展：accounting / compensation / liveness

针对 [进展总览](progress-overview.md) 中的安全形状缺口，新增三个 Lean 模块，均已机器检查、无 `sorry`。

## 1. `Accounting.lean`：陈旧索引/引用（runtime#3566）

**建模来源**：固定源码版本中的 `ExpiredItems()` 路径只删除 store 条目、未在同一步更新 team-index SET；模型把这种可能形成陈旧引用的代码形状作为校准样例，不据此单独宣称某个部署已经出现系统漏洞。

**形式化**：一个 `State` = 活跃 store + 发布索引，`Consistent` 要求二者指向同一集合：

```lean
def Consistent (s : State) : Prop := ∀ sid, s.index sid ↔ s.store sid
```

- `consistent_insert` / `consistent_removeBoth`：成对增删保持一致性。
- `consistent_healIndex`：从 store 重建索引可修复。
- **`sweepStoreOnly_is_stale`**：只删 store 的清扫会**打破一致性**——被删的沙箱仍留在索引里成为陈旧引用。
- **`sweepStoreOnly_leaves_stale`**：显式给出“store 中没有、索引中仍有”的坏状态。

这把 #3566 的代码形状变成了一条**机器检查的定理**：安全性质“索引只引用活跃沙箱”在正确维护下保持，在省略索引更新的抽象路径下被违反。它证明的是模型内的因果关系，不是对任意 E2B 部署的复现结论。

## 2. `Compensation.lean`：主状态机内的失败补偿

早期版本把补偿写成独立 `abortCreate` 函数。它不是 `Step`，不属于 `Reachable`，而且会绕开 operation 不变量，因此不能算协议级证明。该路径已删除。

现在的补偿使用主 `Event/Step/Reachable` 关系：

```text
Starting(create/resume) or Pausing(pause)
  -- beginCompensation(failure) -->
Revoking + terminal fence + operation.compensating(failure)
  -- stop/reclaim resources -->
Destroyed + operation.failed(failure)
```

已机器证明：

- `beginCompensation_closes_authority`：补偿开始后进入 terminal revoking，desired/effective authority 都关闭；
- `publishFailure_requires_closed_resources`：所有 VM 必须已 `stopped/reclaimed`，且所有 slot inactive；
- `publishFailure_finalizes_operation`：保留原 operation 身份，以同一 failure 原因写入最终状态和结果；
- `step_protocol_secure`、`reachable_protocol_secure`：补偿步骤与其他生命周期步骤使用同一安全证明；
- `Scenarios.create_failure_trace`：从 create 到 compensation fence 再到 failed/destroyed 的完整可执行见证。
- `Scenarios.controller_accepts_pause_failure_compensation`：pause 的存储失败也进入同一失败关闭路径。

## 3. `Liveness.lean` / `ControllerLiveness.lean`：条件化完成性

**问题**：安全只证“坏事不发生”，不证“最终会完成”。

**形式化**：早期先用阶段级转移证明“存在一条完成路径”；现在进一步定义允许空转的无限执行、时间性质、连接真实处理函数的具体工作项和弱公平调度。操作记录持久性与最终结果稳定性也已提升到整条执行轨迹。

最终定理的含义是：若未完成操作总有一个持续可执行的工作项，公平调度最终执行它，并且包含外部响应、有限重试和安全清理在内的自然数进度量会严格下降，那么该操作最终写入成功或明确失败，并从此永久保留该结果。

**诚实的边界**：公平调度不能让永久故障的虚拟机、存储或网络自动成功。当前定理把这些必要条件放在显式进度合同里；还需要为 create/pause/resume/kill 分别给出具体进度量和适配器响应/安全清理条件，才能闭合逐工作流证明。运行时 checker 对以过渡状态结束的有限轨迹仍只能判为 Inconclusive，因为有限观测不能证明未来永不完成。

## 4. 与缺口清单的对应

| 缺口 | 现在状态 |
|---|---|
| 陈旧索引/引用 | **已形式化并证明**（Accounting），检测规则 `stale_index`，正例对照通过 |
| 部分失败清理 | **已接入主 Step/Reachable 并证明安全**；下一步是故障后必然收敛的 liveness |
| 缓存读一致性 | 归属为 accounting 的一致性问题（Accounting 覆盖“索引与 store 一致”）；通用缓存一致性仍待定义 |
| liveness | **弱公平与真实处理函数已连接，条件化最终完成已证**；逐工作流进度量和适配器收敛条件仍待闭合 |
| adapter | 明确出范围（Linux/FC 内部），保持合同+实验 |

## 5. 验证

```sh
lake build                                   # 15 jobs
python3 -m tools.trace.selftest              # 26 tests
python3 tools/trace/run_scenarios.py         # 12 scenarios
```

主模型、补偿路径和场景见证均通过 `lake build`，无 `sorry`、`admit` 或新增自定义公理。
