# 未知宿主释放的原生反例

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本包固定当前真实 `ControllerJournal::release_host_slot`，复用已有协议 4 的两 owner、
两轮恢复、位置复用及重开用例。同一个原测试在宿主释放回调返回 `ReplyLost` 后要求拒绝，
保留 Releasing，后续只有原确认成功才能继续恢复。另保留提交前不调用宿主、丢确认后
仍可接续，以及两种提交未知结局的原控制用例。不复制业务算法或修改 Rust 断言。

错误版仅在私有源码副本中把真实释放回调的错误传播改为忽略结果，随后继续原确认。
正确、错误、恢复三版均编译实际 service libtest；必须绑定该副本的 package、manifest、
src、test profile、新编译标记及实际二进制。错误版必须在原 slot-reuse 测试的准确拒绝
断言处退出 101，另两个控制用例继续通过。编译失败、零测试、忽略、超时、其它 panic
均不是反例成功。三版使用相同的完整原测试文件及全部其余提交输入。
每版在构建前只更新本副本原 `lib.rs` 的时间戳并记录前后元数据，保持 inode、字节和
权限不变，促使 Cargo 实际重编译；仍检查原编译 JSON 的 `fresh=false`，不清理共享缓存。

本片是第六类错误版中“未知宿主释放回复不能当确认”的同步 journal/SQLite 接口代表。
`ReplyLost` 是受控宿主端口结果；该用例没有实际查询宿主进程，不把进程缺席、超时或
回调返回当作内核退出。它没有真实 VM、文件映射或 KVM 验收，不补造 Host 成功事实，
没有新增 Lean/抽取证明，也不代表六类跨层错误版、整个 E9 或完整生命周期已覆盖。

## 执行

从干净的精确候选导出 `git -c tar.umask=0022 archive --format=tar HEAD`，记录其 SHA-256。驱动逐成员
核对当前完整 Git 树的对象和执行位；缺失、额外、链接或变动输入均拒绝。沿用固定
Rust、离线 Cargo 和原重锁，不重装依赖，不设置 `RUST_TEST_THREADS`，不改期限或断言。

先按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)分配 owner/0700 的空 payload，
把它作为 `--work-root`。共享 Cargo 缓存在 payload 之外，`--target` 指向原已批准缓存，
`--output` 是缓存及 payload 之外尚不存在的私有证据目录。不得复用未知写入目录。

```sh
bash scripts/python-entry.sh verification/experiments/unknown-host-release/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

原始编译 JSON、stdout/stderr、每次真实退出、完整输入、二进制及断言位置保留在证据中。
失败不重试或改签成功。确认原进程/资源收尾后，由原操作者按实际状态退休 payload；
不由驱动自动清理、不删除共享缓存。普通完整入口和最终 ning 验收分别执行、分别报告。
本入口尚未登记为 `proof_native_suite` 的形式准入 case，其结果不能作为 V3 或 `all` 封口。

无 Cargo 的检查器回归：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_lifecycle_unknown_host_release
```
