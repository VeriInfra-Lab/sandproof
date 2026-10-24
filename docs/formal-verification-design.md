# 形式化验证设计总览

[文档目录](README.md) · [项目蓝图](project-blueprint.md) · [模块合同](module-contracts.md) · [证明覆盖](proof-coverage.md) · [证明工作流](proof-workflow.md)

当前目标是第一阶段有状态沙箱生命周期。代码定位基线为 integration `56b14d642c392316043dfbdbe4d26bacc663fb30`；设计分支 `stage1`。本页描述待实现和待证明的目标，不能把文档调整记为定理完成。已交付组件仅按覆盖记录复用。

## 1. 目标与故障范围

证明真实生命周期实现的允许历史符合合同：创建、暂停并卸载、同宿主恢复、销毁和操作查询；多会话、至少两个用户，一个有效中心写者、单权威 Redis、单固定 Linux/KVM 宿主及 SQLite。正常请求必须有成功路径，同时允许资源不足等合法拒绝。

中心/宿主服务可能在每个允许微步骤之间崩溃，VMM 与文件可能存活。消息可延迟、重复、丢失；提交和外部效果可未知；错误返回可能已有部分效果。将访问撤销、运行代切换、snapshot/mmap 引用和精确清理纳入同一历史。进程崩溃不是自动 Drop，客户端取消不是后台任务结束。

不承诺任意 VMM 死亡后的 RAM 保存、断电/介质损坏、恶意宿主、历史存储回滚、多中心、多宿主迁移。完整 get/list、全部数据协议、两执行保密、整个平台长期容量及 T9 浮点兼容算法后置。未纳入功能必须在本阶段配置不可达，或有不干扰本阶段状态/预算的证明；不能只在文档中宣布排除。

## 2. 最终性质

以下 LC 前缀区别于历史 G1/G2 类型工作与 P/C 缺口编号。

| 性质 | 可观察含义 |
|---|---|
| LC-G1 状态连续 | 成功恢复延续指定完整捕获；普通暂停失败不丢原唯一最新状态；可能执行后不重载旧捕获，不用冷启动冒充恢复 |
| LC-G2 唯一执行 | 同一会话不会有两个获准运行者；恢复来源只产生一个可能执行的后继；旧命令和访问不能取得新代权限 |
| LC-G3 如实卸载与复用 | Unloaded、Cleaned、Released 分别有对应物理依据；只有旧资源与迟到工作不能影响新占用者时才复用 |
| LC-G4 故障/重试安全 | 每个可能外部效果都有持久责任；全文重放、并发和独立重启不重复分配、不误认当前事实、不误删 |
| LC-G5 合法请求可接受 | 支持域内条件充分的请求通过检查并进入规定路径；不能靠全部拒绝获得安全 |
| LC-G6 条件最终完成 | 外部可用性与声明调度条件成立时，已接受操作及必要收尾最终结算；内部工作启用/预留/驱动不作环境假设 |

LC-G1 是指定捕获边界下的内存/设备/磁盘连续性，不是任意外部网络副作用的恰好一次。LC-G2 是本协议和实际控制动作的结论，不是“有效记录只有一条”就自动成立。

## 3. 按真实模块划分证明责任

| 模块 / issue | 实现归属 | 详细计划 |
|---|---|---|
| V0 / [#632](https://github.com/VeriInfra-Lab/sandproof/issues/632) | 共享类型、实际公共调用与依赖/适配接口 | [基础及 T0–T9](formal/00-foundations.md) |
| V1 / [#633](https://github.com/VeriInfra-Lab/sandproof/issues/633) | sandbox-center-core | [中心纯决定](formal/01-center-core.md) |
| V2 / [#634](https://github.com/VeriInfra-Lab/sandproof/issues/634) | sandbox-center-store | [Redis 与持久协调](formal/02-center-store.md) |
| V3 / [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635) | sandbox-protocol/internal、host-service 收件 | [协议与持久收件](formal/03-protocol-inbox.md) |
| V4 / [#636](https://github.com/VeriInfra-Lab/sandproof/issues/636) | controller-core | [本地状态与转移](formal/04-local-core.md) |
| V5 / [#637](https://github.com/VeriInfra-Lab/sandproof/issues/637) | controller-service、SQLite 与快照登记 | [本地持久编排](formal/05-local-service.md) |
| V6 / [#638](https://github.com/VeriInfra-Lab/sandproof/issues/638) | sandbox-node、sandbox-runtime | [真实捕获、执行与回收](formal/06-host-execution.md) |
| V7 / [#639](https://github.com/VeriInfra-Lab/sandproof/issues/639) | sandbox-api、center-service 公开管理入口 | [请求、鉴权与结果](formal/07-public-api.md) |
| V8 / [#640](https://github.com/VeriInfra-Lab/sandproof/issues/640) | 两端网关、data-transport、必要 guest 连接 | [撤销、排空和恢复授权](formal/08-data-gateway.md) |
| V9 / [#641](https://github.com/VeriInfra-Lab/sandproof/issues/641) | 服务装配、实际驱动与全历史 | [系统组合和研究验收](formal/09-system-composition.md) |

不另立 T 或 stage 工作包；V0 仅补上述真实调用需要的公共能力。模块详细计划是唯一精确证明义务来源，issue 记录验收和推进，不另复制规范。业务跨 crate 时按状态所有者分工，不引入第二份业务实现。

## 4. Rust 与模型的连接

1. 对可抽取的真实 Rust 同步决定、编码/校验/参数绑定，沿固定 Rust→Charon→Aeneas→Lean；证明生成函数与所属模型的输入、状态、结果及事件对应。
2. Tokio、锁/任务驱动、数据库引擎、TLS、Linux/文件系统与 Firecracker 使用逐接口条件合同；仅已登记机械适配属于可信代码。async 业务回调、恢复判断、SQL/Lua 条件、manifest 校验、准入和 GC 决定仍是自有证明义务。
3. 对当前不能抽取的自有业务，做保持行为的小拆分/重构，记录拆分对应；不能整函数变公理。本阶段不扩展 Charon/Aeneas，不建设独立 Rust 解释器。
4. V9 组合真实调用与表示关系，允许内部步骤和暂时的持久/物理差异。Aeneas 翻译器本身及边界条件仍受信任；Lean 内核检查的是列明前提下的定理，不是无条件证明整个 Rust/Tokio/Linux。

每项结果区分模型定理、真实生成体对应、机械接线审查、原生测试和实机证据；未完成前提有明确提供方。依赖的精确保证和使用条件只在[依赖登记](formal/dependencies.md)维护，不用“边界可信”代替合同。

## 5. 收敛与验收

先固定 V0 最小身份/事件/边界与 V9 系统不变量；三个切片按暂停交接→唯一恢复→材料回收推进，同时接公开入口和实际持久驱动。正常无故障路径先用来发现合同不可实现处，随后证明全部允许分支/交错，并接实机故障矩阵；不把轨迹测试当全称证明。

保留已有 create/丢回执/销毁首链作为基础，扩展最终闭环为 create→运行态修改→unload→管理服务重启→resume→再次修改→destroy→材料与位置结算。LC-P1–14 中间义务及 LC-I1–10 实现连接在[V9](formal/09-system-composition.md)登记。证明门槛、精确候选与复用判据遵循[工作流](proof-workflow.md)，不能只刷新摘要绕过失败。

科研完成要求见蓝图；具体实验及负例在验收清单。本次文档/issue 重排不宣称新生命周期性质已证明，也不使旧工具验收自动适用于新状态机。
