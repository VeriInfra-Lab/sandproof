# Redis 调用与结果的隔离候选

[验证入口](../../README.zh-CN.md) · [依赖保证](../../../docs/formal/dependencies.md) · [证明工作流](../../../docs/proof-workflow.md) · [任务 #564](https://github.com/VeriInfra-Lab/sandproof/issues/564)

这里保存真实 Rust 的候选拆分及其来源，不修改当前生产文件。原公开 `commit/reaffirm` 在候选里转调同一份泛型业务函数；原自有 codec 保持原实现，通过显式回调传入，时钟也显式传入。结果解释、身份检查、修订推进、健康检查和写围栏决定仍有真实 Rust 函数体，不能把这些决定当作外部库的保证。

[candidate.json](candidate.json) 固定原基线、候选文件摘要、工具和调用根；[candidate.patch](candidate.patch) 只应用到新私有副本。抽取包装通过路径包含该副本的真实文件，不复制业务算法。

| 调用根 | 覆盖的实际候选代码 |
|---|---|
| `finish` | 已解码输入的完整参数、结果解释、后置健康检查及未知围栏 |
| `commit` | 围栏拒绝、时钟和编码调用顺序、替换检查及结果核心 |
| `reaffirm` | 原载荷克隆、时钟、原身份与新修订、替换检查及结果核心 |

后两项抽取的是公开方法实际调用的泛型业务体。具体 `JsonCodec` 的正确性与原版本的一般等价，不由抽取成功自动得到，也不因此成为可信库前提。`src/bindings.rs` 另从真实公开方法生成完整 Charon AST；`check_storage_binding.py` 检查四个薄转调中的原输入、完整结果、关联格式常量、具体 codec/clock 顺序和异常清理。这个机械准入不是具体 codec 的 Lean 证明。

## 新诊断与日志边界

候选基于 `f60e81f81629b23a512da52e6511b5f46699535f`，保留 #604 的诊断，生产文件未改。查询错误先清空原连接再记录诊断；健康配置/持久化不匹配先调用日志，再按原首次拒绝返回。日志输出只经零资源 `storage_io::Diagnostics` 端点转交原 `diagnostics::emit`；它不持有账本、连接、载荷、租约或用户回调。该窄输出合同已经 owner 采纳，规范及限制只见[依赖保证](../../../docs/formal/dependencies.md#a11b4-redis-提交结果接口的具体范围)。

空端点的 Lean 表示保存可能的输出结果与调用历史；这不是给真实 Rust 新增日志队列或文件所有权。written、writeFailed 和 suppressed 都可正常返回，panic 与 nonReturning 不产生正常后态；调用正常返回不能证明已写日志，更不能证明完成业务或资源释放。模型没有总成功空桩，未给物理时间、日志隐私/限额或异常恢复提供新定理。三类健康拒绝的实际状态/元数据和配置仍透明抽取，日志内部锁/原子计数没有机器证明。

[来源检查器](../../../tools/check_redis_diagnostics_binding.py)在候选重建后、抽取前执行；新查询、日志调用和拒绝顺序不符即拒绝。12 项回归覆盖漏日志、改参数、引入业务对象、改 phase、反转清连接顺序及改围栏；这些是准入检查，真实提前解除围栏错误版仍需独立编译/原工具抽取并由同一性质拒绝。

## 重建与编译

在批准的 Linux x86_64 验证环境中，从包含冻结基线的仓库重建；目标目录必须不存在：

```sh
bash scripts/python-entry.sh verification/experiments/redis-result/prepare.py \
  --source-base /path/to/sandproof \
  --destination /path/to/new-private-directory
```

准备程序核对原文件、补丁、模板文件清单及应用后的全部候选摘要。它只准备源码，不运行抽取或证明；额外模板文件也会被拒绝。

已有生成模块使用附加视图编译，不改变原 Lake 配置或依赖版本：

```sh
lake -f lake-storage.toml --no-cache build \
  SandproofStorage.Audit
```

这条命令只检查所列模块，不代替重新抽取一致性、错误版检查或项目完整验收。正式范围及验收状态以任务和覆盖记录为准。

## 原工具重抽取与公开调用连接

```sh
bash scripts/python-entry.sh verification/experiments/redis-result/replay.py \
  --source-base /path/to/sandproof \
  --destination /path/to/another-new-private-directory \
  --tool-root /path/to/frozen-tools
```

入口复用既有工具版本、二进制和源码准入，在新私有副本准备同一候选；然后执行 Rust 编译、根文件格式、三条业务根的 Charon/Aeneas 翻译和两条公开方法的 Charon 翻译。生成的 `Types.lean`、`Funs.lean`、`translation.json` 必须与登记文件逐字相同。公开 LLBC 的完整摘要只规范化 Charon 的无序短名称字典，保留其余字段和有序数组；不删除 AST 字段来获得一致。

公开连接还独立检查参数及回调来源，并拒绝换参数、换返回位置、额外调用、控制分支、调换回调、改写格式常量、异常路径返回等七类 AST 错误。它们检查准入规则本身；真实 Rust 提前解除围栏错误版仍需单独编译、抽取并被原同一性质拒绝，不能由这些 AST 检查代替。

`evidence/replay.json` 保存阶段、命令、退出码、完整 LLBC 和生成文件摘要以及源码前后一致性。此入口不执行原组件测试、Lean 定理或两个完整验收入口；任何阶段失败都保留私有副本，不覆盖失败证据或重用旧生成物。

## 完整候选验收

```sh
bash scripts/verify-storage.sh
```

入口要求原固定真实 Redis 二进制及工具环境，并保存新的私有证据目录。它核对 `storage-inputs.sha256`，重新抽取，逐符号检查外部模型和原生产源，校验两版标准库/固定 Redis 库源与全部析构位置，执行候选的严格 Clippy/真实 Redis 组件检查，再编译并审计完整定理集。

随后 `negative.py` 另建私有副本，只把真实 Rust `fence_unknown` 的 `write_fenced = true` 改为 false。错误版先编译、用原 Charon/Aeneas 抽取，其生成文件保留原样；相对正确生成物只允许目标赋值和工具生成的模块 import 差异。原 Results 文件只改模块 import，就必须在 `fence_unknown_exact` 和 `eval_error_returns_unknown` 失败；编译失败、缺模块或其它诊断不算拒绝成功。三个独立反例/允许输入定理还须通过内核和标准依赖审计。

最后用原真实 Lean 服务的 lean_diagnostic_messages 直接检查正式 Audit 文件；不设严重性、行号或声明筛选，不把正式验收交给虚拟 scratch 文件。核对完整、未超时、非陈旧、无错误/警告/未解目标、全部定理回执，以及 Audit 和全部证明源码的指纹；缺失/重复审计、陈旧、筛选、部分结果或服务错误等 18 类回执变体必须被拒绝。`verify-controller.sh` 保留原有阶段并增加本入口。该入口通过也只支持此冻结候选；原 Linux 基础检查与全项目证明验收仍须单独完成，不能据此关闭整个 V0。

原 LLBC 保留的析构没有因为 Lean 生成物未显示就被忽略：原函数/位置/类型/异常路径完整摘要纳入准入。普通值域、原连接关闭与异常退出分别按[依赖明细](../../../docs/formal/dependencies.md#a11b4-redis-提交结果接口的具体范围)解释；不把资源析构当纯值，也不从逻辑连接消失推导物理关闭或回滚。

## 如何理解接口与定理

[Results](../../aeneas/storage/SandproofStorage/Results.lean) 直接使用未修改的生成函数：未知结果必须保持写围栏并弃用连接；已围栏的普通提交保持账本和回调状态；成功结果需要完整 CAS 调用及后置检查证据，并保留修订、状态和完整载荷。四个已知拒绝码的定理只解释返回值及本地状态，不从 `Err` 推出 Redis 没有写入或已经回滚。[Key](../../aeneas/storage/SandproofStorage/Key.lean) 复用既有字符串往返证明，保持完整部署字符串。

[Health](../../aeneas/storage/SandproofStorage/Health.lean) 证明健康检查只回写连接和逻辑日志历史、保持所有原账本业务字段及围栏，正常错误只可能是 Configuration/Unavailable；这不是 INFO 解析和物理耐久条件的完整证明。[Caller](../../aeneas/storage/SandproofStorage/Caller.lean) 把未知围栏性质接到真实泛型提交/重确认调用链，显式要求自有 codec 的正常错误范围；[Success](../../aeneas/storage/SandproofStorage/Success.lean) 保留匹配部署、完整旧载荷、新修订及实际编码/解码证据。`commit_success_state` 还要求同一 codec 状态链上的往返性质。两个 codec 条件都归自有 V2，不是外部库或机械适配的保证；尚未实例化就不能声称具体生产类型已闭合。

[Examples](../../aeneas/storage/SandproofStorage/Examples.lean) 在原生成循环与提交调用链上提供具体正常见证：固定合法版本、全部六项配置和四项持久化字段，加上 CAS 成功回复，可通过完整泛型 commit。示例 codec 有容量不足和非法字节拒绝，并满足自己的错误范围/往返条件；另有已围栏拒绝及送达未知的执行见证。它说明接口条件可以实现，不代替实际生产 JsonCodec、租约或部署前提的实例证明，也不以有限样例代替通用围栏定理。

外部值模型保留正常回复、调用前拒绝、送达未知、panic 和不返回。`LibraryRequest` 记录传给适配的参数，不代表数据已经送达、执行或耐久化。定理中的正常返回不保证调用一定返回，也不为 panic 后的状态提供围栏承诺。业务侧仍须连接异常处理、禁用和恢复义务。

支持域是有限且可表示的字符串/字节、正常分配、固定纯 `String/str` 字典及普通值载荷；不覆盖有可观察自定义 `Drop`、共享可变资源或任意自有比较器的泛型实例。连接、文件和时刻在本接口未观察的内容用身份或值表示，不能据此推导物理关闭、释放或总墙钟期限。

Lua、自有 codec、读取/reconnect、具体调用方和崩溃恢复仍有各自义务。此目录及其局部定理不表示生产存储、整个 V0 或部署系统已经验证完成。
