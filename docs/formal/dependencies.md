# 外部依赖与可信链登记

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：依赖合同与接入规则；包含已有模型和待连接接口，具体完成情况以覆盖记录为准。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

本页是外部依赖 A1–A12 和获批可信薄适配 B1–B4 的唯一明细；模块计划引用合同编号，不再各自列一套“依赖不会出错”。当前采用原 Charon/Aeneas 抽取真实同步业务核心、窄接口模型和显式可信适配的路线，不以开发通用 async/Pin/Drop 翻译器为前置。可信范围来自项目 owner 的明确选择；其余自有业务继续由对应模块证明。

### 当前中心身份的薄连接

[`CenterPeerRegistration::context_for_verified_peer`](../../sandbox-internal/src/lifecycle/peer.rs)只实例化既有 A9/B4：输入必须是原 `ProtectedServer` 在链、用途、时间、角色 pin 和 ALPN 检查后交给 `ProtectedHttp::respond` 的那个 `CertificatePin`。受保护配置将最多两个轮换 pin 绑定到同一 `CenterIdentity` 和完整本地 `NodeIdentity`；未登记 pin 拒绝，不能从 HTTP 头、正文或任意证书字节创建这个认证事实。

输出 `AuthenticatedCenterContext` 不可反序列化，但公开工厂是明确的可信装配接缝，Rust 类型本身不证明调用者无法伪造。配置/pin 来源与调用绑定仍须在正式服务接线时核对；已有摘要值测试只检查映射，不证明 TLS 握手。`AcceptedLifecycleIntent::validate` 的中心全文、部署、原请求节点和接受版本检查属于自有同步判断，不将其整体列为 B4。持久接受、授权、原对象选择、材料完整、退出、清理和发布仍由各业务模块证明。

显式 `InternalServer::with_center_peer` 路由沿原 `ProtectedHttp::respond` 的实际 peer
调用该映射；原 TLS/连接预算和有限请求期限保留。原 HostFacade 只转交给同一
HostLifecycle；许可进入实际 blocking worker 后才调用原应用收件，不因 HTTP 取消提前归还。
可依赖的机械保证限于原对象、完整参数、结果和许可寿命；接受资格、全文重放、
SQLite 提交与回执匹配是自有业务，仍需对应证明。超时/错误不证明没有提交，
正常 Recorded 也不证明当前 Ready、执行完成或释放资格。
真实 TLS 与原 SQLite 的组合测试仅支持该有限接法的工程采用；
不证明任意 async Rust 精化、panic/abort 收尾或完整生命周期。

## 第一阶段新增依赖合同

这些是固定 Firecracker/文件系统接口的**拟采用条件合同**，并非本次新增已证事实。原 A1–A12/B1–B4 的范围不扩大；下表在 A1/A3 下细化本阶段所需接口。实现时必须补实际调用符号、版本/配置、失败路径和实测证据，不能仅登记编号就算连接完成。

固定参考：[Firecracker snapshot 文档](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/docs/snapshotting/snapshot-support.md)。其中 RAM/设备状态与磁盘文件分别管理，File 后端在 VMM 生命周期内依赖原内存文件，加载可保持暂停。文档不替我们实现 bundle、所有权或 GC。实际发布前再次核对所用 1.16.1 二进制源码与配置，不把网页版本当产物指纹。

| 编号/调用 | 可以依赖的准确后果与条件 | 错误、取消、崩溃及不保证什么 | 必须由自有实现提供 |
|---|---|---|---|
| S-C1 / A1：原 VM Pause、Full snapshot | 在支持的 CPU/内核/设备组合、精确受控 VMM 和成功暂停条件下，成功捕获给出该暂停 VM 的内存与设备状态；只采用 Full | 请求/响应丢失可能动作已做；不假设超时未暂停、不假设输出文件不存在；不含外置磁盘或业务提交 | V6 绑定原 VM/参数，V4/V5 禁止在完整来源接管前停止原 VM；V8 排空相关访问 |
| S-C2 / A1/A3：设备完成与磁盘复制 | 只在声明设备写入已完成到宿主文件系统、原 VM 持续暂停且无宿主写者时，按原 FD 完整复制得到与捕获匹配的 disk；固定支持的磁盘模式 | snapshot 返回不单独证明磁盘介质持久；文件复制可短写/ENOSPC/留下部分对象；不支持任意外部块设备或并发写者 | V6 建立截面/关闭写者/核全字节和盘集合，V5 预留临时峰值与恢复责任 |
| S-C3 / A3：write/fsync/rename/dir sync | 原受保护目录、同文件系统、无替换写者、成功完整写入及规定文件/目录同步下，发布的唯一对象可在声明 manager-crash 故障域重新打开；rename 只管目录可见性 | 文件与 SQLite 不原子；sync/rename 错误或崩溃可能已产生效果；不覆盖断电/盘损坏/任意旧备份回滚 | 原路径/FD/身份、manifest 内容校验、发布次序、对账和失败保留由 V5/V6 证明；不能从存在即推可恢复 |
| S-C4 / A1：snapshot Load | 完整兼容的受保护材料、精确新 VMM、File 后端且 resume_vm=false 成功时装载为暂停；设备路径/配置满足固定约束 | 超时可能已装载，加载失败可能有进程/映射；不保证 envd/current credentials 就绪、不赋予运行资格 | V6 核材料/参数/新原对象，V5 pin 先于使用；V4/V5 唯一 claim 和激活前消费由自身证明 |
| S-C5 / A1/A3：File backing 生命周期 | 原内存文件保持不可变且可用、所有已登记 reader/mapper 受控时，VMM 可按固定实现使用 backing；原 VMM 与相关 worker 结束后对应使用才结束 | load 返回不是使用结束；unlink、Arc 数量、manager Drop 或旧路径缺席均不证明物理释放；外部修改不支持 | V5/V6 持久具名 pin、先取后用、真实退出、retire/取用互斥、精确删除和计费全部是业务证明 |
| S-C6 / A1/A7/A9：设备重置与恢复连接 | 只按固定设备/guest 接口解释恢复后的连接与初始化；现有连接可断，外部时钟/网络按原合同重新观察 | 不承诺 TCP/vsock 无缝延续、外部副作用回滚或旧凭据自动失效；自有 Rust guest 不能整体当 A7 | V8 关闭旧入口、当前凭据重授权、必要 guest hook 与保留进程断线行为；V6/V7 对接就绪和结果 |

**S-C7（来源/资格）和 S-C8（退休/成功/回滚）不是依赖保证。** 它们分别由 V4/V5/V6 及 V1/V7/V8 提供业务定理：选定正确 manifest、唯一认领、激活前消费、普通失败保状态、当前事实发布、pin/GC 竞争与旧工作失效。这里保留编号仅为防止把它们挪入可信边界，不新增公理。

B1–B4 仍仅允许已登记的参数/回调/锁/任务/通知机械接线。新增 async helper 若含 capture 选择、claim、SQL 条件、授权、成功判断或删除决定，必须拆出透明业务并证明；不能把 helper 全部改为可信。尚未落实的边界不能填“已证明”，实机测试也不使外部假设变成 Lean 定理。

边界变化的阶段验收：V0 登记精确符号与正常/错误/未知/取消/崩溃/修改范围；V5/V6 实测 S-C1–5，V8 实测 S-C6；V9 审计所有输入前提有提供者。只采用管理进程崩溃域；若未来纳入断电，需要重新定义持久性与恢复实验，不能自动扩张结论。

## 如何阅读和使用本页

这里的“保证”是**在写明条件下，一个具体接口允许我们依赖的行为**。它不是“这个库永远不会出错”，也不是“用了这个库，我们自己的调用就一定正确”。例如：数据库可以保证一笔事务原子提交；把哪些记录放进同一事务，仍是我们要证明的事。

本页统一维护保证内容；[V0 验收点](00-foundations.md#3-t0t9-的有限验收清单)维护完成要求；[覆盖记录](../proof-coverage.md)记录哪些实现已经证明、适用版本和未完成连接；[#632](https://github.com/VeriInfra-Lab/sandproof/issues/632) 是本阶段公共能力执行任务。原 #515/#510 的工作归入该任务，旧 Future/MIR 工具扩展计划不再执行。T9 继续排除。

后续模块使用公共成果时只需：选定合同编号和版本，指出真实调用位置，证明自己的输入满足前提，说明结果怎样进入业务状态，再检查该调用的异常分支。相同库接口不重复建模。新用法超出原范围时补充该合同并检查受影响的调用者；V0 完成不意味着以后任何 Rust 或依赖用法都无需新增工作。

### 一条保证必须回答的八个问题

| 问题 | 必须写清的内容 |
| --- | --- |
| 调用什么 | 完整函数/类型、实际调用者、版本、目标平台、编译特性和配置；不能只写 Tokio、SQL 或 Linux |
| 调用前有什么 | 原对象/池/锁/连接身份、合法输入范围、已经持有的许可、时间和空间限额 |
| 成功说明什么 | 返回值的准确含义；观察发生在哪个时刻；是否只表示本地接收、远端接受或持久提交 |
| 失败留下什么 | 尚未开始、部分执行、结果未知分别处理；明确错误是否可能在修改之后返回 |
| 取消和崩溃怎么办 | 停止等待、停止任务、进程退出、正常析构分别定义；崩溃后靠哪些持久记录恢复 |
| 能修改哪里 | 哪些字段、对象或字节可以改变；其它对象保持什么；多次读是否构成同一快照 |
| 谁提供依据 | 外部依赖假设 A、已批准自有适配条件 B、实际 Rust 对应定理三者分开；列源码审查和测试入口 |
| 变更影响谁 | 模型/源码/配置摘要、使用定理和调用模块；变化后重新检查这些依赖，不能只刷新摘要 |

“允许登记的合同族”“已写出可计算模型”“已证明模型定理”“真实调用已经连接”是四种状态。下表给出的接口要求不表示它们均已完成模型和真实连接。每条实际使用记录必须能找到上述八项和覆盖记录；空白项就是待完成工作。

### 底层接口的最小保证和调用者责任

以下条款限定正常返回和明确失败的含义；程序仍允许断网、并发、取消、部分效果和独立崩溃。A 编号沿用后文目录，不新增一套不兼容编号。

| 接口与合同 | 满足前提后可以依赖 | 失败、取消及明确不提供的结论 | 我们还要证明 |
| --- | --- | --- | --- |
| A6：固定标准库的整数、String/str、Option/Result、Vec 操作 | 在有效有限 Rust 值的表示范围内保持下文登记的值、顺序、短路和错误传播 | 正常值模型不保证物理分配成功；自定义 Clone/比较/回调/Drop 不自动正确 | 自有编号/合法性检查、算术边界、循环与具体回调；接口逐项见后文库模型登记 |
| A6：`serde_json::from_slice/to_vec`、固定 Serde 派生及读写接口 | 按实际类型和属性解释字段；字符串保持合法 UTF-8 文本，数字不被模型静默缩窄；标准派生调用对应字段处理 | 不自动拒绝所有未知字段；失败可已消费输入/输出前缀；不承诺任意两次有状态编码相同 | 实际 schema/属性/字典绑定、自有校验和完整参数；叶子定理不能代替整个字节入口 |
| A6：`Semaphore::try_acquire_owned/acquire_owned`、许可的正常 Drop | 原池中取得独占许可；容量不够时拒绝或等待；取消排队归还该 future 已获许可，Drop 归还原池 | 唤醒前可能已经获额；不保证等待必定结束；close 不证明已有工作退出；不覆盖 forget、额外增额或进程崩溃 | 同一池家族、实际容量、许可用途，以及每条持额/转交/归还路径；具体 B1 见下文 |
| A6：`spawn_blocking`、`JoinHandle::await/abort/Drop` | 运行时接管给定闭包；回调至多调用一次；正常 join 返回该回调结果 | 丢弃 handle 只停止等待；已开始的阻塞工作不能靠 abort 停止；JoinError 之前可能有业务效果 | 把原许可和完整输入移入同一任务，跨越回调持有；回调本身正确，错误不能冒充未执行 |
| A6：`Mutex/RwLock`、`Arc::clone/strong_count`、`AtomicBool::load` | 锁只约束同一受保护对象；克隆 Arc 保持对象；计数/原子读取返回相应单次观察，内存顺序按实际操作 | 多次读不自动成为一个快照；Arc 不保证内部数据线程安全；Acquire 不保证看见任意“最新”写入；锁不自动防死锁 | 哪些字段归哪个锁、对象未串线、读写顺序和共同不变量；`count == 1` 是否足以支持具体决定 |
| A6：`watch::Receiver::wait_for/changed`、`select!`、`timeout`、实际 channel send/recv | 只采用实际接口的通知、队列、完成与关闭语义；watch 保存最新值，不是全事件日志；`biased` 按分支顺序 poll | 未观察撤销前可已有 I/O；超时不回滚副作用；发送成功不等于接收方已处理；不同 channel 的取消/丢消息语义不可混用 | 撤销谁、有权撤销的时间点、完整身份、缓冲界限、关闭/丢消息处理，以及需要保留的未确认责任 |
| A6：实际 `read/write/poll_frame/flush/shutdown`、HTTP body | 成功读写 `n` 只对应本次接受的前 `n` 字节；保持该连接内的顺序，允许短读短写和背压 | flush/HTTP 200 不等于远端业务成功；EOF 不等于协议完整；错误/取消可发生在部分传输后，Drop 不保证错误响应送达 | 短写续传、字节不丢不重、协议结束、所有缓冲与许可、半关闭和真实退出；固定 `StreamHistory` 供 V8 连接 |
| A4：rusqlite 事务、execute/query/commit | 在受保护原数据库、声明配置和故障范围下，一笔事务按数据库合同提交或回滚，成功查询反映其事务视图 | 连接断开或提交错误未必能判定最终结果；不回滚 VM、文件和网络效果；不同事务不是同一快照 | SQL/参数/列映射、必须同事务的记录、提交确认、缓存更新、恢复和派发次序（V5） |
| A11：Redis 命令/EVAL 和持久确认 | 固定配置下采用命令及脚本的原子执行和持久化合同；脚本执行期间无其他命令插入 | Lua 运行错误不自动回滚先前写入；断线超时可能已生效；一次读取不是耐久提交证据 | 脚本内容、写前检查、CAS 修订、完整载荷、确认和未知围栏、缓存/outbox/恢复（V2） |
| A9/A12：TLS 握手、证书验证、认证连接读写 | 固定算法/验证策略下，握手验证所配置身份条件，并保护本连接的字节；按密码学假设使用摘要/认证值 | 认证成功不自动授予业务权限；跨连接会重复/乱序/丢请求；有效凭据被窃取不由 TLS 识别 | 证书到部署/角色/节点映射、端点、完整请求绑定、撤销、重放和错误解释（V3/V7/V8） |
| A2/A3/A5：进程句柄、文件描述符、文件/同步/网络规则操作 | 对已经绑定的原对象执行声明动作；成功返回只提供该动作规定的事实 | 打开句柄前可能选错对象；路径/PID 可复用；单进程退出不等于所有辅助进程退出；命令返回成功不等于规则覆盖全部流量 | 身份检查、受保护路径、对象与租户绑定、调用顺序、部分失败、围栏和精确清理（V6） |
| A1：Firecracker/jailer 配置、启动及状态接口 | 在支持且安全配置的宿主上提供约定隔离；配置/启动响应只说明对应阶段 | 不替我们授权租户、配置网络或保证 envd/应用就绪；启动后的观察可能立即失效 | 下方 SafeLaunch 各项、资源/配置来源、实际控制组/规则、就绪与发布条件（V6） |
| A8：实际时钟读数 | 只按声明的单调性、现实时间误差和重启条件解释读数 | 单机单调时钟不等于跨机同一时间；读到时间不保证之后仍未到期 | 单位/溢出、持久时间下界、到期边界及检查与使用竞争（V5/V7/V9） |
| A7/A10：固定 envd/SDK 与模板交付 | 在各自功能合同下使用请求、完成和不可变产物描述 | envd 任意响应不能成为宿主安全事实；模板子系统本身整体排除；文件/命令失败可留部分效果 | 端点/产物完整绑定、授权、长度/形状/摘要、结束解释和清理；现有排除范围不扩大 |

这一表是合同的使用要求，不给任何整个自有类型或服务加“可信”标签。Redis/SQLite 配置、标准库具体函数、Firecracker 条件及 B1 的精确限制以下文为准；动态接口必须再登记实际实现，不能把未知 trait 实现当作已覆盖。

Tokio 条款按锁定的 **1.49.0** 核对：[阻塞任务](https://github.com/tokio-rs/tokio/blob/tokio-1.49.0/tokio/src/task/blocking.rs)、[等待句柄](https://github.com/tokio-rs/tokio/blob/tokio-1.49.0/tokio/src/runtime/task/join.rs)、[许可池](https://github.com/tokio-rs/tokio/blob/tokio-1.49.0/tokio/src/sync/semaphore.rs)、[watch](https://github.com/tokio-rs/tokio/blob/tokio-1.49.0/tokio/src/sync/watch.rs)、[select 的取消规则](https://docs.rs/tokio/1.49.0/tokio/macro.select.html)。API 文档说明接口意图，冻结源码/摘要和使用点核对说明本项目实际使用哪一版；二者均不代替外部库的形式化证明。

### 两个具体的复用例子

**后台持额。** 模块先证明回调的业务前提和结果。A6 提供原任务/许可操作语义，B1 提供“原许可被移进执行同一回调的任务，并持有至该回调结束”的条件。满足同池和输入绑定后，可直接使用 Task 的次数与容量定理。HTTP 调用方取消只改变等待状态；任务还在运行，就仍计入额度。回调可能先写数据库再返回 Busy，因此 Busy 不能用作“可以安全重放”的证据。

**网关撤销。** 业务模块证明原授权已失效、不得新增租约。B3 只负责把原撤销信号接到原工作 future，以及传递 body/relay 的生命周期通知。DataDrain 的真实读体定理只证明本次 `count == 1 && !unknown` 判定；V8 还需建立计数覆盖了哪些持有者、没有新 pin、未知怎样产生和保持、原 relay 的退出证据。取消等待或 body Drop 都不能直接推出可以释放 VM。不同模块复用这些规则，不重新定义一套“取消就是退出”。

### 接入记录和失效处理

一个新调用点的记录应写成：“在版本 X 的函数 F 中，调用依赖操作 G；原对象为 O、输入为 I、限额为 L；成功使用条款 A，错误使用条款 B；业务前提由定理 P 提供；机械接线依赖 B1 的哪些句子；源码检查/组件检查入口为 C；未覆盖 U。”不能只写“符合 Tokio 合同”。

先用原工具预检真实入口。签名无法表达时，优先拆出真实使用的同步决定和具体值接口；单纯标成 opaque 不能消除不支持的类型。小范围重构需保留首个拒绝、短路、状态/回写、回调次数及资源寿命。未经一般性对应证明的旧新实现对照，只计审查和测试依据。

版本、配置、模型或调用代码发生变化时，先列出受影响合同和调用者，再检查源码差异、重新抽取/编译和相关回归，最后更新冻结记录。发现合同不满足时保留错误或未知，停止引用依赖它的结论。无法通过窄边界解决的实际阻断记入 #632 对应 T 项，不自行扩大工具开发、信任范围或改变业务要求。

## 依赖登记与使用规则

每批冻结：合同ID、库/程序精确版本与源码/二进制摘要、实际配置、调用前提、成功/失败/未知/部分效果/取消/panic/abort/崩溃语义、使用函数与回调、对象身份、状态修改范围、预算、部署检查、提供方、验证方式、失效影响和未验证项。版本来自仓库锁文件/工具链/候选部署，不从官网latest自动升级；官网作为语义出处，不能替代版本核对。

| 保证类别 | 使用方式 |
|---|---|
| 自有业务定理 | 鉴权、完整请求绑定、准入、SQL/Lua 条件与参数、提交/恢复、撤销与释放判断、同步回调及业务组装由实际实现对应定理提供 |
| 外部合同 A | 最小库/OS/引擎/端点合同作为显式参数，说明哪类失败仍允许，不能假设永不出错 |
| 可信薄适配 B | 仅下表逐项登记的任务/资源/锁/取消和参数转发接线；合同符合性依赖源码审查与测试，不称其真实异步 Rust 已机器证明 |
| 调用绑定与工程证据 | 核对真实调用目标、输入/结果、同一池/锁/句柄、features/cfg 与合同版本；审查、差分/故障/KVM测试不代替一般性精化定理 |
| 当前不承诺 | 明确范围外能力，不以不支持分支伪装完整实现 |

A_safe允许恶意来宾/坏请求/断网/资源不足，但依赖宿主隔离和受保护控制数据；A_function额外依赖本次 envd、文件系统和客户端功能合同。安全定理不因不可信envd而相信其任意Ready/文件声明；功能定理也不声称用户程序结果正确。

### 逐符号可信适配登记

生产运行仍调用真实依赖；验证时使用模型描述该接口允许的返回、事件与状态变化。模型是合同，不是总返回成功的替代业务实现。A/B 合同由定理的显式参数承载，不新增自定义公理；合同模型通过内核检查，只证明所写模型的性质，不证明真实依赖或适配已符合它。

每个操作保留 `Pre/Input/Event/Post/Frame/Fault/Budget`，并明确：未开始而拒绝、已执行后的错误、结果未知、部分完成、取消等待、工作退出、panic/unwind 与进程 abort/崩溃。未登记的失败不能默认无副作用；panic 不保证回滚，abort/崩溃不保证执行 Drop。后者进入对应恢复模型，不凭资源析构得出已清理。一个对外错误码可以有多个来源，模型须保留来源或保守保留可能效果，**不能由 Busy 推出回调未执行，也不能由取消、超时或 Future 被丢弃推出外部工作已退出**。

| 编号与实际使用点 | 可依赖的机械保证 | 使用方仍须证明或绑定 |
|---|---|---|
| B1 后台任务与许可：[HostLifecycle](../../sandbox-host-service/src/lib.rs)、[status_admission](../../sandbox-host-service/src/status_admission.rs) | 使用原 work/status 池；许可与回调交给同一任务，许可不复制、至多释放一次；排队/运行的后台工作继续持额，丢弃等待方不提前释放。普通入口领取失败不提交；健康查询先有一个 status 预留，再排队取得原 work 许可；排队取消归还已取得预留，取得后两张许可随同一后台工作持有。排队公平及任务最终获调度仅在版本和调度前提成立时使用 | 池初始容量、全局共享关系、实际回调与完整参数/结果绑定；回调业务定理及峰值预算。领取失败与 JoinError 均可能映射 Busy，但后者允许已有部分业务效果；不把状态等待队列合同当业务总能启用或总能结束 |
| B2 锁、共享对象和快照的薄接线 | 原锁互斥保护同一对象；获取/释放、共享句柄身份、被保护快照与读写回写按登记事件对应；中毒、错误、等待取消及可能阻塞不被抹去 | 哪些字段受该锁保护、快照是否足以作决定、在锁下调用哪个真实核心、回写顺序与共同不变量；跨锁顺序、重入、跨 await 持锁和无环等待不自动由互斥保证 |
| B3 网关任务、撤销信号与流驱动：[until_revoked](../../sandbox-data-transport/src/revocation.rs)、[drain](../../sandbox-host-service/src/data_gateway/drain.rs) | 原信号/连接/租约身份不串线；已撤销或权限方已关闭时优先结束本次等待，不 poll 工作；后续撤销按实际 select/poll 观察中止该 future；字节、结束、错误和句柄按库合同传递；body 与 relay 的独立持有、Drop 通知按登记接线发生 | 撤销权限和生效点、已授权完整绑定、已写字节及预算、终局分类、真实退出/排空证据、Unknown 保持及释放许可。撤销到被观察前可能已有 I/O；Drop 通知不提供 relay 退出或全部排空证明，未确认退出时保留未知 |
| B4 外部调用的薄参数/结果接线 | 固定 API/调用目标接收给定的原完整参数与稳定句柄；给定码的封装、成功值与错误如实传递，部分效果和未知不被抹去，修改范围及借用/所有权按合同传递 | 选择哪个对象、命令、SQL/Lua 文本和参数；错误/未知的业务解释及是否重试、提交、发布、确认或释放。动态外壳不包含自有鉴权、恢复或结果判断；有状态回调的次数/顺序/状态必须显式保留 |

B1 和 B3 的 `until_revoked` 已有下面的逐符号范围和条件模型；B2、B3 的其它操作及 B4 是获准登记的类别，只有补齐实际符号、合同和依据的使用点才能引用，不能把整类直接当作已验证适配。以上不是整文件/整个服务的可信白名单。每批列出实际函数及不可抽取语句，固定摘要，删去与本批无关的保证；有业务判断的同一函数先拆出真实同步核心。B1–B4 的源码审查、确定性测试和错误对照核对真实接线；若未形成机器精化，最终结论明确以该绑定符合合同为条件，不能声称所有 Rust 已获证明。接口签名仍含不支持类型时改用具体值/句柄/结果边界并重新预检；无法以窄接口和合理重构接通则报告阻断，不自动扩大工具开发。

进展另区分外部调用最终返回、运行时调度/队列公平和业务自身的启用/出队/预算/无环等待。前两项可按精确条件依赖 A/B，后一项仍由模块/V9 证明；公平不能使一个从未启用的业务步骤发生。

### A6 JSON 与标准派生的最小合同

固定 serde/serde_json 引擎及其标准 `Serialize/Deserialize` 派生可按实际类型与属性登记：保持完整字段/字节和值，不截断编号；字段名、顺序要求、默认/可选值、缺失/重复/未知字段、枚举/数组形状及错误消费状态都按该派生配置，而非一律假设未知字段会拒绝。只覆盖冻结的标准派生与读取/写出接口，不覆盖任意自定义 visitor、校验器或业务解码器。失败允许已消费输入或已输出前缀；有状态两次编码不假定同结果。

自有 `TryFrom`/`is_valid`、长度/预算、完整身份和参数绑定、接受/拒绝规则仍需实际抽取及证明；真实类型/属性到模型字段的接线由审查、合同样例和差分测试登记证据，未机器证明时显式保留该符合性前提。不能为绕过输入业务而直接假设“解码结果已获授权且合法”，也不要求先证明第三方 JSON 内部扫描/UTF-8/转义算法。其他容器/整数库模型仍按下文实际使用点单独冻结。

生命周期请求的具体 schema 还需保留固定派生的一个差别：NodeIdentity、ControlIdentity、Resources、CreateBinding、LifecycleRequest 的对象拒绝未知字段；内部标记的 Intent 中，Create/Destroy 使用的固定 unit visitor 会消耗并忽略可成功解析的额外字段，Renew 的结构体分支才拒绝未知字段。`deny_unknown_fields` 属性本身不能替代这个逐分支登记。输出仍按声明字段生成对象；标准结构体的序列输入按固定字段顺序处理。上述差异由原解码器回归核对，不改变生产策略，也不将忽略的字段解释为命令或授权。

[字段回调连接](../../SandboxModel/Boundaries/JsonRequest/Callbacks.lean)要求每个自有字段有原 Deserialize、原字典和输入的调用证据，再由已有真实叶子证明推出完整输入域；有限 value reader 仅证明条件可实现，不能提供实际 parser 绑定。[完整投影](../../SandboxModel/Boundaries/JsonRequest/Representation.lean)保留全部身份、创建字段和 intent，投影相等可推出原值相等。[编码字段规约](../../SandboxModel/Boundaries/JsonRequest/Encoding.lean)调用原 String/u64 转换；JSON 渲染、标准派生的字段接线及 Clone 仍按实际 A6 使用点落实。

[文档结束顺序](../../SandboxModel/Boundaries/JsonRequest/DocumentEnd.lean)按固定 `serde_json::de::from_trait` 先 Deserialize 再 end 的顺序组合；前面的叶子错误不执行后面的尾随检查，已有错误、失败、发散和效果保持。[字节组合定理](../../SandboxModel/Boundaries/JsonRequest/Codec.lean)显式接收字段字节关系及派生合同参数；[具体 SliceRead 连接](../../SandboxModel/Boundaries/JsonRequest/SliceBinding.lean)将其限定为本请求的原输入切片、原错误字典、六个编号与三个计数器回调。固定库的现场、标准派生组装及紧凑 Vec 渲染由 `SliceDeriveContract` 作为显式 A6/B4 条件提供；自有转换、校验、完整字段和守卫结论则由实际生成体的定理提供。该条件的采用依据是下述逐符号源/AST 登记与原生回归，不是库或异步外壳已经机器精化。

[容器结束与错误位置](../../SandboxModel/Boundaries/JsonRequest/ContainerEnd.lean)另区分内部收尾与最外层文档结束：固定 `deserialize_struct` 即使收到 visitor 的普通错误，仍执行 `end_map/end_seq`，可能继续消费空白；普通收尾错误不能覆盖更早的字段错误。原 `fix_position` 只在错误行号为零时补齐当时位置，外层不能再次覆盖已有位置。直接对自有编号/计数器调用 `from_slice` 时可能仍是 `(0,0)`；嵌入结构体后才由最近容器补齐。模型接收固定 SliceRead 提供的实际字节、偏移和位置，不另实现 JSON 扫描器，也不声称错误后解析状态回滚。

具体 `A6-json-lifecycle-derive` / `B4-json-lifecycle-call` 范围见[调用登记](../../verification/json-request-binding.json)。它逐项列完整字段/声明顺序、三个 intent 的固定行为、六个原编号和三个计数器字典、private SliceRead/Vec 的状态与错误、实际生产调用及独立预算来源。两个包装根由原 Charon 生成的完整 LLBC 和单次调用 AST 核对参数、结果、类型字典及 unwind 路径；[绑定检查器](../../tools/check_json_request_binding.py)同时固定生产源和十份库源，并拒绝隐藏守卫、替换类型/参数、增加调用或业务析构。它不翻译完整 Serde trait 图，也不是编译器、库或异步外壳的机器精化证明。`SliceDeriveContract` 到公共 `StandardDeriveRecipe` 的组合已由内核检查；这不会消除固定库符合该条件的可信责任。负向自动入口和原完整验收分别检查，不由登记文件存在推定通过。

正常分配域中，固定紧凑 renderer 对本 schema 的安全长度上界为 `2048 + 6 × 六个编号的 UTF-8 字节数之和 + 4 × options 字节数`。2048 覆盖固定键名、标记、标点、有界整数和 32 字节摘要；6 覆盖最坏字符转义，4 覆盖 u8 的三位十进制与分隔符。源/属性改变后必须重查这个界，不能沿用给新 schema。第三方 renderer 的完成及此结构长度保证属于明确 A6 正常分配条件；自有编号每项不超过 128 字节、options 不超过 65536，以及由此得到总长不超过原 270336 字节，则由自有输入域和原创建守卫的已证结论推出。所有合法有界请求有完整往返见证，初始请求有真正接受见证；不只证明拒绝安全，也不假设编码已经成功。物理 OOM/abort 或任意自定义序列化器不在该完成条件内。

本项纯值析构仅限普通 String、Vec<u8> 和 SliceRead/Vec 路径产生且不含自定义 IO 错误来源的库错误；原输入/请求为借用，parser 的可变游标与 scratch 由这次固定库调用私有持有。FD、许可、锁、任意自定义 Drop、panic/unwind 收尾、OOM/abort 和 async 取消均不在此域。真实服务器中 receiver、连接和任务所有权仍需其原模块提供条件，不能因 JSON 编解码采用 A6 而一并排除。

## Redis 与 SQLite 的落地合同

| 引擎/代码 | 外部合同与部署核对 | 自有待证连接 |
|---|---|---|
| Redis 7.2.16，[store](../../sandbox-center-store/src/store.rs)/[writer](../../sandbox-center-store/src/writer.rs) | 专用受保护IPv4回环端点、ACL、单有效写者和进程租约；冻结AOF always、noeviction、禁止截断加载及重写配置；恢复/错误语义按[中心持久化合同](../module-contracts.md#中心持久化与恢复) | READ/CAS、完整字节/修订、耐久确认、写围栏/reaffirm、缓存/outbox/确认/退休（V2），不能把整个RedisLedger当外部原语 |
| SQLite，[journal](../../controller-service/src/journal.rs)/[transaction](../../controller-service/src/controller/transaction.rs) | 候选锁定的rusqlite/libsqlite3版本，WAL＋synchronous FULL、文件系统同步合同、排他控制租约 | 同事务保存身份/输入/计划/额度/寿命/outbox；未知提交/缓存/检查点连续尾/退休（V5）；VM不在SQL事务中 |

Redis 脚本原子执行指不被其他请求插入，不保证任意多写脚本报错后回滚；当前实际脚本把检查放在唯一HSET之前，需V2证明这一事实。读成功只构造Observed；Committed要经过实际CAS与健康/耐久确认。未知写围栏不能靠再次读到旧记录解除，reaffirm须提交原完整payload的新修订后再派发。[Redis脚本](https://redis.io/docs/latest/develop/programmability/eval-intro/)、[Redis持久化](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/)

### A11/B4 Redis 提交结果接口的具体范围

[#564](https://github.com/VeriInfra-Lab/sandproof/issues/564) 的[隔离候选](../../verification/experiments/redis-result/README.md)固定原生产基线、补丁、回调入口、工具与完整生成摘要。以下是该候选的使用条件；当前生产 `RedisLedger` 尚未因此获得证明。`JsonCodec`、健康检查和返回码判断是自有代码，不能因使用 Redis 而移进 A11。

| 操作或对象 | 可使用的保证 | 不能由此推出的结论与自有义务 |
|---|---|---|
| A11：Redis 7.2.16 与 redis-rs 1.7.1 | 原受保护 IPv4 回环、ACL、单有效写者及冻结持久化/内存配置下，采用该版本命令和回复的合同。EVAL 执行期间不插入其它命令 | 不承诺外部调用总返回；错误不保证无写入或回滚。实际 Lua 的检查、参数和唯一写入顺序仍由 V2 提供；磁盘/文件系统及机器故障范围必须另有部署条件 |
| B4：候选 `storage_io::eval` | 将原 CAS 文本、单个 key、部署、格式、旧修订、新修订、完整新旧 payload 和限额按实际 KEYS/ARGV 顺序传给原连接；保留实际数字或错误 | `LibraryRequest` 只记录传给适配的参数，不表示发包、执行或落盘。选择文本/对象/字段、解释返回码以及确认/未知决定不属于机械适配 |
| B4：候选 `storage_io::info/settings/query` | 把实际 section、完整有序配置键和原绝对 deadline 交给同一连接；按原调用设置剩余读写超时，忠实传递回复/错误及连接回写 | 多次检查不是原子快照；超时设置不是整个调用的严格墙钟上界。失去响应、协议/类型转换错误或读取失败都可能在命令已执行之后发生；不能因本地连接被清空推导数据库没变或物理连接已经关闭 |
| B4：原 `diagnostics::emit` 经候选 `storage_io::emit` | 仅把原 Phase、Failure 和两个时长交给同一进程的原日志函数；正常返回不改写账本业务字段、连接或围栏。参数中没有账本、连接、载荷、租约或用户回调 | 允许写入失败、记录被抑制、阻塞、panic 或不返回；不保证日志写入成功、64 次内一定结束，亦不保证异常后已设置围栏。健康判断、拒绝先后和返回码仍是透明自有代码 |
| A6：候选 `trim_cr/split_colon` 与固定字符串/表操作 | 固定标准库的 UTF-8/字节接口；按第一个冒号分割、按原规则去掉末尾 CR，Lines 保留原 LF/CRLF 行语义；仅采用纯 String/str 比较和借用、唯一键字典及纯字节克隆 | 自有 INFO 扫描、重复键拒绝及配置选择仍抽取；任意自定义比较器、Borrow、Clone、格式回调或有状态两次编码不在本范围 |
| A8/B4：候选 `OperationClock/SystemClock` | 本次原超时配置传给原时钟，产生的 deadline 原样交给后续调用；回调状态在抽取结果中保留 | 不假设跨机时钟一致，也不从一个 deadline 推出调用一定结束；单位、溢出和更高层寿命仍由调用者负责 |
| B4：公开 `commit/reaffirm` 到泛型核心 | 实际薄转调传递原账本、原观察、原 next、`T::FORMAT_VERSION`、原 `JsonCodec` 和 `SystemClock`；完整返回与异常路径保留 | 完整公开 Charon AST 的源码/参数准入是审查和检查证据，不是具体 Serde 回调图的 Lean 精化；改变回调、参数、分支或字典必须重查 |

此日志合同由 owner 于 2026-10-20 采纳，见[具体提案](https://github.com/VeriInfra-Lab/sandproof/issues/333#issuecomment-6288404741)与[采纳记录](https://github.com/VeriInfra-Lab/sandproof/issues/333#issuecomment-6289377201)。候选增加的 `storage_io::Diagnostics` 在真实 Rust 中是空结构，不拥有文件、锁或连接；其模型中的后续结果和调用历史是对输出行为的逻辑表示，不是新增的 Rust 存储。健康检查允许推进这一逻辑历史，原 endpoint、config、租约、围栏及载荷标记保持不变。它不能证明日志内部原子计数、stderr 锁或输出格式正确。

[输出模型](../../verification/aeneas/storage/SandproofStorage/FunsExternal.lean)保留正常写入、写入失败但调用正常返回、抑制、panic 和不返回；历史只记录接口调用，不作为实际写入证据。本批透明健康拒绝只传 ConfigurationMismatch/UnhealthyPersistence 两种 Failure；其它查询诊断仍在原 `query` 适配内，沿用其可能失败或不返回的合同，不新增任意日志器的通用模型。`Duration::ZERO` 另按 A6 表示零秒、零纳秒，源码仍由原两版 `core/src/time.rs` 摘要核对。输出的失败和发散不会被换成业务成功。

[来源检查](../../tools/check_redis_diagnostics_binding.py)同时核对当前生产诊断文件原字节、完整移出的查询体、先清空连接再日志、CAS 的原 phase、三类健康调用的元数据与第一次拒绝顺序。它支持 B4 的采用，不是一般旧新等价证明。隔离候选、模型或任何实际调用发生变化时，重新核对这些连接与完整 LLBC/Drop/审计；不能只刷新原文件摘要。

原 LLBC 的析构逐位置登记在候选 `environment.json`：普通 String/Vec、已登记字典和自有载荷容器，仅在普通值域中忽略其内存释放；泛型 T 不得含可观察自定义 Drop 或共享资源。`fence_unknown` 对原 Option<Connection> 的替换另按外部 A6/B4 处理，**不把 TCP 连接析构说成纯内存操作**：实际关闭可能让对端继续处理已发请求或产生其它网络结果，物理效果仍未知。此域只允许原受控非 PubSub TCP 连接，不安装用户 push sender，不据模型中的 None 证明物理关闭、退出或回滚。写者文件租约及 codec/clock 原所有者均为借用，不在这组调用中释放。异常清理位置同样冻结，但正常后态定理不覆盖 panic/unwind/abort；若将来引入资源载荷、PubSub、用户析构或新关闭行为，须重审该域。

正常值模型允许数字、文本、配置表、调用前拒绝、送达未知、panic 与不返回。正常拒绝或错误返回不携带物理“未生效”保证；panic/不返回没有正常后态，不能引用正常返回的围栏定理来断言异常被捕获后仍安全。自有 V2 调用方须连接异常后的停用、责任保留和恢复；进程崩溃/abort 不依赖 Drop 收尾。

正常成功要经过实际 CAS 返回 0 及原后置健康检查；抽取定理保留部署、修订、完整载荷和解码后的状态。1/2/3/4 仅分别映射 Missing/AlreadyInitialized/InvalidRecord/Conflict，物理无写入仍依赖自有脚本证明；未知数字、EVAL 错误和后置健康错误进入 OutcomeUnknown、保留围栏并清空本地连接。读取观察不能自行解除该围栏，reaffirm 的成功必须保留原完整 payload 并使用新修订。

泛型定理显式保留两个自有条件：codec 的正常错误限于 InvalidRecord/InvalidState/Capacity，以及同一 codec 状态链上的成功编解码往返。它们不是 A6/B4 的保证，具体生产 schema 的实例由 V2 提供；没有实例就只是有条件结论。接口限于正常分配、有限可表示字符串/字节和普通值载荷；带许可、FD、锁、共享可变资源或可观察自定义 Drop 的载荷须另登记，不能套用纯值析构擦除。写者租约在本调用中借用而非销毁；身份模型也不提供物理锁/文件释放证据。

候选源、外部模型、公开 AST、完整生成物和真实 Rust 错误版均须绑定同一冻结版本。原工具的抽取、定理依赖审计及回归分别给出证据；局部通过不替代两个完整验收入口，不关闭整个 T7。完整 codec/Lua、读取/reconnect、缓存/outbox/派发与崩溃恢复组合仍由 V2 负责。

SQLite可恢复到提交前/后不等于外部效果未发生。同步承诺受实际文件系统/硬件合同限制；只有明确冻结的故障范围才可依赖，未约定断电行为仍排除。[SQLite同步配置](https://www.sqlite.org/pragma.html#pragma_synchronous)、[WAL](https://www.sqlite.org/wal.html)

### A4/B2/B4 SQLite 提交回执与缓存发布接口（隔离候选）

本条适用于 [#570](https://github.com/VeriInfra-Lab/sandproof/issues/570) 的[固定候选](../../verification/experiments/sqlite-commit/candidate.json)，不是整个 ControllerJournal 的可信声明。原生产基线为 `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`，候选只在私有副本应用[补丁](../../verification/experiments/sqlite-commit/candidate.patch)。原 WorldTransaction 的 commit_with/Drop 实际转调新增 memo 核心；核心生成物及条件性质位于 [SandproofSqlite](../../verification/aeneas/sqlite/SandproofSqlite/Results.lean)。完成与完整验收状态见覆盖记录及任务，不从本登记推定全入口已证。

| 接口及提供方 | 可以依赖的具体保证 | 条件和不能推出的结论 |
| --- | --- | --- |
| A4：rusqlite 0.40.2 / libsqlite3-sys 0.38.2 的原 Transaction::commit | 对原事务执行提交接口；正常 Ok 采用同一受保护数据库和声明同步合同下的事务提交含义 | 本候选实际链接的 bundled SQLite 为 3.53.2（3053002），与固定 0.38.2 源匹配；要求原 WAL、synchronous FULL、可靠文件/同步环境及原排他控制租约。成功不补造哪些行应当同事务，也不把 VM/I/O 纳入数据库事务 |
| A4：原 commit 的 Err | 允许提交前或提交后两种耐久结局，原错误值继续可见 | Err 不是未写入或已回滚证明；也不保证连接已回到 autocommit。A4 不负责本项目的缓存失效、恢复身份或是否派发 |
| A4：原 Transaction::rollback 及其内部结束处理 | 对仍持有的原事务调用原 rollback；正常 Ok 采用数据库的对应事务合同 | 失败不提供回滚完成、连接关闭或物理资源释放证据。公开 API 调用一次不意味着引擎内部只执行一条 SQL；内部结束及部分失败仍按 A4 解释 |
| B2：隔离 TransactionCells::set_committed(value) | 将给定原 Option<World> 写入原 committed.value；不选择值、不写 candidate 或 data_version | 两个单元与所属事务/外层 CommittedWorld 必须绑定，正常借用有效，操作的临时 RefMut 不跨数据库回调。逻辑模型记录原单元的值，未证明 RefCell 内部实现 |
| B2：隔离 TransactionCells::take_candidate() | 取走同一原 candidate 单元的值，将该单元改为 None，返回原完整值 | committed 与 candidate 是两个不同单元。调用期间没有冲突借用或并发/重入改写；不能将其推广到任意共享引用或可观察自定义析构。取值后临时借用较早结束的差异不构成任意程序一般等价证明 |
| B4：WorldTransaction::commit_with/Drop 的原参数、状态与回调接线 | 原 transaction 槽、上述两单元和完整回调交给同一真实 memo 核心，原 Rust 结果如实返回；Drop 采用原 Transaction::rollback | 正常生产提交回调固定为 Transaction::commit；测试丢回执闭包保持相同缓存修改范围。回调可以改变数据库，但不能改写这两个缓存单元。FnOnce 捕获值的析构也不得改写缓存；任意自有回调捕获共享缓存或额外资源时不得直接套用。T 为原事务句柄时，其结束/析构仍属 A4，不是可任意擦除的资源。需要单独证明或登记修改范围 |

**本项目仍要证明的决定。** 先给原 committed 单元传入 None，再取出原 live transaction 并调用给定提交回调；正常 Err 包装为 CommitUncertain，保持已清空状态；仅正常 Ok 后取出并发布原 candidate。rollback 核心仅在原槽仍有事务时调用回调，失败才给 committed 单元传入 None。None/候选的选择、错误解释、调用次序及原事务槽的变化保留真实生成函数，未移入 B2/B4。通用方程通过显式原语调用条件使用任意状态，具体双单元模型提供可计算实例，不增加自定义公理。

这些状态方程描述同步核心的调用边界。核心返回时的 candidate 值不承诺在外层 WorldTransaction 析构后仍有对应内存；失败路径可正常丢弃其候选。外层 committed 单元仍由原 CommittedWorld 拥有。实际字段析构、作用域和借用绑定须单独核对，普通 World 值域不提供许可/FD/外部进程的释放保证。panic/unwind、abort/崩溃、非返回、分配失败及未登记自定义 Drop 不从正常后态定理推出。

**V5 的自有条件。** live transaction、候选来源/正确性、所有前置校验、完整 SQL/参数/列、投影/额度/寿命/outbox 的同事务一致性、提交后读缓存、data_version/autocommit 判断、检查点和恢复仍由 V5 提供。缓存被清空不等于数据库恢复完成，也不授权重放或派发。主 commit() 的验证器调用图在本次原 Charon 采集中仍有分配失败；实际 commit_with/Cells/Drop 子图可以提取，不以它代替全入口已证。完整声明、实际调用绑定、正负用例及原完整门槛分别验收。

## TLS、传输库、来宾与工具链的登记

| 依赖 | 固定/检查来源 | 精确前提及自有责任 |
|---|---|---|
| rustls/tokio-rustls、证书验证和密码学 | [Cargo.lock](../../Cargo.lock)、[tls.rs](../../sandbox-internal/src/tls.rs)；当前0.23.45/0.26.6 | TLS1.3、证书链/有效期/用途和DNS证书名；关闭提前数据/会话恢复。安全算法为A9/A12，叶摘要→角色/节点、固定地址、私钥保护、错误处理归V3 |
| Tokio/Hyper/Axum/bytes、异步I/O | Cargo.lock及实际poll/await/select/drop调用 | A6的短读写、错误、关闭、取消语义及库队列上界；B1–B3 提供登记的薄接线，V8 等模块证明业务回调、预算/撤销/退出判断；取消future不等于外部进程退出 |
| serde/JSON、redis/rusqlite客户端、容器/整数模型 | Cargo.lock、实际derive/校验/命令、固定外部模型 | A6 的引擎/标准派生与库最小合同；自有大小/合法性、完整字段绑定、拒绝顺序及 SQL/Lua 业务桥接由V0/V2/V3/V5/V7证明 |
| Linux/文件系统/netfilter/pidfd/cgroup/进程 | 候选宿主版本、受保护目录/账号、稳定句柄与控制租约、运行预检 | A2/A3/A5，包含部分失败/对象替换/未知；对象选择、调用顺序/脚本与精确清理归V6 |
| 时钟 | [clock](../../controller-service/src/application/clock.rs)及中心Clock入口、候选时间同步配置 | A8需区分逻辑单调与跨机误差；V5/V7/V9证明单位、截止/重启及竞争，不凭单调时钟承诺现实时间SLA |
| 固定envd/SDK | [版本锁](../../references/v1-baseline.json)、[兼容合同](../../tests/compat/contract.json) | A7分开启动/完成、Connect会话、退出/子进程、文件完成和失败部分效果；V8忠实转发/原绑定，不证明整个envd/SDK |
| 模板服务 | [蓝图](../project-blueprint.md)和[模块合同](../module-contracts.md) | A10包含第三方工具及自有构建/管理/别名/登记/发布/产物服务，整体不证明；核心使用授权、不可变描述/长度/摘要/形状、获批绑定和清理仍证明 |
| Rust编译器、Charon/Aeneas、Lean内核 | [工具链说明](../../verification/README.zh-CN.md)、lean-toolchain/lake-manifest与抽取脚本 | 单列生产编译器/抽取前端/翻译器/内核信任角色和外部模型；不宣称任意Rust无损；V0/各模块补实际精化与生成摘要 |

[rustls官方说明](https://docs.rs/rustls/0.23.45/rustls/) 与 [Aeneas项目](https://github.com/AeneasVerif/aeneas) 仅作接口/工具范围出处，不是我们使用正确的证明。机器部署、库模型及翻译器可靠性也不是Lean内核检查自身能证明的事项。

### A6/B4 TLS 叶证书指纹接口

本接口为后续模块提供“给定原证书链和原 pin 列表，准确检查首叶并返回同一摘要”的公共结论。[正式模型与对应](../../SandboxModel/Boundaries/TlsPin.lean)直接导入固定原工具生成的两个同步 Rust 核心；[来源与范围](../../verification/tls-pin-environment.json)、[隔离重构补丁](../../verification/fixtures/tls-pin-candidate.patch)及输入摘要固定同一候选。**证明适用于该隔离候选；当前生产的借用入口尚未机器精化。** 原式到候选的一般 Rust 等价证明未交付，有限对照只作工程依据。

| 实际部分 | 可以使用的保证 | 条件及调用者责任 |
| --- | --- | --- |
| 候选 `verify_pin_bytes` | None、空链和不匹配准确返回 Authentication；成功当且仅当原第一张证书的完整摘要属于原完整 pin 列表；后续证书不改变这个决定 | 输入是完整有序字节序列，pin 保留全部 32 字节；不预先假设“已获授权”。实际角色、节点和登记来源由 V3/V7/V8 提供 |
| 候选 `leaf_pin_bytes` | None/空链准确拒绝，否则返回原首叶摘要 | 本函数单独不授权。外层必须先使用同一原链和原 pins 通过检查，不能只调用返回函数 |
| `CertificateDigest` 与 `Sha256CertificateDigest` | 核心显式调用传入的摘要接口；在 `DigestCorrect` 条件下，正常返回参考摘要 | A9/B4 绑定实际 SHA-256 和原完整输入，保留输入只读及正常返回条件；不假定全局摘要单射，不覆盖任意有状态或自定义实现 |
| 候选 `certificate_chain_bytes` | B4 条件要求保留 None/空链、全部元素、顺序和每个原字节；pins 不截断、不换对象 | 这是逐符号可信复制接线，尚未机器精化。两次复制均须来自同一原 TLS 连接的原链，不能以客户端请求头或别的连接替代 |
| 候选 `verify_pin`、两类 `verify_peer`、`ServerTls::peer_pin` | B4 条件要求原参数/字段转交、先验证、原样传播拒绝，再返回摘要；成功路径仍两次哈希 | 外层没有被自动抽取；`linkedPeerPin` 是两个已证核心的条件组合，不能冒充外层源码的机器精化。两个完整快照和 pin 列表的表示关系须分别落实 |
| 原 `ProtectedServer::serve`、客户端 `exchange_bytes/data_connection` | 采用 A12 的实际握手、证书验证、连接字节与错误合同；核对 pin/ALPN/HTTP 的原调用顺序 | 证书链、时间、用途、DNS、TLS1.3、禁用提前数据/恢复等配置分别绑定；握手成功不自动证明节点、请求或当前业务权限正确 |

`RustPinSafe` 精确连接实际生成的授权核心与上述全输入策略；`linked_matches_original` 再连接两份原完整快照和原 pin 列表。`peer_acceptance_iff` 给出成功时的首叶、返回值和成员关系；接受/返回见证排除“总是拒绝”的空结论。摘要接口自身的失败/发散在核心行为等式中保留，不伪造 Authentication；正常功能结论额外使用明确的摘要正常返回条件。

固定 A6 使用点为 slice 的 first/contains、Vec 的 deref、完整 u8 数组相等及结果操作；只有 first 需要本包的有限模型，其余复用原 Aeneas 库。该模型返回原列表的第一个值或 None；它不是新 Rust 解释器，也不证明实际标准库本身正确。自有比较和两个业务函数必须保留真实生成体，不允许整段 opaque。两个抽取体不包含可观察自有 Drop 或循环；正常字节复制/释放依赖原库合同，自定义回调、额外许可/锁/FD、panic/abort 和物理退出不自动获得同样结论。

候选增加了复制和分配。采用前须明确正常分配、连接数及字节和容器开销预算；固定 rustls 的证书载荷限制不能代替整体堆内存证明，也不能把本地 identity 的八张证书限制套给任意 peer。分配失败或 abort 不算鉴权拒绝，更不提供完成析构/排空证据。真实握手已可能收发网络字节，核心拒绝只约束本判定，不声称此前没有 I/O。

[验收入口](../../scripts/verify-tls-pin.sh)核对源/选项/完整 LLBC/原样生成体、逐符号调用和全定理审计，在独立副本运行原工具、真实 SHA-256、实际 TLS 回环与同一性质的首叶→末叶错误版。错误版必须先编译和抽取，再由原断言及 Lean 反例拒绝；结构防漏测试不冒充程序证明。变更补丁、原 source/pins/配置、摘要接口、库模型或工具时，重新抽取、证明、核对绑定及受影响调用；不能只刷新摘要。真实生产采用、全局授权、进度和部署仍按各模块交付，T8/V0 不因本接口自动完成。

## 失败时怎么把控前提

预检可证伪版本/配置/权限/预算不匹配；运行观察可证伪原资源身份或健康，二者都不能证明所有外部合同永远成立。可失败观察统一保留Unknown/Error。库/配置变更时登记哪些R_i和系统结论失效并重查依赖闭包；不能沿用旧摘要/旧测试自动背书。

进入证明批次先由V0维护合同清单、各模块认领使用路径、V9审查系统使用的前提集合。最小受控失败和真实部署核对沿用现有测试，不为了“证明库可靠”增加无限黑箱测试。获批 B 合同逐项显式使用，不要求先完成通用异步翻译；未建立的自有 Lua、规则字符串或业务决定仍保持未解，不借 B 合同消除。

### 真实使用点的来源清单

[入口清单](../../verification/use-inventory/config.json)固定中心、宿主、请求窗口辅助进程、来宾服务与 worker，以及额外保留的 VM 运维入口；分别编译开发和普通发布配置。前者开启调试断言，后者关闭，两者保留溢出检查。SDK/Redis 测试、诊断特性、示例和非 Linux 配置不计入这份生产源码清单；启用新的部署特性后必须补采。模板函数即使不属于证明范围，也不从混合 crate 的编译证据中删除，T9 仍不排除 Guest 的安全责任。

额外运维入口只作防漏观察，不新增 V0 或业务模块的证明前置；哪些行为需要证明仍以 V0–V9 详细计划及实际生产调用为准。编译清单中的死代码、模板代码和辅助入口不能自动变成新的证明目标，也不能反过来借“辅助”标签排除真正被服务调用的函数。

[采集工具](../../tools/v0_use_inventory.py)仅给原生产编译器增加 MIR 输出，不改生产 Rust、Charon/Aeneas 或翻译规则，也不把 MIR 送进新的解释器；本批产物只作来源审计，不用于发布。完整原文、实际 Cargo 产物/参数、签名、局部类型和终结语句保存在本轮忽略的证据目录；[登记结果](../../verification/use-inventory/catalogue.json)及[输入摘要](../../verification/use-inventory-inputs.sha256)绑定本轮来源。用 Cargo 实际产物逐项查漏，不能用几个函数计数代替完整输出。`query` 按原文定位调用/类型，搜索没有命中不证明路径不存在。

[责任清单](../../verification/use-inventory/responsibilities.json)给每个自有包和本轮观察到的自定义 Drop 指定复核归属，**不是新增可信白名单**。归还计数、锁下修改、标记 Unknown、选择取消、转交子进程和是否释放仍是自有业务；只有具体登记的 A/B 原语及机械接线可以作为条件。清单中“尝试解锁/发送/终止”不能改读为这些动作必定成功，也不能推出工作已经退出。

这份清单是源码与编译来源审计，**没有证明调用图闭合或新的 Rust 语义**。稳定 MIR 文本不能单独补齐动态回调实际实例、结构体全部字段/运行时合法域、future 在每个挂起点被取消时的生成清理、嵌套库析构及外部可执行脚本的完整使用绑定；这些缺口在责任清单继续归 T1–T8 和业务模块，不能因为有 MIR 就勾选整个 T0。正常 Drop、实际发生的 unwind、Drop 自身 panic、abort 和进程崩溃仍分开，后两者不保证执行 Drop。

可复现入口（固定 Linux 证明环境；完整日志不提交）：

```sh
bash scripts/python-entry.sh tools/v0_use_inventory.py capture artifacts/use-census
bash scripts/python-entry.sh tools/v0_use_inventory.py check artifacts/use-census
bash scripts/python-entry.sh tools/v0_use_inventory.py query artifacts/use-census --contains OwnedSemaphorePermit
```

`publish` 在完成来源/责任复核后生成登记结果和摘要，属于审查更新，不能用它接受未经核对的变更。`verify-use-inventory.sh` 只做已登记来源的快速准入，不冒充重新编译或 Lean 证明。已有真实核心仍执行原抽取和完整保留证明验收；来源清单不扩大任何已证明范围。

### A6 公共基础库模型使用登记

`A6-foundations-stdlib` 只覆盖抽取前端 `nightly-2026-08-18` 中九个标准库函数：`Option::as_ref/is_none_or/is_some_and`、Option 的 `Try::branch`/`FromResidual::from_residual`、`Result::ok`、`String::eq`、`Vec::is_empty/retain`。前提是有效、有限的 Rust 值/借用及容器表示；正常返回分别保持可选值、精确传播残余、将 Result 的成功值投影为 Some（错误为 None）、比较完整字符串、判断长度是否为零。这些库接口不自行构造提交或物理证据，不提供 Ready/Cleaned/Committed，也不把外部未知转换为明确失败；用户回调的失败、效果或发散必须原样传播，具体回调语义由调用方证明。`Result::ok` 不保留错误原因，不能据此区分未执行、明确失败或结果未知。失效后不得继续使用依赖它们的资源加法、身份判断、回执检查、观察、窗口后继查询和创建参数对应。

可计算模型位于 [FunsExternal](../../verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean)，精确源码/模型/脚本摘要及编译器、翻译器、Lean版本、目标和配置统一见[冻结清单](../../verification/foundations-environment.json)及其输入摘要。原语模型与实际标准库的协议一致性是显式外部条件，不因无占位或内核检查而变成已证明的标准库实现正确性；这些自有协议函数本身仍从生产源码抽取。内置整数、Result、Vec/Array 与容器模型由同修订 Aeneas 后端提供，同样属于工具/库可信链，不能遗漏。

使用方：资源加法及创建参数守卫依赖 Option/Vec/整数模型；完整身份相等依赖 String/整数模型；结果观察依赖 Option 借用模型；`RetentionLane::current` 依赖 Option/Result/整数模型，并另证其成功后继非零，None 精确对应编号上限。部署不包含 Redis、SQLite、TLS 或宿主调用；它们不成为这一批的隐含前提。版本/模型/配置或源函数变更使输入摘要失配，重新抽取并重查这组对应定理、调用它们的邻接合同和未来 V9 实例化；不能只更新摘要后沿用旧结论。完整目标的自有编解码与 Lua 桥接仍分别由 V0/V2/V3 提供，未移入本合同。

`Store::get_copy/put` 另使用同版 Vec 索引、可变索引回写、push、usize 运算和相等字典模型，不新增外部函数。循环终止、首次匹配、首次替换/缺失追加、完整列表保持及其他键查询不变是自有证明。泛型 `TableLawfulEq` 是调用模块需提供的相等对应，u64 实例已经连接实际内置字典；其他自有键比较不得整体放入 A6。写入模型保留顺序和重复行，唯一性只在原键唯一的前提下保持。替换不需追加余量；缺失追加需原长度小于 `Usize.max`，由调用模块提供，不假定实际物理内存无限。固定纯 Vec 模型仍不覆盖物理分配失败、OOM/abort；指纹变化须重查读取及写入闭包。这些定理不证明删除、业务额度、退休许可或 SQLite/Redis 持久提交。

`Option::is_none_or` 的共享定义和 [OptionPredicates](../../SandboxModel/Foundations/OptionPredicates.lean) 区分 None 不调用回调、Some 恰调用原 FnOnce，并原样传播失败/效果/发散；缺失值返回 true 不代表权限或物理证据。调用方还须将回调捕获状态限制在已登记的纯值/析构擦除范围，不覆盖自定义 Drop 的外部副作用；不能由 None 没有调用 FnOnce 推出 Rust 没有析构捕获值。真实 u64 探针根只连接标准库调用，不等于 V4 的创建守卫已实例化。

`Option::is_some_and` 使用相同的 FnOnce 合同，但 None 返回 false、不调用回调，Some 恰调用一次原回调，保留其失败及结果。公共 `WorkReceipt::validate_for` 的版本、完整身份和观察资源检查从真实生产函数抽取；观察的 `ResourceEvidence::resource` 字典须由具体观察类型的提供方证明，不能把任意自有 getter 归入 A6。没有观察只表示无需资源比较，不表示成功、停止、清理或外部效果不存在。析构和可变外部效果的限制与 `is_none_or` 相同。

`Vec::retain` 增加有限有序列表的可计算模型：每条原记录按原顺序调用一次 FnMut，更新的回调状态传入下一条，布尔结果决定保留，保留记录内容和顺序不变；空表不调用回调。自有 `Store::retain` 及其包装闭包保留实际生成体，由 [TableRetention](../../SandboxModel/Foundations/TableRetention.lean) 对应独立纯/状态化筛选模型。调用方必须证明自己的回调字典正常返回且符合给定状态转换，不把自有谓词正确性放入 A6。此合同只覆盖有效有限值的逻辑正常返回，不覆盖内含可变引用的外部堆效果、用户析构副作用、panic 后部分表的回滚或物理内存释放；回调失败不能伪造成功，也不据此保证表未部分修改。纯谓词的幂等性不推广到有状态谓词。

两版真实标准库 `alloc/src/vec/mod.rs`、`core/src/option.rs` 摘要单列在同一环境清单并在工具审计中实查。源码审查、状态化回归和摘要不代替标准库形式化证明；这是有限库接口的显式合同，不是把自有退休/删除许可作为可信前提。源版本或模型变化须重查筛选、Option 判断、其 V4 调用字典及邻接证明闭包。

### A6 身份字符串与规范编号库模型使用登记

`A6-foundations-identifiers` 增加 14 个精确库接口，连同上述 9 个合为 23 个不透明函数模型；实际清单由[抽取审计器](../../tools/check_foundations_extraction.py)检查，不允许把自有身份构造器、谓词、闭包或中心解析包装改为不透明调用。模型分为：

| 库观察 | 合同和调用边界 |
|---|---|
| `str::len/is_empty/bytes`、`u8::is_ascii_alphanumeric`、`Bytes::all` | UTF-8 字节长度和原字节顺序；ASCII 判断；回调按序、短路且保留更新后的状态，失败原样传播，回调本身由 V0 证明 |
| `String::deref`、`str::to_owned`、字符串/借用字符串相等 | 有效且长度可表示的 UTF-8，逐字节保持内容；复制正常返回不改变文本，不采用有损 UTF-8 解码，不保证物理分配一定成功 |
| `str::split_at_checked`、`str::parse`、`u64::from_str` | 只在范围内的 UTF-8 边界切分，其他切点返回 None；实际 u64 十进制解析接受前导 `+` 和零，但只接受 ASCII 数字并拒绝溢出。错误原因仅在本项目随后 `.ok()` 的观察中商为“解析失败”，不能用于保留 ParseIntError 细节的其他调用 |
| `ToString::to_string`、`bool::then_some`及固定 u64 默认 Display | 默认十进制输出无符号、无前导零；ToString 调用原 Display 并读取其输出，不能忽略回调后伪造成功；then_some 保留给定值。本批仅连接实际 u64 默认实例，不覆盖自定义格式参数或用户 Display |

可计算实现见 [IdentifierLibrary](../../verification/aeneas/foundations/SandproofFoundations/IdentifierLibrary.lean)、[TypesExternal](../../verification/aeneas/foundations/SandproofFoundations/TypesExternal.lean) 与既有 FunsExternal。Bytes 保存尚未消费的字节；格式缓冲区是明确的输出字符串，不使用无观察内容的类型替代。UTF-8 字符串/字节往返引理由 Lean 内核检查；这些定义与固定 Rust 标准库符合仍是显式 A6 条件，不等于标准库本身已证明。

内部错误文本子树另使用默认 **String-backed** `Formatter::write_str`：在有效 UTF-8、可表示的字符串缓冲及正常分配域内追加完整文本；`ToString` 仍调用原 Display 并保留其结果，外层失败继续传播，Display 返回格式错误时对应原 panic。模型不保证任意 `fmt::Write` 设备成功，不声明物理 OOM/abort 可恢复，也不覆盖格式标志、浮点、Serde Error/Expected 或最终 JSON 错误行列。真实自有 `InternalError::fmt` 的三分支及文字由生产源码抽取，不能以库合同假定它们正确。无效 UTF-8 的数学 Str 位于合法 Rust str 域外，模型返回外层失败，不将其改成成功的空文本。此接口属于固定内置模型绑定，原 23 个非内置函数的清单不减少。

本模块的字面量构造包装仍委托同版 `Aeneas.Std.toStr`，保持同一文本、字节及 U32 长度前提，只显式传入内核检查的范围证明，不使用其默认本机求值证明参数；不得手改生成代码或放宽公理审计。新增其它字面量或模型引用后须重新检查实际生成定义、范围证明及依赖闭包，不能以包装自身无公理代替调用根的审计。

两版标准库的字符串、字节迭代、解析、布尔、格式及 `Result::expect` 源码摘要在同一环境清单登记并实查。`ToString` 的格式错误经 `expect` 进入原 panic，仅对应失败类别，不声明 panic 消息、栈展开或进程恢复。生产/抽取编译器、目标、翻译器和 Lean 未改变；标准库来源、模型或默认格式实例变化后，重查构造/取值/规范编号、完整身份输入域及其调用闭包。调用方还须建立有效 Rust UTF-8/有限值、成功分配和后端表示条件，不从纯正常返回定理推出 OOM/abort 可恢复。

V0 证明自有 `valid_identifier`、八类 String/&str 构造和取值、协议规范编号解析与中心实际包装；不是将这些自有行为作为外部假设。中心恢复中完整 `CenterState::is_valid`、存储身份恢复、JSON 自有字段访问及全部调用资格仍分别属于未闭合连接。Serde/serde_json 的整套编解码不纳入本合同：将 `from_slice/to_vec<ControlIdentity>` 整体设成不透明，即使翻译成功也不能计作自有字段往返已证。

### A6 v3 同步协议的字符串与完整字节视图

本登记限 [v3 的三组实际抽取根](../../verification/experiments/stage1-protocol/src/lib.rs)。
Carriers 没有新增外部函数；Request 只使用完整 String 相等和 Vec::is_empty；
Manifest 的准确七项为 String::eq/deref、Vec::is_empty、str::len/is_empty/as_bytes、
str::to_owned。自有请求、路径/profile、文件名/数量/总长和 capture 绑定检查保留真实生成体，
不能把这些决定作为库假设。精确原始 IR、外部清单、完整导入和源摘要在
[模块冻结](../../verification/experiments/stage1-protocol/frozen.json)检查。

除 as_bytes 外，命名适配只转发已有固定 A6 字符串/容器模型与原内核检查的字面量构造。
新增的 `str::as_bytes` 表示连接要求：输入是有效、有限且长度可表示的原 Rust str；
在原借用有效期内返回完整 UTF-8 字节的只读视图，保持长度、内容和次序，不做有损解码、
规范化或截断。后端 Str 本来用同一 Slice 表示，所以可计算模型是 `.ok value`；
**模型与固定 Rust 原语符合仍是显式 A6 条件**，无公理审计不能把它变成标准库实现证明。
该普通值视图不携带 FD/许可，不授权延长借用、修改原字节或擦除可观察 Drop。

普通分配成功、panic/abort 与工具链信任沿原 A6/Aeneas 范围；不增加“库永不失败”保证。
32 字节 ContentDigest 构造仅保存值，不执行哈希。原受保护 profile、文件句柄/内容摘要、
捕获截面、同步发布及恢复资格仍由 V6/V5 提供，路径文本合法不等于文件系统路径安全。
新 codec 的 serde/Writer 调用及预算/首错顺序尚未由本组件证明；既有 A6/B4 和 Writer 结果
只能在实际 schema/原对象/输入/限额连接完成后使用，不能整体信任自有 codec。
依赖、生成体或上述表示条件变化时，使本模块及实际调用者对应结论失效并重查。

### A6 JSON 非零计数器叶子接口

这组根并入原 V0 抽取，与已有身份/数值共享同一份生成 Rust 类型；实现对应和原公共定理可以在同一 Lean 环境组合，不维护第二份同名类型。

`A6-json-counter-leaf` 只覆盖 [PositiveCounter](../../sandbox-protocol/src/internal.rs) 的真实泛型 `Deserialize`、原 `TryFrom<u64>` 和有限错误分支。它依赖已有显式闭包重构，未新增生产改动；不将整个 `from_slice<ControlIdentity>` 或自有校验设为不透明接口。实际 `u64::deserialize`、具体整数 visitor、校验、错误闭包及 `InternalError::fmt` 均保留透明生成体。三个[抽取根](../../verification/extract-foundations/src/json.rs)直接调用原实现；错误透传根仅登记引擎错误的表示，不认证其内容。

解码器和错误字典是显式参数：整数解码成功后才运行非零检查；非零值原样返回，零走原 `custom(InvalidCounter)`，已有引擎错误、模型失败、发散及外部效果按原顺序传播。`custom` 允许失败。这是生成函数与独立后处理规格的对应，不是 JSON 扫描器、完整字节入口或可变 parser 已验证。具体引擎须落实正确的 u64 读取和原输入/状态/错误/字典绑定，不能先完整读取并检查尾随字符再检查非零；真实 `0 x` 数据错误与 `1 x` 尾随语法错误由引擎回归核对。

[类型模型](../../verification/aeneas/foundations/SandproofFoundations/TypesExternal.lean)提供普通错误观察和默认字符串格式化的有限表示；[写入操作](../../verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean)保持有效 UTF-8 的完整字节和缓冲追加。仅本批固定 ASCII 字面量使用此路径，不覆盖任意 Formatter 配置、用户 Display、错误 source 链或物理分配失败。库符合性仍是显式 A6 条件，不因内核检查成为库内部实现证明。计数器的真实 visit_u64 覆盖默认实现；字符串叶子新增的四个有限默认方法模型按下一节登记，不导入模板公理。

固定 preset 仍擦除 Drop。两个泛型包装根和错误透传根共七处条件清理逐项登记于[环境清单](../../verification/json-counter-environment.json)，完整 AST 摘要保留其位置、类型、目标及 unwind 内容；准入检查拒绝业务析构或未登记变化。**本接口不覆盖 D/E 的可观察自定义析构、许可/FD 持有、panic/unwind 收尾或 abort/崩溃释放**。普通借用解析器及库错误对象的表示/析构条件由使用点核对；泛型 Lean 定理不背书任意带资源 Rust 解码器，外部效果传播也不是 async/取消证明。

固定 serde/serde_core 1.0.228、serde_json 1.0.142，实际库源码、Charon 选项、完整 LLBC、声明和条件清理见环境记录。工具/两版编译器/cfg 复用[Linux 工具登记](../../verification/first-chain-linux-environment.json)，全部输入由[接口摘要](../../verification/json-counter-inputs.sha256)冻结。调用方仍须落实字节/大小预算、schema/属性及其他自有叶子、完整请求/身份、版本/节点/槽位、授权及业务拒绝顺序。源文件、依赖、模型、工具或配置变化使本模块和使用方失效，须重新抽取检查，不能仅刷新摘要。

### A6 JSON 字符串编号叶子接口

`A6-json-identifier-leaf` 覆盖 TenantId、SandboxId、OperationId、LogicalActionId、ResourceId、ClientRequestId、DeploymentId、NodeId 的实际泛型反序列化接线。它复用 #481 的显式闭包候选和原 V0 身份构造证明；这一批不新增生产 Rust 改动。九个[直接调用根](../../verification/extract-foundations/src/json_ids.rs)与原 V0、计数器共用生成类型。自有 Deserialize、TryFrom、校验器、错误闭包和 ContractError::fmt 均保留原工具生成的透明函数体。

给定解码器读出的字符串，合法编号原样接受，不裁剪、不截断、不规范化；非法编号走原 custom(InvalidIdentifier)。成功值满足非空、UTF-8 字节长度最多 128、字节属于 ASCII 字母/数字或 `-_.`。已有错误、外层失败、发散和外部效果保留。该结论验证自有后处理，不从数学值读取器推出 JSON 字节引擎正确，也不证明任何授权或完整请求绑定。invalid_type 是显式错误字典参数；本批未实例化 Error 的通用默认实现，后续若直接引用未实现的默认函数仍须补充库连接，不能将泛型定理称为 from_slice 的完整单态入口证明。

实际 String::deserialize、StringVisitor 的 expecting/visit_str/visit_string 由原 Charon/Aeneas 抽取；拥有字符串原样返回，借用和临时字符串按有效 UTF-8 复制完整文本。原后端在默认方法构造 Expected 字典处无法消除递归引用，因此四个 Serde 库默认方法在 [FunsExternal](../../verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean) 用有限模型登记：visit_borrowed_str 转发原 visit_str；visit_string 先借用再转发；默认 visit_str/visit_u64 调用原错误字典的 invalid_type，携带原输入值及原 expecting 回调，然后返回其错误。所有回调保持效果和失败；这些模型与固定 Serde 源码一致属于 A6 条件。没有给自有校验写替代实现，也不假定错误回调总成功。Unexpected::Float 的 F64 仅以 64 位载荷表示，不提供浮点运算、转换或格式化保证。

默认 String-backed 格式化另外登记共享引用 Display 的转发、u16 默认十进制输出、Argument 对原回调的捕获，以及唯一冻结的 Arguments::new 调用形状：29 字节文字 `unsupported protocol version ` 加一个默认 u16 Display 参数。模型保存格式字节及参数，write_fmt 恰执行原参数回调一次，保留其返回结果和效果；不使用后端丢弃全部格式内容的简化实现。其他格式形状不在合同域内，准入检查固定真实调用点；不推广为通用格式化器。正常分配、有效 UTF-8、缓冲可表示条件沿用现有 A6，不覆盖任意设备、格式标志、OOM/abort 或 panic 收尾。

接口继续受固定 preset 擦除 Drop 的限制。泛型调用根的条件清理，以及自有构造/解码路径实际调用的标准 String DropGlue 均逐项登记；后者只按正常返回域中的标准堆释放处理，不证明释放效果或物理内存用量。准入检查核对其确为外部 String 清理，拒绝改为业务析构。实际 D/E/visitor/回调的使用点必须无本接口忽略的可观察自定义析构和资源持有语义。本批不是 Future、取消、许可或 Drop 证明。完整 JSON 扫描、尾随字节处理、复合 schema、编码往返、实际输入/状态/字典绑定以及业务拒绝顺序仍由具体使用点落实。有限值读取器只作合法接受和拒绝的非空见证；真实 JSON 回归覆盖未转义、转义及拥有字符串分支和首错顺序，回归不代替引擎证明。

独立 [IdentifierSafe](../../SandboxModel/Boundaries/JsonIdentifiers/Model.lean) 复用[公共编号词汇](../../SandboxModel/Foundations/IdentifierModel.lean)，不导入自有构造正确性证明，允许在真实校验缺失版重新抽取后用同一性质给出反例。源码/依赖/模型/工具/配置改变后重查整个共享 V0 和计数器闭包；实际证明状态和版本只见覆盖记录，不以刷新冻结摘要替代复验。

### 完整生命周期请求的自有形状校验

[请求守卫合同](../../SandboxModel/Boundaries/JsonRequest/Model.lean)区分三个来源：JSON 引擎/标准派生解释字段；原编号和计数器 Deserialize 检查叶子；原 `LifecycleRequest::validate` 检查版本、完整节点、非零槽位、创建参数和 Renew 非零结束时间。最后一层仍保留真实函数体，[对应证明](../../SandboxModel/Boundaries/JsonRequest/Guard.lean)复用原身份及创建参数结论，不属于可信 JSON 适配。

该守卫先返回原身份拒绝；身份通过后才检查创建绑定与 intent 形状。成功只表示这组检查成立，不表示请求已获授权、续租没有超过原寿命、存储已提交或宿主已执行。`retention_slot=0` 属于可解析的 u16 输入，由守卫拒绝；输入总字节数由调用路由的 Limited 提供，不能归给 `from_slice`。调用者仍须建立实际字节/schema/原叶子和错误字典的连接，以及后续授权和生命周期条件；GuardContract 不提供这些结论。

### A6 网络地址库模型使用登记

`A6-network-stdlib` 是独立于上述六函数的最小 IPv4 标准库合同，覆盖真实调用的 `String` 精确相等、`String` 到 `str` 借用、`str::parse` 的字典调用、`Ipv4Addr::from_str` 及 `u32::from(Ipv4Addr)`，以及所需的 `Ipv4Addr`、`AddrParseError` 表示；下方克隆与析构登记限定新增的两个接口及内置模型使用条件。生产编译器为 Rust 1.97.0，抽取前端为 `nightly-2026-08-18`；两版标准库解析器的实现和文件摘要不同，不将抽取版源码等同于生产版源码。精确编译器提交、两版库源码摘要、目标及配置冻结在[网络模型环境清单](../../verification/network-environment.json)，源码/模型/检查脚本摘要见[网络输入清单](../../verification/network-inputs.sha256)，生成物由重新抽取逐字节核对；变更后重新核对依赖闭包。[生产版解析器源码](https://github.com/rust-lang/rust/blob/2d8144b7880597b6e6d3dfd63a9a9efae3f533d3/library/core/src/net/parser.rs)、[抽取版解析器源码](https://github.com/rust-lang/rust/blob/8fa1c96cfd489e4c27654c144ae871ce2c4db6c6/library/core/src/net/parser.rs)

调用前提是有效、有限的 Rust 字符串和借用；表示关系显式要求字符串字节长度不超过目标 `Usize.max`（`StringRepresentable`）。模型对超出 Rust 可表示域的 Lean 字符串返回外层大小错误，不把它当成正常解析失败。固定 Aeneas 的 `Str` 表示是字节切片，模型保持原始 UTF-8 字节，不先去除空白或替换字符；精确相等不作域名规范化。借用保持完整字符串字节，泛型 `parse` 忠实调用给定的 `FromStr` 字典。IPv4 解析成功恰对应四个非空 ASCII 十进制段、三个 ASCII 点、完整消费输入；每段 1–3 位且数值在 0–255，除单个 `0` 外拒绝前导零。空白、符号、非 ASCII 数字、少段/多段、超范围、端口、CIDR 后缀、IPv6、域名和其他尾随内容均返回解析错误，不隐式规范化成可接受地址。对本前提内的输入，正常语法失败是外层 Aeneas `Result.ok` 中的内层 Rust `Result.Err`，成功是外层 `ok` 中的内层 `Ok`；两者不混同于执行失败、提交未知或物理网络结果。

地址表示保持四个字节的数值：`a.b.c.d` 转为 `a × 2^24 + b × 2^16 + c × 2^8 + d`，`u32::from` 正常返回该整数，等价于源码中的 `u32::from_be_bytes`，不随目标平台端序交换字节。例如 `1.2.3.4` 对应 `0x01020304`。`AddrParseError` 当前只被 `Err _` 分支消费，可以用单构造的可观察商表示；不覆盖错误的相等、Debug/Display，也不覆盖其他地址类型的解析。新增这些观察或调用后须扩展表示与合同，不能沿用错误被忽略时的模型。[生产版地址转换源码](https://github.com/rust-lang/rust/blob/2d8144b7880597b6e6d3dfd63a9a9efae3f533d3/library/core/src/net/ip_addr.rs)

真实标准库符合该可计算模型是显式外部条件，源级审查和边界对照测试不是标准库实现的形式化证明；不以自定义公理或不透明的成功模板消除自有义务。解析器接受 `0.0.0.0` 或私有地址也不意味着允许访问。自有 CIDR 掩码及合法性、固定平台禁止表、解析失败和非单地址请求的拒绝、平台禁止 → 用户允许 → 用户拒绝 → 默认联网值的短路顺序，仍由本批自有实现对应证明提供。真实宿主规则安装、动态宿主地址排除、全部出口覆盖和授权/当前实例绑定属于后续模块连接，不置于本库合同。本合同无 I/O、DNS 查询、规则安装或 Ready/Cleaned/Committed 证据，不扩大为整个标准库或网络实现已验证。

#342 的 Linux 回执检查及中心分类单独登记 [FunsExternal](../../verification/aeneas/first-chain/SandproofFirstChain/FunsExternal.lean)：字符串比较和 Option 共享借用沿用 V0 的完整相等/借用合同；另提供固定前端的 Option 逻辑枚举标签模型，None 为 0、Some 为 1。标签模型描述编译器中间表示，不承诺原生内存布局。Option 相等函数本身从同版 Rust 标准库抽取真实函数体；u64 以及完整回执的实际相等字典分别由对应定理连接，不用外部相等公理。中心事实等级、协议事实/回执相等和判定算法保留自有透明函数体；错误映射使用生产代码内的显式匹配，共享引用相等使用固定 Aeneas 后端的具体模型。库标签/字符串/借用、内置模型及翻译器的剩余可信责任仍显式保留，不能称标准库或编译器已验证。版本及摘要见 [Linux 环境清单](../../verification/first-chain-linux-environment.json)与[输入摘要](../../verification/first-chain-linux-inputs.sha256)；V0 环境由其冻结快照接入，不静默覆盖原记录。

## 精确条款

第一阶段续期确认/回收根复用已登记的 Option 相等和 Result 判别：
[请求命名空间适配](../../verification/aeneas/stage1-protocol/request/SandproofStage1Request/FunsExternal.lean)
的 `core.option.Option.Insts.CoreCmpPartialEqOption.eq` 直接委托网络库模型，
先比较 None/Some，仅 Some/Some 调用给定的完整元素相等；本调用的 U64/I64 字典由具体引理连接。
`core.result.Result.is_err` 使用原 [SQLite 模型](../../verification/aeneas/sqlite/SandproofSqlite/FunsExternal.lean)
相同的两构造分支（Ok 为 false、Err 为 true）；因两个生成类型环境的辅助声明重名，未共同导入。
它仅判别纯请求校验的 Rust Result，不把 Err 解释为事务未执行，也不提供提交或物理效果证据。
这些适配不新增业务假设；库符合性、固定工具责任和有限正常值范围仍沿用原 A6 条件。

### 网络授权所用的克隆与析构边界

同一 `A6-network-stdlib` 登记增加 `String::clone` 与 `Option<T>::clone` 两个接口：在有效 Rust 值的正常返回中，字符串保持完整内容；None 不调用字典，Some 恰调用给定元素克隆并传播其结果。两版 `option.rs`、`clone.rs`、`tuple.rs`、`alloc/vec/mod.rs` 和 `alloc/slice.rs` 源码一并冻结。自有记录、策略、命令、维护状态和完整 World 的克隆均保留真实生成函数体，由 [Cloning](../../SandboxModel/Access/Cloning.lean) 提供逐类型的值保持证明，不把自有克隆算法归为外部函数。

共享当前状态的抽取还登记原始布尔 `PartialEq::ne`：两个固定编译器的 `core/src/cmp.rs` 都调用布尔原始不等比较，正常返回恰为两个布尔值不相等。签名按真实外部模板核对，使用可计算模型而非公理；四种布尔组合的模型定理进入依赖审计。两版源码摘要进入同一环境清单，不将此条扩展为任意自定义相等字典的合同。

当前生命周期请求的 `Renew` 生成闭包也包含该布尔原语；请求命名空间的
[适配](../../verification/aeneas/stage1-protocol/request/SandproofStage1Request/FunsExternal.lean)
直接委托同一网络库模型，不另造合同。字段相等仍由真实生成体及固定后端的 I64/Bool
比较连接；四值模型核对进入请求审计。key-only 错误版不再到达操作相等，因此不产生该外部模板；
请求抽取只准许原两项与这项已登记原语，拒绝其它未登记符号。

同一合同还登记 `Option<T>::PartialEq::eq`：None/None 返回真，None/Some 与 Some/None 返回假且不调用元素字典，Some/Some 恰调用给定相等字典并保留结果；两版固定 `option.rs` 的四分支已核对。可计算方法补足固定后端需要的方法名，不改生成函数体或启用不安全可约性。任意元素字典正确性不是外部承诺；实际恢复来源、三模式网络策略、十二种命令及完整尝试键的相等仍由自有证明落实，才能实例化有限表查询合同。构造器不匹配不能触发元素比较的外层失败，Some/Some 也不能吞掉字典失败。该登记不提供操作去重、回执可信性或重试许可。

固定后端的元组内置克隆不保留字段克隆字典。因此本批 `Store::clone` 的连接只适用于已分别证明 `CloneExact` 的具体键和值；不宣称对任意有状态、失败或改变值的自定义 Clone 都正确。向量、可选值及嵌套记录所需字典条件由同批具体引理落实。该纯分配模型不覆盖物理分配失败、OOM/abort、缓冲区地址和容量；正常返回的逻辑值相等不能代替物理容量或崩溃恢复证明。

Linux 前端还产生固定类型的自动 `DropGlue`，纯后端不输出这些胶水。审计单列精确类型集合，要求真实结构化体和声明顺序，只接受存活/结束、局部单元返回值、字段析构、枚举分派及编译器异常终止；拒绝普通调用、参数写入、自定义析构和集合外的目标。只有已登记自有胶水或 String/Vec/Option 的库析构可作为目标。其内存回收的擦除属于固定翻译器/库模型边界，不能用此例外遗漏自有业务函数，也不证明物理内存清理；新增自定义 Drop 后必须重新建立对应而非沿用该登记。

单字段编号类型的别名表示逐项核对真实 `_0: u64` 结构及生成定义，只允许已登记的编号类型；别名本身不提供非零、有效窗口或资源归属证明。入口/出口的记录查询、精确实例及策略相等、入口租户判断、顺序拒绝和完整状态保持仍是自有定理，不由这些库合同担保。

以下条款为本模块的规范定义；跨模块条款通过前提提供方连接，不将下游自有义务假定为已证。

创建入口的同一合同增加五个标准库绑定：`Option::as_ref` 委托固定 V0 借用模型；`is_none_or` 在 None 时返回真且不调用回调，Some 时恰调用给定 `FnOnce` 并传播其正常、失败或不终止结果；`unwrap_or_default` 仅在 None 调用默认值字典；`cloned` 委托已登记的可选值克隆；`Vec::default` 返回空逻辑向量。两版固定 `option.rs` 和 `alloc/vec/mod.rs` 的原分支及签名须匹配冻结摘要。仅接入可计算库绑定，不在 V4 重证公共容器/回调引理；通用性质由 V0 提供，具体维护高水位、查询、检查顺序及成功状态更新仍归 V4。库符合性是显式 A6 条件，不因定义可计算而升级为标准库已验证。

`fresh_identity` 的三个真实闭包分别捕获两个、一个、一个不可变 u64 编号引用。固定后端将它们表示为标量对或标量；审计逐项核对捕获数量、顺序、共享引用及原 u64 类型，不允许任意闭包都使用类型别名。闭包调用体与编译器胶水仍逐项检查；这不是遗漏业务函数或自定义析构的豁免，也不加入物理分配失败的保证。

### A6 DataLease 只读观察接口

`A6-data-drain-read` 只用于真实 [DataLease](../../controller-service/src/application/data_drain.rs) 的 `routes_dropped/drained` 两个共享借用读方法。抽取入口通过 `#[path]` 引用原生产文件；实际自有判断保留生成体，由 [独立策略](../../SandboxModel/Boundaries/DataDrain/Model.lean)、[表示关系](../../SandboxModel/Boundaries/DataDrain/Representation.lean)及[对应证明](../../SandboxModel/Boundaries/DataDrain/Refinement.lean)连接。源码、三项库模型、抽取参数、两版标准库、工具与审计根由[输入清单](../../verification/data-drain-inputs.sha256)及[环境清单](../../verification/data-drain-environment.json)固定。工具检查复用已有 Linux 环境，不引入新工具版本。

| 实际依赖接口 | 本批使用的保证 | 不能推出 |
|---|---|---|
| `Arc::strong_count(&arc)` | 有效共享借用下返回此分配的一次强引用计数观察，模型保留原 `usize` 值；固定两版实现使用内部 Relaxed load | 计数在返回后保持不变、观察与随后原子布尔读取构成同一快照、以后不能新增 pin、所有物理进程已经退出 |
| `Arc::deref` | 共享借用指向原 Arc 分配中的同一 AtomicBool 对象，不选择另一个对象 | 解引用本身读取布尔值，或建立管理锁、租户身份和退出证据 |
| `AtomicBool::load(Acquire)` | 对原布尔对象执行一次 Acquire 读取，返回该次观察值；实际调用的排序由 LLBC 审计限定 | 一定观察到所有并发或未来写入、`false` 自身等于真实退出、对另一对象或整个系统建立同步 |

外部模型的 `Arc` 记录是**单次调用的观察载体**：计数与条件性布尔读取可以发生在不同时间；布尔字段不是在 deref 时取得的快照。同一记录不能作为跨调用不变的共享堆使用。计数不是 1 时原程序短路，不需要发生布尔读取。模型只为实际三个读接口提供定义；其他内存排序以外层失败拒绝为范围外，不是断言真实标准库的 Relaxed/SeqCst load 会失败。没有 Arc 创建、clone、Weak、store、Drop、任务取消或任意 unsafe 的模型。

实际 `drained` 的已证接口是：相对于本次提供的观察，返回真当且仅当计数为 1 且 unknown 为假；同时证明 unknown 为真或计数不为 1 时返回假及正常可表示输入被接受。`routes_dropped` 只判断计数，不判断 unknown。三项原语模型与真实标准库的一致性仍为显式 A6 条件，源码审查、原生回归和摘要不是标准库的形式化证明。

调用者继续负责：绑定原 lease/AtomicBool 与协调方保留的句柄、管理锁和所有 pin 来源；在释放决策期间阻止新持有者，包括可能的旧持有者克隆或 Weak 升级路径；建立并保持持久授权失效；哪些失败必须 `mark_unknown`；依据精确宿主观察处理未知；保持检查至实际释放间的条件。本批没有把这些自有业务义务假定为已证，也不提供真实排空、释放、存储恢复或完成性定理。生产源码的“known exit”注释不能替代上述使用条件。变更原方法、库接口、调用排序、对象绑定或模型后重查本批及未来调用者的证明闭包。

## A 底层依赖的详细约定

本附录把正文中的“依赖外部软件”细化为证明前提。总目标写成：

```text
外部原语合同 A ∧ 已登记薄适配合同 B ∧ 合法部署 D ∧ 自有业务实现对应 R
    ⇒ 每条真实执行历史都满足规约 P

A_safe     := 宿主隔离、受保护控制数据、依赖原语的安全合同
A_function := A_safe，加上本次调用期间 envd、来宾文件系统
              和固定客户端遵守功能协议
```

自有恢复、鉴权和业务决定属于待证明代码；仅上文批准且登记到实际函数的薄适配可按 B 作为可信条件。能由业务程序建立和保持的部署条件仍是其证明义务，不能一并假设成立。系统结论列明 A/B/D 和剩余调用绑定，区别真实同步核心的机器证明与依赖审查/测试支撑的适配符合性。

允许的故障包括恶意或格式错误的请求、并发、重复和迟到消息、拥有自身 root 权限的来宾、伪造响应、网络中断、超时、资源不足、部分完成，以及控制器或工作进程在任意两步之间崩溃。权限错误或观察失败都必须有明确分支。

不承诺宿主隔离已被攻破、管理员恶意篡改受保护数据、硬件或存储静默损坏、任意回滚旧备份、未约定的断电行为，以及外部窃取有效凭据后仍识别真实持有人。不证明用户程序计算正确或必然终止，也不承诺在允许任意内容的外部通道上阻止所有信息外传。

模板边界包含自有构建编排、注册、别名、发布和产物服务，均工程测试；核心使用时仍证明授权/不可变绑定、准入、资源和清理。本阶段新增同宿主全量快照卸载/恢复，按上方 S-C1–6 连接；共享卷、迁移、多中心及存储自动切换不启用，相关类型需证明所选初态/允许输入不进入排除分支。当前仅单固定宿主的协议与组合纳入验证；多宿主证明后置，部署限制见[项目蓝图](../project-blueprint.md)。

翻译障碍总单的 T9（浮点与来宾兼容算法工作包）本轮排除，不作为关单前置，也不声称已证明；这不排除来宾其他安全业务。T0–T8 依实际入口分别采用模型、薄适配、业务证明或行为保持重构，未逐个预检前不承诺全部原式可抽取。

每个外部操作使用四项合同：**调用前提、成功后果、失败可能留下的状态、崩溃后可用的证据**。失败默认不等于没有副作用。

### A1 Firecracker 和 jailer 的具体边界

Firecracker 官方将来宾视为不可信方，并说明网络流量过滤由宿主负责。它不是我们租户授权和出站访问规则的实现者。[Firecracker 设计](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/design.md)

官方生产部署要求特别指出：jailer 的输入是可信输入；相关路径及其父目录需受保护；宿主安全更新、系统调用过滤和资源限制也是部署责任。因此不能推理为“用了 jailer，所以用户传来的任意路径和配置都安全”。[生产部署要求](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/prod-host-setup.md)

定义 `SafeLaunch(b, cfg, h)`，其中 `b` 是获批实例绑定、`cfg` 是将交给 Firecracker/jailer 的配置、`h` 是宿主状态。它必须同时满足：

1. `cfg` 来自已提交计划，完整绑定所有者、沙箱、代次、虚拟机和运行位置，不由客户端直接指定宿主路径。
2. 二进制、控制套接字、配置、登记文件、监狱目录及父目录受保护；校验至使用期间，目标身份不会被未授权替换。
3. 使用匹配的 Firecracker/jailer 版本和支持的构建；实际降权身份、隔离目录、系统调用过滤、资源控制组均符合部署方案。
4. 每个传入磁盘、网络接口及其他句柄只指向获批资源；基础镜像不因实例运行而被改写。
5. 客户获批的处理器、内存和磁盘形状被准确转换；单位换算与序列化无溢出、缩窄或静默截顶。
6. 宿主网络路径已处于批准规则之下，默认拒绝未获批路径；来宾不能修改这层规则。
7. 启动中的资源已有可恢复登记，故障或丢回执不会留下无法归属的实例。

其中第 1、2、4–7 项包含我们的程序义务，不是外部软件保证。第 3 项同时包含外部部署前提与我们对实际配置的核对义务。

一个容易忽略的细节：jailer 在特定的第二版资源控制组配置下，指定父控制组不存在时可能继续运行而不加入它。因而“jailer 成功退出配置阶段”不足以证明资源限制生效；需要核对实际控制组成员与限制。[jailer 参数和行为](https://github.com/firecracker-microvm/firecracker/blob/v1.16.1/docs/jailer.md)

对启动分三层记录，不能合并：

| 事件 | 可以推出 | 不能推出 |
|---|---|---|
| 配置请求被接受 | 对应配置操作满足 Firecracker 接口的接受条件 | 资源属于请求租户、全部宿主约束已生效 |
| 启动操作被接受或观察到运行 | 该虚拟机在相应观察点已经启动 | envd 就绪、用户程序健康、以后不会退出 |
| 同一绑定的宿主检查和 envd 初始化完成 | 已达到本项目规定的服务就绪条件 | 用户任意应用已启动成功或未来持续健康 |

Firecracker 的接口分别暴露配置、启动动作和实例状态；我们的“沙箱可用”需要在这些接口之上定义。[固定版接口模式](https://raw.githubusercontent.com/firecracker-microvm/firecracker/v1.16.1/src/firecracker/swagger/firecracker.yaml)

这里的成功证据通常是带身份的历史事实。定义 `ReadyObserved(b,k)` 为第 `k` 个事件中对原绑定取得规定就绪证据。运行发布需要这样的证据、没有已知的失效事件，并再次核对围栏和期限；不能推出“在响应到达客户端的那一刻，进程一定仍活着”。来宾可以在观察后立即退出，任何普通健康检查都无法消除这个时间间隔。

### A2 外部原语的合同清单

| 编号 | 原语与前提 | 可依赖的结果 | 我们需要证明的使用方式 |
|---|---|---|---|
| A1 | 固定受支持环境中的虚拟化隔离 | 来宾只能通过获准设备与宿主交互，不任意改写宿主内存 | 设备、磁盘、控制套接字和网络绑定不越权；禁用未纳入模型的设备路径 |
| A2 | 对已验证原对象持有稳定进程句柄 | 终止或观察作用于该句柄指向的进程 | 句柄创建前后核对原进程身份；跟踪所有受管辅助进程，不能把单一进程退出当作子进程全部退出 |
| A3 | 受保护目录及带身份约束的文件操作 | 读取、替换、删除作用于指定对象；显式错误保持可见 | 先登记身份再产生副作用，清理前核对原身份，处理路径替换和部分写入 |
| A4 | 指定 SQLite 配置和排他写入纪律 | 事务恢复为提交前或提交后的一致状态 | 核心、派发计划、身份、额度和期限同事务；未知提交按两种可能处理 |
| A5 | 已确定网络空间与规则集合的内核执行 | 经过该路径的数据包服从安装的规则 | 所有来宾出口都经过此路径，规则对应获批策略，安装期间无放行窗口 |
| A6 | 库提供的有序字节流、错误和取消语义 | 读写返回值与已接受字节对应；失败可发生在部分传输后 | 短写循环、背压、结束标记、预算、取消和半关闭组合正确 |
| A7 | 固定 envd 和客户端的功能协议 | 请求、输出、退出事件、文件完成回执具有约定含义 | 不改变请求含义，不伪造成功，不把超时当作未执行 |
| A8 | 可信时钟和约定的漂移范围 | 读数与真实时间的关系满足声明 | 单位、溢出、持久时间下界、重启和期限竞争正确 |
| A9 | 密码学和认证边界 | 未授权方不能在威胁模型内伪造身份或有效凭据 | 凭据精确绑定实例，撤销生效；一个实例的凭据故障不影响无关实例 |
| A10 | 不可变模板交付合同 | 描述、授权和产物对应，产物在使用期间受保护 | 固定实际修订、核对长度与摘要、按形状计费，不在恢复时重解析别名 |
| A11 | 固定 Redis 版本、[中心持久化合同](../module-contracts.md#中心持久化与恢复)中的 AOF/内存/重写配置、可靠存储及单有效写者 | 原子生效与冻结耐久/未知合同，恢复不补造账本 | 自有 CAS/Lua、编码、版本、缓存失效、耐久确认及派发；原子执行不是多命令错误回滚 |
| A12 | 双向认证连接和明确连接错误语义 | 认证对端与连接内字节合同；跨请求/连接仍可能重复、延迟、丢失或乱序 | 证书到部署/节点映射、字段绑定、端点白名单、参数保真、去重和旧消息拒绝 |

进程句柄解决的是“打开后不要因进程编号复用而改指向”；它不会自动证明打开时就是我们原来登记的那个进程。[Linux 进程句柄说明](https://man7.org/linux/man-pages/man2/pidfd_open.2.html)

SQLite 的原子提交也依赖底层输入输出合同，且不替我们把数据库之外的虚拟机或目录变成同一事务。[SQLite 原子提交](https://sqlite.org/atomiccommit.html)、[日志与检查点机制](https://sqlite.org/wal.html)

摘要不是数学上无条件唯一的身份。模型优先使用完整描述相等；若实现仅比较摘要，应单列抗碰撞前提。类似地，随机令牌的不可伪造性是密码学前提，不应伪装成确定性程序定理。

### A3 哪些前提可以检查

部署预检可以检查二进制版本、配置格式、权限、路径、控制组、转发设置和资源预算；运行期间还需保持受保护目录、排他控制和规则不被外部写者破坏。

预检不能证明内核没有漏洞、未来磁盘永远可写或外部命令永远及时返回。因此“预检通过”不是 `A1–A12` 全部成立的证明。凡是可失败的观察，都保留 `未知/错误` 分支，不用一个布尔值把“没看到”当作“不存在”。

## B1 Host 任务与原许可池的逐符号可信适配

本登记保留旧 V0 #333 的 T6 成果（原 #510 已迁移），本阶段适用性由 #632 核对。当前候选的生产实现定位为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；实际版本由[环境清单](../../verification/task-contract-environment.json)和[输入摘要](../../verification/task-contract-inputs.sha256)固定。许可取得、转移和归还的三个实现文件与此前组合基线逐字一致；驱动回调及测试夹具的变化单列在环境清单，须完成新组合的组件和完整入口复验后认领其适用性。本批不修改生产 Rust、Charon/Aeneas 或工具版本。

### A6-task-pools 外部原语条件

固定 Tokio 1.49.0：同一池中每次领取一个独占许可，容量不足时非等待领取拒绝；等待领取允许排队，取消撤销排队责任并归还已经分配给该 future 的许可；正常 Drop 归还原池且不复制许可。许可可以在唤醒 future 之前分配，因此 `.await` 尚未返回不等于尚未获额。固定源码的队列/关闭/Drop 路径在环境清单登记；Cargo.lock 的 `.crate` 校验和及全部解包文件由检查器核对。此完整性检查不是 Tokio 已形式化证明。

`spawn_blocking` 接管给定 FnOnce 及捕获值，至多调用该 FnOnce 一次；丢弃 JoinHandle 不主动丢弃仍在排队／运行的闭包。**许可是否一直跨越业务回调，由下方 B1 的命名绑定与作用域负责**；Tokio 不阻止闭包主动提前 `drop` 捕获值。已开始的阻塞任务不能靠 abort 证明退出。尚未开始的任务可能被运行时丢弃，捕获值按实际 Rust 析构路径处理。回调错误和 unwind 前都可能已有业务效果；JoinError 只表示结果不可按正常成功使用，不提供回滚事实。[固定版 JoinHandle](https://docs.rs/tokio/1.49.0/tokio/task/struct.JoinHandle.html)、[固定版 Semaphore](https://docs.rs/tokio/1.49.0/tokio/sync/struct.Semaphore.html)。

这些是有限接口的外部条件，允许排队、执行延迟、关闭和失败，不假定公平调度或后台工作必然结束。进程 abort、崩溃、OOM、泄漏／forget 许可及未登记的增额、合并／拆分操作不由正常 Drop 合同担保。宿主进程崩溃后的持久责任、VM 与外部进程清理由原恢复模块处理，本合同不把内存许可当作物理资源释放证据。

### 自有适配与使用者义务

| 逐符号范围 | 作为 B1 保留的机械符合性条件 | 仍须由使用者提供 |
| --- | --- | --- |
| `HostLifecycle::new/clone`；`HostFacade::new/clone/lifecycle`；宿主 `assemble/serve` 和 `RegisteredHost::status` 接线 | 两个不同的原 work/status 池，各容量 1；克隆保持原 application 和原池；注册接收器、driver、模板缓存复用原 facade | 实际部署只组装一个该门控家族；另建 wrapper 会产生另一个池，不能套用全 application 的单任务结论；请求／节点及完整回调输入绑定 |
| `HostLifecycle::blocking`、`receive/confirm/close/validation_evidence` | 领取原 work 许可后提交原 FnOnce，将命名许可绑定移入闭包并跨越实际回调；Join 错误保持原映射 | 每个真实回调的业务不变量及每个可观察前缀；请求、模板、compute、回执、发布等参数正确；来源不同的错误不被用于补造“未执行”或释放事实 |
| `status_admission::acquire`、`HostLifecycle::free_resources` | 先非等待领取 status，再等待同一 work；队列取消归还已得预留；获额后两许可跟随原只读回调，调用方取消不提前归还 | 节点／compute 绑定、健康观察本身正确、时间和结果解释；不由有界等待推出健康时效、宿主工作终止或公平进展 |

`blocking` 的真实调用者共五处：`advance/pending/cache_storage_ready/recover` 分别调用原 `advance_internal_lifecycle/internal_pending/validate_internal_cache_storage` 及顺序的 `recover_on_startup/expire_due_report`；`drive_once` 在同一阻塞回调内执行维护、恢复、过期处理及有限扫描。驱动只在原模板已取得时，按原请求的 create 绑定查询可选 `CandidateVerification`，再将原 request、template、compute 和该凭据交给 `advance_internal_lifecycle_with_verification`。它没有另建许可池或提前归还原 work 许可；模板凭据来源、缓存有效性、允许省略哪些重复校验及完整生命周期行为仍由相应业务模块证明。直接入口保持原 `record_internal_intent/confirm_internal_result/close_internal_window/internal_validation_evidence`，健康路径保持原 `internal_free_resources`。这些回调体没有整体列入 B1；本批也未新增它们的 Aeneas 对应证明。可信的是登记的任务／许可机械接线，其符合性依据为逐符号源码审查、冻结摘要和确定性组件回归，不是完整异步 Rust 的机器证明。

### 可复用合同及组合方式

[Task.Model](../../SandboxModel/Boundaries/Task/Model.lean) 描述每次调用的取得、提交、开始、调用方停止等待、业务效果、结果来源和退出；[Task.Capacity](../../SandboxModel/Boundaries/Task/Capacity.lean) 将任意有限任务表接到同一对池。预算规则仅做逐次领取／归还，容量界限由初态与逐步守恒证明。所有活跃／已提交／已获额任务都计入 work；等 work 的健康任务持有 status；回调已停止但尚未析构的许可仍计入各自账本。

`acquired` 包含队列已获许可、尚未再次 poll 的状态；取消须归还两项实际已得责任。`settled` 表示回调不会继续执行，允许 work/status 一个一个归还，并保留其间其它任务的交错。它不是持久提交或外部进程退出。模型中的 job 标签仅区分调用实例，不是业务请求去重或 exactly-once 证明。账本的 free 是抽象未分配额度，不承诺 `available_permits()` 等多个观测组成原子快照。

使用者将实际调用历史投影到合同步骤，并提供同池、封闭许可使用范围和回调绑定后，可复用每调用次数至多一次、取消不提前放行仍在执行的工作、原池身份保持、有限任务集合不超 work/status 容量，以及 Busy 不能推出无业务效果。模型有合法非空执行和提前释放／错误池的同性质反例。**实际异步历史符合该投影仍属于上述 B1 条件**；模型定理不自动证明这层符合性，更不证明回调业务、全系统 CPU/内存/VM 额度、网关撤销或无条件进展。

不同模块只可实例化同一合同，不复制一套许可语义。新增池、公开增额、许可逃逸、其它提交入口、自定义 Drop 或生产源码变化，须更新符号登记、调用范围、摘要与组件证据后再使用当前结论。


## B3 撤销等待的逐符号可信适配

登记对象仅为 [until_revoked](../../sandbox-data-transport/src/revocation.rs)；以 integration `a2b8a4b6ce5c170c953130b57abc8c2d6c468d0b` 为来源，当前输入由[清单](../../verification/revocation-contract-environment.json)和[摘要](../../verification/revocation-contract-inputs.sha256)绑定。生产函数体保持原样，只增加确定性测试入口。原固定 Charon 直接预检真实入口返回 `Coroutines are not supported`，因此这里采用获准的条件接口模型，不将 async Rust 标记为已抽取。

### A6-watch-select 原语提供什么

固定 Tokio 1.49.0 的 `watch::Receiver<bool>::wait_for` 读取最新值；值为 true 时就绪，最后一个 sender 关闭后也结束等待。`watch` 不是事件日志：在观察前 true 已被 false 覆盖时，不能从历史上曾有 true 推出本次等待应结束。`tokio::select! { biased; ... }` 按源码分支顺序 poll；本轮撤销分支就绪后不再 poll 工作分支。两个分支之间仍允许其它线程更新信号；不能把这一轮当作原子事务。固定 `.crate`、完整解包源码和 watch/select 审查位置均检查摘要。该完整性检查不是 Tokio 的形式化证明。

### B3 适配自身提供什么

| 原函数/语句 | 有条件保证 | 不包含的承诺 |
| --- | --- | --- |
| `until_revoked(signal, work)` 的参数与固定 biased 分支 | 使用传入的原 receiver 和原 work，不重新授权、不替换对象；撤销分支先观察到 true 或关闭时返回 None，不 poll work | future 的构造表达式可能已执行，捕获值即使从未 poll 也可能有 Drop 效果；None 不能推出从未产生效果 |
| 撤销分支 Pending 后 poll work | 工作 Ready 时原样返回 Some(value)，包括 value 本身是 Err；工作 Pending 后，下一轮仍先检查撤销；一次 poll 内可先产生效果再 panic 或不返回 | 信号在两分支之间才变为 true、work 随后 Ready 时，允许 Some(value)；不承诺撤销发送之后全局再无 I/O，也不能抢占一个尚未返回的同步 poll |
| 分支选择后的正常局部析构；调用方取消 | 被丢弃的局部 future 不再被该等待 poll，正常析构其剩余捕获值；外层取消不伪造一次 None 返回 | 不回滚已发出的操作；Drop 内部可能有新的清理效果；丢弃 JoinHandle 不等于其后台任务停止、许可已归还或 relay 退出 |
| panic/unwind/abort 或未获再次调度 | 保留中断/未知分支；不凭函数未返回补造完成或 Drop 证据 | 不保证析构完成、无泄漏、公平进展、外部工作必定退出 |

[Revocation.Model](../../SandboxModel/Boundaries/Revocation/Model.lean) 单独记录原 signal/work 标签、分支检查、进入/正在执行/返回的工作 poll、观察到的工作效果历史、选择结果与正常析构观察。效果历史不代表可回滚的全局内存快照；析构的内部效果不在这个变量里。`drops` 仅记录正常结束路径提供的析构完成凭证；中断时为零不表示没有运行析构，也不排除部分或全部清理已经发生。正在 poll 时可观察到工作效果前缀，随后允许 panic 或永不返回，不虚构一次 Pending 来代表它。模型证明标签保持、选择之后无后续 work poll、正常析构观察至多一次、返回分类、上述两个允许竞态和错误分支顺序反例，并直接复用 B1 的“取消运行中任务的等待仍持额”定理。实际 Rust 历史符合这些步骤，仍是本段 B3 可信条件；模型不是该符合性的机器证明。

### 当前四处生产接线及使用者要证明什么

四处调用均在 [data_gateway.rs](../../sandbox-host-service/src/data_gateway.rs)：正文准备、HTTP 握手、上游连接驱动、请求转发。三个直接等待点（正文准备、握手、请求转发）使用本次 permit 的 receiver；连接驱动使用从同一 receiver 克隆出的 `connection_revocation`，同时持有原 child/relay_exit/connection_lease，结束等待后仍执行原 relay stop。检查器记录全部 `*/src` 调用数量，包括测试，新增调用必须重新审查；文件摘要绑定原接线。这是源码审查范围，不是四个完整业务函数都已证明。

使用者仍须证明：permit 属于正确请求/对象/代际及授权；信号对应同一权限且需要单调撤销时不重新置 false；撤销后的新准入拒绝；工作内业务、字节预算和可能已发生的效果；原 body/lease/relay 生命周期、未知保持及真实退出/排空证据。不能仅凭 Some、None、watch 当前值或局部 Drop 释放 VM 或宣告持久事务成功。网关其它循环、流操作和 B3 大类未由本登记自动覆盖。

工程核对运行原 Rust 的两个已有测试及八个新增确定性测试，覆盖预先撤销/权限方关闭、捕获值析构、两个分支间竞态、最新值覆盖、外层取消、工作产生效果后 panic 和后台许可仍被持有。隔离错误版只交换两个 biased 分支，先确认独立编译成功，再由原 `prior_revocation_or_lost_authority_never_polls_work` 同一断言拒绝。组件证据与 Lean 定理分别报告，不把一组测试提升为一般等价证明。

## A6/B3/B4 双向字节中继与结果处理（隔离候选）

本条只适用于 [#575](https://github.com/VeriInfra-Lab/sandproof/issues/575) 的[固定候选](../../verification/experiments/tunnel-contract/candidate.json)，不表示完整数据网关已证。来源为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；[隔离补丁](../../verification/experiments/tunnel-contract/candidate.patch)让原 `tunnel::relay` 调用 `control::decide/finish_upload`，生产源码未改。工具、实际库源码、完整中间表示、声明和析构位置见[环境登记](../../verification/experiments/tunnel-contract/environment.json)；完成状态仍见覆盖记录与任务。

实际生产调用为 Linux 的 `sandbox_host_service::data_gateway::relay`：将原匿名管道的 Receiver/Sender 和同一原 TcpStream 的 OwnedReadHalf/OwnedWriteHalf 交给中继。命名空间、目标实例、原进程和许可是否正确仍由 V6/V8 提供。父 HTTP 网关的 WriteIdle、TLS、正文限额和撤销不是这一原调用的 I/O 参数，不随本条自动完成证明。

| 提供方与接口 | 可以依赖的具体行为 | 不能据此推出 |
| --- | --- | --- |
| A6：Tokio 1.49.0 的 BufReader::with_capacity、AsyncBufRead 与 copy_buf | 两方向分别使用原 64 KiB 读缓冲；每次确认写入只消费对应缓冲的前 n 字节，短写继续处理剩余部分，零写按 WriteZero 失败；原读端 EOF 后还要完成该操作的 flush 才正常返回 | 不能把 64 KiB 当成 HTTP frame、全部库/内核队列或整个系统驻留上限；返回计数未被本中继用于预算或释放，也不表示远端应用已接收 |
| A6：原 read/write、flush、shutdown 的结果 | Pending 可继续等待；正常结果与原对象上的操作对应；错误前可以已传输一段前缀，flush/shutdown 本身也可以失败 | 错误不回滚已发送字节；写入端半关闭不等于另一方向结束，socket EOF 不等于 HTTP/Connect 或文件业务完整 |
| B3：原两个复制 future、pin/select 和正常局部结束 | 保持原四个流对象及所选结果来源；select 未设置 biased，同时可就绪时允许任一分支先被选中；返回或停止等待后，本函数不再 poll 被丢弃的复制 future | 不承诺取消前没有字节，不消除已进入缓冲的后续交付；局部 Drop、flush、原函数返回都不是子进程退出、通道排空或 VM 释放证据 |
| B4：原 First/Action 和给定结果的机械接线 | 将所选原结果交给同一真实 control 核心；依照核心给出的 AwaitDownload 动作等待原下行，再把原两个结果交给 finish_upload；Finish 动作返回给定原值 | 不能把“选择等哪一侧、是否保留错误”整个假设正确；这两项决定保留实际生成体。新增或替换流/结果、提前返回、错误来源串线须重新核对 |

本项目同步核心负责的决定是：先取得上传结果时继续等待下行；下行成功时保留原上传结果，下行错误时返回原下行错误。先取得下行结果时直接结束这次单请求中继，允许上传尚未完成，防止客户端等 stdout EOF、中继又等 stdin EOF 的循环。因而本函数的 `Ok(())` 是本层的结果，**不能命名为整个请求已成功**。两支都可就绪时，不推定上传分支必先选中，也不把未选中分支的潜在结果当作已观察回执。

[Contract](../../verification/aeneas/tunnel/SandproofTunnel/Contract.lean) 连接原生成决定与选择/继续等待/返回/中断，并复用现有 StreamHistory。字节历史只记录本次原流的读取、确认写入及待写内容；原始字节 EOF 与协议/业务结束使用不同的责任。[Results](../../verification/aeneas/tunnel/SandproofTunnel/Results.lean) 对任意允许的有限交错保持两个方向的前缀、缓冲界和原绑定；Raw 返回成功时只能推出在上述原 I/O 合同下，完整原始下行字节已由写端确认接收；不能推出客户端已经读到、上传已完整或业务协议已成功。合法轨迹包括任意满足预算的有限源完整通过，以及下行结束时仍有未读上传；存在可行轨迹不保证实际调度最终完成。原命令输出不新增累计字节上限。

正常同步值方程使用原 Aeneas 的 Result/Try/FromSame 模型；错误载荷按完整原值传递。当前实例限于上述固定管道/TCP 类型，以及没有可观察资源或共享状态析构效果的错误载荷。任意自定义 AsyncRead/AsyncWrite、有副作用的错误 Drop、额外许可/FD 捕获不能直接套用。原流句柄自身的关闭与其它析构效果仍在 A6/B3 条件和 V6/V8 责任中，不因同步核心的值证明而成为已机器验证的资源释放。

实际异步历史到这些 I/O/选择步骤的对应仍是明确的 B3/B4 符合性条件，依据是原符号/参数审查、源码摘要及确定性组件回归，不是完整 async Rust 的机器精化。源库、实际类型、四个对象、调用者、分支接线或捕获/析构变化都须重查，不能只刷新摘要。

本条不提供一般原版/候选等价、panic/unwind/abort/崩溃后态或无条件进展。完整 HTTP/Connect framing、授权/路由、其余缓冲与额度、子进程真实退出、DataLease/Unknown 和释放准入继续由 V3/V5/V6/V8 提供；不以 EOF、中继返回或一次错误码替代这些义务。


### A6/B3/B4 Guest 持额消息

范围是 [CommandOutput 的交接](../../sandbox-guest-service/src/command/streams.rs)和 [Frame 的持有/正常析构](../../sandbox-guest-service/src/command/body.rs)中的具体操作，不把 `publish`、`next`、`Producer`、驱动任务或整个 Guest 服务列为可信。来源为 integration `81885df6`；固定 Tokio 1.49.0、bytes 1.12.1。逐操作的源码锚点、公共结论、条件与业务提供方见[调用登记](../../verification/owned-message-environment.json)，输入见[摘要](../../verification/owned-message-inputs.sha256)。模型与工程检查的实际完成状态仅见[覆盖记录](../proof-coverage.md)。

| 实际操作 | 允许依赖的规则 | 使用方必须补的条件 |
|---|---|---|
| `mpsc::channel`、`try_send(output)`、借用原 receiver 的 `recv` | 容量来自原通道；发送失败返回原对象，成功交给通道；正常取出转交原对象；本范围的异步 `recv` 真正返回 None 时，发送已关闭、队列为空且没有已获额未发布/未 poll 的发送预留。该完成视图不是 `is_closed/len` 等多个读数拼接，也不套用于 `try_recv`。取消仍在等待的借用不消费消息，不产生一次 None 返回。关闭不立刻销毁队列；已有内部预留的发送可在关闭后完成。正常 receiver/通道析构按实际库处理未取走的对象 | 原通道/订阅/消息身份；容量合法；消息选择、Full/Closed/Lagged 的解释及消费者掉线责任（V8）。队列空位在取出时归还，不表示消息所带字节许可已归还 |
| `oneshot::Sender::send`、借用 receiver 等待/关闭/Drop | sender 消耗一次；拒绝返回原值；已发送值可在 receiver close 后被取走；空 receiver 主动关闭后，即使 sender 仍活着，也可立即得到传输错误。成功不保证接收或处理；sender 丢失可成为传输错误，receiver 丢失可丢弃未取走的值。接收方结束后，尚未发送的 sender 仍可返回原值；不是整个通道再无动作 | 给定 `Result<CommandExit, CommandError>` 来自哪个业务事实；`Unavailable` 与 End 的解释、Data-before-End、是否允许完成（V8）。不能把传输错误当成功退出 |
| 构造 `CommandOutput`、转交给通道/Frame；正常字节许可析构 | 原 `_bytes` 与原字节池/数量一同移动；发送失败、接收/Frame Drop 时按实际所有权销毁，不能提前归还后继续持有原输出 | 字节数量转换/领取条件、每个订阅的独立预算与全部持有者（V8）。字节计费不是整个进程物理内存界限，编码副本及额外缓冲另算 |
| `Arc::clone` 原订阅许可、`Bytes::from_owner(frame)` 与 `Bytes::clone` | 原对象被保留；多个共享引用对应同一张许可，不重复领取；最后一个实际所有者正常析构才执行该所有者的 Drop | 完整的引用账本、真实类型及有副作用的自定义 Drop、无新增持有者的条件（V8）。多次 `strong_count` 读取不构成快照；最后一个强引用可已减为零而内部析构仍未归还许可，模型单列这一持额阶段。复制字节值也不保证保留原 Frame |
| `Producer::frame` 的参数转移和 `Frame::drop` 中三段顺序 | 原 bytes/output/lease 被接入原 Frame；正常 Drop 先销毁编码缓冲，再销毁原 output，再尝试发送释放通知；随后才自动析构 Frame 的租约字段。通知可能早于最后一个订阅/记录/请求引用销毁 | 何时构造/丢弃 Frame、响应授权、协议、期限、消费者掉线处理及释放决定（V8）。通知不是全部许可已归还、relay 退出或物理资源已停止的凭证 |

尤其要区分 `_subscriber: Arc<OwnedSemaphorePermit>` 和 `_record: Arc<Shared>`。前者在这组无 `forget/add_permits/split/merge` 的用法中保留原许可；后者只保留原 `Shared` 对象，不能保证其 `record: Mutex<Option<...>>` 仍有可归还许可。真实 `OwnerGuard::drop` 会对未知运行方丢失调用 `permit.forget()`；这项隔离策略及所有修改者属于 V8/V6 业务，不进入机械适配。正常测试夹具中记录未被修改的归还观察不能背书生产异常路径。

[OwnedMessage](../../SandboxModel/Boundaries/OwnedMessage.lean)提供原载荷/资源身份保持、持额、共享引用与单张分配、FIFO 观察、oneshot 值/传输错误及 Frame 通知顺序的条件模型。公共历史由实际 A6/B3/B4 操作提供，业务仍证明自己何时使用它们；源码摘要和确定性回归不是一般性的 async Rust 精化。`recv` 等待取消、整个 receiver/body 被丢弃、panic/unwind 的可见前缀与已完成析构分别保留；abort/崩溃不承诺执行 Drop，也不提供后续恢复或资源归还。没有无条件调度、公平或最终消费保证。

本批只登记有界 mpsc 的 `try_send/recv/close/Drop` 与上述 oneshot/Bytes 用法；不把异步 `send` 取消、公开 `reserve/Permit`、Notify、JoinSet、broadcast 或 watch 一并纳入。完整命令协议、授权、进程/FD/输入输出归还及系统容量由 V8/V6/V9 提供，其他机制按本阶段实际需要在 V0 #632 核对。


## A6/B2/B3 Notify：关闭广播与单接收者通知

来源为 integration `70d08fd0`，固定 Tokio 1.49.0。范围是原 [PtyMaster](../../sandbox-guest-runtime/src/pty.rs) 的关闭信号，以及原 [Record.retired](../../sandbox-guest-service/src/command/registry.rs) 的单个保留任务；逐点条件和业务提供方见[登记](../../verification/notify-environment.json)，源码、库和验收绑定见[摘要](../../verification/notify-inputs.sha256)。这不是整个函数、服务或通用 Future 的可信白名单。

| 提供方与实际操作 | 可以依赖的保证 | 仍需使用方证明 |
| --- | --- | --- |
| A6：原 `Notify::new`、`notify_one` 与同一信号的 `notified/enable/poll`；本条单接收者域 | 初始无通知；无已登记接收者时最多保存一个待取通知，多次早到通知会合并。已有等待者可获分配；该已分配通知与另外一个保存通知可以并存。Ready 表示本次通知被消费，不表示业务成功 | V8：同一原 Record、同一信号、至多一个当前接收者，没有混入广播或其他接收者；退休资格、真实结束、期限与下一步动作。通知不是计数信号量，不能按发送次数推算容量 |
| A6：取消原 `Notified`；单通知与广播分别处理 | 已分配但未消费的单通知，在正常 Drop 时交还给下一等待者或合并到保存位；`enable` 也可能已消费通知，返回 Ready 后 Drop 不再补发该通知。广播等待者取消不产生一个新的单通知 | V8：区分取消一个借用等待、丢弃整个任务、正常析构完成和未知中断；不能假造一次 Ready 或归还许可。丢失队列位置不承诺公平进展 |
| A6：`notify_waiters`，仅使用本次广播前已经 `enable` 的原等待者保证 | 正常广播完成后，这些原等待者的通知可被观察；它不是为后来的新等待者保存一张单通知。多个等待者可以同时被通知 | V8/V6：原对象绑定和关闭资格；通知后是否继续调度、I/O 结果、错误及真实退出另证。收到通知不是外部进程/FD/全部引用已退出 |
| B2/B3：原 PTY 的 `close` 清空原锁保护的 Option 后广播；`read/write_all` 先创建、pin、enable，再检查原 descriptor | 仅对应这些原字段、锁和信号操作的顺序与结果。登记先于谓词读取；已看到 Open 的原等待者不会因“先检查、后登记”漏掉一次正常关闭广播。关闭后新调用由原 descriptor 检查拒绝，不等待未来通知；锁中毒的原错误或恢复取值语义不抹去 | V8/V6：原关闭调用的资格、所有访问者及不重新打开同一逻辑谓词；原描述符别名与读写在途效果、字节及首错误仍须处理。这是条件接线，不是该 async Rust 的机器精化 |
| B3：`previous.retired.notify_one()` 与原 `guard.record.retired.notified()` 的机械身份 | 原通知不会被替换成另一 Record 的通知，早于等待的单通知可以保存；只传递通知事件，不执行或授权业务删除 | V8：`finished` 的真实性、tag/原记录选择、表中替换与 `Arc::ptr_eq` 的旧记录保护、原保留期限及 `Guard::remove`。这些判断不进入可信适配 |

[Notify](../../SandboxModel/Boundaries/Notify.lean)分别提供有限单消费者历史和单调关闭谓词下一个任意观察者的模型。单通知模型记录幽灵发送/消费数量，证明消费不会凭空超过实际通知；它不是实现内部的机器计数器。广播模型证明登记后看到 Open、且仍活跃的原等待者，在正常关闭完成后已被通知，并有合法迟到拒绝、非空成功历史及单通知替代广播的同性质反例。每个使用方仍须提供自己的状态表示和真实 A6/B2/B3 事件投影，不能只凭模型定理声称 Rust 已机器证明。

通知方法可在返回前唤醒其他任务，故不承诺它返回时整个业务状态或资源账本未变。已有 I/O 可能在关闭被观察前产生效果；本次 close 清除访问入口，也不会立刻销毁其他 Arc 持有的 FD。原 Record 的 `_slot: Arc<OwnedSemaphorePermit>` 复用已有 [OwnedMessage](../../SandboxModel/Boundaries/OwnedMessage/SharedLease.lean) 的原许可/最后引用规则；通知本身不提供真实归还凭证。Record 工程夹具使用原类型及 reserve 得到的许可，只核对这个持有关系，不冒充 `publish/run` 的业务证明。

panic/unwind/abort、取消或崩溃只保留已观察前缀；没有正常返回就不补造广播完成、析构完成、退出或恢复。安全模型不承诺最终再次 poll、最终删除或调度公平。模型之外的真实 I/O、锁顺序和内部推进仍按所属 V8/V6/V9 义务完成。

本条不覆盖 `notify_last`、多接收者单通知队列或任意混用单通知/广播。源码中 [reaper.wake](../../sandbox-runtime/src/child_process/reaper.rs) 的同名 `notify_one` 实为标准库 **Condvar**，由已有 typed MIR 定位确认；其窄接口和未覆盖项见本页独立 Condvar 合同；队列谓词/回收策略归 V6，不借本 Notify 条款认领。当前源码定位、原工具不支持诊断和确定性原生回归只支持所列可信条件，未证明全部 Rust 并发。


## A6/B2/B3 JoinSet：收齐原任务与返回值

固定 Tokio 1.49.0，生产来源为 integration `886f2dd6`。范围限于原 [中心监督](../../sandbox-center/src/runtime.rs)、[受保护连接组](../../sandbox-internal/src/protected_server.rs)和 [Guest 连接组](../../sandbox-guest-service/src/listener.rs)使用的 `new/spawn/len/is_empty/join_next/abort_all` 与正常组析构。三处源码、五份库实现、十二组机械操作和三项业务责任见[逐点登记](../../verification/joinset-environment.json)；[摘要](../../verification/joinset-inputs.sha256)固定实际输入。本条是既有 A6/B2/B3 的窄接法，不将整个服务、TLS 或业务回调列为可信。

| 提供方与实际操作 | 可以依赖的保证 | 使用方仍须证明 |
| --- | --- | --- |
| A6：原组的构造、spawn、len/is_empty | 新组为空；成功 spawn 加入这一次原任务。完成但尚未 join 的任务仍计入组长；返回的 AbortHandle 被丢弃不等于取消任务 | 原组和逻辑 spawn 次序、完整参数/回调及有限合法输入；分配失败或进程中止不造正常返回。逻辑任务键不是永不复用的 Tokio 原始 ID |
| A6：借用原组的 join_next | Some 取出一个原任务的原值或原 JoinError，至多交付一次；None 表示观察时组内已无任务。T 本身可以是 Result，业务 Err 仍是正常返回值。JoinError 的 panic 载荷也可能持有资源，错误值尚未丢弃时不归还这份责任 | 结果表示关系、错误解释、后续动作和每次取出值的所有者；取消这次等待不移除成员，但任务可以同时完成，不能推导整个组状态不变 |
| A6：abort_all 与组 Drop | abort_all 只向当时组内任务请求取消，不移除它们，也不影响后来 spawn 的任务；已完成任务仍可返回原值。正常 Drop 请求取消并移除组的持有关系，不等待任务退出 | 原对象、正常返回/取消/实际 unwind/abort/崩溃的区别。Drop 中 panic 可以成为 JoinError::panic；没有返回就不补造取消完成。仍存活的工作和部分效果继续保留 |
| B2/B3：中心通知后收齐、受保护连接 abort 后收齐、Guest 正常关闭后收齐 | 只连接登记的原 group、spawn 参数、结果和调用次序。保留组至 join_next 返回 None，可由历史守恒推出每次加入的任务都已被取出；中心不额外请求 abort，Guest 保留原连接期限 | V2/V6/V8/V9：为何关门、通知谁、全部 worker/handler 行为、何时不再加入任务、逐结果处理和进展。中心保留首个观察到的错误，不自动等于物理上最早的失败 |
| B1 与已有持额合同：捕获值、返回值及独立工作 | 原许可沿实际对象移动；取出任务结果可以把仍持额的返回值交给调用者。空组不自动增加原池容量，取消 wrapper 不停止已开始的 spawn_blocking | V6/V8/V9：原许可/全部别名、真实资源归还、进程/文件/网络工作退出，以及未知时保持责任；不把处理函数或释放决定整体设为可信 |

[JoinSet](../../SandboxModel/Boundaries/JoinSet.lean)提供两个条件模型：组历史记录仍运行、已完成待取出、已取出和随组移除的数量；任意原任务的投影保留逻辑身份与原结果。两者必须由同一实际 A6/B2/B3 执行提供相容的投影，不是两套可任选的业务事实。结果字段分别保存正常值的表示 α 和原 JoinError 的表示 ρ；ρ 保留原错误身份与 panic 载荷的持有关系，不能只用 panic/cancel 标签替代。它们不复制 Rust 资源所有权，也不把别名指向的可变堆冻结成快照。`forgotten` 只表示组 API 不再持有该任务，不表示析构、许可归还或外部效果已经完成。

容量定理检查每次 spawn 前的原长度，再逐步推出上界，不把正确容量当作后置假设。Guest 的原 `len < limits.connections` 可用该接口；受保护服务的许可只约束持额连接，不能仅凭许可推导全部已完成待收齐的组成员数量。模型的 `accepting` 是调用者停止新增任务的逻辑决定，不是 JoinSet 自带 close 方法，必须由真实控制流建立。

这 36 条声明证明所列模型的历史守恒、原身份/值和单次交付、受检容量与完整收齐；另保留合法非空成功，以及提前 drop/只 abort 不能满足同一收齐性质的反例。原 Charon 的实际异步入口预检、逐符号源码核对和确定性原生回归支持可信接线的采用，不提供完整 async Rust 机器精化。原生错误版分别去掉中心/Guest 原收齐循环，必须先新编译，再被正确版相同的即时断言拒绝；编译失败或超时不算反例。

本条不覆盖 join_all、shutdown 便利方法、detach_all、JoinSet::spawn_blocking 或任务 ID 接口。没有无条件最终调度、终止或清理保证；不返回、panic/abort/崩溃只保留已观察前缀。业务回调、首次拒绝/错误、持久提交、真实 I/O/命令退出、Report 语义与授权仍归原 V2/V6/V8/V9，Mutex/RwLock、Condvar 及其它通道的公共缺口继续归 V0。


## A6/B2 标准 Mutex：原守卫、中毒与唯一借用

范围为固定 Rust 1.97.0 与前端 nightly-2026-08-18 标准库的 `Mutex::new/lock/try_lock/get_mut`、原守卫的借用/正常析构和 `PoisonError` 恢复。两版标准库的 mutex、poison 与 Linux futex 实现、三份代表生产源码和十三组操作/责任见[登记](../../verification/std-mutex-environment.json)及[冻结输入](../../verification/std-mutex-inputs.sha256)。本条细化既有 A6/B2，不增加整个业务正确的可信条件。

| 提供方与接口 | 可依赖的窄保证 | 使用者仍须提供 |
| --- | --- | --- |
| A6：同一原 Mutex 的 lock/try_lock | 正常取得返回原守卫；中毒错误也已经取得并持有原守卫。try_lock 先尝试原底层锁，忙时返回 WouldBlock，即使中毒也不返回一个可用守卫 | 原对象/有效 Rust 借用和守卫所有权，哪些字段由该锁保护；阻塞、重复领取、锁顺序与无死锁另证。逻辑领取序号不是指针、线程 ID 或公平队列 |
| A6：PoisonError 恢复与守卫读写 | 恢复返回原载荷/原守卫；不回滚已写数据，不清除 poison。持锁期间的写入须由调用者自己的关系描述 | 选择拒绝还是恢复、修复哪个业务不变量、原返回值及错误解释。中毒只是建议性观察，不是内存安全/完整业务状态的凭证 |
| A6：正常守卫 Drop | 在原锁上结束该次持有并按实际 panic 状态更新中毒；不析构 Mutex 内的 T。若取得时已经处于 unwind，后续释放可以不设置新的 poison | 区分正常返回、实际 unwind、Drop 自身 panic、abort/崩溃。forget 不解锁；外部进程、FD、许可等真实归还按原载荷全部所有者及业务证据处理 |
| A6：get_mut 的真实唯一借用 | 由实际 &mut Mutex 排除新的共享领取，不需要取得底层锁；返回原正常/中毒分支的可变借用，写回只修改原 payload | 独立的唯一 Rust loan 条件，不能从“当前没有活跃守卫”推出唯一访问。此前 forget 留下的底层锁可以仍锁住，get_mut 不修复它 |
| B2：SerialMutex/SerialGuard 诊断薄层 | 构造/lock 使用原 Mutex；lock 的诊断通知先于真正尝试，不证明取得。正常执行到 drop(self.guard.take()) 后，原解锁先于后续 CURRENT 诊断修改 | 此适配只在 contention-diagnostics 特性启用；计时表达式先完成才观察到该释放事件。measure/host_call 的业务回调、诊断状态恢复及计数不整体可信 |

[守卫模型](../../SandboxModel/Boundaries/StdMutex/Guard.lean)证明逐步/有限历史的互斥、原身份、守卫失效、忙碌/中毒、正常释放和调用者给定写关系下的不变量保持。payload 仅投影原内联字段及句柄，不把别名可变堆冻结成快照，不复制 Rust 资源所有权。守卫被遗忘时原底层锁仍忙；停止使用该守卫也不授予唯一借用。

实际 [SerialMutex::get_mut](../../controller-service/src/application/diagnostics.rs) 由原 Charon/Aeneas 透明抽取，[借用合同](../../SandboxModel/Boundaries/StdMutex/BorrowModel.lean)、[实现对应](../../SandboxModel/Boundaries/StdMutex/GetMut.lean)和[模型连接](../../SandboxModel/Boundaries/StdMutex/Bridge.lean)固定正常/中毒分支、原字段和回写。标准 Mutex 的独占借用库模型仍依 A6 符合性，不是标准库机器证明。吞中毒错误的实际 Rust 版本重新编译抽取后，被正确版同一 GetMutContract 内核反驳；它使用的 PoisonError<&mut T>::into_inner 专用绑定先核对实际 LLBC 借用实例，不适用于任意拥有值的 PoisonError<T>。

真实 Guest Meter::lock 和 PtyMaster::descriptor/close 分别登记原 Snapshot 和 Option<Arc<AsyncFd<OwnedFd>>>。Meter 的 exact=false、PTY 的中毒拒绝/恢复及 close 资格属于 V8/V6 业务；原句柄克隆仍可持有 FD，清空 Option/通知不提供物理退出或最后归还证据。两处共享借用入口的原工具预检保留后端非可结束借用、动态 I/O 错误类型障碍；条件模型、原源码审查和原生回归不冒充这两个入口已机器精化。

本条不覆盖 Condvar 原子解锁/重锁、Tokio Mutex/RwLock、所有生产 Mutex 使用点、全业务不变量或无条件进展。它们的公共缺口继续归 V0，实际业务守卫、恢复、授权与释放条件归 V1–V9，不能由 A6/B2 整体代替。


## A6/B4 Status 字段与普通 JSON 字节

[SchemaDomains](../../SandboxModel/Boundaries/SchemaDomains.lean)只连接原 StatusRequest/HostStatus 守卫和登记的普通字节调用。来源、完整字段、标准派生及四个字节根见[逐符号登记](../../verification/schema-domains-environment.json)与[字段分类](../../verification/experiments/schema-domains/catalogue.json)。固定工具、原 NodeIdentity/Resources 和原切片/游标不能换成另一个请求或字典。

真实同步 validate/validate_for 由原工具抽取，证明全文节点绑定、原拒绝及 slots 的 1..=4096 守卫；可表示的 4097 在守卫拒绝，不假装解码器不能表示它。资源零值及任意可表示时间是原允许输入，不因此获得授权、新鲜性或资源真实性。两条真实错误分支使用正确版同一独立合同反驳。

A6 仍只提供固定 JSON 引擎、标准派生和普通值接口；B4 只连接登记的原对象、完整参数/结果及实际字节位置。正常分配的 SliceRead/Vec 合同保留字段次序、对象/声明顺序数组、缺失/重复/未知字段、原错误和结束检查；异常可能已有输入/输出前缀，不许假定回滚或两次有状态编码相等。自有 ID/非零计数器及守卫继续使用真实生成体定理。任意自有 Serialize/Deserialize 回调、资源载荷、可观察 Drop、panic/unwind/OOM/abort 不属于普通值合同。

客户端请求编码、服务器请求解码、客户端响应解码有实际来源绑定；普通 Vec 的响应编码仅是参考。真实服务器的 encode_bounded 使用私有 Writer，不能用参考 to_vec 的定理代替；长度/分配失败/部分输出/错误优先级由独立 BoundedWriter 公共接口及其实际调用条件处理，见本页对应条目；其当前验收层次见覆盖记录。22 类型/10 上下文的分类不表示它们已全部证明。V3/V7 提供端点/认证绑定，V6 提供观察真实性，V1/V4 提供实际容量使用。

Status 与 TLS 的独立生成根会产生同名的 TransportError 判别辅助声明，不能在同一个 Lean 环境直接混合导入。公共根提供[纯值词汇和接受条件](../../SandboxModel/Boundaries/SchemaDomains/Values.lean)；完整 Status Rust 对应与 46 项审计保持独立文件/模块检查，并进入全部保留文件构建、真实文件诊断和总清单。没有删除 TLS 或 Status 检查，也没有改写生成物。调用方须区分可共同导入的值合同和各生成根的实现对应；本接线不承诺所有独立生成根能直接共同导入。

## A6 原 Store 共享查询与成员判断

[LoopCallbacks](../../SandboxModel/Foundations/LoopCallbacks.lean)复用有限首次匹配理论，连接原 Store::get/contains_key 和四个透明 Rust 委托。完整来源邻域见[登记](../../verification/experiments/loop-callbacks/source-map.json)，函数/字典及借用域见[环境](../../verification/loop-callbacks-environment.json)。本条只使用原工具/A6 和有效安全共享借用的既有可信链，不增加整个调用者或比较器正确的 B 条件。

返回值保留原有序表的首次匹配，允许重复键；contains_key 由同一次 get 的结果判断。实际 u64 比较已有对应定理；其他键须由使用方证明 TableLawfulEq，键投影须单射，V 不要求 Copy。共享借用仅给出普通值观察，不承诺物理地址、不复制资源，也不把别名可变堆当作不可变快照。跳过首项和反转成员判断的真实错误版均由独立的同一查询/成员合同拒绝。

源码邻域只登记 World、checkpoint 及 runtime 的实际调用，未证明它们的完整业务；V4/V9 继续提供自有比较/表示、恢复、授权、状态选择及释放决定。回调次数/状态和有状态 FnMut 是独立义务，不由这组只读查询定理自动得到。

## A4/A6/B4 SQLite 条件 UPDATE 的固定五参数接口

[SqliteStatements](../../SandboxModel/Boundaries/SqliteStatements/Model.lean)为原 write_accepted_with_ids 中一条条件 UPDATE 提供有限行片段；[环境](../../verification/sqlite-statements-environment.json)与[候选](../../verification/experiments/sqlite-statements/candidate.json)固定 SQL、五个位置/类型、原连接/事务、checked_add、整数边界、原错误和 changed != 1 的决定。这里只证明隔离副本实际调用的核心，生产未改；分离核心/固定五元组的有限数据库对照不构成一般旧新等价。

A4 采用固定 rusqlite 0.40.2/SQLite 3.53.2 的完整 Connection.execute API；A6 保留固定标准库与普通值。B4 只承担已登记的原对象和值传递。固定五元组的 NormalReplyContract 是联合接口：参数编码成功不等于 raw_bind 已成功，run 仍包含 raw_bind 的第一个错误及 step/reset 的原返回优先级。不能从内部 __bind_in 直接推导执行成功，也不支持任意有状态 ToSql、裸 Statement 或旧绑定复用。Statement::drop 忽略 finalize 的 Result，成功返回不证明清理无错。

行效果要求调用者/V5 提供真实 schema、有效非 NULL 列域、BINARY 文本比较、数据库保护、没有未登记触发器及同一事务连续历史；非 STRICT 类型声明本身不提供这些条件。成功恰改一行才能继续；零行的正常观察被错误版接受时由同一性质反驳。底层错误仍可有部分效果，影响一行不等于耐久提交，SQLite 不回滚外部 I/O。完整 outbox/投影/编码、事务提交、读取/确认/恢复、授权和寿命仍归 V5；panic/unwind/abort/崩溃后态与资源真实释放未因此证明。

本模块的完整生成根存在与旧根重名的全局声明，故保留独立 Lake/物理诊断视图；公共根只复用纯行 Model。独立的 38 项正例及 7 项错误版内核审计都进入完整批次清单，不因未强行共同导入而省略任何模块或检查。

## A6/B2 Condvar：等待时释放，返回前重新持有原锁

本条只覆盖固定标准库的 `Condvar::new/wait/wait_timeout/notify_one`，以及原 [Reaper](../../sandbox-runtime/src/child_process/reaper.rs) 的有限调用连接。两版库源、原对象、七组操作和业务提供方见[接口登记](../../verification/condvar-environment.json)。模型限定一次活跃等待，可以有其它线程在此期间取得、修改和释放同一个 Mutex；V6 提供这个 Reaper 的单一等待消费者及原锁绑定。多个同时等待者、`notify_all`、便利等待方法和公平性不能直接套用本条。

| 满足前提后可使用的保证 | 调用者仍须提供的条件和限制 |
| --- | --- |
| 原守卫交给 wait，注册等待与解锁按库合同连接；等待期间该次旧领取不能再访问受保护数据。暂停不是守卫 Drop，不设置新的 poison，也不析构原 payload | 有效的原 Rust 所有权、同一个原 Condvar/Mutex/守卫及一次等待。不能在检查谓词后先自行解锁，再把之后的通知视为已保存 |
| 正常返回前重新取得原 Mutex，返回同一个被传入的守卫值，保留其最初取得时的 panic 标记；读取当前数据而非等待前的快照 | 守卫值身份不是地址或线程编号。其它持锁者的实际写入关系和业务不变量由调用者证明；锁保护原字段/句柄，不冻结外部进程、别名可变堆或其它资源 |
| `PoisonError` 也已经持有原守卫；`wait_timeout` 的错误保留完整 `(guard, timeout-result)` 元组，恢复不回滚数据或清除 poison | 拒绝、恢复或 unwrap 的选择由业务证明。没有正常返回时只保留实际前缀；不假设 panic、abort 或崩溃一定完成析构或归还 |
| 通知不会改变原锁数据，也不保存给未来等待领取的许可；允许虚假唤醒 | 通知不证明队列非空、授权有效、子进程退出、任务完成或许可归还；返回后须重新检查实际谓词 |
| 实际超时结果只说明该次库等待的超时观察；通知和超时均不替代重新取得锁 | 本模型不保证时钟/调度推进，也不给出包含锁竞争在内的硬返回时限。10ms 参数不是进程退出或真实清理证据 |

[Core](../../SandboxModel/Boundaries/Condvar/Core.lean)和[等待协议](../../SandboxModel/Boundaries/Condvar/Protocol.lean)复用标准 Mutex 的逻辑领取与原对象规则。公开返回步骤同时要求匹配的活跃等待已经可以返回，以及原 Mutex 空闲；裸 `resume` 只是值构造器，不能单独作为真实调用已获准的证据。如果调用者证明每次实际写入保持自己的不变量，可以复用历史保持结论；[合法历史](../../SandboxModel/Boundaries/Condvar/Examples.lean)同时表明虚假唤醒不建立业务谓词、超时不保证已经重新取得锁。

B2 仅连接原 `self.wake/self.pending`、完整守卫/时长/结果及原 `Slot::retain` 中的调用顺序，不把 `run/reap_ready`、入队资格、队列谓词、真实退出或释放决定整体列可信。正常执行到 notify 前，源码已经执行原 push 并结束该临时守卫；若先前 panic 则不补造通知；notify 正常返回时工作也可能已被其它线程处理，不能断言原条目仍在队列。V6 继续提供每个原 Child/Slot、容量、全部业务写入、错误处理和释放依据。

真实原 `run/reap_ready` 的 Charon 预检可得到透明函数体；固定 Aeneas 在 `run` 循环的借用合并处报错，原文保留。本条的模型定理、源码/原生证据和条件接线不构成整个 Reaper Rust 的机器精化；不会通过把这个循环设为 opaque 来消除业务义务。模型中“不重新持锁就返回”被同一 Safe 性质反驳；原生错误锁例只是接口夹具，不冒充真实 Rust 的形式错误版。复现及原入口见[实验说明](../../verification/experiments/condvar/README.md)。


## A6/B2 Tokio RwLock：保留原读守卫的共享观察

本条限定固定 Tokio 1.49.0 的 `RwLock::new/read/read_owned/write`、原读写守卫的正常释放，以及 `OwnedRwLockReadGuard::try_map` 接 `Option::as_ref`。实际对象是 Guest 的 `Session.shared.current`；两份生产源码、五份固定库实现、七组机械操作和三项业务责任见[来源登记](../../verification/tokio-rwlock-environment.json)。[公共模块](../../SandboxModel/Boundaries/TokioRwLock.lean)只提供条件接口；不把整个会话或文件授权函数列为可信。

| 满足前提后可以使用的保证 | 调用者仍须提供的条件和限制 |
| --- | --- |
| 同一原锁可以同时持有多个读守卫；返回的写守卫独占，活跃或被遗忘的读持有尚未结束时不能完成写取得 | 有效 Rust 所有权、同一个原 Arc/锁及哪些字段受保护；没有写者只是必要条件，不保证下一次读立刻完成。锁序、重入、跨 await 持锁和无环等待另证 |
| `try_map` 成功将原守卫线性转为原字段的共享视图，不释放原读持有；失败把仍持锁的原守卫放在 Err 中 | 固定 `Option::as_ref` 是共享投影；原 Rust 守卫被消耗，不能复制成两次拥有。自定义有状态回调、内部可变对象及任意回调析构不能直接套用 |
| 持续保留同一个活跃原读守卫期间，其保护的内联字段/句柄不被合法写者改写；正常读/写守卫释放终止本次访问，forget 保留底层排斥 | `ReadHistory` 由实际守卫寿命和 A6/B2 事件提供；每次真实写入的业务关系及不变量由调用者证明。仅复制字段后解锁不能获得同一 `ProtectedRead` 结论；不冻结外部进程、时钟或别名可变堆 |
| 取消等待撤销本次内部排队/预留，不凭空提供可用守卫；其它任务可能因此继续执行 | 内部已分配的 semaphore 预留不是已返回的读守卫。模型只投影已取得的读写权，不说整个队列/运行时不变；真实取消、正常释放、panic、abort 与崩溃分别连接 |
| 在原锁继续存活的范围内，守卫释放本身不改写锁内载荷；本接口没有标准库 Mutex 式 poison 结果 | owned 守卫持有 Arc。若它是最后一个强引用，完整 Drop 还可能销毁原锁及其载荷，不能引用存活载荷的保持结论。T5/V8 继续连接所有别名、最终析构、秘密清零和资源责任；panic 不回滚已写内容，也不保证任意异常都完成析构 |

B2 只连接原 `current`、原守卫、共享投影和调用次序：`authorize/authorize_file_clock` 返回的 `AuthPermit` 保留原映射守卫；`apply` 的旧身份读取在该作用域结束后释放，之后才等待原写锁。旧快照能否用于后续决定、令牌/签名和期限、元数据、默认值校验、完整替换及何时结束许可仍由 V8/V9 提供，不能由锁定理推出授权正确。

模型和有限非空历史证明读写排斥、原领取、映射及保留读者时的值保持，同一 `ProtectedRead` 拒绝提前释放模型。原 Rust 错误副本改持一个副本锁、提前释放原会话锁，由原轮换等待断言检查；这是原生回归证据。固定 Charon 对真实 async `Session::authorize` 的 coroutine/GAT 限制保留，未完成 async Rust 机器精化。源审查和测试仅支持所列 A6/B2 符合条件的采用。

本条没有 FIFO、最大读者数、无条件最终取得或硬返回时限定理；不覆盖任意 `map` 回调、升级/降级、`write_owned`、`blocking_*`、任意其它 RwLock/Mutex 使用点或整个 Guest 业务。未被本条覆盖的公共接法仍归 V0，业务授权和真实资源/释放决定仍归所属模块。


## A2/A3/A6/A8/B4 原 Child 的退出观察与未知交接

[公共入口](../../SandboxModel/Boundaries/ChildObservation.lean)针对原 `OwnedChild::stop_and_reap` 的隔离同步候选。来源冻结于 integration `646fc2c8`，生产未采用该拆分；原实现和候选的有限对照是源码/测试证据，不是一般新旧 Rust 等价证明。[逐符号合同](../../verification/experiments/child-observation/contracts.json)保存九个机械函数的完整片段摘要、固定标准库版本、原对象和失败含义；核心、实际调用方及同性质错误版都使用固定原工具。

入口由 V6 提供有效原 Child、已预占原 Slot、真实创建/回收身份及两项都存在的条件。A2 解释这一次原进程观察，A3 解释原管道关闭，A6/A8 解释正常值、时钟和暂停；B4 只连接 `system/try_wait/kill/close_pipes/now/add_grace/pause/retain/unknown_exit` 的原对象、完整参数及该次回复。它不担保整个停止函数、reaper 或释放决定正确。

| 正常返回提供的窄保证 | 不提供的结论与调用者责任 |
|---|---|
| `try_wait` 的 Some 是原 Child 的同一完整退出状态，可来自其已缓存状态；None 与普通 io::Error 仍是合法回复 | None/Err 不说明进程仍活着或无效果；V6 防止提前回收后仍向复用 PID 发信号。一个进程退出不代表后代、VM 或全部 FD 已结束 |
| `kill` 保留原终止请求及回复；已缓存退出时可不发新信号便成功。`close_pipes` 顺序取出并正常析构本地三个原管道 | kill 成功与管道为空均不是退出/排空证据；复制 FD、在途字节及全部后代由 V6/V8 处理。部分关闭不回滚 |
| 每次 `now` 重新观察时钟，`add_grace` 使用原两秒常量，`pause` 是原十毫秒调用 | 不能推出两秒硬上界、时钟必然推进或最终回收；不可表示加法、panic、不返回没有正常后态 |
| `retain` 正常返回表示原 Child/Slot 曾一并交给原 reaper，随后执行原通知；`unknown_exit` 保留原错误类别与全文 | 这是历史事实，worker 可能已经回收，不能断言当前队列成员关系；全局计费与原 reaper 释放政策归 V6。通知不提供退出证据 |

机器证明的自有决定是：按原顺序观察/kill/关闭/比较截止；成功保留完整退出观察与本地原名额；未知返回前先取原 Child、再取原 Slot，转交原 pair 后构造原错误。实际生成调用方和具体接口字典另有检查。形式响应流与历史属于每次调用的逻辑上下文，不声称空 Rust SystemIo 物理存储这些字段，也不冻结外部 OS、时钟或其它队列。

普通 Err 是正常回复，不能被删去。正常返回合同不为中断调用、panic/unwind、abort、崩溃或取消补造后态、清理或回滚。Error/Status/Instant 等表示限普通值，不允许把额外许可、FD 或自定义可观察析构藏在其中。缺值 panic 诊断、OwnedChild::Drop/into_child、整个 pump/exchange、SafeLaunch、全局容量和完整生命周期仍由 V6/V8 证明。原生错误版与形式错误版均检验原 pair 责任；编译失败不是反例。

## A6/B4 有状态回调：原外部状态与访问顺序

[CallbackState](../../SandboxModel/Foundations/CallbackState.lean)证明隔离 crate 内真实 `toggle_step/trace_step` 的生成体，再以逐次正常观察连接通用筛选理论。[来源登记](../../verification/experiments/callback-state/source-map.json)明确：trace/calls 来自原 Store 回归，toggle 是隔离代表，完整生产谓词没有因这两个例子完成证明。

A6 仅提供普通值 Vec::retain 的正常有限遍历：对原每一行按原顺序调用一次，按该次 bool 保留原行；B4 只把同一个 helper、原外部可变状态、完整 key/value/配置及结果原样接上。自有比较、克隆、谓词、授权、退休与释放仍由 V4/V6/V9 证明，不能把整个 FnMut 或业务回调列可信。

`Chain/Selected` 记录每次真实返回与原行选择；公共定理推出完整状态转移、次数、顺序、幸存行及 trace 的完整前缀追加。调用者须提供有效独占借用、普通值且无额外 Drop、每次对应及可表示 trace 容量；正常分配与模型容量不同，不保证物理内存足够。bool 回调没有 Result 错误短路；有业务 Result 的接口须另保留首错。中断只说明已完成调用的历史前缀，不声明当前外部堆/Store 回滚；受控 panic 仅为原生测试证据。

两个真实错误 helper 均重新编译抽取，再由正确版同一 ToggleContract/TraceContract 拒绝。机械外壳的来源审查、原生测试与模型条件各自保留；未获得完整高阶闭包的机器精化。

## T7/T8 有限依赖使用目录的读法

[OS/存储目录](../../verification/os-boundary-catalogue.json)在固定五根、60 个编译变体之上核对 25 类接口、71 个代表源码位置；它没有新增可信保证或程序定理。每行说明原对象/配置、正常结果、错误和部分效果、未知/取消、进展和业务提供方。其 public_status 与待办是 review_baseline 的历史审查快照，历史交付状态见 #333，本阶段适用性见 #632 与覆盖记录；business_obligation 表示仍待业务证明。

本页 A/B 是依赖合同的权威位置。目录中对源码行为的解释仍需其自有前提和业务证明，不能把该行整函数变成可信依赖，也不能把有代表例子理解为所有 SQL/Lua、驱动、Guest 或 Firecracker 调用已抽取。尚未接通的具体签名须给最小预检，不提前保证工具支持。有限目录不提供完整调用图、依赖析构或 Future 取消胶水语义。TLS 原借用输入连接仍是公开缺口，不由现有首叶候选或来源定位自动完成。


## A6/B4 有界 Writer：真实写入、结束和响应选择

[BoundedWriter](../../SandboxModel/Boundaries/BoundedWriter.lean)连接原私有 Writer 的隔离同步候选：`checked_add → limit → capacity → reserve → append`，以及 `exceeded → serializer result → bytes` 的结束顺序。原 `json_response` 的成功/失败选择另行抽取，HTTP 构造只机械转交原字节。[候选来源](../../verification/experiments/bounded-writer/candidate.json)、[完整差异](../../verification/experiments/bounded-writer/candidate-diff.json)和[原语登记](../../verification/bounded-writer-environment.json)固定原 e576a847 源码与具体实现；生产未采用拆分，一般旧新等价未机器证明。

A6 仅提供固定普通 Vec 的长度、capacity、正常 reserve 不改内容、append 原字节及移出原字节；两个固定 IO 错误工厂保留原类别/文本。B4 只连接原 Writer/限额/块/serializer 结果与实际 HTTP builder，不担保限额判断、错误解释或任意 Serialize 正确。实际生成函数证明正常写入的全部字节与计数、历史中已接受块的精确连接、逻辑字节界、原超限优先级和 200 响应的原完整字节。

拒绝不追加本次字节，但不撤销此前私有前缀，回调也可继续写较小/空块。大小加法溢出或超限才置 exceeded；reserve 失败保留原标记，不能统一叫超限。Writer 没有保存任意 IO 错误的全局首错；单次检查顺序、serializer 最终错误和超限优先级分别保留。自定义 Serialize 可以吞掉分配错误再正常返回，此时 finish 可以接受不完整前缀；完整 JSON 仍要求固定派生/实际回调正确传播错误与轨迹条件，由实际 schema/V3/V7/V8 提供。

初始字节界由调用方建立；原入口从空 Vec 开始，预填充调用不自动满足。普通值、有效借用、有限可表示输入与正常分配是适用域，逻辑输出字节界不控制 Vec capacity、分配器、临时数据或整个服务 RAM。panic/unwind、OOM/abort、资源载荷的可观察 Drop、取消和网络最终交付不由正常后态定理提供。真实错误 Rust 删除限额拒绝后成功抽取，由同一 WriteContract 拒绝；库条件有合法非空接受与分配拒绝见证，不以不可满足条件获证。


## A6 BTreeSet 普通键插入

[实际连接](../../SandboxModel/Foundations/BtreeSetBinding/Refinement.lean)和[使用范围](../../verification/experiments/btree-set-binding/model/README.md)限固定 Global 分配器及 `&str`、`(&str, u16)` 普通键，`u64` 只作对照。Rust 仍调用原 `BTreeSet::insert`；五个透明委托的实际生成体已连接集合值模型，不把生产集合替换成 Vec。

| 接口条件或结论 | 提供方和准确范围 |
|---|---|
| 正常插入返回 true 恰好表示原集合没有此键；随后集合包含该键，重复插入返回 false | 固定 Std 的 A6 正常值合同；真实委托与此合同的组合由 Refinement 证明 |
| 后一次调用读取前一次回写后的同一集合 | 原生成函数的移动/回写连接；不能把连续调用改成两份旧快照 |
| 字符串按完整 UTF-8 字节比较，元组先比较字符串再比较 u16 | 固定字典符合性；借用必须有效稳定，元素和长度可表示，所需分配可用 |
| 只允许普通键和 Global 的不可观察内存释放 | A6 使用域；自定义 Ord、allocator、资源键 Drop 和异常清理不在本合同内 |

模型中的列表仅表示集合含有哪些值，不保证真实树形、地址、迭代顺序、比较调用轨迹或物理内存上界。正常返回的合同不承诺 OOM/panic/abort 回滚或无条件进展。`contains/len/collect/iter`、其它键类型及完整 validator 不由插入接口推出；V1/V6/V8 仍证明实际唯一性要求、字段选择、首错和业务状态。22 项正确/辅助目标与2项真实错误版目标分别审计；真实返回值取反版经原工具抽取后由同一 `InsertContract` 拒绝，不把 Std 树算法称为已机器证明。

## A3/B4 Metadata 目录判断

[原函数对应](../../SandboxModel/Boundaries/ProtectedDirectory/Refinement.lean)验证 `protected_files::directory(&Metadata, owner)`。A3/B4 只提供同一份已取得 Metadata 的 `is_dir/uid/mode` 三个 getter，按固定生产/抽取编译器的八份 Std 原文核对；getter 不再次查询文件系统。目录类型、属主比较、权限掩码、短路顺序与 Ok/Untrusted 选择保留原 Rust 生成体。

正常返回 Ok 当且仅当快照表示目录、属主为 root 或指定 owner，且 group/other 不可写，或该目录同时属于 root 并设置 sticky 位。合法私有目录和 root sticky 目录有原函数接受见证；去掉写权限掩码的真实 Rust 副本由相同 `AcceptanceSafe` 性质拒绝。14 项正确/辅助与2项错误版目标使用各自实际生成环境；局部 elaboration 适配仅在两个具名命名空间启用现有标准逻辑实例，不修改生成物或加入业务公理。

本合同不保证 owner 选择正确，也不保证路径解析、首次打开、返回的 FD 身份、查验到使用之间没有替换、读写/同步/关闭顺序或物理退出。V6/V8 提供这些业务和对象条件。后续 `open_checked` 试点的循环/借用翻译仍有阻断；目录快照证明不代替完整受保护读取。原源码归档和八份 Std 原文均是正式复现必需输入，缺失或内容变化不能复用旧结果。

## A6/B2 Tokio Mutex 原借用守卫

[公共模型](../../SandboxModel/Boundaries/TokioMutex.lean)复用 RwLock 的独占写者规则并限制为没有读者的状态。固定 Tokio 1.49.0 的 borrowed `Mutex` 操作仍按 A6/B2 信任，实际异步表达式没有由 Aeneas 机器证明。三份生产源码中的七组操作在[逐点登记](../../verification/experiments/tokio-mutex/environment.json)维护：Session 的 init 创建/使用，以及命令 input 的创建、写入、关闭、安装和完成处理。

| 操作或观察 | 可依赖的保证 | 调用者仍须提供 |
|---|---|---|
| 取得并持续持有原守卫 | 同一锁的其它取得不能同时返回可用守卫；逻辑 ticket 相等不允许复制 Rust 守卫 | 原锁/原守卫/实际操作历史符合登记，各次写入和中间状态保持业务不变量 |
| try_lock 返回 Busy | 没有取得新可用守卫；Busy 也可能来自等待者预留 | 不把 Busy 推成另一个业务任务正在执行或输入已经关闭 |
| 取消尚未取得守卫的等待 | 不凭空释放别人的已返回守卫 | 只描述本等待的守卫投影；取消整个持有资源的业务 future 须核对真实 Drop、I/O 和其它资源 |
| 正常归还或 forget | 归还使旧访问失效；forget 使可用访问消失但锁仍可能一直占用 | 原锁存活；释放事件本身不改 inline payload，不保证解锁后其它写者没有立即修改 |

Tokio Mutex 不提供中毒结果，本合同不承诺 FIFO、无条件完成或 panic 回滚。Session.init 的 payload 是 `()`，不能推导另一把 current RwLock 中的令牌、身份或默认值正确。CommandInput 的 take/替换可能有可观察析构和 I/O；finish 可以在 Busy 时发布完成，Started 不能推成 input 为 Some 或 FD 已关闭。授权、实际写入、部分前缀、退出、资源归还和内部进展由 V8/V9 证明。

21 项目标是条件模型、合法见证和模型反例；实际 Session 提前释放副本由原生同一持锁断言拒绝，不能称作 Rust 形式反例。正常组件测试、实际接线审查和条件模型证明分别报告，不扩大已有 A6/B2 信任范围。
