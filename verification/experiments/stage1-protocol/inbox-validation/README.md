# 完整 Inbox 值校验候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包固定 a9c 的 `Inbox::validate`，复用先前叶子候选，组合完整外层校验。生产只读。
固定工具对原迭代器、标准字节遍历和泛型字典的限制保留在开发证据中；隔离候选拆出
owner、session 和 entry 循环，保留短路、首拒绝、checked 算术及编码回调顺序。
`production-candidate.patch` 供工程审查；原实现一般等价和生产采用仍未闭合。

`InboxValidation.FullInboxContract` 是正反视图共用的精确规格。
`inbox_refines` 对任意 Inbox 和四种编码/digest 参数，证明真实生成的 `validate_using`
等于 `inboxView`，包括错误、panic 和发散传播。配置字符串须满足已登记的表示范围；
完整 `runInbox` 使用独立抽取的标准 ASCII graphic 实现。它不是有限样例结论。

校验顺序依次为配置、session/owner 容量、lane 数量与从 1 开始的有序 slot 及完整
lane 校验、退役身份、可选 host 的完整 node/registration、owner 高水位及两种身份
唯一性，然后按枚举顺序校验所有 session。owner.local=0 没有被额外禁止。
每个 session 依次校验 shape、renewal、全部 entry、lineage 和最终摘要。entry 使用
严格递增版本、Destroy 的前驱豁免、真实 entry_kind 及五个 U16 checked 计数；
Create/Destroy 至多一次。摘要检查 current=max、terminal 与 Destroy 计数、local
及 incarnation 与 Create 的关系、最高版本 Destroy 和同一 owner 的 external/local
配对。当前 Inbox lineage 要求 checked successor；请求 Resume 只要求目标代更大、
允许空洞。两者的兼容缺口单列，不能反过来加强请求规格。

合法空 Inbox 和含完整 Create 的正常见证与字段遗漏反例共同使用上述规格。
三个实际 Rust 错误版分别丢弃 owner 错误、把 lane 外层 `||` 改成 `&&`、丢弃 session
错误。错误视图实际接受重复 owner、错序 lane 或错误 current_version，并在同一
`FullInboxContract` 下证明拒绝；不导入被变异函数的正确对应证明。

## 复跑与依赖边界

`native.py --output ...` 检查原版、候选、三个错误版及恢复版，共用实际 Rust 断言、
格式和严格 Clippy；核对协议库和服务测试的 fresh 源码、manifest 与二进制。错误版
必须到达精确断言，编译失败不算反例。`formal.py --variant correct --development-native
.../result.json --output ...` 为本地开发入口；错误版选择对应 variant。
执行者持有共享重锁，各输出有独立源码、Cargo target 和 `.lake/build`，保留失败证据。

每个 formal 视图重新抽取实际 Inbox 根和独立 graphic/Result 原语根，核对固定工具、
选项、透明单调用转发、全部函数及外部符号。原生成体逐字安装，不手改。
所有真实导入文件经内核及工作流同服务客户端完整检查，审计清单不能缺项。
`inventory.json` 绑定真实生产输入、父包证明提供者和每个视图的精确闭包；新增证明
并不把历史冻结摘要迁记为通过。`--require-native` 要求 ning 完整功能与本组件封口。
本地开发结果不替代 ning、共享套件或 E2E，公共脚本未登记本包时也不能报告它通过。

外部模型按符号复用现有字符串、Vec、Option 字典/回调、Result 和借用迭代合同。
`library_binding.py` 逐字核对旧定义及属性；Result.is_err 直接连接独立生成的标准实现。
ASCII graphic 的 Bytes/all 仍使用已登记合同。完整 Serde、handler 和事务均未被设为可信。
公共 Lake、原生/形式登记、覆盖与 V0 提供者采用由主控处理候选补丁。

## 未闭合义务

此结论从已解码值开始。标准 Serde 字节/错误、真实 SHA 计算、原候选一般等价、生产
采用、完整 Inbox 转换保持、同事务接受、未知提交与重启仍属 V3 后续义务；
ContentDigest 的 32 字节值不证明哈希已计算或材料完整。
V7 提供独立认证上下文来源，V1/V2 提供当前意图和耐久发送，V4/V5 提供本地接受、
结果与实际收据，V6 提供真实文件/VM/清理事实。字段相等、合法值和 Confirmed 标签
不提供这些事实；完整 JSON/TLS/网络外壳和系统结论另列。
