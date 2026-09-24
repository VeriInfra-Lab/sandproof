# Trace checker：实现与结果

本文记录 `model-based-testing-survey.md` 落地路径的第一步实现：轨迹格式、采集、checker v0、以及 Lean 侧可执行 monitor + soundness。对应路径步骤 1–5。

## 1. 已实现组件

| 组件 | 位置 | 作用 |
|---|---|---|
| 轨迹格式与映射 | `docs/trace-format.md` | 线格式、evt/obs 映射、未知语义 |
| 采集驱动 | `tools/trace/collect.py` | 在本地 harness 上跑正常/竞态，产出 JSONL |
| checker v0 | `tools/trace/check.py` | 基于 Lean 性质导出的规则判定轨迹 |
| Lean monitor | `SandboxModel/Monitor.lean` | 可执行 monitor + `trace_monitor_sound` |

## 2. 数据流

```text
collect.py（真实 api + Redis + Postgres + dummy node）
  → JSONL 轨迹（事件 + 部分状态）
check.py（evt/obs 映射 → 规则判定）
  → Conforms / Violates / Inconclusive
Lean Monitor.lean（同一规则的形式化版本 + soundness）
  → trace_monitor_sound: 合法 StepTrace 必被接受
```

## 3. checker 的规则

规则以可插拔方式注册在 `tools/trace/rules.py`，每条声明所需观测字段（`requires`），缺数据时自动跳过，因此逻辑完整而插桩可以逐步补齐。当前实现 **24 条规则**：

- 生命周期：`kill_irreversible`、`terminal_monotone`、`running_requires_start`、`pause_requires_snapshot`、`resume_requires_snapshot`、`running_requires_applied_policy`
- 身份稳定：`snapshot_manifest_stable`、`workspace_stable`、`file_policy_stable`
- 单调：`generation_monotone`、`epoch_monotone`
- 资源/静默：`unique_execution`、`unique_active_slot`、`reclaimed_not_current`、`stable_states_quiescent`、`active_slot_valid`、`durable_snapshot_immutable`、`fence_well_formed`
- 账本：`operation_stable`、`effect_admission`
- 准入：`event_authorized`、`egress_enforced`、`file_workspace_bound`
- 缺口检测：`stale_index`（对应 runtime#3566，性质尚未形式化，标 `model_gap`）

性质到规则的完整映射见 [checker 覆盖度](trace-coverage.md)（`tools/coverage.py` 生成）。

- 一旦观测到成功的 kill（映射为已接受的终止意图、`terminal = true`），后续不得出现成功的 resume 或 `running` 观测。
- 未知结果（timeout）：保留两种可能，若违反仅依赖 unknown 则判 Inconclusive。

自测：`python3 -m tools.trace.selftest`（22 条，覆盖每条规则的触发与“缺数据不误报”）。

## 4. 运行结果（`references/trace/`）

- `normal.jsonl` → **Conforms**：create→pause→resume→kill 的正常序列。
- `race-f1.jsonl` → **Violates**：kill（seq 5）成功后，resume（seq 7）成功、沙箱观测为 running（seq 8）。最小子序列 `[5,7]`、`[5,8]`。
- 判定输出见 `references/trace/verdicts.txt`。

这条 race 轨迹就是已上报的 E2B 问题（`e2b-dev/runtime#3636`）的最小事件历史，现在由 checker 自动判定，而不是人工阅读。

## 5. Lean 侧形式化

`SandboxModel/Monitor.lean`：

- `SandboxView`：只含 E2B 通过 API 可观察的字段（phase、terminal、generation）。
- `monitor : List (Option SandboxView) → Bool`：可执行判定。
- `StepTrace`：初始世界 + 每步合法 `Step`，每步携带其观测 view。
- **`trace_monitor_sound`**：若观测来自 `Reachable` 世界且相邻由合法 `Step` 连接，则 `monitor` 必接受。因此 checker 的拒绝意味着该观测历史不可由模型产生。
- 只依赖标准公理（`propext`、`Classical.choice`、`Quot.sound`），无 `sorry`/自定义 `axiom`。
- 可执行例子：正常序列 `= true`，F-1 形状（terminal 后 running）`= false`，均由 `decide` 机器检查。

这样 checker 的规则不是手写启发式，而是规范的一个**已验证投影**：checker v0（Python）是它的运行实例，`trace_monitor_sound` 是它的可靠性证明。

## 6. 校准与边界

已完成：
- 正例：正常生命周期轨迹接受。
- 反例：F-1 真实轨迹拒绝（与 `#3636` 一致）。

待办：
- **历史负例校准**：对 `84362fc6` 的 catalog 竞态、`eef64c7` 前的 stale 决策竞态跑 checker，必须命中。这两条属于路由/决策规则，需要先扩展 monitor 规则集（当前只有生命周期规则）。
- 扩展规则：`running` 必须先有成功 create/resume；effect commit-before-dispatch、cleanup 精确寻址等（需要存储层插桩）。
- 插桩下沉：当前只从 API + 观测采样得到事件；下一步在 store 层加结构化事件，覆盖内部转移。

## 7. 复现

```sh
# 前置：本地 harness 运行中（见 docs/e2b-f1-repro.md）
python3 tools/trace/collect.py normal > /tmp/n.jsonl
python3 tools/trace/collect.py race   > /tmp/r.jsonl
python3 tools/trace/check.py /tmp/n.jsonl   # Conforms
python3 tools/trace/check.py /tmp/r.jsonl   # Violates
```
