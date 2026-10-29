# 生产 HostObservation 与完整回执的值连接

[组件入口](../README.md) · [通用回执候选](../current-receipts/README.md) ·
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包复用当前回执候选的同一个公共 checkReceipt 及已证通用对应，连接生产
`HostObservation` 的实际 ResourceEvidence 实例。正确私有视图只向 node 的模块入口
追加透明根：两个原回执根继续从原文件加载，另三个根只转发资源 getter、泛型校验和
真实 `WorkAction<EffectId>` 校验。生产、Cargo 和公共配置不修改，生成体不手改。

`ResourceGetterExact` 在这里由生产 getter 的真实生成体证明，返回同一观察的完整
resource 字段。具体 FullHostReceiptContract 不再把该 getter 方程作为外部条件；
仍使用原版本→全文身份→观察资源的精确规格，资源种类和原全部字段都保留。
类型保留 HostObservation 的 resource/command/result，全部十二种观察结果及其载荷，
完整 Command、EffectId 和回执。没有添加库函数、业务 opaque 或另一份协议模型。

正常 Snapshot 观察使用真实 EffectId 类型；错误资源与错误身份分别给出原类别拒绝。
真实 owner-only Rust 错误版通过同一实际字典，却接受 Snapshot 动作配 Workspace 回执，
从而否定同一个完整合同。两版 Definitions/Values 及共享纯规格相同，错误版不导入
正确版实现定理。准入回归另用实际 IR 检查字段/boot/可选观察载荷遗漏和业务 opacity；
这些回归不是程序性质反例，错误 Rust 必须实际编译、抽取并到达反例。

## 开发复现入口

按证明工作流持已有共享重锁，复用固定工具和 packages；每版独立 source/target/.lake/build：

```sh
python3 verification/experiments/stage1-protocol/host-receipts/development.py \
  --variant correct --output artifacts/host-receipt-correct
python3 verification/experiments/stage1-protocol/host-receipts/development.py \
  --variant owner-only --output artifacts/host-receipt-wrong
```

该入口核对完整源/模板与固定库、真实编译/抽取、根和全部生成符号、完整字段/标签及
实际 IR 准入回归；然后装配导入闭包，经原检查器执行全部实际文件的内核、同服务完整
非 stale/partial 诊断与依赖审计。`--prepare-only` 仍编译/抽取，但 complete=false。
缺失依赖、跳过、超时、编译/导入失败都不算性质拒绝，不关闭检查或过滤字典字段。

这是开发入口，不签发普通/native 或公共形式验收回执。它依赖父 current-receipts 候选，
只在私有副本应用其公共纯规格拆分/Snapshot 补丁；公共树仍待 V0/主控协调采用。
旧、新生成类型需保持独立视图，不能同时塞入含重名 discriminant 实例的旧总根。
正式登记、根导入、冻结、覆盖及 ning 正确/错误/恢复原生验收另由主控统一，
不能仅跑旧 verify-stage1-protocol.sh 说本包已验收，不加入当前冻结交验批次。

## 结果的范围

此处证明 getter 读取原字段，不证明该字段来自可信观察。回执校验不检查观察内的
command/result；一般不变引理及不匹配命令/结果仍被接受的具体值保留这一边界。
原 probes_match 的 HostEvidence::validates、原进程 pid/start/boot 与独立 action 资源、
同 World/sandbox 来源和外层长度/重复快照检查仍须分别接线。观察真实性交 V6，
认证上下文交 V7；完整 Ready、JSON/Serde 字节、SQL/SHA、提交未知/重启及系统结论
不从本包推出，完整 V3 仍未完成。
