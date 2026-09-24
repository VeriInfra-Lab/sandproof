# 模型与 E2B 的系统关联：相关工作与方法

2026-09。回答一个问题：我们有一个已证明安全的 Lean 状态机，E2B 是现实实现，如何**系统地**把两者关联起来做测试，而不是逐条人工对照。

结论先行：最合适的主线是 **trace validation（轨迹校验）**——把 Lean 模型当 oracle，让 E2B 跑出带故障注入的轨迹，用模型判定每条轨迹是否合法。配套用 **model-guided test generation** 生成有目标的故障轨迹，用 **runtime monitor** 做持续检查。下面先分类介绍相关工作，再给出我们的具体桥接架构。

## 1. 相关方法分类

### 1.1 Trace validation / conformance（轨迹校验）

**机制**：让真实程序运行并记录有限轨迹（事件序列 + 部分状态更新），然后用形式规范判定"这条轨迹是否对应规范允许的某个执行"。它不是穷举目标系统的状态空间，而是把轨迹当作**约束**去问规范："能否有一条行为与观测一致？"

**代表工作**：
- Cirstea, Kuppe, Loillier, Merz, *Validating Traces of Distributed Programs Against TLA+ Specifications*, arXiv:2404.16075 / ABZ 2024。把轨迹校验归结为**受约束的模型检测**（用 TLC）；提供 Java 插桩 API、一组 TLA+ 关联算子、以及运行脚本。关键设计：轨迹只记录**被追踪变量的更新**而非全量值，且**允许只追踪部分变量**；不在规范里复刻实现细节，而是让规范"吸收"观测。作者在多个分布式程序上检出规范与实现的偏差。
- 工业界用例：MongoDB 等用 TLA+ trace validation 做上线前后的一致性检查（TLA+ 社区工作流）。Ron Pressler 把 trace validation 形式化为一种 refinement check，是这一路的理论出发点。

**对我们的适配**：⭐⭐⭐⭐⭐。我们已有 `Step`/`Reachable` 规范和 obs/evt 映射草案，天然适合。轨迹约束也正好绕开"无界状态无法穷举"的问题——轨迹本身限制了搜索。

### 1.2 Model-checking guided testing / MBT 生成

**机制**：用模型（或其状态空间）**生成测试用例**，驱动真实实现，再比对两者状态。

**代表工作**：
- Wang, Dou, Gao, Wu, Wei, Huang, *Model Checking Guided Testing for Distributed Systems* (Mocket), EuroSys 2023。用 TLC 导出状态空间图（节点=状态，边=动作），据此生成测试序列；用代码注解把实现变量/函数映射到 TLA 变量/动作，注入事件并在运行时比对模型态与实现态，报告两类不一致：**inconsistent state**（状态不同）和 **missing action**（模型里有、实现没发生）。
- AWS 的 P / P# 工作流：模型既用于验证，也用于生成测试和运行时检查（Newcombe et al., *How AWS Uses Formal Methods*, CACM 2015）。
- 经典 MBT：Utting, Pretschner, Legeard, *A taxonomy of model-based testing approaches*, STVR 2012。
- 状态化属性测试（stateful PBT，QuickCheck/Proper/quickcheck-state-machine）：用状态机模型定义 pre/post 条件，生成命令序列，对真实系统回放并比对模型态。可加入故障注入（"simulation testing"）。

**对我们的适配**：⭐⭐⭐⭐。我们有官方 dummy harness（可延迟、可注入故障），非常适合生成并驱动。但我们的模型状态**无界**，不能像 TLC 那样穷举状态空间生成全覆盖用例，只能做**定向生成**（按故障维度 + mutant + 场景）。Mocket 的"注解映射"与我们的 evt/obs 映射是同一件事。

### 1.3 Runtime verification（运行时验证 / 监视器合成）

**机制**：从安全性质合成一个监视器，在系统运行时消费事件流，实时检测违反。Leucker & Schallhart, *A brief account of runtime verification*, JLAP 2009。

**对我们的适配**：⭐⭐⭐。`Safe`/`ProtocolSecurity` 是单状态不变式，容易合成监视器；但它只检查"状态不变量"，不检查"这一步是否合法转移"，比 trace validation 弱。适合做 CI 常驻检查，作为 trace validation 的轻量补充。

### 1.4 Refinement proof（精化证明）

**机制**：证明实现是规范的 forward simulation（IronFleet, SOSP 2015; Anvil, OSDI 2024）。这是唯一能给出"实现正确"普遍结论的方法。

**对我们的适配**：⭐⭐（仅对自有实现）。它要求实现由我们掌控（我们的 Rust reducer，轨道一），无法用于 E2B 的 Go 代码。它能证明我们的核心，但**不能连接 E2B**——E2B 只能被测试/审计。

### 1.5 与我们的定位

| 方法 | 检查什么 | 需要实现可改 | 覆盖 E2B？ | 强度 |
|---|---|---|---|---|
| Trace validation | 观测轨迹是否是合法执行 | 需插桩（可不改逻辑） | ✅ | 强（逐步 conformance） |
| Model-guided testing | 模型生成的目标轨迹 | 需插桩 + 驱动 | ✅ | 中强（覆盖率靠生成策略） |
| Runtime monitor | 状态不变量 | 可低侵入 | ✅ | 中（不含转移合法性） |
| Refinement proof | 实现=规范 | 需自研实现 | ❌ | 最强（但只对核心） |

## 2. 把 Lean 模型和 E2B 关联：桥接架构

核心思想：**Lean 规范是唯一 oracle；E2B 通过插桩产生轨迹；一个从规范导出的可执行 checker 判定轨迹**。四层组件：

```text
E2B (api + store + dummy node)
   │ ① instrumentation：在副作用边界发结构化事件 + 状态快照
   ▼
Trace (事件 + 可观察状态，带 happens-before 时序)
   │ ② obs/evt maps：具体 → 抽象（conformance-method 已定义）
   ▼
Abstract trace
   │ ③ checker：从 Lean 规范导出的可执行判定
   ▼
Verdict: Conforms / Violates / Inconclusive / Contract difference / Unsupported
```

### 2.1 ① 插桩（instrumentation）

参考 TLA+ 轨迹校验的 Java API：只记录**被追踪变量的更新**，且允许只追踪子集。E2B 侧的插桩点与我们映射表一致：

- controller 事件：acceptCreate / acceptPause / acceptResume / acceptKill / rpcTimeout / controllerCrash
- store 写点：`StartRemoving`、`Add`、transition 脚本、快照软删除
- effect 生命周期：commit / dispatch / ack（Redis effects/outbox）
- worker 边界：dummy/FC 的 prepare / activate / freeze / stop / cleanup / fence
- 可观察状态快照：running store、snapshots 表、envs.deleted_at、workerFences、appliedFilePolicies

**不变式**：轨迹必须记录**原始事件与结果**，不能记录 SDK 语义压缩后的布尔值（如 409→false），否则无法回放判定——这正是 evaluation.md 的纪律。

### 2.2 ② 抽象映射（obs / evt）

即 `conformance-method.md` 已有的两份映射：`obs`（World 字段 ↔ 真实存储）与 `evt`（Event ↔ 真实动作/角色）。插桩产生具体事件，映射函数把它们投影为抽象 `Event` 与部分 `World`。**不可观察字段显式声明**，checker 不对其施加约束（对应 TLA+ 的"只追踪部分变量"）。

### 2.3 ③ checker：从 Lean 规范导出

这是最关键、也是研究的核心贡献点。`Step : World → Event → World → Prop` 是关系式，不能直接执行。两种可行设计：

**设计 A（推荐）：可执行的 Lean 轨迹校验器 + 可靠性定理。**
- 在 Lean 中定义轨迹同步关系 `TraceAccepts : List Observed → Prop`，含义为"存在一条 `Reachable` 历史，其 `obs`/`evt` 投影等于该轨迹"。
- 提供一个可执行判定 `check : List Observed → Bool`，并证明 **soundness**：
  ```lean
  theorem check_sound : check t = true → ∃ w, Reachable w ∧ projects w t
  ```
  即"checker 接受 ⇒ 模型确实允许这条历史"。可选：对可观察片段证明完备性。
- 好处：**规范仍是唯一来源**；checker 的正确性本身被证明；这是把 Lean 规范变成测试 oracle 的正式桥。
- 挑战：模型含无界性与不可观察分量，checker 需要在"候选世界集合"上做存在性选择。实用做法不是全 `Step` 存在性，而是**分层判定**：先用 `Safe`/`ProtocolStepSecurity` 直接查观测到的状态矛盾（便宜、强），再用 guard 表做每步准入 conformance（我们审计已在用的尺子）。

**设计 B：用可执行归约器当 oracle。**
- 等待轨道一的 Rust reducer（Aeneas），用 `reduce` 的确定语义作为预测器，与 E2B 对同序列的状态比对（差分测试）。
- 缺点：依赖轨道一，且 reducer 与 E2B 的抽象层次要对齐；但它能做**预测**而非仅**接受/拒绝**，更容易定位分歧。

**建议**：设计 A 为主（现在可做、且把规范变成可执行 oracle 是论文亮点）；设计 B 作为轨道一完成后的增强。

### 2.4 处理三个难点

1. **非确定性**：同一个抽象状态可能有多个合法后继（不同 effect/operation ID）。checker 维护**候选世界集合**或做规范化抽象；轨迹把选择约束住。
2. **部分可观察 / 未知结果**：timeout 时，事件可能已生效也可能没生效。按 evaluation.md，保留两种可能的世界（"已生效"与"未生效"），任一分支被后续观测排除则收敛；都合法则判 Inconclusive 而非违例。
3. **无界状态**：不做穷举；轨迹校验是**受约束**问题，轨迹长度与观测内容界定了搜索，因此无界不构成障碍。

### 2.5 生成侧：model-guided fault injection

用 Lean 模型做**定向**生成，而不是全覆盖：

- 从 `Counterexamples.lean` 的 mutant 直接导出负例（删掉某个 guard 的模型会产生哪些事件序列）。
- 从 `Scenarios.lean` 导出正例。
- 按 evaluation.md 的故障维度组合：重复/迟到、timeout、controller crash/recover、旧资源延迟清理、pause/kill/resume 交错。
- 在 dummy harness 上用 failpoint（如我们已有的 `DUMMY_CREATE_DELAY_MS`）把这些序列逼出来。

## 3. 落地步骤（按优先级）

1. **定义轨迹格式**（事件、可观察状态、happens-before；含未知事件）。先覆盖 controller 侧生命周期。 — **完成**（`docs/trace-format.md`）
2. **在 dummy harness 上加插桩**，产出可回放的轨迹；用 F-1 复现轨迹作为第一个正样本。 — **完成**（`tools/trace/collect.py`，正/反样本见 `references/trace/`）
3. **实现 checker v0**：直接检查 `Safe`/`ProtocolStepSecurity` 观测矛盾 + guard 表逐步 conformance。 — **完成首版**（`tools/trace/check.py`，规则 `kill_irreversible`）
4. **校准**：对修复前 commit（`84362fc6`）跑 checker，必须命中两个历史竞态；对 F-1 轨迹判定 Violates。 — **部分**（F-1 已判 Violates；历史竞态待扩展路由/决策规则）
5. **checker v1（Lean）**：把校验器写成 Lean 可执行函数并证 soundness。 — **完成**（`SandboxModel/Monitor.lean`，`trace_monitor_sound`）
6. **model-guided 生成**：输入故障维度与 mutant，自动产生目标轨迹并驱动 harness。
7. （轨道一就绪后）**差分测试**：E2B vs 已验证 reducer。

## 4. 与既有文档的关系

- `conformance-method.md` 的 obs/evt 映射与 guard 表是桥接的 ② 与 checker 的输入；静态审计（读代码）和动态轨迹校验（跑代码）是同一套映射的两个用法。
- `strategy.md` 的轨道二（边界验证）在本文件中具体化为 trace validation + 生成。
- `evaluation.md` 的判定五分类与时序纪律直接作为 checker 的判定规则。

## 5. 参考文献

- Cirstea, Kuppe, Loillier, Merz. *Validating Traces of Distributed Programs Against TLA+ Specifications.* arXiv:2404.16075 (ABZ 2024).
- Wang, Dou, Gao, Wu, Wei, Huang. *Model Checking Guided Testing for Distributed Systems (Mocket).* EuroSys 2023.
- Newcombe et al. *How Amazon Web Services Uses Formal Methods.* CACM 2015.
- Utting, Pretschner, Legeard. *A Taxonomy of Model-Based Testing Approaches.* STVR 2012.
- Claessen, Hughes. *QuickCheck: A Lightweight Tool for Random Testing of Haskell Programs.* ICFP 2000.
- Leucker, Schallhart. *A Brief Account of Runtime Verification.* JLAP 2009.
- Hawblitzel et al. *IronFleet: Proving Practical Distributed Systems Correct.* SOSP 2015.
- Sun et al. *Anvil: Verifying Liveness of Cluster Management Controllers.* OSDI 2024.
