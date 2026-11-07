# 原事务的 commit / Drop 有条件连接

Refs #637。这个隔离证明视图处理 `c3e47b98` 的实际 `commit_with` 和 Drop
处理函数，尚未接入公共验收，也不表示完整 Create/Replay 或 V3 已证明。
依赖的权威合同仍是 [A4/B2/B4](../../../../docs/formal/dependencies.md)。

原固定 Charon/Aeneas 生成真实 wrapper、memo、Drop 和 rollback 闭包；只在生成前
把 `TransactionCells::set_committed`、`take_candidate` 两个机械方法设为边界。
没有修改生成文件、翻译器或四个校验器，也没有通用共享内存解释器。

## 证明和信任分别覆盖什么

| 部分 | 实际含义 |
|---|---|
| 真实生成代码的证明 | commit 先清空缓存，再消费原 raw 事务并调用原回调；只有回调正常返回 Ok 才取出并发布完整候选。回调正常 Err 返回原 CommitUncertain，缓存保持空，不推断是否持久提交 |
| 真实 Drop 处理函数的证明 | 原 raw 槽为空时不调用 rollback；非空时消费它，正常 rollback Ok 保留缓存，正常 Err 清空缓存。没有从 Err 推出连接已关闭、数据库没变或回滚完成 |
| 两个可信单元格操作 | 在正常借用、两个原对象不同、无冲突 guard 的范围内，setter 把给定完整值写入原 committed；taker 返回完整原 candidate 并只清空 candidate。模型的值等式不是这些物理写入的机器证明 |
| 原对象和回调连接条件 | committed/candidate 对应原字段，不能互换；回调及其清理不修改或重入这两个单元格，数据库调用期间不持有可变 guard。每次新调用都重新取得实际观察，不能沿用旧 Self 快照 |
| 正常退出调用 Drop | 仍依赖 Rust 编译器与本调用的正常所有权清理。LLBC 中保留清理位置，但生成 wrapper 没有自动调用已抽取 handler；抽取 handler 不等于证明自动调度 |

`LinkedCommitted` 是具体等式：实际仍存活的 committed 值，等于这一次原 memo
返回的单元格观察。它不把目标 None 或候选发布写进前提；目标来自真实生成 memo
的定理。候选只在 memo/handler 返回且对象仍存活时观察，不承诺消费 owner 后候选
对象继续存在。panic、unwind、abort、崩溃及任意 `forget` 不在本正常返回范围。

`RefCell T = T` 只表示这个有限边界的内容观察，不能解释任意 RefCell 操作。
原透明抽取版本曾丢失共享写回，不能给其添加与定义矛盾的 setter 假设；本视图只给
新抽取的两个外部方法提供模型。没有提供 borrow_mut/deref_mut 的虚构实现。

## 复用和检查

原 `SandproofSqliteRust.Model.Slots` 的声明仅移到无生成类型依赖的
`SandproofSqlite.SlotData`，名字和字段保持不变；旧 Model 重新导入该唯一类型。
新字典保留原两项更新表达式，13 项原 Results 只按 `provider-reuse.json` 的替换
映射到本次实际生成函数。有限读观察复用原 witness 的数据定义，避免导入另一套
完整生成类型。没有另造 World、缓存或 SQL 执行器。

`profile.json` 固定源、两根、恰好两个自有边界及原工具摘要。复现时在原隔离环境
按其中范围捕获并保留 raw LLBC、Types/Funs 和外部模板；检查实际符号/签名后，
将本目录模板原样物化到私有 `SandproofTxFinish` 库，再运行
`lake build SandproofTxFinish.Audit`。原 SQLite Audit 单独构建，避免混入两个
生成命名空间。实际文件仍通过项目原 `formal.py::actual_files` 和原服务记录读回
检查，不能只看 Lake 的退出码；私有执行脚本与原始证据由本任务交接保留。

有限生成成功；48 项本视图定理及受影响旧 SQLite 的18项审计已通过原内核和实际文件
服务。最后一轮检查9份受影响文件，19份按前轮完整原件、全部导入和固定工具核对复用。
旧的写后读证明仍绑定其原 `9de84ca1` 输入，不改签为本次 Rust。

原生检查只使用原缓存一致性测试
`uncertain_commit_reconciles_both_durable_outcomes_for_every_write_family`：正确版通过；
仅交换 commit 中两 RefCell 引用的错误版确实编译，被原 parity 相等断言拒绝；恢复
原字节后通过。三相没有修改断言、memo、两原语、SQL 或注入逻辑。它支持具体接线
条件的采用，不把可信写入或自动 Drop 升级为机器证明。

完整父链仍须连接真实 record、候选来源、四项校验的实际调用和首错顺序、构造前段
及本次库观察；各业务算法的条件保持其内部提供责任，不因本有限视图成为可信库。
