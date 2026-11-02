# StandardRecord 的两个只读方法

本目录只准备当前 `recording.rs` 中 `StandardRecord::releasing` 和
`StandardRecord::current_operation` 的真实实现对应，不改生产方法、父链或公共登记。
目前为准备／草稿，尚未取得本目录的原工具捕获或 Lean 检查结果，不是验收完成。

`root.rs.in` 只在新的私有源码副本追加两个透明委托。`support.py` 复用现有
create-standard-adapter 的 INCLUDE／OPAQUE，两个根均指向原实际方法；不能将这两个
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
