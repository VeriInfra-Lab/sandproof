# 当前执行来源选择候选

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包从固定 a9c 的已解码 Session 开始，连接真实 current_execution_origin 的
[隔离反向扫描候选](production-candidate.patch)，堆叠父项为 #724。原方法的
rev/find 在固定 Aeneas 中遇到高阶/自由生命周期限制，历史原始捕获及失败记录
保留；当前生产源及工具仍相同。候选仅显式展开原反向索引扫描，生产没有修改。
原实现一般等价与生产采用仍是独立义务，有限原生回归不能代替它们。

[共同规格](InboxOriginModel.lean.in)使用同次生成的完整 Entry 和 Vec.val 序列。
FullOriginContract 要求返回最后一个 Create/Resume 的完整条目；LastOrigin 用
前缀、该条目和后缀描述其位置，并要求后缀没有匹配项。返回 None 当且仅当整个
列表没有匹配项。[实现证明](InboxOriginProofs.lean.in)对任意有限列表和安全
前缀成立，覆盖递减游标、减法/索引安全、提前返回及完整调用结果。

选择只依赖操作类别及列表顺序，不检查 local_operation、当前代、版本或 terminal。
因此未附加的 Resume 也可以被选中，None 也不表示没有运行中的 VM。完整返回值
保留 intent、local_operation、期限、前期有效终点和 unload_resolution；不能
用 key、session 或仅操作类别相同代替完整 Entry。模型结论是值对应，原生检查
另核对返回引用指向原存储位置，不把有限指针检查称为一般内存模型证明。

[共同见证](InboxOriginValues.lean.in)复用已有 ValidEnvelope 和 Create 形状，
包含合法 Create、Resume 1→9、续期/销毁尾项、空列表，以及请求相同而本地元数据
不同的两个条目。它们不声明整个 Session/Inbox 历史合法，也不加强公共 Resume
为必须加一。[正常返回](InboxOriginWitnesses.lean.in)从同一一般合同推出。
三个真实 Rust 错误版分别漏掉 Resume、额外要求 local_operation 非空、返回第一
存储槽；均须实际编译、抽取，并由生成体的实际返回反驳同一 FullOriginContract。
错误视图只导入共同规格、值和不依赖变异行为的循环支持。

[四个外部模型](FunsExternal.lean.in)逐字复用已登记的 Bool、Option 字典、字符串
相等和 Vec.is_empty 绑定；[库核对](library_binding.py)检查固定原 iterator/index
源和后端内容。减法与索引沿用固定 Aeneas 的既有模型及引理，没有新增借用 Vec
迭代模型。逐符号条件按[依赖登记](../../../../docs/formal/dependencies.md)解释，
没有整包 Serde、handler、事务假设或业务 opaque。ContentDigest 仍只是 32 字节值，
不证明计算过哈希或材料真实完整。

完整 Inbox.validate、成员关系、转换保持性、持久 codec/同事务/未知提交/重启
及 JSON/TLS/handler 继续是后续义务。V1/V2 提供当前意图和耐久发送，V4/V5 提供
实际本地接受与结果，V6 提供文件/VM 事实，V7 提供独立认证来源；本方法不提供
这些业务条件。历史 Manifest 值证明保留，不能由此认领真实材料链或完整 V3。

## 检查

持有共享重锁后串行执行，各视图使用独立 .lake/build，保留缓存与失败证据：

```sh
python3 verification/experiments/stage1-protocol/inbox-origin-selection/native.py --output artifacts/inbox-origin-selection/native
python3 verification/experiments/stage1-protocol/inbox-origin-selection/formal.py --variant correct --output artifacts/inbox-origin-selection/correct
python3 verification/experiments/stage1-protocol/inbox-origin-selection/formal.py --variant skip-resume --output artifacts/inbox-origin-selection/wrong-resume
python3 verification/experiments/stage1-protocol/inbox-origin-selection/formal.py --variant require-local-operation --output artifacts/inbox-origin-selection/wrong-local
python3 verification/experiments/stage1-protocol/inbox-origin-selection/formal.py --variant return-first-slot --output artifacts/inbox-origin-selection/wrong-slot
```

正式 --require-native 要求 ning 的完整普通及本包回执；--development-native 只
连接本地开发记录，prepare-only 只准备视图。[清单](inventory.json)登记固定
生产输入和全部审计，形式驱动重新抽取并执行准入回归、真实文件的完整服务诊断
及依赖审计，stale/partial 或输入变化均拒绝。[公共登记与覆盖候选](coordinator-candidate.patch)
接在 #713–#724 的 12 份父增量后交主控，保留旧入口；verify-controller.sh 与
proof_native_suite 由主控同批验收和内容复用。本地通过不代替 ning 或整套验收。
