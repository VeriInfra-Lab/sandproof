# 原生退休收据的值校验候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包绑定 a9c 的 `native_retirement::validate_saved`，生产只读。原函数的固定
Charon/Aeneas 抽取在 Serde 路径失败；窄编码候选进一步遇到 Destroy 模式判断的
借用合并限制，原 slice.last 透明抽取也遇到投影限制。`prepare.py` 只在隔离副本
参数化原两次编码及 digest 调用，提取透明 Destroy 判断，并复用既有末项索引写法。
`production-candidate.patch` 供工程负责人审查，未直接采用到生产。

原实现与候选的一般等价、生产采用、标准实例的精确 Serde 字节/错误及真实 SHA
计算连接仍开放。原生对照不能替代一般等价证明，抽取成功也不能替代实现证明。
形式结论以 `FullSavedContract` 为唯一正反合同，从已解码 Session 值开始；不声称
JSON/TLS/handler、持久接受或完整 V3 已完成。

## 合同和表示

`SavedModel` 直接使用真实生成类型，`SavedProofs.validate_saved_spec` 对任意类型的
编码参数和任意 Session 连接实际生成的 `validate_saved_using`：

- 没有收据直接返回成功；有收据而没有最后一项返回 CorruptStore。
- 第一次编码前检查 terminal、完整 local binding、Destroy、当前 accepted version
  和最后一项的本地操作编号。local binding 包括 sandbox、owner 和原创建操作编号。
- 编码最后一项的完整 accepted intent，传播 JSON 错误，再计算并比较全部 32 字节
  digest。此处不假设 hash 单射，也不从 digest 相等推出材料真实性。
- 随后检查收据与 local 的 sandbox/owner、quiescence 的 sandbox/owner/epoch/operation，
  以及四个 Confirmed 字段。最后编码完整收据，传播 JSON 错误并检查长度不超过 8192。

各比较仍使用现有逐符号合同。两次编码和 digest 的外层失败、发散及短路顺序保留；
没有假设编码总成功或固定输出长度。`SavedLibrary` 的末项查询、Destroy 分类和完整
比较引理连接实际生成函数；没有用有限见证代替任意值定理。

本函数不调用 accepted.validate，不检查完整 SessionShape，不比较 workspace，也不
为 retain_until 增加范围条件。后两字段仍进入实际收据编码，因此不能声称它们对所有
编码结果都无影响。正常见证使用合法 Create→Destroy 封套；缺收据时的无检查行为另给
见证。抽象编码器的 8192/8193 边界不表示真实标准收据 JSON 已达到这个长度。

## 实际错误版与检查

`prepare.py` 生成六个实际 Rust 错误版：只检查 local 存在、忽略 intent digest 比较、
漏查 worker 状态、漏查 quiescence epoch、提前编码、以及将长度界限放宽到 8193。
忽略比较的版本仍执行原比较/回调，只丢弃比较结果。所有版本复用同一完整合同和
同一组值；错误视图不导入 `SavedProofs` 或 `SavedWitnesses`。

`native.py` 在独立源码和 target 中运行原版、候选、六个错误版、恢复版，检查格式、
严格 Clippy、实际库和测试程序的 fresh 来源、精确断言失败及输入摘要。编译失败、
Clippy 失败或超时不算性质拒绝。

`formal.py` 用固定工具重抽取每个视图，检查透明根到实际方法的调用、选项、目标、
外部符号集合、真实生成文件和声明审计，随后经规定同服务客户端检查完整导入闭包。
原始 LLBC、生成体和失败证据保留。仅复用既有四个外部函数模型；两个 A6 定义及属性
逐字生成导入视图，三个错误值表示复用原登记。生成的 ContractError 保持透明，未为
整个 Serde、SHA、校验器或收件事务增加可信假设。

开发入口为 `native.py --output ...`，以及
`formal.py --variant correct --development-native .../result.json --output ...`；每个错误
视图使用其对应 variant。运行时持有共享重锁并保留独立 `.lake/build`，不能并发共写。
正式入口须使用 `--require-native` 验证 ning 的功能及本组件收据。公共登记、覆盖记录
和共享完整入口由主控协调；本包局部检查不代表旧入口已覆盖本包，也不替代 ning、
`verify-controller.sh`、`proof_native_suite` 或 E2E 验收。

## 待落实的连接

V3 负责原候选对应、真实编码/digest 调用连接、SessionShape、完整 Inbox.validate、
成员关系和转换保持，以及持久 codec/同事务/未知提交/重启。只按 dependencies.md
复用 V0 已登记的逐符号合同，不把待证业务条件改成外部公理。

V7 提供独立认证上下文来源；V1/V2 提供当前意图与耐久发送；V4/V5 提供原生 Retire、
同事务收据产生和接受/结果连接；V6 提供真实退出、连接排空、宿主回收及 pin 释放事实。
值相等和 Confirmed 标签均不提供这些事实。当前收据并未在本包证明已获授权、持久
接受或对应实际清理；完整系统结论继续单列。
