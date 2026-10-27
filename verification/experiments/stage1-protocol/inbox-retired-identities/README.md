# 退役身份校验候选

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包从固定 a9c 的已解码 Inbox 值开始，连接真实
Inbox::validate_retired_identities 的[隔离扫描候选](production-candidate.patch)。
堆叠父项为 #723，生产文件只读。固定工具成功捕获原入口，Aeneas 先在 Iterator
的 any 字段处失败；精确透明抽取原 any 后又在泛型签名翻译处失败。候选仅将原有
三段查询移入具名循环，保留此前退役 lane、当前 session、session 内条目的顺序
及完整身份比较。原实现一般等价和生产采用仍待完成，原生回归不替代这两个义务。

[共同规格](InboxRetiredModel.lean.in)对所有视图不变：每个 last_retired 都不得
与此前任一退役记录共享完整 SessionId 或完整 OperationKey，也不得匹配任一活跃
Session.id 或任一活跃条目的完整 key。没有 last_retired 的 lane 不参与检查。
[扫描证明](InboxRetiredScans.lean.in)覆盖任意长度、任意游标的全部扫描；
[外层证明](InboxRetiredProofs.lean.in)证明前缀索引安全、短路步骤及任意 Inbox 的
完整返回，给出接受与身份互斥当且仅当、拒绝与其否定当且仅当。它不把相邻记录
比较、sequence-only 或有限例子代替完整相等与任意列表定理。

[共同见证](InboxRetiredValues.lean.in)保留合法完整封套，并覆盖正常退役记录、
sequence 相同但 owner 不同的完整 key、活跃条目冲突及先前退役 session 冲突。
[正常返回](InboxRetiredWitnesses.lean.in)由一般定理导出。三个真实 Rust 错误版
分别只比较条目 sequence、忽略活跃冲突结果、忽略先前 session 冲突；
[原生断言](native_tests.rs)对原版、候选、错误版和恢复版相同。
每个错误视图必须实际编译、抽取，并由生成体的实际返回反驳同一完整合同；
编译失败、超时和导入失败均不计作性质拒绝。错误视图只导入该变异未破坏的引理。

本方法单独只检查身份互斥，不重新验证退役 observation 的封套、slot/window
形状、Cleaned 状态或资源事实。原生负边界明确保留这一范围。字段相等、不同或
存在都不推出独立授权、已持久接受、真实清理或资源存在；不加强成 key.window
必须等于 retention.window。完整 Inbox.validate、成员关系、状态转换保持性、
持久 codec、同事务、未知提交、重启及真实 JSON/TLS 链继续是独立义务。
历史 Manifest 值证明保留，32 字节摘要值不证明实际计算哈希或材料完整。
V1/V2 提供当前意图和耐久发送，V4/V5 提供实际本地接受及结果，V6 提供文件/VM
事实，V7 提供独立认证上下文来源。

[库绑定](library_binding.py)逐字复用五个既有外部函数模型，核对固定 std 源及
Result 逻辑标签。完整 key 等价复用既有逐字段证明；借用 Vec 的接线条件在定理
中具名，在当前模型中给出实例，V0 采用仍由主控负责。所有依赖按
[逐符号合同](../../../../docs/formal/dependencies.md)解释，没有整包 Serde、
handler、收件事务假设或业务 opaque。

## 检查

持有共享重锁后串行执行；每个视图使用独立 .lake/build，保留缓存与失败证据：

```sh
python3 verification/experiments/stage1-protocol/inbox-retired-identities/native.py --output artifacts/inbox-retired-identities/native
python3 verification/experiments/stage1-protocol/inbox-retired-identities/formal.py --variant correct --output artifacts/inbox-retired-identities/correct
python3 verification/experiments/stage1-protocol/inbox-retired-identities/formal.py --variant key-sequence-only --output artifacts/inbox-retired-identities/wrong-key
python3 verification/experiments/stage1-protocol/inbox-retired-identities/formal.py --variant ignore-active-conflicts --output artifacts/inbox-retired-identities/wrong-active
python3 verification/experiments/stage1-protocol/inbox-retired-identities/formal.py --variant ignore-prior-session --output artifacts/inbox-retired-identities/wrong-prior
```

正式模式 --require-native 要求 ning 的完整普通及本包回执；--development-native
只连接本地开发记录，prepare-only 只准备视图。[清单](inventory.json)记录生产
输入与审计声明。形式驱动必须重新抽取，核对准入回归、实际文件的完整同服务诊断
及证明依赖；输入变化或 stale/partial 结果均拒绝。[公共登记与覆盖增量](coordinator-candidate.patch)接在 #713–#723 的 11 份父候选之后交主控，
verify-controller.sh 和共享 proof_native_suite 由主控同批验收与内容复用。
本包独立开发证据不表示 ning、统一整套验收或完整 V3 已完成。
