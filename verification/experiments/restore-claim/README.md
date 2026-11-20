# 恢复原认领的比较与重开读回

共同基线为 `1b15872a07568f9f34aad514a64673ac328c7ee8`，源码树为
`42b4f7fda5df6c10c9310f78b9edec6bbb98048a`。本包交付 LC-P7 的选中行决策
和真实 SQLite 读回用例。LC-P7 的完整历史、物理候选和许可发送由后续组合落实。

## 真实入口及原事务

- `advance_recorded_lifecycle_resume` 按原收件上下文认领会话，再调用
  `ControllerJournal::admit_lifecycle_resume`。准入在 `WorldTransaction::begin`
  创建的 IMMEDIATE 事务中重读认证 Inbox，按 session、原 Unload 的 source incarnation
  和 capture id 选择来源行。已有 claim 时比较完整 `AcceptedLifecycleIntent`，
  全文一致走 `Inbox::state(s,e)`。`find_entry` 先比较原操作键，随后核对全文。
  历史 entry 的 `local_operation` 仍来自原 entry，`state.local` 则是当前 session 映射。
  读历史候选应使用原 claim 和原效果计划。
- 首次准入先比对当前 incarnation、已完成 manifest、原暂停计划和耐久 snapshot、
  原 VM/slot 的释放见证、寿命及容量。它分配新的 native operation，在同一来源行
  保存 `RestoreClaim` 和 pin，随后撤出旧凭据，调用 Core Resume 并预留 startup，
  保存原 Inbox 的 entry/local/incarnation，最后提交。Core 请求经
  `storage::apply → checkpoint::check_input_profile → permits_resume_input`
  从同一事务读刚保存的原认领。已进入该入口的历史结果也仍按原行核对。
- `ControllerJournal::claim_next` 从 ready outbox 读取完整动作，检查宿主 epoch，
  预演 Core Dispatch/Worker，再调用 `claim_activation`。后者按原 World 的
  `logical_effect(action.command.logical).origin == Operation(claim.local.sandbox,
  claim.local.operation)` 查找来源行。材料 pin 已关闭则拒绝。对激活，已有标记
  要求完整动作相等，首次保存 `Some(action.clone())`。同事务再应用 Core Dispatch、
  更新原 outbox 行为 dispatched，并提交。确认后才向调用者返回动作。
- 重开的 `open_current` 从原 checkpoint/账本恢复 World。当前 Resume 由
  `lifecycle_pending_resumes` 取出。原认领由 `lifecycle_restore_work` 按全文读取，
  派发恢复由 `lifecycle_restore_action_work` 按原效果操作键读取，并由
  `uncertain_actions` 枚举原 outbox。后续授权由 `lifecycle_restore_activation_permit`
  与宿主执行入口承担。

初次认领的 `activation` 为空，表示准备及材料责任。`Some` 是可能执行标记。
两者各用自己的事务保证。激活事务回滚时，初次认领、pin、原候选与 ready 动作
仍在，标记与 Dispatch 更新均未持久化。激活提交结果未知时保留原责任，重开按
原 claim、效果状态及 outbox 读回。提交已成立时读回同一个完整动作，未成立时仍
只处理原 ready 条目。初次准入未知同样须先读回原键，再判断是否需要完成首次准入。
这些 SQLite 结论依赖 [A4 事务与恢复](../../../docs/formal/dependencies.md) 的
固定版本、WAL/synchronous FULL、单写者、同一受保护文件和可靠同步条件。

## B 线引用的身份来源

| 身份 | 持久字段和原核对 |
|---|---|
| Owner、会话、所属节点 | Inbox entry 与 `RestoreClaim.intent.request.binding` 有协议 owner、session、node，intent 也有 center/retention。原 `SourceAnchor` 保存原 Unload 及 native local owner/sandbox。`validate_record` 复核这些对应。 |
| 捕获和 manifest | 同一 `SourceWorkRecord` 保存原 Unload、原 Full 工作、Seal 动作和 `capture.completed`。Resume 全文保存 source incarnation/capture id/target incarnation/manifest digest。原 Pause plan 的 `target_snapshot` 通过原操作键关联，须与 Resume plan 的 `restore_from` 一致。 |
| 完整请求及 native operation | `RestoreClaim.intent` 保存全部原 accepted 请求与元数据。Inbox entry 的 `local_operation` 与 claim 的 `InternalLocalBinding` 关联原 World 的 `OperationKey`。这些是事务写入和应用交叉验证的键。`outbox(sandbox_id, owner_id, operation_id)` 由 SQL 外键约束到原 operations，后者由 `(sandbox_id, owner_id)` 约束到 sandboxes。 |
| 新运行代、候选 VM | `RestoreClaim.generation` 为 native 运行代。VM 来自原储存的 Core plan `vm` 和原 ActivateVm 命令。checked next generation 与协议 incarnation 分开检查。 |
| 实际 action、effect、attempt | 首次激活后 `RestoreClaim.activation` 持久保存完整 `WorkAction<EffectId>`。它含 `ActionIdentity` 的版本、owner、sandbox、operation、epoch、generation、逻辑动作、attempt、resource，以及 native `EffectId` 的完整命令和 attempt。`validate_record` 核对命令各适用字段、计划 VM 和实际效果 origin/status。 |
| SQL 行和恢复 work binding | 源行的 `work_key` 为原 `SourceAnchor` 编码的摘要，唯一约束及 `load_records` 复核它，并保留整行及 JSON 摘要。`RestoreWorkRequest::binding_digest` 对完整 intent、source requirement、Full work、manifest、Seal action、native operation、generation 的原 Serde 编码取 SHA-256。许可只存完整 action 与此摘要。 |

上述摘要依赖原 [A6/A9 Serde 和 SHA-256 合同](../../../docs/formal/dependencies.md)。
原字段、键、完整字节及作用域仍须随绑定核对。源进程 PID、宿主路径、单个摘要均不足
以标识原捕获和候选。现有 `restore-work` 的 `FullWorkContract` 覆盖从已解码
`SourceWorkRecord` 构造完整 `RestoreWorkRequest`，其调用和耐久义务由本包保持显式。

## 定理与证据边界

`SandboxModel.Stage1Protocol.RestoreClaim.existing_activation_exact` 接收原生成的
`RestoreClaim` 和两个 `WorkAction<EffectId>`。在 `claim.activation = some original`
及两个动作均为 `ActivateVm` 的条件下，真实派生比较分支返回 `.ok (some true)`
当且仅当 `proposed = original`。`keeps_recorded_activation` 用同一判据约束持有 pin
的选中行。`existing_restore_exact` 对所有已选 claim 的完整 accepted intent
给出相等判定，`same_intent_replays` 和 `other_intent_refused` 分别对应两支。
`marked_claim_refuses_resume_prefix` 证明无论其余前置比较的值如何，带标记的
已选行在原生 Resume 检查前缀均拒绝。`unmarked_claim_accepts_resume_prefix`
核对允许继续到原 capture 计划检查的条件。三处结果组合于
`recorded_claim_readback`，以读回同一原 claim 及相应类型条件为前提。

在来源已经选中后，`selected_first_activation_write_exact` 核对原标记赋值进入
`codec::save` 时的完整记录。持有 pin 的未标记行第一次接受 `ActivateVm`，
只把 activation 写成完整原动作。已有标记时，`selected_activation_preserves_marked_row`
核对动作比较后原记录不变，且控制流在 `codec::save` 前返回。
`selected_closed_pin_refuses_without_write` 核对 pin 关闭的拒绝。Clone 证明复用
原 restore-work、inbox-record 和 Foundations 证明。

正误视图均抽取选中行的原判断和标记赋值，用同一条 claim、同一 Resume/C、
VM 2 与 VM 3 检验同一个 `PreservesMarkedRow`。错误视图只将已有标记的比较
替换为 true，异候选得到错误的成功返回，但记录仍保留原标记。该反例的可达范围止于
选中行调用。Core Dispatch/Worker 的前置条件，以及 B 的许可、发送和物理启动，
仍分别验证。

SQLite 首次准入、原 VM 与原映射重开复用使用原
`resume_sql_claim_reserves_one_successor_withdraws_old_credential_and_reopens_unchanged`。
首次准入丢回包使用原
`resume_unknown_sql_commit_keeps_source_claim_index_and_native_mapping_atomic`。
`activation-readback.rs.in` 复用相同来源和工作流，补激活事务已回滚和已提交的
丢回包，并复查原 claim 与原 outbox。Host 回执采用原组件受控端口。

### 现存证据及缺口

- [#766](https://github.com/VeriInfra-Lab/sandproof/pull/766) 的现有 restore-work
  值组装和全文比较可复用。原 [#770](https://github.com/VeriInfra-Lab/sandproof/pull/770)
  提交、缓存未知合同仅按其原方法、状态和实际参数适用。
- [#838](https://github.com/VeriInfra-Lab/sandproof/pull/838) 头
  `5dc7d1c917bc19eeb925ae5eb63490f30e5d149f` 测 Core 在激活未知后拒绝再次
  ReserveStartup，并有原断言失败的错误视图。其 52 个 Core 输入与原报告基线对应。
- [#939](https://github.com/VeriInfra-Lab/sandproof/pull/939) 头
  `48358f79131dbd616d6bcb85041dc55af5126b66` 及后继
  [#959](https://github.com/VeriInfra-Lab/sandproof/pull/959) 头
  `8bb7fbcbef2fd565677b171607508c2e7349f2bc` 覆盖首次卸载/来源观察保存尾部。
  [#962](https://github.com/VeriInfra-Lab/sandproof/pull/962) 验收候选
  `35dfaf84ae33e1d78d1d069bac439abc0b35f5d3` 是首次卸载的 SQLite 行选择、
  单行更新、回滚和写锁证据。三处保留其原覆盖与证据身份。
- 完整 `claim_activation` 的生成在来源 codec 的 `save` 中停止，固定 Aeneas
  拒绝嵌套循环中的提前返回。完整 `permits_resume_input` 的生成在
  `load_records` 遍历行时的早退和错误传播停止。未选中记录或来源账本缺失时，
  `claim_activation` 可返回 `Ok(true)`。从可达生产恢复动作证明选中原行、完整
  codec 读写、真实验证器及重开连接仍需接入。
- `admit_lifecycle_resume` 的原薄转发根使用已登记的 rusqlite 错误表示时，
  Charon 在 `controller-service/src/journal.rs:96` 匹配该外部错误枚举处失败。
  `admit-root.rs.in` 保留该探针入口，外部错误分支和完整父方法的抽取尚待接通。
- Inbox 的 `validate_execution_lineage` 验证同一 session 的 capture frontier
  严格单调，以及 Resume 紧跟匹配的 Unload。来源选择、所有写者对 claim 的保留、
  退休后历史键不可复活仍需作为实际连接提供。
- 其它授予资格路径包括 Inbox 当前请求/版本裁定、Core Resume/ReserveStartup、
  Core 与 journal 的效果派发、重开后的未知激活继续、通用宿主动作为离开数据库后调用、
  失败卸载原实例的 continue 路径，以及后继捕获和退休后的历史窗口。
  这些入口及其历史组合仍待核对。

## 在 ning 复验

从原固定输入建立新干净工作树，使用已经提供的固定 Rust/Lean、离线 Cargo、原重锁
和原共享依赖，`TOOL_BIN` 中为固定 Charon、Aeneas 及配套 Charon driver。
运行 [本包形式入口](verify-decisions.sh)，保留两版原始 LLBC、Lean 生成物、内核和
实际文件服务诊断。复验脚本只在各自的私有源码副本写入薄根和 Lake 登记。

```sh
case_dir=verification/experiments/restore-claim
bash "$case_dir/verify-decisions.sh" "$FORMAL_EVIDENCE" "$PROOF_LOCK" \
  "$TOOL_BIN" "$LAKE_PACKAGES" "$LEAN_MCP_ENV"
```

普通入口为原 `scripts/check-function.sh`。激活读回组件由操作者在另一份干净
私有副本、既有原生锁及私有 Cargo target 下运行。

```sh
cat "$case_dir/activation-readback.rs.in" \
  >> controller-service/src/controller/lifecycle_inbox/source_work/tests.rs
cargo test --locked --offline -p sandproof-controller-service --lib \
  restore_activation_commit_unknown_reads_back_original_claim -- --nocapture
```

本包和现有 Stage1 公共模板目录同级，集成窗口统一登记。独立形式任务含正确与错误
两个隔离 Lake 视图，每个按真实导入闭包
与审计清单检查。普通任务应加入激活提交未知读回及两个已存在的 Resume 用例。公共
根、冻结、覆盖和标准批次入口在合并这些任务时登记。
