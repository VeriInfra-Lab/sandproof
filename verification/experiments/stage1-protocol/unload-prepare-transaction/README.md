# 首次卸载来源观察的 SQLite 夹具

本夹具固定 integration `f26fc26a3fb6987a18ad69015c56bb99c0748e12` 的 Rust 与 Cargo.lock。
沿真实 `ControllerJournal::prepare_lifecycle_source_work` 入口，复用原 Create 完成、Unload
收件和 Host epoch 测试提供方，覆盖 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)
从已解码接受值到首次登记来源观察要求的两个有限原生场景。

- **首错及重放。** 临时库的原写连接保存错误 allocator 字符串与实际 REAL revision。
  外层 begin 首先拒绝 allocator。修复该列后，外来中心上下文先于 revision 拒绝。
  正确上下文读到真实 SQLite revision 类型错误。另一连接核对原 Inbox 字节与原工作集合，
  再恢复原整数并调用真实入口。已有工作时，再设同一错误 revision，完整重放仍返回
  原工作，最后恢复原整数。此路径检查 revision 的读取，工作锚点自身没有单独的 revision 字段。
- **提交边界。** 在原 `BeforeCommit` 中，另一连接以零等待尝试取得写事务，实际得到
  `SQLITE_BUSY`，并且只能读到提交前的 Inbox 与工作集合。注入的提交前失败退出后，
  原连接回到 autocommit，外部仍看不到新工作。同一请求重试，`AfterCommit` 时另一连接
  能取得写事务，并读到唯一的完整工作，等于本次返回值。重开原库后，同请求仍返回
  同一条工作。观察连接没有写入业务数据。

这组检查提供当前夹具下的物理锁、可见性与实际错误证据。原始 [#890](https://github.com/VeriInfra-Lab/sandproof/pull/890)
是直接使用内层 `base_snapshot` 的夹具。外层尚未到达该调用时，已有更早的 begin 与认证
错误。候选 World 的通用来源、完整读写帧、未知提交及回滚失败仍由
[V5 的具名合同](../../../../docs/formal/dependencies.md#v5inboxtransactionframe具名内部待证条件不能转为外部信任)
和对应实现提供。这里使用受控的中心上下文和 Host 来源，范围不包括 TLS、物理资源或
已解码值之前的字节接口。

## ning 运行

在该 PR 的干净独立工作树中，仅加载一次原生夹具，使用仓库固定的离线 Cargo 环境。

```bash
set -euo pipefail
test -z "$(git status --porcelain)"
target=controller-service/src/controller/lifecycle_inbox/source_work/tests.rs
fixture=verification/experiments/stage1-protocol/unload-prepare-transaction/native-tests.rs.in
sha256sum Cargo.lock "$target" "$fixture"
{
    printf '\n'
    cat "$fixture"
} >> "$target"
sha256sum "$target"
cargo test --locked --offline -p sandproof-controller-service --lib proof_first_unload_prepare_ -- --list
cargo test --locked --offline -p sandproof-controller-service --lib proof_first_unload_prepare_ -- --nocapture
```

定向记录应包含 `proof_first_unload_prepare_writer_and_visibility` 与
`proof_first_unload_prepare_first_error_and_replay` 两条真实用例。完整普通与 V3 原生
验收继续使用[证明工作流](../../../../docs/proof-workflow.md)原入口。

## 真实 `BEGIN IMMEDIATE` 的写锁首错

`begin-contention.rs.in` 为同一原入口补第三条用例。用例先由原连接把输入前沿设为
无效字符串，然后用 SQLite authorizer 在准备原 `BEGIN` 语句时，
让独立连接取得同一个数据库的写锁。此前的 WAL 截断和容量维护仍执行原实现。
原连接零等待，须在读输入前沿前返回原 SQLite `SQLITE_BUSY`。另一只读连接核对
原 Inbox 字节与空工作集合仍相同。释放竞争锁后，原入口报告原前沿错误；恢复前沿后
才登记唯一完整工作，重放取得同一条工作。此用例没有调用真实来源 provider。

从干净的独立工作树开始，另开一个同提交工作树给错误版。两者都追加相同夹具，
`rusqlite/hooks` 仅在此次测试命令启用，提交内无 Cargo 配置变更。

```bash
set -euo pipefail
test -z "$(git status --porcelain)"
target=controller-service/src/controller/lifecycle_inbox/source_work/tests.rs
cases=verification/experiments/stage1-protocol/unload-prepare-transaction
sha256sum Cargo.lock "$target" "$cases/native-tests.rs.in" "$cases/begin-contention.rs.in"
{
    printf '\n'
    cat "$cases/native-tests.rs.in" "$cases/begin-contention.rs.in"
} >> "$target"
sha256sum "$target"
cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks \
  --lib proof_first_unload_prepare_begin_busy -- --list
cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks \
  --lib proof_first_unload_prepare_begin_busy -- --nocapture
```

错误版在追加相同夹具之后，仅把 `controller/transaction.rs` 中 `WorldTransaction::begin`
的 `TransactionBehavior::Immediate` 改为 `TransactionBehavior::Deferred`。保留其它
BEGIN 和全部断言。先用同组选项执行 `cargo test --no-run`，再运行同一个用例并保留
实际失败日志。编译须成功，错误版须在原 `SQLITE_BUSY` 断言处报告 `Err(CorruptStore)`。
错误版能打开 Deferred 读事务，随后读到同一错误前沿，这会违反原首错要求。
编译失败、超时或其他失败均不计为性质拒绝。

这项有限原生对照绑定同一固定 Rust 输入和追加的夹具。事务开始的
真实抽取、连接到 `from_started_transaction` 的形式证明，以及完整 V3 验收仍分别取证。

## 逐行写入和首错的原生验收

`codec-rows.rs.in` 保留原三条逐行写入测试。原工作键保留已有行，缺键时使用最小空槽。
SQLite 返回零个更新行时保存必须报错。第二行拒绝时，第一行已写入的内容必须随原事务一并回滚。
本组与上述真实写锁用例共用一次正例编译。三个错误版本分别在私有副本改变更新行数判断、
第二行错误后的提交和事务开始方式，成功编译后须触发对应原断言。

在已准入的 ning 普通测试环境，使用 `/data2` 上的私有工作盘和 `/data1` 上的新证据目录。

```bash
SANDPROOF_ACCEPTANCE_ROLE=native bash scripts/python-entry.sh \
  verification/experiments/stage1-protocol/unload-prepare-transaction/native_rows.py \
  --work-parent "$native_work_parent" --output "$native_evidence"
bash scripts/check-function.sh
```

验收脚本只在独立源码副本中安装夹具，保留原始源码归档、每版实际代码、编译和断言日志。
四版共用一个私有 Cargo target，测试结束后收回。`source_work/tests.rs` 的提交字节保持与
原正式入口的冻结一致。此入口提供 SQLite 回归证据，证明覆盖和原公共正式入口单独记录。
