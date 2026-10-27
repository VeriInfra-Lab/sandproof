# 原终态观察的完整请求匹配

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包堆叠 #727，生产输入固定 a9c，从已解码 TerminalLifecycleObservation 与
AcceptedLifecycleIntent 开始，直接抽取原 matches_request。只在隔离抽取 crate
追加[透明调用根](roots.rs.in)，没有生产或算法重构。历史 carriers 的计数器与
摘要值往返证明保留；该原方法虽然已在历史冻结源中，历史 carriers 并未抽取或
证明它，文件被收入输入清单不等于其中所有方法已证。

[完整合同](TerminalModel.lean.in)复用既有 request/accepted 的表示与规格，要求
实际正常返回恰为以下条件的合取：

- observation.request 与传入的完整 accepted intent 相等，涵盖全部请求字段、
  操作参数、accepted_version、center 和 retention。
- 传入操作是 Destroy，accepted_version 大于零。
- 原 retention.valid_for(4096) 的 slot 形状成立。
- accepted_version_for_request 返回 Some(accepted_version)，保留 Absent→1、
  Version 的 checked_add 和溢出拒绝。

[实现证明](TerminalProofs.lean.in)对任意完整值连接原生成体，给出接受/拒绝当且仅当、
完整 intent 保留以及任意状态标记不影响结果。相等证明复用现有 Replay.accepted_eq，
retention/version 使用原 Validation.retentionOK/versionOK；不为正反版本改换规格。
该 Bool 方法不提供可区分的首错码。retention window 不要求等于 key window。

[共同值](TerminalValues.lean.in)与[正常返回](TerminalWitnesses.lean.in)给出正常
Pending/Cleaned、独立 retention window、最大有效版本与最大 slot 的合法封套见证，
以及 checked_add 溢出拒绝。另证明 Destroy+Absent 能通过这个匹配守卫，同时被原
accepted.validate 拒为 InvalidExpectation，说明匹配方法本身没有调用上下文或
请求合法性检查。构造 context、相等字段或 Cleaned 标签都不提供独立认证、实际
清理或持久接受；Pending 也可完整匹配。

四个错误版均修改实际 Rust：key-only-observation 弱化为完整 key 相等；allow-unload
额外接受 Unload；omit-retention-shape 删除 slot 形状；omit-version-relation 删除
版本对应条件。它们分别实际接受不同中心的同 key 请求、Unload、slot=0、错误
accepted version，并反驳同一 FullTerminalContract。原生检查还覆盖全部完整绑定
字段、所有非 Destroy 类别、slot 上界、零版本、溢出和守卫未观察的认证/协议条件。
错误视图不导入正确终态实现证明；编译失败、超时、导入失败或警告不算性质拒绝。

外部函数逐字复用[既有四项绑定](../inbox-config/FunsExternal.lean.in)：Bool.ne、
实际 Option 比较字典、String.eq 和 Vec.is_empty；数组/向量相等及 checked_add
沿用固定 Aeneas 后端与[逐符号依赖](../../../../docs/formal/dependencies.md)。
同次生成视图复用现有完整 request/accepted 证明，核对两版标准库、固定后端、
实际 Result 逻辑标记及绑定内容，没有新增外部符号或业务 opaque。ContentDigest
的 32 字节值相等不表示计算过哈希，也不证明材料真实完整。

## 检查与交接

持有共享重锁后串行运行，使用独立 .lake/build，保留缓存与失败证据：

```sh
python3 verification/experiments/stage1-protocol/terminal-binding/native.py --output artifacts/terminal-binding/native
python3 verification/experiments/stage1-protocol/terminal-binding/formal.py --variant correct --development-native artifacts/terminal-binding/native/result.json --output artifacts/terminal-binding/correct
python3 verification/experiments/stage1-protocol/terminal-binding/formal.py --variant key-only-observation --development-native artifacts/terminal-binding/native/result.json --output artifacts/terminal-binding/wrong-key
python3 verification/experiments/stage1-protocol/terminal-binding/formal.py --variant allow-unload --development-native artifacts/terminal-binding/native/result.json --output artifacts/terminal-binding/wrong-operation
python3 verification/experiments/stage1-protocol/terminal-binding/formal.py --variant omit-retention-shape --development-native artifacts/terminal-binding/native/result.json --output artifacts/terminal-binding/wrong-retention
python3 verification/experiments/stage1-protocol/terminal-binding/formal.py --variant omit-version-relation --development-native artifacts/terminal-binding/native/result.json --output artifacts/terminal-binding/wrong-version
```

正式 --require-native 要求 ning 的普通完整及本包回执；development-native 只连接
开发证据，prepare-only 不计证明通过。[清单](inventory.json)与驱动核对固定生产源、
原生/形式的实际 Rust 与透明根、完整输入闭包、原生成体、全部真实文件的同服务
完整诊断和准确审计；输入变化、stale/partial 或新增业务模型均拒绝。
[公共登记候选](coordinator-candidate.patch)接 #713–#727 的 15 份父增量，保留旧入口，
登记本包原生、五个形式视图及覆盖行。公共 verify-stage1-protocol.sh 需主控采用后
才覆盖本片；verify-controller.sh/proof_native_suite 由主控同批统一验收与复用内容。
本包开发通过不表示 ning、共享整套或完整 V3 通过。

后续仍须连接 lane 的 successor/上下文/slot/window/真实编码长度、SessionShape、
native_retirement、owner/member 与完整 Inbox.validate，以及转换保持性、持久
codec/同事务/未知提交/重启和 JSON/TLS/handler。V7 提供独立认证来源，V1/V2 提供
当前意图和耐久发送，V4/V5 提供本地接受与结果，V6 提供实际文件/VM 事实；终态
观察与请求匹配不能代替这些提供者。
