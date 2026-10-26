# V3 高级协议与持久收件

原保留历史的版本关系候选见[收件历史校验](../../verification/experiments/stage1-protocol/inbox-retained/README.md)。
本边界直接对应 follows_retained 的初始、相邻和压缩空洞条件；保存的压缩位置和确认
版本不证明删除或确认实际发生。完整 Inbox.validate、原 validate_lifetimes 的循环、
确认事务保持性与真实读取/提交仍各有义务，不能由此局部成功认领持久收件闭合。

[收件寿命校验候选](../../verification/experiments/stage1-protocol/inbox-lifetimes/README.md)
将原 validate_lifetimes 的循环及收尾原样移入私有辅助函数，以接通固定工具；生产采用
与原实现的一般等价仍待完成。值合同覆盖开始时间、首条 Create、转换/加法边界、
每条 applied deadline 及最终值；版本空洞只断开前驱关系，不证明压缩实际发生。
真实 read_lifetime 的同一对象/事务、确认保持性与完整 Inbox.validate 继续另列。

[续期会话校验候选](../../verification/experiments/stage1-protocol/inbox-renewal-session/README.md)
保留完整确认形状、编码及字节上限、首个全文匹配项、压缩边界和未应用续期的检查顺序。
其值合同覆盖任意编码回调；标准 Serde 实例连接、原函数一般等价、生产候选采用和 V0
借用 Vec/Result 标签采用仍待完成。local/previous 的存在、保存的压缩标记及完整字段
相等不证明资源存在、实际删除、授权或耐久接受；完整 Inbox.validate 与持久组合另列。

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实际差距与所有权

[当前生命周期协议](../../sandbox-protocol/src/lifecycle.rs)已表达 Create/Renew/Unload/Resume/Destroy。正式收件、查询与原 SQLite 映射已连接；Resume 已沿原收件、`admit_lifecycle_resume` 与后台 `advance_recorded_lifecycle_resume` 接入。沿 [sandbox-host-service](../../sandbox-host-service/src/lib.rs) 连接 V4/V5；协议接受不等于 VM 动作成功，完整收件与恢复调用对应仍未闭合。

规范化消息至少绑定 deployment、认证中心/固定节点、owner、session、operation key、全文输入、期望版本、源/目标运行代及 capture。外部用户不指定宿主路径/PID、manifest 成功标志或任意命令；当前模板/配置来源仍受原合同约束。

首批证明固定于 `a9c9967889c4cf298e4378c7106948018835fc64`，从已解码且按生成类型表示的值开始：复用原 `ReplayContract`、`ValidationModel`，连接完整请求重放、请求接受与六类首错，以及 Accepted/Recorded 的完整封套关系。协议版本为 4；正常和错误 Rust 使用同一规格，重新抽取和验收后才迁移适用版本。此切片交给 V1 的是值规格和经实际检查的引理，不要求 V1 等待网络与持久收件证明。

独立认证上下文的连接来源由 V7/连接适配提供；V1/V2 提供当前意图与耐久发送，V4/V5 提供本地接受、耐久结果及调用次序，V6 提供文件/VM 的实际事实。全文相等、slot 形状和版本关系都不提供这些前提。Accepted 的首错依次为 center、部署、内层请求首错、retention slot、接受版本；retention window 不要求等于请求 key window。Recorded 只按完整 intent、版本下界和 current 一致性短路，不检查 terminal 真值。

完整 manifest 任意文件集校验、JSON 字节到值的实际接线、持久收件事务、网络外壳及系统组合另行验收。保留仍适用的 carrier 值往返、manifest 正常见证与局部循环证据；它们不证明任意文件集、哈希计算、材料完整或恢复资格。首批不认领这些后续义务完成，具体候选检查状态仍由覆盖记录和本批 PR 记录。

## 2. 状态与义务

| 状态/事件 | 验收义务 |
|---|---|
| 原始字节→结构→合法请求 | 真实 schema/全部字段/版本/限额/错误顺序对应；解析接受不等于业务授权 |
| 收件、重复、冲突 | 在原 SQLite 事务绑定全文与本地身份；同键重放不生成另一份映射，不同参数明确拒绝 |
| 高级意图→本地操作 | unload/resume 使用同一会话、原来源与当前版本；不绕过 V4 检查，不转成裸宿主命令 |
| 宿主结果→中心事实 | Accepted、SnapshotCommitted、Unloaded、Ready、StateLost、终止/释放保持原类别与原身份 |
| 确认、窗口退休 | 只确认已耐久保存的完整结果；关闭窗口仍保留未结责任，旧消息不能影响复用位置 |
| 乱序、重启、终止竞争 | 旧意图不能复活已终止代；有效终止前已发出的动作保持原责任，不从消息丢失推物理不存在 |

中心要求与宿主应用分开允许暂时不一致；模型不强制“收到 HTTP 就已经 applied”。所有字段以实际 Rust 表示关系连接，不能把整个 serde 或内部网络层当业务正确。

## 3. 实现与验收

V0 提供数值/JSON/TLS 结果基础；V1/V2 提供当前授权意图和耐久发送责任；V4/V5 提供本地接受/回执；V6 提供真实资源证据。必须补原 codec、映射函数、事务回调与恢复入口的生成体/绑定。

验收：全部新消息合法往返，字段变异/非法输入/冲突拒绝，重复和乱序回执，收件提交未知，两管理进程独立重启，destroy 与旧 resume 同时到达。保留真实 mTLS/限额检查；模板分发、跨节点发现不扩入本任务。负责 LC-I3/4 的邻接和 LC-P2/3/8/9，不能单独声称系统唯一运行。

## 4. 当前字段合同与证明边界

[真实字段和同步校验](../../sandbox-protocol/src/lifecycle.rs)的协议常量为 4，正式生命周期路由、持久收件及后台已经接线；不维护另一旧协议产品路径。数字格式标记不代表证明模块 V3 已完成。字段校验成功不能作为已接受、已捕获或已恢复的回复。请求、manifest、carrier 已有真实生成体及局部定理；当前源码重新绑定与事务/网络调用对应仍须验收。

| 字段/接口 | 准确表示与检查 | 不能从中推出 |
|---|---|---|
| SessionId | 包装原协议 SandboxId，复用原不透明字符串域，保留全部字节 | 路径安全、实例存在或所有权 |
| IncarnationId、CaptureId、RequestWindowId、RequestSequence | 各自包装受检 PositiveCounter；零拒绝、耗尽不回绕；不改变旧 core 数值类型 | 已分配、唯一认领或业务可重用 |
| OperationKey | deployment、经认证的 owner、window、sequence 全部保存；内部 DTO 可解码不等于调用方已认证 | 窗口已签发、当前授权或持久去重 |
| OperationBinding | 版本、完整 NodeIdentity、session、完整 key、ExpectedSession | 原端点/owner 的选择正确 |
| ExpectedSession | Create 使用显式 Absent；其它操作使用 Version(u64)，不把零改作不存在标记 | 当前账本确实满足期望版本 |
| LifecycleOperation | Create 完整原 CreateBinding 和目标代；Unload 源代和 capture；Resume 源代/capture/严格更大目标代/完整摘要；Renew 原代、绝对结束时间及 extend_only；Destroy 只指逻辑会话 | 任意宿主命令、路径、PID、删除列表或已满足业务寿命政策 |
| replay_matches | 比较整个原请求，包含全部操作参数 | 已查原键、已保存结果或不会再次派发；这些仍由收件事务提供 |

请求的纯校验按版本、完整节点、deployment、期望类别和操作形状依次拒绝。Resume 允许废弃尝试留下的编号空洞，不要求目标恰好加一。原键查找/全文重放必须在当前阶段、窗口和资源检查之前处理；本库不读取账本，不用类型代替这项控制流。

[SnapshotManifest](../../sandbox-protocol/src/lifecycle/manifest.rs)保存原 CaptureBinding、受保护配置指纹和完整文件记录。CaptureBinding 是原已接受捕获的操作/会话/源代/capture；后来的 Resume 具有自己的键，不能覆盖来源 capture 的原键。ContentDigest 是完整 32 字节 SHA-256 值，不与旧 core 的数值 ManifestDigest 转换；构造值不计算哈希或签发验证收据。

文件按 state、memory、设备 profile 的全部可写盘顺序保存。每盘同时匹配原 drive_id，固定内部文件名为 state.bin、memory.bin、disk-N.img；profile 明确登记 drive_id 到原 jail 相对路径，不能仅靠数组位置选择来源。重复 drive/路径、缺少或额外文件、路径不符、零长度及总长度溢出拒绝。实际第一 runtime 的映射为 rootfs → rootfs.ext4 → disk-0.img，恢复复制到新 jail 的私有盘；不允许 VM 写回 bundle。

最多 16 个可写盘是本接口的有限预算，不是 Firecracker 的通用上限或兼容性保证。snapshot_format 和工具/CPU/设备/镜像配置摘要须与原受保护 profile 比较；即使两个任意 profile 相同，也不能据此声明 VMM 兼容。实际文件白名单、原句柄/长度/内容摘要、冻结截面、同步发布及恢复资格分别由 V6/V5 提供，不能让 manifest 自己的字段批准自己。

字节适配位于 [lifecycle](../../sandbox-internal/src/lifecycle.rs)：先检查原始字节预算，再调用固定 serde_json，最后调用上述真实守卫；原 JSON 错误及行列保留。输出复用已有 encode_bounded，不使用普通 to_vec 参考冒充服务器 Writer。该接线仍需本批实际生成体/库条件与正式验收；尚未宣称完整 serde、异步外壳或新端点已证明。实际证据与剩余条件只在覆盖记录和本批 PR 维护。

真实收件路由的字节入口另在 [server/lifecycle](../../sandbox-internal/src/server/lifecycle.rs)：它直接解析
`AcceptedLifecycleIntent`，使用独立连接 context 校验完整封套，并在回复前检查完整 recorded 关系；
它不调用上述独立 request/manifest codec。[隔离接线候选](../../verification/experiments/stage1-protocol/json-wire/README.md)
用于验证这一真实路由所需的最小同步接口，未接纳的生产候选或抽象 parser 回调证明不能算当前字节入口已证。

[协议角色叶子候选](../../verification/experiments/stage1-protocol/schema-leaves/README.md)把 SessionId、
CenterId 和五种计数器角色的实际 Deserialize 接回同一 V0 编号/非零计数器生成体，保留原值、
错误及效果；原 SliceRead 字典绑定条件下的叶子拒绝仍先于文档结束检查。它不重证或迁记旧
协议 2 的全文 schema，不把普通 accepted_version、ExpectedSession::Version 或 slot 加强为非零。
完整派生字段覆盖、具体 parser 调用位置、字节预算及输出 Writer 仍是后续义务。共享根和
独立生成的标准 Result::map 接线以候选 diff 交 V0/主控，未公共采用前不认领正式入口通过。

[完整封套字段表示](../../verification/experiments/stage1-protocol/envelope-schema/README.md)
使用同一实际生成类型及原守卫合同，涵盖五种操作和 Recorded 全部字段。输出表示的
单射不等于整个 Serde parser 已证，也不能把标准对象输出限制为唯一合法输入；固定派生
还接受对应声明顺序的序列。具体字节/字典、容器收尾、原预算和 Writer 接线仍需独立验收。

字段字节组合候选见[封套原回调](../../verification/experiments/stage1-protocol/envelope-bytes/README.md)：
在原 `SliceFieldReader` 上覆盖全部自有字段并明确 Recorded 的 `intent` 路径，复用完整
表示与原守卫；字节往返仍以显式标准派生条件和成功输出为前提。生产 parser 实例、
正常编码完成、字节预算、实际 Writer、持久收件与完整 V3 继续列为待证义务。独立 JSON
抽取视图仅服务本组合，不替代公共 V0/lane 目标或其验收。

[收件记录候选](../../verification/experiments/stage1-protocol/inbox-record/README.md)继续从已解码值
证明保留请求重放/冲突的完整状态保持，以及新会话记录的原首错顺序和完整 Session/Entry
追加。实际 `record` 的新会话组合显式要求没有保留 key、没有相同 session ID；这是容器
条件，不提供授权或持久性。固定工具所需的迭代/单元素向量改写仅在隔离副本验证，生产
采用和公共入口登记仍交主控；原版对照测试不等于机器检验的重构等价证明。
已有会话追加由[独立追加候选](../../verification/experiments/stage1-protocol/inbox-append/README.md)
连接真实 `record_existing` 与 `record`：完整首拒绝顺序、原会话的全部字段更新和整个
Inbox 的保持关系，沿用同一全文查找/封套规格。非 Destroy 的接受版本加一属于前置
封套校验，不能补作该记录函数自己的检查；SDK 门槛只约束新 Renew，Destroy 保留末槽路径。
[统一 record 候选](../../verification/experiments/stage1-protocol/inbox-record-total/README.md)
进一步按原全文查找和首个 session 选择，组合保留重放/冲突、新会话及已有会话的完整
状态合同，对任意已解码输入陈述，不把分支选择作为外部假设。两种追加错误版沿用原
合同的反例接到同一个统一合同。该组合仍是纯内存 record，不覆盖实际事务外层。
这些候选仍需生产采用、公共登记与正式验收；SQLite 提交/未知结果/恢复、GLOBAL→LOCAL
映射及完整 V3 继续开放。
尤其协议 Resume 校验允许编号空洞，而当前收件追加路径另检查 `checked_next`，必须分别
陈述并由对应提供者连接，不能用后者加强公共请求规格。

[原收件查询候选](../../verification/experiments/stage1-protocol/inbox-state/README.md)直接连接
未重构的 `Inbox::state`：两个索引有效时，完整 intent 与其余六个返回字段均对应
选中的 Session/Entry。旧条目是否 current 仅由版本相等决定，local_operation 来自
条目自身。这不表示执行/释放许可或资源存在。record 返回索引的有效性及同一视图
的 record→state 组合另列；持久外层、公共采用与 ning/共享验收仍未完成。

[返回位置与查询组合候选](../../verification/experiments/stage1-protocol/inbox-position/README.md)
进一步在同一生成视图中连接 record/state：从原完整 RecordPost 的三条成功路径推导
实际返回索引有效、该位置保存完整 intent，再得到七字段查询关系；所有原拒绝/状态
关系仍保留。它沿用隔离 record 重构的采用/等价缺口，不覆盖事务外层在两次调用之间
进行的 Destroy 本地效果及 operation 映射，也不说明已持久接受或资源存在。

[原执行谱系校验候选](../../verification/experiments/stage1-protocol/inbox-lineage/README.md)
独立连接未重构的 Session::validate_execution_lineage：任意长度历史逐步检查来源代、
跨代不回退的 capture frontier、相邻 Unload/Resume 关系与最终存储 incarnation。
收到 Resume 只有在本条目的 local_operation 非空时才更新计算代；该字段本身不提供
持久附加或 VM 运行事实。内部 checked_next 与允许编号空洞的公共请求校验分别陈述。
其调用者合同显式保留借用 Vec 迭代入口的最小 V0 接线义务，私有候选只委托固定
Vec.deref/Slice.iter 模型。库采用、原 current_execution_origin 来源选择、完整
Inbox.validate、外层事务/重启与真实资源关系仍另行交对应提供者闭合。

[卸载决议校验候选](../../verification/experiments/stage1-protocol/inbox-unload-resolution/README.md)
从已解码 Entry 开始连接 validate_entry：决议缺席时直接接受；存在时按操作类别、
任意编码计算及长度、快照/捕获元数据、可选结果时间关系依次检查。BeforeAdmission
不检查快照所有者或寿命内容，AfterCapture 的 operation 存在也不表示实际捕获已完成。
编码错误和提前返回保持原顺序；同一完整合同用于字段遗漏错误版。候选仍有生产采用、
原函数一般等价及标准 Serde 实例连接义务，不能据此认领原 Inbox.validate、持久收件、
新的物理 Ready 观察或完整 JSON/网络链已证。
