# V3 事务录入与来源行的边界

### A4/A6/B4 StandardRecord 的逐符号 IO 接线

本条只细化[已登记的薄参数与结果接线](dependencies.md#逐符号可信适配登记)、[A6 JSON 与标准派生](dependencies.md#a6-json-与标准派生的最小合同)及[原 SQLite A4/B2/B4 条件](dependencies.md#a4b2b4-sqlite-提交回执与缓存发布接口)。实际调用者是 [`recording::StandardRecord`](../../controller-service/src/controller/lifecycle_inbox/recording.rs)，下一跳分别为 [`StandardConfigRead`](../../controller-service/src/controller/checkpoint/config.rs) 和 [`StandardInboxSave`](../../controller-service/src/controller/lifecycle_inbox/codec.rs)。以下每项单独采用源绑定条件，不把整个 StandardRecord、RecordEffects 字典或收件函数设为可信，也不称这些转调已获机器精化。

| 原 StandardRecord 方法 | 可以依赖的具体机械保证 | 调用者责任及限制 |
|---|---|---|
| `ReadConfigEffects::query_i64` | 原 tx 解引用所得 connection、原 SQL、空参数和同一 Row 第0列交给 `StandardConfigRead::query_i64`；仅原 Integer 原样转有符号 i64 | 不接受浮点或数字文本，不隐式转 u64；保留准备、参数、无行、索引和类型错误 |
| `ReadConfigEffects::query_bytes` | 同一原 connection/SQL、空参数、第0列；仅原 BLOB 的完整字节传回 | NULL、无行或错误不替换为空 Vec；SQL、表、行与原身份的选择仍属业务 |
| `InboxLoadEffects::query_bool` | 同一原 connection/SQL、空参数、第0列；Integer 零为 false，任意非零为 true | 不是仅1为 true；无行、NULL、类型错误与 false 区分 |
| `InboxLoadEffects::query_i64_pair` | 对同一原 Row 先取第0列，再取第1列，返回两个完整有符号 i64 | 第0列错误不读第1列，保留原首错和列顺序 |
| `ReadConfigEffects::decode_maintenance` | 原完整切片和同一借用的 connection 交给 codec D。生产 `StandardRecordCodec` 调用固定 `from_slice::<MaintenanceConfig>`；采用原字段、属性、字典及错误 | 不制造默认配置，不保证配置通过 `validate` |
| `InboxLoadEffects::decode_inbox` | 原完整切片和同一借用的 connection 交给 codec D。生产 `StandardRecordCodec` 调用固定 `from_slice::<Inbox>`，保留原完整 Inbox 和 Json 错误 | `DecodedStringsRepresentable` 仅由成功产生实际 Rust String 的 A6 可表示性提供，不含 Inbox 有效、授权或来源真实 |
| `InboxLoadEffects::encode_inbox` | 原完整 Inbox 和同一借用的 connection 交给 codec D。生产 `StandardRecordCodec` 调用固定 `to_vec::<Inbox>`，保留完整输出或原 Json 错误 | 原 schema、字段顺序、serde 属性、`required_option` 与自有字段回调必须逐项连接；不能任选模型 codec |
| `InboxSaveEffects::encode` | 保存方也调用同一个 `RecordCodec::encode_inbox`。生产实现调用同一固定 `to_vec::<Inbox>`；同一值在该固定纯实现正常完成时，与上一项编码一致 | 不推广到有状态 Serialize，不推出 UPDATE、恰一行或提交成功 |

结果包装保留原 `JournalError::Sqlite` / `Json`；外层失败、非返回及正常分配域的限制不变。以下三项沿用已登记的 SQLite 与 A6 合同。采用这些条件须绑定同一实际 owner、原 raw transaction/connection、原 namespace、完整参数和具体字典；纯值或标签相等不能代替物理来源连接。

| 原 StandardRecord 方法 | 采用的具体条件 |
|---|---|
| `InboxLoadEffects::query_bytes_pair` | 同一行依次读取第 0、1 列，保持两个完整 BLOB，首列失败时不读第二列。 |
| 两个 `digest` | 载入通过原 `StandardConfigRead`，保存通过原 `StandardInboxSave`。对同一完整输入按固定 SHA-256 合同传回全部 32 字节。 |
| `InboxSaveEffects::update` | `execute_inbox_update` 使用原 SQL、完整 BLOB 与 digest 参数。正常返回的 Ok 和 Err 都保留该次调用返回的事务值。原事务仍活动、原单元格未移位且无逃逸借用是额外的 A4/B2/B4 物理条件。 |

只有本调用实际为只读语句、原 snapshot/lease 及正常读框架条件成立时，才沿同一 T 的正常读前缀继续。SQL Err 不提供事务仍活动、数据库未变或旧观察仍可复用的保证；需要继续查或重试时另接真实后态。正常 Drop 的 rollback 另按其实际时刻的原 T 和 A4 解释。SQL 常量、预算、规范编码比较、load/save/record 决定、四项校验和提交次序仍由真实业务或上面的 `V5InboxTransactionFrame` 提供。

[V5 内部待证条件](dependencies.md#v5inboxtransactionframe具名内部待证条件不能转为外部信任)逐项是 `checkpoint::validate_candidate`、`lifecycle_inbox::codec::validate`、`capacity::validate`、`checkpoint::validate_transaction`。前三项读取该次调用的原 connection 与候选 World，第四项接收完整事务 owner。当前提交定理采用原顺序和各次实际成功返回，四个检查的业务规则、正常 Ok 与 Err 的帧和物理同源仍由下游履行。

### A6/B4 原始来源行的显式编解码与薄入口

[`capture::OriginalRowCodec<E>`](../../controller-service/src/controller/capture.rs) 的两个 `&mut self` 方法分别接收原完整切片，返回原 `WorkAction<EffectId>`、`WorkReceipt<E>` 或原 `serde_json::Error`。标准 `JsonOriginalRowCodec` 的方法只直接转调对应具体类型的原 `from_slice`；这两个标准方法的类型、完整参数、原字典和结果接线按既有 A6/B4 采用，不证明解析器内部，不假设解码出的动作、回执或 evidence 满足业务资格。

`original_source` 的可信薄范围仅为：保留原 transaction、namespace、World、sandbox 四个参数，构造原标准 codec，交给同一个真实 `original_source_using`，如实返回其结果。实际 using、`accept_original_source_row`、`agree`、闭包中的来源/身份/阶段检查、action 之后才可能 receipt 的顺序及最终两个观察的比较，必须由真实 Rust 抽取和对应证明覆盖；不把整个 handler 或 decode 后的业务判断纳入可信范围。Codec 的完整后态随原可变借用和 FnMut 捕获状态传递，不能每行重置，也不能用请求/封套 codec 替代这里的具体类型。

### B4 原 `accept_original_rows` 的 lazy 驱动

只采用 [`capture::accept_original_rows<I,F>`](../../controller-service/src/controller/capture.rs) 这一机械驱动：用给定原 Iterator 和 FnMut，先调用一次 `next`；None 返回 Ok 和当前完整业务 state，不调用 callback；Some item 原样交给 callback，包括 item 本身为 RowErr 的情况。每次先采用 callback 返回的完整 state' 和 F 后态，再判断其业务 Result；Err 保留原错误并停止，不再 next；Ok 才以更新后的 I/state/F 继续。Iterator、callback 的外层失败和非返回按原 Result bind 保留。

这不是先收集全部行再处理，也不允许首错后继续 SQL 或回调。MappedRows 的 SQL RowErr 可以跳过列 mapper，但仍作为 Some Err 到达原 outer handler。驱动不判断来源是否可接受，不默认 next/回调成功，不假设遍历有限、最终返回或数据库在 Err 后未变；真实 handler 的首错和逐行状态性质由其业务证明提供。已有原 `Std.loop` 的条件方程只证明所写模型的步骤，不使这个 Rust 驱动的 B4 符合性变成机器精化。

当前正常清理域中，原 iterator/closure 按原正常路径处理，statement 留在 outer 作用域；仅限本调用的标准 codec 和普通值载荷，不推广任意资源型 F 或自定义 Drop，也不从这些清理推出物理退出或提交完成。

### A4/A6/B4 来源查询的 SQLite 薄接口

本使用点限 [`original_source_using`](../../controller-service/src/controller/capture.rs) 的原 SQL、一个 Str 参数和两列完整 BLOB。共享表示复用原 Error、Connection、Statement、MappedRows F、Row、FromSqlError；实际 ValueRef/Value/ToSqlOutput 和 Transaction 保持原类型，不建立第二个连接、World 或 SQL 解释器。Statement 保留同次 prepare 来源、原行元数据和完整有序 raw 绑定输入；issued Row/MappedRows 同时保留原行及同次 Statement。

| 原 API | 保证与必须保留的结果 |
|---|---|
| `Connection::prepare`、`Transaction::deref` | 使用同一原事务的 connection 和完整原 SQL；prepare 保留原成功元数据或原错误，不凭模型标签证明对象相同 |
| 数组 `Params::__bind_in`、共享引用 `ToSql::to_sql`、`str::ToSql::to_sql` | 一个参数先检查该位置是否越界（越界不调用 ToSql），否则原 ToSql→raw bind，最后检查总数；共享引用调用原字典，str 是 Borrowed Text。完整 Null/Integer/Real/Text原字节/Blob 都交给 raw bind 观察，保留错误、非返回和返回 Statement 后态，不把其它变体默认成功或丢弃 |
| `Statement::query_map`、`MappedRows::next` | 使用原 Params 返回的完整后态，选择该次调用的 scan；next 保留 None、RowErr、正常行及原 F 后态；RowErr 不调用列 mapper，正常行才调用原 mapper |
| `Row::get`、`usize::RowIndex::idx` | idx 接同次完整 issued Statement；column 和 conversionError 来自保留的原 Row。先索引后转换，第0列错误阻止第1列，原错误不覆盖 |
| `Vec<u8>::FromSql::column_result` | 仅 Blob 返回原完整字节；其余原标签返回原类型错误，不把 NULL 当空 Vec，也不将 Text 隐式解码成 BLOB |

`NormalTextScanBinding` 是此单 Str 使用点的显式 A4/B4 正常接线条件：prepare/绑定正常后，原 SQL 与有序 Text 参数对应原 `ReadView.scans`，所选 trace 属于同次调用；正常行与 issued Statement 只要求 `SameIssuedMetadata` 和有序 `canonical.arguments` 对应。两份原值均保留，不要求整个 RowStatement 或未观察的 future SQL 行为相等。旧 Query 的参数投影不对 NULL/Real 或其它 Params 宣称完整；完整 raw 值仍进入真实绑定观察。只读、物理参数和原 snapshot 来源需由原 A4/B4 调用条件提供，不能将“查到想要的业务行”写进 adapter。

准备、ToSql、raw bind、next、mapper 均可失败或非返回；普通 SQL Err 不提供 active/frame，也不提供回调已经执行或没有执行以外的业务结论。原成功查询不保证后续调用成功；本条不证明 SQLite 引擎内部、SQL 选择、来源判断、同事务业务一致性或提交完成。

### A6 有限格式化与已证模块的链接

[`identity`](../../controller-service/src/controller/identity.rs) 在本来源切片只采用原四种格式：`{prefix}-{namespace}-{number}`、`tenant-{number}`、`op-{namespace}-{sandbox}-{operation}`、`sb-{namespace}-{sandbox}`。原 `Arguments::new`、Display 引用转调、str/有界整数 Display 和 `alloc::fmt::format` 按固定 A6 来源接线，保持全部原参数、分隔符、十进制输出、回调顺序和 formatter 后态；前项失败不调用后项，保留库的错误、失败和非返回。只覆盖这些实际有限程序，不推广任意 format/Display，也不承诺分配成功或称标准库源码已机器证明。选用哪个前缀、完整身份如何构造和是否合规则仍是自有业务。

`sandproof_protocol::valid_identifier` 是[已有真实函数对应](../../SandboxModel/Foundations/Identifiers.lean)及[原源码](../../sandbox-protocol/src/lib.rs)的模块复用，不新增“标识符总合法”的可信条件；本次调用仍须核对原函数、闭包和完整输入的实际链接。纯 `Option::ok_or` 仅按原标签作 Some→Ok、None→给定 Err 的有限值适配，不选择业务错误或假设 Some；本处只涉及三处实际普通无载荷错误值，错误参数按原调用顺序求值，资源型泛型 E 须另行登记。精确对齐采用 `lift(pure ok_or) = fixed Std ok_or` 的一般等式，不新增 Some 前提。已有模块链接、有限适配符合性和整个实际来源业务证明分别记录，适用版本、结果及剩余连接统一见[覆盖记录](../proof-coverage.md)、[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)与[#637](https://github.com/VeriInfra-Lab/sandproof/issues/637)。
