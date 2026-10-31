# 原生退休收据的完整值校验

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包面向本分支采用后的唯一 Rust 实现。正确准备视图只追加透明调用根；错误视图只修改指定真实表达式，不复制替代算法或手改生成体。历史 a9c 候选和原检查证据保留在对应旧 PR，不迁记为当前输入通过。当前适用版本与验收状态见[覆盖记录](../../../../docs/proof-coverage.md)。

`FullSavedContract` 连接当前 `validate_saved_using`。无收据直接成功；有收据先查同一 Session.last_entry，再检查 terminal、完整 local、Destroy、当前版本和本地操作。随后编码完整 intent、计算并比较完整 digest，再检查收据与 quiescence 的身份、epoch/operation 及四项 Confirmed，最后编码完整收据并检查 8192 字节界。

record、shape 和保存收据复用同一生产末项查询。此方法本身不调用 accepted.validate，不额外校验 workspace 或 retain_until；这些字段仍进入完整收据编码。六个错误版弱化 local、遗漏 digest/worker/epoch、提前编码或放宽长度，合同不变。

## 检查与边界

`native.py --output ...` 保留正确版、真实错误版和恢复版的同一断言、格式、严格 Clippy 及实际 fresh 库/测试程序检查。历史 `candidate` 运行标签现在选择已采用的同一生产体，不再应用候选算法。编译失败、导入失败和超时不是性质反例。

`formal.py --variant correct --output ...` 重新抽取实际根，核对选项、透明调用、完整生成函数/外部符号和全部真实文件的同服务完整诊断及依赖审计；其余 variant 沿 `prepare.py`/`mutations.py` 登记。拒绝 stale/partial、输入变化和业务 opaque。持共享重锁，使用独立 `.lake/build`，保留失败和缓存。

`--require-native` 消费 ning 对同一输入的正式普通/组件封口；`--development-native` 仅用于开发，`--prepare-only` 不是证明完成。公共登记和受影响验收在本次 integration 采用批次集中交付，不逐个重启旧 PR。共享整套、ning 和 E2E 结果分别报告。

以上从已解码值或明确效果接口开始，依赖逐符号合同。完整字段、摘要值和状态标签不推出认证、资源存在或持久接受。V7 提供独立认证/配置来源，V1/V2 提供当前意图与耐久发送，V4/V5 提供本地结果及事务事实，V6 提供真实文件/VM/清理。真实 JSON/SHA/SQL 连接、同事务/未知提交/重启及完整系统结论按 V3 范围逐项登记，不把整个 Serde、handler 或收件事务列为可信。
