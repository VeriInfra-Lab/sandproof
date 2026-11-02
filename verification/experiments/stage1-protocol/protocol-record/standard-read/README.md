# StandardRecord 的两个只读方法

本目录给出当前 `recording.rs` 中 `StandardRecord::releasing` 和
`StandardRecord::current_operation` 的真实实现对应，不改生产方法或父链。
两个方法已由原 Charon→Aeneas 捕获；八条数学声明经原 Lean 服务检查通过。
正式物理页、公共登记与整批验收分别记录，不能由此认领完整 V3 完成。

`root.rs.in` 只在新的私有源码副本追加两个透明委托。`support.py` 复用现有
create-standard-adapter 的 World／库表示规则，并将 INCLUDE 限定到实际查询闭包，
OPAQUE 保持原 `rusqlite::error::Error`。两个根均指向原实际方法；不能将这两个
自有方法、比较、Store 查询或循环整体设为 opaque。独立生成命名空间为
SandproofStandardRecordRust，子目录为 generated/SandproofStandardRecord。

目标合同只覆盖正常借用、候选为同一 some World、普通值克隆／分配条件下的一次读取：

- releasing：在该次原 World 的 maintenance.host_slots 中，存在原行满足
  slot.binding.sandbox == local.sandbox 且 phase == Releasing，恰好返回 true；
  maintenance 缺席或没有这样的行返回 false。原代码不检查 local.owner／operation，
  不能将它们添加为已检查条件，也不假设槽表唯一。
- current_operation：通过原 World.sandbox_record／Store.get 取原顺序的首次匹配行，
  返回该记录 current_operation 的完整 id；缺行或首行无当前操作则为 None，不能用
  后面的重复行补救，也不从 id 取值推出新鲜性、终止或成功。
- 两次方法各自保持完整效果值，不换 namespace、事务、候选、连接或其它字段；
  这是普通值观察，不能冒充整个 RefCell／堆／物理析构的证明。

WorldTransaction.world、NormalWorldBorrow 和 WorldReadCell／Guard 复用原 Create
借用模型，Store 首次查找与普通共享查询复用 V0。库条件不包含自有比较或循环已经
正确；借用冲突、缺候选、panic/unwind、资源 Drop／取消／崩溃不变成正常 false／None。
原正常借用模型不保证任意时点、并发读或跨调用始终属于同一快照。

父链仍须由具名 V4/V5 条件提供：原 tx/candidate 来源、调用后读取的是终止作用后的
同一事务候选，以及 terminate_local 建立的真实业务状态。这里不证明 terminate_local，
也不假设“V5 正确”就跳过原 sandbox 首查与 operation.id 连接。

数学与有限提供方先在私有草稿中尝试；原工具实测由统筹协调 msra 四核。
未变的 World 克隆／Store 数学优先复用，不增加无消费者 helper、堆解释器或错误矩阵。

实际抽取只保留两根、两个方法及闭包、WorldTransaction.world、core World／Clone／Store
和实际 Option 调用。StandardRecord、WorldTransaction、CommittedWorld 的完整字段透明；
不因此抽取 SQL／commit／terminate_local，没有更改 typecheck 或扩大 opaque。

准入必须在原 IR／translation 中核对两个方法、WorldTransaction.world、World.clone、
World.sandbox_record、Store.get 及其循环、自有 key／phase 比较和实际回调透明；原方法
所有实际可达业务调用都要列明，不能靠未登记 opaque 隐藏。未调用的其它 trait 方法
不属于这两个查询的证明依赖；terminate_local 仍是父链具名业务义务。

## 库条件与复用

`TypesExternal` 和 RefCell／克隆模型沿用原 StandardCreate 的普通值合同。
`Access.Cloning` 只绑定到本生成命名空间，证明正文沿用原文件；Store.get 通过
`get_agrees` 连接现有 V0 LoopCallbacks 的首次匹配定理，不重新证明表查询算法。

额外标准库符合性仅涉及固定共享切片 `Iterator::any`：按顺序调用原 FnMut，传递
回调状态，第一个 true 即停止，失败／发散原样传播，返回迭代器保留实际消耗位置。
模型使用原 Aeneas `Iter { slice, i }`。它适用于此处普通共享借用和只捕获 SandboxId
的闭包，不提供资源 Drop、并发内存、越界裸指针或自定义析构保证。自有闭包中的
sandbox 比较和 Releasing 判断由 `actual_slot_callback` 证明，没有放进库假设。

`actual_releasing` 给出整份效果值不变及原槽表上的完整判断；`actual_current_operation`
给出原 sandbox 首次匹配后的 `current_operation.id`，并保持整份效果值。
它们各自消费该次 `CandidateAt`，不证明跨调用同一候选、并发无干扰，或 V5 已建立
该前提。父 `record_using` 的接口表示连接和终止后读取责任仍须单独实例化。

## 有限验收入口

`capture.json` 固定原成功捕获、完整生成物、57项实际源码/配置与两根范围。
`view.json` 固定32份有限视图文件及原样复制或仅命名空间绑定；生成 Funs／Types
保持原字节。未变公共提供方仍绑定完整输入；新增和重绑定的七页经原内核及服务
检查，完整九项审计包括八条读取定理及原 CaptureHandoff 克隆引理。

```sh
export SANDPROOF_STANDARD_RECORD_CAPTURE_EVIDENCE=/private/original-read-capture/actual
python3 verification/experiments/stage1-protocol/protocol-record/standard-read/formal.py \
  --output /private/new-read-proof
```

入口使用同一固定工具和原 `actual_files` 检查器。可显式设置
`SANDPROOF_STANDARD_RECORD_PREPARED_PROJECT` 复用本模块已准备的私有视图和 `.lake`；
每份输入与工具/库绑定仍需一致，不改旧源来凑摘要。
后续同一数学输入可用 `SANDPROOF_STANDARD_RECORD_REUSE_EVIDENCE` 指向上述完整结果目录，
严格读回所有内核／服务／捕获证据并保留原执行身份，不重跑未变证明。缺失、失败、
变化或错误视图均拒绝，不能以仅抽取成功、临时稿通过或 `complete` 字段替代验收。
此目录不启动普通测试；既有 `stage1-protocol-record` 原生组件由公共批次负责。
