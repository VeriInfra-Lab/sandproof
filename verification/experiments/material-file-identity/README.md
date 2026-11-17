# 原材料文件身份的原生错误版对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本入口复用 `sandbox-runtime/src/snapshot_material_retirement_tests.rs` 的三个完整原测试，
不新增或改写 Rust 断言。主要用例在保存原回收计划后，分别加入迟到文件、替换原路径、
放入符号链接、增加硬链接、放入外来条目及写入迟到标记，要求首次 unlink 前拒绝。
正常控制覆盖原部分文件/标记已删除后的幂等收尾；另一控制核计划原本没有的目录不能
被后来出现的同名目录取代。

私有错误版只修改实际 `verify_leaf` 的一个匹配守卫：已有计划项时，不再比较当前文件的
完整 `FileStamp`。路径和计划仍存在，原普通文件/owner/链接数/mode 检查、目录身份、
文件集及无计划项的拒绝保持。原 `FileStamp` 包含 dev/inode、owner/group、mode/链接数、
长度及修改/变更时间，不是内容摘要或真实文件材料完整性的证明。

三版都须实际编译当前 Runtime libtest。错误版须在原主要用例的同一 `is_err()` 断言
退出101，两个控制仍通过；正确和恢复版各通过全部三项。完整原测试文件和其它全部
提交输入逐 mode/blob 相同。编译失败、超时、忽略、零测试、其它 panic 或更换断言均
不能记为性质拒绝。每版执行前保存原实际测试程序，并核对 fresh=false 与程序摘要。

这是 E9/E11 的计划与原文件身份接口代表，使用真实私有文件系统和原实现；没有 reader
并发、真实 VMM/backing、SQL 删除许可、额度结算或 KVM。它不代替“检查引用后无保护
删除”的全部业务错误版、外部使用者排空或六类跨层矩阵，也没有新 Lean/抽取证明或
公共 `proof_native_suite` 登记。源码和日志中的有限反例与系统验收分别报告。

## 复验

复用[共享编译与来源核验入口](../unknown-host-release/native.py)，保留原 journal 和
[namespace FD](../namespace-reference/README.md) 调用者。当前入口只提供固定的包、
实际源码及原断言，不接收外部算法、测试或编译选项。

从干净精确候选导出 `git -c tar.umask=0022 archive --format=tar HEAD` 并记录摘要。
按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)分配自有0700空 payload；
证据和现役缓存放在 payload 外，使用原重锁、固定 Rust/Cargo1.97.0 和显式离线
`CARGO_HOME`。默认 libtest 并行、原断言和期限保持，不重试未知失败。

```sh
bash scripts/python-entry.sh verification/experiments/material-file-identity/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

入口保存完整提交输入、编译JSON、原stdout/stderr、退出、断言位置及实际程序副本。
只更新私有原 `lib.rs` 的时间戳以重编译，不改字节/inode/mode 或清理缓存。失败仍按
原状态保存；原执行者确认进程/资源收尾后退休 payload，保留源码宽限期和独立证据。
普通完整检查、CI、ning 最终组合和实机验收均另记，不从此有限对照推出系统完成。

不调用 Cargo 的检查器回归：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_material_file_identity_native
```
