# Sandproof

有状态沙箱生命周期管理与验证：在管理服务崩溃、请求重试和并发操作下，实现可靠的暂停卸载、同宿主恢复及资源回收。

第一阶段保留真实中心→宿主链路，一个有效中心写者和 Redis、一个固定 Linux/KVM 宿主及 SQLite，支持多用户、多会话。核心问题是暂停时交接最新状态、恢复时取得唯一执行资格，以及恢复材料何时可安全删除。范围、故障假设和论文目标见[项目蓝图](docs/project-blueprint.md)。

`stage1` 从 integration `56b14d64` 建立。本次是设计与任务调整：既有中心/宿主、冷启动/清理、网关和公共证明可以按原范围复用；真实 Full snapshot、先暂停加载和材料持久引用尚未完成。已有 Pause/Resume 类型不等于实际可恢复，准确结论见[证明覆盖](docs/proof-coverage.md)。

固定 E2B 核心兼容、模板链和既有 CI 继续保留。第一阶段不证明完整云平台、所有来宾协议、多宿主迁移或任意 VMM 故障后的状态恢复。新增生命周期操作需要明确操作键；旧无键 SDK create 不自动获得安全重试语义。

## 开发入口

- [总计划与任务](https://github.com/ningZhang-cs/sandproof/issues/1)、[阶段里程碑](https://github.com/ningZhang-cs/sandproof/milestones)
- [开发指南](CONTRIBUTING.md)：分支、提交、issue、PR、测试和文档维护规则的统一入口
- [文档目录](docs/README.md)：蓝图、合同、验收清单与模型／证明说明的总分索引
- [系统架构](docs/ARCHITECTURE.md)：统一入口、组件责任、运行路径、存储与 E2B 对照
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
