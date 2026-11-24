# Unload 父调用、来源行与观察请求的连接候选

对应 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。本包固定
integration `c265145f5614535c068c464bc37b6162edcdc45c`，复用 #768、#770–#774
的原规格及实际生成视图。生产 Rust、公共 Lake/Cargo、根导入、依赖合同和覆盖冻结均未改。
这是独立的证明开发候选，普通与原生验收交 ning，公共登记由主控协调。
[#776](https://github.com/VeriInfra-Lab/sandproof/pull/776) 负责四批上游包的公共接线，
本包没有重复实现它，也没有修签历史 continuation 输入或将新捕获冒充它要求的旧 LLBC。

## 证明与表示

- `SourceParentLink` 将 #770 的真实 `original_source_using` 接到原父调用的
  `readOriginalSource`。两者直接使用同一生成的完整 World、transaction、namespace、
  sandbox；`.conn` 原样提供给后续选项和凭据位置。成功返回复用原
  `actual_source_success_complete_contract`，保留同游标来源、完整行字节、字典后态、
  两槽、命令、身份和 HostEvidence 检查；不重新假设业务行处理器正确。
- `OptionsParentLink`、`FirstParentReadback` 安装 #774 的实际读函数和选项编码函数。
  原 SQLite 行的完整重编码必须等于原 BLOB；正常首次父调用保留该 BLOB 和完整摘要输入，
  并得到原完整观察工作请求。缺行/default 的原分支仍在上游合同，BLOB 定理限于原行存在。
- `SharedParentCarriers`、`SharedInboxCarriers` 实例化原 transaction、World、请求与错误
  连接。共享值字段的类型等式由 `rfl` 给出。独立生成的 ContractError、CrashPoint 和
  ControllerReject 使用完整构造器变换，双向定理覆盖嵌套 payload，不假设不同归纳类型相等。
  `views.json` 只将原手写 Inbox 桥的三项错误类型等式改为具体变换，原文件保持不变。
- `SameParent` 证明 Inbox/base/startup 与上述实际 source/options 安装作用于同一个
  完整调用字典，实例化 #772 的原 World/record/ledger/sealed-plan 结果；原定理的
  native、ledger、codec、读快照和 Host 条件保留。`LiveParentLink` 安装 #768 的真实 guard，
  精确保持其接受与拒绝合同。未读取字段的代表值只在 #768 已证的不相关字段范围内使用。
- `ContinuedParentLink`、`ContinuedSameParent` 把 #773 的实际 `ready_records` 装入同一
  父调用，覆盖 Err/None/Some，保留原完整工作值、回执、同连接/World、全部 native 字段。
  动作和 native 的结构变换均有完整往返定理。`ContinuedParentSuccess` 将原父函数的
  继续分支接到完整 question，保留原动作/回执前缀并准确追加一次。
- `ParentErrors`、`ParentRejects` 按原顺序证明已选前缀后的拒绝：查找/缺项、非当前/终局、
  已解决项、非 Unload、缺 bootstrap/active/本地绑定/执行来源、base/缺 native/历史缺失或
  不匹配/live、source/继续读取缺项或错误、来源 boot/pid/ticks、startup、options、credential
  缺项/错误/代不符。每个定理只要求到该首错为止的成功读取，不要求后缀成功。
  `NoQuestion` 对任意最终完整 anchor 编码回调保持同一错误；该回调即使不返回也不会影响结果。
  这不声称此前读操作没有效果，不把坏索引的 panic 解释为 JournalError。
- 两个 JSON 错误位置分别检查。选项编码失败跳过最终 question；anchor 编码失败保留
  原 JSON 错误，并在 question 内跳过最终 digest。该结论不消除之前已经执行的 options digest。
  `JournalErrorFromJson` 使用当前真实生成体，错误 payload 原样保留。

`UnloadParentLink` 汇总本包 86 项依赖审计；这些包含类型变换和辅助引理，不能计为
86 个独立业务保证。`prepare.py` 从原带哈希清单装配 279 份实际输入，不重复搬迁上游算法。
原 raw Types/Funs 保存在本包生成目录，类型 helper 的可逆 metadata 隔离及外部薄绑定
单独登记；生成函数体不编辑。`LiveDiscriminants` 使用固定后端的原 helper 生成定义，
仅给实例明确命名，解决多个生成视图的全局辅助名冲突，不改变判别值或工具。

## 依赖与范围

依赖只采用[逐符号合同](../../../../docs/formal/dependencies.md)及
[V3 来源接口](../../../../docs/formal/v3-transaction-source-boundaries.md)。
每个参数是原调用的具体接口实例，不能用“整个 reader 正确”代替业务证明。

| 未由本包消解的条件 | 提供方及限制 |
| --- | --- |
| 原 SQLite 读快照、issued statement/row、同一物理事务及正常读框架 | A4/B4 和 V5；生成句柄同一不证明 BEGIN、提交或错误后仍 active |
| 具体 codec 的完整字节/原值、标准 JSON/克隆/字典/有限循环接线 | 已登记 A6/B4；保留全部原成功、错误与非返回行为 |
| 原 HostEvidence/ResourceEvidence 与真实宿主事实对应 | V6；VM 记录或相同 PID 不证明 OS 存活、Stop 或暂停许可 |
| 独立认证上下文与完整原意图来源 | V7、V1/V2；从已解码值开始，未证明 JSON/TLS 全链 |
| 原父调用的 readCredential、base/native/ledger 及历史/工作校验读条件 | 原具名 V4/V5 提供方；没有把它们藏入一个总正确 reader 假设 |
| 原 Create options 的首次 INSERT/commit、录入到后续读取的事务历史 | V5/#637；本批只接同次读调用及现有封存计划合同 |
| 原 actions/receipts 的 identity/digest 完整收集及最终 anchor 编码/SHA 使用点来源 | V3 自有调用连接与逐符号 A6/B4；当前保留 FirstRead/ContinuedRead 的逐项观察，不把完整收集业务列为外部信任；digest 值相等不证明材料真实完整 |

正常 FirstRead/ContinuedRead 采用上游同一完整请求/工作规格，保留原成功域和正常分配条件。
原正常见证与错误版仍归各上游包，验收需核对它们的当前适用输入；本包没有新的 Rust 错误版，
不以导入失败、超时或有限样例充当一般反例。首次/继续正常返回证明与首错证明并列，
不能据此宣称任意请求必成功、循环必终止、持久收件、完整 manifest 或整个 V3 完成。

## 证明机复现与 ning 交接

先复用原固定工具、Lake 依赖和标准服务，使用独立输出及 `.lake/build`，保留共享重锁。
下列是开发检查，不能替代 `verify-stage1-protocol.sh` 的已登记原生准入和完整验收：

```sh
package=verification/experiments/stage1-protocol/unload-parent-link
# 原已配置的共享重锁；每次命令的输出目录必须尚不存在。
python3 "$package/capture.py" --mode source --output artifacts/unload-parent-source --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/capture.py" --mode parent --output artifacts/unload-parent-current --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/prepare.py" --output artifacts/unload-parent-project
python3 "$package/check_capture.py" \
  --source artifacts/unload-parent-source --parent artifacts/unload-parent-current \
  --project artifacts/unload-parent-project --output artifacts/unload-parent-capture-check
# 在同一外层协调锁及四核预算内构建，再释放该锁供 diagnose.py 取得。
(cd artifacts/unload-parent-project && lake build UnloadParentLink)
python3 "$package/diagnose.py" --project artifacts/unload-parent-project \
  --output artifacts/unload-parent-diagnostics --lock "$SANDPROOF_LOCK_FILE"
```

`check_capture.py` 重新执行 #770 的完整原源码/字典审查，核对 raw 生成体、命令/日志及
父调用除注释、字典参数和已登记 helper metadata 外的全部 token。它不签发原生封口。
`diagnose.py` 使用项目原 `tools/lean_session.py` 服务检查全部实际文件和准确审计，
拒绝 stale/partial、缺文件、错误和本包新页的警告。上游未改页的警告原样记录；
返回成功只表示这份开发检查完整，不代表公共入口已接受这些警告。

### 当前 revision 的两份原始抽取

复用 [#893](https://github.com/VeriInfra-Lab/sandproof/pull/893) 固定的工具和选择器。
在同一精确驱动提交分别执行 `base-revision` 和 `parent`，两次指定同一个完整
`--source-commit`。保留两个
输出目录中的 `source.tar`、`actual.llbc`、两个原始日志、`generated/` 和 `result.json`。
在核对结果的 checkout 中取得同一 Git 源提交及实际驱动提交，再运行配对检查。两个
目录可以移动，记录中的命令仍须显示各自 Aeneas 消费本次 Charon 写出的同一 LLBC。

```sh
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/test_revision_pair.py"
bash scripts/python-entry.sh "$package/check_revision_pair.py" \
  --source-commit f26fc26a3fb6987a18ad69015c56bb99c0748e12 \
  --driver-commit "$capture_driver_revision" \
  --base "$base_evidence" --parent "$parent_evidence" > "$pair_report"
```

检查器从 Git 独立重算原归档摘要，再核对两份源和驱动身份、原工具命令和日志、实际
LLBC、所有生成文件及原准入。base 的真实 LLBC 含一处 revision SQL 字面量，固定
Aeneas 把原查询展开到六条分支，每条查询的 SQL 参数均须相等。parent 核对对原
`base_snapshot` 的唯一调用。两个生成视图的 `ReadyCandidate` 字段清单须一致。
它只接受有原始文件的开发证据。生成编号可不同，跨视图类型和同一实际 SQLite 事务的
表示关系另外检查。配对报告只用于开发。

### 当前生成类型的表示连接

独立入口 [prepare_revision_transport.py](prepare_revision_transport.py) 从
[输入清单](revision-transport/inputs.json) 的两个原始 `Types.lean` 物化隔离项目。
保留原生成文件。组合视图引入各自的显式表示字典，并在 base 视图隔离两个冲突的
判别实例，正反检查字节。沿用历史请求和创建观察转换，显式传递当前
`ReadyCandidate.native_revision`。

- `RevisionReadyTransport.intentRoundtrip` 核对完整原请求和接受上下文的往返。
- `RevisionReadyTransport.projectionTransport` 核对全部候选字段和嵌套观察。包括
  原 `Option U64`、动作、回执、凭据、选项和 ready 事实。独立生成的 readiness
  仍使用具名转换参数。
- `RevisionReadyTransport.sharedInputCarriers` 核对 Transaction、Connection、
  World 和认证上下文的表示类型相同。这个等式只涉及类型。实际父调用借用和同一
  SQLite 事务的读框架由原调用连接及 V5 条件继续提供。

```sh
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_revision_transport.py" \
  --base "$base_evidence/generated/SandproofUnloadBaseSnapshot/Types.lean" \
  --parent "$parent_evidence/generated/SandproofUnloadFirstObservation/Types.lean" \
  --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp" \
  --output "$new_type_project"
# 在原重锁和四核预算内构建，再释放该锁供 diagnose.py 取得。
(cd "$new_type_project" && lake build RevisionReadyTransport)
bash scripts/python-entry.sh "$package/diagnose.py" \
  --project "$new_type_project" --output "$new_type_diagnostics" --lock "$SANDPROOF_LOCK_FILE"
```

该入口只装配当前真实类型及表示定理。原 `base_snapshot` 与父函数体之间的调用、
当前 revision 查询的首错由下面的独立入口检查。

### 当前父调用和 revision 首错

[prepare_revision_call.py](prepare_revision_call.py) 从
[输入清单](revision-call/inputs.json) 的两份原始生成函数装配独立项目，复用表示入口。
函数视图只接入显式表示和操作字典。反向变换恢复原字节。SQL 字面量沿用原
内核检查的 `toStr` 绑定；rusqlite 的操作继续按符号提供。

- `RevisionParentCall` 把真实 `base_snapshot` 装进实际父函数的调用字典，传递同一个
  Transaction、命名空间、World、来源意图和认证上下文。父函数的连接读取与 base
  共用同一个 deref 操作。正常回执完整传给父调用。错误结果保留原首错，任意替换
  最终观察请求编码器都不改变结果。
- `RevisionReadErrors` 验证原 SQL、唯一绑定键和第一列读取。到达 revision 读取前缀后，
  身份键构造错误、原 SQLite 错误和负 revision 各有首错定理。负值经同一父调用返回
  `CorruptStore`，后续编码不被求值。这些定理无需后续分配器返回成功。
- `RevisionReadSuccess` 给出生成函数的原 clone 和分配器后段。正常 revision 保留完整
  非负有符号值到 `Option U64` 的转换。读取成功后，原分配器错误仍原样返回。
  本地绑定缺失与绑定存在但 World 记录缺失分别核对，均跳过 revision 查询并保留该
  分支的完整结果。后段等式也保留原 clone 或分配器的外层失败和非返回。
- `RevisionReadSuccess.successful_read_preserves_original_fields` 给出正常读取后
  可供来源证明使用的字段等式。在原读取、revision 校验及分配器成功的条件下，结果
  保留原请求、当前条目、绑定、历史结果、原生视图和 readiness。revision 与分配器
  计数分别取各自的读取结果，原来源读取和物理事务对应仍由各提供方落实。
- 在同一父调用，正常 revision 与分配器值各进入自己的字段。原 base 返回缺
  `native` 的候选时，真实父函数返回 `CorruptStore`，后续观察请求编码不被求值。
- `BeforeRevision` 和原父调用前缀逐操作记录所需返回，保留 V4/V5 已登记生产义务。
  实际 SQLite 活动事务、同一物理快照和读错误后的帧仍需 V5 提供证明。

```sh
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_revision_call.py" \
  --base "$base_evidence/generated/SandproofUnloadBaseSnapshot" \
  --parent "$parent_evidence/generated/SandproofUnloadFirstObservation" \
  --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp" \
  --output "$new_call_project"
# 在原重锁和四核预算内构建，再通过注册 Lean 服务检查 plan.json 中所有文件。
(cd "$new_call_project" && lake build RevisionParentCall)
```

固定后端的 `__bind_in` 名称保留原 rusqlite 拼写及命名警告。整套封口还需修正驱动的
独立重抽配对、ning 原生回执及完整登记审计。

### 首次登记的外层抽取入口

`prepare-transaction` 在精确 integration `f26fc26a3fb6987a18ad69015c56bb99c0748e12`
的未编辑 Git 归档上选择原 `ControllerJournal::prepare_lifecycle_source_work` 方法。
沿用现有重锁、固定工具以及 80 GiB 起跑、64 GiB 停止、四核和 4 GiB target 预算。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
SANDPROOF_ACCEPTANCE_ROLE=formal bash scripts/python-entry.sh "$package/capture.py" \
  --mode prepare-transaction --source-commit f26fc26a3fb6987a18ad69015c56bb99c0748e12 \
  --output "$new_prepare_evidence" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/test_prepare_transaction_capture.py"
```

新的准入检查核对原方法的透明 LLBC、`SourceWorkRecord` 和 `Sources` 字段、
实际原 begin/World/Inbox/认证/来源负载/内层 current/容量/保存/commit 的调用身份
以及 Aeneas 的原方法映射和生成定义。调用清单记录出现次数。首错、重放分支跳过
后段、实际借用和物理事务帧仍需独立证明。`current` 保留为单独连接点，后续使用
原父视图及 #917 的类型关系。其余自有操作继续由具名提供方提供。没有新增可信模板。

入口自测使用合成声明和反例。只有实际 Charon、Aeneas 均通过并保存完整原件才有
新抽取证据。既有 base/parent 配对继续使用 [#914](https://github.com/VeriInfra-Lab/sandproof/pull/914)
登记的原驱动提交。新入口不签发该配对或 [#920](https://github.com/VeriInfra-Lab/sandproof/pull/920)
的原生验收。

### 已开始事务的原身份核对与候选 World

独立 Lean 根 `UnloadStartedTransaction` 复用 #770 保存的真实
`WorldTransaction::from_started_transaction`、`CommittedWorld::check_identity` 和
`WorldTransaction::world`。物化时按既有 [提供方清单](../../create-transaction-link/current/materialization.json)
逐份校验生成源码和依赖，并将事务文件、原 World 源文件和 Cargo.lock 对回
integration `f26fc26a3fb6987a18ad69015c56bb99c0748e12`。

- 构造函数在传入事务解引用到的连接上核对版本。生成结果传播原检查的内外层结果。
- 匹配版本后返回同一原事务、完整 committed 值、候选 World 与输入前沿。
  通过已登记的 `ownerView` 和真实 `world` 函数可读回该候选，并保留同一原事务的表示。
- 版本查询仍使用原 `PRAGMA data_version`、无绑定参数和原第一列 I64 回调。
  已登记的无冲突借用条件下，版本不匹配返回 `CorruptStore`，读取错误保留
  原 SQLite 错误载荷。失败分支没有返回已构造的事务所有者。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_started_transaction.py" \
  --output "$new_started_project" --lake-packages "$fixed_lake_packages"
# 在原重锁和四核预算内构建。注册 Lean 服务检查 plan.json 的全部实际输入。
(cd "$new_started_project" && lake build UnloadStartedTransaction)
```

结论从已开始的事务和已提供的候选 World 起步。实际 `BEGIN IMMEDIATE`、缓存来源、
物理回滚和 #921 新生成视图的借用连接仍由 V5 和对应调用证明提供。

### 原 `WorldTransaction::begin` 抽取

`world-begin` 从同一份固定 integration 源归档直接选择原方法，并额外抽取
`allocators::input_high_water`。准入检查五处原 Rust 调用及实际生成的 SQLite
`Immediate` 参数。输入前沿必须从原 allocator 的 `input` 类型与空作用域读取。
原 `from_started_transaction` 后续连接使用 #922 的已证结果。
两个固有方法根的 `--start-from` 使用 Charon 按类型解析的 `Type::method` 路径。
传给声明排序和 `--include` 的模式只在原模块的 impl 段使用通配，并限定原方法名。
准入核对两类根参数、完整生成 impl 路径和原源文件，正文缺失或 opaque 时拒绝。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
SANDPROOF_ACCEPTANCE_ROLE=formal bash scripts/python-entry.sh "$package/capture.py" \
  --mode world-begin --source-commit f26fc26a3fb6987a18ad69015c56bb99c0748e12 \
  --output "$new_begin_evidence" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/test_world_begin_capture.py"
```

沿用原固定工具、重锁、80 GiB 起跑、64 GiB 停机、四核与 4 GiB target 预算。
驱动自测使用合成生成体。仅实际 Charon 和 Aeneas 成功后才收录新原件。
物理 SQLite 行为、World 缓存与外层事务仍按原 V5 合同。

### 原 `begin` 的首错与已开始事务连接

`UnloadWorldBegin` 物化 `world-begin` 已准入的两份未改动生成文件，并复用上节
`UnloadStartedTransaction` 的完整依赖闭包。输入固定为 integration `f26fc26a`、
#923 驱动 `a290351c` 及该驱动的原源码归档、LLBC、生成映射和输出。两个外部模块
只提供具体类型转换与具名调用观察。`from_started_transaction` 直接调用 #770 的生成体。

- 四个首错定理依次保留缓存读取、写前维护、SQLite 开启和输入前沿的原错误载荷。
  各分支保留当时应返回的连接，SQLite 错误仍由原 Rust 的错误转换包入 `JournalError`。
- 开启成功后，前沿读取使用原事务解引用所得连接、原 `input` 类型和空作用域。继续使用
  同一个原事务、初始缓存参数、前沿和候选 World 调用已开始事务的构造。版本变化时返回
  `CorruptStore`，版本匹配时原 World getter 在同一事务表示上读回原候选 World。
- `BeginCalls.world`、`prepareWrite` 和 `readAllocator` 分别由 V5 的当前缓存读取、写前维护
  和真实 allocator 提供。`beginSqlite` 的物理连接、写锁与回写由 V5 的 SQLite 事务框架提供。
  定理对这些原值及调用观察成立。回滚、单写者租约、存储读写帧与完整外层登记仍是独立义务。
  原生 `Immediate`/`Deferred` 对照及运行命令见 [#929](https://github.com/VeriInfra-Lab/sandproof/pull/929)。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_world_begin.py" \
  --capture "$new_begin_evidence" --output "$new_begin_lean" \
  --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
# four_cpus 为当前获准 CPU 中的四个编号，用逗号分隔。
(cd "$new_begin_lean" && LEAN_NUM_THREADS=4 flock -n "$SANDPROOF_LOCK_FILE" \
  taskset -c "$four_cpus" lake build UnloadWorldBegin)
bash scripts/python-entry.sh "$package/diagnose.py" \
  --project "$new_begin_lean" --output "$new_begin_audit" --lock "$SANDPROOF_LOCK_FILE"
```

外层 `prepare-transaction` 仍按原 `prepare_lifecycle_source_work` 捕获。固定 Aeneas 在原
切片迭代器 `find` 的高阶生命周期约束处停止。外层认证、同一事务中的观察请求构造、
持久登记，以及构造前早错的无构造性质仍待实际生成体的组合证明。

主控可使用新物化项目的 `plan.json` 登记该闭包的全部实际文件，并使用
[覆盖候选](world-begin/public-registration.patch) 记录本切片。共享 Lake 接线和现有登记的
合并由主控按实际生成文件和回执统一处理。

### 外层首次登记的抽取候选

`probe_prepare_iterators.py` 默认在原 integration `f26fc26a` 的隔离源码副本中应用候选。
原 `prepare-transaction` 的高阶生命周期失败保留。试验中，循环内返回先触发提前返回
限制，持有借用迭代器的循环随后触发嵌套借用限制。现有三组对照使用固定工具和各自的
原始日志。

| 源视图 | 触及的变化 | 实际工具边界 |
|---|---|---|
| `replay-only` | 全文重放使用有界索引，第一次命中后返回原记录 | Charon 通过，Aeneas 在 `codec::save(&tx, ...)` 的借用展开停止 |
| `replay-and-capacity` | 容量按同一完整 session 计数，并公开 `LifecycleRequest`/`OperationBinding` 的实际字段 | Charon 通过，Aeneas 在 Serde 序列化 trait 递归组停止 |
| `json-boundary` | 在上一行基础上使用同一 `SourceWork` 值的具体 JSON 调用边界 | Charon 与 Aeneas 通过，准入核对完整父方法、两个循环和原调用 |

候选维持 Begin、读 Inbox、认证、读来源、全文重放、构造、序列化、容量、保存及提交的
原顺序。重放只克隆首个全文相等记录。长度超限先拒绝，其后只数同一 session，索引和
计数均受原记录长度约束。读取和保存继续通过外层持有的同一个 `tx` 借用连接。附加
`--wrapper-ir` 会单独查具体编码函数的真实 Charon 正文。检查完整 `&SourceWork` 的
原借用、唯一的 `serde_json::to_vec::<SourceWork>` 和直接返回的原结果。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
mkdir -m 700 "$new_probe_parent"
for variant in replay-only replay-and-capacity json-boundary; do
  if bash scripts/python-entry.sh "$package/probe_prepare_iterators.py" \
    --variant "$variant" --output "$new_probe_parent/$variant" \
    --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"; then
    test "$variant" = json-boundary
  else
    test "$variant" != json-boundary
  fi
done
bash scripts/python-entry.sh "$package/probe_prepare_iterators.py" \
  --variant json-boundary --wrapper-ir --output "$new_probe_parent/wrapper" \
  --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
```

前两条对照预期退出非零，分别保留 `probe-result.json` 和原工具日志。第三条须记录
`generated=true`。独立包装检查须有同一候选源码摘要和实际 `serde_to_vec_id`。
源码归档和实际源码改动分别记账，候选只供主控评估及后续对应证明。公共登记须等待
候选采用、完整 Lean 接线及正式回执。具体 JSON 边界的原调用证据后续须连接到
V0 JSON 合同。事务和存储来源仍由 V5 具名提供。

ning 可在干净的临时 `f26fc26a` 工作树中重放本次候选。`proof_checkout` 指向本 PR 的
独立工作树，当前运行目录为临时原生工作树。下面的原生用例复用 #920 和 #929。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = f26fc26a3fb6987a18ad69015c56bb99c0748e12
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" bash scripts/python-entry.sh - <<'PY'
from pathlib import Path
import probe_prepare_iterators as probe

path = Path(probe.prepare.SOURCE)
assert probe.digest(path) == probe.prepare.SOURCE_INPUTS[probe.prepare.SOURCE]
path.write_text(probe.rewrite(path.read_text(), 'json-boundary'))
assert probe.digest(path) == '4b433e2be3337db97e9fb7e8d9b868d159c3e2cb8c894a2a0ec78ea4867489fd'
PY
fixture_commit=87845bc241fe09c7803c2f9e0a1cdebf4c823b07
fixture_dir=verification/experiments/stage1-protocol/unload-prepare-transaction
target=controller-service/src/controller/lifecycle_inbox/source_work/tests.rs
git fetch --no-tags --no-write-fetch-head origin refs/pull/929/head
{
  printf '\n'
  git show "$fixture_commit:$fixture_dir/native-tests.rs.in"
  git show "$fixture_commit:$fixture_dir/begin-contention.rs.in"
} >> "$target"
cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks \
  --lib controller::lifecycle_inbox::source_work::tests::
```

### 当前基线的重抽取与 SQLite 首错

当前独立重抽取使用 integration `6fd0852025468e2583c436d37af18383bf2e71ea`，
源码树 `1eb064834e310a2006b3df6c5981ab15a07afc12`。从原 `f26fc26a` 到 `a044095c`，
变更限于控制器 SQLite 测试与验收说明。从 `a044095c` 到 `6fd08520`，变更限于中心
Redis 测试及验收说明。两份后续归档均以提交、树和归档摘要准入；驱动还检查原源码输入
和候选源码。下列命令在证明机用原固定工具重新产生两份真实视图。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
latest=6fd0852025468e2583c436d37af18383bf2e71ea
export SANDPROOF_ACCEPTANCE_ROLE=formal
bash scripts/python-entry.sh "$package/probe_prepare_iterators.py" \
  --variant json-boundary --source-commit "$latest" --output "$new_parent" \
  --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/probe_prepare_iterators.py" \
  --variant json-boundary --source-commit "$latest" --wrapper-ir --output "$new_wrapper" \
  --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/check_current_prepare_capture.py" \
  --source-commit "$latest" --old-parent "$old_f26_parent" --old-wrapper "$old_f26_wrapper" \
  --current-parent "$new_parent" --current-wrapper "$new_wrapper"
```

比较器保留原始 LLBC 及其摘要。逐份核实目的路径确实指向本次文件，再按同一相对文件名
比较该字段。短名表按唯一键排列后比较完整键值，其余 LLBC 字段严格比较，包括数字和
布尔类型。生成映射和全部输出的摘要逐一相同，父方法调用与独立包装的 Serde 调用一致。
这些是抽取证据。外部模板仍须按原合同接线并对实际 Lean 文件审计。

新原生夹具从已有 `ready` 初始化真实 SQLite。测试按原顺序观察三个不同错误。
原收件摘要损坏先返回 `CorruptStore`，收件修复后错 center 返回 `Conflict`，
换回原认证上下文后才到达来源摘要列的 `InvalidColumnType`。每次检查事务回到
autocommit、第三连接可见的工作槽仍空，且 Host 来源提供方未调用。修复来源摘要后，同一
意图产生一份实际登记。负例调换原方法的认证与来源读取，构建后应在第二步实际返回
`InvalidColumnType` 并命中新增断言。

另一条测试在 `codec::save` 通过原 SQL 给首个空槽赋值时由临时触发器拒绝。这个错误
发生于构造和编码之后。测试观察到原 SQLite 错误、事务回退、无持久登记和无 Host 来源
调用。去掉触发器后，同一意图成功登记并全文重放。构造前的拒绝和构造后的失败需要
不同结论。提交后确认丢失继续按既有的未知提交用例处理。

ning 在原生机使用三个独立的干净临时 checkout，均固定为上述 integration，分别设置
`variant=original`、`variant=candidate` 和 `variant=reordered`。
`proof_checkout` 指向本批证明工作树。试验只改各自临时 checkout。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 6fd0852025468e2583c436d37af18383bf2e71ea
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
  bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
import frontier_capture
import probe_prepare_iterators as probe

path = Path(probe.prepare.SOURCE)
assert probe.digest(path) == probe.prepare.SOURCE_INPUTS[probe.prepare.SOURCE]
variant = sys.argv[1]
if variant in ('candidate', 'factored', 'factored-reordered'):
    candidate = probe.rewrite(path.read_text(), 'json-boundary')
    if variant.startswith('factored'):
        candidate = frontier_capture.factor(candidate, reordered=variant == 'factored-reordered')
    path.write_text(candidate)
    expected = {
        'candidate': '4b433e2be3337db97e9fb7e8d9b868d159c3e2cb8c894a2a0ec78ea4867489fd',
        'factored': 'da11e9e7e8f9aab90b4ef6911169f62ba2f5cab1b2066491903045d90a83dbf6',
        'factored-reordered': '650362a8fc2dfb2811152c13b8514af907c05af3fb5dfdcec9ce6eccf31ab009',
    }
    assert probe.digest(path) == expected[variant]
elif variant == 'reordered':
    opening = '    pub fn prepare_lifecycle_source_work'
    following = '    /// Inspect the original finite responsibilities'
    prefix, method = path.read_text().split(opening, 1)
    body, suffix = method.split(following, 1)
    authenticate = '        authenticated(&inbox, intent, context)?;\n'
    read = '        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;\n'
    assert body.count(authenticate + read) == 1
    path.write_text(prefix + opening + body.replace(authenticate + read, read + authenticate)
                    + following + suffix)
else:
    assert variant == 'original'
PY
fixture_commit=87845bc241fe09c7803c2f9e0a1cdebf4c823b07
fixture_dir=verification/experiments/stage1-protocol/unload-prepare-transaction
target=controller-service/src/controller/lifecycle_inbox/source_work/tests.rs
git fetch --no-tags --no-write-fetch-head origin refs/pull/929/head
{
  printf '\n'
  git show "$fixture_commit:$fixture_dir/native-tests.rs.in"
  git show "$fixture_commit:$fixture_dir/begin-contention.rs.in"
  cat "$proof_checkout/$package/current-inputs/native-first-errors.rs.in"
} >> "$target"
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
if test "$variant" = reordered || test "$variant" = factored-reordered; then
  "${native[@]}" --no-run
  rc=0
  "${native[@]}" proof_first_unload_prepare_orders_inbox_authentication_and_source_decode \
    -- --nocapture > "$native_negative_log" 2>&1 || rc=$?
  test "$rc" = 101
  rg -F 'authentication must precede source decode: Err(Sqlite(InvalidColumnType(1, "sha256", Text)))' "$native_negative_log"
  rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
else
  "${native[@]}" controller::lifecycle_inbox::source_work::tests::
fi
```

### 首次登记前缀的 Lean 对应

在同一个 `6fd08520` 候选副本中，从原父方法提取 `open_source_work_inputs`。原 Begin、
读 Inbox、认证和读来源仍在同一事务内顺序执行。父方法的来源读取字典只转发原
`source_work::codec::load`。辅助函数接受来源解码值的实际类型参数并完整返回该值，
避免重定义来源表和 V3 协议类型。成功时返回原事务及两次读取的值。

两份正确版抽取来自同一候选源码。窄视图逐分支证明首错，复用 #930 的实际 Begin 定理与
已物化的原 Inbox 认证函数。全父方法视图核对真实字典、原 `codec::load` 调用、实际前缀
结果的 `ControlFlow` 分支，以及构造观察请求只位于成功分支。调换认证和读来源的错误版
独立重抽，在同一认证拒绝条件下，Lean 证实实际返回来源的 SQLite 错误而非 `Conflict`。
原生错行由 [上一节](#当前基线的重抽取与-sqlite-首错) 的同一测试命中。

证明从解码的 Inbox 与来源结果开始。V5 须提供原事务中的两种读取及其返回值关系，V7
负责独立认证上下文的来源。正确版定理保留原错误及连接，覆盖 Begin 拒绝、Inbox
缺项/错误、认证拒绝、来源缺项/错误和成功携原事务。外层完整 Lean、来源 JSON 编解码
对应、持久保存及提交/回退组合仍是后续义务。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
latest=6fd0852025468e2583c436d37af18383bf2e71ea
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in prefix parent reordered; do
  bash scripts/python-entry.sh "$package/probe_prepare_iterators.py" \
    --variant json-boundary --frontier "$mode" --source-commit "$latest" \
    --output "$new_frontier_capture/$mode" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
for mode in prefix reordered; do
  flags=()
  root=UnloadFrontier
  if test "$mode" = reordered; then
    flags=(--reordered)
    root=UnloadFrontierReordered
  fi
  bash scripts/python-entry.sh "$package/prepare_unload_frontier.py" \
    --prefix "$new_frontier_capture/$mode" --parent "$new_frontier_capture/parent" \
    --begin "$original_begin_capture" --output "$new_frontier_lean/$mode" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp" "${flags[@]}"
  (cd "$new_frontier_lean/$mode" && LEAN_NUM_THREADS=4 flock -n "$SANDPROOF_LOCK_FILE" \
    taskset -c "$four_cpus" lake build "$root")
  bash scripts/python-entry.sh "$package/diagnose.py" --project "$new_frontier_lean/$mode" \
    --output "$new_frontier_audit/$mode" --lock "$SANDPROOF_LOCK_FILE"
done
```

各输出父目录事先创建，子目录须全新。证明机保留三份原抽取、两份 `plan.json` 和实际
文件审计。ning 在两个原生临时副本使用上一节命令，分别设置 `variant=factored` 与
`variant=factored-reordered`，核对 121 项来源组件测试和错误版的实际首错。主控按
两个计划提取公共登记候选；活动工程闭包仍以最终采用的源码和正式验收为准。

主控使用 [登记候选](registration-candidate.json) 和 [公共覆盖候选](public-registration.patch)
接入全部实际输入和审计；先解决 #776 已列的旧输入/原件阻断，不能只跑旧根报告本包通过。
ning 在确定的新提交上完成普通检查及相关上游原生原版/错误版/恢复检查；严格按原收据键、
完整输入与日志决定可复用部分，`functional` 同候选只运行一次。
`proof_native_suite` 和 `verify-controller` 由主控去重，不在本包另签全项目结果。
