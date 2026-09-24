# F-1 复现：kill × 在途 resume

复现日期：2026-09-13。对象：`e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`。结论：**在本地控制面 harness 上稳定复现**——DELETE 返回 204（删除成功）且快照删除路径执行，同时一个在途 resume 返回 201 并把沙箱重新发布为 Running。

这是 [Conformance 审计第一轮](e2b-conformance-round1.md) 中 F-1 从“源码级候选”到“实机复现”的验证记录。复现脚本 `scripts/repro_f1.sh`，原始输出 `references/f1-evidence.log`。已上报 E2B：[`e2b-dev/runtime#3636`](https://github.com/e2b-dev/runtime/issues/3636)（仓库由 `infra` 更名）。

## 1. 为什么不需要 Firecracker

F-1 整条竞态位于 **API 控制面**：kill 走 Redis running store + Postgres 快照删除，resume 的竞态点是最后写 running store 的无锁 `Add`。节点只需表现为“restore 进行中并最终成功”。因此用真实 api + 真实 Redis + 真实 Postgres + **官方 `dummy-orchestrator`** 即可，不需要 KVM/Firecracker。这正是模型里 adapter 用桩的形态；桩只替换节点执行，控制面逻辑是 E2B 原码。

## 2. Harness

| 组件 | 来源 | 说明 |
|---|---|---|
| Postgres | `postgres:18`，db `mydatabase`，user/pass `postgres/local` | 官方 integration 配置 |
| Redis | `redis:8` | 同上 |
| DB schema | `make -C packages/db migrate` | 官方迁移 |
| seed | `tests/integration/seed.go` | 合成 team / API key / template / build，无需真构建 |
| api | `make -C packages/api build` 后运行 `bin/api`，`ENVIRONMENT=local` | 自动 `local` service discovery，指向 `127.0.0.1:5008` |
| node | `packages/orchestrator/cmd/dummy-orchestrator`（官方 dummy） | 见下方两处测试改动 |

### dummy 的两处测试改动（`references/dummy-orchestrator-failpoint.patch`）

1. **可控 create 延迟**：`DUMMY_CREATE_DELAY_MS` 非零时在 `Create` 开头 sleep，把“节点 restore 窗口”从毫秒拉长到秒级，使并发请求能稳定插入。这是 evaluation.md 要求的确定性 failpoint。
2. **上报容量指标**：dummy 的 `ServiceInfo` 原本不上报 `MetricCpuCount`，导致放置评分除以 0 判为不可用；补上合成容量值。这是本地 harness 必需项，与被测逻辑无关。

两处都不改动任何生命周期语义：延迟只放慢节点 ack，容量只是让放置算法能选中 dummy。

## 3. 复现序列

```text
1. POST /sandboxes                         → 201，S running
2. POST /sandboxes/S/pause                 → 204，S paused（running 记录删除，Postgres 写入快照行）
3. POST /sandboxes/S/resume  （后台，节点 create 被延迟 8s）
4.   +2s: DELETE /sandboxes/S              → 204（StartRemoving 找不到 running 记录 → 走快照删除 → 成功）
5. resume 的节点 create 返回，api 无锁 Add → 201
6. GET /sandboxes/S                        → 200，state = running
```

## 4. 结果（`references/f1-evidence.log`）

```text
## race iteration 1/2/3
pause_http=204
kill_http=204
resume_http=201
final_state=running

## control: kill a paused sandbox with no resume in flight
pause_http=204
kill_http=204
GET /sandboxes/S → 404（保持删除）
```

- 三轮 race 全部：kill 204、resume 201、最终 running。
- 对照：无在途 resume 时，同样的 kill 204 之后沙箱保持删除（404）。**竞态被隔离**，不是“paused kill 本来就删不掉”的假象。

### 4.1 删除落点（`references/f1-deletion-evidence.log`）

直接查询数据库确认 kill 删除了什么，并在同一次运行中观测删除与复活：

```text
before : envs(快照env).deleted_at=NULL, active_envs=1
kill   : 204，期间 deleted_at=<时间>, active_envs=0
resume : 201
final  : state=running, redis running_key=1，快照 env 仍为 deleted_at=<时间>
```

`deleteSnapshot` → `SoftDeleteTemplate(TemplateID = snapshot.env_id)`，对快照 env 执行 `UPDATE envs SET deleted_at=NOW()` 并释放 alias、删除 active builds。此前“204 只可能在删除成功后返回”是代码推断，现已成为直接观测：**kill 确实删除了 paused 表示，却未阻止无锁 `Add` 重新发布 Running。**

api 日志时序（`references/f1-evidence.log` 对应轮次）：

```text
15:14:45  Serving DELETE: "Sandbox not found, already removed"  (kill_reason=request)
15:14:45  快照删除路径执行 → DELETE 返回 204
15:14:51  resume: "Sandbox created details"  (template.id=快照 env)
15:14:51  POST /resume        status=201 latency=8.02s
最终       GET /sandboxes/S    state=running
```

## 5. 对照 Lean 性质

固定映射下（paused 沙箱在 E2B 中无 running 记录；DELETE 映射为终止意图；`Add` 映射为发布 Running）：

- 被接受的终止意图必须有不可逆落点，之后任何发布 Running 都被阻断——模型由 `step_never_clears_terminal` 与 `publishRunning` 的 `record.terminal = false` guard 保证。E2B 在“无记录但有快照”的 paused 表示下没有终止落点，发布点 `Add` 也不核对，故模型禁止的历史在 E2B 可实现。
- 删除 paused 表示在模型中是 revoking 类副作用，只存在于 `beginKill` 之后；E2B 在无 tombstone 时施加了该副作用。

判定：**Violates（本地控制面 harness）**。按 [evaluation.md](evaluation.md) 的五分类，这不再是“模型缺口/合同差异”——用户可见的 kill 成功与最终 Running 直接矛盾。

## 6. 边界与诚实标注

- 未在真实 Firecracker 上跑：节点用官方 dummy 桩。由于竞态在控制面、桩只影响 restore 时长，这不削弱该路径的结论，但仍是本地 harness 而非云端复现。
- failpoint 把窗口从毫秒拉长到 8s。真实系统窗口更短，但竞态性质相同；窗口长度只影响触发概率，不影响可实现性。
- 未声称可在 E2B 云生产环境稳定触发，也未做多节点/多副本验证。下一步若是提 issue，附本 harness 与 patch 作为最小复现。
- 删除落点已由直接观测确定（`public.envs.deleted_at`，见 §4.1 与 `references/f1-deletion-evidence.log`），不再是未知项。

## 7. 复现步骤

```sh
# 前置：Docker(Colima) 运行中
docker run -d --name postgres -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=local \
  -e POSTGRES_DB=mydatabase -p 5432:5432 postgres:18
docker run -d --name redis -p 6379:6379 redis:8
make -C packages/db migrate POSTGRES_CONNECTION_STRING='postgres://postgres:local@localhost:5432/mydatabase?sslmode=disable'
# seed（见 tests/integration/seed.go，参数见 docs 正文）
# 给研究副本打 references/dummy-orchestrator-failpoint.patch，build-dummy
# 启动 dummy（DUMMY_CREATE_DELAY_MS=8000）与 api，然后：
scripts/repro_f1.sh 3
```

## 8. 严重性（补充实验）

`references/e2b-f1-severity-long2.log` 用**长 TTL 的 resume**（`{"timeout":3600}`）复测：

```text
DELETE -> 204
resume(timeout=3600) -> 201
t+0 / t+60 / t+180: GET -> 200    # 持续运行，列表中可见（listing contains: True）
re-pause -> 204
```

**危害是有界的，窗口 = resume 的 TTL**：

- 沙箱会随 TTL 到期被 evictor 回收（我们早期 15s 的样本就是被自己的超时收走的），**不是永久泄漏**。
- 但真实默认值并不小：SDK 的默认 sandbox timeout 是 **300 秒**（JS `DEFAULT_SANDBOX_TIMEOUT_MS = 300_000`、Python `default 300s`），上限 Hobby 1 小时、Pro 24 小时；裸 API 省略 timeout 时才是 15 秒。因此"删除成功"后的存活窗口默认约 5 分钟，长 TTL 沙箱则可达小时级。
- 该窗口内：沙箱被控制面当作 running（真实部署里数据面可达）、出现在列表、持续计费；删除语义失效。
- 数据留存同样有界：窗口内数据在运行实例里；窗口结束后被回收。若窗口内再次 pause，实测得到"paused 但无可用快照"的坏状态（唯一快照行已软删，resume 404），数据实际丢失而非长期保留。

对照说明：早期复现中沙箱很快消失，是因为测试里 resume 请求未带 timeout、采用了裸 API 默认 15s；日志 `end_time` 证实是**它自己的超时**回收，而不是 DELETE 生效（`references/e2b-f1-severity.log`）。
