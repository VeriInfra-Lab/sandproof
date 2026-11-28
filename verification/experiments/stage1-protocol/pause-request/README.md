# Unload 到 Pause 请求的完整投影

[协议范围](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包采用真实 `SourceWork::pause_request` 的两行局部变量改名，将 `capture` 绑定为 `capture_id`，避免固定生成 Lean 中的模块名遮蔽。Rust 仍只有一份原方法，分支、字段、调用和失败语义不变；不声称一般新旧等价已经机器证明。原名称失败和候选的原始证据保留。

全部数学模板从已完成候选逐字复用，来源和摘要见 `reuse.json`，不重写合同或克隆数学。`FullPauseContract` 对任意已解码完整 SourceWork 给出 Unload 的完整请求：原操作 binding、wire source/capture、完整 work binding、node、resource 和 process；其它四类操作返回 Conflict。正确/三错使用相同 Spec、Values、Examples，错误版分别误取 execution_origin、native generation 和 options 摘要。保留正常 Unload 及四类非 Unload 见证。

透明根 `probe.rs.in` 只在独立验证副本加入，恰调用同一真实方法一次，不替换业务算法或提交额外生产入口。Charon/Aeneas 固定版本、实际生成文件、空 exclude、借用/类型检查和原单一 rusqlite Error 载体保持。String clone 及普通错误值载体复用现有合同；没有新增库模型。正式入口检查全部实际文件、原服务的完整诊断及依赖审计；生成体不手改。

```sh
python3 verification/experiments/stage1-protocol/pause-request/native.py --output artifacts/pause-request-native
python3 verification/experiments/stage1-protocol/pause-request/formal.py --output artifacts/pause-request-formal --lock artifacts/.proof-heavy.lock
```

原生入口在独立源码和 target 中执行原版、采用版、三错和恢复版同一组十项断言；它是本机开发检查，ning 仍负责普通完整与正式原生验收。公共 Lake/入口/冻结/依赖/覆盖只交候选。

原方法只检查操作类别，不调用 PauseWorkRequest::valid。它允许给定 source 中的 pid=0 等值被原样投影；字段相等和返回成功不证明当前授权、已持久接受、资源存在或执行过 SHA。标准 JSON/SHA 与当前请求/完整工作绑定来源、加载/保存及真实调用者/结果连接仍属 V3；V1/V2 意图与耐久发送、V4/V5 本地接受/事务未知与重启、V6 物理事实、V7 独立认证分别保留。本包不关闭完整 V3。
