# Conformance 审计第一轮：kill × resume 竞态候选

审计日期：2026-09-08。固定版本：`e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`（本地 `research-sources/e2b-infra`，已用 git rev-parse 核对）。方法按 [conformance 审计方法](conformance-method.md) 执行：guard 表从 `Step.lean` 导出，Controller 侧关键文件由主审计逐行验证，worker 侧（节点/FC/proxy）与 create/fcgate 路径由两个子审计覆盖后主审计复核关键结论。

**结论先行**：在固定映射下发现一处与协议性质矛盾的源码形状——**对 paused 沙箱的 kill 会删除快照并返回 204，但不阻止一个在途 resume 通过无锁注册把同一沙箱重新发布为 Running**。已在本地控制面 harness 上稳定复现（无 Firecracker，见 [F-1 复现](e2b-f1-repro.md)）：三轮 race 均为 kill 204 / resume 201 / 最终 running，对照（无在途 resume）保持删除。这对应 [源码分析](e2b-code-audit.md) 中 H1 类候选的具体化。

## 1. 校准结果

| 校准 | 结果 |
|---|---|
| 正例（尺子必须命中已修复竞态的修复后形状） | evictor 携带 `ExpectExecutionID = sbx.ExecutionID`（evict.go:181）；catalog 用 Lua 原子 compare-and-delete（catalog_redis.go:31-45）；live-map `MarkStopping` 在 shard 锁内核对 LifecycleID、fail-closed（map.go:195-212）。命中。 |
| 负例（对修复前 commit `84362fc6` 跑同一流程） | **未执行**，列入待办。当前判定的可信度依赖此步。 |
| 正常路径控制 | `Scenarios.lean` 已证明模型允许完整正常生命周期；本轮未把正常路径判为 violation。 |

## 2. Findings ledger

| ID | 映射点 | 层 | guard-diff | 判定 |
|---|---|---|---|---|
| F-1 | paused 沙箱的 DELETE（快照删除路径）× 在途 resume | B+C | **缺失**：终止意图无 tombstone；注册点无复核 | **Violates（已复现，[F-1 复现](e2b-f1-repro.md)；上报 [runtime#3636](https://github.com/e2b-dev/runtime/issues/3636)）** |
| F-2 | create 窗口（reservation → 节点创建 → `Add`）× kill | B | 同 F-1 形状，但新沙箱 ID 在 create 返回前不可知 | model_gap + Inconclusive（可达性存疑） |
| F-3 | 节点 `MarkRunning` 的 `InsertIfAbsent` 静默失败 | B | **缺失**：go-live 竞争失败后实例漏管、无错误、无清理 | 缺失（正常 API 流程规避；直连 gRPC 可达性未验证） |
| F-4 | pause 完成语义 | C | 远端 durable 为 fire-and-forget 异步上传（sandboxes.go:931,939,1567-1602）；stopped/网络关在返回后 | **Contract difference**（与源码分析 H2 一致；StrictLocal 有意更强） |
| F-5 | restore 身份绑定 | C | 快照身份=buildID 查找键；恢复后无核对、节点无 fence | **Conforms**（机制等价：每代新 buildID 结构隔离 + pause 只作用于 running 记录 + resume 抓取经 reservation 串行），但节点侧无 fence 属 A 类观察 |
| F-6 | keep_alive / refuse-pause 恢复 / refusal 转换 | C | 检查在写点锁内（keep_alive.go:22-25, operations.go:153-185; state_change.go:107,154,243,271） | Conforms |
| F-7 | fork（sandbox_fork.go） | — | 模型未建模 snapshot fork | Unsupported configuration |
| F-8 | auto-resume、动态网络策略、timeout 计时 | — | 有意出范围 | Unsupported configuration |

未发现缺失/错位的点（Conforms，正面机制记录）：catalog compare-and-delete 的 Lua 原子性等价于单步 guard；节点破坏性操作全部实例句柄化（cgroup/FC 进程/快照路径含 per-instance randomID，storage/sandbox.go:17-19,68-70），未发现按 sandboxID 的第二查找误伤路径；数据面新准入 live-map fail-closed（proxy.go:90-93）；连接/限流清理按 LifecycleID 隔离（proxy.go:139-141）。

## 3. F-1 详细：最小历史与逐行证据

### 背景：两种"paused"表示的分歧

模型中 paused 沙箱保留完整记录（phase=.paused + durableSnapshot + terminal=false），所以 kill 可以 `beginKill` 写 tombstone。**E2B 中 paused 沙箱 = running 记录不存在 + DB 快照行**。因此对 paused 沙箱的 kill 找不到任何记录，终止意图没有任何持久落点——这是 E2B 表示法与模型的根本映射分歧，F-1 是它的直接后果。

### 最小历史

```text
world: S 无 running 记录，DB 快照 B1（来源代 G）存在

R: POST /sandboxes/S/resume
   → Reserve 持有 pending（仅串行并发 start）
   → snapshotCache.Get 取 B1（sandbox_resume.go:154,215）
   → 节点 restore 开始（FC LoadSnapshot，较长）

K: DELETE /sandboxes/S
   → StartRemoving：running 记录不存在 → ErrNotFound
     （state_change.go:59-62 → delete_instance.go:42-48）
   → 不写任何 tombstone；不触碰节点上正在 restore 的实例
   → deleteSnapshot(B1)：soft-delete template + 缓存失效
     （sandbox_kill.go:85,21-37）
   → killedOrRemoved=true → 返回 204（sandbox_kill.go:85-103）

R: restore 完成
   → Add(S)：无条件 lockless SET+SADD 发布 Running
     （operations.go:20-44，注释自认 "Add is lockless"，
      state_change.go:76-79 "excludes the other lock-taking
      operations but not Add"）
   → resume 返回 201，S 以 Running 存活
```

**最终状态**：kill 返回成功（204）且快照已删，但沙箱 Running。调用者的 kill 意图丢失。

### 与 Lean 性质的矛盾

模型中这条历史不可实现，且 E2B 的复合结果违反已证明结论：

1. `beginKill` 后 `terminal = true`，`step_never_clears_terminal` 保证任何后续步骤不能清除；`publishRunning` 的 guard `live : record.terminal = false` 阻断在途 resume 的发布。协议语义：**被接受的 kill 胜出，最终 destroyed**。
2. 删除 paused 表示（快照）在模型中是 revoking 类副作用，只存在于 `beginKill` 之后的终态域；**不存在任何 Step 允许在 terminal 未置时删除 durable snapshot**（`PausedSnapshotValid`/`DurableSnapshotsImmutable` 覆盖其合法性）。E2B 在无 tombstone 的情况下施加了 revoking 类副作用。
3. `OperationResultStable`：kill 的 204 是对调用者公开的最终结果；同一 operation 域内"沙箱重新 Running"使该结果对调用者失真（协议观点；模型不建模 HTTP）。

### 修复方向（供 E2B conformance 记录，非本仓库行动项）

- kill 对"无记录但有快照"的沙箱写 tombstone（如 DB 快照行置删除标记 + resume 注册点 `Add` 改为条件脚本：`Add` 仅当无删除标记时成功），或
- resume 注册点复核（`Add` 前锁内检查 DB 快照是否已被 soft-delete）。

`Add` 的无锁形状与存储层其他写点（Lua 原子、锁内、`ExpectExecutionID`）明显不一致——存储层刻意按 execution 隔离（operations.go:31-36 注释），`Add` 是唯一无核对发布点。

## 4. 模型侧产出（A 类）

1. **映射分歧记录**：E2B 的 paused = 记录缺席；模型 paused = 有记录。任何"对 paused 的 kill"映射到模型时必须先问 tombstone 落点。已记入方法文档的 obs-map 要求。
2. **`activateVm` 的 `exclusive : noExecutingVm` guard 在 E2B 节点侧无对应物**（MarkStopping 先于 FC freeze，窗口内新代可 go-live；防护在 API 状态机层，节点层未兑现）。正常部署等价；节点层缺口即 F-3。
3. beginCreate 的原子性假设覆盖了 F-2 的窗口；E2B 的分解（accept → Reserve → 节点创建 → Add）是真实合同，模型如需严格 conformance 判定 F-2 的可达性，应把"沙箱 ID 何时对外可见"列入 obs-map 的不可观察声明，而不是假设不可见。

## 5. 待办

- [x] F-1 实机确认：本地控制面 harness（官方 dummy + 8s failpoint）稳定复现，见 [F-1 复现](e2b-f1-repro.md)。
- [ ] 负例校准：对 `84362fc6` 跑 Layer B/C，确认尺子命中两个历史竞态（方法可信度依赖）。
- [ ] F-2 可达性：确认 create 的沙箱 ID 由谁生成、何时对外可见。
- [ ] F-3：检查是否存在绕过 API 状态机直连节点 gRPC 的部署路径（edge/内部服务）。
- [ ] 填充 `references/conformance-maps/` 三份映射表（本轮已隐式覆盖 Controller 侧生命周期路径与节点侧，按方法要求显式成文）。
