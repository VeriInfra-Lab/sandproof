# 第一阶段协议的实际同步入口

[文档目录](../../../docs/README.md) · [V3](../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../docs/proof-workflow.md)

本 crate 只调用生产 `sandproof_protocol::lifecycle` 方法，不复制校验算法。
固定原 Charon/Aeneas 分别抽取请求、manifest 和值载体；生成 Lean 不手改。
这是 [#642](https://github.com/VeriInfra-Lab/sandproof/pull/642) 的有限组件，
不是正式完整 V0 验收，也未安装 v3 HTTP 路由。

## 性质的准确范围

| 文件 | 检查内容 | 限制 |
|---|---|---|
| Model | 独立的值保持、Resume 顺序、重放 pair、capture 绑定 pair 合同 | 不导入被检查的生成函数 |
| Carriers | CaptureId 构造/get、零拒绝、非空接受、最大值耗尽；32 字节摘要完整保值 | 不推广成其它三种角色已证明；不计算 SHA-256 |
| RequestValues/Request | 任意错误版本首拒绝；其余字段固定合法时任意 source/target 的严格顺序；capture 1/2 的全文重放 pair | 不是任意两请求的全文等价或完整请求守卫充要条件 |
| LoopWitness/ManifestWitnesses | 原生成循环、checked 算术和路径/profile/三个文件的正常见证 | 一盘、固定可表示路径 r、state/memory/disk 各长度 1、零摘要；不是实际捕获材料 |
| Manifest | 任意错误格式版本首拒绝；改变原 capture 操作 sequence 的首拒绝；上述完整清单接受且总长 3 | 不是任意文件集、所有非法 profile 或物理恢复的证明 |

正确视图准确登记 38 个审计声明，其中有循环/字符串/算术辅助引理；数量不能当作业务覆盖率。
当前源码和原始记录的实测结果以覆盖记录、PR 及组件 result.json 为准，不能只凭本表认领通过。

## 同一性质检查真实错误版

`negative_native.py` 在私有 Git 归档中只改变一处生产表达式：

- same-incarnation：把 target <= source 改成 target < source，错误接受同一运行代。
- key-only-replay：只比较 operation key，错误接受 capture 已变化的原请求。
- skip-capture-binding：使 manifest 原 capture 绑定检查失效，错误接受另一 capture 操作。

ning 必须先实际编译生产库和原测试二进制（fresh=false、原源路径），再由原测试同一断言拒绝；
编译失败不算反例。源副本、程序及日志留在 ning。
msra 从另一份同内容源归档执行相同唯一修改，由原工具重新生成错误版，不复制 Lean 错误算法。
三个 `.lean.in` 是隔离候选；正式重放将其原样放入私有 artifacts，检查错误接受见证以及对正确版
同一 Model 合同的否定。Model 文件逐字节一致。每个错误版有两条反例声明；
manifest 的独立视图还须检查原 23 条辅助声明，不能省略不被公共根导入的文件。

第三种错误的真实生成体把文件循环拆为 loop0/loop1。绑定不匹配分支实际使用 loop0；
formal.py 仅在该错误版私有 ManifestWitnesses 中替换六处 helper 引用，并核对真实声明和原/新证明摘要。
错误 Rust、生成文件和独立 Model 不变，全部 23 个辅助与 2 个反例目标仍须通过实际文件检查。

## 外部条件和真实调用者责任

request 的两个外部函数复用已登记完整 String 相等和 Vec::is_empty；
manifest 的七个函数还包括 str 长度、空值、deref、to_owned 及 as_bytes。
`FunsExternal` 中 **str::as_bytes = .ok value** 是新增的窄 A6 表示连接：
仅对有效、长度可表示的 Rust str，借用期间返回其完整、原序、只读 UTF-8 字节；
不截断、不规范化、不复制资源身份。它仍依赖固定标准库与模型符合，不是已证明的标准库实现。
其它函数转发原固定 A6 模型。候选模板中的 axiom 仅为原工具输出证据，不导入正式证明。

普通值与正常分配范围沿用原 Aeneas/A6；不覆盖 OOM/abort、资源载荷 Drop、unwind 或异步取消。
生产 `lifecycle_v3` 的原 serde_json/有界 Writer 调用和自有预算/调用顺序已有普通测试，
本组件尚未抽取或证明这些完整函数，不能把整段自有 codec 当 B4。
原 capture、认证 owner、账本版本、受保护 profile、原文件/哈希/同步和 claim/pin/激活
分别由 V1–V9 真实提供；这里不从字段一致推出捕获成功、兼容、安全路径或恢复资格。

## 可复现组件

普通测试/格式/Clippy 仅在 ning，形式化仅在 msra；两机重任务锁不同时持有。
两个入口不写全局 proof-native seal，不冒充已有共享完整入口。

ning 在批准的固定环境与私有副本中：

```sh
python3 verification/experiments/stage1-protocol/native.py \
  --output /private/native --target /private/native-target
python3 verification/experiments/stage1-protocol/negative_native.py \
  --source-archive /private/native-source.tar.gz \
  --output /private/wrong-native --target /private/wrong-target
```

msra 使用固定工具和独立 .lake/build，CARGO_TARGET_DIR 指向原形式 target；
不传 ning 的 Cargo target、不升级依赖、不清理 .lake：

```sh
python3 verification/experiments/stage1-protocol/formal.py \
  --output /private/formal \
  --source-archive /private/current-source.tar.gz \
  --native-mutants /private/wrong-native/result.json \
  --native-source-archive /private/native-source.tar.gz
```

frozen.json 固定本模块源码、真实导入闭包、生成物和检查器，不循环收录自身。
完整 LLBC 只沿原检查器排序无序 short_names，保留其它字段、源码、选项、清理路径。
Charon 从固定 crate 工作目录写固定相对 raw 位置；旧 raw 先归档，确认新文件再交 Aeneas。
正确输出与冻结原始 IR/Types/Funs/translation 逐项比较；错误版必须重新抽取。
原生准入核对原归档、生产/测试输入、唯一改动、构建与断言日志，不能只信 success。
全部实际文件经过原 Lean 内核、原 lean_session 批量服务的完整非陈旧诊断和准确审计。

三个生成环境存在全局 discriminant 辅助实例重名，按既有 Status/Writer 模式独立构建检查。
公共消费者只导入独立 Model；不能同时导入所有实现视图，也不能因此省去某个正确/错误目标。
组件通过后仍须共享模块登记、同一候选完整验收及正式集成；不据此关闭 T1/T2/V0 或启用路由。

## 任意请求全文重放的局部证明

`ReplayContract` 独立规定：任意两个请求的比较总返回成功，布尔值精确等于完整值是否相等。
`ReplayEquality` 对真实请求生成体证明这个合同及接受/拒绝充要条件，覆盖四种操作的全部字段，
包括 options 的长度和完整字节、两个摘要以及所有身份/版本字段。范围和实际验收状态只在
[覆盖记录](../../../docs/proof-coverage.md)维护；它不表示请求合法、已认证或持久重放闭合。

在批准的证明机、固定工具和已准备好的 Lake 依赖环境运行：

```sh
python3 verification/experiments/stage1-protocol/replay_formal.py \
  --output artifacts/full-request-replay
```

入口对当前 Git 工作树的跟踪文件及非忽略新文件做摘要快照，使用独立源码和 `.lake/build`。
固定原工具从真实 Rust 重新生成两版；key-only 修改复用 `negative_native.py` 的原单点变体。
两版保留逐字节相同的独立合同，分别检查完整正例和通用合同的否定，并经既有严格实际文件服务
核对准确审计清单。原 LLBC、全部原工具输出、固定工具/配置/源摘要与服务日志保存在输出目录。
生成体只安装进该次隔离目录，不手改，也不改变历史冻结记录。

这是便于推进这一条性质的局部形式入口，输出明确保留 `native_checks=not-run` 和
`full_acceptance=false`。它不接收或伪造 ning 原生回执，也不替代前述 `formal.py`、
完整普通检查、共享登记或 `scripts/verify-controller.sh`。在当前候选完成这些门槛前，
不能把局部成功报告为整套证明验收成功。

当前请求迁移包含 Renew：其运行代、I64 期限和 `extend_only` 进入完整值相等。
仓库请求生成根由固定原工具完整更新，历史 `frozen.json` 仍只认领旧版；不更新摘要使旧组件假绿。
新增布尔模板仅委托已有 A6 网络库模型；key-only 变体不再到达该模板。
实际文件清单同时检查这份模型及其类型闭包；其它未登记外部函数仍拒绝。

## 完整请求守卫与首错的局部证明

`ValidationModel` 只导入生成类型和既有 Foundations 数学模型；`ValidRequest` 规定纯守卫的接受域，
`FirstError` 要求所有更早守卫通过，`FullValidationContract` 同时要求成功和每种错误的充要条件。
`Validation` 连接实际生成函数，`ValidationExamples` 给出四种操作的正常接受输入。
`Renew` 补第五种操作、三个字段的重放条件、Absent 拒绝和负期限的守卫边界见证。
validation 模式同时回归原 `Request` 的五项定理；replay 模式检查实际 key-only 错误版
接受被改动的 Renew 模式，并否定正确版同一个独立合同。
它不验证 serde、当前账本、认证 owner、模板授权或持久收件；具体边界见覆盖记录。

同一局部入口新增可选模式，默认 replay 保持原检查：

```sh
python3 verification/experiments/stage1-protocol/replay_formal.py \
  --property validation --output artifacts/full-request-validation
```

该模式重抽取正确版和原 `same-incarnation` Rust 变体，逐字节核对两版独立合同，
检查精确依赖/审计清单和所有实际文件。它继续明确输出原生检查未运行、完整验收未完成，
不替代 ning 检查、历史组件入口或共享证明批次。

## 清单长度与溢出的局部证明

`ManifestLengthModel` 仅导入生成类型，用自然数总和及逐文件非零定义独立的成功结果合同。
`ManifestLength` 对任意可表示清单连接真实 `SnapshotManifest::validate`：成功返回值精确等于
全部长度之和，且每个文件非零。因此含零长度或自然数总和超过 U64 上界的清单不能被接受；
这些安全结论不要求预先假定 profile、路径或文件形状合法。

循环另证明已有错误原样保留、当前文件拒绝先于加法，以及当前文件检查成功但累加越界时
返回 `LengthOverflow`，不受后续文件影响。这些一步结论显式带当前真实文件检查的结果，
不代替任意 profile/路径/磁盘集合的完整接受域证明。四个实际入口见证覆盖最大合法总长、
溢出、当前零长度先于加法及先前溢出先于后续零长度；前置绑定和一盘 profile 确实通过。

```sh
python3 verification/experiments/stage1-protocol/replay_formal.py \
  --property manifest-length --output artifacts/manifest-length-formal
```

同一入口从正确与私有错误 Rust 重新抽取；`wrapping-manifest-length` 只把原 checked 加法
替换为 `Ok(current.wrapping_add(length))`。错误版实际接受长度 `[U64::MAX, 1, 1]` 并返回 1，
两个反例声明由此否定正确版逐字节相同的独立合同。`ManifestValues` 只是从原
`ManifestWitnesses` 分离未改动的值/profile/单文件辅助证明，供两个视图共同检查；原 23 项
辅助和 4 项 manifest 结论仍在正确视图内，没有删除旧检查。

正确视图还比较本次生成的 Types/Funs/translation 与仓库版本，并按原规则核对完整 LLBC；
两版全部实际文件经内核、完整非陈旧服务诊断及准确审计清单检查。长度模式的物理清单包含
实际导入的 Foundations 值/字符串证明闭包。原生入口登记原 `lengths_are_positive_and_sum_never_wraps`
断言及新增边界/首错回归；是否实跑以原生回执为准。局部结果继续明确
`native_checks=not-run`、`full_acceptance=false`，历史冻结和完整验收门槛不变。

## 清单文件集合的局部证明

`ManifestFilesModel` 仅导入类型，独立定义文件数量、state/memory 的唯一位置，以及每个
profile drive 对应的完整 ID 和十进制 `disk-N.img` 路径。`ManifestFiles` 证明真实入口成功
必满足该合同和 1–16 盘边界，并组合前批长度结论；不要求输入预先合法。它没有证明完整
接受域、任意 profile 的 ID 唯一性或 jail 路径合法域，物理文件和 profile 来源也不在本片内。

```sh
python3 verification/experiments/stage1-protocol/replay_formal.py \
  --property manifest-files --output artifacts/manifest-files-formal
```

正确视图保留长度模式的全部声明，并增加文件集合合同、正常/少文件/多文件/错误 ID/错误
路径见证。`ManifestFilesValues` 给两版共用真实有效绑定和 profile 的执行前缀。
`skip-disk-id` 在私有 Rust 中只将磁盘 ID 不等的拒绝条件变为 `条件 && false`；从错误版
重新抽取后，证明实际接受 `foreign` 磁盘且总长为 3，由此否定同一独立合同。
两版都检查完整实际文件、内核、非陈旧服务诊断和准确审计清单，正确生成物/完整 IR 的比较
及输入摘要检查沿用同一入口。原生错误版使用原 `file_names_kinds_and_all_profile_drives_are_checked`
断言，另有 16 盘全部位置及多文件回归；是否实跑以原生回执为准。
局部入口仍输出 `native_checks=not-run`、`full_acceptance=false`，实际状态只在
[覆盖记录](../../../docs/proof-coverage.md)及精确候选 PR 维护。
