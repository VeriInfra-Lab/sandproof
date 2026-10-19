# 证明覆盖与剩余义务

[文档目录](README.md) · [验证设计](formal-verification-design.md) · [本地状态机](state-machine.md) · [证明流程](proof-workflow.md)

## Redis 提交结果：隔离候选与完整验收（#564）

[冻结候选](../verification/experiments/redis-result/candidate.json)以生产基线 `824abe2b6285774ef8983fbe02db3a6509c9b5fb` 加固定补丁构建，生产文件未改。[生成核心](../verification/aeneas/storage/SandproofStorage/Funs.lean)来自固定原 Charon/Aeneas。公开 commit/reaffirm 的四个薄转调另经完整 Charon AST 核对参数、结果、关联常量、codec/clock 字典和 unwind 路径；该机械准入不等于具体 Serde 回调图已由 Lean 闭合。[依赖明细](formal/dependencies.md#a11b4-redis-提交结果接口的具体范围)区分外部 A6/A8/A11、获批 B4 与仍由调用者提供的自有条件。

正式库包含[结果与写围栏](../verification/aeneas/storage/SandproofStorage/Results.lean)、[健康检查回写范围](../verification/aeneas/storage/SandproofStorage/Health.lean)、[泛型调用链的未知围栏](../verification/aeneas/storage/SandproofStorage/Caller.lean)及[原完整载荷和新修订来源](../verification/aeneas/storage/SandproofStorage/Success.lean)，共 40 项核心与辅助声明；[具体见证](../verification/aeneas/storage/SandproofStorage/Examples.lean)另有 91 项见证及辅助声明。中央 Audit 逐项审计全部 131 项，只依赖标准逻辑。成功性质显式要求同一 codec 状态链的往返，正常错误范围由自有 codec 提供，未把这些业务义务移入外部边界。示例 codec 会拒绝超容量和非法字节，并证明自己的条件；完整成功见证经过实际生成的写前健康、修订推进、CAS、写后健康和 Committed 构造，不实例化生产 JsonCodec/schema、物理租约或部署条件。

两个不同私有目录的原工具重建通过，完整核心/公开 LLBC 摘要一致，三个生成文件逐字一致；仅按既有规则规范化无序 short_names 字典，不删除其余 AST。七类公开接线错误和五类源码/析构准入错误被拒绝。全部 27 个不透明调用、52 个由原工具处理的内建/全局/析构声明、363 个 Drop 位置及两版标准库/固定 Redis 源码均有登记。真实 Rust 错误版只将 fence_unknown 的赋值 true 改为 false，编译、原 Charon/Aeneas 抽取及生成 Funs 构建先成功；原 Results 仅更换 import 后，被同一 fence_unknown_exact 与 eval_error_returns_unknown 性质拒绝，三个内核反例/允许输入声明完成依赖审计，未手改生成 Lean 或改弱性质。

完整验收冻结于 `dfaf79a710db0475c3390ac1b888d37b2c37024a`，全部构建、抽取、测试和证明在指定 Linux x86_64 验证环境运行。原 verify-controller.sh 退出 0（1569.987 秒），全部保留模块重抽取、构建、真实正负用例、占位检查及 966 项唯一声明审计完成；原 Linux check-function.sh 退出 0（1717.152 秒），13 个阶段和 1210 项必需工具测试通过，工具测试无跳过、无 Linux 项排除。候选严格 Clippy 与 40 项组件测试通过，其中 26 项使用固定真实 Redis，候选无忽略项；仓库原有 opt-in/特权忽略项不计为已执行。既有 Lean 源的样式 linter 警告保留，未修改旧证明或禁用检查。

本批正式文件的真实 Lean 服务完成 131 项 Redis 审计，最终主 ProofAudit 服务另完成 15 项审计；均完整、非陈旧且无错误/警告。服务准入不筛选行号、声明或严重性，检查实际文件与全部证明源码指纹；18 类无效回执及保留的虚拟 worker 崩溃回执被拒绝。1710 项跟踪输入最终一致；两份原 LLBC 仅允许的无序短名排列先封存、核对完整规范化摘要后恢复原字节。完整机器收据 SHA256 为 `fc9490f8e39bfdbf4eb63e26bec1de18f79eba4ced3c449ff3951c6c56f0f1c6`，原失败记录保留。最后补录仅修改本覆盖章节，不改变已验收代码、定理、工具或冻结输入。

交付依赖固定 #563（头 `824abe2b`，前置 #556），最终目标 integration。验收期间 integration 已推进到 `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；该版本未改 Redis 主体、根锁文件或本批依赖合同，但新增 Host CandidateVerification 传递与中心后台接线。它们须重新核对对应使用点；冻结候选的完整通过不自动背书最新 integration 组合，集成后仍需复验。

正常返回性质仅覆盖登记的普通 payload 值域，排除可观察自定义 Drop、共享可变或资源 payload；异常、panic/unwind、abort、崩溃及终止性不从正常后态定理推出。逻辑连接消失不是物理关闭、回滚或持久化证据，错误允许已发生部分效果，读取/reconnect 不能单独解除未知围栏。完整生产 codec/schema、全部 Lua、读取/reconnect、缓存/outbox/派发和崩溃恢复继续由 V2 提供；原实现与候选的一般等价、物理外部效果及完整 VM/TLS/云端部署未据此证明或验收。不关闭整个 T7/V0，T9 继续排除。

## V0 底座统一与组合验收

#515/#510 的公共任务统一归 V0 #333，按 T0–T8 验收；具体依赖保证见[依赖登记](formal/dependencies.md)。[汇总 PR #551](https://github.com/VeriInfra-Lab/sandproof/pull/551)组合已有 V0、任务/排空、隔离业务试点和旧生成链退役成果，不新增业务证明。下文历史记录继续绑定各自冻结提交；#551 的组合证据以本节和该 PR 的最终验收记录为准。后续接口扩展以本页 T6/T8 节各自冻结组合为准，不能把一个批次的证据用于其它未核对组合。

组合 91fc769be12eacba589afb9a839c3421823f43f7 在指定 Linux 证明环境通过原 check-function.sh（1752 秒、1166 项工具测试、13 阶段）和完整 verify-controller.sh（1229 秒）。143 个保留证明文件及公共入口编译，744 个唯一声明的实际依赖审计通过；两个总入口经原 Lean 服务完整诊断，无错误、非陈旧。原任务正确/错误版组件及 root 安装文件场景通过。1629 个跟踪输入最终一致；原始 LLBC 差异先封存，经原审计器确认仅为允许的无序短名排列后恢复。机器收据 SHA256 为 8ebe95f4a09ca3ce290509a5aa779784a7db47e9de0e48d0392a096a08692174。

SDK 修复的可执行代码冻结于 0a80a9468cf6f3c35a42446e51e9844f3a094020：相对此组合，只增加受限夹具修复、绕开预读的真实 Listener/CommandHttp 同步与异步回归及验收说明；生产 Rust、Cargo.lock、证明/抽取、工具和正式冻结输入均未改变。复用上述完整证明证据，不把测试修复计为新增定理。固定 SDK 原完整兼容入口已在同一类 Linux 环境通过（410 秒）；移除真实 keepalive 拒绝的版本先编译成功，再被原 PID SDK 断言及新增夹具回归拒绝。完整基础入口、普通 CI、最终报告提交和 integration 合入状态在 #551 绑定记录，不能仅据局部通过宣称全部门槛通过。

此前 SDK 提前拒绝断连的失败证据仍保留；#550 中复用的是测试夹具的接收/消费时序修复，没有修改生产处理器或降低错误断言。新增传输回归不经过预读层，且原“头拒绝不读正文”和鉴权顺序检查保留。它们只提供已声明时序的工程证据，不是整个 HTTP/SDK、Tokio 或任意网络交错的形式化证明。

**当前保留证明集合的组合验收已通过；V0 的全部使用点、完整异步 Rust、业务生命周期与部署系统仍未完成证明。** #333 的未勾验收点和 #101 的业务/组合义务继续保留，T9 仍排除。


## T0 真实使用来源与析构复核

[#577](https://github.com/VeriInfra-Lab/sandproof/issues/577) 的隔离来源候选基于固定 #556（398b114d），实际生产来源仍为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；相对该依赖不修改生产 Rust、Cargo.lock 或 Charon/Aeneas。原生采集代码冻结于 `1d051915`，发布/准入实现为 `570c8997`，两者的 Rust 输入和编译配置相同。采集由原生产 Rust 1.97.0 完成，不经过新的解释器，也不产生新的 Lean 程序定理。

[固定入口](../verification/use-inventory/config.json)的五组开发/普通发布构建在 a6000 全部退出 0，完整采集退出 0（611.036 秒）；覆盖 18 个自有包、60 个实际编译单位变体与 798 项 Rust/Cargo 输入。45,129 个函数体记录包含配置重复，不能读作独立函数已证明；完整原文、调用/局部类型/终结语句及实际 Cargo 产物保留在忽略的证据目录。[登记结果](../verification/use-inventory/catalogue.json)与原文回执核对，[责任清单](../verification/use-inventory/responsibilities.json)精确匹配 25 项已观察自定义 Drop。解锁、计数、Unknown、取消、转交子进程与是否归还仍保留原业务责任，不新增整类可信假设。

初版十组编译均成功，索引因发布 MIR 的单行/匿名常量等格式退出 1；该轮原文和失败完整保留，修正后重新执行上述整轮采集。四个保留抽取根另外实际重建，生成 Lean 逐字一致，完整 LLBC 按原允许规则一致；本批只更新已审查的文档、新增来源检查入口及相应摘要，没有更换工具或修改生成语义。发布/来源回归及真实缺项错误对照的结果见 #577。

验收代码冻结于 `ba92936119c08f4c14811672f596a7762d377bcb`。独立 Linux x86_64 检出的原完整 `verify-controller.sh` 退出 0（1800.266 秒），原 `check-function.sh` 退出 0（2005.029 秒，13 阶段、1223 项工具测试无跳过）；768 个预期命名声明全部由实际依赖审计覆盖，无缺项。来源检查及完整入口预检共 26 项定向回归通过，完整原文与发布清单核对通过；三个真实数据缺项错误均被对应检查拒绝，它们不是业务错误版的形式化反例。两个总入口文件经原真实 Lean 服务完整、非陈旧诊断（13.701 秒），主审计的 15 项依赖检查通过。1668 个跟踪输入前后核对一致；两份原始 LLBC 只有原规则允许的无序短名排列差异，先封存、确认完整规范化摘要相同后恢复，没有修改生成 Lean 或清理 `.lake`。完整验收汇总回执 SHA256 为 `63f9f7893736083cc4050b097f29530ae00f011ad0ed8d5588f0d85ac08a6671`。

首轮完整证明因 Lean 创建线程失败退出 134；同一失败命令在原 64 GiB 地址空间上限内，采用 `LEAN_NUM_THREADS=8`、`LEAN_STACK_SIZE_KB=65536` 后通过，上述完整证明沿用该资源设置，不更换工具版本、证明目标或内核。首轮基础检查另发现预检源码副本缺少 Git 上下文；修正测试副本的登记后，原 Cargo 哨兵/陈旧父摘要断言及新增未登记 Rust 拒绝检查均通过。两类失败证据保留，未放宽准入或安全检查。最终报告只补本节、将公共文档的机器别名改为通用环境称谓并同步对应摘要，可执行代码、模型、证明和生成语义与验收代码相同。既有 opt-in/特权忽略项不计为执行，未新增产品 VM、SDK 云端、TLS 或部署验收。

本项只加强已观察源码的清单与可复现定位。动态回调实际实例、结构体字段/合法域、future 取消时的编译器清理、嵌套库析构及外部脚本/配置的完整绑定仍按[依赖登记](formal/dependencies.md#真实使用点的来源清单)和 #333 继续处理。额外 VM 运维入口只作防漏观察，不增加历史工具证明前置；模板范围与 T9 排除不扩大。**T0 全部使用连接、T1–T8 其余义务和整个 V0 均未因此完成。**

## T7 边界：SQLite 提交回执与缓存发布（隔离候选）

[#570](https://github.com/VeriInfra-Lab/sandproof/issues/570) 的 Rust 输入固定为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；本批验证代码冻结于 `c47c42329560cd226ae1ca48674d206f5d509fc3`，依赖固定 PR #556（398b114d）的最新 B1 接线复核。候选仅在私有副本应用[固定补丁](../verification/experiments/sqlite-commit/candidate.patch)，原 WorldTransaction::commit_with/Drop 实际调用新增同步核心。生产 Rust、锁文件和 Charon/Aeneas 没有因此改动。正式源、生成物和条件分别见[候选清单](../verification/experiments/sqlite-commit/candidate.json)、[环境与完整析构登记](../verification/experiments/sqlite-commit/environment.json)及[接口保证](formal/dependencies.md#a4b2b4-sqlite-提交回执与缓存发布接口隔离候选)。

[Results](../verification/aeneas/sqlite/SandproofSqlite/Results.lean) 的 13 条核心/通用状态性质连接原清空、回调结果分类、事务槽消耗、原候选发布和 rollback 分支；正常返回须取得原 live 事务，正常成功必须由原回调确认。[Examples](../verification/aeneas/sqlite/SandproofSqlite/Examples.lean) 的 5 条允许输入/结果见证分别保留提交未知的写前、写后结局，两者都不能保留旧缓存。18 项声明由原标准依赖命令与真实物理 Audit 文件检查，不增加自定义假设。这些是隔离候选真实核心的条件性质，不是整个生产事务外壳已证。

A4 提供固定 SQLite/rusqlite 的原事务结果；B2/B4 连接原事务句柄、两个原缓存单元及给定值/结果传递，要求不同单元、正常借用、回调及捕获值析构不额外改写缓存。逻辑状态表示两单元的值，不声称机器证明 RefCell 的堆实现。核心后态早于外层 WorldTransaction 候选字段销毁；普通 World 值域及原事务内部结束按登记限制，不提供物理关闭/回滚/释放、终止、panic/unwind、abort 或崩溃后保证，也没有证明原版/候选任意行为等价。

实际保留旧缓存错误版只删除首次清空，先编译并通过原 Charon/Aeneas，再由未改动的 cache_tests.rs:102 拒绝。正式 Results.lean 逐字复制，在四个对应性质处拒绝；三个错误见证/辅助声明经独立内核和标准依赖审计。核心 35 项前端声明/81 个析构位置，包装 49 项声明/117 个析构位置均完整登记；这些包含预声明，不代表每项均在运行。两版标准库各 26 份源及固定 SQLite 源核对通过，实际使用 bundled SQLite 3.53.2（3053002）。完整中间表示只按原规则规范化无序 short_names；生成 Types/Funs/translation.json 逐字再生核对。

指定 Linux x86_64 验证环境的原完整 verify-controller.sh 在 c47c4232 退出 0（1797.588 秒）：全部保留模块、实际重新提取一致性、占位检查及 786 个唯一声明依赖审计通过。专用 verify-sqlite.sh 独立退出 0（182.247 秒），正确候选的原 16 项缓存测试、真实错误版和 18 项物理 Audit 服务检查均通过。原完整 Linux check-function.sh 退出 0（1747.478 秒、13 阶段），1199 项工具测试无跳过，包含工作区与 contention-diagnostics 的 Rust 测试、文档测试和严格 Clippy。最后对 SandboxModel.lean、verification/ProofAudit.lean 和本库 Audit.lean 的真实服务检查退出 0（18.922 秒）；主审计 15 项和本库 18 项均完整、非陈旧、无错误/警告。1683 个跟踪输入按原规则核对一致；两份新 LLBC 的原始输出已保留，仅完整摘要确认无序 short_names 排列等价后恢复原序列化。预置工具链接另行核对。原 opt-in/特权忽略项不计为已执行，产品 VM、TLS、外部云端 SDK 及部署验收没有运行；这不是整个 V0 的验收结论。

完整 commit() 的自有验证器图在原 64 GiB 地址空间限制下仍出现分配失败，没有完整 LLBC；没有证明实际峰值为 64 GiB，也没有把验证器设为可信。具体 SQL/参数/列、完整投影/额度/寿命/outbox、候选正确性、data_version/autocommit、恢复与派发仍归 V5 #338。当前接口不关闭整个代表性 SQL 写入链、T7/V0 或全系统，T9 继续排除。
