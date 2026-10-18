# V2 中心持久协调：Redis、派发与恢复

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：详细证明计划；拟定定理尚待建立。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

负责把 V1 的决定变成可恢复责任，再派发到固定节点。它拥有持久提交点和未知提交处理；与纯决策拆开后，可以单独审查 Redis/Lua/缓存，而不把数据库原子性误用为虚拟机或两库原子性。

## 1 实际入口

| 代码 | 对应责任 |
|---|---|
| [store.rs](../../sandbox-center-store/src/store.rs) | `RedisLedger::initialize/open/read/commit/reaffirm/reconnect`，`Observed/Committed` 类型与写入围栏 |
| [scripts.rs](../../sandbox-center-store/src/scripts.rs) | 实际 READ/CAS 字符串：前置检查、唯一 HSET、原 revision 和完整 payload 比较 |
| [codec.rs](../../sandbox-center-store/src/codec.rs)、[writer.rs](../../sandbox-center-store/src/writer.rs) | 有界权威编码、格式/部署/校验、独占写入租约 |
| [authority.rs](../../sandbox-center-store/src/authority.rs)、[窗口协调](../../sandbox-center-store/src/authority/window.rs) | `CenterAuthority::decide/recover/dispatch_batch/apply_response`、结果确认与窗口关闭 |
| [application.rs](../../sandbox-center-service/src/application.rs) | 原操作 wait/reconcile/退休调用的提交与释放锁顺序；只认领持久编排部分 |
| [业务权威块](../../sandbox-center-store/src/business.rs) | 核心与元数据/模板/构建引用同块的框架、投影及核心责任接口；模板业务算法仍按 A10 排除 |

## 2 状态、原子点与故障

`Jc=(durable block, revision, writer lease, write fence, observed bytes, confirmed cache, dispatch/confirmation jobs)`。权威块保存完整 `Cc` 与合同允许的外围数据；缓存必须可由已确认修订派生。

| 微步骤 | 可承诺什么 | 故障后责任 |
|---|---|---|
| 读取/解码原块 | 取得 `Observed`，不是耐久派发证书 | 错格式、部署、TTL、旧注册或超包络则拒绝；不初始化覆盖旧数据 |
| 计算 V1 决定/编码 | 本地候选；此时不能发执行消息 | 崩溃丢易失候选，原块继续权威 |
| CAS 与健康/耐久确认 | 原修订及完整字节相符时单块更新；确认后才构造 `Committed` | 错误/丢回复可能已写，围栏派发及后续写入，保留原责任 |
| 恢复/reaffirm | 读原责任、恢复健康，按完整原 payload 提交新修订并确认 | 只读到原记录不能解除未知耐久；不能产生替代操作 |
| 派发、收结果、确认 | 派发原提交身份；保存原结果后才发确认 | 网络丢回复由原 pending 恢复；不能靠 wake-up 取代持久 outbox |
| 压缩/闭窗 | 依赖节点耐久确认和完整回执 | 未确认不退休；同一位置保留窗口前沿 |

当前 Redis 是受保护同机回环端点，版本/配置与租约条件见 [A11](dependencies.md#redis-与-sqlite-的落地合同)。单写者不等于任意错误都未生效；不加入远程 Redis、自动切换或多中心共识。

## 3 表示关系和拟定定理

`R_store(Jc,Cc) := Decode(block.payload)=Cc ∧ format/deployment/revision 一致 ∧ cache 对应已确认提交 ∧ 未决责任可恢复`。修订和 payload 的完整相等必须保留，不能只比较摘要或计数。

| 拟定定理 | 义务 |
|---|---|
| `center_codec_roundtrip_bounded` | 合法完整块编码/解码保真；拒绝损坏、重复/未知字段及超限符合实际规则；未来结果包络足够 |
| `lua_read_refines`、`lua_cas_refines` | 实际脚本文本在所用 Lua/Redis 原语语义下，检查顺序、唯一写点、错误返回及原字节 CAS 对应模型 |
| `center_commit_before_dispatch` | 每个 Dispatch 有此前同一身份/参数的耐久责任；仅 Observed、失败提交或旧缓存不能派发 |
| `unknown_commit_preserves_liability` | 可恢复“写前/写后未知”集合；健康读取不伪造耐久；reaffirm 后责任仍同一，不改绑 |
| `center_recovery_refines` | 独立中心/Redis重启重建某合法提交，原窗口/高水位/未确认结果与预留不丢 |
| `center_fact_ack_order` | 原节点事实耐久进入权威后才确认；确认/退休不丢未决责任 |
| `authority_microsteps_refine` | 在登记的 A/B 合同下，实际 store/网络调用、同步回调业务、锁保护范围和错误/取消解释的事件历史对应上述转换；薄机械接线不称为已机器证明 |

Lua 不能由 Aeneas 对 Rust 字符串的抽取代替证明。首批先固定 READ/CAS 的受限语法与 AST 对应检查、所用 Redis 命令合同，然后证明逐语句语义到 CAS 模型。解析器/转录/语义连接的可靠性须登记；若无法建立，就报告工具/对应缺口，不能手写一个正确 CAS 函数后把它当实际脚本已证。Redis 引擎原子执行与耐久合同可作 A11，**自有 Lua 仍是待证对象**。

主要性质 S6/S8/S12/S13/S14、DS3/DS5/DS7；F1及局部进展。容量包含权威块、JSON包络、旧/新块峰值、AOF/重写和在途工作，实际峰值由 V9 连接。

## 4 可立即建立的任务

首批：codec+READ/CAS桥接可行性 → 原创建决定 commit/dispatch → VM已运行但回复丢失、中心重启/reaffirm → 原终局保存/确认。最小反例：只有读成功就派发；CAS回复丢失后创建新身份；修改原payload却用相同revision提交。

扩展：续期后查询新鲜视图；逐位置确认/关闭/恢复；有界编排及错误故障域。复用 [Redis组件测试](../../sandbox-center-store/tests/unit_contract.rs) 与 [双服务故障入口](../../tests/system/center_host_faults.py)，测试不充当 Lua/异步精化证明。

入口依赖 V0/V1，消息端点依赖 V3，授权/查询调用接 V7，跨层责任交 V9。中心决定锁不能跨远程 I/O 持有；实际 wait/reconcile 的出队、重投、锁保护范围和释放时点的业务顺序必须证明，不把“内部最终正确运行”列为外部假设。原锁互斥与获取/释放、任务/回调机械交付仅按[依赖登记](dependencies.md#逐符号可信适配登记)的 B1–B4 范围可信，采用源码审查及合同测试核对，不以完整异步翻译为前置。开工/验收沿用 [V0](00-foundations.md#5-开工与完成条件)。
