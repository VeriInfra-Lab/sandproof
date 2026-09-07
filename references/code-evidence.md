# 固定源码证据索引

阅读日期：2026-09-07。所有链接固定到提交 SHA；行号与本地原始文件一致。源码阅读不是动态缺陷复现。

| 仓库 | 固定提交 | 参考版本 |
|---|---|---|
| [e2b-dev/infra](https://github.com/e2b-dev/infra) | `82f6529660ea98087d7a479247c789905b0a3867` | `main` |
| [e2b-dev/E2B](https://github.com/e2b-dev/E2B) | `473d8bf3e62b68ee731cf18afb2e8258f9ca7a7c` | `main` |
| [firecracker-microvm/firecracker](https://github.com/firecracker-microvm/firecracker) | `2038188f145fb81b8d098147a10e9d9f392fd22f` | `v1.16.1` |

| 编号 | 实际阅读位置 | 对研究的作用 |
|---|---|---|
| E01 | [docs/ARCHITECTURE.md:11–33](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/docs/ARCHITECTURE.md#L11-L33) | Firecracker 与控制面/数据面分离的总体架构 |
| E02 | [packages/api/internal/sandbox/sandboxtypes/states.go:95–116](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/sandbox/sandboxtypes/states.go#L95-L116) | 运行记录只有 running/pausing/killing/snapshotting；paused 另查快照 |
| E03 | [packages/api/internal/sandbox/sandboxtypes/states.go:82–94](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/sandbox/sandboxtypes/states.go#L82-L94) | 已有实例标识与延迟删除保护，不将 epoch 本身作为新贡献 |
| E04 | [packages/api/internal/sandbox/storage/redis/state_change.go:35–174](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/sandbox/storage/redis/state_change.go#L35-L174) | 分布式锁、转换 ID、执行 ID 复核、转换完成回调 |
| E05 | [packages/api/internal/sandbox/storage/redis/scripts.go:30–64](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/sandbox/storage/redis/scripts.go#L30-L64) | 执行 ID 检查在 Lua 写入处，而非只在 Go 读路径 |
| E06 | [packages/api/internal/sandbox/reservations/redis/README.md:12–19](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/sandbox/reservations/redis/README.md#L12-L19) | 创建/恢复的 reservation、结果键、通知与轮询 |
| E07 | [packages/api/internal/handlers/sandbox_connect.go:84–194](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/handlers/sandbox_connect.go#L84-L194) | connect 先保活，等待过渡，必要时从快照恢复 |
| E08 | [packages/api/internal/handlers/sandbox_resume.go:97–183](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/handlers/sandbox_resume.go#L97-L183) | 显式 resume 对 running/killing/pausing 的处理 |
| E09 | [packages/api/internal/handlers/sandbox_kill.go:39–114](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/handlers/sandbox_kill.go#L39-L114) | 删除运行记录与暂停快照分属不同调用步骤 |
| E10 | [packages/api/internal/orchestrator/pause_instance.go:29–133](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/orchestrator/pause_instance.go#L29-L133) | 快照数据库记录、节点 Pause RPC、build success 与缓存失效 |
| E11 | [packages/api/internal/orchestrator/delete_instance.go:136–193](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/orchestrator/delete_instance.go#L136-L193) | 暂停拒绝的回滚/兜底删除受配置条件约束 |
| E12 | [packages/orchestrator/pkg/server/sandboxes.go:659–727](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/server/sandboxes.go#L659-L727) | MarkStopping 后后台 Stop，RPC 不等待全量清理 |
| E13 | [packages/orchestrator/pkg/server/sandboxes.go:809–982](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/server/sandboxes.go#L809-L982) | 节点 pause 预检、快照缓存、异步上传与后台停止 |
| E14 | [packages/orchestrator/pkg/server/sandboxes.go:1567–1607](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/server/sandboxes.go#L1567-L1607) | 上传重试及成功/失败完成记录；不能等同远端已持久化 |
| E15 | [packages/orchestrator/pkg/sandbox/sandbox.go:356–365](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/sandbox/sandbox.go#L356-L365) | ExecutionID 与具体 Firecracker 进程 LifecycleID 不同 |
| E16 | [packages/orchestrator/pkg/sandbox/map.go:192–216](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/sandbox/map.go#L192-L216) | 节点 live map 的删除按 LifecycleID 核对 |
| E17 | [packages/orchestrator/pkg/proxy/proxy.go:64–148](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/proxy/proxy.go#L64-L148) | 代理从 live map 找实例，连接池按 LifecycleID 分组 |
| E18 | [packages/api/internal/sandbox/sandbox_envd_secret.go:27–35](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/sandbox/sandbox_envd_secret.go#L27-L35) | access token 从 sandbox ID 派生；不能假定每次恢复都撤销用户句柄 |
| E19 | [packages/orchestrator/pkg/sandbox/sandbox.go:1643–1678](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/orchestrator/pkg/sandbox/sandbox.go#L1643-L1678) | 物理停止包括 FC/cgroup 终止与等待进程退出 |
| E20 | [packages/js-sdk/src/sandbox/sandboxApi.ts:1469–1514](https://github.com/e2b-dev/E2B/blob/473d8bf3e62b68ee731cf18afb2e8258f9ca7a7c/packages/js-sdk/src/sandbox/sandboxApi.ts#L1469-L1514) | SDK 将 HTTP 409 映射为 false，必须保存原始响应 |
| E21 | [packages/js-sdk/src/sandbox/sandboxApi.ts:1193–1231](https://github.com/e2b-dev/E2B/blob/473d8bf3e62b68ee731cf18afb2e8258f9ca7a7c/packages/js-sdk/src/sandbox/sandboxApi.ts#L1193-L1231) | SDK kill 的布尔值与原始 HTTP 状态的对应 |
| E22 | [packages/js-sdk/src/sandbox/commands/index.ts:427–489](https://github.com/e2b-dev/E2B/blob/473d8bf3e62b68ee731cf18afb2e8258f9ca7a7c/packages/js-sdk/src/sandbox/commands/index.ts#L427-L489) | commands.run 最终调用独立 envd process RPC |
| F01 | [docs/snapshotting/snapshot-support.md:87–110](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/docs/snapshotting/snapshot-support.md#L87-L110) | Firecracker 不替集成方管理快照可信性与生命周期 |
| F02 | [docs/snapshotting/snapshot-support.md:180–200](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/docs/snapshotting/snapshot-support.md#L180-L200) | 暂停 API 的前置条件与成功语义 |
| F03 | [docs/snapshotting/snapshot-support.md:242–316](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/docs/snapshotting/snapshot-support.md#L242-L316) | 全量快照的组成；宿主 FS 刷新不等于持久介质保证 |
| F04 | [docs/snapshotting/snapshot-support.md:395–485](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/docs/snapshotting/snapshot-support.md#L395-L485) | 恢复默认停在 Paused；可显式禁用自动 resume；内存文件须不可变 |
| E23 | [packages/api/internal/orchestrator/create_instance.go:177–258](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/orchestrator/create_instance.go#L177-L258) | CreateSandbox 的 reservation 与并发启动等待 |
| E24 | [packages/api/internal/orchestrator/create_instance.go:435–531](https://github.com/e2b-dev/infra/blob/82f6529660ea98087d7a479247c789905b0a3867/packages/api/internal/orchestrator/create_instance.go#L435-L531) | 节点创建之后的 running store 注册与失败清理路径 |
| E25 | [catalog_redis.go:81–108（修复前）](https://github.com/e2b-dev/infra/blob/84362fc62d94efd64370ce41e53ad00fd95772d7/packages/shared/pkg/sandbox-catalog/catalog_redis.go#L81-L108) | `GET → ExecutionID 核对 → DEL` 分成两次 Redis 操作，中间可被新 execution 的 Store 插入 |
| E26 | [catalog_redis.go:15–113（`2a0c0af`）](https://github.com/e2b-dev/infra/blob/2a0c0af69a0405500eb6e7eaded32349f7f4c7fa/packages/shared/pkg/sandbox-catalog/catalog_redis.go#L15-L113) | 修复使用 Redis 原子 compare-and-delete，提交注释明确说明防止 stale teardown 删除 newer execution |
| E27 | [evict.go:159–175（`eef64c7`）](https://github.com/e2b-dev/infra/blob/eef64c73e48c3134ef9d76317e37ceffddcdb3e4/packages/api/internal/orchestrator/evictor/evict.go#L159-L175) | 过期扫描后的 pause/kill 决策改为携带 `ExpectExecutionID`，拒绝 resume 之后的旧决策 |
| E28 | [pause_instance.go:50–72（`42f6997`）](https://github.com/e2b-dev/infra/blob/42f699786534f1c0ad625ce9b4b7b922b3a16834/packages/api/internal/orchestrator/pause_instance.go#L50-L72) | Pause RPC 失败后也必须将已创建的 snapshot build 写成失败终态 |
| E29 | [snapshot_template.go:150–200（`8586a66`）](https://github.com/e2b-dev/infra/blob/8586a66a5e84504536b2df9b7e28efe9069d8284/packages/api/internal/orchestrator/snapshot_template.go#L150-L200) | snapshot build 的最终结果使用脱离 client cancellation 的有界 context 持久化 |

## 官方文档与相关工作

- [E2B persistence](https://docs.e2b.dev/sandbox/persistence)：公开的生命周期与 connect 语义。
- [E2B auto-resume](https://docs.e2b.dev/sandbox/auto-resume)：入站流量可能唤醒暂停实例。
- [Firecracker v1.16.1 release](https://github.com/firecracker-microvm/firecracker/releases/tag/v1.16.1)：本项目唯一实现后端基线。
- [Anvil, OSDI 2024](https://www.usenix.org/conference/osdi24/presentation/sun-xudong)：controller 的并发、失败和最终收敛验证，排除笼统的“首个 verified controller”主张。
- [IronFleet, SOSP 2015](https://www.microsoft.com/en-us/research/wp-content/uploads/2015/10/ironfleet.pdf)：状态机 refinement 与实现级推理衔接。

检索范围有限，尚未完成系统性 novelty review。上述工作用于界定方向，不能支撑“本领域首个”的结论。
