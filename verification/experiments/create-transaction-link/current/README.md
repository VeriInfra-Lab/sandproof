本目录核对从已开始事务进入真实 `StandardRecord` 的 Rust 调用。当前验证入口会在私有副本中加入原参数直转的抽取根，用固定 Charon 和 Aeneas 重新抽取调用方，并比对完整的生成 Types/Funs。独立 Lake 视图从已登记提供方取精确证明文件。实际生成函数、只读访问、错误转换、录入与提交连接按当前闭包进入原 Lean 内核、实际文件服务和依赖审计。

`rust-source-binding.json` 固定公开 `ControllerJournal::record_lifecycle_intent` 的来源。它先用原连接和已提交缓存开始事务，再把同一事务、Journal 的 namespace 和 `StandardRecordCodec` 移入 `StandardRecord`。五个生产字典是 `StandardRetirementEncoding`、`StandardTerminalEncoding`、`StandardRenewalEncoding`、`StandardUnloadEncoding` 和 `StandardGraphicBytes`。证明保留这些字典的调用与完整输入。SQLite 及 Serde 的固定接口合同供应实际字节、错误、事务和可表示条件。Coordinator 选 `NeverCrash`，通用 Journal 仍向同一状态 `C` 传入原 `BeforeCommit` 和 `AfterCommit`。

- Replay 经过完整载入及验证，独立认证和完整请求查找成功后提前返回。它不走显式提交。正常自动析构和回滚由独立边界说明。
- 新 Create 的两个 Destroy 判定为假。原追加和保存返回的完整 `StandardRecord` 被移入原提交，两个提交结果都保持正确的异常传播和同一缓存观察。四项 V5 检查仍分别有待履行的内部合同。正常 Ok 和 Err 上的物理事务、单元格与借用条件沿原 A10 和 SQLite/B4 合同。单纯比较 Lean 返回值无法得出这些物理条件。
- BEGIN、SQLite 自身实现、自动析构派发、服务外层与完整 V3 均有独立验收范围。
- ning 端保留原 Codec 输入、调用顺序和 D 状态断言。交换原提交中两个单元格引用的隔离错误版须被同一个缓存一致性断言拒绝，随后恢复对照须通过。这个反例检验实际单元格接线。

在 ning 的原 native 环境中，使用 `bash scripts/python-entry.sh tools/proof_native_suite.py run --scope stage1-v3-transaction-source --output <new-native-evidence>`。该固定范围包含完整普通功能检查、实际事务断言和实际来源行断言。按原证明机入口传入同一提交的 native seal 后，在 MSRA 运行 `bash scripts/verify-v3-transaction-source.sh <new-formal-evidence>`。普通测试和形式化检查分别在相应机器执行。原始 proof 运行和当前公共验收的执行身份分别保留。

依赖接口的完整条件见 [V3 事务录入与来源行的边界](../../../../docs/formal/v3-transaction-source-boundaries.md)。
