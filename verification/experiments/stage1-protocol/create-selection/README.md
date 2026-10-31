# Create 前置选择：当前真实 Rust

[V3 设计](../../../../docs/formal/03-protocol-inbox.md) · [公共验收](../README.md#固定-create-交付) · [依赖合同](../../../../docs/formal/dependencies.md)

当前检查器直接调用生产 `create_attachment_selection`，薄根转发完整 Inbox 和
AcceptedLifecycleIntent 的两个共享借用。正确版不再应用 `candidate.patch`；原补丁
只保留来源。准入核对实际函数体、两个参数来源、完整 JournalError/ContractError、
生成函数及全部物理文件/审计。固定 Charon/Aeneas 不修改，生成体不手改。

## 同一完整合同

`CreateSelectionModel.FullSelectionContract` 从已解码完整值开始，使用现有 `InboxLookup.inboxSpec`：未找到或全文冲突返回 Conflict；找到原位置后依次检查 terminal、refusal、当前版本不等于 accepted version、已有 local，任一成立拒绝。随后只允许 Create，正常返回原两个位置和其原 startup deadline。合同不添加唯一性、非零或相邻代等条件。

`CreateSelection.selection_complete` 对任意完整输入证明实际生成函数满足该合同，复用原 `InboxLookup.find_outcome` 和 `InboxPosition.inbox_found_reads`，从原查找结论推出真实索引边界。Model/Values 在正确和错误视图中逐字节相同；错误视图不导入正确业务对应定理。正常与 terminal/refusal/旧版本/local 见证均使用同一规格；正常 intent 的完整封套形状另有证明。封套合法不推出整个 Inbox 可达、认证成立、已持久接受或实际资源存在。key window 与 retention window 没有额外相等要求。

五个错误版分别在真实 Rust 上删除 terminal、删除 refusal、把版本不等比较改为小于、删除 local、把返回 startup deadline 改为 timeout。每版须实际编译、抽取，再以同一合同证明否定。编译、导入失败或超时不算性质反例。

## 交付边界

原九条 Rust 断言用于正确版、五个真实错误版和恢复版共七个视图；每个错误版只在
已核函数范围内变异，须编译并触发同一断言，编译或 Clippy 失败不能算反例。
当前有限正式交付由公共 `create-delivery` profile 纳入，本目录裸结果不能充当宁封口。

这里证明只读选择，不从封套相等推出认证、持久接受、资源存在或整个 Inbox 可达。
owner/安装/寿命/ID/SQL/save 的其它包分别检查，完整事务、提交不确定与恢复仍有
独立责任；不把整个 codec、HTTP 或事务设为可信。普通值与标准函数条件见依赖合同。
原 AE/candidate 的证据保持原提交身份，不迁记为当前实现通过；精确当前候选、原始
RUN/REUSE 和验收结果由 #635 与交付 PR 登记。
