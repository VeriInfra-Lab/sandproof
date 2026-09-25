# 项目进展总览

> 当前入口（2026-09-25）：[项目蓝图](project-blueprint.md)与 [GitHub 总计划 #1](https://github.com/ningZhang-cs/sandproof/issues/1)。Sandproof 当前先做 E2B 核心兼容 V1 功能和测试；暂停恢复与集中证明在后续阶段。本文以下内容保留协议研究和实验进展，不能用旧任务排序替代新蓝图。

> 方向更正：当前主线已转为建设边界内 fully verified sandbox controller。本文中把 F-1/G-1 称为“已确认系统漏洞/一致性失败”的历史表述已撤回；它们只作为本地 harness 和建模过程的历史材料。最新路线见 [verified-controller-plan](verified-controller-plan.md)。

> 2026-09-23：Rust 纯控制核心已经实现，现有公开输入的 Rust—Lean 对应及有限执行
> 安全证明已在明确机器容量条件下完成；最终完成性与长期容量边界仍须补齐。
> 已编译的具体输入及剩余义务见 [证明覆盖记录](proof-coverage.md)。真实持久化、
> E2B 兼容服务与运行层按当前蓝图开发，不属于这些既有证明。下文保留的审计
> 与轨迹实验进度不等于当前控制核心的证明覆盖。

2026-09。本文汇总目标、已完成工作、边界、缺口与下一步。细节见各专题文档。

后续两批已证明创建、暂停、恢复的正常阶段，以及销毁和失败补偿阶段中，
当前尝试的有效回执会减少剩余工作量；两类回执步骤也已接到无限执行轨迹。
容量行为与结构问题分别见 [第一批复核](liveness-capacity-review.md)、
[架构复核](architecture-review-2026-09-23.md)。最终完成性和长期容量仍待完成。

## 1. 目标

为单节点 Firecracker 沙箱的 `create → pause → resume → kill` 生命周期，给出**可证明安全**的控制协议，并用它**系统性地审计/测试真实系统 E2B**。

中心命题：**snapshot 保存数据与状态，不保存执行权**。resume 的权限来自 controller 当前持久记录，绑定新 generation。

## 2. 已完成的工作

### 2.1 安全规范与证明（已完成）

- `SandboxModel/`：`World / Step / Reachable`，异步协议关系；双账本（operation + effect）；三层身份（generation/epoch/fence）。
- 顶层定理：`reachable_protocol_secure`、`step_protocol_secure`，任意有限历史、无界标识。
- 无 `sorry`/自定义 `axiom`，只依赖 `propext`、`Classical.choice`、`Quot.sound`。
- `Counterexamples.lean`：删除关键 guard 的机器检查反例（规范敏感性）。
- `E2BAudit.lean`：非原子 check-then-delete 历史的机器重放。

### 2.2 E2B 审计与历史实验（不作系统漏洞结论）

- 系统性 conformance 审计方法（guard 表 + evt/obs 映射 + 校准）。
- F-1、G-1、G-2 是本地 harness 中观察到的候选行为和建模材料；由于没有锁定部署配置、真实节点语义与完整观察映射，不再称为已经确认的 E2B 系统漏洞。
- 这些实验仍用于检验轨迹检查器是否能识别模型禁止的历史，但不能替代公开实现或生产部署的一致性证明。

### 2.3 轨迹校验（MBT）工具链（已完成首版）

- 轨迹格式与 evt/obs 映射（`docs/trace-format.md`）。
- 采集器 + Observer 插桩（真实存储 → 抽象观测）。
- checker：**25 条规则**，24 个自测；每条规则声明所需观测，缺数据自动跳过。
- Lean 侧 `Monitor.lean`：可执行 monitor + **`trace_monitor_sound`**（合法历史必被接受）。
- 覆盖度报告（`docs/trace-coverage.md`）。
- 综合测试：11 个场景，F-1 本地 harness 系统性复现，`stale_index` 正例对照成功。

### 2.4 Issue 语料与相对完备性（进行中）

- 采集 642 条 issue（runtime 253 + SDK 389），边界候选 317 条。
- 自动 triage + 人工第一遍（`references/issues/proposal.json`，30 条精确判定）。
- 结论：safety 性质的缺口主要在 **liveness/完成性**；发现 1 个 safety 形状缺口（陈旧索引/引用）。

## 3. 边界

### 3.1 范围

| 建模 | 不建模 |
|---|---|
| 单节点、一个 controller/worker | 多节点迁移、HA 共识 |
| create/pause/resume/kill、managed exec/file/network | guest 内进程/TCP/文件内容 |
| 多 tenant/sandbox、每代独占 VM/workspace/slot | 共享卷、snapshot fork/clone |
| 静态网络（denyAll / allowlist） | 动态 DNS/SNI、auto-resume |
| 不透明 `FilePolicyRef` / `SnapshotManifestRef` 身份 | Linux ACL/路径/UID/GID、ext4 |
| 复制/迟到/超时/crash/迟到清理 | 无适配器响应条件的完成性、恶意宿主 |

### 3.2 三层信任（“完全验证”的真实含义）

```
① 协议规范证明      安全性已证；工作流最终完成性仍有内部义务
② 实现 = 规范        Rust 纯核心全部现有公开输入已证明对应；有明确容量前提
③ 外部组件接入      后续阶段；关键自有连接代码需证明符合合同
```

“verified controller” 只指 ①+②；③ 是窄合同 + 故障实验。

## 4. 缺口清单

### 4.1 模型层缺口（safety 形状）

| 缺口 | 来源 | 状态 |
|---|---|---|
| 无陈旧索引/引用 | runtime#3566 | **已形式化并证明**（`Accounting.lean`：`sweepStoreOnly_is_stale`），检测规则 `stale_index` |
| 部分失败清理 | create/resume/pause failure | **已接入主 `Step/Reachable`**：pause 也采用 `beginCompensation → terminal fence → publishFailure` 的失败关闭策略，并纳入全局 safety/ledger 证明 |
| effect 结果不确定性 | timeout / 丢回执 / 迟到回执 | **已接入主模型和可执行处理函数**：结果未知、安全重发、迟到/重复/冲突回执及成功/失败证据检查 |
| 失败与补偿脱节 | 外部失败可被任意宣称 | **已解决**：失败 effect 必须与当前 `sid + epoch` 绑定；取消/截止时间使用本地原因 |
| kill 遗留旧 operation | create/pause/resume 被打断 | **已解决**：旧 operation 原子写入 `failed(cancelled)`；terminal 状态拒绝第二个新 kill |
| 缓存读一致性 | runtime#3595 | 归为 accounting 一致性（索引与 store 一致已证）；通用缓存一致性仍待定义 |

详见 [模型扩展](model-extensions.md)。

### 4.2 liveness / 完成性

- 已定义允许空转的无限执行、时间性质和弱公平，并将公平性连接到请求、具体动作、调度、执行端动作和稳定发布处理函数。
- 已证明操作记录不会消失，最终结果一旦写入便永久不变；在“未完成时总有持续可执行工作、该工作得到公平调度、有限进度量严格下降”的合同下，操作最终并永久得到成功或明确失败结果。
- 已证明正常阶段与销毁／补偿阶段的当前尝试有效回执会减少具体进度量，
  并把销毁／补偿的回执步骤接到无限执行轨迹。
- 资源预留条件持续有效、工作项选择、等待外部回执及执行节点动作与进度量的
  组合仍未闭合。因此还不能推出四类操作在公平调度下最终完成。

### 4.3 adapter 缺口（不建模，靠合同/实验）

快照内容与元数据忠实性、Firecracker/UFFD 生命周期、存储 GC：#884、#1104、#3226、#3248、#2057、#3359、#3131、#3239。

### 4.4 checker 插桩缺口

- 已 monitored：25 条。
- 仍 `needs_instrumentation`：12 条（资源归属、authority/fence、effect/operation 账本、cleanup 精确性等）——需要节点/存储层结构化事件。
- adapter/liveness/model_gap 各 1 条为边界。

### 4.5 可执行 controller 与 refinement 缺口

- Lean 内已实现请求、具体动作派发/超时/回执/重试、后续命令调度、全部执行端动作与稳定状态发布的确定性处理函数；所有接受结果都携带合法 `Step` 证明。
- 对请求、具体动作、调度、执行端动作和稳定发布，均有从规范允许分支回到处理函数成功分支的完备性证明。暂停停机已与本次操作计划选定的目标快照绑定；操作计划、有限资源清单、效果编号清单、计划封存以及计划与操作账本的一致性都有初始与逐步保持证明。
- Rust 纯控制核心的全部现有公开输入已完成实际抽取语义与 Lean 规范的对应
  定理，并证明明确机器容量前提下任意有限次混合执行保持安全；该容量前提
  尚未证明可以在长期执行中持续满足。
- adapter 合同（journal/worker/firecracker/storage/policy/network）尚未实现或审计。

### 4.6 校准缺口

- 已做：F-1 轨迹判定 Violates、`stale_index` 正例对照、正常场景 Conforms。
- 未做：对**修复前 commit**（`84362fc6` catalog 竞态、`eef64c7` stale 决策）跑 checker 命中——需要先接路由/决策规则与插桩。

### 4.7 复现缺口

- 本地 harness 依赖 patched dummy + seeded DB，不是云端复现。
- 未做多节点、未在 E2B 云生产验证。

## 5. 进展度量

| 里程碑（研究计划） | 状态 |
|---|---|
| G0 固定范围、源码基线、Lean World/Step | 完成 |
| G1 状态/准入/历史 safety + 场景 + mutant | 完成 |
| G2 Rust reducer + Aeneas simulation | 纯核心及现有输入已完成；长期容量与外部服务接入未完成 |
| G3 journal/worker/storage/FC adapter 实验 | 部分（F-1 本地 harness） |
| G4 E2B conformance histories | 部分（轨迹校验 + 语料） |
| G5 可复现 artifact 与论文结果 | 未开始 |

| 指标 | 数值 |
|---|---|
| Lean 规范+证明 | 协议、可执行处理核心、双向分支覆盖、计划/账本保持性和条件化完成性；无 sorry |
| 顶层定理 | 全局安全 + 处理器正确性/完备性 + 条件化最终完成 |
| checker 规则 / 自测 | 27 / 29 |
| 性质覆盖 | monitored 28、needs_instrumentation 12、static 3、liveness 2、adapter 1 |
| issue 语料 | 642 采集 / 317 候选 / 30 精确判定 |
| 综合场景 | 17（F-1 本地 harness 系统性复现 + G-1 孤儿 + G-2 + 正例对照） |
| 历史上报 | 2（F-1 [runtime#3636](https://github.com/e2b-dev/runtime/issues/3636)、G-1 [runtime#3637](https://github.com/e2b-dev/runtime/issues/3637)；现仅作为 harness 候选材料记账） |

## 6. 下一步优先级

1. 闭合资源预留、等待回执、执行节点与稳定发布的最终完成性组合。
2. 建立有限机器的准入预算，保证已接收操作的后续步骤有空间，并保持现有 Rust 对应证明。
3. 把 journal、worker、Firecracker、storage、policy、network 的假设收窄为可测试合同。
4. 轨迹校验研究另行完成插桩、映射校准与 issue 语料复核，不与 verified controller 结论混用。

## 7. 一句话总结

模型层面的协议安全、精确重试、有限计划及处理函数分支覆盖已有机器证明。
最终完成性定理仍带有尚未闭合的内部进展前提，不能把这些前提全部转交给
外部系统，也不能据此宣布最终完成性已经全部证明。实际 Rust 的一致性
证明已覆盖纯控制核心的现有全部公开输入，具体成果及容量前提以 [证明覆盖记录](proof-coverage.md)
为准。数据库、外部执行器接入与部署属于后续阶段；当前既不是已部署的
完整验证控制器，也不构成对 E2B 的系统漏洞证明。
