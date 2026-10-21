# 外部状态回调的隔离连接

本批对应 #333 的 T3/T4。使用固定原 Charon/Aeneas；生产源码和旧失败原型不改。

## 连接哪一段

原 `controller-core/src/store.rs::Store::retain` 把原始行逐个交给
`FnMut(&K, &V) -> bool`。原测试已经使用外部访问记录和调用计数。
本批在隔离 crate 中把谓词体拆成两个真实 Rust 函数：

- `toggle_step` 改写原外部 bool，并把改写后的值作为保留决定。
- `trace_step` 向原外部 Vec 追加完整的原键和值，再比较原 floor。

`retain_toggle/retain_trace` 保留原 Store、原外部状态和原参数，闭包只调用
对应 helper 一次并返回它的原结果。没有把状态移进私有副本、到最后再复制回来。
trace 是原 Store 测试中谓词体的参数化拆分；toggle 是代表性隔离实例，
不是新增生产调用。实际生产清理谓词的业务正确性不由这两个实例代替。

## 哪些结论来自哪里

| 内容 | 提供方及检查 |
| --- | --- |
| 两个 helper 的赋值、完整参数、返回决定和 Vec 值更新 | 真实 Rust 经原工具生成，Lean 检查该生成函数的合同 |
| 原 Vec 正常 retain 按原位置依次调用一次，并按返回 bool 保留原行 | 已有 A6；重复键和相同行也分别按位置处理 |
| 闭包把同一个外部状态、原键和值、floor 交给 helper，并沿用其原结果 | 窄 B4 机械绑定，固定完整源码和原生对照；不是整个 FnMut 的机器精化 |
| 逐次结果如何连接成末态、访问次数和幸存行顺序 | Chain/Selected 条件理论，复用已有 tableRetainState |
| 哪些行允许删除、状态不变量、容量和实际业务错误的含义 | 使用本接口的下游模块证明 |

`Chain` 是已完成调用的证据：每一步保留真实调用等式以及下一步使用的状态。
`Selected` 单独表达库按同一批 bool 选择原行。它们不运行 Rust，不解释 MIR，
也不把最终模型等式作为前提。组合定理据此推出最终对应。

上层需要提供原 Store/状态的身份、有效安全借用、每个 helper 的输入域和实际接线。
trace 还需要整个访问记录可表示且分配正常；模型中的 usize 界不是物理 RAM 保证。
泛型使用者还须重新证明自己的真实谓词体，不能因为签名为 FnMut 就假定它正确。
有状态比较器、自定义 Drop、并发访问同一捕获和任意异步回调不在这两个实例内。

## 中断与错误

这个真实回调签名返回 bool，没有业务 Result 错误短路。不得把 false 说成错误。
本批正常返回定理不覆盖 panic、OOM/abort 或 async 取消的完成性。

中断观察只保留中断前已经正常完成的调用历史，以及一个尚未正常返回的调用位置；
它不把“上次完成后的状态”冒充中断后的当前堆，也不指定中断后的 Store 幸存行。
失败调用本身可能已有部分效果。两项原生 panic 试验确认选定路径的外部变更仍可见，
这是实际测试证据，不是原工具对 Rust unwinding 的机器证明。

## 验收与共享集成

普通测试、格式和 Clippy 在 ning。形式编译、原工具抽取、Lean、实际文件诊断和
依赖审计在 a6000；其新增数据位于 /data1/zn。每版使用自己的源码与构建目录。

错误版分别仅删除真实状态赋值、重复一次真实 Vec::push。每版须先成功编译抽取，
再检查正确版同一个 ToggleContract / TraceContract。编译、导入或服务失败不算反例。

共享主/独立 Lake、总根与审计、普通和形式批次、实际文件服务、使用登记及权威文档
由统筹接入。本窗口组件通过不代表原完整入口、V0 或部署系统全部通过。
完整模块入口必须要求原 acceptance_receipts 的 callback-state case；
未登记、未执行或输入改变时应拒绝，不提供无证据跳过开关。

复现前提交全部冻结输入，并按全局重锁串行占用对应机器；不要持一台机器的锁等另一台。
同一机器的每次输出目录必须是新目录，旧失败记录不覆盖。

```sh
# ning：需要冻结的 Rust 1.97.0，以下不是 Lean 验收
flock -x ~/.cache/sandproof-v0-coordination/native-heavy.lock \
  bash scripts/python-entry.sh tools/check_callback_state_components.py --output /private/path/native-run

# a6000：固定工具和 .lake/packages 已配置，TMPDIR、target、输出均在 /data1/zn
flock -x ~/.cache/sandproof-v0-coordination/formal-heavy.lock \
  bash scripts/python-entry.sh verification/experiments/callback-state/replay.py \
  --tool-root "$SANDPROOF_PROOF_TOOL_ROOT" --output /data1/zn/private-path/formal-run
```

形式组件导出精确 HEAD，在私有副本中补充本模块的 Lake 注册；
所有旧注册保留，每版 .lake/build 独立，固定包只读复用。
它重抽三版完整 LLBC/Lean/JSON，检查内核、准确审计目标及九份实际文件诊断。
正式批次仍须由统筹接入并运行原完整入口；上述单模块命令不代替它。
