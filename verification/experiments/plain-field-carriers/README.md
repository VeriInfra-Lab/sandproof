# 普通字段载体的原式签名预检

[文档目录](../../../docs/README.md) · [V0计划](../../../docs/formal/00-foundations.md)

本批对应V0/T1的两个有限问题：真实本地数值ID能否构造/取值，真实
`ContractVersions::current`能否保留三个准确机器类型并经固定原工具生成Lean。
实际运行状态和精确提交以本批PR/回执为准；源码存在和委托编译成功不表示抽取通过。

只选原 `sandbox_controller_core::TenantId`、`SandboxId` 的 `new/value`，以及原
`sandproof_protocol::internal::ContractVersions::current`。五个自由根直接调用生产方法，
不复制另一份构造算法或版本记录。两个ID是普通u64载体，0和U64MAX均是正常输入，
不能套协议字符串编号或PositiveCounter的合法域。

版本记录保留internal_protocol:u16=2、center_store:u32=6、host_store:i64=16；
不能因这次常量为正就把整个有符号字段的表示改成Nat。动作协议另有自己的版本1，
没有把它当作此记录的内部协议版本。

本批只做原Rust编译、固定Charon/Aeneas的真实透明抽取可行性、完整中间表示/生成签名
及原始回执。保留源码、调用根、类型/函数/trait字典、选项、opaque和Drop/失败阶段，
不手改生成物或丢弃源位置。没有新增Lean模型、定理、错误版或可信保证，不认领已证明
构造往返或整个T1。G3字节/文本和G4业务期限不属于本批。

普通Rust/格式/Clippy和三个边界例子只在ning的独立target按既有锁运行；证明机仅用
原固定运行环境做抽取预检，新增源码/target/临时文件/证据在数据盘。等待证明机锁前
释放普通测试机锁，不搭建新CI或复制运行环境。没有修改生产Rust、Charon/Aeneas、
共享Lake/根/审计/套件/使用目录或权威文档。

## 后置检查与原回执复用

原抽取和新的后置检查分开记录。首轮的Charon/Aeneas即使都退出0，后置检查失败仍是
该轮整体失败，不能改写原`result.json`。本批曾将字段写成`I64`，误拒绝原生成的
`Std.I64`；修正只精确接受原版本记录的`Std.U16/Std.U32/Std.I64`及三个带宽度常量，
不接受`Nat`、无符号替换、宽度交换、漏字段或额外字段。

原工具把两种本地ID分别生成为带不同名称、可归约的`Std.U64`别名。原Rust名义类型
仍然不同，但不能据此宣称Lean类型检查会阻止这两个别名互换。后续对应证明须绑定
准确的Rust函数与字段；本预检不新增名义类型模型，也不证明所有十种本地ID。

当全部原编译输入、依赖、运行适配和工具配置不变，仅检查器和本说明改变时，在ning
读取原完整产物做一次独立后置核对。`review`只接收本批保留的准确首轮失败回执，验证
两个原工具退出及日志摘要、全部未变输入和完整生成物；不调用编译器、Charon、Aeneas
或Lean。新结果标明复用的是原工具步骤，原整体失败及其traceback继续保留。

```sh
bash scripts/python-entry.sh verification/experiments/plain-field-carriers/run.py review \
  --original "$plain_fields_original_extract_evidence" \
  --output "$plain_fields_new_review_evidence"
SANDPROOF_CARRIER_ORIGINAL="$plain_fields_original_extract_evidence" \
  bash scripts/python-entry.sh verification/experiments/plain-field-carriers/test_inspection.py
```

回归使用完整原生成物和临时副本检查准入，修改副本的字段/常量/opaque只是检查器的
有限回归，不是真实Rust错误版或新语义反例。原完整LLBC、字典、配置和模板均保留。
`precise_drops=false`等原默认值照实登记；普通Copy载体的清理记录不能用于背书任意
Drop、panic/unwind、异步取消或资源释放。生成Lean与内核检查、定理和整套验收仍分开。
