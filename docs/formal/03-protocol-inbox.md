# V3 内部通信与节点收件：原操作的唯一映射

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：详细证明计划；拟定定理尚待建立。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

通信层证明消息来自正确角色且内容未被改变；持久收件层证明同一高级操作始终指向同一个本地实例。两者连接后才可以处理重复、迟到与重启，TLS 自身不能提供这些业务保证。

## 1 代码边界

| 代码入口 | 认领内容 |
|---|---|
| [内部协议](../../sandbox-protocol/src/internal.rs)、[lifecycle](../../sandbox-protocol/src/internal/lifecycle.rs)、[window](../../sandbox-protocol/src/internal/window.rs) | 全 ControlIdentity、版本、完整参数、事实、确认与关闭编码 |
| [TLS](../../sandbox-internal/src/tls.rs)、[客户端](../../sandbox-internal/src/client.rs)、[服务端](../../sandbox-internal/src/server.rs) | 固定地址/DNS证书名/CA/叶摘要到服务角色映射、HTTP边界、错误/取消 |
| [持久收件](../../controller-service/src/controller/internal_inbox.rs)及 [model](../../controller-service/src/controller/internal_inbox/model.rs)、[confirmation](../../controller-service/src/controller/internal_inbox/confirmation.rs)、[window](../../controller-service/src/controller/internal_inbox/window.rs) | `record_internal_intent/internal_request/internal_pending`，原映射、结果确认、位置窗口/退休 |
| [本地高级入口](../../controller-service/src/application/coordinator/internal_lifecycle.rs) | `record_internal_intent/advance_internal_lifecycle`；这里认领高级到本地请求映射，执行编排归 V5 |
| [节点 facade](../../sandbox-host-service/src/facade.rs)、[driver](../../sandbox-host-service/src/driver.rs) | 真实接收端口接线与 `drive_once/run_driver` 启用原工作；不另写本地算法 |

数据正文转发和正文持有许可归 V8；这里认领控制消息及认证端点，也为 V8 提供“认证角色＋原绑定”合同。模板产物端口本身按 A10 排除，核心使用的 GLOBAL→LOCAL 仅路径转换、原租户映射及准确描述仍要连接。

## 2 状态与步

持久状态是 `Inbox=(nodeIdentity, full requests, global→local map, original results, confirmation, lanes/windows/frontiers, terminal memory)`。易失状态是连接/解析进度/限额/待唤醒；它们丢失不能让已提交映射消失。

证明侧绑定键 `b=(deployment,node/generation,tenant,globalSandbox,retentionSlot,requestWindow)`；操作键另含operation_id与intent_version，操作内容另含kind和全部原参数。定义 `Map(b)=原本地sandbox/operation关联`，不能用地址、窗口、单个operation_id替代全绑定。表示关系连接SQL各行、核心输入尾和前沿到这些对象；同一实例不同高级操作不产生第二个实例映射。

转移关系 `Receive(Inbox,q)=(Inbox',reply,local_calls)`：首次合法q产生原持久责任；重复q返回原进度/结果；冲突/过期拒绝不产生local_calls。解析阶段失败不修改收件状态；事务阶段失败按V5的提交前/后未知语义解释，不能扩大为所有HTTP错误均无副作用。`Confirmed`与`WindowClosed`分别绑定原完整结果和该位置窗口，不用确认一个实例代替整节点退休。

接收按实际顺序进行：验证连接/角色 → 完整有界解码 → 核对部署/节点代际/全参数/窗口 → 同事务保存原意图或重读原结果 → 唤醒原本地流程。接受高级意图与本地资源准入是不同事件，收件成功不等于 VM 已启动。

| 情形 | 必须对应的处理 |
|---|---|
| 同身份同完整参数 | 复用原映射/进度/结果，不再分配本地编号 |
| 同键异参数、旧代际、过期窗口 | 依固定顺序明确拒绝，不改旧映射 |
| 销毁先到或创建/续期迟到 | 保存终止记忆；迟到请求不能复活原绑定 |
| 事实保存而回复丢失 | 下次原请求仍得到原事实或原进度；发送不等于中心确认 |
| 结果确认/关闭窗口 | 核对原完整结果及位置/窗口前沿，按耐久确认和保留合同退休 |
| 接收中/事务前后崩溃 | 未提交不取得本地执行资格；已提交责任由原数据库恢复 |
| 传输超时/客户端取消 | 结果可未知；关闭连接不撤销节点已接受工作，不提前释放未退出任务额度 |

中心与节点窗口依靠共同不变量连接，不要求消息 FIFO、不要求网络 exactly-once；有限连接数不能被无限等待队列绕过。

## 3 拟定定理与下游合同

| 定理 | 义务 |
|---|---|
| `peer_role_binding` | 成功内部入口对应受保护登记角色、原节点/代际及固定端点；同 CA 其他身份不能冒充 |
| `internal_codec_preserves` | 实际编码/解码保留完整身份/参数/事实，大小、版本及错误顺序正确 |
| `inbox_unique_local_mapping` | 同一有效全局绑定最多一个原本地映射；重复/重启不分配替代实例 |
| `inbox_terminal_replay_safe` | 终止先到、旧窗口/版本、异参数、迟到确认均不能改变后继或其他实例 |
| `inbox_accept_durable` | 唤醒/原本地工作启用必须由同一耐久收件及 V5 准入支撑，非HTTP响应码支撑 |
| `fact_retirement_refines` | 原事实在中心耐久确认前不丢；窗口退休保留足够拒绝前沿与原清理责任 |
| `inbox_transport_refines` | 认证、收件事务、端口调用及取消的实际事件对应模型，合法输入不任意拒绝 |

主要性质 S1/S3/S5/S6/S7/S13/S14、F1、DS1/DS3/DS4/DS6/DS7。返回结果给 V2，返回合法本地调用给 V5；其前提通过 V9 连接，不能循环假设 V2 已正确退休。

## 4 批次

首批：固定协议2与 TLS登记；收件/重复/冲突/终止先到及原映射；收件提交后、节点完成后两处丢回复；原事实确认。代表性抽取先检查身份/参数纯验证和映射决定，不一开始抽整个异步 server。

后续：续期语义、保留位置/窗口关闭与独立重启，完整连接预算及角色轮换。复用 [收件回归](../../controller-service/src/controller/internal_inbox/tests.rs) 和 [通信测试目录](../../sandbox-internal/tests)；证书/握手实际检查是 A9/A12 前提核对，不能代替自有绑定定理。

依赖 V0、A9/A12；V5 提供事务与合法本地入口，V1/V2 提供原中心意图/确认，V9 提供共同不变量。具体职责在同一 controller-service 中按函数/状态分界，不能两人各定义一份 inbox。开工/完成沿用 [V0](00-foundations.md#5-开工与完成条件)。

## 精确条款

以下条款为本模块的规范定义；跨模块条款通过前提提供方连接，不将下游自有义务假定为已证。

### B2.1 中心字段与节点映射

中心至少完整建模 [中心状态合同](../module-contracts.md#中心状态与节点责任)的部署/格式、修订、高水位、节点登记、租户/平台预留、owner 与固定节点/代际、不可变模板/形状、原输入、意图/版本、期限、未决操作/outbox、已确认事实、保留结果/窗口和退休责任；内存索引从提交修订派生，不另设权威。

节点持久维护全局绑定到原本地身份、完整高级操作参数/结果、终止围栏及窗口；回执丢失、重启和历史回收不能重新分配映射。节点代际改变不能迁移认领旧实例。

### C10 中心与节点的协议步骤

中心只使用高级生命周期/查询操作，节点连接原本地核心。以下区分实际可并发/失败/崩溃的步骤，不把一整行视为跨库原子事务。

| 步骤 | 必要合同 |
|---|---|
| CenterCommitIntent | 同次保存身份、固定节点、参数、预留/收尾、意图及 outbox；失败/未知不派发候选状态 |
| CenterSend / NetworkDeliver | 仅发送确认耐久的原操作；可重复/延迟/丢失/乱序，重发不新分配 |
| NodeAcceptHighLevel | 核验证书映射、部署/节点代际/租户/绑定/窗口/意图；持久原本地映射；同键异参数拒绝 |
| NodeLocalAdvance | 运行既有本地算法，部分效果/未知保留原身份；传输失败不等于未执行 |
| NodePersistFact / SendFact | 先保存事实及版本，再回报接受/就绪/停止/清理等不同事实 |
| CenterApplyFact | 核对来源、全身份、操作/观察版本和终止状态；耐久保存原结果，迟到事实不回退 |
| CenterConfirmRecorded | 只在中心相应耐久提交后产生接收确认；发送确认不等于已经投递 |
| NodeRetire / CenterRetire | 各自满足责任、保留、窗口与引用合同，不因发送或心跳到期即遗忘 |
| Crash / Restart / CommitUnknown | 保留各自持久状态、真实资源及可能已发生的投递/副作用 |

协议编码、具体端点和截止裕量由[模块合同](../module-contracts.md#中心与宿主的高级操作合同)在实现首批冻结；这些是语义事件，不强制每项新增 HTTP 端点。中心操作、节点高级操作、本地工作/尝试不是同一编号。无外部幂等键的两次公开 create 不自动合并；可靠派发不为任意用户命令/文件写提供 exactly-once。

**公开创建的已知限制**：中心内部 operation_id 只保护同一已接受操作的派发/恢复；用户未收到创建回复时，再发无幂等键的 create 可能得到第二个沙箱。metadata/list 查询不能原子地排除原请求稍后提交，客户端也可能根本收不到服务端生成的编号。因此当前不承诺用户能安全重试并找回唯一原创建。可选公开幂等键及经授权的操作查询是未来候选，须另经 owner 确认，不加入 #292 基本兼容或 #101 首条证明链路的必做接口。[E2B 公开幂等需求 #1658](https://github.com/e2b-dev/E2B/issues/1658)
