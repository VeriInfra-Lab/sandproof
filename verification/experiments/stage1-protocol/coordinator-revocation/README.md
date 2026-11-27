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

[事务包登记候选](journal-provider-candidate.patch)以 #1061 的 `de37231d` 为补丁基线，供主控在叠入本生产版本的候选上接入。补丁将新页面加入现有事务入口的构建根与逐文件检查清单，用 `current_file_overrides` 记录本版协调器，保留原冻结。事务独立入口的既有输入漂移仍需按各自提供方回执核对。

## 复验与登记候选

在证明机干净候选上使用固定工具和现有重锁，给出新的私有输出目录。

```bash
SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1 \
  bash scripts/python-entry.sh verification/experiments/stage1-protocol/coordinator-revocation/check.py \
  --tools "$proof_tools" --packages "$fixed_lake_packages" --lock "$shared_proof_lock" --output "$new_output"
```

`--tools` 下应有原 `aeneas` 与 `lean-mcp`。入口从 Git 解包源树，保留四份 Charon 产物、公开 journal 和联合视图的 Lean 生成体、隔离的 Lake 构建、实际文件诊断和审计。联合视图的两份 `*_Template.lean` 只由翻译器输出，实际联合工程使用已登记的五个函数绑定和四个类型绑定。

交主控登记这个形式入口及 `SandboxModel.Lifecycle.Protocol.CoordinatorRevocation` 的九条审计。覆盖记录可登记“在已解码 `Result` 上证明应用撤销选择，全文重放命中给出实际查询和选择，原父函数接线经实际 Charon 核对”，适用版本取通过对应检查的候选。生产协调器是现有原生输入，相关登记与冻结由主控结合 #1061 逐闭包复核。

[公共登记候选](registration-candidate.patch)供主控在包含本包的候选上审阅、试套。[覆盖记录候选](coverage-candidate.patch)注明已实测的源码和边界。登记通过 `scripts/verify-stage1-protocol.sh` 默认使用的 `full` 计划调用同一检查器。其原生前提包括已封口的完整 Linux `functional` 日志中的两个指定测试，以及原 `stage1-inbox-lookup` 和 `stage1-inbox-state` 两份收据。单项命令和缺少封口的日志可用于开发复核。

[默认入口的新增输入候选](inputs-candidate.patch)向 `verification/stage1-protocol-inputs.sha256` 增列本包九份 Python 检查文件、两份 Rust 薄根、两份 Lean 定理模板、三份生产 Rust 源码以及原事务提供方的 profile 和薄根，共十八项。生产行取追加抽取薄根前的 Rust 全文。主控在接入登记补丁后还需核对原清单已有行、各提供方清单及同一候选的原生封口。

登记子任务在分配的四核内继承并核对原协调锁描述符。检查点读取四份指定 LLBC、公开 journal 的五份生成产物、真实内核命令的退出状态与日志、物理文件和原服务回执。收敛与 Create 有限计划继续按各自的原任务清单验收。主控合入时需重新校验受影响的公共冻结和完整原生输入。

ning 在同一候选运行普通完整检查，核对现有 `historical_unload_keeps_resumed_route_current_unload_revokes_it` 与新增 `uncertain_commit_selects_broadcast_only_for_access_ending_operations` 两个原生断言组。前者通过原应用和受控端口观察实际通知，后者覆盖不确定提交和普通拒绝的决策边界。完整公共入口和实际网关分别验收。
