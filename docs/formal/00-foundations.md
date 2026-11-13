# V0 公共能力与 T0–T9 验收

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#632](https://github.com/VeriInfra-Lab/sandproof/issues/632)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 边界与复用

V0 固定本阶段共享身份、数值、事件、表示关系和逐接口依赖；不负责 VM 生命周期业务，也不建通用 Rust 异步解释器。原 #333/#515/#510 的历史成果按准确适用范围保留，旧计划由本任务接替。T0–T9 是本 issue 内验收点，不另开 stage 或通用工具项目。

主要代码为 [sandbox-protocol](../../sandbox-protocol/src/lib.rs)、[数值](../../sandbox-protocol/src/numeric.rs)、[内部生命周期](../../sandbox-protocol/src/internal/lifecycle.rs)、[有限表](../../controller-core/src/store.rs)。现有 [Model](../../SandboxModel/Foundations/Model.lean)、[Representation](../../SandboxModel/Foundations/Representation.lean)、[FiniteCopy](../../SandboxModel/Foundations/FiniteCopy.lean)、[FiniteInsert](../../SandboxModel/Foundations/FiniteInsert.lean)、锁/任务/存储边界可以复用；具体实例仍需证明实际调用对应。

共用合同写清输入前提、输出事件、修改范围、错误/未知/部分效果、取消/崩溃和预算。Accepted、SnapshotCommitted、LoadedPaused、MayHaveExecuted、Ready、Stopped、Cleaned、Released 不互相替代。地址、PID、位置、逻辑编号与内容哈希不是同一身份。

## 2. 六组有限公共接口

| 接口 | 提供什么 | 调用者还要证明 |
|---|---|---|
| F1 身份/数值/字节 | 完整相等、版本与单位、有界加减及耗尽拒绝、文本/字节表示 | owner/会话/运行代/capture/operation 的业务绑定和实际合法域 |
| F2 有限容器与循环 | 原顺序、首次匹配、修改范围、有限终止 | 实际比较/克隆/循环体及容量；不能假设自有 Ord 正确 |
| F3 编解码与有界输出 | 固定 schema/字段/首错、合法值往返、写入与错误分类 | 实际新消息、manifest、查询结果和限额连接 |
| F4 锁、任务和资源责任 | 已登记锁/持额/消息/取消规则，资源移交与原所有者 | 回调业务、pin/claim/GC 原子边界；Drop 不是进程崩溃清理 |
| F5 存储/外部结果 | 提交确认/未知、原连接/参数绑定、精确观察类别 | 实际 SQL/Lua/文件发布顺序与恢复策略 |
| F6 历史/精化/进展 | 表示关系组合、内部步、有限工作终止接口 | 真实 Init/Step、待办启用、驱动调度、跨模块前提提供者 |

字段与公共记录可以先按模型命名，但每条实现对应必须给出真实 Rust 字段，不要求为了论文新增空壳 crate。F6 的环境公平条件不能代替本项目的出队、激活或回收算法证明。

## 3. T0–T9 的有限验收清单

每项记录“可复用 / 公共缺口 / 实例连接 / 下游业务 / 本阶段排除”，附实际根、版本、合同、定理与证据。只覆盖本阶段启用根及其闭包，不能用删除未知点造覆盖。

| 点 | 第一阶段要验收的内容 | 已有基础与仍缺什么 | 完成判据 |
|---|---|---|---|
| T0 来源与证据 | 公开 create/unload/resume/destroy/query 到 Redis、收件、SQLite、VMM、pin/GC 的实际根、回调、资源与 cfg | 已有来源目录/冻结与复用工具；卸载、Resume 与材料退休已有候选入口，完整根/闭包登记及当前版本绑定未齐 | 每个根到真实调用/外部条件/业务提供方可追踪；未启用功能不可达或有修改范围证明；源码、工具、依赖变化失效传播明确 |
| T1 类型、字节和单位 | 身份、运行代、capture、操作窗口/序号、期限、大小、真实摘要；零/耗尽/溢出/非法 UTF-8 | 已有编号/算术和部分 G1/G2；旧 manifest 逻辑编号不能冒充内容摘要 | 每个实际字段有域、单位、完整参数与往返/拒绝对应；不同业务角色不能凭相同 u64 自动互换 |
| T2 JSON 与输出 | 新生命周期请求/结果/确认、窗口、manifest 和必要授权头 | 已有代表 schema、有界 Writer；新增 schema 及输出接线待做 | 全字段、derive 属性、字节/文本、限额、未知字段和首错有同实现证据；不把 serde 当整个自有 decoder 正确 |
| T3 容器与循环 | 生命周期验证、工作表、claim/pin/ref 集合、精确清理扫描所需操作 | 有限复制/插入、普通键 BTreeSet insert 已有；contains/迭代/可变借用按真实需要补 | 原顺序/首错/回写/删除与唯一性可复用；无需先完成全部标准库容器 |
| T4 回调与借用 | Redis codec/clock、SQLite 事务、后台捕获/加载/清理业务回调 | 已有若干真实回调接法；新回调仍需抽取/对应 | 同一实例/全部参数/次数/顺序/结果；自有回调不是可信条件；必要重构有行为对应证据 |
| T5 资源移动与 Drop | pin/worker/FD/VMM 的所有权转交、等待取消、reaper 接管 | 现有持额消息/Child 观察只覆盖部分；新材料使用者需精确规则 | 不提前归还、不丢责任、不重复释放；正常析构、panic、abort、进程死亡分开；持久 pin 由 V5/V6 证明 |
| T6 异步与共享状态 | 已有 Mutex/RwLock/Condvar/Notify/JoinSet/任务池合同实例；同会话竞争、注销/取 pin 原子性 | Tokio 机械接线为已登记 B 条件；新业务策略归 V4/V5/V8 | 原锁/池/信号/回调完整绑定；正确版通过、提前释放/绕过当前授权错误版被同性质拒绝；不声称 async Rust 自动翻译 |
| T7 Redis/SQLite/提交 | 原事务/CAS/Lua/参数与未知提交；file publish 与 DB 非原子 | 已有 Redis/SQLite 代表合同；新增 schema、脚本、事务和对账待做 | 已提交/未提交/未知不混淆；原回执/引用和工作同事务；实际 SQL/Lua 业务由 V2/V5 证明 |
| T8 OS/TLS/Firecracker | 受保护原句柄、短写/sync/rename/删除、进程观察、Full capture、LoadPaused、激活与 File backing | TLS/relay/Metadata 等已有局部结果；open_checked 仍有抽取缺口；Full 捕获已接线，LoadPaused/激活/运行 backing 与回收未闭合 | 精确符号/版本/条件登记，新 S-C1–6 可实测；不能把路径安全、manifest 或 GC 决策整段信任；原句柄路径必须有可实施连接方案 |
| T9 范围排除 | 浮点与完整来宾兼容算法 | 不进入本阶段 | 本阶段证明根不依赖它；若实际必经，应先报告范围冲突，不能隐式绕过或假设结果 |

原 Guest `file_query::into_text`、完整流/PTY 编解码与无关容器不再是 V0 前置；新 manifest/协议真的使用到的字节处理仍要做。旧 P4 `protected_files::open_checked` 是潜在真实阻断：先选保持 flags/原句柄/错误顺序的小拆分，不能用 Metadata 三个 getter 的证明掩盖路径取得和替换竞态。

## 4. 最小推进与验收

先选 V3/V5/V6 的一个完整 capture/claim/pin 样例冻结所需接口；公共能力和真实调用连接一起交付，避免只堆新抽象。按需小重构可行，但保持固定 Charon/Aeneas；没有可抽取接法时报告具体阻断及可信条件变化，不扩大工具开发。

每批应有：真实入口与 cfg、Rust/LLBC/生成 Lean 绑定、公共模型及调用证明、显式 A/B 或 S-C 条件、负例/边界用例、未覆盖项。原生测试、接线审查、生成体证明和实机结果分别报告。不能用测试替代证明，也不要求重复证明外部引擎。

## 5. 开工与完成条件

开工提供真实根、有限输入域、外部效果、表示关系、性质 ID、前提提供方、候选与环境。V0 完成须所有阶段所需 T 点均有明确结论，并由 V1–V9 实际调用；下游业务算法不变成 V0 前置，未完成公共缺口也不能改名下游来关单。

不以旧 issue 关闭、定理数量或工具安装作完成证据。普通检查在 ning，证明在 msra；具体主机信息只放私有操作记录。沿用[证明工作流](../proof-workflow.md)的完整入口与精确输入复用规则。
