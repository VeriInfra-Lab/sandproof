# Issue 语料与相对完备性

本文定义如何用开源项目的 issue 语料，为“边界内的安全性质是否相对完备”提供经验证据。它对应讨论中的结论：**语料给经验证据，性质矩阵给 checker 定义“完整”的边界**。

仓库内保存的 issue 语料副本已遮盖第三方帖子中的凭据样式字符串；分类和复现结论不依赖这些字符串的原值。

## 1. 我们主张什么，不主张什么

形式化：设 `Corpus` 为边界内、可重建错误历史的 issue 集合，`P` 为已证明的安全性质集。若

> 对每个 `i ∈ Corpus`，存在 `p ∈ P` 使 i 的坏行为违反 p

则称 `P` **对已报告故障模式充分（relatively complete w.r.t. the reported corpus）**。

**这是经验充分性，不是数学完备性**：它只排除“已报告且落在边界内”的故障，不能覆盖未报告、私有、云上特有的故障。因此所有结论必须写明语料来源、截止时间和 commit。

## 2. 流程与工具

| 步骤 | 工具 | 输出 |
|---|---|---|
| 采集 | `tools/issues/study.py fetch` | `references/issues/<repo>.jsonl`（全部 open+closed） |
| 边界筛选 + 映射 | `tools/issues/study.py filter` | `references/issues/ledger.jsonl` |
| 指标 | `tools/issues/study.py report` | 分类计数 |
| 人工修正 | `references/issues/overrides.json` | 覆盖单条 issue 的分类 |

采集自 `e2b-dev/runtime`（原 `infra`，已更名）与 `e2b-dev/E2B`（SDK，可能描述后端生命周期问题）。

## 3. Ledger 与分类

每条 issue 一条记录：

```json
{"repo": "...", "number": 3636, "title": "...", "state": "OPEN",
 "url": "...", "in_scope": true, "score": 15,
 "matched": ["sandbox","pause","kill","snapshot","..."],
 "suggested_property": "TerminalConsistent", "verdict": "violates", "note": ""}
```

判定类别（与 `evaluation.md` 对齐）：

| 判定 | 含义 |
|---|---|
| `violates` | 边界内、且有性质排除其行为 |
| `model-gap` | 边界内、但没有性质排除 → 需要扩展模型 |
| `adapter` | 问题出在 Linux/FC/journal 等被抽象层 |
| `liveness` | 永久 pending 类，当前只证 safety |
| `out-of-scope` | guest 数据面、性能、feature 等 |
| `insufficient-evidence` | 无法从 issue 重建历史 |

**启发式只产候选**：`filter` 的关键词打分与映射会产生噪声（例如网络策略问题里出现 `kill`/`route` 也会被列入）。真实判定必须人工确认，并通过 `overrides.json` 落库。当前报告中的大量 `violates` 是**待复核候选**，不是结论。

## 4. 当前语料状态（首次运行）

- 采集：`runtime` 253 + `e2b-dev/E2B` 389 = **642** 条；边界筛选后 **317** 条候选。
- 已确认样本：`runtime#3636`（F-1）映射到 `TerminalConsistent`，与 checker 规则 `kill_irreversible` 一致。
- 待办：逐条人工复核 317 条候选（用 `overrides.json`），统计每个性质维度覆盖了多少根因类，找出 `model-gap`。

## 5. 与 checker 覆盖的关系

issue 研究回答“性质是否抓住了真实故障”；`docs/trace-coverage.md`（由 `tools/coverage.py` 生成）回答“checker 能否观测并判定每条性质”。两者都指向同一批性质 `P`：

- 若某条性质覆盖了某 issue，但 checker 没有对应规则/插桩 → checker 不完整；
- 若某条边界内 issue 没有任何性质覆盖 → 模型有缺口（`model-gap`），回到 Lean 扩展。

当前现状：性质注册表 42 条，其中 `monitored` 5、`needs_instrumentation` 32、`static` 3、`adapter` 1、`liveness` 1。即多数性质在模型内已证明，但 checker 尚未接线——这是实现工作，不是模型缺口。

## 6. 重要局限

- 语料只含**被报告**的故障；私有/云端问题不可见。
- 关键词映射有误报/漏报，必须人工复核。
- 版本漂移：老 issue 的代码形状可能已变；应以修复 commit 作为历史证据（如 F-1）。
- “被性质覆盖”可能是**空洞覆盖**：必须确认模型对该故障本质的抽象是忠实的，否则应归 `adapter`。
- 反向缺口：有些性质未被任何 issue 触发，可能是过度设计，也可能是未被测试，需另行标注。
