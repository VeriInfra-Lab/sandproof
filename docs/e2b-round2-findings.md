# E2B 第二轮测试发现

日期：2026-09-14。对象：`e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`（本地控制面 harness，官方 dummy orchestrator + 让 pause/delete/create 可延迟的 failpoint）。方法：`tools/trace/run_scenarios.py` 的 17 个综合场景 + 手动核对观测。

**结论**：在 F-1 之外发现两处新的不一致性。G-1 是确定性、可复现的资源孤儿（部分失败清理缺失）；G-2 是 kill 后仍接受 pause 的 guard 违背（严重性较低）。

## G-1：create 请求超时留下节点孤儿（确定复现）

**触发**：客户端在 create 期间超时（客户端 1s，节点 create 耗时 8s）。API 报“创建失败/request timed out after 1 placement attempt”，但节点已经创建了 VM，且**没有任何补偿**。

**证据**（`references/e2b-orphan-evidence.log`）：

```text
before: store [] node []
create with client timeout=1s -> None timed out
t+10s: node_only=['i5ps69jmgxpn4q8kzkhvz']
t+15s: node_only=['i5ps69jmgxpn4q8kzkhvz']
t+20s: node_only=['i5ps69jmgxpn4q8kzkhvz']
t+25s: node_only=['i5ps69jmgxpn4q8kzkhvz']

api: ERROR Failed to create sandbox {"sandbox.id":"i5ps69jmgxpn4q8kzkhvz",
       "node.id":"local", "error":"[Canceled] context canceled"}
api: WARN  error when creating instance "... request timed out after 1 placement attempt(s)"
```

**根因**：客户端取消 → API 请求 context 取消 → 放置返回超时错误；但节点侧 `Create` 已（或仍会）完成并写入实例表。API 的失败路径没有回收节点实例，也没有写 running 记录，于是产生一个**节点上有、store 和索引里都没有**的孤儿。

**严重性更正（实测）**：孤儿**不是永久的**。API 有兜底：`Store.Reconcile → killOrphanSandbox` 会杀掉"节点上有、store 里没有"的实例，宽限期 `orphanGracePeriod = time.Minute`。实测孤儿存活约 1 分钟后被 `Killing orphaned sandbox not found in store` 回收（证据 `references/e2b-g1-severity.log`）。因此 G-1 是**有界泄漏（≤1 分钟）**：窗口内有一个未注册的 VM 在跑、对 API/索引/路由不可见，且创建失败路径本身不做补偿。`docs/e2b-g1-issue-body.md`（已同步到 #3637）按此更正。

**根因的模型含义**：这对应 `Compensation.lean` 的对象——失败应调用补偿回收；E2B 在此路径没有补偿，靠周期性 reconcile 兜底。

**与 E2B#1498 的区别**：E2B#1498 是 MCP 启动失败留 running sandbox；本项是 create 请求取消/超时留**未注册**的节点孤儿। 两者同属“部分失败清理缺失”。

## G-2：kill 成功后仍接受 pause（guard 违背）

**触发**：`race_pause_kill` 场景——create 后并发发起 pause（节点延迟 4s）与 kill。kill 先返回 204，随后 pause 也返回 204。

**证据**（`references/trace/scenarios/race_pause_kill.jsonl`）：

```text
seq3 EVENT acceptKill  http=204 success
seq5 EVENT acceptPause http=204 success
```

**与模型的关系**：`acceptPause` 的 guard 要求 `phase = running` 且 `terminal = false`；`beginKill` 已置 `terminal = true`，故 pause 必须被拒绝。E2B 返回 204。检测规则扩展为 `kill_irreversible` 同时禁止“kill 后成功的 pause”。

**严重性**：本轮观测中该 pause 产生的快照行随后被软删（`snapshot_deleted=True`），未直接导致复活，**严重性较低**；但它破坏了“被接受的 kill 不可逆”的 guard，是 `TerminalConsistent` 类一致性偏差，应记录。

## 规则与场景更新

- 新增规则：`node_orphan`（G-1）、`kill_irreversible` 扩展覆盖 pause-after-kill（G-2）。
- 新增性质：`NoOrphanOnNode`；`CreateCompensation` 升级为 monitored。
- 场景从 12 增至 17（新增 `race_pause_kill`、`race_double_kill`、`kill_settle`、`autopause_resume`、`client_timeout`）。
- 自测从 26 增至 29。

## 场景结果

```text
Conforms: normal, dup_kill, dup_pause, resume_resume, kill_resume, kill_pause,
          multi, expire, bad_create, race_double_kill, kill_settle,
          autopause_resume
Violates: race (F-1), stress (F-1 ×2), stale_index_control (正例),
          race_pause_kill (G-2), client_timeout (G-1)
```

## 诚实边界

- 节点为官方 dummy 桩；但 G-1 的孤儿由 **api 控制面**产生（节点只是执行者），桩不影响结论。
- 未在真实 Firecracker 上验证；未在云生产复现。
- G-1 的“节点 Create 在 cancel 后仍完成”依赖我们给 dummy 加的 8s 延迟来放大窗口；真实系统窗口更短，但机制相同（cancel 后副作用仍发生）。是否可达取决于真实 create 耗时与客户端超时设置。

## 复现

```sh
python3 tools/trace/collect.py client_timeout > /tmp/t.jsonl
python3 tools/trace/check.py /tmp/t.jsonl     # Violates: node_orphan
```