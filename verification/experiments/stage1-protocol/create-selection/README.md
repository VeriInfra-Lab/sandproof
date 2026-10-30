# Create 附接的只读前置选择候选

[V3 设计](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本目录是精确 AE `ae8e5a564ab3d81652a23acb2b99e242193fa3a2` 上的后续独立证明输入，未进入 AE+65 当前封口。生产采用、普通/原生验收、公共登记和完整 V3 均未完成。它不修改生产或公共配置；`candidate.patch` 仅应用到检查器新建的私有副本。

## 对应的真实代码

原 `attach_create` 全体 SHA 为 `708dddf1e811d096b0492e85283d146b93a28c60e73ab4cc1b057a32d9ad2268`；已核对 AE 与冻结调用者 `65cf8a33cb284562f1e986d10db2dfa986bdefea` 的该体一致。候选把原 load 后的全文查找、四个状态守卫及 Create 匹配移入 `create_attachment_selection`，返回原 `(session index, entry index, startup_deadline_ms)`。原父函数调用它后继续原 owner 分配、绑定、SQL 和 save；没有复制查找算法或改写生成体。

每个视图都重新编译并用固定 Charon/Aeneas 抽取真实候选。透明转发根只传入原完整 Inbox 和 AcceptedLifecycleIntent 的两个共享借用；准入检查原参数顺序、实际透明业务根、完整错误类型和精确生成闭包。独立父函数 MIR 预检及两个借用来源的复核属于源码/MIR 连接证据，**不等于原父函数的一般等价或完整 Lean 证明**。生产重构需由主控指定唯一负责人采用。

## 同一完整合同

`CreateSelectionModel.FullSelectionContract` 从已解码完整值开始，使用现有 `InboxLookup.inboxSpec`：未找到或全文冲突返回 Conflict；找到原位置后依次检查 terminal、refusal、当前版本不等于 accepted version、已有 local，任一成立拒绝。随后只允许 Create，正常返回原两个位置和其原 startup deadline。合同不添加唯一性、非零或相邻代等条件。

`CreateSelection.selection_complete` 对任意完整输入证明实际生成函数满足该合同，复用原 `InboxLookup.find_outcome` 和 `InboxPosition.inbox_found_reads`，从原查找结论推出真实索引边界。Model/Values 在正确和错误视图中逐字节相同；错误视图不导入正确业务对应定理。正常与 terminal/refusal/旧版本/local 见证均使用同一规格；正常 intent 的完整封套形状另有证明。封套合法不推出整个 Inbox 可达、认证成立、已持久接受或实际资源存在。key window 与 retention window 没有额外相等要求。

五个错误版分别在真实 Rust 上删除 terminal、删除 refusal、把版本不等比较改为小于、删除 local、把返回 startup deadline 改为 timeout。每版须实际编译、抽取，再以同一合同证明否定。编译、导入失败或超时不算性质反例。

## 依赖与未完成项

复用既有逐符号 String.eq、Bool/Option 比较、Vec.is_empty/索引、受检整数和普通错误值载体；没有新增库合同或填充外部 axiom 模板。完整 ContractError 首先在无该枚举 opaque 的真实捕获中核对，再绑定既有载体；JournalError 保留全部原变体。完整本地输入和库/工具/服务内容由原 runtime、源码快照及实际文件闭包记录。

本目录仅证明未采用候选的只读前置选择。原父函数一般等价、owner 排除新 sandbox 的重算、后续完整 local/entry/owner 修改、deadline 转换、原 ID 格式、SQL 两参数和行效果、save、commit/未知提交/重启，以及 JSON/TLS/认证来源和持久收件组合继续开放。V7 提供独立认证上下文来源，V1/V2 提供当前意图和耐久发送，V4/V5 提供本地接受/结果，V6 提供实际文件与 VM 事实；字段相等不能代替这些业务事实。不把整个 codec、HTTP 或事务设为可信。保留既有任意已解码 Manifest 值证明与 `ProtocolRecord.full_flow_contract` 的原适用范围，不重做或迁记旧证据。

## 独立开发检查

在工作流规定的证明机、固定工具和已存在共享重锁下运行（本地四核/四线程、每视图独立 target 和 `.lake/build`）：

```sh
python3 verification/experiments/stage1-protocol/create-selection/formal.py \
  --variant all --output artifacts/create-selection-development-01 \
  --lock "$SANDPROOF_PROOF_HEAVY_LOCK"
```

该入口新建目录，核对全部实际生成体，并调用原正式物理文件检查器检查所有安装文件及其真实导入闭包，要求完整、非 stale/partial 诊断及依赖审计。新切片未登记到共享入口，故其局部通过不表示 `scripts/verify-stage1-protocol.sh` 或整套证明验收已覆盖它。公共登记/根导入/冻结/覆盖候选由主控统一处理，当前封口不追加本目录。

普通与原生检查交给 ning。`native.py --output <新目录>` 准备候选、五错误和恢复候选共七个独立视图，共用九条原断言；记录新编译协议库和服务测试二进制、精确失败集合、格式及 Clippy。源代码原实现没有新 helper，故不伪造“原实现 helper”视图。它尚未执行；此入口的局部通过仍不能代替主控同批基础检查/共享原生回执。原父函数完整回归由主控普通入口承担。

原始日志保留在忽略的 artifacts 并绑定实际提交；GitHub 源可读与主控能读取原始证据分别报告。本目录不自动新增 PR、不合并或修改保护。
