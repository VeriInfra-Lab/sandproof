# 模型—实现连接的理论：可验证的映射契约

本文回答一个系统性问题：**把 Lean 模型和真实实现 E2B 连接起来的那层"映射"本身没有被验证，若映射错了，测试结论就是空的。** 我们把它形式化为带证据的映射契约，并给出可机械验证的部分。

## 1. 问题：oracle / 抽象函数问题

checker 的可靠性定理 `trace_monitor_sound` 是**相对于抽象函数 α** 的：

> 若 α 是忠实的，则 checker 拒绝 ⇒ 实现不存在合法历史。

α 本身是一个**假设**。今天 α 是手写的（`tools/trace/observe.py` 曾经），因此整条链的可信性压在它上面。这在测试理论里叫 **oracle problem**（Barr et al., TSE 2015）的形式化版本。

## 2. 定义

- **具体轨迹** `ct = c₀, a₁, c₁, …`：插桩得到的具体状态与动作。
- **抽象函数 α**：一组子句，每条把具体来源投影为抽象 `World` 字段。α 是**部分且非确定**的：不可观察字段返回 `⊥`；部分可观察字段对应抽象状态的**集合**。
- **事件映射 β**：具体动作 → 抽象 `Event`（`tools/trace/mapper.py`）。
- **conformance**：`ct` 符合模型，当且仅当 `∃` 模型历史 `w₀ →…→ wₙ`，使 `wᵢ ∈ α(cᵢ)` 且事件经 β 对齐。

由此得到两个**证明义务**：

1. **sound rejection（已证）**：拒绝 ⇒ 不存在这样的历史（由 `trace_monitor_sound`，条件在 α）。
2. **tight acceptance（未证）**：接受 ⇒ 存在这样的历史。这要求 α **不过宽**——否则会产生"空洞接受"。

## 3. 信任边界：每条子句带三样东西

α 不再是一段手写代码，而是一份**机器可读的子句账本**（`tools/mapping/clauses.json`）。每条子句：

| 字段 | 含义 |
|---|---|
| `concrete_source` | 具体来源（Redis 键 / Postgres 行 / 节点集合） |
| `relation` | 具体 → 抽象的关系 |
| `fidelity` | **信任假设**：为什么这个来源忠实于该抽象字段 |
| `citation` | 固定 commit 的源码位置 |
| `observability` | `observable` / `partial` / `unobservable` |
| `probe` | 实现该关系的探针函数名 |

**不可观察/部分可观察字段必须显式声明**，并列出它们"阻塞"了哪些性质。这样"未施加约束"这一事实本身进入账本，而不是被悄悄忽略。

## 4. 机械验证：三类证据

`tools/mapping/validate.py` 对每条子句施加三类**可复现**证据：

1. **citation 校验**：引用的源码文件在固定 commit 下存在（`observable` 子句）。
2. **witness（存在性证据）**：对真实 harness 跑一次 create→pause，断言各阶段的探针输出等于子句声明的期望（`expected_after_create` / `expected_after_pause`），并检查索引一致性。
3. **mutation（灵敏度证据）**：把某条子句的探针替换成错误结果，断言 witness 会失败——证明这条子句不是装饰。当前 5 项检查、0 逃逸。

这三者把"α 可信"从一个不可检查的大假设，拆成**逐条、可复核、可回归**的断言集。

## 5. 大模型放哪儿（可验证地依赖）

- LLM/agent **只做候选生成**：为子句提出 `concrete_source`、`relation`、`citation`。
- **接受必须靠机械证据**：witness 通过 + citation 存在 + mutation 生效。LLM 的产出永远不直接进可信集。
- 人类只复核"语义是否忠实"（citation 指向的字段是否真是那个意思）——唯一无法机械化的部分，被压到逐条且有引用。

## 6. 当前 α 账本状态

`tools/mapping/validate.py`（首次运行）：

- 子句 17：**observable 10**，non-observable/partial 7。
- 10 条 observable 的 citation 全部存在；witness PASS；mutation 0 逃逸。
- 显式声明的信任边界（阻塞的性质）：

| 字段 | 类别 | 阻塞性质 |
|---|---|---|
| `epoch` | unobservable | `epochMonotone`, `fenceMonotone` |
| `generation` | partial | `generationMonotone` |
| `effects` | unobservable | `effectAdmission`, `effectResultStable` |
| `operations` | unobservable | `operationResultStable`, `operationIdentityStable` |
| `applied_policy` | unobservable | `RunningRequiresAppliedFilePolicy` |
| `fence_epoch` | unobservable | `FenceWellFormed`, `FenceClosesAuthority` |
| `executing_vms` | partial | `UniqueExecution`, `ActiveSlotValid`, `ReclaimedResourcesNotCurrent`, `StableStatesQuiescent` |

## 7. 与既有理论的关系

| 理论 | 关系 |
|---|---|
| Data refinement / gluing invariant（Abrial；Back & von Wright） | α 就是胶合不变量；对**自研代码**可证明，对 Go 只能声明+验证 |
| Forward/backward simulation（Lynch & Vaandrager） | 给出 conformance 的模拟定义；同属自研代码 |
| Trace validation（Cirstea/Kuppe/Merz） | 同样依赖一个被信任的插桩/抽象层；我们把该层显式化为带证据的子句 |
| ioco（Tretmans） | 提供 conformance relation 与测试充分性的理论骨架 |
| Translation validation（Pnueli） | 核心精神：不做一次性大证明，做可复现的小验证——本文的 witness/mutation |
| Oracle problem（Barr et al.） | 本问题的一般形式 |

## 8. 局限

- witness 目前只覆盖 create→pause 一条生命周期；更多字段需更多 witness。
- 这是**经验验证**，不是 α 正确的证明；"tight acceptance"仍未证。
- citation 校验只证明文件存在，不证明字段语义忠实——后者由人复核。

## 9. 复现

```sh
python3 tools/mapping/validate.py                 # 覆盖率 + citation
python3 tools/mapping/validate.py --witness       # 现场 witness
python3 tools/mapping/validate.py --mutation      # 映射变异灵敏度
```

`observe.py` 已改为由 `clauses.json` 驱动，消除手写漂移。