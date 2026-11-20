# Host24 同步调用方切片的合同与交接

本开发批次的抽取源码固定为 `1053c1e5d345efd998da529b9984f1361920a8ed`，父提交为
`f3627b7bb9995251abe31e2bd2bf6252daa1f2c8`。后续仅同步已验证的手写证明字面值、说明及未运行原生断言，四个生产文件不变。
本候选保留原协议4与Host存储24；
它尚未合入，不据开发证明认领原生、正式或共享验收。生产的四文件拆分、两行
StandardInboxSave可见性修正和旧提案遗漏均见候选README及#635交接评论。

## 实际入口与表示关系

生产 `ControllerJournal::record_lifecycle_intent` 在原WorldTransaction内创建
StandardRecord，调用唯一的 `recording::record_using`。证明视图只追加透明根，
最后一根 `recording::proof_protocol_record_using` 直接转发此函数；其余14根复用
现有请求、封套、record/load/save/validate/state入口。真实Rust业务正文不在
正确抽取副本中替换，原始生成Types/Funs不手工修改。

Aeneas值包含完整AcceptedLifecycleIntent、AuthenticatedCenterContext、Inbox、
RecordOutcome和LifecycleWorkState。结果中的完整intent、current_session_version、
current、terminal、refusal、local、local_operation都保留。效果接口上的可变E状态
表示为返回元组中的后态；Result为原ITree效果语义，错误、失败和发散均不假定消失。
正常Trace只是这一通用E参数的一个实例，有限见证不替代通用定理。

FullFlowContract的run对任意类型/编码提供者、意图、独立context和效果状态量化，
以原FullLoad和FullSave规格组合已证明的认证、全文查找、完整record与state函数。
此处直接复用这些实际值函数；它们各自的独立值合同由下表定理给出。没有另造一份
协议、授权或收件模型，也不以整个handler或WorldTransaction正确作为前提。

调用次序是load → 独立context/封套检查 → 全文查找。命中后返回Replay；新操作才
继续释放中守卫 → 完整record → Destroy附加 → save → state。不同阶段的首错
必须按这个顺序理解；纯request.validate的首错顺序并不被推广成load之前的守卫。

## 已检查的主要定理

下列均采用前缀 `SandboxModel.Stage1Protocol.`，已在当前正确200文件/799审计
完整结果中出现；本表没有把未完成的错误版全批检查标为通过。

| 定理 | 准确保证 |
|---|---|
| ProtocolRecord.full_flow_contract | 实际通用调用方与完整flowView对任意输入及效果状态对应 |
| InboxLoad.full_load_refines | 同一返回Inbox的读取、摘要值、重编码、预算、完整validate和两次独立配置读取 |
| InboxAdmission.authenticated_contract | 原配置的完整center/node与封套值检查；不证明context来自已认证连接 |
| InboxLookup.find_contract | 原首次会话/键查找及全文重放、冲突；不要求历史预先去重 |
| InboxRecordTotal.record_contract | 实际record的完整结果及后态；不保证后态必然通过save的完整校验 |
| InboxSave.save_refines | 同一Inbox按validate、encode、容量、digest、UPDATE、行数分类；保留已发生效果 |
| InboxState.state_contract | 有效位置上的完整返回状态和原intent |
| ProtocolRecord.guard_refines | 原释放中查询在追加前执行；普通非Destroy分支不调用它 |
| ProtocolRecord.attach_refines_total | 任意位置/操作的实际Destroy附加行为，含失败和越界 |
| ProtocolRecord.attach_view_config | 实际附加步骤保持原config，连接既有表示条件 |

完整流模型SHA为 `fe63edcb5074ba6e0c9715477c5e2481c7480cdff2f49bd79538d4be0a4cc2f0`；
Host24完整load模型SHA为 `c9618071efa68abf4da7fa00e146427757dd3215561822f6fafce2e17fddee8a`。
前者保持原字节内容；后者只按实际23→24迁移原版本分支。正常读取fixture另有两处
手写版本字面值修正，原失败、成功试验及精确diff保留。没有修改任何生成业务体。

通用前提只有原FullGraphicContract和DecodedStringsRepresentable：后者约束实际
decode_inbox成功返回的config字符串表示域，不预设整个Inbox已合法。其余数值、
String/Vec/Option/字典、编码等库条件沿dependencies.md逐符号合同。没有新增公理，
没有将retention window与key window强行相等，也没有把Resume目标必须加一加入
公共请求合同。旧版本且收件表/SourceWork表均缺席返回None的helper边界保留。

## 正反见证与原生连接

正常Create及完整Destroy见证复用同一模型。Destroy见证中binding.operation为30，
独立查询返回32，实际返回的完整状态记录32。它证明选择查询返回值，不证明真实
操作分配器、VM退出或持久提交。三个错误版分别删除独立context检查、让Replay
继续执行、写回旧operation；它们都否定同一个FullFlowContract，不更换性质或假设
save必然成功。变异已在当前源码真实编译/抽取；完整物理检查的终态另见原始结果。

错误视图排除正确调用方Record/Values/DestroyActualValues定理；旧operation视图
额外排除正确Attach/AttachTotal/AttachConfig定理。正确load/record/save等未被变异的
值函数可复用。首轮缺少依赖产物及预构建入口错误只作为准备失败保留，不计性质拒绝。

三条真实原生断言候选使用原ControllerJournal及标准JSON/SHA/SQL：异节点context
拒绝并保持完整原始收件行/World；Replay返回同一完整State且不进入提交故障点；
Destroy返回/查询的operation等于真实World新当前值并区别旧binding值。候选未在
本机执行，交ning按原正确/错误/恢复、完整失败集合与fresh库/二进制规则验收。
它们不替换既有load的19项候选断言、12项原始/恢复断言或共享原生套件。

## 标准实例与未闭合提供者

| 实际使用点 | 当前连接与仍需落实的事实 | 提供方 |
|---|---|---|
| StandardRecord读配置和收件 | 逐方法转发原StandardConfigRead及同一连接；具体SQL列/参数、JSON字节到值、SHA计算和表示关系须逐符号实例化 | V3接线；V0具体库合同 |
| StandardRecord保存 | 复用原StandardInboxSave的encode/digest/update正文；32字节值相等及行数结果不自动证明材料完整或耐久 | V3接线；V0库合同；V5持久事实 |
| releasing(local) | 原World维护slots中sandbox相等且phase=Releasing的查询；不能擅加原代码没有检查的owner/代比较，也不能从bool推出物理退出 | V3字段连接；V4/V5 World/slot关系 |
| terminate_local(local) | 原同一tx和namespace调用internal_inbox::terminate_local；返回成功只沿实际值结果继续 | V3调用顺序；V4/V5本地业务；V6物理事实 |
| current_operation(sandbox) | 原World同一sandbox的当前操作号；缺失CorruptStore，不能写旧local.operation | V3选择/保存；V4/V5操作与World事实 |
| WorldTransaction begin/commit/恢复 | 原调用方保留提交故障点，Replay提前返回；同一快照、原子保存、未知提交与重启尚未由本切片证明 | 后续V3与V4/V5事务提供者 |
| 独立context来源、当前意图 | 值相等和校验成功不签发授权或耐久发送事实 | V7、V1/V2 |
| 材料释放与SourceWork | 只连接本切片确实读取的结果；不假设整套GC正确 | V4/V5/V6 |

以上是逐使用点的待连接关系，没有增加一个名为“标准实例正确”或“整个事务正确”
的业务公理。源码转发审查、Rust类型检查、原生断言及机器定理属于不同证据层次；
当前standard_provider_connection_proved和original_equivalence_proved仍为false。

## 准确交付状态

正确原始结果SHA为 `303973e47883b4189ff33cdc978b575fadff73313675ade5de3fc67f755b151a`；
完整回读SHA为 `f0b6951f5d05c3747619a1e2712b0e32872e4cd522ba33e3bf1cf646b2b9cdb3`。
独立load正确/错误185/751与108/424已封包；当前三个调用方错误版均已完整通过并逐项回读：context为192/755、
Replay为194/764、旧operation为193/782，均使用同一完整合同。公共登记、根、冻结、覆盖与正式汇总由验收窗口统一维护。

历史ManifestValidation已有表示条件下任意manifest/profile的完整值合同及四组真实
反例证据，原服务/摘要已回读且相关源/证明未变。保留原适用版本，现版完整准入及
物理文件/JSON/TLS事实不能由此推出。完整V3仍须满足#635全部验收点；本说明只交接
本切片，不关闭字节、持久收件、未知提交、跨模块或整套验收义务。
