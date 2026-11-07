# 共享 SQLite 观察接口候选

[切片范围](README.md) · [正式依赖登记](../../../docs/formal/dependencies.md)

本接口供 Create/Replay 事务连接和真实 Unload 来源读取共用。定义在
[SqliteSessionData.lean.in](SqliteSessionData.lean.in)，尚未构建、连接实际 Funs 或通过正式验收。
候选提交不是已经交付的库保证；实际 Rust、模型绑定、内核、错误版和完整验收仍须接通。
本包不修改 Charon/Aeneas，不定义另一套 World 或 RawTransaction，不解释任意 SQL。

## 唯一类型与复用位置

| 名称 | 内容与边界 |
|---|---|
| `SingleReadView` | 仓库已有 `SandproofLoadBlobRust.ConnectionReadObservation` 的精确别名；保留整个 canonical 连接、参数、列、首行与错误观察 |
| `Row` / `RowStatement` / `SqlError` / `SqlValue` | 沿用原 standard-blob / sqlite-statements 的完整载体；不新增第二份行或错误模型 |
| `Query` | 原 SQL 全文和绑定后的完整有序参数；参数不由业务键或摘要代替 |
| `ScanTrace` | 一次实际游标的逐步库观察，包含外层失败/不返回、SQL Err、行或结束 |
| `ReadView` | 原单行观察和允许的扫描观察集合；不是物理数据库堆的已证表示 |
| `Connection` | 唯一 `current + step` 载体；`step` 返回原完整库结果和后续视图 |
| `Statement` / `Cursor` | 原 issued statement 和一次查询的观察；不拥有另一份事务或连接 |
| `MappedCursor` | 原游标及原回调状态；回调返回 Err 时仍保留其实际后态 |

这些类型将物化为一个共享模块 `SandproofSqliteSession`。两线必须从同一候选定义复制，
核对字节与依赖，不各改一套定义。原 `SandproofLoadBlob.TypesExternal` 及其
`ReadValueData` 继续由现有提供方物化；这里不导入、复制或转换完整 World 类型。
实际 raw Transaction 使用本批真实生成类型，只将它的外部 Connection 别名接到此载体。
DropBehavior、原槽和其它字段不由库模型替换。

## 实际生成签名与未完成部分

`61fd3361` 的实际更新后读取窄入口已于 2026-11-07 在原 MSRA 环境完成原版
Charon/Aeneas 生成，尚未连接下列模板或进行 Lean 内核检查。实际库转发签名为：

```lean
execute_blobs : RawTransaction → Str → Slice U8 → Slice U8 →
  Result (SqlReply Usize × RawTransaction)
```

这里 `RawTransaction` 是实际生成的 `rusqlite.transaction.Transaction` 的简称，
不是本包新增类型。真实窄入口返回业务结果与 `StandardRecord` 后继，生成体将
UPDATE 返回的 `effects1` 交给实际 `query_bytes_pair`；UPDATE 的正常 SQL Err
分支同样返回 `effects1`。因此既有 Aeneas 可以保留原事务所有者的后继，无须定义
另一套 World 解释器；这不等于已经证明后继满足预期数据库条件。

实际外部模板还含 `Option.as_ref/as_mut`、`Connection.query_row`、单位参数绑定、
`Row.get`、`usize` 列索引和 `Vec<u8>` 的 `FromSql` 等待绑定项。应按这些实际
签名提供定义并核对原库行为，再检查实际生成体；生成模板中的 `axiom` 洞不能
作为已交付实现。游标扩展也仍需与实际生成签名连接，不能从本次窄入口推称已覆盖。

## 写入后的视图

唯一新增机械边界为真实 `transaction::sqlite_io::execute_blobs`：借用原 raw T，
转交原 statement、bytes、digest 这两个完整 BLOB 及其顺序，保留原行数和错误。
`executeBlobs` 只描述观察载体的转交；还须对真实生成的原槽投影、所有者返回和
StandardRecord 方法做对应，不能由这个辅助定义宣布实际接线正确。

逐调用 A4/B4 条件应分别说明：

- 此次调用确实使用同一个原 raw T 与完整原实参，不靠 Nat 标签相等证明物理身份。
- 对固定 Inbox UPDATE，正常成功且恰好影响一行后，当前事务该行的两列是这两个参数。
  后续 SELECT **如果正常返回行**，其完整两列必须来自这个更新后的视图。
- UPDATE 成功不保证下一次 prepare、step、读取或转换必然成功；零行和其它行数也不改成一行。
- 正常库 Err 也返回后续观察，允许已有修改、自动回滚或未知状态；保留的 Rust 句柄
  不代表 SQL 事务仍活动。外层失败、不返回、panic/崩溃不提供正常后态。
- SQL 选择、参数与实际编码/摘要绑定、恰好一行检查、四个校验器、memo 和提交决定
  都是自有代码义务，不属于此库边界。

## 读取和游标

先沿原模型处理实际 prepare、Params 绑定和首错，再连接此次 issued cursor 的观察。
`ReadView.scans query trace` 只是逐调用库条件的接口：真实绑定仍须证明这个 trace
来自本次原连接、原 SQL、原完整参数和原 statement，不能任意挑一条成功序列。
本候选不提供未检查的 prepare/query_map 外部定义或选择公理。

同一 SQL 不等于同一次调用。数据库内容快照、一次调用的错误/行观察和游标位置要分开：
允许不同调用各有 prepare/绑定/next 错误，无 ORDER BY 的扫描也不固定重复调用的顺序。
因此扫描是允许观察的关系，不是由 SQL 字符串决定唯一行序列的函数。
将来实际外部绑定若不能保留本次观察或所需后态，必须先报告连接缺口，不能把关系
当成可直接执行的成功函数，也不能把未覆盖的错误轨迹说成已证明。

| 库观察 | 需要保留的行为 |
|---|---|
| 单行 `query_row` | 原 SQL/参数、只取首行、每列 get 和转换顺序；它不额外证明唯一行 |
| `OptionalExtension` | 仅原 QueryReturnedNoRows 变成外层 None；其它错误保留。外层无行和行内 SQL NULL 不同 |
| `query_map` | 原 prepare/绑定/游标创建的结果；不能在它们失败时提前 next 或调用业务回调 |
| `cursorNext` | 本次游标的下一项观察；结束、SQL Err 和实际行保持不同 |
| `mappedNext` | SQL Err 不运行回调；有行才运行原回调一次，回调 Err 及回调后态保留 |
| 源码中的循环 | 首个 Err 或结束后不再 next，列读取首错后不读后列；这仍需证明真实自有循环 |

游标中的 `calls` 是逻辑观察序号，不是数据库行号或物理位置。其变化不证明 DB 内容或
事务状态。正常成功的只读前缀，在同一原 T 且没有中间写入时保留相关数据库快照；
这是 A4 逐操作条件和实际自有只读保持的组合，不能由载体值相等推出。
SQL Err 后不保证 T 继续活动，也不提供随后读取仍有效的义务；真实调用者应按原首错返回。
reset/finalization、正常 Drop/rollback 和缓存后态仍分别按原合同处理。

## 本批有限消费范围

Create 线使用原 Inbox 两 BLOB UPDATE，随后原 Inbox SELECT。Unload 线复用已登记的
单行读取，并需要下列扫描；不增加或改写生产 SQL：

| 扫描 | 参数和顺序 |
|---|---|
| outbox `action_json,receipt_json`，按 sandbox 选择且 receipted | 原 sandbox 文本；原 ORDER BY id |
| outbox `action_json,receipt_json`，按 operation 选择且 receipted | 原 operation 文本；原 ORDER BY id，不能套 sandbox 条件 |
| source-work `state_json,sha256,work_key` | 零参数、原 ORDER BY id；NULL work_key、无行和空 BLOB 不同 |
| credentials `sandbox_id,generation,digest` 全表 | 零参数，原来没有 ORDER BY；不新增顺序，保留全部行和原重复摘要检查 |

每列值、损坏行、重复行和不相关行都交给实际自有回调；库模型不预先过滤为合法数据。
标量 i64/bool/String/BLOB、nullable 列及原 OptionalExtension 仍需各自固定 FromSql/
RowIndex 字典的符合性。局部字典 transport 可以由消费者提供，不能另造 SQL 世界或
把整个 source/current、凭据校验、JSON/摘要业务当作可信调用。

两次 Inbox load 一致性须由同一原 T、成功只读前缀、实际固定行选择和完整解码合同推出；
不假定两次 load 相等，也不由字段相等推导物理连接相同。对 ORDER BY id 的重复扫描，
同样需要原 SQL/快照条件；无序全表扫描没有“重复顺序必相等”的合同。
