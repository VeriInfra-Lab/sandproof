# 第一阶段协议的实际同步入口

[文档目录](../../../docs/README.md) · [V3](../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../docs/proof-workflow.md)

本 crate 只调用实际 sandproof_protocol::lifecycle 方法，不复制验证算法。请求守卫、全文重放、manifest 及 CaptureId/完整摘要载体分别作为固定原 Charon/Aeneas 的小根预检；生成成功与 Lean 对应证明、普通测试和完整验收分开报告。实际状态与精确候选见本批 PR 和覆盖记录。

普通测试、格式和 Clippy 在独立测试环境；Cargo/MIR、原工具、Lean 及同一 lean_session 标准客户端在形式验证环境。工具和库版本固定；原始 LLBC、完整字典、生成物、命令/配置与失败日志保留在私有 evidence，不手改生成 Lean，不以抽取失败作为性质反例。

字段和原方法已交付给并行消费者，不表示新 HTTP profile 已启用。原 v2 继续原语义；固定 serde/派生和既有有界 Writer 的窄合同仍须绑定实际输入、原参数/错误和原对象。manifest 形状不证明真实文件/兼容/持久发布，普通值析构也不提供任务取消或物理资源退出保证。
