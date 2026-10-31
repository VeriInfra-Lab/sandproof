# 续期会话校验候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**最终统一候选的抽取与整套原生、Lean 验收尚待完成**；局部结果只按原回执的精确提交保留，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。

标准编码实例仍调用真实 `serde_json::to_vec`。通用证明只接收同一完整值对应的编码结果，并保留失败、长度及调用顺序；用于标准实例时仍须提供这一次编码调用到结果模型的窄连接。它不假设整个校验函数正确，也不覆盖持久事务或任意 JSON 解析。
[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

旧证明起于 a9c 的 renewal_confirmation::validate_session，从已解码 Session 和
RenewalConfirmation 值开始。[原重构](production-candidate.patch)已被当前生产采用：
首次全文查找及两个 any 调用展开为具名循环，编码调用经任意 RenewalEncoding 参数传入。
标准实现仍调用真实 serde_json::to_vec。本轮直接检查已采用实现的合同，不另声称旧实现的一般等价。
标准编码实例的窄连接仍需落实；有限对照不能消除这项义务。

固定 Charon/Aeneas 对原函数在 Slice.find 的高阶生命周期处失败；仅改首次查找后，
失败转移到 Serde 参数翻译。进一步展开标准 Slice.any 又遇到不透明 Iter 类型限制。
本候选没有修改工具或生成体，也没有增加整个 Serde/handler/事务的可信模型。
这些失败属于工具预检，不能作为业务性质反例。

[共同规格](InboxRenewalModel.lean.in)和 [组合证明](InboxRenewalComposition.lean.in)
覆盖完整候选的接受条件、检查顺序及任意编码计算。无确认时仅要求没有压缩标记；
有确认时依次检查完整确认形状、编码及 8192 字节上限、local 存在、首个全文匹配项
的 applied/previous 字段、压缩边界，以及未应用的 SDK 续期。无确认或形状非法时
不调用编码；编码错误保留为原 Json 错误，先于剩余 CorruptStore 检查返回。
任意编码参数不假设成功、长度合规或字段正确，也保留外层失败/发散效果。

[首次匹配](InboxRenewalCheckpoint.lean.in)与[版本扫描](InboxRenewalContains.lean.in)、
[未应用扫描](InboxRenewalUnapplied.lean.in)针对任意有限列表。首个全文匹配项不合格时，
后续好重复项不能补救。末尾扫描沿用真实较弱分类器：window=2 且操作为 Renew；
不额外要求条目本身满足确认形状。压缩边界允许编号空洞，检查值至少为 2、严格小于
确认版本且未保留该编号的条目。previous 仅要求 Some；local 仅要求 Some。
这里没有增加历史可达性、实际删除、寿命边界、当前版本或资源存在条件。

八份 Renewal 模板逐字节复用 #696 的 3e2a7dc2efc00c8efbe226486ee704605b73bf82；
[清单](inventory.json)绑定原始摘要，形式检查在本包新生成视图中重新检查全部真实文件。
它们保持原 Shape/Matches/ConfirmedMatches，包括完整封套和请求值、不额外要求
retention.window=key.window。借用 Vec 循环基础引理和较弱分类器来自 #713；
不将复用证据计为新业务性质，也不改写已有公共根或冻结来迁记结果。

[逐符号检查](library_binding.py)只保留既有七个函数绑定；没有新 Serde 函数模型。
full_contract_with_binding 显式接收 BorrowedVecBinding，V0 调用采用仍待完成。
full_contract 是隔离模型的推论，不表示 Rust 标准库已证明。#696 的 Result 逻辑标签
与本次完整 LLBC 的 Ok/Err 字段和判别值核对；其 V0 采用仍待完成，不涉及物理布局。
字符串、Option、比较、算术和错误值载体继续沿用[逐符号依赖](../../../../docs/formal/dependencies.md)。

[原生断言](native_tests.rs)在原版、候选、三种错误版和恢复版上相同。
key-only-checkpoint、ignore-retained-frontier、ignore-unapplied-sdk 必须实际编译、抽取，
并分别到达[全文遗漏](InboxRenewalWrongKey.lean.in)、[保留边界遗漏](InboxRenewalWrongFrontier.lean.in)、
[未应用遗漏](InboxRenewalWrongUnapplied.lean.in)反例。每份反例拒绝同一
FullRenewalSessionContract；错误视图不导入完整正确组合或被修改 helper 的正确证明。
[回调测试](callback_tests.rs)仅属于候选接口，不能冒充原编码连接已证明。

完整 Inbox.validate、同事务 read_lifetime、确认/压缩转移保持性、持久 codec、未知提交
及重启由 V3/V4/V5 继续连接；V1/V2 的当前意图和耐久发送、V6 的实际文件/VM事实、
V7 的独立认证来源仍归相应提供者。完整 manifest、实际 JSON 字节到值、网络外壳和
完整系统结论均未由本切片完成；ContentDigest 的值表示不证明计算过哈希或材料真实。

## 检查

持有工作流的共享重锁后执行；每份副本使用独立 .lake/build，保留依赖缓存和失败记录：

```sh
python3 verification/experiments/stage1-protocol/inbox-renewal-session/native.py --output artifacts/inbox-renewal-session/native
python3 verification/experiments/stage1-protocol/inbox-renewal-session/formal.py --variant correct --output artifacts/inbox-renewal-session/correct
python3 verification/experiments/stage1-protocol/inbox-renewal-session/formal.py --variant key-only-checkpoint --output artifacts/inbox-renewal-session/wrong-key
python3 verification/experiments/stage1-protocol/inbox-renewal-session/formal.py --variant ignore-retained-frontier --output artifacts/inbox-renewal-session/wrong-frontier
python3 verification/experiments/stage1-protocol/inbox-renewal-session/formal.py --variant ignore-unapplied-sdk --output artifacts/inbox-renewal-session/wrong-unapplied
```

正式模式使用 --require-native 和 ning 的有效普通/native 回执；开发回执只能用
--development-native 指定，二者不能互相替代。prepare-only 只准备抽取视图。
形式驱动执行固定工具、完整实际文件服务诊断、依赖检查和声明审计；原始输入、LLBC、
原样生成体及失败日志保存在 artifacts。公共入口和覆盖只提供
[主控候选](coordinator-candidate.patch)，接在 #713–#720 候选之后；主控同批执行
verify-controller.sh 与 proof_native_suite 并内容复用，旧根通过不代表本包四个视图通过。
