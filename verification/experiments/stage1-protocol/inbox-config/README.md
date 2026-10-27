# 原收件配置容量公式

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包堆叠 #725，从固定 a9c 的已解码 LifecycleInboxConfig 开始，直接抽取原
required_bytes，没有生产或算法重构。[共同规格](ConfigBudgetModel.lean.in)使用
同次生成的完整配置及五个容量字段，精确容量为：

```text
max_sessions × (4 × max_options_bytes + max_operations_per_session × 8192
                + 2 × max_manifest_bytes + max_profile_bytes + 32768) + 8192
```

[实现证明](ConfigBudgetProofs.lean.in)覆盖全部 u16/u32 字段值，不预先要求配置
通过 valid。每一步原乘加均正常完成，返回值当且仅当其自然数值等于完整公式；
统一最大值 2005480217559047 小于 u64 上限。零或字段最大值只是本算术函数的
有类型输入，不因此成为合法部署配置。该数字是逻辑容量估算，不能推出实际
分配器 RAM、磁盘预留、持久接受或物理资源存在。

[共同见证](ConfigBudgetValues.lean.in)复用原 CenterIdentity/NodeIdentity 值，
给出两个正常配置的完整数值、deployment 一致、host_epoch=None、范围与预算
形状；[正常返回](ConfigBudgetWitnesses.lean.in)由同一一般合同推出。原生检查
还要求这两个见证通过原 valid；这不替代任意配置的 valid 对应证明。原方法
允许 None 的配置形状，不由此提供原生 Create 授权或独立 host 事实。

三个真实 Rust 错误版分别少算 options 的两份预留、少算一份 manifest、漏掉
全局预留。错误版须实际编译和抽取，Lean 反例展开实际生成体返回，并反驳同一
FullBudgetContract；不导入正确实现的证明。原生检查另覆盖每字段增量、每槽
固定开销、零槽全局预留、整个类型域最大输入，以及非容量字段不影响本计算。
编译失败、超时或导入失败不算性质拒绝。

[外部绑定](FunsExternal.lean.in)逐字复用既有四个具体 Bool/Option 字典/String/
Vec 模型，原 u16→u64、u32→u64 转换和 u64 乘加使用固定 Aeneas 后端模型及引理。
[库核对](library_binding.py)固定两版标准库对应源、后端定义和 Result 逻辑标记；
逐符号条件按[依赖登记](../../../../docs/formal/dependencies.md)的整数与普通值
合同解释，不称标准库实现已证明，没有业务 opaque 或新自定义公理。

原完整 config.valid 的 Charon 捕获成功，但固定 Aeneas 在 HostEpochRegistration
的区域分析处报告 Unimplemented，原失败/IR 留存。完整 valid 的短路、host
形状、范围与 16 MiB 条件仍独立待证，不把整个 host 校验当作外部模型。持久
收件、事务/未知提交/重启、完整 Inbox.validate 及 JSON/TLS/handler 继续开放。
V1/V2 提供当前意图与耐久发送，V4/V5 提供本地接受/结果，V6 提供文件/VM 事实，
V7 提供独立认证来源；配置字段和摘要值不代替这些业务条件。

## 检查

持有共享重锁后串行运行，使用独立 .lake/build 并保留缓存和失败证据：

```sh
python3 verification/experiments/stage1-protocol/inbox-config/native.py --output artifacts/inbox-config/native
python3 verification/experiments/stage1-protocol/inbox-config/formal.py --variant correct --output artifacts/inbox-config/correct
python3 verification/experiments/stage1-protocol/inbox-config/formal.py --variant halve-options-reservation --output artifacts/inbox-config/wrong-options
python3 verification/experiments/stage1-protocol/inbox-config/formal.py --variant omit-manifest-copy --output artifacts/inbox-config/wrong-manifest
python3 verification/experiments/stage1-protocol/inbox-config/formal.py --variant omit-global-reserve --output artifacts/inbox-config/wrong-global
```

正式 --require-native 要求 ning 的普通完整及本包回执；--development-native
仅连接本地开发证据，prepare-only 不计为证明通过。[清单](inventory.json)登记
固定生产输入与全部审计；驱动重抽取真实函数，检查准入、真实文件的完整服务
诊断及依赖审计，stale/partial、输入变化和新增业务模型均拒绝。
[公共登记与覆盖候选](coordinator-candidate.patch)接 #713–#725 的 13 份父增量后
交主控，保留旧入口；verify-controller.sh/proof_native_suite 由主控同批验收并
复用内容。局部通过不等于 ning、共享整套或完整 V3 已完成。
