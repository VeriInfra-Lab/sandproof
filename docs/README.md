# Sandproof 文档目录

[项目入口](../README.md) · [开发、分支与测试规则](../CONTRIBUTING.md)

只维护下列长期主题；同一规则不在多篇文档重复定义。任务状态在 GitHub issue/里程碑，旧报告和旧方案在 Git 历史，实验产物在忽略的 artifacts。

| 层次 | 文档 | 唯一负责的内容 | 维护者 |
|---|---|---|---|
| 总体 | [项目蓝图](project-blueprint.md) | 做什么、不做什么、功能编号、分工和里程碑 | A/B/C |
| 总体 | [系统架构](ARCHITECTURE.md) | 组件关系、运行路径、存储用途和 E2B 架构对应 | A/B/C |
| 工程 | [模块合同](module-contracts.md) | 请求、执行、回执、持久化与失败语义 | 接口两端 |
| 工程 | [系统验收清单](testing-plan.md) | 本地系统场景、分布式组件/双服务故障场景、兼容与评测判据 | C 汇总，A/B 负责各自场景 |
| 设计总览 | [形式化验证设计](formal-verification-design.md) | 最终目标与边界、V0–V9责任、性质索引和推进顺序 | A/B/C |
| 规范 | [状态机说明](state-machine.md) | 既有本地模型的状态、步骤、安全性质与抽象边界 | A/B |
| 证明 | [覆盖与剩余义务](proof-coverage.md) | 哪些真实输入已对应、前提和未完成证明 | 改动对应模块的人 |
| 证明 | [证明操作流程](proof-workflow.md) | 目标读取、隔离试验和完整验收的专门做法 | 证明开发者 |
| 工程 | [CI 运行环境](ci.md) | 普通隔离执行环境、通用宿主要求及私有运维配置 | CI 维护者 |
| 评测工具 | [Terminal-Bench 评测](../benchmarks/README.md) | 2.0/2.1 的独立版本锁与工具入口；验收判据和两题晋升范围链接至测试清单 | C |

证明工具链的专门细节见 [verification 说明](../verification/README.zh-CN.md)；固定外部源码出处见[证据索引](../references/code-evidence.md)。它们是叶子资料，不是第二份项目计划。

## 证明详细计划

下列页面属于验证设计，不是独立产品规格或进度表。精确状态、规则、定理和前提只在所属页面维护；实际认领与进度见验证总览链接的新 V0–V9，已证明范围见覆盖记录。

| 责任 | 详细计划 |
|---|---|
| 公共定义与外部条件 | [V0 公共基础](formal/00-foundations.md) · [依赖与可信链登记](formal/dependencies.md) |
| 中心控制线 | [V1 中心纯决策](formal/01-center-core.md) · [V2 Redis持久协调](formal/02-center-store.md) |
| 宿主本地控制线 | [V3 协议与持久收件](formal/03-protocol-inbox.md) · [V4 本地纯控制](formal/04-local-core.md) · [V5 SQLite与编排](formal/05-local-service.md) |
| 宿主执行线 | [V6 真实资源、围栏与清理](formal/06-host-execution.md) |
| 公开接口与数据线 | [V7 鉴权、参数与查询](formal/07-public-api.md) · [V8 两端网关](formal/08-data-gateway.md) |
| 系统连接 | [V9 生命周期组合与条件完成性](formal/09-system-composition.md) |

阅读顺序：新成员先读 README、开发指南和蓝图，再用架构文档了解组件如何协作；做接口再读合同；做验证先读验证总览、V0与依赖登记，再读所属模块详细计划、覆盖记录和证明流程。本地状态机用于核对历史模型，不代替两层系统计划。普通功能开发不需要先通读全部研究材料。
