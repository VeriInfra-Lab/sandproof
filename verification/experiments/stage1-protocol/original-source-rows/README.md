# 原始来源查询：限定支持范围与复现

初次独立验证使用 Rust 提交 `10037e47c3f78f258eba4683c5b8db2eeab7e7e6`。
当前入口在固定组合上重新抽取 `original_source_using` 和实际行处理器。
本范围验证正常成功返回时的完整来源条件。

## 已证明什么

主要定理是
`SandproofOriginalSourceRowsRust.SourceObservedRows.actual_source_success_complete_contract`。
对于满足本包接口解释的输入，实际函数正常返回一个来源记录时，可以同时得到：

1. 沙箱记录处于 Running、非终态，并有当前 VM；VM **记录**的属主、沙箱、运行代和
   Executing 状态与该沙箱匹配。这些是数据库中的记录条件，不是操作系统进程存活证明。
2. 函数确实调用了原查询、原参数和原游标。查询是：

   ```sql
   SELECT action_json,receipt_json FROM outbox
            WHERE sandbox_id=?1 AND dispatch_state='receipted' ORDER BY id
   ```

   参数来自同一次 `identity::sandbox_id` 和 `as_str` 调用。查询产生的更新后 Statement
   与后续游标相连；不是任意指定一组可能存在的行。
3. prepared 和 activated 两个槽位各有同一游标实际经过的行作为证据。这些行通过原处理函数
   的动作解码、命令选择、效果与逻辑操作检查、计划及完整身份检查、回执解码与校验、
   ObservedSuccess、HostEvidence 校验、来源提取和 agree。
4. 两个槽位记录的来源与最终返回值一致；记录已有值时保持已有值。每次行处理前后的完整
   解码器状态 D、行状态和游标状态相连，不把有状态解码回调替换为无状态纯函数。
5. 最终来源满足实现实际检查的 pid 非零、boot_id 非空条件。不额外假设时钟精度、ticks
   为正、UUID 格式或实现没有检查的约束。

这是成功路径的条件性结论。它不声称调用必定结束、一定成功，也不把模型中的任意回调
假定为正确回调。正式主结论没有把“每一行已满足业务规则”留作额外前提：实际新增槽位
对应的完整行证据已经从抽取出的处理函数证明出来。

## 哪些是真实抽取，哪些仍有边界

| 部分 | 本包如何处理 | 使用方仍需保证什么 |
| --- | --- | --- |
| `accept_original_source_row`、`original_source_using` 和业务判断 | 原工具透明抽取，当前 Types/Funs 由实际代码重新生成 | 使用相同源码、工具、cfg 和绑定；改动后重新判断适用性 |
| `valid_identifier` | 链接已有单独证明的实现与定理 | 维持精确源码和提供方绑定，不把它记作未证明业务桩 |
| `accept_original_rows` 机械循环 | 原 Rust 的顺序驱动被设为不透明；Lean 中的完整驱动与循环性质已证明，原 Rust 对应关系由审查和原生测试支持 | 按原顺序取行；每行交给真实回调；完整传递 I/Rows/F 状态；遇首错停止；不额外调用回调或覆盖状态。这段 Rust 与模型的对应关系仍是可信适配条件 |
| SQLite/rusqlite | 保留原 SQL、绑定参数、prepare/query_map/游标调用及可失败结果；使用共享接口解释 | 同一事务/连接的结果确实来自该查询；原字节、顺序、游标和错误语义与接口解释一致。数据库实现及崩溃恢复未在本包证明 |
| 字符串、向量、Option/Result、转换和 JSON | 复用已有精确接口；保留错误和完整字节/状态；没有全局 JSON 自由函数替身 | 共享 Std 接口与真实库一致；具体 JsonOriginalRowCodec 把完整字节交给原 Serde 解码，不截断或漏传错误 |
| `HostEvidence` / `ResourceEvidence` | 证明实际调用了校验和来源提取，并使用其实际结果 | 实例对外部资源及进程观察的含义由提供方保证；成功布尔值本身不等于证明 OS 事实 |
| Charon/Aeneas、Rust 编译器、Lean 内核 | 沿用原工具和项目既有信任标准 | 不修改工具或借新解释器替代原抽取；不使用占位证明或关闭内核检查 |

具体 `original_source` 的 JsonOriginalRowCodec 包装是经过审查、原生用例覆盖的薄适配。
本包主定理入口仍是实际泛型 `original_source_using`，不能改写成“具体 HTTP 入口已经完整证明”。
正常值范围沿用原抽取记录：不承诺自定义 Drop 可观察效果、展开栈、OOM 等未覆盖语义。

观察来源不授予暂停或 Stop 权限。本包不证明进程此刻仍存活、不证明调度公平性、网络、
TLS、Tokio、SQLite/Serde 实现，也不覆盖原 Unload HTTP、完整 V3 或整个 V0。

## 原验收与复用

三组证据必须分别报告，不能把一组通过当作整套通过：

| 证据 | 实际结果 | 不代表什么 |
| --- | --- | --- |
| ning 原生验收 | 9 个精确用例、每项实际运行 1 个测试，全部通过；格式与严格 Clippy 通过，源码恢复且干净；内层约 21.80 秒 | 没有共享 native seal；没有整套功能或当前批次验收 |
| msra 原工具抽取 | 原 Charon→Aeneas 成功、范围检查通过、可逆名称元数据视图恢复；内层约 38.41 秒 | 抽取成功本身不等于 Lean 性质通过 |
| msra 最终证明 | 29 文件闭包中 3 RUN、26 REUSE；新增 3 文件的内核、原文件服务及 13 项标准依赖审计全部通过；内层约 93.59 秒 | 不是重新运行了 29 文件，也不是当前公共入口已通过 |

26 个复用文件来自原 base46 和 row12 已成功记录，依赖字节与最终闭包对应。
原件包保留它们的结果和服务记录。不能只取最终 3 个文件的结果而丢掉这条复用依据。
本包没有另行执行错误 Rust 变体；9 个原生用例中的拒绝路径不能改称形式化变体验收。

## 当前入口

`execution-inputs.json` 固定独立 Lake 视图、当前需要物化的 27 份输入、两份原外部接口模板和实际文件审计库存。当前 Types 与 Funs 由 `formal.py` 从实际 Rust 生成。它使用固定有序抽取配置及可逆名称标注，在私有源码副本恢复 `lib.rs`，并检查已证标识符提供方的原文件与服务记录。该提供方证据由 `SANDPROOF_SOURCE_ROWS_PROVIDER_ROOT` 指向原正式机挂载的只读目录。

`native-plan.json` 固定原 9 项精确断言。`native-admission.json` 固定当前源码和两个隔离测试片段的来源。它们由唯一 `stage1-original-source-rows` 组件进入普通验收。与事务部分共用以下入口。

- ning：`bash scripts/python-entry.sh tools/proof_native_suite.py run --scope stage1-v3-transaction-source --output <new-native-evidence>`
- MSRA：沿原正式机锁及工具环境，使用同一提交的当前 native seal 执行 `bash scripts/verify-v3-transaction-source.sh <new-formal-evidence>`。

两份历史原件保留自己的版本和审计范围。当前入口使用独立视图，不覆盖旧全局固定原件，也不把旧私有成功算作当前执行。
