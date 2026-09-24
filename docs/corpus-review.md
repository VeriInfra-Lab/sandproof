# 语料复核：issue 的翻译、安全维度与性质对应

本文是 [缺口登记表](gap-register.md) 中 G-7（语料复核）的第一遍完整总结。数据：`references/issues/corpus-review.json`（317 条边界内候选，含判定/性质/维度）；原始语料 `references/issues/*.jsonl`。

## 1. 总览

- 采集 **642** 条（`e2b-dev/runtime` 253 + `e2b-dev/E2B` 389，open+closed）。
- 边界筛选后 **317** 条候选。
- 判定分布：out-of-scope 153、needs-review 109、liveness 26、adapter 16、violates 8、model-gap 3、contract 2。
- 其中 **30 条**做了逐条人工翻译与性质对应（下表）。

**安全维度**用我们的性质族表示：生命周期终态（TerminalConsistent 等）、资源归属（ResourceOwnership/SnapshotOwnership）、路由（ActiveSlotValid）、补偿（CreateCompensation）、索引一致性（NoStaleIndexReferences）、活性（liveness）、适配（adapter）、合同（contract）。

## 2. 边界内后端生命周期问题（逐条）

| issue | 中文简述 | 安全维度 | 对应性质 / 判定 |
|---|---|---|---|
| runtime#3636 | 对 paused 沙箱的 kill 返回 204 且删快照，但在途 resume 仍把它发布为 running（F-1） | 终态不可逆 | `TerminalConsistent` / violates（已复现上报） |
| runtime#2637 | 快照查询未按 team 过滤，存在跨团队访问风险 | 资源归属 | `SnapshotOwnership` / violates |
| runtime#3178 | `GetLastSnapshot` 未按 team 限定，可跨团队读取 | 资源归属 | `SnapshotOwnership` / violates |
| runtime#3354 | 上述问题的修复（SQL 层按 team 限定） | 资源归属 | `SnapshotOwnership` / violates |
| runtime#3573 | 模板标签接口先查库/开事务，之后才做团队归属校验 | 资源归属 | `ResourceOwnership` / violates |
| runtime#2836 | 从 Team A 恢复的 paused 沙箱会从 Team B 消失（共享 base template） | 资源归属 | `SnapshotOwnership` / violates |
| runtime#1763 | OrchestratorIP 被空串覆盖，集群节点沙箱路由失败 | 路由身份 | `ActiveSlotValid` / violates |
| runtime#3595 | `StartRemoving` 状态转换未广播，分配方从缓存读到过时的 Running | 终态/静默 | `TerminalConsistent`/`StableStatesQuiescent` / needs-review（缓存一致性） |
| runtime#3566 | 沙箱键非 `Remove()` 路径丢失时，team-index SET 残留陈旧成员 | 索引一致性 | `NoStaleIndexReferences` / **model-gap→已形式化** |
| runtime#3557 | `Cleanup.Add` 与 `run` 的 TOCTOU 导致清理函数被静默丢弃 | 活性/完成 | liveness（cleanup-completion）/ model-gap |
| runtime#3193 | 节点无独立超时执行，API evictor 停止后 VM 无限泄漏 | 活性/完成 | liveness |
| runtime#2813 | 主机压力下大量孤儿 Firecracker 进程与残留 iptables | 活性/完成 | liveness |
| runtime#2136 | orchestrator 重启后残留孤儿 Firecracker | 活性/崩溃恢复 | liveness |
| runtime#1568 | kill 后 Firecracker 进程变孤儿，手动清理阻塞新沙箱 | 活性/完成 | liveness |
| E2B#1498 | MCP 启动失败留下 running 沙箱 | 部分失败清理 | `CreateCompensation` / **model-gap→已形式化**（G-1 同族） |
| E2B#1005 | kill 请求未完成 | 活性/完成 | liveness |
| E2B#424 | kill 之后请求未被取消 | 静默 | `StableStatesQuiescent` / needs-review（数据面 vs adapter） |
| E2B#884 | 多次 pause/resume 后文件修改丢失 | 内容忠实性 | adapter |
| E2B#1104 | 超时 auto-pause 产生不完整文件系统快照 | 快照忠实性 | adapter |
| runtime#3226 | 模板 memfile.header 磁盘缺失，几分钟后不可用 | 快照忠实性 | adapter |
| runtime#3248 | Local 模式未写 memfile.header | 快照忠实性 | adapter |
| runtime#2057 | 恢复 VM 的 GIC 状态失败 | FC 恢复 | adapter |
| runtime#3359 | UFFD handler 运行时失败未杀沙箱，留僵尸 VM | FC 生命周期 | adapter |
| runtime#3131 | UFFD handler 失败时应终止沙箱（修复） | FC 生命周期 | adapter |
| runtime#2041 | orchestrator 重启后所有沙箱消失 | 崩溃恢复 | contract（单节点范围） |
| runtime#1215 | 重启后沙箱无法连接 | 崩溃恢复 | contract（单节点范围） |
| runtime#3418 | create/resume/fork 的非正超时绕过校验导致立即终止 | 输入校验 | out-of-scope |
| runtime#3579 | connect 接受非正超时导致立即终止 | 输入校验 | out-of-scope |
| runtime#3154 | 内部超时被误判为用户取消 | 超时语义 | `operationResultStable` / needs-review |
| runtime#3239 | 模板删除不清理底层构建产物 | 存储 GC | out-of-scope |

## 3. 其余候选（109 条 needs-review）的分桶

| 桶 | 数量 | 代表 | 处置 |
|---|---|---|---|
| SDK/集成/bounty | 21 | E2B#241/243/245/246（LangChain/LlamaIndex/AutoGen 集成） | out-of-scope（SDK/生态） |
| 问答/自托管/部署 | 16 | E2B#381、runtime#754/#850/#857/#1073/#1112 | out-of-scope（部署运维） |
| 功能/路线图 | 10 | runtime#55/#79/#121/#2305/#2658、E2B#730 | out-of-scope（feature） |
| 可观测/日志/诊断 | 9 | runtime#71/#78/#83/#3274/#3278/#3553 | out-of-scope（observability，非安全） |
| 构建/模板/存储/NBD | 14 | E2B#310/#684/#1113/#1161、runtime#1516/#1079 | out-of-scope（构建/存储，少数 adapter） |
| 其他（崩溃/错误报告/杂项） | 36 | E2B#16/#506/#592/#736/#899、runtime#54/#69/#89/#170 | 多为 out-of-scope；#736/#899（resume/connect paused 404）属 contract difference |

## 4. 性质覆盖矩阵（对已复核语料）

| 性质 / 类别 | 覆盖的 issue |
|---|---|
| `TerminalConsistent`（终态不可逆） | #3636、#3595（缓存） |
| `SnapshotOwnership` / `ResourceOwnership` | #2637、#3178、#3354、#3573、#2836 |
| `ActiveSlotValid`（路由身份） | #1763 |
| `CreateCompensation` / `NoOrphanOnNode` | E2B#1498、G-1（我们新报 #3637） |
| `NoStaleIndexReferences` | #3566 |
| liveness / 完成性（当前出范围） | #3557、#3193、#2813、#2136、#1568、E2B#1005 |
| adapter（内容/快照/FC 忠实性） | E2B#884、E2B#1104、#3226、#3248、#2057、#3359、#3131 |
| contract（崩溃恢复/租约语义） | #2041、#1215、E2B#736、E2B#899 |
| out-of-scope（输入校验/存储 GC/观测/部署/feature/SDK） | 其余 |

## 5. 结论（相对完备性）

对**已复核的边界内正确性问题**：

1. **safety 形状的问题，全部被某条已证明性质覆盖**（上表前五行），没有发现“有坏行为但没有任何性质禁止”的新 safety 缺口。
2. 曾发现的 safety 形状缺口 `#3566`（陈旧引用）**已形式化并证明**（`Accounting.lean`），并有检测规则 `stale_index` + 正例对照。
3. 部分失败类 `E2B#1498` **已有补偿模型**（`Compensation.lean`）；G-1 仅是在本地 harness 中观察到的同形候选行为，不能作为 E2B 系统复现结论。
4. 完成性已从“完全未覆盖”推进为带明确公平与有限进度条件的定理；逐工作流进度量、适配器最终响应和安全清理成功仍需闭合。
5. adapter 与 contract 类按边界声明处理。

因此可以定稿的表述是：

> **在已复核且映射成立的边界内问题上，安全性质具有经验上的覆盖证据；这不是数学完备性，也不把本地 harness 行为升级为 E2B 系统漏洞。剩余模型工作集中在逐工作流完成条件。**

## 6. 局限

- 语料只含被报告故障，私有/云端问题不可见。
- 106 条 needs-review 仍为启发式分桶，个别可能误分（如 #3595 缓存归属、#3154 超时语义），需后续逐条确认。
- 判定“被性质覆盖”仍需确认抽象忠实（见 [映射理论](mapping-theory.md) 的 α 契约）。
