# Trace 格式与映射

本文定义 E2B 轨迹（trace）的线格式，以及轨迹记录如何映射到 Lean 模型的 `Event` 和可观察 `World` 字段。它是 `model-based-testing-survey.md` 中桥接架构的 ①② 两层的具体化。

## 1. 线格式（JSON Lines）

每个轨迹文件是一个 JSONL：每行一个记录，按 `seq` 递增、`ts` 非递减。

```json
{
  "seq": 4,
  "ts": 1726200000123456789,
  "source": "client",
  "kind": "event",
  "event": "acceptKill",
  "sandbox": "i2saxe0lr0qi78756nkqq",
  "attrs": {
    "method": "DELETE",
    "path": "/sandboxes/i2saxe0lr0qi78756nkqq",
    "http": 204,
    "result": "success",
    "raw_body": ""
  }
}
```

```json
{
  "seq": 6,
  "ts": 1726200000223456789,
  "source": "observer",
  "kind": "observe",
  "sandbox": "i2saxe0lr0qi78756nkqq",
  "obs": {
    "running": false,
    "snapshot_present": true,
    "snapshot_deleted": true,
    "phase_hint": "paused"
  }
}
```

字段说明：

| 字段 | 含义 |
|---|---|
| `seq` | 单调递增序号 |
| `ts` | 进程内单调时钟（纳秒）。跨组件不依赖 wall clock；如需因果序，另加 `cause` token |
| `source` | `client` / `observer` / `api` / `store` / `node` |
| `kind` | `event`（一次被观测的动作）或 `observe`（一次状态采样） |
| `event` | 抽象后的事件名（见 §2） |
| `sandbox` | 抽象 sandbox 标识 |
| `attrs` | 原始证据：HTTP 方法/路径/状态码、原始 body、内部调用结果 |
| `attrs.result` | `success` / `reject` / `unknown`（timeout 或连接失败） |
| `obs` | 部分可观察状态；缺省字段表示**不可观察**，checker 不施加约束 |

## 2. 事件映射（evt）

从 API/存储层观测到的动作映射到 Lean `Event` 构造子。这里固定的是首版 controller 生命周期子集：

| 观测动作 | 条件 | Lean `Event` | 角色 |
|---|---|---|---|
| `POST /sandboxes` | 2xx | `.acceptCreate` | controller |
| `POST /sandboxes/{id}/pause` | 2xx | `.acceptPause` | controller |
| `POST /sandboxes/{id}/resume` | 2xx | `.acceptResume` | controller |
| `DELETE /sandboxes/{id}` | 2xx | `.acceptKill` | controller |
| 任一生命周期请求 | timeout/无响应 | `.rpcTimeout` | controller |
| 存储层 `StartRemoving` | — | `.acceptKill`/`.acceptPause` 的持久化部分 | controller |
| 存储层 `Add`（running 注册） | — | `.publishRunning` 的持久化部分 | controller |
| 节点 Create/Pause/Stop/Cleanup | — | `.prepareVm`/`.freezeVm`/`.stopVm`/`.cleanupVm` | worker |
| 快照 env 软删除 | — | `.beginKill`/`.deleteSnapshot` 的 revoking 副作用 | controller |

**映射纪律**：只映射可观察、且语义明确的动作；`2xx` 的 DELETE 视为**已接受的终止意图**（对应 `beginKill` 写 terminal），这是 F-1 判定的关键映射。无法确定语义的动作记为 `unknown`，不用于判 Violates。

## 3. 状态观察映射（obs）

`obs` 是抽象 `World` 的部分投影。首版可观察字段：

| obs 字段 | Lean 对应 | E2B 观察点 |
|---|---|---|
| `running` | `record.effective ≠ .closed` 的代理 | running store 是否存在该 sid 的记录 |
| `phase_hint` | `record.phase`（部分） | `GET /sandboxes/{id}` 的 `state`；404 + 快照存在 → paused |
| `snapshot_present` / `snapshot_deleted` | `durableSnapshot` / 已删的 paused 表示 | `snapshots` 行存在；`envs.deleted_at` 是否置位 |
| `terminal` | `record.terminal` | 由成功的 kill/pause 事件推导 |
| `generation` | `record.generation` | **不可观察**（E2B 不通过 API 暴露代）；留空 |
| `epoch` | `record.epoch` | **不可观察**；留空 |
| `operation` | `record.currentOperation` | 请求携带的 operation 标识与结果 |
| `reclaimed` | `ReclaimedResourcesNotCurrent` | VM/slot 回收在本地 harness 中不可直接观察；留空 |

**不可观察字段显式留空**，checker 不对其施加约束（对应 TLA+ trace validation 的“只追踪部分变量”）。`generation`/`epoch` 留空意味着我们**不**能靠轨迹声称它们单调；这些性质由静态审计和 adapter 合同承担，而非本轨迹层。

## 4. 未知语义

- `attrs.result = "unknown"`：请求可能已生效、也可能未生效。checker 保留两种可能；若某条规则的违反只依赖 unknown 事件，判 **Inconclusive**，不判 Violates。
- `reject`：请求被明确拒绝，映射为该事件未发生（stutter），不改变抽象状态。

## 5. 判定

checker 对每条轨迹输出五分类之一：Conforms / Violates / Inconclusive / Contract difference / Unsupported。Violates 必须给出：违反的规则名、对应的 Lean 定理名、最小事件子序列、原始证据行。
