# 收件条目类别校验候选

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包从固定 a9c 的已解码 Inbox、Session、Entry 和认证上下文值开始，连接
Inbox::entry_kind 的[隔离重构候选](production-candidate.patch)，堆叠父项为 #722。
生产文件保持原样。固定工具在原 Serde 参数、flat_map 借用上下文及 Ord.max 字典处
分别遇到限制；候选复用已有 UnloadEncoding，将全表计数移入具名循环，并将有符号
最大值写成显式比较。标准包装仍调用真实编码器。生产采用、原实现的一般等价及标准
Serde 实例连接尚未完成；原生对照提供回归证据，不替代这些义务。

[完整规格](InboxEntryModel.lean.in)复用既有封套和卸载决议规格，不为错误版改换性质。
[对应证明](InboxEntryProofs.lean.in)覆盖任意输入值和任意编码计算：先校验卸载决议，
其错误、失败和发散先于后续守卫；随后检查完整封套、retention、session、owner、
本地操作及续期字段形状，再计数全部会话中的完整 OperationKey，最后检查操作参数。
计数要求恰好一次，不用 key 的部分字段或提前截断计数代替。所有 CorruptStore 分支
保留 Rust 的短路顺序；计数溢出早于后续操作检查。

[计数证明](InboxEntryCount.lean.in)对任意长度列表、任意游标及初始 usize 成立，
既覆盖正常计数也覆盖 integerOverflow。它显式区分 Entry/Session 的借用 Vec 接线
条件，并在当前固定模型中证明实例；V0 采用仍待协调。Create 保留 options 长度上界；
Unload 要求正来源代及 local 存在；Destroy 返回类别 2；Renew 保留非负期限、两个
可选期限的成对形状及 extend_only 的 max(previous, requested)；Resume 复用允许
编号空洞的公共请求校验，没有加强成必须加一。

[共同见证](InboxEntryValues.lean.in)包含五类正常请求、Resume 1→9、跨会话重复 key、
owner 不同而 sequence 相同的完整 key，以及续期保留历史较大期限。正确实现的
[实际返回](InboxEntryWitnesses.lean.in)由一般定理导出。三个真实 Rust 错误版
分别将计数封顶为一、只比较 sequence、忽略历史期限；每个都须编译、抽取并到达
同一 FullEntryContract 的反例，错误视图不导入受变异破坏的正确证明。
[原生断言](native_tests.rs)另覆盖所有 key 字段、边界及首错，
[回调检查](callback_tests.rs)核对编码错误优先级与 4096/4097 字节边界。

该方法不保证传入的 Session/Entry 就是 Inbox 中被计数的那一行；实际调用者的
成员关系由外层 Inbox.validate 提供。完整相等或字段存在均不提供独立授权来源、
当前意图、已持久接受或真实资源事实。retention 与 key.window 保留原有独立关系。
完整 Inbox.validate、状态转换保持性、持久 codec/提交/未知结果/重启、实际 manifest 材料链、
JSON/TLS 链和系统结论仍待后续切片。V1/V2 提供当前意图和耐久发送，V4/V5
提供本地接受及结果，V6 提供文件/VM 事实，V7 提供独立认证上下文来源。

[库绑定](library_binding.py)逐字节复用八个既有函数模型，核对原 std count/max
源码、Result/Phase 完整逻辑标签及固定工具。Bool、Option、String、Vec、算术及
错误载体依赖[逐符号合同](../../../../docs/formal/dependencies.md)；不引入整个
Serde、handler 或收件事务假设。32 字节摘要值不表示计算过哈希或材料真实完整。

## 检查

持有共享重锁后串行执行；使用独立 .lake/build，保留缓存与失败证据：

```sh
python3 verification/experiments/stage1-protocol/inbox-entry-validation/native.py --output artifacts/inbox-entry-validation/native
python3 verification/experiments/stage1-protocol/inbox-entry-validation/formal.py --variant correct --output artifacts/inbox-entry-validation/correct
python3 verification/experiments/stage1-protocol/inbox-entry-validation/formal.py --variant duplicate-cap --output artifacts/inbox-entry-validation/wrong-duplicate
python3 verification/experiments/stage1-protocol/inbox-entry-validation/formal.py --variant key-sequence-only --output artifacts/inbox-entry-validation/wrong-key
python3 verification/experiments/stage1-protocol/inbox-entry-validation/formal.py --variant requested-deadline-only --output artifacts/inbox-entry-validation/wrong-maximum
```

形式驱动重新抽取全部视图，检查准入回归、所有实际文件的同服务完整诊断及证明依赖。
[清单](inventory.json)绑定固定生产输入与审计声明；完整输入、工具、库、生成物和
导入闭包保存在运行记录中。正式模式使用 --require-native，要求 ning 的完整普通及
本包回执；--development-native 仅连接开发证据，prepare-only 只准备视图。
[公共登记候选](coordinator-candidate.patch)接在 #713–#722 的候选之后，交主控接入
verify-stage1-protocol.sh、proof_native_suite 和覆盖记录；保留全部旧检查，不以
旧根通过代替本包。ning 和 verify-controller.sh 共享整套验收分别报告。
