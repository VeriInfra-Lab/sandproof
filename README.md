# Sandproof

**简体中文** · [English](README.en.md)

Sandproof 是一个以 Rust 实现、以 Lean 验证关键控制逻辑的有状态沙箱系统。当前目标是：在管理服务崩溃、请求重试和并发操作下，实现可靠的暂停卸载、同宿主恢复及资源回收。

## 当前目标

第一阶段聚焦三个相互关联的问题：

- **保住最新状态**：暂停时保存同一次冻结期间的全量内存、VM/设备状态和匹配的私有磁盘，再卸载旧 VM；恢复延续原会话状态。
- **保证唯一执行资格**：恢复为新的运行代，旧执行者、重复请求和迟到回执不能造成重复执行；客户端通过操作键查询原请求结果。
- **安全回收材料**：卸载后释放运行位置，保留会话及恢复材料的记账；仍被恢复过程或运行 VM 使用的材料不能提前删除。

保留真实中心→宿主网络链路：一个有效中心写者及权威 Redis，一个固定 Linux/KVM 宿主及其 SQLite，支持多用户、多会话。中心负责全局授权、绑定、预留和意图；宿主负责本地控制、持久化、实际执行与精确清理。Redis、SQLite、文件和 VMM 之间没有跨层原子事务。

当前范围以[总计划 #1](https://github.com/VeriInfra-Lab/sandproof/issues/1)和 [stage1 项目蓝图](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/project-blueprint.md)为准；新的 V0–V9 任务见总计划中的 #632–#641。交付要求包括真实系统、指定 Rust 实现的条件证明，以及可复现的状态连续性、故障和资源实验。

## 目标与现状

既有中心/宿主、冷启动与清理、E2B 核心兼容、模板链、网关和公共证明是后续工作的基础。**新的有状态生命周期正在分批实现与验证，完整 create → unload → 管理进程重启 → resume → destroy 闭环尚不能声明已验收或已证明。** 纯核心中的 Pause/Resume 类型、局部组件测试和历史证明都不能代替真实产品链路的验收。

当前阶段设计维护在 `stage1`，尚未整体进入本分支的文档树。阅读最新范围、合同与验收要求请使用下方 stage1 链接；本分支的代码和[覆盖记录](docs/proof-coverage.md)仅说明各自已集成的基线，后续候选的实际结果见对应 issue/PR。

第一阶段要求同宿主全量快照及先加载为暂停状态，暂停/恢复不重置会话总寿命；新生命周期操作需要客户端首次发送前已知的操作键。旧无键 SDK create 不自动获得安全重试语义，也不承诺旧 TCP/vsock 连接在恢复后连续。

多宿主迁移、fork、增量快照、对象存储、中心高可用、完整来宾协议证明，以及任意 VMM 死亡后的 RAM 恢复均不在本阶段范围。模板构建与镜像转换继续通过工程测试验收；**运行态快照管理、恢复资格和清理决策属于本阶段必须验证的自有业务**。

## 开发入口

详细文档目前主要为中文；本页提供英文入口，未表示全部文档已有译本。

- [总计划与任务](https://github.com/VeriInfra-Lab/sandproof/issues/1)、[里程碑](https://github.com/VeriInfra-Lab/sandproof/milestones)
- [开发指南](CONTRIBUTING.md)：分支、提交、PR、测试和文档维护规则；任务 → integration → test → main
- [本分支文档目录](docs/README.md)：已集成基线的文档与工具说明
- [当前阶段蓝图](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/project-blueprint.md)、[架构](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/ARCHITECTURE.md)、[模块合同](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/module-contracts.md)：最新目标、责任及失败语义
- [当前验证设计](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/formal-verification-design.md)、[证明覆盖](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/proof-coverage.md)、[验收清单](https://github.com/VeriInfra-Lab/sandproof/blob/stage1/docs/testing-plan.md)：目标性质、适用范围及实机判据
- [CI 运行记录](https://github.com/VeriInfra-Lab/sandproof/actions)

## 代码和证明

| 目录 | 用途 |
|---|---|
| `sandbox-center-core/`、`sandbox-center-store/` | 中心纯决策与 Redis 持久协调 |
| `controller-core/`、`controller-service/` | 宿主本地控制、SQLite 和恢复编排 |
| `sandbox-node/`、`sandbox-runtime/` | 节点协议接入、真实资源执行与精确清理 |
| `sandbox-center/`、`sandbox-server/` | 中心与受保护宿主服务的装配入口 |
| `SandboxModel/` | Lean 模型、性质及指定 Rust 的对应证明 |
| `verification/` | 固定版本 Charon/Aeneas 抽取、工具链说明和证明依赖审计 |
| `scripts/`、`tools/` | 功能检查、证明验收、部署与开发辅助工具 |
| `docs/`、`benchmarks/` | 已集成设计与覆盖记录，以及固定兼容/评测入口 |

## 本地检查

先按[开发指南](CONTRIBUTING.md)及 [CI 环境说明](docs/ci.md)准备固定工具链、Python 和校验后的 Redis，并显式设置 `SANDPROOF_REDIS_SERVER`。

```sh
bash scripts/check-function.sh             # Linux 完整基础检查
bash scripts/check-function.sh --portable  # macOS 开发检查，不代替 Linux 验收
```

证明开发先完整阅读[证明工作流](docs/proof-workflow.md)，使用其指定服务与固定工具链；完整证明验收入口为：

```sh
scripts/bootstrap-aeneas.sh  # 首次准备固定版本工具链
scripts/verify-controller.sh
```

Lean 是协议规范来源；Charon/Aeneas 将指定真实 Rust 抽取为 Lean 函数，再证明其与规范的对应关系。抽取受固定工具链支持子集和外部合同限制。模型定理、实现对应、原生测试及 Linux/KVM 实测分别报告；已有结论只适用于记录的代码、输入、故障模型和前提，不代表整个系统、虚拟化隔离或底层 OS 已被完整验证。
