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
