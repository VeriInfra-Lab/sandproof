# Sandproof

单节点、E2B 核心兼容的沙箱控制系统，目标是对安全关键的 Rust 实现进行形式化验证。

当前先完成 V1 功能和真实测试，再推进完整生命周期与证明。**这仍是开发中的项目，不是已经完整验证、可直接部署的沙箱服务。** 当前范围以[项目蓝图](docs/project-blueprint.md)和[总计划 #1](https://github.com/ningZhang-cs/sandproof/issues/1)为准。

## 我们要做什么

在单台 Linux/KVM 主机上，使用自己的 Rust 控制器与执行层管理 Firecracker/jailer，支持多用户和多个沙箱。在固定版本、明确功能子集内兼容 E2B 客户端：

- 创建、连接运行中沙箱、查询、寿命续期、到期销毁和主动销毁。
- 重复命令执行、后台任务、输出，以及运行期间的文件读写与传输。
- 私有可写系统盘、最小模板、鉴权、资源限额和受控网络。
- 持久记录、提交后派发、未知结果处理、精确资源身份和重启恢复。

暂停恢复在后续阶段实现；多节点、共享卷、完整云平台复制和自动部署不在 V1。兼容性与 Terminal-Bench 评测不能替代安全证明。

## 现在有什么

| 部分 | 当前状态 |
|---|---|
| Rust 控制核心 | 已有纯状态处理实现及 78 项单元测试，不包含真实存储、HTTP 服务或 VM 操作 |
| Lean 规范与对应证明 | 已有协议安全证明和当前 Rust 公开输入的对应证明；有限执行安全性带明确容量前提 |
| 最终完成性与长期容量 | 仍有未闭合义务，见[证明覆盖记录](docs/proof-coverage.md) |
| E2B 兼容服务与运行层 | 待按 V1 工作包实现和实机验收；旧小沙箱原型已从当前版本移除，不作为可用运行层 |
| CI | Rust 格式、静态检查、测试及 PR 标题检查；不包含 KVM、E2B 对照或完整证明验收 |

已移除的原型可从 [Git 历史](https://github.com/ningZhang-cs/sandproof/tree/0f1901a210e24da50d259e07df75e8c2cb9d181f/sandbox-runtime)查看。当前目录不再保留旧方案的副本，也不把删除旧模块计算为新功能完成。

## 开发入口

- [总计划与任务](https://github.com/ningZhang-cs/sandproof/issues/1)、[阶段里程碑](https://github.com/ningZhang-cs/sandproof/milestones)
- [项目蓝图与三人分工](docs/project-blueprint.md)、[模块合同](docs/module-contracts.md)
- [提交、issue 与 PR 约定](CONTRIBUTING.md)、[测试与验收计划](docs/testing-plan.md)
- [CI 运行记录](https://github.com/ningZhang-cs/sandproof/actions)

A 负责控制、持久化和恢复；B 负责节点执行与隔离；C 负责接口兼容和测试框架。每位开发者负责自己模块的测试。当前不强制另一位成员批准 PR，但自动检查仍是合并门槛。

## 代码和证明

| 目录 | 用途 |
|---|---|
| `controller-core/` | 已有 Rust 纯控制核心 |
| `SandboxModel/` | Lean 状态机、安全性质、处理函数及 Rust 对应证明 |
| `verification/` | 固定版本 Charon/Aeneas 抽取结果、工具链说明和定理依赖审计 |
| `scripts/` | 功能检查、抽取与完整证明验收入口 |
| `tools/` | Lean 交互、轨迹与研究辅助工具 |
| `docs/` | 当前蓝图、合同、测试计划、证明覆盖和相关研究记录 |

项目品牌改名不改动 Rust crate 名、Lean 命名空间或抽取符号，避免引入与本次清理无关的证明变化。新增服务模块在各自工作包中建立，不先放空实现冒充能力。

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
