# V3 真实封套字节入口

[V3 义务](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md) · [上级组件](../README.md)

本目录使用当前源码中的唯一 `decode_accepted_with`。真实
`server/lifecycle.rs::respond` 在有限 body 收集后调用该核心，并传入原
`serde_json::from_slice` 和原 context。它没有调用独立的 request/manifest codec。
这些独立函数的抽取根保留在本 crate；不能用其结果宣称真实 HTTP 收件接线已证。

固定 Charon 能捕获原 codec，完整 Aeneas 翻译遇到 Serde 递归 trait、visitor 和部分移动障碍。
`accepted/production-candidate.patch` 保留为已采用拆分的历史来源，驱动不再应用它。
当前生产路由保持解析拒绝为 400、封套拒绝为 409；有限读体、后续 handler、完整
recorded 匹配和响应仍按真实调用核对。正确版读取当前生产体，错误版只在私有副本
移除唯一守卫；不能把旧候选的结果自动认作当前实现已经通过。

## 候选对应合同

`AcceptedWireModel` 复用原 `ValidationModel.expectedAccepted` 和完整 `ValidEnvelope`，
不重写协议模型。`WireContract` 对任意原回调和输入规定完整结果：先调用解析回调，
解析拒绝优先；成功后运行原完整封套守卫；接受时保留全部 intent 值。回调的失败、发散和
可见效果保留，不能用永远成功的桩代替。正确版证明所有参数；有限 value reader 只提供合法
接受与外来 center 拒绝的非空见证，**不是实际 JSON 字节或 parser 正确性的见证**。

生成体保留实际 `AcceptedLifecycleIntent::validate` 及原 request/envelope 四根，复用原
String、Vec、Bool、Option 四个外部模型。新增核心没有自有 opaque 函数。原 Aeneas 正常返回域
及 Drop 擦除限制仍在：具体使用点是固定 SliceRead parser 和普通值/错误，不推广到具有业务
析构、FD、许可或自定义回调清理的任意 Rust 对象。外部效果的数学保持不提供 async 取消/退出证明。

`skip-envelope-guard` 在隔离的真实 Rust 中唯一移除原 `intent.validate(context)`；两版
使用逐字相同的完整合同。错误版必须实际编译、抽取，并证明同一 `WireContract` 不成立；
编译失败、导入失败或超时均不算性质拒绝。其余 request/envelope 证明也在两版实际生成视图中检查。

## 局部检查与待接纳范围

已有固定工具和本任务私有 `.lake/packages` 准备好后，持共享重锁运行：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/json-wire/formal.py \
  --output artifacts/json-wire/formal
```

每版有独立源码、Cargo target 和 `.lake/build`；同任务依赖缓存只在串行重锁内复用。
驱动核对固定工具、真实自有包重新编译、完整原始 LLBC、实际生成体、四个外部模型、
完整导入闭包、逐物理文件内核与同服务非 stale/partial 诊断、准确审计及同合同反例。
`--case` 仅为诊断，报告 `all_cases_complete=false`。驱动不签发或伪造 ning 原生封口。

仍待完成：主控接纳生产候选及原路由机械接线审查；协议 4 全 schema、全部自有叶子与实际
parser 字典/输入连接；原 body 限额、Writer 和客户端结果字节连接；全部 observation、确认和
窗口消息；ning 正确/错误/恢复原生证据；公共登记、完整组件和共享批次验收。
认证 context 来源由 V7 提供；耐久发送、接受、文件和 VM 事实仍由原责任模块提供。
本候选的局部通过不表示当前生产 JSON 已证明，更不表示持久收件、TLS 或 V3 已完成。
