# 插桩与综合测试

本文记录把本地 E2B harness 的内部状态投影为抽象观测（插桩），以及在此之上的综合场景测试结果。对应工作：`tools/trace/harness.py`、`tools/trace/observe.py`、`tools/trace/collect.py`、`tools/trace/run_scenarios.py`。

## 1. 插桩：真实存储 → 抽象 obs

`Observer.sample(sid)` 从真实 E2B 组件采样，映射到 checker 使用的抽象字段：

| 抽象字段 | 真实来源 |
|---|---|
| `running` | running-store 键 `sandbox:storage:{team}:sandboxes:{sid}` 是否存在 |
| `phase_hint` | running 记录的 `state`；否则由快照行推导 paused/deleted/absent |
| `snapshot_present` / `snapshot_deleted` | Postgres `snapshots` 行 + 其 env 的 `deleted_at` |
| `workspace` | running 记录 `baseTemplateID`（稳定代理） |
| `execution_id` | running 记录 `executionID`（代身份代理） |
| `index_members` | team index SET `sandbox:storage:{team}:index` |
| `existing_sandboxes` | running-store 键集合 |
| `catalog_present` | `sandbox:catalog:{sid}` |

**映射纪律的修正（实测发现）**：最初把 running 记录的 `templateID` 映射为 `file_policy`，综合测试立刻报 `file_policy_stable` 违例。核查发现 E2B 在 resume 后把 `templateID` 指向快照 env（合法的新代表示），它不是稳定的策略身份。因此**移除该映射**——E2B 不暴露忠实的 file-policy 身份，不应硬造。这正是综合测试的价值：暴露了错误的 obs 映射。

## 2. 综合场景

`tools/trace/collect.py` 提供场景，`tools/trace/run_scenarios.py` 全部跑一遍并判定：

| 场景 | 内容 | 结果 |
|---|---|---|
| `normal` | create→pause→resume→kill | Conforms |
| `race` | F-1：resume 在途时 kill | **Violates** |
| `dup_kill` / `dup_pause` | 重复 kill / pause | Conforms |
| `resume_resume` | 并发两个 resume | Conforms |
| `kill_resume` / `kill_pause` | 终态后 resume/pause | Conforms |
| `multi` | 3 个沙箱交错生命周期 | Conforms |
| `stale_index_control` | 注入假索引成员（**正例对照**） | **Violates**（规则生效） |
| `expire` | 短 TTL 等待驱逐后采样 | Conforms |
| `stress` | 4 沙箱并发 resume×kill | **Violates**（F-1 系统性复现） |
| `bad_create` | 非法模板创建失败后检查孤儿/索引 | Conforms（无孤儿） |
| `race_pause_kill` | kill 与 pause 并发（节点 pause 延迟） | **Violates**（pause-after-kill，G-2） |
| `client_timeout` | 客户端 1s 超时于慢 create | **Violates**（节点孤儿，G-1） |
| `race_double_kill` / `kill_settle` / `autopause_resume` | 并发双 kill、kill 后节点残留、auto-pause 后 resume | Conforms |

第二轮发现（G-1 孤儿、G-2 pause-after-kill）详见 [E2B 第二轮测试发现](e2b-round2-findings.md)。

汇总：11 个场景，8 Conforms，3 Violates；命中的规则：

```
kill_irreversible: 8
terminal_monotone: 3
resume_requires_snapshot: 3
stale_index: 1   (正例对照)
```

## 3. 结论

- **F-1 是系统性的**：`stress` 场景在 4 个并发沙箱里让 2 个同时复现，证明不是偶发。
- **插桩有效**：`stale_index_control` 注入假成员后规则立即命中，证明“观测→规则”链路真的工作，而不是空转。
- **没有发现 F-1 之外的新 safety 违例**：11 个场景中其余全 Conforms。
- **未复现 runtime#3566**：`expire` 场景的自然驱逐同时清理了索引，没有产生陈旧成员；说明该 issue 依赖“键在非 Remove 路径丢失”的特定条件，本 harness 未触发。

## 4. 覆盖边界与下一步

当前插桩能喂的规则集中在**生命周期 + 索引/执行身份**。以下性质需要更深的插桩才能实时喂数据：

- 节点/VM/slot 状态（`unique_execution`、`active_slot_valid`、`reclaimed_not_current`、`stable_states_quiescent`）
- effect/operation 账本（`effect_admission`、`operation_stable`）
- authority/fence（`fence_well_formed`、`fence_closes_authority`、`authority_shape`）
- cleanup 事件（`reclaimed_not_current`、`cleanupAdmission`）

要测出这些维度上的问题，需要在 orchestrator/存储层加结构化事件（或读取节点 live map / effects 表）。当前 E2B 没有与模型一一对应的 effect 账本，映射本身是开放问题。

## 5. 复现

```sh
# 前置：本地 harness 运行中
python3 tools/trace/run_scenarios.py        # 全部场景 + 判定 + summary.json
python3 tools/trace/collect.py race         # 单个场景输出 JSONL
python3 tools/trace/check.py references/trace/scenarios/race.jsonl
```
