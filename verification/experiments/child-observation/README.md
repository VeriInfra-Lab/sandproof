# 子进程停止观察：隔离候选

返回[证明工作流](../../../docs/proof-workflow.md) · [依赖合同](../../../docs/formal/00-foundations.md) · [T8 范围登记](https://github.com/VeriInfra-Lab/sandproof/issues/333#issuecomment-6295842064)

这项试点只处理 `sandbox-runtime/src/child_process.rs::OwnedChild::stop_and_reap` 的正常返回。生产实现保持原样。`candidate-core.patch` 在完整冻结源码的隔离副本中拆出同步控制核心和 9 个机械适配接口；核心及实际调用方都交给原版 Charon/Aeneas 抽取。它不是 Rust 或操作系统的独立解释器。隔离核心用显式 `Some` 匹配替代三处取值检查；正常路径及取值顺序保持不变，缺值的 panic 诊断和异常清理不作一般等价声明。这也避免原库 `toStr` 默认字符串证书引入 `native_decide` 依赖；原生成体不手改，审计不放宽。

## 可以使用的结论

入口必须同时持有原 `Child` 与预占原 `Slot`。在已登记接口的正常返回保证下，实际抽取的核心满足同一条 `Accounted` 性质：

- 成功返回哪个完整退出状态，历史中就必须有对原 `Child` 的同一个退出观察；本地仍持有原名额。
- 返回原“退出结果未知”错误之前，必须已把原 `Child` 和原 `Slot` 一并交给原 reaper；本地两项都已移出。

`kill()` 成功、关闭管道、到达截止均不是退出观察。`try_wait()` 的 `None` 和普通 `Err` 也不是退出证据。成功退出状态不等于退出码为零。

`NormalCalls` 只要求每个底层接口保留原对象、原参数和该次正常回复；不先假设整个业务正确。`Refinement.stop_accounted` 是原生成函数的通用证明；`Concrete.caller_model_accounted` 又检查实际生成的 `OwnedChild` 调用方和具体接口字典。`Meaning` 中的原生退出/交接含义仍由依赖绑定提供。模型定理不能替代对标准库、Linux 或 reaper 的信任。

## 接口保证和边界

精确函数片段、版本摘要、正常回复、故障含义和下游条件在 [contracts.json](contracts.json) 登记。概览如下：

| 接口 | 正常返回提供什么 | 不提供什么 |
|---|---|---|
| `system` | 构造空适配器 | 不执行 OS 操作；逻辑历史不是 Rust 空结构的真实字段 |
| `try_wait` | 原 Child 的完整 `Some(status)`、`None` 或错误；已有缓存状态保持原含义 | `None`/错误不说明仍活着或没有变化；一个进程结束不等于全部后代结束 |
| `kill` | 保留对原 Child 的终止请求及完整回复；缓存退出时允许无需发信号便返回成功 | 成功不是回收；错误不保证外部世界没变 |
| `close_pipes` | 原 stdin、stdout、stderr 依次取出并析构，正常后本地字段为空 | 不保证排空、关闭全部复制 FD、全部子进程退出；中途失败不回滚 |
| `now` / `add_grace` | 每次重新读取原时钟；原值加同一 2 秒常量 | 不保证时钟必然推进或整个函数两秒内结束；不可表示的加法不属于正常后态 |
| `pause` | 同一次原 10ms sleep 调用 | 不保证调度时限、退出或任何公平进展 |
| `retain` | 原 pair 曾进入原 pending，随后调用 notify；这是历史事实 | 返回时 worker 可能已完成回收；不证明当前仍在队列、全局计数或 reaper 释额策略 |
| `unknown_exit` | 原错误类别和完整文本 | 错误不是“没有效果”的凭据；OOM/abort 不提供正常后态 |

原 `Child` 由有效创建身份和回收权绑定，不能把可复用的裸 PID 永久当原对象。实际 Linux 标准库含已缓存状态、pidfd 和 PID 回退分支。调用方必须保证没有别处先回收后还向被复用 PID 发信号；本批不证明这项宿主生命周期条件。

形式 `Child`/`Io` 中的响应流和历史是每次调用的逻辑上下文，不声称 Rust `Child` 内含这些字段。每种 API 使用独立响应流，错误版跳过某项调用也不会被模型预先卡住。其他线程、OS、时钟和队列可继续变化；这里没有冻结整个外部世界。

普通 `io::Result::Err` 是必须保留的正常回复。panic、unwind、abort、不返回、部分关闭或部分入队没有本定理的正常后态，不能据此推出回滚、回收或释额。异步取消和任意资源析构仍在本批范围之外。

## 正反例与真实实现的关系

正确版和错误版使用相同 `Model`、`Library`、`Fixtures` 和 `Accounted`。错误版是真实 Rust：收到 `kill` 成功后直接 Drop 原 Child/Slot，再返回未知错误。它仍会经过原工具抽取；Lean 证明它正常返回，却没有原 pair 交接，因此违反同一性质。不是把翻译失败当拒绝。

错误版额外出现的两个 `core::mem::drop` 只使用这两个被冻结类型实例的正常返回投影；允许真实 Slot Drop 归还名额等未观测外部效果。模型只记录适配接口历史，不虚构 Drop 等同 `retain`，也不把它当一般析构定理。ning 的两个独立进程反例另用真实 Child、Slot、kill、retain 和回收清理检查同一责任断言。

原实现和候选分别运行真实 OS 回归；受控 `None`/`Err` 和时钟用于检查真实原 pair 的交接，不声称 Linux 必然返回这些注入回复。回归不是一般新旧等价证明，也不是生产启用。未来若不采用同一核心，仍须提供另一条实现连接。

## 复现与证据

遵守[证明工作流](../../../docs/proof-workflow.md)的主机分工和重任务锁。普通 Rust 测试、Clippy、流程脚本自测只在普通测试机运行；抽取、Lean 编译和原 Lean 服务检查只在证明机运行。新源副本、target、tmp、日志、构建和证据放证明机的数据盘目录，保留既有 Lean 缓存。具体机器地址/目录不写入仓库。

从 `candidate.json.base` 执行 `git archive --format=tar`，把完整归档放在忽略的证据目录；其 SHA-256 必须等于 `source_archive_sha256`。所有输出目录必须是新目录，不能覆盖失败记录。

```bash
# 普通测试机：固定生产工具链，独立 CARGO_TARGET_DIR，锁定离线依赖
python3 verification/experiments/child-observation/native.py \
  --archive "$SOURCE_ARCHIVE" --output "$NATIVE_EVIDENCE"
python3 -m unittest tools.tests.test_child_observation

# 证明机：固定工具、只读包缓存、独立 formal target
bash scripts/regenerate-child-observation.sh --archive "$SOURCE_ARCHIVE" \
  --tool-root "$ORIGINAL_TOOLS" --output "$EXTRACTION_EVIDENCE"
bash scripts/verify-child-observation.sh --output "$LEAN_EVIDENCE"
python3 -m tools.check_child_observation
```

重放比较完整 LLBC，包括函数体、源参数、类型、正常/展开 Drop 和原编译器选项；只沿用现有检查器对输出文件参数及无序短名映射的规范化。原生成文件和模板逐字节比较。Lean 的核心、具体调用方、错误版各有独立视图，逐个物理文件取得原服务完整、非陈旧、无警告/错误的诊断，并核对准确定理清单。它们不替代协调窗口的整套 V0 验收。

## 下游仍负责什么

V6 负责原对象/动作/名额的业务对应、reaper 的实际释放政策、整体计数、异常前缀和 Firecracker/jailer 生命周期。V8 负责协议、缓冲完整排空、命令和会话。B1/调用方负责“停止等待不等于后台工作退出”的资源所有者关系。

`exchange_until` 可能在收尾后返回原 pump 错误；不能仅据公开错误种类反推无业务效果。`pump` 还存在自然退出加管道 EOF 的另一组合路径。`Drop` 和 `into_child` 不是本定理的自动推论：后者会交出 Child 并释放本 helper 名额，必须另有独立的有界 owner。全部 `OwnedChild` API、整个宿主、VM、Guest 命令、全部后代、严格墙钟上界、T9 和历史 #337 均不在本批验收中。
