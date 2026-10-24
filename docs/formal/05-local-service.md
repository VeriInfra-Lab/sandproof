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
| releasePin / beginRetire | 具名持有者结束证据；同一事务检查新引用关闭、资格/引用状态 | 活跃/未知使用不清零；重启不靠内存引用数 |
| scheduleDelete / settleRelease | 精确对象与原删除责任、实际删除/映射释放结果、计费 | 部分成功可重试，未知仍占账 |

上述接口需映射实际函数/SQL，不要求建立同名壳函数。状态更改与生成待办必须同事务，接受时保留后续记录空间；普通请求不能耗尽内部终止/清理能力。

## 3. 崩溃恢复和进展

每次外部效果之前都已有可枚举的原身份责任。文件 rename 与 SQLite 提交之间崩溃时，原 capture 意图说明文件为何存在；仅扫描到完整文件不能赋予恢复资格。恢复必须核对版本/完整内容/路径身份并按原状态登记或清理。

pin 是具名持久集合，可重建 original loader、candidate、running backing 和 cleanup 责任。未知加载仍持 pin，实际进程消失需精确证明，不用 PID 查询缺席冒充永无迟到 worker。Retiring 后禁止重新取 pin，所有删除者服从同一状态。

恢复驱动独立于 HTTP future。证明待办在条件满足时可被选中、每个本地有限扫描终止、锁/队列不会让停止等待自己的 backing 删除；外部 I/O 永久阻塞不承诺最终完成。锁的机械保证不是这些业务推进的证明。

## 4. 连接与验收

抽取真实同步事务决定、参数构造、回调和结果分类；原异步 SQLite/任务接线仅按已登记窄合同。表约束/受影响行数/比较字段/提交错误顺序全部接实际版本。

覆盖提交前后崩溃、文件发布未登记、capture 已提交但 stop 回执丢失、claim 与 GC 竞争、load 未返回、activate 丢回执、部分删除、ENOSPC、取消等待以及两次 manager 重启。正确版本满足 LC-P3/5/7/10/11/12；“commit unknown 当未提交”“加载返回就释放 pin”的真实错误版本须失败。V6 仍需证明实际文件与进程后果，不能由数据库行自证。

## 5. 显式卸载检查点

当前服务只接受完整初始化的生命周期库。服务格式取共享 `HOST_STORE_FORMAT_VERSION`（本次核对为 21），核心检查点版本为 2；旧 `INTERNAL_INBOX_FORMAT_VERSION = 16` 只用于历史组件断言，不是另一生产模式。旧 FirstUnload/Source18 分阶段启用说明已被当前唯一格式替代；不保留自动迁移或旧读者备用逻辑。

检查点和原 SQL 投影支持首次 Pause/Capture/Stop、来源工作及材料责任；`check_input_profile` 仍显式拒绝 Resume。结构校验包含原身份、epoch、计划、来源、资源和捕获未知状态，不能靠反序列化出一个 Paused 值声明物理卸载。原 WorldTransaction、输入日志、outbox、回执与剪枝共同承担重启恢复，不无限保留全局历史代替当前责任。

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

## 8. 尚缺的持久闭环

- Resume：当前收件模型与检查点拒绝；唯一 claim、加载/候选/运行 backing 的具名持久 pin、激活前 MayHaveExecuted 与恢复待办仍需实现。
- 普通 Pause 失败：保留原状态已实现；安全解除捕获责任并继续原 VM 的完整 `ContinueOriginalVm` 路径未交付。
- 卸载后 Destroy：`source_work::retains_session` 保守保留来源责任，`access::destroy_cleaned` 还要求 snapshot 和 capture/restore 引用消失。没有原责任退休、精确材料删除与额度结算闭环，不能宣称最终 Cleaned。
- 材料 GC：固定 `capture_work`/`unload_source` 两个责任槽不是通用 pin 集合；禁止新引用与退休必须原子决定，实际删除及未知结果仍由 V6 返回原身份事实。

同事可先证明当前绑定、预算校验和首次卸载的同步决定；不可对不存在的 Resume/GC 实现签发“已证”结论。完整义务保持第 2–4 节不变。
