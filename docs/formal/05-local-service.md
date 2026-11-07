# V5 SQLite、持久引用与恢复编排

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#637](https://github.com/VeriInfra-Lab/sandproof/issues/637)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 唯一持久权威

对应 [controller-service](../../controller-service/src/lib.rs) 原 SQLite、工作/结果和恢复入口。snapshot 元数据、唯一认领、持久 pin 与删除意图扩入原权威；不新造第二本业务账。V4 决定是否合法，V6 提供原文件/进程事件。

模型区分已提交状态、内存候选、文件已发布、外部工作已发生和提交结果未知。SQLite 引擎是外部条件；实际 schema、SQL 条件、事务边界、回调、参数和重启枚举是自有义务。

## 2. 事务与责任接口

| 接口（语义名，非已实现 API） | 事务必须绑定的内容 | 失败/未知 |
|---|---|---|
| acceptCapture | 原请求全文、源代、capture、原物理身份、预算、工作 | 未确认不派发；未知查询同一操作 |
| registerBundle | 原捕获计划、完整校验回执、不可变文件身份、pin/资格 | 文件发布不自动变业务成功；孤儿只按原计划对账 |
| claimResume | 当前会话/来源/寿命、唯一操作/新代、资源预留、pin | 同键返回原 claim；其它竞争失败，不复制 claim |
| recordMayExecute | 原 claim/候选和当前终止版本、不可逆标记、激活工作 | 确认提交后才能可能激活；未知不换候选 |
| releasePin / beginRetire | 绑定原材料的具名结束证据，终止或完整后继接管；同一事务检查并永久关闭新引用、保存固定原计划及待办 | 活跃/未知使用不清零；提交未知不给许可，重启不靠内存引用数 |
| scheduleDelete / settleRelease | 精确对象与原 work/plan、锁外实际删除/映射释放结果、原计费 | 部分成功和丢回执沿原计划续做；后继进一步演进不妨碍旧计划结算，重复不双退额 |

上述接口需映射实际函数/SQL，不要求建立同名壳函数。状态更改与生成待办必须同事务，接受时保留后续记录空间；普通请求不能耗尽内部终止/清理能力。

## 3. 崩溃恢复和进展

每次外部效果之前都已有可枚举的原身份责任。文件 rename 与 SQLite 提交之间崩溃时，原 capture 意图说明文件为何存在；仅扫描到完整文件不能赋予恢复资格。恢复必须核对版本/完整内容/路径身份并按原状态登记或清理。

pin 是具名持久集合，可重建 original loader、candidate、running backing 和 cleanup 责任。未知加载仍持 pin，实际进程消失需精确证明，不用 PID 查询缺席冒充永无迟到 worker。Retiring 后禁止重新取 pin，所有删除者服从同一状态。

后继回收使用原 SourceWork 和 SQLite，不新建第二本 GC 账。A 恢复为 R1 后保存 B，原 R1 释放完成、B 成为 Unloaded 的同一事务先固定匹配 A.restore/R1/B 的真实释放事实；其它写者或 pin 尚未结束也保留该事实，不能等每 sandbox 的最新释放见证被覆盖后再猜。全部原使用闭合、B 完整持久接管后才进入 Retiring。保存的理由和结束事实绑定原材料，不依赖当前会话 terminal/epoch/operation；B 又被 C 接管或后续续期不得令旧计划 Corrupt 或永远不能结算。计划结构校验与新删除许可的现场物理核验分别证明。

原退休 plan 保存 `bytes/work_binding/plan_binding/closure`；closure 的
`release_binding` 绑定 producer/consumer 原释放事实，`restore_binding` 绑定原恢复
intent/local/generation/activation（不含可关闭的 material_use，无 claim 时显式 null）。
`reason` 严格区分原 terminal operation/epoch/workspace 与 superseded 的后继 work、
完整 Full 及 manifest 摘要。后继还须连接原 execution_origin、已释放 R1 的 generation/VM
及原 Pause 已登记成功结果；仅编号增长不提供接管。保存后保留历史 B 的这些绑定，
不要求 B 永远是当前来源。上述摘要只命名原事实，不代替真实结束或业务资格。

物理准备、删除及观察在管理锁和 SQL 事务外执行；返回后重核原 identity/work/plan，不把旧 World 整体写回。Held 与新引用竞争由同一事务裁定，Retiring 保留原计划和整笔未结额度，Settled 仅由匹配的实际回执建立。大文件删除保留消费标记、结果重放及必要 manifest 摘要；原 SourceWork/inbox 的有界元数据退休仍沿既有窗口规则。旧 Resume 不重新取得加载资格。

恢复驱动独立于 HTTP future。证明待办在条件满足时可被选中、每个本地有限扫描终止、锁/队列不会让停止等待自己的 backing 删除；外部 I/O 永久阻塞不承诺最终完成。锁的机械保证不是这些业务推进的证明。

## 4. 连接与验收

抽取真实同步事务决定、参数构造、回调和结果分类；原异步 SQLite/任务接线仅按已登记窄合同。表约束/受影响行数/比较字段/提交错误顺序全部接实际版本。

覆盖提交前后崩溃、文件发布未登记、capture 已提交但 stop 回执丢失、claim 与 GC 竞争、load 未返回、activate 丢回执、部分删除、ENOSPC、取消等待以及两次 manager 重启。正确版本满足 LC-P3/5/7/10/11/12；“commit unknown 当未提交”“加载返回就释放 pin”的真实错误版本须失败。V6 仍需证明实际文件与进程后果，不能由数据库行自证。

## 5. 显式卸载检查点

当前服务只接受完整初始化的生命周期库。服务格式取共享 `HOST_STORE_FORMAT_VERSION`（材料级退休格式为 24），核心检查点版本为 2；旧 `INTERNAL_INBOX_FORMAT_VERSION = 16` 只用于历史组件断言，不是另一生产模式。旧 HOST23、FirstUnload/Source18 分阶段启用说明不提供另一生产模式；不保留自动迁移或旧读者备用逻辑。SourceWork 的必填 `material_release` 精确保存 producer/consumer 两个原 `HostReleaseWitness` 或显式 null，单记录上界 4096 字节，并在每条原 SourceWork 预留中单独计入；缺字段不能按旧形状读取。

检查点和原 SQL 投影支持多轮 Pause/Resume、来源工作及材料责任；`check_input_profile` 通过 `permits_resume_input` 核对原持久恢复记录，拒绝绕过准入的 Resume。结构校验包含原身份、epoch、计划、来源、资源和捕获未知状态，不能靠反序列化出一个 Paused 值声明物理卸载。原 WorldTransaction、输入日志、outbox、回执与剪枝共同承担重启恢复，不无限保留全局历史代替当前责任。

## 6. 正式接收、启动与首次卸载

`record_lifecycle_intent` 在原事务保存中心完整请求和顺序，同键全文重放，冲突拒绝。`admit_lifecycle_create` 沿原准入，在同一次事务保存外部请求到本地身份的映射和待办；提交未知查询原键，不重分配。真实后台通过 `advance_recorded_lifecycle_create_with_verification` 消费它，独立于 HTTP 等待者。宿主 epoch 必须重新观察和核对，重开后的历史配置不能自行授予新派发资格。

当前卸载链已接入 `application/coordinator/lifecycle_unload.rs`：

1. 原已接受 Unload 与 Create 来源、寿命、版本及 SourceWork 对应；撤销数据访问并取得所需排空事实。
2. `lifecycle_inbox/source_work/full/admit.rs` 在原 SQL 接受 Pause、材料预算与工作；准备物理 worker 后，`lifecycle_inbox/source_work/full/authorize.rs` 保存原 worker/outbox 与当前条件，确认提交才给 `FullCaptureLaunchPermit`。
3. V6 放行真实 Full 子进程，`lifecycle_inbox/source_work/full/complete.rs` 在原事务采纳完整材料；Stop、运行清理与位置归还沿各自实际回执推进。
4. `lifecycle_inbox/source_work/full/unloaded.rs` 只在原材料、无 current_vm、VM/slot 收尾、寿命及持久释放确认齐全时提供 Unloaded。核心 Paused、文件发布或 VMM 退出单项均不充分。

这些是当前源码接线事实，完整 SQL、外部效果与调用顺序的 Lean 对应尚未闭合。有限成功实机和故障缺口见覆盖记录；不能把引擎事务保证扩为整个自有回调正确。

## 7. 当前观察与锁外慢操作

`observe_lifecycle_create` 区分历史结果与当前 Ready；后续 Unload/Destroy、到期、源身份不足各返回相应类别，历史成功不是当前访问资格。中心保存当前事实和确认结果仍需 V2/V3 对应。

SourceWork 绑定原完整 Unload、Create 映射、VM/运行代/epoch、寿命、源进程、启动事实、选项与受保护 HostBootstrap。应用先在锁内核对原工作，再释放管理锁做 Host 只读 I/O，最后重新进入原事务核对当前条件并保存；不把旧 World 整体写回。另一会话的推进不应被慢来源观察阻塞；同会话终止/换代后的迟到结果不得采纳。

来源 GET 配置、摘要和长度是观察事实，不是捕获许可、固定硬件兼容证明或物理预算。提交未知保留原责任；manager 重启、worker 完成及部分文件发布仍按原身份对账。已发布 Full 的有限恢复不能代替所有准备阶段故障恢复。

## 8. 恢复、失败继续的实现连接与待证义务

恢复已在原 SQL/SourceWork 接入唯一后继、材料使用责任、候选身份、激活前持久记录及多轮工作；不再将 Resume 描述为收件模型一律拒绝。相关正式组合已有指定版本的正常与部分故障实机证据，适用版本和未覆盖场景见覆盖记录；完整 SQL 与真实 Rust 对应证明仍须逐项完成。

普通 Pause 失败沿唯一原 Pause/Full 工作推进：`prepare_lifecycle_original_continue` 在原 `WorldTransaction` 验证当前 Unload、原寿命、源身份和 typed End，封住后续捕获放行，并将 End、核心 `PrepareOriginalContinue` 及 outbox 一起提交。提交未知不返回执行许可，重开后按原工作查询。SQL End 区分 `NeverReleased` 与 `WorkerEnded`；后者保存原工作器的 `NoRequest / Returned` 互斥结果，分别对应明确未发送或真实 API 结束，并要求原 worker 退出，不以摘要、缺席或一次 Running 查询代替。原未决 Seal 可按核心的精确结束规则退休，但 Freeze/Commit 未知仍须解决。原始 `apply(Capture)` 不能绕过这条事务准入。具体核心条件见[V4](04-local-core.md)。

Node 已有原实例执行/只观察方法：实际执行要求原 action 与完整工作摘要的私有许可，只继续同一 VM，不加载快照、不重新初始化来宾、不替换凭据；前后核对原配置、进程、位置、网络及文件授权。外层 `application/coordinator/lifecycle_continue.rs` 已接原许可、实际调用、观察与继续结果发布；源码接线和组件通过不代表完整实现对应或真实 VM 组合已验收。

仍需证明或补足故障验收：

- 验证已接入原后台的事务、私有执行许可、实际调用、原回执及新鲜授权观察；准入前尚无 native Pause 的失败已沿 `close_failed_unadmitted_unload` 关闭原 Inbox 并观察原执行，仍须证明无捕获 SourceWork 的退休和结果发布，不得伪造 Full 工作。
- Full 已放行而 API 结束记录丢失时，不能由 worker 缺席或新连接 GET 推定旧调用已经结束。安全继续原 VM 的充分观察尚未闭合；显式 Destroy/TTL 的原终止计划也须能停止精确原 VMM、排空写者后收尾，不能被同一个未知 Full 永久挡住。
- 原持久退休计划、精确删除许可、物理回执结算及 `maintain` 的材料/元数据收尾已有接线；终止与后继接管沿同一材料级流程，当前会话仍存活时也须能调度旧代。其引用关闭、固定原释放事实、重启续计划及最终退额仍须组合证明，真实回收证据按覆盖记录的具体场景复用。`retains_session` 保守保留不是完成；有界删除计划也不等于已获删除许可。
- 禁止新引用与退休在原事务决定，File backing、loader、VMM 和写者的责任分别结束，才能按实际删除结果结算；TTL 不直接 unlink。

当前已实现内容及检查范围以[覆盖记录](../proof-coverage.md)的精确版本为准。可先认领稳定同步入口的证明，不能把工程接线、受控物理输入或尚未验证的故障收尾作为已证业务前提。
