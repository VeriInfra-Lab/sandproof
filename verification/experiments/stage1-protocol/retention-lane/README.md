# 保留窗口的完整校验

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包面向本分支采用后的唯一 Rust 实现。正确准备视图只追加透明调用根；错误视图只修改指定真实表达式，不复制替代算法或手改生成体。历史 a9c 候选和原检查证据保留在对应旧 PR，不迁记为当前输入通过。当前适用版本与验收状态见[覆盖记录](../../../../docs/proof-coverage.md)。

`FullLaneContract` 连接当前 `LifecycleRetentionLane::valid_using`。current_window 必须等于 retired_through 的 checked successor；最大 floor 的 successor 为 None。无 last 时仅接受 floor=0，不额外校验 slot。

有 last 时依次要求正 floor、Cleaned、完整终态 self-match、完整 accepted/context 校验、相同 slot、retention.window=floor，然后才编码。编码错误拒绝，8192 字节界保留，外层失败/发散及短路不变。六个真实错误版遗漏后继、状态、上下文、slot/window 或编码界，均使用同一合同。

## 检查与边界

`native.py --output ...` 保留正确版、真实错误版和恢复版的同一断言、格式、严格 Clippy 及实际 fresh 库/测试程序检查。历史 `candidate` 运行标签现在选择已采用的同一生产体，不再应用候选算法。编译失败、导入失败和超时不是性质反例。

`formal.py --variant correct --output ...` 重新抽取实际根，核对选项、透明调用、完整生成函数/外部符号和全部真实文件的同服务完整诊断及依赖审计；其余 variant 沿 `prepare.py`/`mutations.py` 登记。拒绝 stale/partial、输入变化和业务 opaque。持共享重锁，使用独立 `.lake/build`，保留失败和缓存。

`--require-native` 消费 ning 对同一输入的正式普通/组件封口；`--development-native` 仅用于开发，`--prepare-only` 不是证明完成。公共登记和受影响验收在本次 integration 采用批次集中交付，不逐个重启旧 PR。共享整套、ning 和 E2E 结果分别报告。

以上从已解码值或明确效果接口开始，依赖逐符号合同。完整字段、摘要值和状态标签不推出认证、资源存在或持久接受。V7 提供独立认证/配置来源，V1/V2 提供当前意图与耐久发送，V4/V5 提供本地结果及事务事实，V6 提供真实文件/VM/清理。真实 JSON/SHA/SQL 连接、同事务/未知提交/重启及完整系统结论按 V3 范围逐项登记，不把整个 Serde、handler 或收件事务列为可信。
