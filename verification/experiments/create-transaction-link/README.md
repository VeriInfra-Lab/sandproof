# Create/Replay 的连续事务连接候选

Refs #637。`9de84ca1` 已完成真实 Rust 双 BLOB 更新后读取的局部证明，以及同一性质的
原生正确、错误和恢复检查。尚未接入公共证明入口，不表示完整记录/提交链或 V3 已完成。

实际 commit/Drop 的后续有条件连接见 [有限两单元格视图](finish/README.md)：
它复用原 memo 数学，并明确保留物理写入、对象绑定和正常退出调度的可信条件。

生产候选把同一个 `WorldTransaction` 移入现有 `StandardRecord`，记录结束后再移出，
继续执行原重放返回或提交分支。它不复制事务或 World；记录出错时由原事务的 Drop 收尾。
保存仍执行原编码、完整字节与摘要、原 SQL 和恰好一行检查；只有原 UPDATE 的转交
经过 `transaction::sqlite_io::execute_blobs`。该函数把原 raw transaction、statement
和两份完整 BLOB 按原顺序交给真实 SQLite，保留原始行数和错误。
四个自有校验器、候选读取、提交与 memo 逻辑不改。

验证中的库接口需要返回完整结果及原事务的后续观察：成功和库错误都传回同一个原
句柄的后态。错误允许已有部分效果、自动回滚或未知状态，不能由句柄仍存在推出 SQL
事务仍活动。后一次读取必须消费前一次写入留下的视图，不能从旧观察函数任选结果。
这只具体使用既有 [A4/B4 合同](../../../docs/formal/dependencies.md)，不把整个事务、
四个校验器或记录业务设为可信函数，也不引入自定义公理。

## 已证明的实际入口和条件

[roots.rs.in](roots.rs.in) 只追加到隔离源码，不进入产品运行入口。
正确函数调用原 `StandardRecord.update`，再调用原 `query_bytes_pair`；错误函数只交换
这两个调用的顺序。两者都由固定原版 Charon/Aeneas 抽取，保留原生成体，未把这些
自有方法设为 opaque。`execute_inbox_update` 直接匹配原事务槽；`None` 保留原 panic，
不添加自定义的可变 `Option.expect` 翻译。

| 结论 | 实际证明和范围 |
|---|---|
| 完整写后读 | `actual_full_write_read_contract` 对实际正确生成函数证明：SELECT 消费 UPDATE 返回的视图，保留原行数、两份完整 BLOB、后继所有者和首错；外层失败/不返回按原 Result 传播 |
| 读取刚写入的内容 | `actual_success_returns_complete_new_blobs` 另要求 A4 固定 UPDATE 的一行写入条件，以及这次 SELECT 的正常 prepare/首行观察，推出完整新 bytes/digest；不能从 UPDATE 成功推出 SELECT 成功 |
| 同一规格拒绝真实错误版 | `actual_wrong_rejected` 对实际先读后写生成函数给出具体反例；旧新字节不同，行数和摘要长度相同，不能靠编译失败、只比摘要或只比长度通过 |
| 原槽与值的转交 | 保留两项值枚举往返及两项实际更新所有者/错误后继定理；SQL Err 后没有连接仍活动的结论 |

规格在 [FullWriteReadSpec.lean.in](FullWriteReadSpec.lean.in)，不导入正确函数的对应定理。
正确版、内容推论、错误版证书分别在 `FullWriteReadCorrect.lean.in` 和
`FullWriteReadWrong.lean.in`，均检查真实生成函数，而非另写一个 Rust 解释器。
反例的有限库观察在 `FullWriteReadWitness.lean.in`，不提供任意 SQL 的执行器。

## 已执行的检查

`9de84ca1` 的两根实际 Rust 已重新经过原 Charon/Aeneas；同一三项结论、原四项审计、
受影响的十份实际物理文件均通过原内核、文件服务诊断及原始记录读回，总耗时约 131 秒。
另外十二份未变闭包文件按原源码、依赖、工具和原始证据严格复用，不称本次重新证明。
旧 `161f242b` / `6fe28de6` 的证明和捕获保留原版本；格式后根的生成物另行保留，
不把旧成功记录换标签用于新输入。

[full-write-read-native.rs.in](full-write-read-native.rs.in) 在原普通测试环境接通真实
SQLite 和原 `WorldTransaction`。完整 BLOB/原行数断言在三相中不变，仅切换被调用的
真实根：正确版退出 0，先读后写版确实编译后因完整内容断言退出 101，恢复版退出 0。
三份测试程序均为当前同 crate 业务与测试源码的新编译结果；恢复源码及程序摘要与
正确版一致，总耗时约 42 秒。最初格式检查失败的记录另存，不计为性质反例。
普通检查和证明检查分属原各自环境；这些是局部检查，不是完整普通或公共证明封口。

## 仍需提供的保证

[共享 SQLite 接口](INTERFACE.md) 列明具体库绑定和条件。SQLite 及机械转发仍依赖
已登记 A4/B4：实际使用同一个原事务、完整 SQL/参数，成功写入与后续成功读取一致。
纯 Lean 载体相同不证明物理句柄相同，接口模型的定义也不等于真实 SQLite 已被验证。

生产 `record_lifecycle_intent` 尚须连接实际 load/认证/重放或记录、编码与一行检查、
原四个校验器、同一候选 World、commit/memo 和真实 Drop。当前窄入口没有覆盖这些
完整业务决定。正常 Drop 的库测试不能代替 Drop 对应证明；错误保留部分写入的
trigger 只是库原语故障夹具，不是生产 schema。后续先固定最小实际调用段和库接口，
不以当前局部成果或宽闭包抽取失败宣称这些义务已解决。
