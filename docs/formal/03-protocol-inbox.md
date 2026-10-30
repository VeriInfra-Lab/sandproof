# V3 高级协议与持久收件

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实际差距与所有权

[当前生命周期协议](../../sandbox-protocol/src/lifecycle.rs)已表达 Create/Renew/Unload/Resume/Destroy。正式收件、查询与原 SQLite 映射已连接；Resume 已沿原收件、`admit_lifecycle_resume` 与后台 `advance_recorded_lifecycle_resume` 接入。沿 [sandbox-host-service](../../sandbox-host-service/src/lib.rs) 连接 V4/V5；协议接受不等于 VM 动作成功，完整收件与恢复调用对应仍未闭合。

规范化消息至少绑定 deployment、认证中心/固定节点、owner、session、operation key、全文输入、期望版本、源/目标运行代及 capture。外部用户不指定宿主路径/PID、manifest 成功标志或任意命令；当前模板/配置来源仍受原合同约束。

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
| LifecycleOperation | Create 完整原 CreateBinding 和目标代；Unload 源代和 capture；Resume 源代/capture/严格更大目标代/完整摘要；Destroy 只指逻辑会话 | 任意宿主命令、路径、PID 或删除列表 |
| replay_matches | 比较整个原请求，包含全部操作参数 | 已查原键、已保存结果或不会再次派发；这些仍由收件事务提供 |

请求的纯校验按版本、完整节点、deployment、期望类别和操作形状依次拒绝。Resume 允许废弃尝试留下的编号空洞，不要求目标恰好加一。原键查找/全文重放必须在当前阶段、窗口和资源检查之前处理；本库不读取账本，不用类型代替这项控制流。

[SnapshotManifest](../../sandbox-protocol/src/lifecycle/manifest.rs)保存原 CaptureBinding、受保护配置指纹和完整文件记录。CaptureBinding 是原已接受捕获的操作/会话/源代/capture；后来的 Resume 具有自己的键，不能覆盖来源 capture 的原键。ContentDigest 是完整 32 字节 SHA-256 值，不与旧 core 的数值 ManifestDigest 转换；构造值不计算哈希或签发验证收据。

文件按 state、memory、设备 profile 的全部可写盘顺序保存。每盘同时匹配原 drive_id，固定内部文件名为 state.bin、memory.bin、disk-N.img；profile 明确登记 drive_id 到原 jail 相对路径，不能仅靠数组位置选择来源。重复 drive/路径、缺少或额外文件、路径不符、零长度及总长度溢出拒绝。实际第一 runtime 的映射为 rootfs → rootfs.ext4 → disk-0.img，恢复复制到新 jail 的私有盘；不允许 VM 写回 bundle。

最多 16 个可写盘是本接口的有限预算，不是 Firecracker 的通用上限或兼容性保证。snapshot_format 和工具/CPU/设备/镜像配置摘要须与原受保护 profile 比较；即使两个任意 profile 相同，也不能据此声明 VMM 兼容。实际文件白名单、原句柄/长度/内容摘要、冻结截面、同步发布及恢复资格分别由 V6/V5 提供，不能让 manifest 自己的字段批准自己。

字节适配位于 [lifecycle](../../sandbox-internal/src/lifecycle.rs)：先检查原始字节预算，再调用固定 serde_json，最后调用上述真实守卫；原 JSON 错误及行列保留。输出复用已有 encode_bounded，不使用普通 to_vec 参考冒充服务器 Writer。该接线仍需本批实际生成体/库条件与正式验收；尚未宣称完整 serde、异步外壳或新端点已证明。实际证据与剩余条件只在覆盖记录和本批 PR 维护。

## 5. 同步核心与证明入口的连接

正式请求处理只维护一条实现路径：有界读体后调用 `decode_accepted_with`，传入原固定解析器，
再按原顺序校验完整封套和独立上下文；解析失败仍为 400，封套拒绝仍为 409。
收件查找、首个会话选择、顺序拒绝与完整追加使用原同步核心；内层查找拆为具名 helper，
保持首次命中和首次冲突语义，不要求键或会话列表预先唯一。

寿命检查将原扫描尾部移入 `validate_applied_lifetimes`；续期确认与卸载决议把编码调用
通过窄接口传入，生产实例继续使用原 `serde_json::to_vec`。不改变首错、续期确认的
8192 字节限额、卸载决议的 4096 字节限额、
快照阶段、原本地操作或结果时间链条件。证明薄根只转发这些实际函数，正确视图不得
在私有副本中再次替换业务算法；错误视图才局部修改被检验的真实表达式。

[Resume 完整值合同](../../verification/experiments/stage1-protocol/resume-observation/README.md)
包含 Continued-Unload 的 StateLost，但同一实例确定退出、原继续资格、材料和授权事实
仍由 V4/V5/V6/V7 提供。原“允许该 StateLost”错误版随旧合同保留在历史中，当前反例
检查绕过实际仍必需的原来源绑定。

[完整收件合同](../../verification/experiments/stage1-protocol/inbox-record-total/README.md)、
[查询组合](../../verification/experiments/stage1-protocol/inbox-position/README.md)、
[寿命](../../verification/experiments/stage1-protocol/inbox-lifetimes/README.md)、
[续期确认](../../verification/experiments/stage1-protocol/inbox-renewal-session/README.md)和
[卸载决议](../../verification/experiments/stage1-protocol/inbox-unload-resolution/README.md)
分别给出值规格和条件。库模型采用、实际编码实例、同事务接线和完整持久收件的
未闭合义务必须单列；新增候选、局部通过和当前实现已验收的范围只在覆盖记录维护。

这些同步合同从调用时的完整值开始，不要求列表预先去重或历史已经可达。它们证明
函数怎样选择、检查和返回；V5 仍须证明读到的这些值属于同一个持久事务，检查后的
结果确实保存，未知提交没有被当成未执行。外部库与薄接线的具体条件见
[V3 使用点](dependencies.md#v3-同步协议与收件的具体使用点)，不能以整个 handler
正确作为输入前提，也不能从一次纯校验成功推出系统已经完成卸载或恢复。

## 6. 同步收件链的采用边界

后续同步切片沿当前唯一实现连接
[load](../../verification/experiments/stage1-protocol/inbox-load/README.md) →
[完整查找与 record](../../verification/experiments/stage1-protocol/inbox-record-total/README.md) →
[完整 validate](../../verification/experiments/stage1-protocol/inbox-validation/README.md) →
[save](../../verification/experiments/stage1-protocol/inbox-save/README.md)，复用原
FullLoad / FullRecord / FullInbox / FullSave 合同。正确视图只追加透明调用根；
不重新应用历史业务候选。当前源、局部检查与正式验收状态由覆盖记录分别登记。

load 的正文、完整摘要、重编码、预算及完整校验必须属于同一返回 Inbox。两次配置读取
各自保留，不能假定它们一致，也不补上原实现没有检查的 length 查询与正文长度等式。
标准读取/编码/摘要实例的逐接口结果、SQL 列与参数顺序仍须显式连接。

record 成功本身不保证后态通过完整校验。已有 capture frontier 的相同或更旧 Unload
可被 record 追加，随后由 lineage/save 拒绝；严格更大的 capture 允许编号空洞。
公共 Resume 请求只要求目标代严格更大，当前 record/lineage 还检查 checked successor
及前一个对应 Unload；两层支持域必须分别保留。组合要证明实际拒绝/后态，或落实精确
业务前提，不能把未解的 V3 保持命题交给外部提供者，也不能收窄公共请求合同。

save 对完整同一 Inbox 先校验、再编码、检查输出容量、摘要、原 SQL UPDATE，最后检查
更新恰好一行。当前使用 to_vec 后检查长度，有界输出不等于编码分配上界已证明。
预算拒绝保留编码效果；SQL/行数错误保留已发生的更新效果。外层失败、重放时未调用
save、BeforeCommit 或 AfterCommit 错误都不能独自推出事务没有效果或已经耐久接受。

同步效果接口不替代具体 JSON、SHA、SQL、同事务快照、提交未知和重启证明。认证来源、
当前意图与耐久发送、本地结果及真实资源仍分别连接 V7、V1/V2、V4/V5、V6。
Manifest 的任意已解码文件列表校验，与原文件白名单、实际句柄/长度/字节/摘要及
冻结截面事实分别陈述；前者沿用其精确适用范围，不能以局部正常见证替代一般定理，
也不能把后者或真实 JSON 字节、网络外壳和系统结论一起声称完成。

## 7. Create 身份映射的同步边界

[原收件实现](../../controller-service/src/controller/lifecycle_inbox.rs)的 `owner_for`
按首次完整外部 owner 匹配返回原映射；没有映射时，取完整 World owner 与原高水位的
最大值并受检加一，耗尽返回 IdentifierExhausted。显式循环保留首次匹配和受检计数，
不把表唯一性或 ID 非零添加为调用前提。

`attach_create` 先从原 load 的同一 Inbox 调用 `create_attachment_selection`，
全文查找后按 terminal、refusal、当前版本不等、已有 local、Create 形状检查，返回原
session/entry 位置和 startup deadline。随后由
[`check_existing_create_owner`](../../verification/experiments/stage1-protocol/create-owner-check/README.md)
按完整外部 owner 查找首次匹配；该行本地 owner 不等时立即 Conflict，相等时返回已有映射。
只有没有匹配才读取 World，并调用 `check_new_create_owner`，由原
`create_owner_successor` 排除本次新 sandbox，重算 owner 最大值与高水位的受检后继。
分配错误先原样传播，成功后才比较本地 owner；现存映射不调用该重算，排除 ID 也不能
替换成 owner 数字。两个只读守卫的完整值合同不代替整个父函数或同事务对应。

只读 helper 返回不表示映射已保存。原父函数在 owner 检查之后调用
[`create_attachment_install`](../../verification/experiments/stage1-protocol/create-install/README.md)，
按原 session.owner 完整匹配决定是否追加映射；追加时把高水位赋为本次 local.owner，
已有映射时保留原表和高水位。写入完整 local 和选中 entry.local_operation 后，其余完整 Inbox、
session 和 entry 字段保留。索引由原完整选择给出；
[容量连接](../../verification/experiments/stage1-protocol/create-load-capacity/README.md)
在同一次真实抽取的 Inbox 表示下，从完整 load/validate 接受结果给出 owner 数量上限，
供给完整写入的追加空间条件。该结论只约束实际返回的同一个值；不把合成解码/摘要效果
当作真实 JSON/SHA/SQL 或同事务证明。当前存储格式为 24，旧 Store23 冻结保持其原范围，
不直接改摘要沿用。

[高水位组合](../../verification/experiments/stage1-protocol/create-high-water/README.md)
继续复用原 FullExistingContract、FullNewContract 和 FullInstallContract，以同一完整
Inbox/World/local binding 陈述检查成功到整个 Inbox 写入的合同。已有 owner 分支
保持高水位；缺失分支须由实际 new 守卫成功推出新编号严格大于原高水位，再连接
原完整写入。组合同时保留全部原字段与高水位不下降，不从独立赋值推出单调性。
从合法 load 和完整选择开始的正常见证仅说明此前缀可执行；写后整个 Inbox 仍可能
由后续校验/save 拒绝，不能把安装成功当成持久接受。

deadline 转换、固定 ID、原 UPDATE 与 save 仍属于同一原父函数的业务义务；原寿命
UPDATE 不检查成功行数，不能据此宣称它检查了恰好一行。外层同事务插入、提交未知与
重启以及真实资源/认证来源继续分别连接。函数采用、实际生成体对应与完整组合分开
验收，候选结果的精确版本和当前缺口仍由覆盖记录及 #635 维护。
