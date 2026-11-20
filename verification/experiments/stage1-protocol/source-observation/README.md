# 原始源码观察守卫

返回 [V3](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)。Refs #635。

固定 `9c50da7cc9391518514f14405e6c779e5c5cbe2c`，生产只读。薄 lib 根
原样调用 `RawSourceObservation::matches`，Types/Funs 为固定原工具输出。

`FullObservationContract` 使用同一 `Accepts`：完整32字节工作绑定、资源kind/id、
进程pid/start/boot、配置非空且不超上限、内核/磁盘字节数为正且不超各自上限。
`matches_exact`、`full_observation_contract`、接受/拒绝iff及总性适用于所有普通
已解码值，正常/边界/遗漏见证不替代一般定理。

三个真实Rust副本分别漏工作绑定、只比PID、漏配置上限；同一Spec/Values/Examples
用于正确和错误环境。错误版须编译、抽取并到达同一完整合同的反例；编译/导入
失败与超时不算性质拒绝。错误环境不导入正确Core/Witnesses；PID版也不导入
已删除的进程比较支持。FunsExternal仅复用A6的String::eq和Vec::is_empty；
长度/转换、数组与资源/进程比较保留实际生成体，不导入axiom模板。

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/source-observation/formal.py \
  --output artifacts/source-observation-formal --lock /path/to/shared-proof-heavy.lock
```

`native.py --output artifacts/source-observation-native` 在独立源码副本运行17项
断言，含原 `save_source_observation` 的三项入口检查。正确/恢复全过；漏绑定
精确2项、只比PID精确3项、漏上限精确2项失败。错误调用须实际返回Ok后违反
原InvalidHostEvidence断言；不把夹具错误算反证。每视图新target并检查协议库与
测试程序来自本源码且新编译，核对格式/严格Clippy、源模式、工具、库和原日志。
本机开发不签ning正式接纳；原生与形式记录绑定相同输入和模式。

本包从已解码值开始，不证明Config::valid、JSON字节解析/兼容、摘要计算或
物理观察。`hash_fields_independent` 明示原守卫不读可执行/内核摘要；node、
bootstrap及其它限制也未被暗增为接受条件。V6提供进程/材料事实，V7认证，
V1/V2当前意图和耐久发送，V5/A4保存事务/未知结果。原生调用检查不构成保存
事务或完整提供者连接证明。完整V3仍保留18项要求/49项义务。

公共Lake、登记、依赖及覆盖只交同目录candidate.patch，由主控单写采用并通过
真实verify-stage1-protocol.sh、proof_native_suite和普通完整验收；局部通过与
公共全套结果分别报告。原执行身份、路径、失败和SHA留在忽略artifacts与#635/PR。
