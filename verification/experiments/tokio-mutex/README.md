# Tokio Mutex：原锁与守卫

[证明工作流](../../../docs/proof-workflow.md) · [V0](../../../docs/formal/00-foundations.md)

本批开发中，尚未接入正式整套入口。只覆盖固定 Tokio 1.49.0 的原 borrowed Mutex 的构造、取得、try_lock、等待取消及正常守卫释放，先用现有 RwLock 的独占规则建立条件接口，不开发 async Rust 翻译器。

模型投影记录已返回的访问权，不代表队列和整个运行时。等待中的预留可能让 try_lock 返回 Busy，即使尚没有可用守卫；取消等待不释放别人的守卫。已持有守卫的业务 future 被取消时须另连接其实际 Drop，不得当成等待取消的空步骤。forget 保留排斥；panic 不回滚已发生写入。只在原锁仍存活时讨论内联载荷，不冻结别名可变堆、IO、进程、时钟或资源析构。

真实使用点包括 Session.shared.init 和 command::Shared.input。B2 只承担原锁、原守卫、实际参数/结果、机械调用顺序；令牌/完整请求、CommandInput/FD身份、Started发布、已交付前缀、关闭/完成/许可释放和无环等待仍由 V8 等调用者证明。模型定理与原生回归不称为 async Rust 机器精化。

早期候选与失败记录保留在忽略 artifacts；正式模块先经过 a6000 原 Lean 服务试验再写入。普通回归和真实业务错误副本在 ning，形式证明和实际文件检查在 a6000 的 /data1/zn。T0 待确认动作不由本批代理执行。

## 两个原对象分别提供什么

`Session.shared.init` 只装 `()`，提供初始化流程的排斥权。它不是保存 `current` 的锁；后者由另一把 RwLock 保护。原 `_init` 从取得到元数据等待、`current.write()`、默认值处理及返回/真实 Drop 的作用域都必须对应起来，不能由空载荷保持定理推出令牌和默认值保持。

`command::Shared.input` 保护 `Option<CommandInput>` 内联值和句柄。`input.take()`、替换旧值及端口关闭可能有另外的 IO/Drop 效果，不能只记成守卫释放。`finish` 在锁 Busy 时仍会发布完成，所以完成状态不证明 input 已取出或 FD 已关闭；`write_channel` 被取消也可能已经写出前缀。

相同逻辑 ticket 只表示同一次领取，不能复制原 Rust 守卫。公开值构造器本身不是现实取得/释放的证据；调用者通过实际 A6/B2 绑定提供 `Safe` 和合法 `Step/History`。`Busy` 不承诺一定有活跃使用者，也不提供无条件取得、FIFO 或无死锁保证。本批不需要扩大到 owned/map/get_mut/into_inner。

## 当前证据

ning 的隔离原生试验有七项固定 Tokio/真实初始化用例、三项已有业务回归和严格 Clippy；提前释放原 `_init` 的真实 Rust 副本成功编译后，被同一初始化持锁断言拒绝。执行源码冻结于 `a6081499`：ning 最终复现入口通过十项正确用例、严格 Clippy 和真实错误版同一断言，运行25.380秒；a6000 最终形式入口通过21项审计及4个实际文件的完整诊断，运行14.715秒，增量复用原构建。审计含辅助、合法见证和模型反例，不是21条业务性质。两者尚未接入共享整套验收。初次启动、私有视图准备和候选证明步骤的失败均保留，没有将环境失败算成语义反例。

## 独立复现与共享验收

先冻结本分支。在 ning 使用现有 Rust 环境、独立 `CARGO_TARGET_DIR`，由外层持 `native-heavy.lock`，运行 `SANDPROOF_ACCEPTANCE_ROLE=native python3 verification/experiments/tokio-mutex/native.py --output <全新私有目录>`；只在副本追加测试并修改错误版原 `_init` 释放点。脚本同时核对业务库与二进制的新鲜构建和来源，最后清理本包错误版缓存，保留依赖及原始证据。六项输入拒绝检查由 `python3 verification/experiments/tokio-mutex/test_checks.py` 执行。

在 a6000 的既有固定环境和原证明重锁下，运行 `SANDPROOF_ACCEPTANCE_ROLE=formal python3 verification/experiments/tokio-mutex/proof.py --output <全新数据盘目录>`。该入口是模型组件验收，构建原模块、21精确审计和4个实际文件；它不运行普通 Rust 测试。完整共享批次还须由统筹登记原生 case/形式任务及输入闭包，先校验 ning 完整凭据；当前尚未接入，不把独立命令成功称整套验收。

实际 driver 用 `process.take_input(shared.identity).ok()` 安装 Option；结果可能是 None。Started 不单独证明存在可写句柄。所有七组来源、原对象及调用者义务见本目录 `environment.json`；它是有限登记，不是全调用图或自有业务函数整体正确的假设。
