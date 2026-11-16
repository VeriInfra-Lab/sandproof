# 激活未知后的再次预留原生对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本片复用原 `unknown_activation_survives_restart_and_never_requests_another_load`。
实际 reducer 历史完成 Resume、Prepare/Network、Activate dispatch/Worker/Timeout，
再 Crash/Recover；同一原测试要求保留原 VM、分配游标与未知激活，并拒绝再次
`ReserveStartup`。完整原 Rust 测试不变，成功效果仍由原受控宿主输入提供。

私有错误政策将当前恢复计划的完整同源 Activate `OutcomeUnknown` 误当再次预留许可：
对应原 operation、sandbox、epoch、generation 和 VM，且原计划有来源和位置时，
同时旁路 sealed、已分配 VM、已分配 slot 三处同义门。其它 controller/phase/terminal、
当前操作/计划、来源对应、全局资源新鲜性及算术边界保持；不修改运行态或伪造输入。
错误版须实际编译并在原末尾 `.is_err()` 断言被拒绝，较早拒绝、其它 panic、编译失败、
零测试、忽略或超时均不算反例。正确与恢复版使用同一合同并通过。

原 Resume fence 后继续、普通启动首错顺序、两轮恢复不复用旧 VM，以及未知激活被
当前 Kill 覆盖后正常终止四个控制在三版均须通过。不以全部拒绝满足性质。
原 checkpoint Resume 文件和 scheduler 测试体分别绑定，完整其它提交输入逐一核对。

这是第三类“激活未知后重新 load 旧来源”的**再次资源预留决定**代表。原断言即使失败，
也不表示已应用新决定、完成另一轮加载或执行了第二个 VM；原来源、checkpoint 合法性、
真实 driver/node/Load、材料使用及完整系统交错继续分别验收。没有新增 Lean/抽取，
不代替第三类全部路径、六类矩阵或 KVM，实际结果绑定精确提交另行记录。

## 执行

复用[共享原生运行器](../unknown-host-release/README.md)和严格产物准入。从干净精确
候选导出完整 Git 归档，按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)分配
自有 0700 空 payload。证据与原共享 Cargo 缓存放在 payload 外；固定 Rust/Cargo
1.97.0、原 CARGO_HOME/重锁、离线依赖、默认测试并行和期限保持。三版实际 Core
libtest 均须新编译；原 Cargo 的 test/debug_assertions/overflow_checks 全为 JSON
布尔 true，fresh=false，package/manifest/src 和实际程序须匹配本副本。

```sh
bash scripts/python-entry.sh verification/experiments/unknown-activation-reservation/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

原始编译 JSON、完整输入、实际程序/退出及原断言位置保留。失败不自动重试或改签；
原执行者确认完整进程和精确资源结束后，按原状态退休，保留源码宽限期、原始证据及
现役缓存。最终普通/CI和必要原生复验交 ning，本入口不登记为 `proof_native_suite`
形式准入或 V3 完整封口。

无 Cargo 的检查器回归：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_unknown_activation_reservation_native
```
