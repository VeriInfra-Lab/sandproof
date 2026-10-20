# SQLite 提交与缓存发布隔离候选

[证明入口](../../README.zh-CN.md) · [V0](../../../docs/formal/00-foundations.md) · [依赖合同](../../../docs/formal/dependencies.md) · [任务 #570](https://github.com/VeriInfra-Lab/sandproof/issues/570)

本目录固定代表性事务控制接口的隔离输入；完成状态与验收证据见任务和证明覆盖，不把试验构建当作整套完成。生产源码、Charon/Aeneas 和工具版本均不由本目录修改。

## 实际代码与边界

证明批次依赖已交付的 [PR #556](https://github.com/VeriInfra-Lab/sandproof/pull/556) 固定头 398b114d，以复用最新后台调用绑定核对和完整入口冻结预检；最终目标 integration。下列 Rust 候选源码仍固定原 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`。补丁仅把 `controller-service/src/controller/transaction.rs` 的提交/rollback 决定移入同目录下 `transaction/memo.rs`，原 WorldTransaction 实际调用它。自己的清空、错误分类和发布决定保留真实函数体；不是另外写一份验证专用算法。

`Cells` 只有两项机械操作：把给定值写入原 committed 单元，以及从原 candidate 单元取出原值。RefCell 薄层不决定该写 None 还是候选，不跨 SQL 回调持有借用。原工具直接提取共享 RefCell 时，没有提供可用于缓存后态证明的状态输出；这个候选让核心显式接收并返回更新状态，不扩展翻译器或添加解释器。

实际 committed 与 candidate 是两个不同的原 RefCell；正常入口没有仍存活的冲突借用。回调的修改范围只包含其原数据库事务，不得同时改写这两个缓存单元。生产回调固定为 Transaction::commit/rollback；测试注入的丢回执闭包也须满足这一范围。带任意共享状态捕获的自有回调不能直接套用本模型。候选较早结束 take_candidate 的临时借用，这只在本同步段、不同单元及无可观察自定义析构的 World 值域内采用，不据有限对照声称一般等价。

生成核心的性质使用实际 Cells 接口及 commit/rollback 回调；通用方程允许不同的状态类型，具体双单元模型给出可计算实例。真实 RefCell 的对象、参数、读写效果和借用符合性仍须按 B2 登记；事务调用的参数/结果传递按 B4，SQLite 按 A4。不能把整个 WorldTransaction、ControllerJournal 或恢复业务放入可信边界。

正常结果保留 CommitUncertain 的原错误身份；错误载荷模型不解释错误文本或 SQL。提交错误允许数据库已经提交，不表示没有效果。rollback 错误只使缓存失效，不提供已完成回滚、连接已关闭或外部资源已释放的证据。普通值域、调用前事务仍在及正常借用条件由使用者建立；panic/unwind、abort/崩溃、终止性和可观察自定义析构不从正常返回方程推出。

核心返回的候选字段只描述该同步调用的边界；外层 WorldTransaction 随后可正常丢弃候选内存。只有原外层 committed 单元继续作为缓存观察对象，不从临时候选字段推出持久保存或资源仍被持有。

具体 SQL、完整投影/寿命/outbox、候选正确性、恢复及允许派发的业务条件仍由 V5 提供。候选与原版的有限回归对照不等于任意执行的一般等价证明。本目录不关闭整个 T7/V0，T9 排除。

## 物化与原工具预检

全部构建、测试和证明在指定 Linux x86_64 验证环境进行。先按候选清单从 Git 对象核对原版本，创建新的自包含私有 checkout，再核对补丁与候选摘要；拒绝已存在的目标目录，不覆盖开发者源码：

```sh
bash scripts/python-entry.sh verification/experiments/sqlite-commit/prepare.py \
  --source-base "$PWD" --destination /absolute/private/candidate
```

此命令只准备源码，不能计为构建或证明通过。使用冻结工具根中的原 Charon，从真实 crate 选择清单中的两个自由函数根；使用 `--preset=aeneas --error-on-warnings --sysroot default`，外部 `rusqlite` 的 opaque 范围必须在实际调用登记中审查。不要把自有 core 或 Cells 决定设为 opaque。Cargo 保留 `--manifest-path controller-service/Cargo.toml --lib --locked --offline` 和原 durable 特性。

将实际新生成的 LLBC 交给同一固定 Aeneas Lean 后端。生成 Types/Funs 不手改；逐次来源、完整 LLBC、外部/析构清单和产物一致性是正式交付前必须接通的准入。独立 `lake-sqlite.toml` 只增加本库视图，原主库配置不变。完整原证明/基础入口、实际服务、真实错误版及其内核反例须分别报告，不能以本步骤取代。

## 一次完整的接口复验

在原工具/后端及 Lean 服务就绪的同一受控 Linux 环境执行：

```sh
bash scripts/verify-sqlite.sh
bash scripts/verify-controller.sh
bash scripts/check-function.sh
```

接口脚本每次创建新的私有目录，分别物化抽取、原生检查和错误版源码，保留全部失败证据。replay.py 的两个完整中间表示只规范化原检查器允许的无序 short_names 表；其余字段、源码位置、调用和异常/析构路径均保留。生成 Types/Funs/translation.json 逐字比较，不把旧文件复制成新生成物。环境清单逐项列出前端声明、外部/内建接口、正常和异常析构位置、两版标准库及固定 SQLite 源。它是源码准入，不是新的解释器，也不是析构已证。

native.py 在没有抽取根的固定候选执行格式、严格 Clippy 和完整原缓存测试；原版对照在全项目入口保留。negative.py 只从真实 memo.rs 删除第一次清空缓存，保留 rollback 分支，先由原数据库断言拒绝、再经原工具生成错误版；它将正式 Results.lean 原样复制，要求相同性质失败，并用独立内核证明可表示的旧缓存反例。错误版生成文件只在私有工程编译，不覆盖正确库。失败证明的诊断恢复不作为正式定理或占位导入。

正式 Audit 复用原依赖检查命令，逐项拒绝非标准假设。通过指定的同一真实 lean-lsp-mcp 客户端检查物理文件，要求完整、非陈旧、无错误/警告及全部定理审计消息；过滤消息、漏项或旧源指纹不能计为通过。对源码准入和服务收据的变体检查只测试门槛，不替代业务正确/错误版。

Examples.lean 还提供固定合法实例：提交未知允许写前/写后两种物理结局，核心都清空缓存；正常成功才发布原候选。这是采用 A4 允许行为的可实现接口见证，不证明引擎、文件系统、连接配置或具体 SQL。World 和回调捕获的自定义析构/共享资源不能任意实例化，原事务内部结束仍按 A4 登记。
