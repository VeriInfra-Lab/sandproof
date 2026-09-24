# Issue 语料第一遍判定（待人工复核）

本文是 317 条边界候选的第一遍 triage，供人工复核。原始数据 `references/issues/ledger-triaged.jsonl`，人工判定建议 `references/issues/proposal.json`。

## 1. 方法

- **自动过滤**（`tools/issues/study.py filter`）：关键词边界筛选，317/642 进入候选。
- **自动 triage**（`tools/issues/triage.py`）：保守规则，只自动判定“明显出范围”的类别，其余留 `needs-review`。
- **人工第一遍**（本文 + `proposal.json`）：逐条看标题/标签，对确实落在边界内的后端生命周期问题给出建议判定。

启发式噪声很大（`route`/`access`/`team` 命中大量无关项），所以 `proposal.json` 才是第一遍结论，自动 triage 只用于分桶。

## 2. 分布（自动 triage）

| 提出判定 | 数量 |
|---|---|
| out-of-scope（SDK/build/envd/feature/perf） | 164 |
| needs-review | 109 |
| liveness | 26 |
| adapter | 13 |
| violates | 5 |

## 3. 边界内问题的建议判定（`proposal.json`）

### 3.1 被现有性质覆盖（`violates`）

| issue | 性质 | 说明 |
|---|---|---|
| runtime#3636 | `TerminalConsistent` | F-1 kill×resume，已复现，checker 规则 `kill_irreversible` |
| runtime#2637/#3178/#3354/#3573 | `ResourceOwnership/SnapshotOwnership` | 快照查询未按 team 限定 → 跨团队访问 |
| runtime#2836 | `SnapshotOwnership` | 共享 base template 导致跨团队 resume |
| runtime#1763 | `ActiveSlotValid` | OrchestratorIP 被空串覆盖，路由身份错误 |
| runtime#3595（待定） | `TerminalConsistent/StableStatesQuiescent` | StartRemoving 未广播，缓存仍显示 Running |

### 3.2 Model-gap 候选（需要回到 Lean 讨论）

| issue | 候选缺口 | 说明 |
|---|---|---|
| runtime#3566 | **stale-index-accounting（缺失）** | 正文确认：`ExpiredItems()` 清 ZSET 但不清 team-index SET，陈旧成员累积。没有性质禁止“陈旧引用”，是 safety 形状缺口。已实现检测规则 `stale_index`（性质待形式化，标 `model_gap`）。 |
| runtime#3557 | **cleanup-completion（缺失）** | 正文确认：`Cleanup.Add` 与 `run` 的 TOCTOU 导致清理函数被静默丢弃。属“队列不丢任务/最终完成”，偏 liveness。 |
| E2B#1498 | **partial-failure-cleanup（缺失）** | 启动失败留下 running sandbox。模型没有失败补偿路径（首版删除了 `OperationStatus.failed`），属部分失败语义。 |

### 3.3 Liveness / 完成性（当前范围外）

orphan/leak/restart 类：runtime#3193、#2813、#2136、#1568、E2B#1005。它们要求“资源最终被回收”，是 liveness，当前只证 safety。这一类比模型缺口多，说明**相对完备性的缺口主要在 liveness，而不是 safety**。

### 3.4 Adapter（不建模）

快照内容/元数据忠实性：E2B#884、E2B#1104、runtime#3226、#3248；Firecracker 级：runtime#2057、#3359、#3131。归 adapter 合同与故障实验。

### 3.5 Contract / 出范围

- 崩溃后恢复语义（单节点范围外或合同差异）：runtime#2041、#1215。
- API 输入校验：runtime#3418、#3579。
- 模板产物 GC：runtime#3239。

## 4. 对三个问题的回答

1. **issue 里的安全规则整理出来了吗**：没有新增“性质清单”，但第一遍发现 **1 个 safety 形状缺口**（陈旧索引/引用，`#3566`）和 **2 个偏 liveness 的缺口**（cleanup 完成性、部分失败清理）。其余绝大多数要么被现有性质覆盖，要么是 adapter/liveness/出范围。
2. **现有规则需要补充吗**：分两类。**(a) checker 规则**：需要把已有 32 条性质接到轨迹（见 `docs/trace-coverage.md`）；**(b) 模型性质**：考虑新增“无陈旧索引/引用”不变量；若要把相对完备扩展到完成性，需要单独引入 liveness 规格（当前明确不做）。
3. **checker 补完了吗**：没有。当前 42 条性质中仅 5 条 `monitored`，32 条 `needs_instrumentation`，3 条 `static`，adapter/liveness 各 1。checker 完整 = 除 adapter/liveness 外全部有“规则 + 插桩”。

## 5. 局限

- 语料只含被报告故障；私有/云端问题不可见。
- 自动 triage 与关键词噪声大，`proposal.json` 也需你复核。
- 判定“被性质覆盖”需确认抽象忠实，否则应为 adapter。
- 版本漂移：应以修复 commit 作为历史证据（F-1 已如此）。
