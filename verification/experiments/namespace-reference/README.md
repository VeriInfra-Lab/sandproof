# 命名空间 FD 引用的原生错误版对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本入口复用当前 `sandbox-runtime/src/retirement.rs` 的三个原 Rust 测试，不增加或改写
Rust 断言。主要用例依次检查命名句柄消失后的线程成员、只剩 FD 的引用和最终无引用；
两个控制分别检查观察边界的 FD→成员转换，以及同 inode 不同设备和不可观察 FD 表。

私有错误版只在原 `require_task_namespace_unheld` 中，把一次 FD 的 dev/inode 检查
替换为丢弃该路径，保留原枚举、线程成员检查和全部其它输入。同一原用例必须在
FD-only 的拒绝断言处实际退出101，两个控制仍须通过；正确、错误、恢复三版均须
新编译实际 Runtime libtest。编译失败、超时、忽略、零用例或其它 panic 均拒绝。

生产检查和原测试恰好在同一个 Rust 文件。驱动另外绑定整个原 `#[cfg(test)] mod tests`
的字节摘要，三版必须相同；错误版整个文件的摘要不同是预期差异，不能把它误报为
“完整源文件未变”。原断言行由未改的测试文本定位。共享入口仍绑定完整 Git 树、
Cargo package/manifest/src/test profile、fresh=false 和实际二进制，不清理共享缓存。

这是资源引用扫描的有限原生对照。原 Rust 测试用普通文件/hard link 和受控 proc 形状
表示 dev/inode，不运行真实 procfs 扫描、命名空间、VM 或 KVM，也不执行位置释放/额度
事务。它与[未知宿主释放](../unknown-host-release/README.md)的 journal/SQLite 接口代表
分别保留；二者都不能替代 #825 的实机顺序、完整第六类错误版、全部外部引用或六类
跨层矩阵。没有新 Lean/抽取证明或 `proof_native_suite` 形式准入登记。

## 复验

沿用[原共享编译/来源核验入口](../unknown-host-release/native.py)，两个已提交调用者
使用不同的固定 Rust 包、源码和断言。原 journal 默认配置保持；新增 Runtime 调用不
接收外部算法、测试或编译选项。两种入口都要实际正确/错误/恢复对照，不能只运行检查器自测。

从干净的精确候选导出 `git -c tar.umask=0022 archive --format=tar HEAD`，记录归档摘要。
按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)分配自有0700空 payload；
证据与现役缓存位于 payload 外，使用原重锁、固定 Rust/Cargo1.97.0 和显式离线
`CARGO_HOME`。保留默认 libtest 并行、原断言和期限，不重试未知失败。

```sh
bash scripts/python-entry.sh verification/experiments/namespace-reference/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

每版只更新其私有原 `lib.rs` 的时间戳以实际重编译，inode/字节/权限保持；仍核对
Cargo fresh=false。保存全部输入、原编译JSON、实际stdout/stderr、退出、断言位置及
二进制摘要。失败保留原状态；确认进程/资源收尾后由原操作者退休 payload，保留缓存、
原始证据和源码宽限期。普通完整入口、CI、最终 ning 组合和 KVM 分别报告。
本候选还在每版执行前保存通过编译准入的原测试程序副本，并核对其摘要，避免后继构建
复用缓存路径后丢失错误版程序。历史仅记录摘要的回执保持原范围，不迁记为已有程序副本。

不调用 Cargo 的检查器回归：

```sh
bash scripts/python-entry.sh -m unittest \
  tools.tests.test_lifecycle_unknown_host_release tools.tests.test_namespace_reference_native
```
