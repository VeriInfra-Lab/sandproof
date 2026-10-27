# 收件配置接受条件候选

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包堆叠 #726，生产输入固定 a9c，从已解码 LifecycleInboxConfig 开始。
原 valid 的函数项在固定 Aeneas 区域分析中受阻；[生产候选](production-candidate.patch)
只将 is_none_or 的函数项改为显式 Option match，并添加 must_use。生产源码保持
只读；原函数的一般等价和生产采用仍待完成，原版/候选的有限原生检查不能代替它。
真实 HostEpochRegistration::valid、required_bytes 和范围检查仍透明抽取。

[共同完整合同](ConfigValidationModel.lean.in)要求返回当且仅当以下全部条件成立：

- center.deployment 与 node.deployment_id 相等。
- host_epoch 为 None，或版本等于 1、原 UTF-8 boot_id 为 1–128 字节且每个字节
  在 33–126；没有额外检查 policy 摘要。
- sessions 在 1–4096，operations_per_session 在 8–128，options/profile 在
  1–65536，manifest 在 1–262144。
- [原完整容量公式](../inbox-config/README.md)不超过 16777216。

[实现证明](ConfigValidationProofs.lean.in)覆盖任意满足表示条件的配置，连接实际
生成体的短路及正常 Bool 返回；accepts_iff/rejects_iff 给出精确接受/拒绝关系。
这里没有可区分的错误码，不能另称首错码定理。表示条件只要求存在的 host.boot_id
为有效、有限、字节长度可由 Usize 表示的原 String；长度是字节数，不是字符数。
预算证明复用同一生成类型视图内的原 required_bytes 任意值定理。

[共同值](ConfigValidationValues.lean.in)及[正常返回](ConfigValidationWitnesses.lean.in)
给出 None/Some host、小配置、原测试配置和恰好 16 MiB 的合法见证；任意 policy
不影响 Host.valid 的引理明确反映实际未检查字段。原生检查还覆盖空/空格/tab/DEL/
非 ASCII/内嵌空格、128/129 字节、全部容量边界和预算超出 128 字节等情形。
None 或相等的字段不提供认证、原生 Create 授权、实际宿主/VM、物理容量或持久接受。
其他 center/node 字段不由这个方法重新验证，ContentDigest 的值也不证明计算过哈希。

三个错误版均修改实际 Rust 并使用同一 FullConfigContract、表示条件和共同见证：
host-version-range 将版本等于 1 放宽到不超过 2；omit-profile-bound 删除 profile
范围；double-budget-limit 把 16 MiB 放宽到 32 MiB。它们分别实际接受版本 2、
profile=0、预算 16777344 的配置。错误版须编译、抽取并到达这些实际返回；Lean
反例不导入正确配置/host 实现证明，仍可复用没有改动的原容量证明。编译失败、
超时或导入失败不算性质拒绝，有限样例也不替代正确版任意值定理。

[绑定核对](library_binding.py)复用[依赖登记](../../../../docs/formal/dependencies.md)
中的 Bool、Option 字典/as_ref、String 相等/deref、Bytes.all、Str.bytes、Vec.is_empty
和固定整数模型。两个新增调用名 String.len/is_empty 是尚未由 V0 采用的候选，
分别委托已登记的 String.deref 与 Str.len/is_empty；公共采用前结论保持此条件。
不可表示的数学字符串触发模型失败，不是生产 String 的可恢复错误语义。
实际 is_ascii_graphic、RangeInclusive/RangeBounds.contains 和 start/end_bound
使用透明标准库生成体；范围引理只用于新构造、未耗尽的 U16/U32 区间。
两版标准库源码、候选绑定、固定后端和 Result/Bound 逻辑标记逐项核对；没有把
整个标准库、host 校验、Serde、HTTP handler 或收件事务设为可信。

## 检查与交接

持有共享重锁后串行运行，独立 .lake/build，保留缓存与失败证据：

```sh
python3 verification/experiments/stage1-protocol/inbox-config-validation/native.py --output artifacts/config-validation/native
python3 verification/experiments/stage1-protocol/inbox-config-validation/formal.py --variant correct --development-native artifacts/config-validation/native/result.json --output artifacts/config-validation/correct
python3 verification/experiments/stage1-protocol/inbox-config-validation/formal.py --variant host-version-range --development-native artifacts/config-validation/native/result.json --output artifacts/config-validation/wrong-host
python3 verification/experiments/stage1-protocol/inbox-config-validation/formal.py --variant omit-profile-bound --development-native artifacts/config-validation/native/result.json --output artifacts/config-validation/wrong-profile
python3 verification/experiments/stage1-protocol/inbox-config-validation/formal.py --variant double-budget-limit --development-native artifacts/config-validation/native/result.json --output artifacts/config-validation/wrong-budget
```

正式 --require-native 要求 ning 的普通完整及本包回执，不能与 development-native
混用。prepare-only 不计为证明通过。[清单](inventory.json)登记固定生产输入和
全部审计，驱动核对真实 Rust 与原生版本逐字相同、准确输入闭包、实际生成体、
全文件服务诊断及依赖审计；stale/partial、输入变化、新增业务模型均拒绝。
[公共候选](coordinator-candidate.patch)接 #713–#726 的 14 份父增量，含本包原生
登记、四个形式视图、覆盖行和两个待采用 String 绑定；主控协调公共文件后才能
称公共 verify-stage1-protocol.sh 覆盖本片。verify-controller.sh/proof_native_suite
由主控同批统一验收并复用内容，本包开发通过不代表 ning/共享整套通过。

仍待连接原函数一般等价、生产/库采用、lane/SessionShape/native_retirement、
owner/member 与完整 Inbox.validate、转换保持性、持久 codec/同事务/未知提交/
重启及真实 JSON/TLS/handler。V7 提供独立认证来源，V1/V2 提供当前意图和耐久发送，
V4/V5 提供实际本地接受/结果，V6 提供文件/VM 事实；完整 V3 不能由本片认领完成。
