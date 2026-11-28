# 暂停 Stop 的实际派发、回执与发布

[证明工作流](../../../docs/proof-workflow.md) · [覆盖与适用版本](../../../docs/proof-coverage.md) · [依赖合同](../../../docs/formal/dependencies.md)

本切片用固定的原 Charon/Aeneas 抽取真实同步控制代码，连接普通暂停的材料登记、Stop 派发、原 VM 停止、两份回执和暂停发布。它不证明整个 V4、异步调度器、SQLite 耐久性或物理 VM 已经退出。实际验收状态及适用源码只在覆盖记录和对应 issue 中维护。

## 真实入口与中间状态

生产 `handle_effect_dispatch` 实际调用 `effect::dispatch_stop`；Stop 分支与其它分支共用 `dispatch_eligible` 和 `dispatch_effect`。抽取根还包括原 `worker::pause_stop::record`、`effect::acknowledge_effect` 和 `publication::handle_publish_paused`，复用已有 `CaptureCaller` 生成环境。没有增加解释器、改写生成算法或升级证明工具。

正常接受验收必须连接同一条轨迹：真实 Commit 在 Stop 原先缺失时登记材料并创建完整 Stop → 实际选择和派发这个 Stop → 同一 Frozen VM 的真实 Stop → 这个 Stop 的成功回执 → 同一 Commit 的成功回执 → 原发布入口。每一步使用前一步实际返回的完整 World；不能把两个独立的存在性结果拼成一条轨迹。其它 Fence、Freeze、Seal 的既有结果仍须保留。这个有限输入域见证不是 V9 的可达历史或系统进展证明。

## 分别提供哪些结论

- `same_property_accepts` 与独立错误版的 `same_property_rejects` 使用同一个 `PreservesBoundStop`。它要求事件带同一个 sandbox、epoch、generation、VM、attempt，目标 effect 行为 `Dispatched`，effects 表以外的整个 World 保持不变。
- 这个单行性质本身不限制 effects 表内其它行。`stop_update_exact` 另证明实际写入恰为原 `Store.put` 的返回结果；`put_exact_at` 再连接其精确列表更新。原生断言保留整张 effects 表的比较。
- 派发只接受原规则允许的状态。它不检查 attempt 是否当前，也不授予停止许可；真实 worker 随后仍检查当前尝试、完整目标和登记许可。不能把“派发成功”解释为“VM 已停止”。
- 回执证明连接实际 `acknowledge_effect`。当前成功写入对应逻辑结果；非当前尝试不改变当前成功检查，失败回执不改写逻辑结果。不同完整命令的回执不会冒充彼此。
- 发布组合执行两次实际回执函数，再构造原发布入口的接受结果。`ReadyRest` 保留原发布输入，明确去掉由这两次执行提供的 Stop、Commit 成功检查；没有把未来回执或未来发布成功放入前提。

## 证明与边界

新增 Lean 辅助定义必须证明与实际生成函数对应，不能成为新的可信业务规则。依赖模型沿用原登记的 11 项；本次所用 `Option::as_ref` 捕获原 Std 函数体，不新增其外部模型。

一般 `handle_effect_dispatch` 和完整 controller 回执入口没有在本切片中整体抽取；其实际转发、首次拒绝、重复和冲突回执由源码/中间表示审查与有限原生调用检查支持。不得将这些检查写成一般性实现等价定理。纯状态中的停止、登记、索引关闭也不替代 Firecracker 退出、网络收尾和持久存储的外部保证。

固定工具转换、既有 Std/外部模型、获批的锁和任务适配仍按依赖合同承担各自责任。物理分配失败、任意并发交错、取消、自动 Drop、真实调度公平性及整个系统的可达性不由本切片新增结论覆盖。

## 验收与复现

普通检查在普通测试环境执行，抽取、Lean 和审计在证明环境执行，使用既有私有 wrapper 与协调锁。原生错误版必须实际编译、通过严格静态检查，并被与正确版相同的身份绑定断言拒绝；编译失败或工具失败不是性质反例。正确、错误、恢复版和实际测试二进制来源检查均保留。

父入口的固定 `pause-dispatch` 范围同时保留完整 Linux 功能检查、本组件及受共享生成闭包影响的三个原组件。每个组件各保留原始证据；不能用本范围代替 `all` 或整个 V3/V4。父入口定义及实际登记以公共脚本为准，不提供任选成功子项的参数。

组件原生 CLI：

```sh
python3 verification/experiments/pause-dispatch/native.py \
  --archive SOURCE_TAR --archive-sha256 SOURCE_SHA --head SOURCE_COMMIT \
  --output PRIVATE_OUTPUT --lock PRIVATE_COMPONENT_LOCK
```

证明机使用同一份源归档、原工具和包缓存，通过原 wrapper 执行：

```sh
python3 verification/experiments/pause-dispatch/replay.py \
  --archive SOURCE_TAR --archive-sha256 SOURCE_SHA --head SOURCE_COMMIT \
  --output PRIVATE_OUTPUT --lock PRIVATE_COMPONENT_LOCK \
  --tools ORIGINAL_TOOLS --packages ORIGINAL_PACKAGES
```

正式准入须核对全部实际文件、完整声明与审计、正确/错误生成字节、同源负例、工具、来源和冻结输入。缺失任一项就拒绝。探索脚本通过、客户端退出码为零、单个组件完成，分别都不能代替正式范围封口；缺少原始 runtime/key/receipt 的记录不能事后补造成可复用收据。完整功能检查只在最终稳定候选执行一次。
