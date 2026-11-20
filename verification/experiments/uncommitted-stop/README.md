# 未提交快照不能先停止来源的原生对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本包复用当前 `capture::tests::early_stop_without_registered_handoff_is_refused_by_real_worker`。
原测试建立真实核心 World，完成冻结/封存但尚未提交快照，向原队列登记并派发同源 Stop，
要求原 `handle_worker` 返回 `SnapshotNotReady`。不复制状态机或修改原 Rust 测试。

私有错误版只旁路 `worker::pause_stop::stop_vm` 的整段快照 gate：包括完整捕获交接、
目标快照及其来源/不可变/已持久条件。此前的 sandbox、epoch、generation、当前 VM、
Frozen 和原非终止 fence 检查保持。不是只删一个被其它同义检查重复保护的条件：
错误版必须实际产生 `Ok(WorkerDecision { event: StopVm, ... })`，并在原 assert_eq! 的
准确位置被同一个 `Err(SnapshotNotReady)` 合同拒绝。其它错误、错误来源或只改变拒绝
种类均不算该反例。正确和恢复版通过；已提交的合法卸载及普通失败保留原 VM 两个控制
用例在三版均须通过。核心返回 Stop 决定不表示宿主已实际停止 VM。

复用[原生运行器](../unknown-host-release/README.md)的完整 Git 归档校验、固定
Rust/Cargo 1.97.0、原共享重锁和独立源码。三版实际 libtest 均须新编译，绑定实际
package/manifest/src，`test`、`debug_assertions`、`overflow_checks` 全为 JSON 布尔 true，
`fresh=false`。每版执行前保存真实二进制，完整原测试文件和其余全部提交输入保持；
错误 Rust 保留在本批隔离源码宽限期，不能把错误版合入生产。

本片是“快照未提交先 Stop”的实际 Rust worker 门代表，尚未运行时不预记结果。
队列/World/worker 为真实实现，宿主效果仍未执行；它没有真实文件、VMM、中心事务、
协议收件、KVM 或新增 Lean/抽取证明，不代表完整第二类跨层错误、六类矩阵或系统完成。
完整系统仍须检查原 VM、材料/backing、围栏、退出与结算，不能从 worker 决定推出这些事实。

## 执行

从干净精确候选导出归档。先按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)
分配自有 0700 空 payload。复用原缓存和重锁，证据目录位于 payload/缓存之外且尚不存在。
显式保留原 CARGO_HOME；不改测试并行、期限或断言，不清理共享缓存。

```sh
bash scripts/python-entry.sh verification/experiments/uncommitted-stop/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

原始编译 JSON、完整输入、实际程序、stdout/stderr、三版真实退出与拒绝位置须保全。
编译错误、零测试、忽略、超时、其它 panic 或环境失败不算性质拒绝，不自动重跑改签。
结束后由原操作者确认精确资源与全机引用，再按原状态退休；保留源码宽限期和缓存。
最终组合普通/CI及必要原生复验交执行方，本入口不登记为 `proof_native_suite` 形式封口。

无 Cargo 的检查器回归：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_uncommitted_stop_native
```
