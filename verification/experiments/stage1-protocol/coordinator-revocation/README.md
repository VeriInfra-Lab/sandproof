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

检查器在私有源码副本加两个单调用薄根，使固定 Charon/Aeneas 在同一次生成中保留实际决策和原 `Inbox::state`。复用原 `state_contract`、完整 intent clone 及现有库绑定，直接使用生成的协议操作、日志错误、查询状态和 sandbox ID。

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

交主控登记这个形式入口及 `SandboxModel.Lifecycle.Protocol.CoordinatorRevocation` 的四条审计。覆盖记录可登记“在已解码 `Result` 和有效收件索引上证明应用撤销选择，原父函数接线经实际 Charon 核对”，适用版本取通过对应检查的候选。生产协调器是现有原生输入，相关登记与冻结由主控结合 #1061 逐闭包复核。

ning 在同一候选运行普通完整检查，核对现有 `historical_unload_keeps_resumed_route_current_unload_revokes_it` 与新增 `uncertain_commit_selects_broadcast_only_for_access_ending_operations` 两个原生断言组。前者通过原应用和受控端口观察实际通知，后者覆盖不确定提交和普通拒绝的决策边界。完整公共入口和实际网关分别验收。
