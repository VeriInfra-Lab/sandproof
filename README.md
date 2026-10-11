# Sandproof

E2B 核心兼容的沙箱控制系统，采用轻中心、重宿主的目标架构，对声明边界内的安全关键 Rust 实现进行形式化验证。

先完成中心/宿主合同与功能拆分、故障及实机验收，再冻结真实组合提交推进证明。**当前本地服务已有实现与分批实测；中心服务和分布式组合仍待实现、验证，不能称为完整验证的系统。** 当前范围以[项目蓝图](docs/project-blueprint.md)为准，任务进度见[总计划 #1](https://github.com/ningZhang-cs/sandproof/issues/1)。

## 我们要做什么

用户经统一中心访问沙箱，中心使用权威 Redis 完成全局鉴权、准入、固定宿主选择和路由；各 Linux/KVM 宿主保留自己的 Rust 控制器、SQLite、Firecracker/jailer 执行和恢复能力，支持多用户、多沙箱。在固定版本、明确功能子集内兼容 E2B 客户端：

- 创建、连接运行中沙箱、查询、寿命续期、到期销毁和主动销毁。
- 重复命令执行、后台任务、输出，以及运行期间的文件读写与传输。
- 私有可写系统盘、通用 Dockerfile/OCI 模板、鉴权、资源限额和受控网络。
- 持久记录、提交后派发、未知结果处理、精确资源身份和重启恢复。

首次工程交付在一台物理 KVM 主机上运行中心、Redis 和一个真实宿主，经双向认证 HTTPS 通信；多台真机验收另行推进。暂停/恢复、快照、共享卷、自动迁移、中心多活、Redis 自动切换、完整云平台复制和自动部署不在当前交付。当前使用冻结的 Terminal-Bench 2.0 兼容回归；两题预装晋升回归与 2.1 原题 89 题全量待办分别见[验收清单](docs/testing-plan.md#4-通用模板与-terminal-bench)，版本锁与工具入口见[评测说明](benchmarks/README.md)。兼容性与评测不能替代安全证明。

通用模板链已接通显式部署的自助 Dockerfile/公开 OCI 构建、转换、发布和原 SDK 创建；实际功能与资源验收按对应 issue/PR 的精确提交和配置判断，不把历史成绩当新候选通过。回归与发布前全题集待办见 [#13](https://github.com/ningZhang-cs/sandproof/issues/13)、[#14](https://github.com/ningZhang-cs/sandproof/issues/14) 及[验收清单](docs/testing-plan.md)。模板构建/转换及其自有编排、登记/发布整体以测试验收，不做形式化证明；核心控制流程的外部前提和准确证明范围见[覆盖记录](docs/proof-coverage.md)。

## 现在有什么

| 部分 | 当前状态 |
|---|---|
| Rust 控制核心 | 已有纯状态处理实现与单元测试，不包含真实存储、HTTP 服务或 VM 操作 |
| Lean 规范与对应证明 | 已有旧版纯控制核心的协议安全和 Rust 对应证明，带明确容量前提；当前 Rust 新增输入尚未同步证明，适用版本见[覆盖记录](docs/proof-coverage.md) |
| 最终完成性与长期容量 | 仍有未闭合义务，见[证明覆盖记录](docs/proof-coverage.md) |
| E2B 兼容服务与运行层 | 本地 SDK 管理/数据、网络、恢复、多模板隔离、自助构建及通用题目链已有分批实机证据；支持范围按候选验收，不自动覆盖新增中心/双服务 |
| 中心/宿主拆分 | 待实现：中心控制与 Redis、内部协议、两端网关及其工程验收，见 [#292](https://github.com/ningZhang-cs/sandproof/issues/292) |
| CI | 普通隔离检查和两题 KVM 晋升已接通；分布式组件/双服务故障组为待实现扩展；规则见开发指南 |

## 开发入口

- [总计划与任务](https://github.com/ningZhang-cs/sandproof/issues/1)、[阶段里程碑](https://github.com/ningZhang-cs/sandproof/milestones)
- [开发指南](CONTRIBUTING.md)：分支、提交、issue、PR、测试和文档维护规则的统一入口
- [文档目录](docs/README.md)：蓝图、合同、验收清单与模型／证明说明的总分索引
- [系统架构](docs/ARCHITECTURE.md)：目标组件、当前本地实现、运行路径、存储与 E2B 对照
- [形式化验证设计](docs/formal-verification-design.md)：目标性质、状态与故障模型、精化和系统组合
- [CI 运行记录](https://github.com/ningZhang-cs/sandproof/actions)

分工以蓝图为准，开发流程以开发指南为准；不在本页复制规则和实时测试数量。

## 代码和证明

| 目录 | 用途 |
|---|---|
| `controller-core/` | 已有 Rust 纯控制核心 |
| `SandboxModel/` | Lean 状态机、安全性质、处理函数及 Rust 对应证明 |
| `verification/` | 固定版本 Charon/Aeneas 抽取结果、工具链说明和定理依赖审计 |
| `scripts/` | 功能检查、抽取与完整证明验收入口 |
| `tools/` | 模板构建服务/客户端、共享配置、CI/验收工具与 Lean/研究辅助；产品入口见[架构](docs/ARCHITECTURE.md)，评测入口见[评测说明](benchmarks/README.md) |
| `docs/` | 蓝图、架构、合同、验收、验证设计与证明覆盖 |

服务、接口和宿主运行模块的职责与目录见[系统架构](docs/ARCHITECTURE.md)。crate 已存在不等于对应工作包已完整验收，不以目录数量计算完成度。

## 本地检查

使用仓库固定的 Rust 工具链执行功能检查：

```sh
bash scripts/check-function.sh
```

证明开发遵循[证明工作流](docs/proof-workflow.md)；完整验收执行：

```sh
scripts/bootstrap-aeneas.sh  # 首次准备固定版本工具链
scripts/verify-controller.sh
```

Lean 是唯一协议规范来源。Charon/Aeneas 将实际 Rust 转为 Lean 函数，再与规范证明对应，而不是另写一份看起来相似的实现。抽取受固定工具链支持子集及外部模型限制，不能视为任意 Rust 的无条件无损翻译。详见[验证设计](docs/formal-verification-design.md)、[验证工具链](verification/README.zh-CN.md)和[本地状态机说明](docs/state-machine.md)。

验证结论只覆盖声明的代码、输入、故障模型与前提，不自动覆盖新增运行服务。Linux/KVM、Firecracker/jailer、存储引擎及底层文件/网络原语必须列明合同；不声称证明虚拟化逃逸不存在、完整 POSIX/ACL 或恶意宿主安全。
