# 用状态机审计 E2B：第一次结果

审计日期：2026-09-07。当前 E2B `main` 为 `1f34e5d6c822f50d8fb86f6b91b8613f25d2c667`；本项目原固定基线 `82f6529660ea98087d7a479247c789905b0a3867` 已包含本文讨论的历史修复。

## 1. 结论与证据等级

这次尝试证明状态机可以有效筛查 E2B 的生命周期代码。它重新定位了两类已经由 E2B 官方提交修复的竞态，并把它们分别对应到 generation-scoped cleanup 和 stale-effect admission。没有在当前 `main` 上确认新的未修复漏洞。

| 结果 | 证据等级 | 对应状态机性质 | 当前状态 |
|---|---|---|---|
| 路由目录的非原子检查后删除可删掉新 execution | 已确认的历史代码竞态 | `CleanupAdmissionSafe`、`ReclaimedResourcesNotCurrent` | `2a0c0af` 已修复 |
| 过期扫描得到的旧 pause/kill 决策可能作用于随后替换的 execution | 已确认的历史代码竞态 | generation/epoch 准入、stale effect 无害 | `eef64c7` 已修复 |
| Pause 失败或 client 断开后 snapshot build 没有进入终态 | 已确认的历史完成语义缺陷 | operation 账本；但“最终完成”超出当前 safety | `42f6997`、`8586a66` 已修复 |
| 多次 pause/resume 后文件内容丢失 | 官方 issue 中的用户报告 | manifest adapter 忠实性 | 当前 Lean 无法判定 |
| resume 后 stdout subscriber 卡住 | 官方 issue 中的用户报告 | guest 进程流和 TCP 状态 | 不在当前模型边界 |

“已确认的历史代码竞态”表示修复前代码、修复提交说明和回归测试形成一致证据。它不表示当前 E2B 仍受影响，也不表示已经证明其云端曾可被外部稳定触发。

## 2. 审计方法

不是从函数名搜索 `bug`，而是把每条 Lean 性质翻译成源码问题：

| Lean 审计尺子 | 源码中要找的危险形状 |
|---|---|
| stale generation 不能改变 current | 先读 ExecutionID，锁外做事，稍后只按 SandboxID 写或删 |
| cleanup 必须精确寻址 | `delete(sandboxID)`，或“先比较、后删除”不是同一原子动作 |
| terminal/fence 不倒退 | kill 后旧 pause/resume callback 仍能恢复 running 状态 |
| operation 结果稳定 | timeout、取消或重试覆盖已经保存的最终结果 |
| effect commit-before-dispatch | 外部 RPC 已发出，但持久状态里找不到对应操作/effect |
| manifest 绑定 | 恢复只按可变名字找 snapshot，没有核对来源 execution/workspace/manifest |

发现候选后，再构造最短事件历史。只有该历史同时符合真实 API/代码前提且违反一条性质，才升级为 bug 候选。

## 3. 历史竞态一：旧 teardown 删除新 execution 的路由

### 修复前代码形状

修复前 `DeleteSandbox(sandboxID, executionID)`：

1. 从 Redis `GET` 当前路由记录；
2. 在进程中比较 `ExecutionID`；
3. 如果相同，另发一次 `DEL`。

证据见 [E25](../references/code-evidence.md)。比较本身正确，但比较和删除之间仍有窗口。

### 最短违反历史

```text
catalog[S] = execution 1

旧 teardown: GET S，看到 execution 1，比较通过
resume/create: SET S = execution 2
旧 teardown: DEL S

结果：execution 2 仍可能运行，但它的当前路由被删除
```

这正是“旧 cleanup 影响当前 generation”。问题不在于缺少 ExecutionID，而在于身份检查没有与副作用原子绑定。

E2B 的 [`2a0c0af`](https://github.com/e2b-dev/infra/commit/2a0c0af69a0405500eb6e7eaded32349f7f4c7fa) 将它改成 Redis server-side compare-and-delete。提交注释直接说明目标是防止 stale teardown 删除 concurrent Store 写入的 newer execution，且新增 mismatch 回归测试。[E26](../references/code-evidence.md)

[`E2BAudit.lean`](../SandboxModel/E2BAudit.lean) 对这个交错给出三个机器证明结论：

- `split_check_delete_removes_new_execution`：分离检查与删除确实能删掉新 execution；
- `atomic_compare_delete_preserves_new_execution`：旧 teardown 在原子比较删除下变成 stutter；
- `atomic_compare_delete_removes_matching_execution`：修复仍能删除真正匹配的旧 execution。

这个案例说明状态机的价值不只是说“要检查 generation”，还会迫使我们问：**检查是不是发生在真正删除的同一个原子点？**

## 4. 历史竞态二：过期扫描后的旧决策

E2B evictor 先从过期索引读取 sandbox，再异步执行 pause 或 kill。修复前，扫描结果中的 `ExecutionID` 没有被带到最终 `StartRemoving` 条件中。读取后若发生 resume/replacement，旧决定就可能面对同一个 SandboxID 下的新 execution。

[`eef64c7`](https://github.com/e2b-dev/infra/commit/eef64c73e48c3134ef9d76317e37ceffddcdb3e4) 做了两件与我们的状态机完全一致的修改：

1. removal 携带 `ExpectExecutionID = scanned.ExecutionID`；
2. 如果扫描后 phase 已改变，旧 pause 决策被识别为 stale 并拒绝，而不是继续推进。

提交中的注释明确给出“resume landing in between”这一交错，测试也固定了 removal 必须携带扫描时 execution。[E27](../references/code-evidence.md)

对应到 Lean：扫描结果是旧 effect；`ExpectExecutionID` 是 generation guard；execution mismatch 是合法 Reject/stutter。只检查 SandboxID 无法满足 `StaleGenerationCannotAccessWorkspace` 和 cleanup/effect 准入合同。

## 5. Operation 账本发现的模型缺口

另外两项 E2B 修复与 operation ledger 有关：

- [`42f6997`](https://github.com/e2b-dev/infra/commit/42f699786534f1c0ad625ce9b4b7b922b3a16834)：snapshot/build 记录已经创建，但 Pause RPC 被拒绝时，旧代码没有把 build 写成失败终态。[E28](../references/code-evidence.md)
- [`8586a66`](https://github.com/e2b-dev/infra/commit/8586a66a5e84504536b2df9b7e28efe9069d8284)：client context 取消可能同时取消最终结果持久化；修复改用脱离请求取消、但有超时上界的 context。[E29](../references/code-evidence.md)

我们的 `OperationResultStable` 能检查“已经写下的最终结果不能被改写”，但它不保证一个 pending operation 最终一定得到 success/failure。这是 safety 与 liveness 的区别：

- safety：不能写错、不能覆盖；当前 Lean 已证明；
- completion：开始的操作在明确前提下最终进入终态；当前模型有意没有证明。

因此，这两项修复不是当前总定理的反例，却告诉我们：若要用模型自动筛查“永久 pending”问题，应增加一个很小的 **审计观察层**，记录 external operation 的 success/failure/unknown，而不需要开始实现 Rust controller，也不需要把 Linux 纳入 Lean。

## 6. 状态机不能发现的两类问题

[E2B issue #884](https://github.com/e2b-dev/E2B/issues/884) 报告多次 pause/resume 后文件修改丢失。当前模型只证明 resume 选择了同一个不透明 manifest；它假设 storage adapter 忠实保存内容和权限元数据。若 digest 身份正确但底层数据错误，Lean 看不见，需要 snapshot 内容校验与故障实验。

[infra issue #3292](https://github.com/e2b-dev/infra/issues/3292) 报告恢复后的旧 stdout subscriber 可能阻塞 fan-out。它涉及 guest 内被冻结的 goroutine、TCP 连接和 subscriber backpressure，不属于 sandbox authority 状态机。把这些状态加入核心模型会明显扩大课题。

另一个很好的边界对照是 [`7faa59e`](https://github.com/e2b-dev/infra/commit/7faa59e64472435c686456d82e54be1d98ad7e4a)：它修复动态 BYOP egress 更新时内存 policy 与内核 firewall 的发布顺序。我们的首版固定静态网络 policy，所以不能声称发现或覆盖该竞态。

## 7. 下一轮找“当前仍存在”的 bug

下一轮应保持三个窄切面：

1. **身份传播审计**：从 Redis/DB 扫描、reservation、callback 一直追到 Stop/Delete/route removal，检查每个副作用点是否仍携带同一个 ExecutionID/LifecycleID。
2. **故障点历史生成**：对 create/resume 注册、pause manifest commit、kill route removal 生成 `read old → replace → delayed effect` 的最小历史，并与 `Step` 做 conformance 判定。
3. **窄实验**：只对源码无法排除的历史做 failpoint；保存原始 HTTP/gRPC 状态、ExecutionID、LifecycleID、节点和时间线。单看迟到 stdout、404 或进程短暂共存都不能判定违反。

当前最值得继续验证的候选仍是：resume 已创建新 LifecycleID 但尚未完成 running/catalog 发布时，与旧 kill/pause callback 或 route cleanup 的交错。现有多层 guard 可能已经正确覆盖，所以在拿到失败历史前只标为候选，不称为漏洞。
