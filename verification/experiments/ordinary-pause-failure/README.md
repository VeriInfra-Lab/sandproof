# 普通 Pause 失败的严格原生对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

沿用[捕获交接](../capture-handoff/README.md)已有的 `terminal-pause` Rust 错误政策和
`ordinary_failure_keeps_live_original_and_invalidates_already_queued_freeze` 原测试。
本片补充具体断言、恢复版、正常控制和当前严格产物准入；旧入口、原证明/抽取、冻结与
历史结果保持，不把已有性质计作新定理。

新入口直接读取原 CASE 定义并调用原变异函数，先检查唯一片段确在
`begin_compensation`。私有错误版将普通 Pause 导向原终止补偿分支；完整原 capture
测试不改。只有原测试经过实际补偿后，在 `assert_original_kept` 的
`assert_eq!(held.phase, Phase::Pausing)` 观察到 `Revoking`，才算本入口的性质拒绝。
仅有用例名 FAILED、更早 unwrap、不同阶段/断言、编译失败、忽略或超时均不算反例。
正确与恢复版必须使用同一完整原合同并通过。

正常 Pause 每步 checkpoint 往返、显式 owner Destroy 保留材料，以及失败启动的精确
abort/发布/未知准备保留三个原控制在三版均须通过，不以全部拒绝满足性质。
完整 capture 测试文件及其它提交输入分别检查。原生结果绑定精确候选另行记录。

这是第一类错误的 **Core 补偿决定**有限代表。错误世界误入 Revoking 后即在原断言失败，
没有实际派发终止 worker、停止真实 VM、执行物理捕获或运行完整继续原 VM 的路径；
成功效果来自原受控输入。它不代替第一类全部路径、六类矩阵、KVM或新增Lean证明。

## 执行

采用[共享严格原生运行器](../unknown-host-release/README.md)。旧批入口不在本调用链中；
其历史线程/缓存选项和四组结果保持各自来源。本入口使用固定 Rust/Cargo 1.97.0、
原 CARGO_HOME/共享缓存/重锁、离线依赖、默认测试并行和期限，不覆盖编译选项。
三版实际 Core libtest 均须新编译，原 test/debug_assertions/overflow_checks 全为 JSON
布尔 true，fresh=false，package/manifest/src及实际程序对应本副本。

从干净精确候选导出完整 Git 归档，按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)
分配自有 0700 空 payload，证据与缓存位于 payload 之外：

```sh
bash scripts/python-entry.sh verification/experiments/ordinary-pause-failure/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

保留原始编译 JSON、输入闭包、实际程序与退出、原断言位置和日志。失败不自动重试或
改签；原执行者核对宿主进程/FD/挂载无引用后按原状态退休，保留源码宽限期、独立证据
和共享缓存，可重建程序登记有限复核期限。最终组合与必要实机验收交 ning。本入口
不登记为 proof_native_suite 或 V3 完整封口。

无 Cargo 的检查器自测不作为真实 Rust 反例：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_ordinary_pause_failure_native
```
