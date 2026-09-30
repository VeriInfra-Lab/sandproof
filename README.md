# Sandproof

单节点、E2B 核心兼容的沙箱控制系统，目标是对安全关键的 Rust 实现进行形式化验证。

当前先完成 V1 功能和真实测试，再推进完整生命周期与证明。**这仍是开发中的项目，不是已经完整验证、可直接部署的沙箱服务。** 当前范围以[项目蓝图](docs/project-blueprint.md)和[总计划 #1](https://github.com/ningZhang-cs/sandproof/issues/1)为准。

## 我们要做什么

在单台 Linux/KVM 主机上，使用自己的 Rust 控制器与执行层管理 Firecracker/jailer，支持多用户和多个沙箱。在固定版本、明确功能子集内兼容 E2B 客户端：

- 创建、连接运行中沙箱、查询、寿命续期、到期销毁和主动销毁。
- 重复命令执行、后台任务、输出，以及运行期间的文件读写与传输。
- 私有可写系统盘、通用 Dockerfile/OCI 模板、鉴权、资源限额和受控网络。
- 持久记录、提交后派发、未知结果处理、精确资源身份和重启恢复。

暂停恢复在后续阶段实现；多节点、共享卷、完整云平台复制和自动部署不在 V1。兼容性与 Terminal-Bench 评测不能替代安全证明。

通用模板链为新增开发目标，当前尚未交付，见 [#123](https://github.com/ningZhang-cs/sandproof/issues/123)。模板构建/转换及其自有编排、登记/发布整体以测试验收，不做形式化证明；核心控制流程的外部前提和准确证明范围见[覆盖记录](docs/proof-coverage.md)。

## 现在有什么

| 部分 | 当前状态 |
|---|---|
| Rust 控制核心 | 已有纯状态处理实现与单元测试，不包含真实存储、HTTP 服务或 VM 操作 |
| Lean 规范与对应证明 | 已有旧版纯控制核心的协议安全和 Rust 对应证明，带明确容量前提；当前 Rust 新增输入尚未同步证明，适用版本见[覆盖记录](docs/proof-coverage.md) |
| 最终完成性与长期容量 | 仍有未闭合义务，见[证明覆盖记录](docs/proof-coverage.md) |
| E2B 兼容服务与运行层 | 固定 base 的选定 SDK 管理/数据、禁网及公网/静态 IPv4、恢复路径已有实机证据；各批范围分别记录，通用模板、完整评测和最终部署容量仍未交付，不能作为完整 V1 部署 |
| CI | 功能、文档和 PR 规则检查；实际门槛与运行方法统一见开发指南 |

已移除的原型可从 [Git 历史](https://github.com/ningZhang-cs/sandproof/tree/0f1901a210e24da50d259e07df75e8c2cb9d181f/sandbox-runtime)查看。当前目录不再保留旧方案的副本，也不把删除旧模块计算为新功能完成。

## 开发入口

- [总计划与任务](https://github.com/ningZhang-cs/sandproof/issues/1)、[阶段里程碑](https://github.com/ningZhang-cs/sandproof/milestones)
- [开发指南](CONTRIBUTING.md)：分支、提交、issue、PR、测试和文档维护规则的统一入口
- [文档目录](docs/README.md)：蓝图、合同、验收清单与模型／证明说明的总分索引
- [系统架构](docs/ARCHITECTURE.md)：V1 组件、运行路径、存储用途及与 E2B 的逐项对照
- [CI 运行记录](https://github.com/ningZhang-cs/sandproof/actions)

分工以蓝图为准，开发流程以开发指南为准；不在本页复制规则和实时测试数量。

## 代码和证明

| 目录 | 用途 |
|---|---|
| `controller-core/` | 已有 Rust 纯控制核心 |
| `SandboxModel/` | Lean 状态机、安全性质、处理函数及 Rust 对应证明 |
| `verification/` | 固定版本 Charon/Aeneas 抽取结果、工具链说明和定理依赖审计 |
| `scripts/` | 功能检查、抽取与完整证明验收入口 |
| `tools/` | Lean 交互、轨迹与研究辅助工具 |
| `docs/` | 当前蓝图、合同、测试计划、证明覆盖和相关研究记录 |

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

Lean 是唯一协议规范来源。Charon/Aeneas 将实际 Rust 转为 Lean 函数，再与规范证明对应，而不是另写一份看起来相似的实现。详见[验证工具链](verification/README.zh-CN.md)和[状态机说明](docs/state-machine.md)。

验证结论只覆盖声明的代码、输入、故障模型与前提，不自动覆盖新增运行服务。Linux/KVM、Firecracker/jailer、存储引擎及底层文件/网络原语必须列明合同；不声称证明虚拟化逃逸不存在、完整 POSIX/ACL 或恶意宿主安全。
