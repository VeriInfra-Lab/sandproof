# 协调器的生命周期数据连接撤销选择

[模块范围](../../../../docs/formal/03-protocol-inbox.md) · [收件查询值合同](../inbox-state/README.md) · [证明工作流](../../../../docs/proof-workflow.md)

`ControllerApplication::record_lifecycle_intent` 在应用锁内调用原日志。纯函数 `data_revocation` 使用传入意图的操作以及这次日志调用的完整 `Result`，返回无动作、撤销指定 sandbox、广播三者之一。调用者在同一锁内执行选择并返回原日志结果。原实现对照固定于 #1056 合入提交 `d642a5f45253a716dbb8411a9942b8b9581dd5fb`。

| 传入操作 | 原日志返回 | 决策 |
| --- | --- | --- |
| Unload、Destroy | 当前条目且存在本地映射 | 映射中的 `sandbox` |
| Unload、Destroy | 历史条目或缺少本地映射 | 无动作 |
| Unload、Destroy | `CommitUncertain` | 广播 |
| Unload、Destroy | 其余错误 | 无动作 |
| Create、Resume、Renew | 任意结果 | 无动作 |

## 实际对应

在包含 #1061 的 `integration@1f0f1b7e` 上，[#1095 的步骤](https://github.com/VeriInfra-Lab/sandproof/pull/1095)可重建调度及登记组合。引入本次检查器后，再由主控应用[检查点增量](parent-checkpoint-inputs-candidate.patch)。同一函数从两份实际 Charon 父方法重新计算锁、定向发送、广播及各自的检查器回归，并与原记录逐字段比较。读取器核对基线、候选提交和候选代码树。抽取和读回都从父方法及 `Inner::revoke_data` 的 Charon 声明找到各自的源文件，将文件路径和完整 Rust 源文同相应版本逐字核对。原记录缺少重构前的广播检查时，按原提交保留历史结果。

基线父方法内联了撤销选择。[旧分支检查](inline_binding.py)沿传入操作、同一 journal 返回的 `current`、`local.sandbox` 和错误变体逐条核对实际 Charon 分支。只有 Unload、Destroy 的 `CommitUncertain` 分支进入同一 guard 的发送表。基线和候选的定向调用、广播双层循环及 `send_replace(true)` 均经过同一个发送检查器。原入口在基线回执中记录六项检查器回归，涉及操作、当前标志、沙箱、错误、发送表和缺失广播体。主控在已采用 #1093 的组合上可应用[输入增量](inline-dispatch-inputs-candidate.patch)。

`check.py` 对基线和候选分别抽取完整父函数。`wiring.py` 从真实 LLBC 检查锁、日志参数及原结果返回，核对候选把传入 `intent.request.operation` 和同一个结果交给决策。三个返回变体分别对应空分支、`revoke_data` 的原 sandbox 参数、原锁下 `data_revocations.values()` 的双层循环与 `send_replace(true)`。这是结构化调用接线检查。固定 Aeneas 翻译完整父方法时停在 `ControllerApplication` 的函数指针标记字段。

`guard_binding.py` 跟踪两版父函数从 `self.inner.lock()` 经 `Result::map_err`、`Try::branch` 到 journal 的正常控制流。它按本次 `FnOnce` 实例读取实际错误闭包，检查其返回 `JournalError::ControllerBusy`。在 `Break` 边，父函数把该边的残余交给 `from_residual`，随即返回。在 `Continue` 边，后续 journal 和撤销分支引用的 guard 来自这次成功载荷，guard 的释放发生在分派之后。三项检查器回归分别改变 guard 来源、错误闭包的返回变体和残余实参。该检查记录标准库调用及其操作数，锁的运行时行为与这些标准库函数的实现语义仍需各自的依据。

`journal_hook.py` 核对两版父函数传给原 journal 的第四个实参。它跟踪新建的 `NeverCrash` 经可变借用和再次借用到实际调用，再核对该调用的泛型参数以及 `CrashInjector::hit` 的具体实现。联合抽取把相同实现接到生成的 Lean 方法，核对候选父函数与联合视图所用的完整 Rust 文件及方法起点。两份父函数分别按自己的源码核对。该性质只涉及应用传入的故障注入钩子，数据库和宿主仍可独立失败。

`sender_binding.py` 在同一份父函数 LLBC 中读取透明的 `Inner::revoke_data`，检查正常控制流和调用实参。

- 定向路径使用所选 `SandboxId.0` 查询原 `Inner.data_revocations`。查不到时直接返回。查到时遍历本次查询返回的向量，`next()` 为 `Some` 时，将这次取出的 sender 作为 `send_replace(true)` 的接收者，为 `None` 时退出循环。
- 候选的广播路径在原锁下取同一张表的 `values()`。外层每次 `next()` 返回的向量产生内层迭代器，内层每次 `next()` 返回的 sender 接收同一个布尔字面值。检查器分别核对两层循环的退出和继续位置。

检查器还在候选 LLBC 的副本上逐项断开查表键、查无结果时的返回边、定向 sender、广播向量四处连接，要求对应诊断命中。这些回归只检验检查器自身。这里的证据止于 Rust 实际发起 watch 调用的位置。固定 Aeneas 单独翻译泛型 `Inner::revoke_data` 时停在 Serde 的相互递归 trait 组。迭代库语义、异常退出以及接收端观察均需独立证据。

检查器在私有源码副本复用收件包的 `proof_find` 和 `proof_state`，与决策和具体故障钩子的薄根一起抽取。固定 Charon/Aeneas 在同一视图生成 `Inbox::find`、`Inbox::state`、`data_revocation` 和 `NeverCrash::hit`。查找、查询和决策的转发根均按顺序传入原参数并返回本次调用的值。

- `never_crash_hit` 对任意 `CrashPoint` 及接收者证明，实际选中的 `hit` 返回 Rust 的 `Ok(())` 并保留接收者。
- `success` 和 `failure` 分别对全部成功值和完整错误枚举给出精确决策。
- `from_indexed_state` 在两个有效索引上把原查询接入原决策，以条目接受版本、会话当前版本及其本地映射表述结果。
- `other_version` 推出任何版本不相等的条目均选择无动作。这里的操作来自父调用的入参；日志返回的 intent 仍是独立字段。
- `replay_decision` 从实际全文查找返回的两个索引出发。原 `find_spec` 与 `inbox_found_reads` 给出命中会话、条目和完整意图，继而得到原查询的全部字段和确切撤销选择。任意重复键和会话均沿用原先遇到的查找结果。
- `historical_replay` 证明命中条目的接受版本与该会话当前版本不同时，选择无动作。`lookup_conflict` 证明全文查找遇到冲突时返回 `Conflict`，该错误对应无动作。`mapped_unload_replays` 在同一份带本地映射的会话中给出旧 Unload 与当前 Unload 的实际查找和选择见证。

本片从已解码收件值及进入查找的完整意图开始。调用前的认证、未命中后的写入和事务仍有各自的合同。日志事务的实际行来源、未知提交的耐久结局由 V5 提供。真实 watch 送达、网关撤销与在途 I/O 收尾由数据通路提供。上表描述此入口作出的选择；任何成功返回均不能单独说明实际连接已经断开。互斥锁、进程取消和错误载荷沿用[逐符号依赖](../../../../docs/formal/dependencies.md)。

## 从公开 journal 进入事务

基线和候选的协调器抽取都包含真实的 `ControllerJournal::record_lifecycle_intent`、`record_transaction_using` 和事务提供方的原 `proof_record_actual_parent`。检查器只在解包源码中追加提供方冻结的薄根，按同一份类型化 LLBC 核对下列正常控制流。

- 公开方法把本接收者的 `journal.connection` 与 `committed_world` 交给 `WorldTransaction::begin`。若结果进入 `Break`，它用本次残余和标准库同型 `From<JournalError>` 转换返回。该边不取命名空间，不构造 `StandardRecord`，也不调用录入。
- 成功边读取本接收者的 `namespace`，用本次事务、解引用后的命名空间和 `StandardRecordCodec` 构造 `StandardRecord`。它将原 intent、认证上下文和可变故障钩子按序传给录入函数，并直接返回该次调用结果。
- 两个调用点使用同一个录入定义和同一个 `StandardRecord` 事务实现。公开方法选择实际的 retirement、terminal、renewal、unload、graphic 五个实现和具体 codec。提供方薄根在相应位置传入自己的类型与字典参数。五个检查器回归各自断开一处操作数、错误返回边、构造字段、编码类型或提供方参数，均要求具体诊断命中。

候选还以四参数转发薄根单独抽取公开 journal。固定 Aeneas 为公开方法、该薄根、通用录入和原提供方薄根生成透明方法体。入口核对该视图与协调器视图的源码和调用选择，保留生成体、映射及外部符号清单。此视图的验收层级为 Rust 抽取与 Aeneas 翻译；下节的内核定理仍在已开始事务的原提供方工程检查。

与原事务视图相比，公开视图多出 12 个外部函数和 8 个类型。接入 Lean 时需要以下逐符号连接。

| 提供方 | 需要连接的内容 |
| --- | --- |
| V5 事务与应用状态 | 将 `WorldTransaction::begin` 的连接、`CommittedWorld`、成功事务与返回连接接到原录入证明的 owner 表示。接收者还携带 `ControllerLease`、`HostEpochObservation`。 |
| V0 字符串、V3 编码 | 连接 `String::deref`、`StandardGraphicBytes::all_graphic`、retirement 的 `digest/encode_intent/encode_receipt` 及 terminal、renewal、unload 各自的 `encode`，并落实五个具体实现类型。 |
| V3 收件、V5 持久化 | 将 `StandardRecordCodec` 的 `decode_maintenance/decode_inbox/encode_inbox` 接到各自现有值与字节合同。 |

## 从已开始的事务取得重放返回值

原事务证明的输入包含实际查找结果、两个机器索引和查询返回值。这里在该提供方自己的 Lake 视图中复用 `actual_root_replays_full_authenticated_request`、`InboxAdmission.accepts_iff` 和完整查找、状态合同，从载入后的值推出这些输入。父函数接收已经开始的原事务、命名空间、codec 和故障钩子。

- `JournalReplayState.retained_state` 适用于任意已载入收件箱。若配置与认证上下文匹配、封套有效且全文查找规范首次命中，实际父调用返回该会话和条目构成的完整工作状态以及原故障钩子状态。其 `current` 精确比较入参接受版本与命中会话当前版本，`local` 取自同一会话。定理内部用实际查找取得索引及界限。
- `JournalReplayState.refused_before_commit` 覆盖配置或封套拒绝，以及二者通过后查找遇到冲突。实际父调用在相应分支返回 `Conflict` 和原故障钩子状态。认证拒绝对收件箱的查找结果无任何要求。
- `JournalReplayState.same_key_capture_replies` 用原查找包的两份合法 Unload 封套作对照。同一来件遇到完全相同的存储条目时返回完整状态，仅改变存储的 capture 而保留键时返回 `Conflict`。每个分支只要求该次实际载入自己的样例。

这三条定理在原事务提供方的生成类型上检查；上节协调器的查找与决策在自己的联合抽取中检查。两处共同使用原 `InboxLookup` 与 `InboxState.workState` 规格。载入结果、字节可表示和 Graphic 字符检查沿用提供方的具名条件。实际 BEGIN、自动析构和物理回滚仍交由事务提供方。

## 载入阶段的首个拒绝如何返回事务调用者

`JournalLoadExit` 从原 `codec::load_using` 的实际返回或它发出的查询开始。定理同时给出两层可观察结果：`record_using` 返回出错时的 effects 状态，外层 `record_transaction_using` 返回同一错误和传入的故障钩子状态。载入失败和空值都会在认证、查找及提交前退出。这组结论对任意编码字典、事务实现和故障钩子成立，无需字符规则或解码字符串可表示的前提。

- `load_exit` 覆盖任意带载荷的普通 `JournalError`，并用原提供方的双向错误转换保留完整载荷。载入正常返回 `Ok(None)` 时录入返回 `UnsupportedStore`。`standard_record_reply` 将这一结论接到使用原事务 owner、namespace、codec 的 `proof_record_actual_parent`。
- `first_header_error` 覆盖检查收件表、读取版本、检查来源表，以及在当前布局上检查内部表的四个查询。每个分支只要求失败前缀的真实调用观察。下一次查询使用上一次返回的状态。
- `header_layout` 覆盖前三个查询均成功且布局提前退出的全部组合。版本与当前值不等且收件表、来源表均缺失时，录入返回 `UnsupportedStore`；其余提前退出组合返回 `CorruptStore`。当前格式且两表存在时继续查询内部表，`unexpected_internal_table` 证明若它存在则返回 `CorruptStore`。

表是否存在、版本值及查询错误来自本次事务的 effects 观察。SQLite 提供方仍负责说明物理库怎样产生这些观察。上述返回性质覆盖 Rust 的普通结果，析构和物理回滚按 V5 合同验收。

### StandardRecord 在同一存活事务上读取表头

`JournalHeaderReads` 将上面的查询条件接到实际读取实现。前提 `before.tx.parent.transaction = some raw` 标识本次录入持有的原始事务。布尔和版本读取沿原两次解引用到达 `raw.conn.base.reads.current.single`，在这份连接观察上使用原 SQL 和空参数。两条读取定理对任意正常返回的 SQL 成功值或错误成立，错误携带原 SQLite 载荷。借用结束后返回完整原 `StandardRecord`，包括命名空间、codec、事务及 commit/terminate 观察。

- `first_prepare_refusal` 只需第一条检查收件表的 SQL 在 prepare 时返回错误。实际录入体交回原 holder，外层事务交回同一个 SQLite 错误与原钩子。该错误的构造为 `Sqlite`，在协调器的撤销选择中属于无动作分支。
- `read_layout` 使用同一连接上的布尔、整数、布尔三个查询结果，覆盖所有在第三次查询后退出的布局。版本是否与当前值相等，以及两张表的存在位，共同决定 `CorruptStore` 或 `UnsupportedStore`。

这里复用 V0/V5 的 `queryBool`、`queryI64`、`queryRow` 和原 `StandardRecordIO` 调用合同。SQL 引擎如何产生这些观察、查询错误后的物理事务状态及 BEGIN 到 `raw` 的对应继续由 V5 提供。

本片复用[原 BEGIN 证明](../unload-parent-link/world-begin/UnloadWorldBegin.lean.in)的 `begin_success_keeps_original_owner`。其[抽取入口](../unload-parent-link/world_begin_capture.py)所列 `Cargo.lock`、`transaction.rs`、`allocators.rs`、`checkpoint.rs` 在提供方基线 `f26fc26a` 和 `a6f6b6b9` 的内容逐一相同。原 BEGIN 构建的 40 个 Lean 输入中，34 个已在同一生产版本的录入工程逐份匹配，其余 6 个属于 BEGIN 和 started-transaction 视图。

### 从 BEGIN 的成功结果连接原录入父调用

`JournalStartedHeader` 使用实际 `WorldTransaction::begin` 产生的事务。输入是原已提交 World 的读取结果、写入准备、`BEGIN IMMEDIATE` 返回的连接与 raw 事务、该事务的分配器上界，以及在它上面读到的匹配版本。结论保留 BEGIN 返回给 journal 的连接。录入时从 raw 事务读取，二者的角色各自保留。

`erases_to_entire_owner` 给出完整字段表示的见证。它只补入原录入视图要求的 commit、V5 调用和 terminate 观察，投影后恢复全部原 owner。`begin_to_first_prepare_refusal` 对每个投影等于本次 BEGIN 返回 owner 的录入调用者成立。它从完整投影推出活动事务槽和同一个底层连接，第一条 SQL 的 prepare 返回错误时，实际录入体交回原 holder，事务和原 `proof_record_actual_parent` 均交回同一 SQLite 错误及原钩子。

[物化与检查](journal_started.py)复用原 `stage1-unload-parent-transaction` 的 `begin` case。原 BEGIN 入口先核对本候选的抽取、类型化调用及源码，随后将联合页加入该 case 的构建和逐文件服务清单。录入一侧使用原事务提供方的同一生成类型、源绑定和表示，两个提供方共享的 Lean 输入必须逐字节相同。主控须在同一候选收齐原录入抽取与该 BEGIN 结果。V5 仍提供真实物理对象对应及 commit、terminate 的观察；公开 journal 如何选定调用者、具体 codec 与网关效果继续按各自入口验收。

[事务包登记候选](journal-provider-candidate.patch)以 #1061 的 `de37231d` 为补丁基线，供主控在叠入本生产版本的候选上接入。补丁将重放、载入首错与实际表头读取三页加入现有事务入口的构建根与逐文件检查清单，用 `current_file_overrides` 记录本版协调器，保留原冻结。事务独立入口的既有输入漂移仍需按各自提供方回执核对。

### 公开收件使用的两个标准解码器

[`DecoderCalls`](codec-read/DecoderCalls.lean.in) 把生产 `StandardRecordCodec` 的两个读取方法接到原事务的 `CodecTransfer` 定理。[接线检查](codec_binding.py) 从公开 journal 选择的具体字典读取方法槽，再在独立抽取中核对同一方法的源位置、完整切片指针和长度、派生字典、JSON 函数及返回槽。Aeneas 从这两个真实方法生成透明函数体。检查器拒绝将切片长度换成连接参数，或用维护配置的派生类型解释收件箱。

两条定理对任意编码方法、任意传入字节及任意存活事务成立。若相应的 `from_slice` 调用返回普通 Rust 结果，实际读取将成功值或完整 JSON 错误交给原 `mapJson`，并交回整个原 holder。表示沿用事务提供方的连接、配置、收件箱与 JSON 错误。固定 Serde 库及两个派生处理器按[逐符号合同](../../../../docs/formal/dependencies.md)提供，具体字节怎样解成字段仍需该提供方的证据。三个方法一同翻译时，编码路径在 Serde 的递归 Serialize trait 组停止。

[解码视图](codec_reads.py)使用单独的 Lake 构建目录。检查器逐项核对事务提供方的 Rust 输入、原模块清单和实际可达导入。生成类型页逐字节使用，函数页仅由工具引入三个外部调用使用的字典参数，两页原件均保留。ning 复用 `stage1-create-transaction-link` 的原生入口，其中的 `record_codec_keeps_full_inputs_state_and_first_error_on_the_original_connection` 在实际 SQLite 连接上观察调用参数和故障分支。

### 公开 journal 的 BEGIN 首错和成功续调用

`PublicBegin` 从生产 `ControllerJournal::record_lifecycle_intent` 开始。World 读取、写入准备、SQLite `BEGIN IMMEDIATE`、分配器读取中任一步正常返回错误，都只需给出到该步的调用观察。原提交版本变化时，在已有未借用条件下返回 `CorruptStore`。每个结论给出完整返回的 journal 和故障钩子。连接取自当前失败步骤或已经开始的事务，命名空间、memo、活动 host epoch 及包含文件字段的原 lease 均保留。定理对该文件句柄的值和类型均参数化。V5 负责操作系统文件及锁的实际身份。

BEGIN 成功后，`started_reply` 连接该次返回的原 owner、实际命名空间借用及完整 intent/context，经公开方法选中的字典进入原录入事务。它保留录入的完整结果、完成后的故障钩子及 BEGIN 返回的连接。`first_record_read_refusal` 复用已有事务 owner 的完整投影和原始首条 SQL 的 prepare 拒绝，推出公开方法返回同一 SQLite 错误和原钩子。录入前的首错不要求后续编码、提交或发送的结果。成功 owner 的额外 V5 调用观察可由现有 `callerOwner` 构造。具体解码复用上节的真实方法。四类标准编码和其余写入观察保留为逐调用参数。

[证明页](public-begin/PublicBegin.lean.in)、[表示和调用](public-begin/PublicCalls.lean.in)及[物化检查](public_begin.py)在主控登记候选中接入原 `stage1-unload-parent-transaction` 的 `begin` case。公开 journal 的抽取包含五个实际标记类型及 `ControllerLease.file`，并保留原录入提供方的全部生成声明。两个生成视图、原 BEGIN 和原有各页在同一隔离工程按完整导入闭包检查。ning 仍在同一集成候选收取原事务和协调器的原生回执。持久化、文件锁和 watch 的真实送达各由其提供方验收。

### 从公共收件结果到锁内撤销决定

从原 `WorldTransaction::begin` 成功返回、公开命名空间借用以及同一 owner 的 `SourceFlowLink.loadPrefix` 观察开始，`PublicReplay.retained` 将首次全文命中的完整工作状态直接交给生产 `data_revocation`。当前性由所选 session 的版本决定，本地映射取自同一 session。旧版本 Unload 保留现有映射，当前版本 Unload 选择其中的 sandbox。合法封套但同键异 capture 的请求，以及认证或封套不匹配的请求，均返回 `Conflict` 且无撤销动作。两条具体见证沿用已有输入，逐一检查公开返回和选中的动作。对其它正常错误返回，只有结束访问的操作遇到 `CommitUncertain` 时选择广播。[组合定理](public-begin/PublicReplay.lean.in)

同批重抽协调器父函数，核对同一锁内的 journal、原 intent/context、实际 `NeverCrash`、传给决策的 operation 和返回值。Lean 使用原决策与钩子方法的完整生成函数页，以及原动作和钩子类型的完整生成类型页。四个共享类型逐构造器、字段和载荷与事务提供方核对，错误沿用双向表示。原 `CoordinatorRevocation` 证明在这个共同环境重新检查。固定字符判断由实际标准实现履行。[抽取与接线检查](journal_decision.py)

载入的普通结果、解码字符串可表示性、V5 的事务观察和受信认证来源仍是具名前提。自动析构和物理回滚、watch 通知送达及网关路由关闭保留原验收分工。ning 在同一个集成候选运行 Linux 完整普通检查，核对下列两个原生断言组与父事务和收件组件回执。

### 直接消费 Rust 载入返回

`PublicLoaded` 采用 `JournalLoadExit.originalLoad`，即 `record_using` 在本次 BEGIN 返回 owner 上实际调用的 `codec::load_using`。调用结果包含完整收件箱以及当步交回的 holder。达到此处后，录入先检查认证，再按首个完整匹配读回工作状态。即使旧代映射指向当前 sandbox，撤销选择也使用该次读取的当前位。认证不符或同键异文时保留 `Conflict`，空库返回 `UnsupportedStore`，载入返回的其它普通错误保留载荷。成功重放时，录入体交回达到载入出口时的完整 holder。[Rust 载入返回的对应](public-begin/PublicLoaded.lean.in)

这一入口适合已经持有实际 `load_using` 返回证据的提供方。使用 SQL 或协议模型的载入观察时，原入口继续要求解码字符串可表示等合同，以便解释数据库内容如何产生这次返回。生成的录入与加载体沿用事务提供方冻结的完整 Request 视图。物理事务和网关通知仍按原分工验收。

### 首次写入的 Unload

同一次 BEGIN 和原 Rust 载入返回后，若认证通过、整个收件箱没有该键、首个同 ID 会话通过现有追加规则，`PublicUnload.fresh_unload` 给出原 `Inbox::record` 返回的新收件箱和索引。原 `state` 在该位置读出的完整 intent 与入参相等，当前版本就是本次接受版本，映射来自同一会话。原保存函数收到这份新收件箱和载入结束时的 holder。保存报错时可直接推出公开错误和撤销选择，无须提供提交观察。保存成功时，实际标准字典提交保存返回的那个事务对象。提交成功选择该会话映射的沙箱，缺少映射时无动作。提交不确定时选择广播，其余提交错误保留原载荷且无动作。[首次录入证明](public-begin/PublicUnload.lean.in)

正常见证复用原追加组件的合法 Unload、已有运行会话及映射到沙箱 10 的记录。证明给出实际追加后的状态。保存与提交的普通返回仍由 V5 提供，网关信号的送达仍由数据通路验收。ning 的同一原应用回归先检查旧请求重放，再首次录入后继 Unload。[原生交接](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-7227746008)

### 首次 Unload 保存后的 SQLite 提交

在上述合法追加和保存成功的前提下，`fresh_sqlite_outcome` 使用保存返回的同一个事务 owner、其中的原始 SQLite 事务及其候选 World。V5 依次校验候选 World、收件箱、容量和事务。`commit_validation_error` 只要求到首错为止的调用结果。公开录入保留该错误的载荷，Unload 遇到 `CommitUncertain` 时选择广播，其余错误无动作。这条分支无需后续校验、底层提交或状态单元的观察。[证明与审计](public-begin/PublicUnload.lean.in)

四项校验均通过、已提交状态单元可借用且观察满足 V5 的 `PhysicalLink.LinkedCommitted` 时，底层 SQLite 提交有两种普通返回。成功分支还要求候选单元可借用，此时公开录入返回追加后的完整状态，选择同一会话映射的沙箱（无映射时无动作），已提交状态单元可观察到原候选值。错误分支保留 SQLite 载荷，返回 `CommitUncertain` 并选择广播，已提交状态单元可观察到 `none`。数据库此时的实际持久状态仍未知，网关送达由数据通路验收。

主控可先按 [#1088 的公开组装步骤](https://github.com/VeriInfra-Lab/sandproof/pull/1088) 采用前序登记，再随本页与 `public_begin.py` 应用[本次增量](sqlite-outcome-integration-candidate.patch)。这会给原页面增加两项审计。五个把登记器列为输入的组件需要在选定的最终提交上复验。

### 保存后提交未正常返回

`recorded_commit_dispatch` 从同一次真实录入返回的 `Recorded` 工作状态和完整 holder 开始。实际事务提交消费这个 holder 的父事务。提交正常返回时，公开 journal 保留其错误或工作状态，协调器把同一个结果和原操作交给 `data_revocation`。[证明与审计](public-begin/PublicUnload.lean.in)

`fresh_sqlite_dispatch` 将上述连接用于已认证、原库无键、现有会话允许追加且已成功保存的首次 Unload。在 V5 的四项校验通过、提交状态单元可借用的条件下，底层 SQLite 调用正常报错会保留错误载荷并选择广播。成功还要求候选状态单元可借用，随后有本地映射就选择本会话的 sandbox，没有映射就选择 `None`。两条决定都可从当次返回推出。若底层调用在 Aeneas 语义中返回 `.fail err` 或 `.div`，公开 journal 与撤销选择的组合分别返回同一错误或发散，调用方取不到撤销动作。该分支无需候选单元的借用条件。原提交状态单元的物理读回继续使用 `fresh_sqlite_outcome` 的 V5 观察前提，实际数据库持久性和网关送达由各提供方验收。

主控在已采用 #1090 登记的组合上使用[提交返回的增量登记](commit-dispatch-integration-candidate.patch)。同一页新增两项审计。ning 在最终共享提交沿用父事务、收件与应用回归的原生回执。

### 首次 Unload 的保存校验与编码首错

`fresh_save_refusal` 从本次 BEGIN、完整载入和合法追加出发。真实保存先校验刚追加的同一收件箱。
若校验返回业务错误，保存保留载入结束时的完整 holder，公开 journal 保留错误载荷。此分支的
撤销选择由原错误类别决定，无需假设有可借用的 SQLite 连接，也无需提供编码结果。

校验通过后，`save_encode_returned` 将 `StandardRecord` 的保存编码调用连接到同一存活事务的
连接、完整新收件箱和公开方法所选的编码字典。若编码返回 JSON 错误，公开 journal 返回原
`Json` 载荷。若编码成功且实际字节数严格大于新收件箱配置的预算，返回 `MaintenanceCapacity`。
两者均保留该 holder，协调器选择 `None`，证明只需到该首错为止的调用结果。等于预算会进入
后续摘要和更新步骤。具体 Serde 编码的字节语义、SQLite 持久化和网关送达分别由其提供方验收。
[实际生成入口的证明与审计](public-begin/PublicUnload.lean.in) · [主控登记增量](save-encoding-integration-candidate.patch)

### 首次 Unload 的保存更新和事务返回

首次 Unload 通过追加、新收件箱校验和编码后，`bounded_save_result` 处理编码长度小于或等于配置预算的路径。配置预算上界保证这条路径落在现有 SHA 库合同的正常长度域。借用同一存活连接的标准保存方法把全部编码字节送给摘要方法。实际更新语句的两个 BLOB 参数依次为这些字节和完整的 32 字节摘要。在当前 SQLite 适配合同下，每次正常返回均带回此次写入后的读视图。对应定理保留 owner 的其余字段。[保存更新证明](public-begin/PublicUnloadSave.lean.in)

`fresh_write_dispatch` 从同一次公开 BEGIN、载入和已认证的 Unload 得到追加后的收件箱与完整状态。SQL 报错时，协调器保留原 `Sqlite` 错误并选择无动作。更新零行或多行时返回 `CorruptStore` 并选择无动作。这两项无需提交观察。恰好一行时，原提交接收本次写入后的 owner，公开结果和撤销选择沿用该提交的实际返回。编码的具体字节由所调用的方法提供。SQL 调用的回执与后续物理耐久性由 V5 提供，接收端是否收到撤销仍由网关验收。[提交与撤销证明](public-begin/PublicUnload.lean.in)

主控在应用保存编码的登记增量后，采用本段的[登记候选](save-update-integration-candidate.patch)。完整事务作业通过 `PublicUnloadSave` 引入先前的拒绝页和本段的四项审计。

### 首次 Unload 在追加前被拒绝

从同一次 BEGIN、命名空间借用和真实 `load_using` 的完整返回开始，若信封与配置通过认证、整个收件箱没有该键且存在同一会话，录入会选择首个同会话行。`fresh_refusal` 使用原 `record_existing_contract` 的有序判定。该判定拒绝 Unload 时，`Inbox::record` 返回原收件箱，`record_using` 交回载入结束时的完整 holder，公开 journal 返回相同的首错，协调器选择不撤销。这个分支只可能返回 `Conflict` 或 `UnsupportedInput`。证明无需提供保存、提交或通知的执行结果。[首错与协调器决定](public-begin/PublicUnloadRefusal.lean.in)

`mapped_guard_order` 固定同一个合法 Unload、映射到沙箱 10 的同一个会话和不匹配的执行代。容量为 2、已有一条记录时，保留给 Destroy 的最后一个位置先触发 `Conflict`。容量为 8 时，到达操作形状检查并返回 `UnsupportedInput`。两次调用的撤销选择均为 `None`。真实锁和网关信号的效果沿用原应用验收分工。

若整个收件箱既没有该完整键，也没有请求指定的会话，`missing_session` 复用 `record_new_session_contract`。按原 Rust 的 `freshDecision` 顺序，它必然拒绝 Unload，并把原错误带回公开 journal。收件箱和载入结束时的 holder 均保留，撤销选择是 `None`。`missing_session_guard_order` 固定一个合法 Unload，分别改变载入值的窗口和占用情况。窗口不匹配总是先报 `Conflict`，即使槽位已被另一会话占用。窗口匹配后，已占用报 `MaintenanceCapacity`，可用槽位才到达操作检查并报 `UnsupportedInput`。该结论从本次载入的返回值开始，物理收件箱及网关观察仍由提供方验收。[查无会话的实际录入和四个见证](public-begin/PublicUnloadRefusal.lean.in)

已经按 #1081 和 #1083 组装协调器登记的候选，主控可在引入本页及 `public_begin.py` 后应用[首错证明的增量登记](refusal-integration-candidate.patch)，一次纳入一页、五项审计。已经采用 #1087 的组合使用[从 #1087 补入两项审计的增量登记](missing-session-integration-candidate.patch)。五个组件把登记器本身列为输入，分别是 `digest-uses`、`record-reply`、`standard-blob`、`standard-digest` 和 `standard-save-digest`。主控采用这一版计数时，须取得这五个读取器对新登记器的复验。若同批采用 `protocol-record` 的协调器输入调整，先处理该组件，再应用增量登记。[组装基线](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-7231590235)

后三个读取器有内部依赖。先验收 `standard-blob`，再处理读取其清单的 `standard-digest`，最后处理同时读取前两份清单的 `standard-save-digest`。

## 复验与登记候选

在证明机干净候选上使用固定工具和现有重锁，给出新的私有输出目录。

```bash
SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1 \
  bash scripts/python-entry.sh verification/experiments/stage1-protocol/coordinator-revocation/check.py \
  --tools "$proof_tools" --packages "$fixed_lake_packages" --lock "$shared_proof_lock" --output "$new_output"
```

`--tools` 下应有原 `aeneas` 与 `lean-mcp`。入口从 Git 解包源树，保留五份 Charon 产物、公开 journal、具体解码器和联合视图的 Lean 生成体、隔离的 Lake 构建、实际文件诊断和审计。联合视图的两份 `*_Template.lean` 只由翻译器输出，实际联合工程使用已登记的五个函数绑定和四个类型绑定。

交主控登记这个形式入口及 `SandboxModel.Lifecycle.Protocol.CoordinatorRevocation` 的九条审计。覆盖记录可登记“在已解码 `Result` 上证明应用撤销选择，全文重放命中给出实际查询和选择，原父函数接线经实际 Charon 核对”，适用版本取通过对应检查的候选。生产协调器是现有原生输入，相关登记与冻结由主控结合 #1061 逐闭包复核。

[公共登记候选](registration-candidate.patch)供主控在包含本包的候选上审阅、试套。[覆盖记录候选](coverage-candidate.patch)注明已实测的源码和边界。登记通过 `scripts/verify-stage1-protocol.sh` 默认使用的 `full` 计划调用同一检查器。其原生前提包括已封口的完整 Linux `functional` 日志中的两个指定测试，以及原 `stage1-inbox-lookup`、`stage1-inbox-state` 和 `stage1-create-transaction-link` 收据。单项命令和缺少封口的日志可用于开发复核。

[默认入口的输入候选](inputs-candidate.patch)列出相对 #1061 新增的全部路径以及原 `unload_audits.py` 的更新。它覆盖协调器、录入、BEGIN 所用的生产源码、证明、薄根、原始抽取与物化入口。事务独立清单一并列出这个共享检查文件的候选内容。主控还需核对既有公共工具冻结以及同一候选的原生封口。

登记子任务在分配的四核内继承并核对原协调锁描述符。检查点读取五份指定 LLBC、公开 journal 和具体解码器各自的五份生成产物、真实内核命令的退出状态与日志、物理文件和原服务回执。收敛与 Create 有限计划继续按各自的原任务清单验收。主控合入时需重新校验受影响的公共冻结和完整原生输入。

ning 在同一候选运行普通完整检查，核对现有 `historical_unload_keeps_resumed_route_current_unload_revokes_it` 与新增 `uncertain_commit_selects_broadcast_only_for_access_ending_operations` 两个原生断言组。前者通过原应用和受控端口观察实际通知，后者覆盖不确定提交和普通拒绝的决策边界。完整公共入口和实际网关分别验收。
