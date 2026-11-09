# 原始来源查询：限定支持范围与复现

本包对应 Rust 提交 `10037e47c3f78f258eba4683c5b8db2eeab7e7e6`，源码树
`e1da4ed15500da0b4d6e37aa1deba84b233f1d4c`。已通过的是实际 Rust
`original_source_using` 经原 Charon/Aeneas 抽取后的**成功返回合同**。
本目录仍是交付候选；公共入口、共享测试登记和统一批次验收尚未完成。

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
| `accept_original_source_row`、`original_source_using` 和业务判断 | 原工具透明抽取；Types/Funs 与原生成物字节一致 | 使用相同源码、工具、cfg 和绑定；改动后重新判断适用性 |
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

## 文件布局与装配

- 本目录 15 个根模块 `.lean.in` 和 `bindings/` 的 4 个模板，分别复制为
  `dependency-inventory.json` 中 `materialized` 指定的原路径。保持 import、命名空间和文件
  内容不变。Types/Funs 是冻结的实际生成物，不能人工修改函数体、绑定器或前缀。
- `dependency-inventory.json` 位于交付包顶层，列出 10 个共享提供方、29 个 Rust 来源和工具/
  依赖指纹。共享定义只引用，不在本包再造或覆盖。虚拟来源文件的 SHA 来自干净的原提交
  检出，不谎称 LLBC 保存了这些文件的正文。
  LoadBlob 两项已经定位到相同字节的公共 `.lean.in`；SQLite Session 的 Data/ReadOps
  已定位到 A 提交 `14493ee37094da6b1220d6d22d23c7310f3edb0c` 的
  `verification/experiments/create-transaction-link/SqliteSessionData.lean.in` 和
  `SqliteReadOps.lean.in`，与原物理字节一致，待统一装配。不能因此声称旧 `10037` 根已有
  这些公共模板，也不能把原物理生成目录当作受版本管理的源码位置。
- 原证明使用的私有 Lake 配置随原件保存。公共 Lake/root/coverage/冻结登记由统一集成者
  维护；本包不自行改动这些文件。公共配置变化时由集成者决定需要补的受影响验收，不能
  把原私有配置的成功改称新公共配置已通过。
- `original-capture/` 和 `original-native/` 保存原执行脚本/输入，供审阅和复现。
  其中有原机器路径、前置提交、锁、资源预算和提供方快照要求，不是可在任意工作目录
  直接运行的公共驱动。未经调度不能直接执行，也不能删掉其前置检查来运行。

## 原件回读的登记候选

交付包顶层提供 `fixed-originals.merge-fragment.json`、
`original-readback-compatibility.candidate.patch`、`formal-plan.candidate.json`。
这些是供统一集成者审查的候选，**未应用、未执行，不是验收通过记录**。

1. 合并固定原件选择中的 additive 字段，处理当前全局指纹与文件冲突；不得覆盖其它包。
2. 兼容补丁只允许本 job、`actual` 分支、精确原结果 SHA 和完成标记的组合读取
   `actual_files`。仅当这份精确历史记录缺少 `property_rejected` 字段时，把局部变量
   `rejection` 归一化为 `False`，随后仍与原式 `case not in ('correct', 'actual')` 严格比较。
   不添加跳过比较的分支；其它 job 仍使用原字段值，原内核/服务检查逻辑不变。
   这是历史字段格式归一化，不表示做过新的错误版本验收。不改写原 JSON，也不引入
   宽泛的“缺字段就算成功”。
3. 两个 evidence-bundle 压缩包一起使用：final01 是原 61 成员包的原始字节；补充包只包含
   原日志和后来只读计算的原项目源码指纹索引，不包含新证明结果。662 项完整原项目
   指纹与原 3 条诊断 revision 一致。新索引不是新证明或当前源码已验证的凭证。
4. 用已有 `original_readback.py prepare --bundle … --output …` 准备原件；设置
   `SANDPROOF_V3_ORIGINAL_EVIDENCE` 后，登记的命令是
   `original_readback.py run --job original-source-rows --output …`。这是未来集成步骤，
   本次打包没有执行它。它只能声明复用原结果，不能创建新的证明执行时间或机器身份。
5. `native-plan.json` 中的 9 项精确过滤器和原生产者供共享 native 登记者使用。
   **尚缺共享 native 登记结果**；不能把旧私有结果伪装成现有公共签章。由集成者沿用既有
   receipt 机制完成准入，再把 native label 接到 formal job。统一候选稳定后，首次公共
   native 准入由 A 使用既有入口运行一次。这里没有新增签章生成器、解释器或自建检查器，
   本窗口也没有重跑原 9 项和基础 26 文件。

原 capture26 和 native9 两份压缩包另外随包保留。它们用于审阅抽取和原生测试的来源，
不参与上述两个 formal 回读包的日志选择，也不是新执行的结果。

依赖与工具未变时复用原证据；任一消费输入变化时，明确审查其适用范围或重新执行受影响
检查。不得仅凭新提交其它测试变绿，把这份限定成功扩大到未连接的父入口。
