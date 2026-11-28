# 收件条目的完整类别校验

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包面向本分支采用后的唯一 Rust 实现。正确准备视图只追加透明调用根；错误视图只修改指定真实表达式，不复制替代算法或手改生成体。历史 a9c 候选和原检查证据保留在对应旧 PR，不迁记为当前输入通过。当前适用版本与验收状态见[覆盖记录](../../../../docs/proof-coverage.md)。

`FullEntryContract` 连接当前 `Inbox::entry_kind_using`：先校验完整卸载决议，再检查完整 accepted 封套、retention、session、owner、本地操作和续期字段，随后计数所有会话中的完整 OperationKey，最后校验操作参数。计数必须恰好一次，保留 usize 溢出及原首错、失败和发散。

Create 保留 options 界；Unload 要求正来源代和 local；Renew 保留非负期限、两个可选期限成对及 extend_only 的历史最大值；Resume 仍沿公共请求守卫允许 1→9。Session/Entry 的实际成员关系由外层完整校验提供。错误版分别截断计数、只比较 sequence、遗漏历史期限。

## 检查与边界

`native.py --output ...` 保留正确版、真实错误版和恢复版的同一断言、格式、严格 Clippy 及实际 fresh 库/测试程序检查。历史 `candidate` 运行标签现在选择已采用的同一生产体，不再应用候选算法。编译失败、导入失败和超时不是性质反例。

`formal.py --variant correct --output ...` 重新抽取实际根，核对选项、透明调用、完整生成函数/外部符号和全部真实文件的同服务完整诊断及依赖审计；其余 variant 沿 `prepare.py`/`mutations.py` 登记。拒绝 stale/partial、输入变化和业务 opaque。持共享重锁，使用独立 `.lake/build`，保留失败和缓存。

`--require-native` 消费 ning 对同一输入的正式普通/组件封口；`--development-native` 仅用于开发，`--prepare-only` 不是证明完成。公共登记和受影响验收在本次 integration 采用批次集中交付，不逐个重启旧 PR。共享整套、ning 和 E2E 结果分别报告。

以上从已解码值或明确效果接口开始，依赖逐符号合同。完整字段、摘要值和状态标签不推出认证、资源存在或持久接受。V7 提供独立认证/配置来源，V1/V2 提供当前意图与耐久发送，V4/V5 提供本地结果及事务事实，V6 提供真实文件/VM/清理。真实 JSON/SHA/SQL 连接、同事务/未知提交/重启及完整系统结论按 V3 范围逐项登记，不把整个 Serde、handler 或收件事务列为可信。
