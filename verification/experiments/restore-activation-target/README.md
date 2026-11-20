# LC-P7 恢复激活的发送目标

固定生产输入为 integration `1b15872a07568f9f34aad514a64673ac328c7ee8`，
源码树 `42b4f7fda5df6c10c9310f78b9edec6bbb98048a`。本包交付原许可载体
的字段保持定理，并记录 B 侧从 SQLite 许可读取到实际写入的源码核对。
真实发送目标的证明仍缺一条宿主身份关系。

## 持久记录如何成为可用许可

- 首次执行由 [应用驱动](../../../controller-service/src/application/coordinator/execution.rs)
  调用 `claim_next`。原事务登记激活动作，并更新 World 和 outbox。提交及
  `AfterDispatchClaim` 检查成功后才将 action 交给 `execute_capture_action`。
  写入的原子性、唯一性和未知提交之后的选择归 A。附件所需前提为
  `OriginalClaimCommitted(C, A, action)`、`CompleteBinding(C, A, action, work)`
  及 `UnknownCannotAuthorizeAnotherCandidate(C, A)`。
  A 侧的 `existing_activation_exact` 对已选中、已有原标记的记录给出完整动作的
  相等判据，原记录选择和完整持久历史仍是显式条件。
  后继的 `selected_first_activation_write_exact` 和
  `selected_activation_preserves_marked_row` 连接已选中行上的首次写入和原标记保留。
  [A 侧原读取证明](https://github.com/VeriInfra-Lab/sandproof/tree/38691108440df6012f211eb6d4ea4dc806413c2a/verification/experiments/restore-claim) ·
  [A 侧后继写入证明](https://github.com/VeriInfra-Lab/sandproof/blob/54e1714c0991648aa442002d9ce520e51bada189/verification/experiments/restore-claim/RowWrite.lean.in)
- `restore_execution_context` 在应用锁内读取按逻辑 effect 的原 Resume operation
  定位的 work，重取原认证上下文，检查当前请求及 World 代数/操作。
  [许可读取](../../../controller-service/src/controller/lifecycle_inbox/source_work/restore.rs)
  在同一原 SQLite 连接上核对 World memo 身份，读取 inbox 和 source-work，再按
  原 intent 选行。它对选中的记录执行 `validate_record`，重新构造 work 并与请求全文
  比较，同时核对寿命、当前代凭据和原 activation action。只有 `Dispatched` 或
  `OutcomeUnknown` 可通过最后的状态门。构造的 permit 保留在局部，第二次
  `check_identity` 和读事务的 `commit()?` 成功后才能返回给应用。
- 初次写提交没有返回可用 action，或本次读事务出错时，当前调用不能据此执行 restore。
  重新取得原许可需要另一次成功的读核对。先前的写入或发送仍可能已发生。
  `CommittedWorld` 的正常性要求单写者、正确的连接／事务来源和受保护数据库。
  `PRAGMA data_version` 用于检查 memo 的连接内身份。
  [World memo 实现](../../../controller-service/src/controller/transaction.rs)

| 身份 | 生产来源及连接 |
| --- | --- |
| owner、session、node、原 incarnation 与原 capture | Resume intent 和原 Unload 请求同 center、retention、session、node、owner、capture；本地 owner/sandbox 与原 source-work anchor 对应。许可的完整 action 直接携带协议 owner，session 经已比较的 work 连接。 |
| manifest 与原捕获动作 | Resume digest 等于原 capture 的 completed digest。work 重建已提交的 Full、manifest digest 和 Seal action。新 plan 的 restore_from 等于原 plan 的 target_snapshot。 |
| 恢复请求、operation、新运行代 | work 保存完整 Resume intent、本地 operation 和新 generation；本地 generation 是原 anchor generation 的后继，并与原 Resume plan 一致。 |
| 候选 VM、action/effect/attempt | 许可保存完整 `WorkAction<EffectId>`。命令携带 sandbox/epoch/generation/VM，effect 保存原命令和 attempt。原 marker 与动作全文比较，World 的原逻辑 effect 来源匹配 Resume operation，plan.vm 匹配 ActivateVm。 |
| work binding | 许可直接携带 work digest，Node 从原 intent、来源要求、Full work、manifest、Seal action、operation、generation 重算后比较。源编码和摘要的对应条件仍须满足。许可不保存 Linux 进程身份。 |

[work 与许可类型](../../../controller-service/src/capture/restore.rs) ·
[应用读取及显式继续](../../../controller-service/src/application/coordinator/lifecycle_restore.rs) ·
[动作与回执的身份](../../../sandbox-protocol/src/lib.rs)

### 原读事务提交拒绝的 SQLite 对照

在 ning 的私有基线工作树用既有 source-work 夹具完成真实 SQLite 认领，调用原
`lifecycle_restore_activation_permit` 取得完整 action 与 work digest。保持两项输入
和 SQLite 连接一致，用固定 rusqlite 0.40.2 的 authorizer 仅拒绝该读事务的
`COMMIT`。该版本把此操作映射到 `TransactionOperation::Unknown`，用例同时核对
`Begin → Unknown → Rollback` 序列、`SQLITE_AUTH` 和回到 autocommit。解除注入后，
同一原方法返回与第一次相等的许可。该用例实测读事务成功门及确定的读侧错误，
写侧提交结果未知的语义仍由 A 提供。观察范围止于原方法的返回值。
[原 SQLite 对照](permit-read-commit-test.rs.in)

使用既有重任务锁、固定 Rust 1.97.0 及离线依赖，在新的私有基线工作树中仅登记
测试模块。下列 `CASE` 和 `CARGO_TARGET_DIR` 均为独立绝对路径。

```bash
module=controller-service/src/controller/lifecycle_inbox/source_work/tests
cp "$CASE/permit-read-commit-test.rs.in" "$module/permit_read_commit.rs"
printf '\nmod permit_read_commit;\n' >>"$module.rs"
cargo test --locked --offline --package sandproof-controller-service --lib \
  --features rusqlite/hooks -- \
  --exact controller::lifecycle_inbox::source_work::tests::permit_read_commit::original_restore_permit_read_does_not_return_on_sqlite_commit_error
```

本次新编译的测试二进制执行 1 项并通过。原始收据位于 ning
`/var/lib/sp-manual-src/lc-p7-permit-commit.fqcDHCjo/evidence`，
`sha256sums` 摘要为
`8134d7783752444448f45abb6fe7732759d603725462adebbfa1f5d7edca612d`。

## 从正式 Host 入口到实际写入

| 路径 | 可能开始执行的动作 |
| --- | --- |
| 首次激活 | `execute_capture_action` 将原 work 和成功读取的 permit 交给 `NodeHost::execute_lifecycle_restore_action`。 |
| 继续原激活 | `reconcile_current_actions` 先观察原不确定动作，观察未取得回执时显式进入 `continue_restore_activation`。它检查 Resume/Starting，重新取得原 work 和 permit，再调用同一执行入口。 |
| 只观察 | `observe_capture_action` 及生命周期只读收尾调用 `observe_lifecycle_restore_action`，传入 `None, true`。`restore_activate` 走 `confirm`，只读原候选状态及 guest 凭据状态。 |
| 同一候选已经 Running | 有许可的继续可完成 guest 重授权；Runtime 的 Running 分支只再次观察，跳过 Resumed 请求。Paused 可再次发向同一候选；性质按动作计，不限制请求总数。 |
| 其它入口 | Prepare 的 `PUT /snapshot/load` 固定 `resume_vm=false`。ConfigureNetwork、ApplyFilePolicy 和退出观察没有 Resumed 请求。通用 Prepare 拒绝 snapshot source，通用 Activate 的 guest 初始化要求已运行。原 Unload 失败后的 `continue_original_vm` 有自己的 Resumed 发送点、原 VM 和许可，属于另一条协议。 |

首次入口查无恢复 work 时可走通用分派。通用 Activate 的凭据及模板写入属于 P9。
专用 Paused 分支配置 MMDS 时，`PUT /mmds` 和 `PATCH /vm` 各自建立连接。
P9 的对端身份关系需要从较早的凭据写入开始保持。

[首次执行与观察路由](../../../controller-service/src/application/coordinator/lifecycle_unload.rs) ·
[Host 适配](../../../sandbox-node/src/adapter.rs) ·
[Node 恢复](../../../sandbox-node/src/linux/restore.rs) ·
[原 VM 继续](../../../sandbox-node/src/linux/original_continue.rs)

Node 核对原 action、占用绑定、来源/profile、当前 boot/fence，再用 action 的 VM
导出 `vm-<namespace>-<vm>`、原位置及 Runtime 配置。执行分支要求 permit.action 和
work digest 匹配，把同一个 name、manifest、work digest 和本次观察的完整进程出生身份
传入 `activate_loaded_snapshot`。Runtime 在原位置 helper gate 下重新核对 restore
intent、进程记录及 Pause/Running 状态。若 `restore-loaded.json` 存在，同时核对
intent digest 与进程全文。其 NotFound 分支仍使用原 intent 和 process claim 观察，
这也需要在后续证明覆盖。Paused 分支先配置 MMDS，再请求 `PATCH /vm` 的 Resumed。
[Runtime 恢复](../../../sandbox-runtime/src/snapshot_restore.rs)

### 最早缺失的发送目标关系

`activate_loaded_snapshot` 对 `before.process == expected` 的检查止于调用
`self.api(id, ...)`。到 `Runtime::api_stream`，新连接前又按 id 调用
`read_record(id)`。此处没有 `expected` 或携带该身份的观察句柄。该函数确实
检查新连接自身的 `SO_PEERCRED`，用本次读取的记录校验 PID、UID/GID，并再次
验证此记录所指进程。随后 `api_reply` 在同一条 stream 上写 header 和 body。
这建立本次所查进程记录与所用 stream 的连接，仍需证明本次记录就是先前的 A。
[实际连接与写入](../../../sandbox-runtime/src/linux.rs)

已核对有利的生产机制。原 claim 创建排斥同名已存在项，进程记录通过
`persist_noclobber` 发布，Slot helper gate 串行化持有同一个锁的参与方。
`Runtime::retire` 也取得 gate，完成物理检查后可删除 claim 和其中的记录。
所以完整证明需要 V6/V9 的命名不复用与保护不变量，覆盖原位置、启动、
退休、重启及所有合法 claim 写者，并把其寿命连接到真正使用的 stream。
Node 正式配置提供原位置 gate，Runtime 通用配置也允许无 slot。
本批未建立合法执行中该等式能够失效的反例。后置观察能拒绝回执，发送事件
的安全性必须在第一次写之前建立。
[原 claim 与记录发布](../../../sandbox-runtime/src/linux.rs) ·
[锁取得](../../../sandbox-runtime/src/helper_gate.rs) ·
[退休路径](../../../sandbox-runtime/src/retirement.rs)

## 已证明的许可载体

`SandboxModel.Stage1Protocol.RestoreActivationTarget.permit_carrier_round_trip
(action : WorkAction<EffectId>) (digest : ContentDigest)` 断言原构造函数和两个原访问器的
顺序求值得到 `.ok (action, digest)`。三个原方法和三个薄根均生成透明定义。
定理的边界是许可值。读取哪一行、何时返回许可、怎样构造请求和哪个 VMM 收到写入，
分别由外层调用关系承担。
[Lean 定理](PermitCarrier.lean.in) · [Rust 薄根](carrier-root.rs.in)

在 ning 使用既有锁和固定工具。在独立的 `RUST_VIEW` 检出上述生产提交，保留
工作树唯一差异为以下薄根和模块声明。将 `OUT`、`CARGO_TARGET_DIR` 设为新的私有绝对
路径，并按工作流配置原离线 Cargo 缓存。

```bash
cp "$CASE/carrier-root.rs.in" "$RUST_VIEW/controller-service/src/proof_restore_activation_carrier.rs"
printf '\n#[cfg(feature = "durable")]\npub mod proof_restore_activation_carrier;\n' \
  >>"$RUST_VIEW/controller-service/src/lib.rs"
cd "$RUST_VIEW"
sysroot=$("$CHARON" toolchain-path)
export RUSTFLAGS="--remap-path-prefix=$PWD=. --remap-path-prefix=$sysroot=<extraction-sysroot> --remap-path-prefix=$CARGO_HOME=<cargo-cache>"
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --targets x86_64-unknown-linux-gnu \
  --include sandproof_controller_service --include sandproof_protocol --include sandbox_controller_core \
  --start-from crate::proof_restore_activation_carrier::construct_permit \
  --start-from crate::proof_restore_activation_carrier::permit_action \
  --start-from crate::proof_restore_activation_carrier::permit_work_binding \
  --dest-file "$OUT/carrier.llbc" -- --manifest-path controller-service/Cargo.toml --lib --locked --offline
unset RUSTFLAGS
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files -emit-json \
  -namespace SandproofRestoreActivationCarrierRust -subdir SandproofRestoreActivationCarrier \
  -dest "$OUT/generated" "$OUT/carrier.llbc"
```

在独立 Lean 视图复制原 `lean-toolchain`、`lake-manifest.json`、
`SandboxModel/ProofAuditCommand.lean` 和 Lean 服务脚本，复用已核对的原依赖缓存。
将 [私有 Lake 配置](carrier-lakefile.toml.in) 与定理模板分别复制为
`lakefile.toml` 和 `PermitCarrier.lean`，并放入原样生成的
`SandproofRestoreActivationCarrier`。运行 `lake build PermitCarrier` 后，
用同一 Lean 服务核对实际文件诊断和公理审计。保留实际展开的三个原方法，
并从 `translation.json` 拒绝空结果或 opaque 定义。

## 拟验收性质与依赖

以下为待实现的定理接口，`sent` 表示在已连接的 stream 上尝试写激活请求的事件，
其目标由该 stream 的实际对端给出。成功回包和 vCPU 状态是另外的观察。

```text
restore_activation_sends_only_original
  (claim : OriginalClaimCommitted C A action)
  (binding : CompleteBinding C A action work)
  (unknown : UnknownCannotAuthorizeAnotherCandidate C A)
  (read : VerifiedPermitRead claim binding permit)
  (claimStable : SameClaimBirthUntilConnect A trace)
  (linux : LinuxConnectedStreamSemantics)
  (sent : FormalRestoreResumeWrite trace permit stream)
  : PeerVmm stream = ProcessOf A
```

`read` 由 B 的原 SQLite 读侧实现对应提供。`claimStable` 只约束每次
记录读取的身份，由 V6/V9 提供。还需要将实际 `api_stream` 的检查和 I/O
顺序连接到 `linux` 的逐接口保证，才能推出对端结论。

- SQLite 提供指定配置下的提交、隔离和状态可见性。原 DB、单写者、准确行选择、
  memo 对应和错误传播由 Sandproof 提供。未知提交可能已持久。
  [SQLite 隔离](https://www.sqlite.org/isolation.html)
- Linux 提供固定 PID namespace / boot 下的进程观察、连接对端凭据及已连接 stream
  的对象语义。将 PID、启动 ticks 和 boot 的出生身份绑定到真正处理请求的
  VMM，并排除未经证明的句柄移交，属于该调用的接线及部署条件。
  [Linux Unix sockets](https://man7.org/linux/man-pages/man7/unix.7.html)
- 固定 Firecracker 解释送达原 VMM 的 Resumed 动作和暂停装载。I/O 错误或丢回复
  仍允许先前已有动作效果。Guest 授权、就绪、终止竞态和完整生命周期进展分别处理。
  [项目依赖合同](../../../docs/formal/dependencies.md)

## 证据及集成交接

- #636 已交付暂停回执层，#637 的 SQLite/Create 与 #962 的原生事务输入只作邻接
  依据，#638 提供物理路径与故障场景。#838 限于未知激活后的再次资源预留条件。
  [#636](https://github.com/VeriInfra-Lab/sandproof/issues/636) ·
  [#637](https://github.com/VeriInfra-Lab/sandproof/issues/637) ·
  [#638](https://github.com/VeriInfra-Lab/sandproof/issues/638) ·
  [#838](https://github.com/VeriInfra-Lab/sandproof/pull/838)
- #766 的 `FullWorkContract` 对应从已解码的原来源记录组装完整 work。
  固定基线下其 20 项业务输入及六份生成文件摘要一致。两份验收提供方文件已有变化，
  保留原收据身份，不迁签本次结果。
  [原 work 证明包](../stage1-protocol/restore-work/README.md)
- #907 的回执缺失窗口以原 pidfd、已核对 socket 的 GET 与无激活 marker 观察
  原 Paused。#908 延伸相关场景。已有 #832 的独立 KVM 报告在其原候选观察原 VM
  激活后且 SQL 回执未入库的状态。三者均保留原执行身份。故障场景涉及的物理状态
  不等于跟踪本批 Resumed 的发送连接。
  [#907](https://github.com/VeriInfra-Lab/sandproof/pull/907) ·
  [#908](https://github.com/VeriInfra-Lab/sandproof/pull/908) ·
  [#832 原 KVM 报告](https://github.com/VeriInfra-Lab/sandproof/pull/832#issuecomment-6920410625)
- ning 在固定生产输入实跑原生检查 24 项。Runtime 3 项、Node 19 项、SQLite 原许可
  继续与准备只读各 1 项均通过。Runtime 丢回复测试走受控回调。读侧读事务提交错误
  另有上述 1 项原生对照。未新增 KVM 执行。
- 固定 Charon `b104e24f`，Aeneas `505b6ca3`。分离后的 `api_stream/api_reply`
  和带完整激活的根均在 Charon 遇到 `rustix::path::Arg` 翻译诊断。SQLite 生产
  许可读根在 Charon 的转换后类型检查出现 Iterator trait clause 不匹配。
  `activate_same_loaded_candidate` 的 LLBC 成功生成，但原 Aeneas 在
  `snapshot_restore.rs:48–50` 拒绝其错误构造参数。隔离该构造的可见性后仍报错。
  发送根和许可读根缺可导入的 Lean 生成体，尚无实际发送同性质的正误版证书。
- 许可载体已在 ning 通过内核、Lean 服务和三项标准依赖审计。公共登记保持独立。
  身份连续性提供方
  和透明抽取就绪后，公共集成需要登记此包的原生用例、正式入口、实际文件、审计及
  精确输入，并按覆盖记录规范登记结论。当前没有 LC-P7 的发送目标定理。
