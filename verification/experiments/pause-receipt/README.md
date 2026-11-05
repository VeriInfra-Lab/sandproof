# 暂停回执的准入与账本写回

本组件归 [V4/#636](https://github.com/VeriInfra-Lab/sandproof/issues/636)。
规格见 [V4 详细计划](../../../docs/formal/04-local-core.md)，实际适用版本和通过范围
只在[覆盖记录](../../../docs/proof-coverage.md)维护。驱动和文件准备完成不等于验收通过。

## 检查的实际实现

使用原 `effect::receipt_pause`、`receipt_awaiting_result`、`receipt_finish`、
`effect_outcome_valid`、`command_result_present` 及 `acknowledge_effect`。
原 CaptureCaller 的十九个根加回执根，共用同一个完整 World 和固定工具/十一项库模型。
不新增业务不透明接口，不用“回执有效”假设替代实际准入判断。

首次 Dispatched/OutcomeUnknown 回执按原顺序检查：Succeeded 要有对应结果，Failed
要没有对应结果；Stop 使用原 VM、sandbox、generation 和 Stopped/Reclaimed，Commit
使用原 snapshot、sandbox、source_generation、manifest 和 Durable/immutable。
相同重复回执返回原 World，不重新查询可能已经变化的结果；冲突和其它状态保持原拒绝。
准入后沿原完整 EffectId/outcome 写回，并复用原派发/确认与暂停发布证明。

这些是逻辑 World 中的记录，不自行证明真实 VM 已停止、文件已经耐久或账本已经提交。
持久事务、实际宿主事实、外层一般回执处理器和系统进展仍由所属模块提供。

## 唯一真实错误版

普通检查和形式捕获都调用同一个 `native.py::mutate`：只跳过首次 Succeeded
回执的结果检查，保留 Failed 检查、原状态/重复/冲突分支及原账本写回。
错误版必须先成功编译/抽取，再在 VM 仍 Frozen 时被正确版同一性质拒绝；
编译、导入、超时或工具失败不能代替业务反例。

正确生成物保持原 CaptureCaller 库；错误生成物保存于
`verification/aeneas/pause-receipt/wrong-outcome/CaptureCaller`，使用相同 namespace。
错误证书以 `.lean.in` 保存原来源，在独立错误项目按原逻辑模块路径恢复成 `.lean`，
该私有项目的实际库输入须与已核对的错误生成物逐字相同。两个视图不能混合导入，也没有 World 转换。
模板本身不算已检查的文件；正式入口须检查实际恢复文件、完整导入闭包和全部审计。

## 有限正式入口

普通检查机执行原生 `--scope pause-receipt`，固定六个组件：完整 Linux functional、
capture-caller、pause-publication、pause-stop、pause-dispatch 和本回执组件。
每个旧组件保留自己的正确/错误/恢复版和原断言；完整 functional 在同一稳定候选上
按原收据执行或严格复用，不另造成功标签。该范围不提供 V3 或全项目封口。

```sh
bash scripts/python-entry.sh tools/proof_native_suite.py run \
  --scope pause-receipt --output /private/acceptance/pause-receipt-native
```

证明机使用原固定 wrapper、重任务锁、工具与原服务客户端，消费完整原生证据：

```sh
export SANDPROOF_ACCEPTANCE_ROLE=formal
export SANDPROOF_NATIVE_EVIDENCE=/private/acceptance/pause-receipt-native
bash scripts/verify-pause-receipt.sh
```

原外层执行器须提供 `SANDPROOF_FORMAL_RUNTIME` 和 `SANDPROOF_FORMAL_CACHE`，以及
`SANDPROOF_PAUSE_RECEIPT_CORRECT_CAPTURE`、`SANDPROOF_PAUSE_RECEIPT_WRONG_CAPTURE`。
后两者指原工具已完成的两份捕获；入口核对原命令/日志/完整生成物和原Git编译输入的
内容/模式，只能复用仍匹配的捕获，不把开发结果改签为正式证明。编译输入改变时拒绝，
不能用刷新摘要或旧产物消除失配。

可显式提供 `SANDPROOF_PAUSE_RECEIPT_CORRECT_PROJECT` 和
`SANDPROOF_PAUSE_RECEIPT_WRONG_PROJECT`，沿同一受控私有项目保留 Lean 缓存。
最终源归档的所有文件和执行位仍重新准备/核对，已知旧负例草稿单独保留，
不清理 `.lake`，也不修改原Git身份。当前内核、全部审计和实际文件服务仍须完成。
入口通过既有批次 `step` 验收四个受影响旧模块，保留其 RUN/REUSE 和原始证据，
再检查本组件。缺少最终数学文件、实际生成物、必需声明、冻结输入、原生封口或
任一视图的内核/完整服务结果都应拒绝；不以空清单或局部开发结果代替正式验收。
