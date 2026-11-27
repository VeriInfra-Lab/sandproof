# 协调器的生命周期数据连接撤销选择

[模块范围](../../../../docs/formal/03-protocol-inbox.md) · [收件查询值合同](../inbox-state/README.md) · [证明工作流](../../../../docs/proof-workflow.md)

`ControllerApplication::record_lifecycle_intent` 在应用锁内调用原日志。纯函数 `data_revocation` 使用传入意图的操作以及这次日志调用的完整 `Result`，返回无动作、撤销指定 sandbox、广播三者之一。调用者在同一锁内执行选择并返回原日志结果。原实现对照固定于 #1056 合入提交 `d642a5f45253a716dbb8411a9942b8b9581dd5fb`。

| 传入操作 | 原日志返回 | 决策 |
| --- | --- | --- |
| Unload、Destroy | 当前条目且存在本地映射 | 映射中的 `sandbox` |
| Unload、Destroy | 历史条目或缺少本地映射 | 无动作 |
| Unload、Destroy | `CommitUncertain` | 广播 |
| Unload、Destroy | 其余错误 | 无动作 |
| Create、Resume、Renew | 任意结果 | 无动作 |

## 实际对应

`check.py` 对基线和候选分别抽取完整父函数。`wiring.py` 从真实 LLBC 检查锁、日志参数及原结果返回，核对候选把传入 `intent.request.operation` 和同一个结果交给决策。三个返回变体分别对应空分支、`revoke_data` 的原 sandbox 参数、原锁下 `data_revocations.values()` 的双层循环与 `send_replace(true)`。这是结构化调用接线检查。固定 Aeneas 翻译完整父方法时停在 `ControllerApplication` 的函数指针标记字段。

`guard_binding.py` 跟踪两版父函数从 `self.inner.lock()` 经 `Result::map_err`、`Try::branch` 到 journal 的正常控制流。它按本次 `FnOnce` 实例读取实际错误闭包，检查其返回 `JournalError::ControllerBusy`。在 `Break` 边，父函数把该边的残余交给 `from_residual`，随即返回。在 `Continue` 边，后续 journal 和撤销分支引用的 guard 来自这次成功载荷，guard 的释放发生在分派之后。三项检查器回归分别改变 guard 来源、错误闭包的返回变体和残余实参。该检查记录标准库调用及其操作数，锁的运行时行为与这些标准库函数的实现语义仍需各自的依据。

`journal_hook.py` 核对两版父函数传给原 journal 的第四个实参。它跟踪新建的 `NeverCrash` 经可变借用和再次借用到实际调用，再核对该调用的泛型参数以及 `CrashInjector::hit` 的具体实现。联合抽取把相同实现接到生成的 Lean 方法，核对候选父函数与联合视图所用的完整 Rust 文件及方法起点。两份父函数分别按自己的源码核对。该性质只涉及应用传入的故障注入钩子，数据库和宿主仍可独立失败。

`sender_binding.py` 在同一份父函数 LLBC 中读取透明的 `Inner::revoke_data`，检查正常控制流和调用实参。

- 定向路径使用所选 `SandboxId.0` 查询原 `Inner.data_revocations`。查不到时直接返回。查到时遍历本次查询返回的向量，`next()` 为 `Some` 时，将这次取出的 sender 作为 `send_replace(true)` 的接收者，为 `None` 时退出循环。
- 候选的广播路径在原锁下取同一张表的 `values()`。外层每次 `next()` 返回的向量产生内层迭代器，内层每次 `next()` 返回的 sender 接收同一个布尔字面值。检查器分别核对两层循环的退出和继续位置。

检查器还在候选 LLBC 的副本上逐项断开查表键、查无结果时的返回边、定向 sender、广播向量四处连接，要求对应诊断命中。这些回归只检验检查器自身。这里的证据止于 Rust 实际发起 watch 调用的位置。固定 Aeneas 单独翻译泛型 `Inner::revoke_data` 时停在 Serde 的相互递归 trait 组。迭代库语义、异常退出以及接收端观察均需独立证据。

检查器在私有源码副本为决策与查询各加一个薄根，并为具体的故障钩子加一个两参数转发根，使固定 Charon/Aeneas 在同一次生成中保留实际决策、原 `Inbox::state` 和 `NeverCrash::hit`。复用原 `state_contract`、完整 intent clone 及现有库绑定，直接使用生成的协议操作、日志错误、查询状态和 sandbox ID。

- `never_crash_hit` 对任意 `CrashPoint` 及接收者证明，实际选中的 `hit` 返回 Rust 的 `Ok(())` 并保留接收者。
- `success` 和 `failure` 分别对全部成功值和完整错误枚举给出精确决策。
- `from_indexed_state` 在两个有效索引上把原查询接入原决策，以条目接受版本、会话当前版本及其本地映射表述结果。
- `other_version` 推出任何版本不相等的条目均选择无动作。这里的操作来自父调用的入参；日志返回的 intent 仍是独立字段。

V3 提供已解码查询值与其选择关系。日志事务的实际行来源、未知提交的耐久结局由 V5 提供。真实 watch 送达、网关撤销与在途 I/O 收尾由数据通路提供。上表描述此入口作出的选择；任何成功返回均不能单独说明实际连接已经断开。互斥锁、进程取消和错误载荷沿用[逐符号依赖](../../../../docs/formal/dependencies.md)。

## 复验与登记候选

在证明机干净候选上使用固定工具和现有重锁，给出新的私有输出目录。

```bash
SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1 \
  bash scripts/python-entry.sh verification/experiments/stage1-protocol/coordinator-revocation/check.py \
  --tools "$proof_tools" --packages "$fixed_lake_packages" --lock "$shared_proof_lock" --output "$new_output"
```

`--tools` 下应有原 `aeneas` 与 `lean-mcp`。入口从 Git 解包源树，保留三份 Charon 产物、联合 Lean 生成体、隔离的 Lake 构建、实际文件诊断和审计。两份 `*_Template.lean` 只由翻译器输出，实际工程使用已登记的五个函数绑定和四个类型绑定。

交主控登记这个形式入口及 `SandboxModel.Lifecycle.Protocol.CoordinatorRevocation` 的五条审计。覆盖记录可登记“在已解码 `Result` 和有效收件索引上证明应用撤销选择，原父函数接线经实际 Charon 核对”，适用版本取通过对应检查的候选。生产协调器是现有原生输入，相关登记与冻结由主控结合 #1061 逐闭包复核。

[公共登记候选](registration-candidate.patch)供主控在包含本包的候选上审阅、试套。[覆盖记录候选](coverage-candidate.patch)注明已实测的源码和边界。登记通过 `scripts/verify-stage1-protocol.sh` 默认使用的 `full` 计划调用同一检查器。其原生前提包括已封口的完整 Linux `functional` 日志中的两个指定测试，以及原 `stage1-inbox-state` 收据。单项命令和缺少封口的日志可用于开发复核。

登记子任务在分配的四核内继承并核对原协调锁描述符。检查点读取三份指定 LLBC、真实内核命令的退出状态与日志、物理文件和原服务回执。收敛与 Create 有限计划继续按各自的原任务清单验收。主控合入时需重新校验受影响的公共冻结和完整原生输入。

ning 在同一候选运行普通完整检查，核对现有 `historical_unload_keeps_resumed_route_current_unload_revokes_it` 与新增 `uncertain_commit_selects_broadcast_only_for_access_ending_operations` 两个原生断言组。前者通过原应用和受控端口观察实际通知，后者覆盖不确定提交和普通拒绝的决策边界。完整公共入口和实际网关分别验收。
