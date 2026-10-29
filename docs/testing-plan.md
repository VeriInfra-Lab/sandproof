# 系统验收清单

[文档目录](README.md) · [产品范围](project-blueprint.md) · [模块合同](module-contracts.md) · [开发规则](../CONTRIBUTING.md)

本页维护场景与通过判据，不维护逐批进度或机器台账。每次验收在 issue/PR 绑定精确提交、配置、产物摘要、结果和未运行项；原始日志放私有 artifacts 或脱敏 CI 附件。基础、组件、KVM、题集和形式化证明分层报告，不能互相冒充。

## 第一阶段生命周期验收

以下 E1–E16 是第一阶段完成判据；各场景是否通过仅按 issue/PR 的精确版本证据判断，不把旧 S/DC/DK 或两题成绩迁记为通过。首次卸载后查询 Unloaded 的成功只覆盖这条有限链，不完成 E1 的恢复连续性或整个故障矩阵。原基础/兼容/晋升检查继续保留，后文旧 V1 描述是既有工程检查，不扩大第一阶段证明目标。每批记录同一源码、工具/FC/guest/镜像/CPU/内核/文件系统指纹、故障点、操作键、持久与物理轨迹及精确残留。

| 场景 | 输入与故障点 | 必须观察的结果 |
|---|---|---|
| E1 正常连续 | RAM-only 随机 nonce、已修改磁盘、长期 guest 进程；unload 后 resume | 新 VMM 继续同一状态和进程；旧 VMM 已退出、运行名额确实可复用；不能用磁盘文件重建 nonce |
| E2 请求重试 | 同键同全文/不同全文、并发重发、过期窗口 | 同一原结果或明确冲突/过期；无第二身份/执行；未授权 owner 不可查询 |
| E3 捕获前/中崩溃 | 暂停确认前后、Full 文件写入及克隆/摘要/同步窗口重启 host manager | 原唯一 live/frozen 状态仍可继续；无未提交来源导致停机；普通失败不变 destroy |
| E4 文件已发布未登记 | rename/同步完成后 SQLite 提交前崩溃 | 由原计划对账；目录存在不直接授权恢复；无静默遗失或逐出 |
| E5 已登记未收 stop 回执 | bundle 登记后旧 VMM stop 回包丢失 | 保留完整来源和原停止责任；只有真实退出/释放后报 Unloaded |
| E6 已加载未激活 | LoadPaused 完成、MayHaveExecuted 标记前后崩溃 | 未越过激活点的原候选可安全对账；旧工作失效后才允许受控重试 |
| E7 激活结果丢失 | 运行状态已推进，但响应/manager 丢失 | 接续同一原候选，绝不停止后重载旧来源；原 VMM 死亡明确不可恢复，不假成功 |
| E8 终止竞争 | unload/resume 与 destroy/到期并发 | 当前终止否决新资格/发布；在途责任仍可收敛，无旧结果复活 |
| E9 复用与迟到清理 | 旧 stop/delete/回执在位置重新使用后到达 | 不影响新身份；不能只按路径/PID 清理 |
| E10 恢复与 GC | 取 pin、加载、File 映射与 Retiring 交错 | 已接受 reader 获完整材料或新请求拒绝；无 check-then-delete；运行 backing 不被提前删除 |
| E11 空间/部分失败 | ENOSPC、短写、sync/rename/删除失败、提交未知 | 不发布不完整材料；保留责任/配额；解除外部故障后能继续或规定失败收尾 |
| E12 多用户多周期 | 至少两 owner、多会话、重复 unload/resume；有限名额竞争 | 不串身份/状态；材料/运行/临时峰值账一致；正常准入不是全部拒绝 |
| E13 独立重启 | center/host 各在持久/发送/回执窗口重启，重复乱序投递 | 每个可能效果可找到原责任；两库可暂时不同，不能永久丢失唯一账 |
| E14 数据断线与新授权 | 关闭旧连接、保留 guest 进程、恢复后持旧/新凭据访问 | 保留进程继续；旧凭据拒绝、新授权成功；无 TCP/vsock 连续承诺 |
| E15 到期与材料 | TTL 到达时存在 loader/VMM/backing/清理任务 | TTL 进入终止而非直接 unlink；真实使用结束后才回收和退额 |
| E16 实用性测量 | 相同镜像/CPU/工作集，冷/热缓存，反复周期与不同材料规模 | 分别报告 freeze/capture/copy/sync/stop/load/activate/ready 时间，p50/p95/样本数，CPU/RSS/实际磁盘及峰值；不只报 API 返回耗时 |

E7 必须在恢复后制造只存在于最新 RAM 的变化，再丢失激活确认，才能排除“重载旧快照也能通过”的伪测试。E10 包含 load 返回后、VMM 仍运行的时段；E1 包含旧 slot 被另一个会话实际复用。观测 oracle 接受临时两库差异，检查最终事实、责任和禁止状态，不要求每瞬间强一致。RAM/文件见证使用明确屏障或描述允许捕获点；不能把应用的两个非原子写入强行要求为永远相等，从而误报协议错误。

独立生命周期实机入口 `tests/system/lifecycle_unload_scenario.py` 使用两个保留会话、两个运行位置：A 卸载后让 B 占用原位置，再恢复 A，核对 B 未受影响，随后销毁 B 并检查 A 再次卸载/恢复及回收。会话保留数直接传给中心账本和宿主 inbox，不从任务数推算。该入口可使用已有基础模板；两题评测的 10 GiB 模板不是此场景的前提。按实际模板大小和 2 GiB VM 内存计算完整包络，继续检查原有限激活上限、实际可用字节/inode、同文件系统以及材料项目硬配额。该场景不代表两题评测或 E1–E16 全部通过；实际部署和结果绑定精确提交另行记录。 实机服务使用优化构建（`opt-level=2`），显式保留调试断言和溢出检查，并记录构建输入与实际二进制摘要；基础单元测试仍走原入口。未优化程序在受限 CPU 下的超时不能直接当作产品性能结论。

同一生命周期入口用 `--scenario` 明确选择 `normal`、`restore-activation-loss` 或
`restore-candidate-exit`。后两者复用受保护的激活观察门，先确认原持久激活标记与只存在于
最新 RAM 的变化，再丢失管理服务的激活回执。存活分支必须接续同一 VMM/netns 和来宾进程，
继续完成两轮恢复；退出分支只运行一个故障恢复周期，由隔离测试监督器先核对原进程身份、
持有 pidfd，再仅对该句柄发送退出信号并确认内核退出，不用 PID 缺席或超时代替退出。
随后检查公开 `StateLost`、同请求重放及管理服务重开后仍为同一结果、旧授权及本次恢复
凭据拒绝、原恢复记录/材料引用保持且未出现替代候选、另一用户会话不受影响，最后由公开
Destroy 和原产品流程完成 Cleaned、物理释放及账本结算。故障注入不修改账本或删除进程/
材料记录；退出本身不是释放位置或材料的凭据。该分支不认领正常两周期或任意故障后的
状态可恢复性，不能关闭存活分支的原断言。

`restore-prepare-destroy` 先核原 LoadPaused 候选，再持同一 pidfd 暂停该 VMM 进程，
使原只读 API 观察暂时不可用。只有真实 Destroy 已进入宿主、原终止围栏已安装且原
Prepare 仍未完成时，才恢复这个进程供原取证/Stop 流程推进；SIGCONT 不等于激活 vCPU。
原另一会话的查询在该候选仍停止期间必须返回。随后复用已有 Cleanup 成功但 SQL 回执
未记录时的管理服务退出点，稳定读取原 Prepared/Stop 完整回执及精确进程退出；未决
Cleanup 阻止明细退休，不靠轮询碰巧抢在 GC 之前。重开后沿原 Cleanup 观察、公开
Cleaned 与产品结算完成收尾，不用缺失记录或测试信号冒充成功。

失败卸载后继续原 VM 再退出，由同一入口分别选择
`continued-after-capture-exit` 与 `continued-before-admission-exit`。前者用受保护的
测试门，在原工作器 Ready 和 SQL 授权之后、Full Release 之前，核对完整动作、Full
摘要和真实工作器，再取消原等待；必须由该工作器自己持久记录 `NoRequest`，不能由
监督器写入结束事实。随后沿产品继续协议确认原 RAM、来宾进程、凭据和寿命未重置。
后者先保留合法来源及失败工作责任，在同一正式材料预算内触发下一次准入拒绝，并
核对确无新的本地 Pause/Full；若原合法回收使所需责任不再占额，须记录场景未到达，
不能暂停回收、降低容量检查或伪造记录来凑出拒绝。

两者都只对已核对身份并持有 pidfd 的原候选注入退出，检查 StateLost、同请求重放、
双管理重启、旧凭据拒绝及另一会话不受影响；历史 `OriginalContinued` 保持不变，
没有替代加载，也不能先清 pin 或退额。最后必须沿公开 Destroy 完成产品回收。
两种资格分别登记到达证据：未准入路径不得借不存在的 Full，已有捕获路径不得借旧
Resume 围栏。存活、错误工作/围栏/进程、观察未知及观察期间终止的拒绝另由确定性
组件覆盖。新增场景的实际执行状态按精确候选回执判断，不由入口存在推断通过。

再次卸载的组件必须经过公开的 create→unload→resume→失败卸载继续→新键 unload，
同时核对原历史和接受结果；不能只手造已接受意图后调用宿主准入。受控 Host 必须区分
旧 Create 与当前 Resume 的真实观察来源，错误来源返回 NotCurrent，不能对任何旧
Create 都返回 Observed 而掩盖路由错误。新的运行观察应核对已有继续事实，不改写历史。

续期组合在同一公开链的 Resume 与失败卸载之间加入原 `set_timeout`：续长后越过旧来源
期限但尚未到当前期限时，新键卸载仍可接受；缩短后即使旧来源尚有效，到达当前期限
也必须拒绝。核对原来源和继续历史不变，并保留错 owner、错全文、终止抢占及迟到观察
的拒绝条件。需要以真实 Redis 组件区分修复前后的同一断言；不把这项组件结论记为
真实虚拟机或任意续期交错的形式化证明。

在途数据写入代表复用正常两会话入口中的原 VM。先发送一个声明正文尚未完整的真实
上传，实际读回已写前缀，并核对原持久 route 与仍存活的精确中继。卸载撤销后，调试
屏障只停在该中继原停止流程之前；必须观察后台确实推进了同一完整卸载请求并因尚未
排空返回 Busy，而非在后台尚未运行时检查一次数据库。屏障期间不得派发 Freeze/Seal，
旧授权必须拒绝；放行后只沿原停止/等待与许可释放流程，再完成捕获、重启和恢复。
恢复后核对真实前缀、旧凭据拒绝及新凭据可用。上传没有发送最后一个字节，不报告上传
成功、原子写入或自动重传。独立 body/relay 两种释放顺序另由实际 SQLite 与子进程组件
覆盖；组件通过不能替代这一物理代表。屏障仅用于隔离调试验收，不提供退出或业务资格。

未知加载与终止的组件分两阶段检查。第一阶段保持 Prepare 未知，销毁/到期及重开不能
清除原加载责任或 backing 引用；此时阻塞可以是正确行为。第二阶段解除故障，通过原
只读恢复入口提交同一 Prepare 的确定观察，必须接续原停止与清理，不能再次 Load 或
Activate。分别核对显式 Destroy 与到期补偿，以及来源摘要/代数错误的拒绝。Resume 的
补偿清理计划排除已确定回收的旧代资源时，还须验证存活、未回收、当前代和记录缺失的对象仍被
保留，并运行原正常恢复及再次暂停回归。受控物理提供方的组件结果与实际 LoadPaused
回执丢失场景分别报告。

上述有限代表使用同一实机入口的显式 `--scenario`，默认 `normal` 不变：

| 选择值 | 实际到达点与接续要求 |
|---|---|
| `data-drain` | 首次卸载前持有实际上传中继；原驱动已经尝试推进但尚未排空，不得派发 Freeze；结束后按正常链恢复前缀 |
| `capture-freeze-receipt` | 原 Pause 已返回、SQL 尚未收到回执；重开原管理服务，保留同一 VM 与原卸载责任（E3） |
| `capture-measurement-partial` | 当前 CoW 路径在 Memory 整文件克隆完成后、目标摘要尚未读完的实际 `Measuring` 窗口注入；原工作器记录真实 Returned End，失败后继续原 VM，再显式销毁，不称第二次 Unloaded，也不把该事件称部分复制写入（E3） |
| `capture-seal-receipt` | Full 包和原物理完成记录已发布，SQL 尚未采纳；按原工作核对完整材料并接续，不能仅凭目录存在授权恢复（E4） |
| `capture-stop-receipt` | 材料已采纳且原 VMM 已确切退出，SQL 尚未收到 Stop 回执；重开后按原责任完成卸载（E5） |
| `restore-prepare-receipt` | 原候选实际 LoadPaused，SQL 回执和激活标记尚未记录；接续同一候选 PID/代数/VM，不能重新加载来源（E6） |
| `restore-prepare-destroy` | 在同一 LoadPaused 候选的观察暂不可用时，经原公开接口接受 Destroy；先确认宿主终止意图及真实终止围栏，再取得原加载事实并沿原 Stop/GC 收尾，禁止第二次加载或激活（E8） |

捕获点复用原第二次卸载，LoadPaused 点复用原第一次恢复；不为每个错误码新建虚拟机。
命中故障开关、退出码 86、发送 gate 命令都不是通过条件，仍须比对完整请求、动作、
来源、实际工作器/进程与持久记录，并完成各分支的公开结果和原产品回收。测试控制文件
仅在原物理完成及产品 GC 后核对原身份退休；未知或失败保留原始证据。

实机 Redis 等待预算通过公共参数 `--redis-operation-timeout-ms` 显式配置，默认 1000，范围 1–60000 毫秒。先定位同步落盘、调度或资源争抢；确认为部署负载与预算不匹配时，才根据证据选择该环境的值，不能因失败直接加大。参数在创建资源前校验，进入原中心配置及其摘要；生命周期报告的 `measurement_identity.redis_operation_timeout_ms` 核对并记录实际值。它不改变 `appendfsync always`、写前后健康检查、未知结果围栏、HTTP 状态断言、请求/场景期限或寿命，也不允许重试未知写入。该值是存储操作的等待配置，不是整个 HTTP 请求的硬墙钟上界；一次查询可包含多次顺序确认，验收须同时检查外层期限。某一环境通过不能说明任意存储负载都满足该预算。

### CoW 文件与恢复组合

固定候选先核对运行根、store、恢复私有目录所在的同一实际挂载、reflink 和项目配额
能力；普通测试沿同一受支持文件系统建夹具，不用 skip 或复制回退让不支持环境通过。
能力不符须在放行捕获/恢复前明确拒绝；能力预检不能代替每次实际克隆/同步错误处理。
初始化须实际完成私有双文件 clone/读回/同步/精确删除，之后才发布 version 2 布局；
旧 version 1 拒绝且无自动迁移。对原公开 `reopen` 检查正常打开→同设备不同 bind mount
拒绝→卸载该别名后恢复正常，原布局字节不被改写；mount ID 仅在当次 FD 上观察。
探针或后续发布失败后保留原目录锚点/部分文件，不能自动清空后伪装为首次初始化。

真实文件检查覆盖 state/RAM/磁盘的原字节、完整长度/摘要、稀疏区间、不同 inode、单链接、
原属主及只读/可写模式。分别改写源与目标、unlink 一方后核对另一方仍保持原内容；
测试可由自身文件的可信属主显式改权限制造写入，不放宽生产封存模式。覆盖重复目标、
符号链接/硬链接、替换、跨挂载/不支持、配额不足以及第二个文件失败，保留已创建文件
和原工作责任。源 stamp（含时间字段）按原判据检查，观察到合法变化时先分析裁定，
不能为测试通过直接删除检查。

捕获故障点须对应真实克隆前后、目标摘要读取、同步与发布窗口；不能将摘要读取进度
标成复制写入。恢复失败不得发布 Loaded/Ready、解除 source pin 或回退另一个 capture。
事件选择使用同一 `SnapshotArtifactEvent` 及原 artifact/work 身份；`Cloned` 不等于
`Synced` 或完整 bundle 发布，Measuring 的已读/总字节须来自原目标而非测试自己推算。
该中途摘要门只接受当前 schema 2、`measuring-cloned-artifact` 阶段和
`0 < measured_bytes < total_bytes`，不保留旧 `capture-copy-partial` 选择值或 copied 进度别名。
历史 #712 的逐块复制故障证据保留原提交身份，不改记为本次克隆窗口已经通过。
一次固定候选的定向真实生命周期仍检查 RAM-only nonce、guest 进程、磁盘和原运行身份、
位置/材料回收；恢复后写入不得污染封存来源。已有工程链只能按实际输入范围复用，
不将旧逐块复制路径的实机结果直接记成新 CoW 路径通过。

小样分别记录逻辑长度、实际分配/共享块、后续写入增长、配额结果、读写量和耗时；
保留 Firecracker Full 写 RAM、目标全量摘要读取的成本，不预设零读写或加速倍数。
普通/文件组件、真实 KVM、形式证明及完整论文矩阵分别报告；本项不提前回收仍被运行
VMM 使用的材料，自动回收按 #745 独立实现和验收。

### 同一性质拒绝的错误版本

隔离副本分别引入：捕获失败就杀原 VM；快照未提交先 stop；激活未知后重新 load 旧来源；检查引用后无保护删除；load 返回立即释放 backing pin；超时/进程查询缺席即退额度。负例必须通过语法/抽取并到达错误分支；预期证明目标不可满足或同一可执行性质产生反例，不能把编译失败、环境超时、改弱断言当拒绝成功。不把错误版本合入生产。

### 论文证据与环境

普通单元/组件/CI 在测试机 ning，形式化验收在 msra；地址、路径和凭据仅存私有操作记录。KVM 场景使用按开发指南批准的专用隔离环境，运行前核精确候选与全部资源；不得把普通证明机默认当特权 VM 测试机。所有原始证据保留，按同一输入复用已有检查而不混用不同版本。

LC-G1–6 的全称证明、LC-I1–10 实现连接与 E1–E16 实测分别报告。额外统计真实证明覆盖、外部条件/自有可信适配的代码规模及剩余缺口；错误率/延迟/开销不预设结论。与已有系统比较时固定版本和能力边界，公开报告、历史已修错误、本地模型候选和真实产品复现分开；不擅自测试外部生产云。

## 1. 系统场景

| ID | 场景 | 通过依据 | 主责 |
|---|---|---|---|
| S01 | 原客户端创建、连接、查询、销毁 | 同一实例；真实停止后才返回销毁完成 | C+A+B |
| S02 | 多次命令、后台进程、输出及非零退出 | PID/句柄、输出和退出码正确，后台任务不会被误重复派发 | B+C |
| S03 | 上传下载、修改系统文件、安装依赖 | 私有系统可写、共享基础镜像不变、宿主文件不被越界修改 | B+C |
| S04 | 两用户、两沙箱交叉访问 | 未授权管理、命令、文件和网络请求拒绝，资源不混绑 | A+C+B |
| S05 | connect、set_timeout 与到期 | 延长/缩短语义正确；旧定时任务不会销毁已续期实例 | A+C |
| S06 | create 与 kill、并发重复 kill | 不发布虚假成功；最终无运行残留，无误杀 | A+B |
| S07 | VM 启动后回执丢失、重复派发 | 恢复认出同一资源，不再启动第二台 VM | A+B |
| S08 | controller/worker 在关键窗口重启 | 提交前无外部动作；提交后能重放/对账；未知结果不丢失 | A+B |
| S09 | 旧资源清理迟到、标识被系统复用 | 只处理绑定身份，不影响其他或新资源 | B+A |
| S10 | 禁网、公网/静态规则与资源限制 | 实际探测不能绕过策略，不能访问宿主管理或其他沙箱 | B |
| S11 | 磁盘/配额不足、恶意大输出或文件 | 新请求合理拒绝，收尾仍有空间，宿主与其他沙箱不受污染 | A+B |
| S12 | 客户端超时及命令通道断开 | 状态未知可观察，不自动重复执行有副作用命令 | B+C+A |
| S13 | Dockerfile/OCI 构建、模板创建、执行、评分与清理 | 原环境定义经通用模板链准备，不按题目追加产品行为；全新实例、原解答/评分、真实资源与精确收尾；报告实际适配范围 | B+C |

每个测试至少关联一个场景或明确缺陷。新增测试只补实际需求或缺失证据，不无限扩展矩阵；成功、拒绝、未知、失败与未运行分别报告。

S02/S11/S12 的命令输出验收采用持续流策略：超过旧累计上限仍完整成功且只派发一次；同时保留请求、文件和一元回复的边界测试、断线重连、背压及撤销测试。宿主辅助工具另测 stdin 阻塞、双输出互堵、无输出不退出、stdout/stderr 超量、早退、正常退出与原进程精确回收；未知退出保留额度使用受控观察点测试，不冒充真实不可中断内核故障。实机启动/初始化、网络观察、启动首帧和销毁收尾须绑定同一提交并检查残留；这些工程验收不代表形式化证明。

选定实机入口复用[外层监督器](../tests/system/server_slow_boundaries.py)的 `--command-output-only`：两个全新隔离 VM，固定同步/异步 SDK 各收完整 33 MiB 输出、核对单次派发与模板启动首帧，然后按原 VM/目录/网络/cgroup 身份销毁并检查残留。该选项不运行原慢文件场景；不冒充完整故障矩阵、完整题集或真实不可中断进程故障。

## 2. 模块与故障检查

| 模块 | 基础/组件重点 | 相关场景 |
|---|---|---|
| A：核心与持久服务 | 顺序拒绝、归属、同键改参、旧代数/尝试、重复回执；真实 SQLite 提交前后、不确定 COMMIT/ROLLBACK、缓存差分与恢复 | S01、S04–S09、S11–S12 |
| B：宿主与数据执行 | 固定资源身份、目录替换、持有者排空、未知观察、磁盘/网络/限额、迟到清理、上传与中继收尾 | S02–S04、S07–S12 |
| C：客户端与公共框架 | 固定 SDK 字段/错误/默认值、get/list 筛选排序分页、寿命、命令/文件协议、模板及 Harbor 全链路 | S01–S05、S12–S13 |

并发使用屏障和明确故障点，时间边界使用可推进时钟。基础检查不创建产品 VM，不访问付费模型或 E2B 云服务；命令见开发指南。功能测试不产生证明结论。

串行边界诊断用[应用组件夹具](../controller-service/tests/application_lifecycle/contention.rs)：两位置完整配置、真实 SQLite、受控宿主与时钟。屏障将一次准备操作至少阻塞 25 ms，分别观察另一实例的正常/过期鉴权、续期和到期清理；比较新库与 12 次创建/销毁/维护后的检查点，输入尾上限固定 64，并核对实际裁剪。运行 `cargo test --locked -p sandproof-controller-service --features contention-diagnostics --test application_lifecycle contention:: -- --nocapture`，输出请求、锁等待/持有和受控工作执行耗时；基础检查也覆盖默认关闭及显式开启两种编译。数字不是实机宿主成本或吞吐承诺，不设置平台相关的性能通过阈值。慢操作的等待影响须如实记录，但不能据此取消安全锁；并发优化需另行决定。

### 持久化、身份与有界运行

来宾账户/上下文的[纯判定与阻塞查询测试](../sandbox-guest-service/src/execution/tests.rs)覆盖 Basic 用户选择、home/目录、环境优先级、预算和未知账户；屏障与可推进时钟验证取消/超时不会提前返还仍在运行的查询额度。[真实 worker 组合](../sandbox-guest-runtime/tests/context.rs)从初始化令牌许可解析上下文后执行真实命令，核对完整环境、数值身份、目录和私有 socket 回收；USER/HOME 覆盖不成为 UID/GID。普通检查及 SDK 脚本执行这些场景，但当前账号的成功不能证明 root 到普通账号的切换。

单独的[特权身份组件验收](../tests/guest_credentials.py)要求已复核的精确提交、worker SHA-256，以及新 mount/PID namespace 的 root PID 1 和独立 proc。入口将原二进制核对后复制到本轮 root 独占文件再执行，只创建/清理自己的临时目录与子进程，不修改宿主账号。分别使用管道与 PTY 检查真实/有效/保存/fs UID/GID、附加组、能力、普通账号的组文件访问、root 文件拒绝及不能恢复 root；PTY 另核对三个终端描述符及主进程的独立会话。另以私有用户命名空间制造 setgroups/GID/UID 失败，验证没有用户副作用。它不由普通 CI 自动获取特权执行，结果单独绑定提交和 worker 摘要，不代表 KVM、完整来宾服务或抵抗来宾 root 的隔离证明。

[命令所有权检查](../sandbox-guest-runtime/tests/owner.rs)使用真实 worker、管道和 pidfd，覆盖多线程初始订阅先于输出、二进制/非零退出与 End 顺序、断线/重连不关闭 stdin、阻塞输入与持续输出下的原执行期限、单个慢订阅失败、在途块到实际额度释放、全部客户端句柄丢弃后的继续管理、预过期无副作用、exec 失败和启动结果未知不重执行。SIGSTOP/waitid、实际输出和可推进时钟建立顺序；真实 I/O 段使用正常时钟，避免假时钟抢跑子进程。丢弃未轮询任务验证 `owner_lost` 和未决额度保留；不将它当作真实 envd 服务强制重启/PID 复用验收。普通基础检查和 Linux SDK 脚本执行这些组件，公开注册/错误映射、保留 TTL、整机内存峰值及 KVM 仍单独验收。

[准备与登记交接检查](../sandbox-guest-runtime/tests/preparation.rs)同样纳入上述两条入口：真实 worker 验证登记确认前无用户副作用、同一额度移交、授权轮换等待至派发、确认接受前后取消的不同语义及丢失回复不重执行。受控握手 socket 与可推进时钟覆盖握手取消、坏协议、绝对登记期限；实际 pidfd/wait 核对中止后退出、私有 gate 回收及容量恢复。测试主动提前 wait 自己的原 child，验证管理者缺失回执时保持 `CleanupUnknown` 和占额；关闭管理 runtime 区分确定未 spawn 与可能已有 child 的责任。没有模拟强制 PID 复用、Linux D 状态、故障文件系统、完整服务重启、Go envd 双端对照或 KVM；这些不由组件绿灯核销。

[阻塞准备回归](../sandbox-guest-service/src/command/preparation/owner/tests.rs)使用原 owner 流程、单阻塞线程、明确屏障及可推进时钟，核对排队时请求取消、等待者丢失、显式撤回和到期均保持原请求/记录额度直到队列项退出，并且不调用 spawn；已进入阻塞工作后丢失异步等待者，线程实际结束前不得归还容量。原同步错误确认无 child 后归还，受控 panic 则保持 OwnerLost 和隔离记录；线程身份另确认工作不在异步调用线程上。此处以故障点替代内核 spawn，真实管道/PTY/worker 仍由原 preparation/process/SDK 组合重跑。原 PID 排队测试改为先准备两台独立 worker 再占用共享阻塞池，使其继续专门验证单写入名额；第二台保留在原观察点的同等剩余期限。这两项阻塞准备回归及原真实组件已在 [#501 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38802891781) 实跑通过：源 59b13e9fbce0078a9231ab84a472ec80c1193154、checkout 174bfbd2afff73b1dda0116a5c26ad246ab39e01；service lib 445 passed / 73 ignored、preparation 59 passed / 3 个实际调用的 helper ignored、process 25 passed，固定 SDK 109 项、封存 Redis 及四项 CI 成功。此回执不覆盖之后 cwd Stat 移动的候选。

准备失败的后续回归在同一单阻塞线程中先用屏障提交准备，再用队尾工作确认失败闭包实际返回，保持异步父任务不再轮询。原代码此时可复用记录；修复后必须继续拒绝，直到父任务处理或丢弃已完成结果。六种组合覆盖原同步失败、排队撤回、排队到期，以及正常结束/取消父任务，原请求释放和最终记录恢复分别核对，结果未消费前不提前发 Stopped，收到通知时原记录已经可用。这项回归不使用真实 spawn，不沿用上述历史 CI 回执背书新实现；真实成功/失败和未知清理仍由原组件与实机入口验证。

[gate 路径退休检查](../sandbox-guest-runtime/tests/process/gate.rs)在真实 READY 后使用原已连接句柄，核对首次 release 删除 launch 路径与空目录、命令只执行一次及实际输出/退出。成功删除后重建同名目录/文件，检查重复退休和原 owner Drop 均不删除该新路径。另在本次私有 gate 中放置夹具文件制造真实 rmdir ENOTEMPTY，检查 release 仍使用同一连接，额外文件保留至原进程 owner Drop 后才由 TempDir 回收。[取消放行回归](../sandbox-guest-runtime/tests/process.rs)以 SIGSTOP/waitid 保持 worker，轮询一次 release 再取消，要求 gate 已退休但 launch 仍为 Unknown，后续禁止再次放行，SIGCONT 后仅出现一次效果。准备登记测试保留放行前 gate/容量，Start 后核对路径已退休而进程额度仍占用。下述真实所有者强杀场景改为要求进入放行后的旧 gate 为零；这些 Linux 断言已在下述 #475 候选 CI 通过，不覆盖握手前崩溃或全部文件系统清理。

[持久 PID 检查](../sandbox-guest-runtime/src/pid_claims/tests.rs)以真实私有临时文件覆盖回执丢弃/启动失败后的永久占用、重开及新会话不复用、有限范围/全满、VM/范围错配、缺失/截断/坏校验/非规范位、权限/链接/多写者和原名称替换。写前、位写后、摘要后和同步后故障点以及 panic 验证未知结果不返回回执，旧位保持，损坏文件不自动修复；六项可移植测试已在 macOS 运行，不冒充断电/故障磁盘实测。入口为 `cargo test -p sandproof-guest-runtime --locked --lib pid_claims::`。Linux [真实准备连接](../sandbox-guest-runtime/tests/preparation/pid_registration.rs)新增三项测试：保存后才实际 exec、结束/重开仍拒绝 PID，重复/错 VM/繁忙/丢弃不执行，单阻塞线程排队取消不写记录，并在原 worker 超时且进程额度已归还后继续保留独立存储名额、拒绝并发登记；使用原 pidfd、明确屏障及实际 wait/私有 gate 清理。Linux 交叉 Clippy 已通过；尚无公开命令注册表、PID 复用内核交错、服务强杀/重启仍存活的完整 E04 或 KVM 证据。

#428 首次 CI（源 e7447ac4e4f0b7ef9bad331ed3af80b74c8bc209，实际 checkout a01e833）在旧文件传输 SDK 用例的 QP 上传错误后立即读取时失败，未运行到 PID 连接测试：收到错误响应并不保证被取消的阻塞写者已经归还唯一传输名额。单槽顺序夹具现通过 FileStreams::available_slots 的真实池快照，在下一次请求前及结束后限时等待实际归还；没有固定 sleep、请求重放或增加池容量。该屏障只属于回环测试，生产准入仍立即 try-acquire；快照不是文件持久化或 VM 清理回执。修正后的源 c9629f9244983d6e2169cd4827200708a1b50114 已在 [#428 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38533020885) 通过（实际 checkout ba9821350613b42be72769a39accd6b3558e442f）：六项真实文件 PID 测试和三项真实准备登记测试全部通过，preparation 共 15 passed；原文件 SDK 496 次请求、service lib 320 passed / 50 ignored 及封存 Redis 通过，四项检查成功。该证据不覆盖正式命令路由、完整 E04 或 KVM。

[PID 登记检查](../sandbox-guest-service/src/command/registry/tests.rs)增加无效配置、元数据/条数满额拒绝及未轮询 owner 丢失保留未知责任；[真实连接](../sandbox-guest-runtime/tests/preparation/pid_registry.rs)增加四项 Linux 场景：受理前取消实际停止原 worker 并保留永久 PID 位，受理后 Start 回复丢失仍连接原 stdin/输出，原始订阅不丢首段及 exec 失败不变成成功终局，错 VM/会话不执行。真实结束/排空时间绑定保留起点，假时钟验证第 29 秒可重连、第 30 秒拒绝，旧外部句柄仍占进程额度，登记记录实际退休后才可再次预留。使用原 pidfd、受控输入和确切完成观察，不以随机 sleep 假定收尾。新增六项测试已在 [#430 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38535708180) 实跑通过（源 4779354d72cd7c7b6625b9427710b045d0fc8165，实际 checkout ae26e678a13b08ccab31eca37c6ec8be024d4927）；该次四项 CI、原 SDK 命令/文件与封存 Redis 检查均通过；没有 tag/HTTP 路由、强杀重启保留原进程或完整 E04/E05 证据。

[所有者进程崩溃检查](../sandbox-guest-runtime/tests/preparation/owner_restart.rs)在独立监督子进程中执行真实所有者与 gated worker/登记/持久 PID 组件；只有该监督进程设置 subreaper，不改变并行测试主进程。私有 FIFO 和有限 socket 回执确认命令已执行一次、原 PID/start_ticks 已公开且仍存活，再通过原 pidfd SIGKILL 所有者并实际 wait。重开同一 PID 文件、建立新会话登记后，旧 PID 的 live/connectable 查询均拒绝、同号再次 claim 为 Reused，而原命令的 pidfd 非消费 wait 仍显示存活；新会话可完成另一合法命令；其真实退出后仍占登记/进程名额，复用第 29/30 秒保留检查推进时钟，恢复正常时钟并等原记录实际退休后才核对容量。最后释放原 FIFO，经被收养的原 pidfd 等待真实退出并回收，分别核对一次效果、新命令效果及新命令 worker socket/容量。当前候选在首次放行前退休原 gate 路径，强杀回归相应要求旧 gate 为零；外层夹具仍只删除本次私有临时树，不把测试清理记为正式重启服务的清理能力。普通 preparation 测试会调用两个显式 ignored 的私有子进程入口，它们不是未执行的场景豁免。该场景已在下述 #474/#475 CI 通过；只覆盖组件所有者的真实进程死亡与登记重建，不代表正式 daemon/config 重启、强制 PID 复用、完整 HTTP/SDK 或 E04 整体验收。

#474 的真实所有者崩溃候选已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38672549530) 通过（源 f1a906a8d8cd7776fb63dd960d4aebfaa81193c2、checkout 27ad0c99660ca82ce8d6efd65c3cd463609e49c1）：process 22 passed、preparation 55 passed / 2 个实际调用的私有 helper ignored、service lib 414 passed / 69 ignored；77 项 SDK、八轮 PID/80 个操作错误调用、封存 Redis及四项检查成功。此前夹具权限和保留期失败已修正，生产规则未放宽。该版本明确观察一个强杀遗留 gate。

#475 在上述基线上完成 gate 清理及成功删除后解除旧 TempDir 析构责任，已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38673355770) 通过（源 65dfdc033fc58605a99a9e7454a69c150ee62572、checkout ab21318763482c56a91ae46340ee8bd83c1b5cda）：process 23 passed、preparation 55 passed / 2 个 helper ignored、service lib 414 passed / 69 ignored；原 SDK、八轮 PID/80 个操作错误调用、封存 Redis及四项检查成功。实际覆盖 ENOTEMPTY 回退、成功删除后同名路径重建不被旧析构删除、取消后 Unknown 与强杀后旧 gate 为零；仍不是握手前崩溃、故障文件系统、正式 daemon 重启或 E04/E07 整体验收。

[准备请求所有权检查](../sandbox-guest-runtime/tests/preparation/request_ownership.rs)新增三项 Linux 实进程场景：原请求已到期时不 spawn；较短请求期限中止准备，而未丢弃票据仍持有请求额度；单阻塞线程排队的 PID 登记被取消、原 child 实际回收后仍占请求额度，闭包退出且文件锁释放后确认没有烧掉 PID；SIGSTOP/waitid 建立登记后的原 worker 暂停点，确认接受到实际放行期间不归还请求额度，恢复后后台命令继续持有独立进程记录。使用真实 pidfd/退出观察、明确屏障及可推进时钟，真实收尾前恢复正常时钟；三项已在 [#437 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38559610341) 实跑通过（源 f2626cea8624be04330951a4d815dcd10862e562，实际 checkout 6dee2c3ecd66e090343b7a19b7ce53ea8c1e8aea）：preparation 25 passed、service lib 341 passed / 50 ignored，原 SDK/封存 Redis 和四项 CI 均通过；后续 Start 装配须另验。公开 Start、tag、HTTP 及完整 E04/E05 仍待验收。

[Start 预检组合](../sandbox-guest-service/src/command_requests/tests/start.rs)新增三项 Linux 单元场景，核对 Basic 账户只查一次、缺省账户失败先于非法命令和 tag 预约，以及默认账户查询取消/到期后的原授权/请求保持。[真实启动组合](../sandbox-guest-runtime/tests/preparation/start_dispatch.rs)新增三项 Linux 场景：原默认 cwd/环境和请求覆盖进入实际 worker，Start 回复丢弃后通过原 PID 重连并往返全部 byte、只执行一次 shell；stdin=false 的真实 EOF、初始 PTY 尺寸窄化及终端输入；首帧超限/登记满额/过期/错 VM 持久登记均无用户效果并实际归还容量。现由 service lib/preparation 入口发现，原提交已在下述 #438 CI 实跑；后续候选须独立重跑。该历史证据不覆盖后续正式 HTTP/tag 分派、Go 双端同调用集或完整 E03/E04/E05。

[共用期限检查](../sandbox-guest-service/src/connect_timeout/tests.rs)增加两项可移植检查，区分同一首字段的执行/HTTP 期限：空值、带符号零、负数、正数和大值，重复字段首值选择、非法首值不能被后值修复，以及符号计入十字节界。命令原执行时钟测试同步改为固定 Go 语义，仍核对原始起点和不可表示期限拒绝。原文件 SetTimeout 的 533 项显式 Go 对照继续回归；该对照验证解析投影，不是完整 Start/Connect HTTP 生命周期对照，也不表示非正 wire 期限可执行命令。

[命令响应体检查](../sandbox-guest-runtime/tests/owner/body.rs)新增七项真实进程场景及一项受控退出编码检查：二进制 stdout/stderr、非零 End 与独立终结帧顺序、终局重连不重放；累计输出大于单消息预算；保留 Bytes 克隆期间禁止预读并持续占请求/订阅/记录额度；假时钟 keepalive 和持有帧到期不终止原命令或关闭 stdin；慢订阅与编码超额不伪造成功终结；无效配置拒绝。core-dump 真/假在受控结束值上检查，没有制造真实 core 文件；原 Linux 信号及 wait 场景另核对真实状态。固定 SDK 夹具新增同步/异步管道及 PTY 四次调用，经过原执行 owner 和新异步响应体，检查 HTTP 发送后额度与实际进程收尾；原十次低层编解码调用仍保留。新增组件与 SDK 用例已在 [#431 CI 第二次运行](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38537986048/attempts/2) 通过（源 dabac36c87beb050d61af6032e24fc0160238ab7，实际 checkout 4854dcd11f075888965260a69b519b18fa94e133）；第一次 SDK 步骤通过但后续 Redis 步骤触发任务 15 分钟期限，第二次四项 CI 与封存 Redis 均通过，没有放宽检查。该证据不计为正式命令路由、Go 双端或完整 E03/E05。

[命令请求准入检查](../sandbox-guest-service/src/command_requests/tests.rs)覆盖无队列容量、每请求 token、头部/正文/帧/trailers 边界、原绝对期限、阻塞解码取消后的实际授权/容量保持；[Linux 预检](../sandbox-guest-service/src/command_requests/tests/preflight.rs)核对显式 Basic 先于 JSON、默认账户延后、阻塞账户超时后仍占额、派发后授权释放与 Bytes 克隆保留额度、未轮询一元正文到期，以及 PID/tag 缺失及错误 Start 分派入口的组件拒绝。[真实 PID 组合](../sandbox-guest-runtime/tests/preparation/request_dispatch.rs)通过原持久登记与 worker 检查 List、Connect、二进制输入/空输入、关闭后终局重连、坏 token/过期请求/未知信号不改变原进程，以及 PTY 尺寸窄化和终端输入。六项可移植检查已实跑；五项 Linux 预检及三项真实进程检查已在 [#433 汇总 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38549727042) 实跑通过（源 3f417a2911fe15c0ee42dc8fd21368fb83be8e6f，实际 checkout c7ad7f542ea302477669be0c1014f6028f113667）：preparation 22 passed、service lib 339 passed / 50 ignored，原 SDK/封存 Redis 及四项检查均通过。该候选修正成功退出 status 的完整断言及暂停时钟跨毫秒刻度的测试等待，生产期限未改；该证据不覆盖后续准备请求所有权变更。它们由已有 service lib/preparation 入口执行，不是公开 HTTP negotiation、固定 SDK 全操作双端对照或完整 E03/E04/E05。

[固定 Go tag 观察](../tests/compat/connect-reference/command_tags_test.go)复制原 getProcess、lookupTerminated、clearTerminatedForTag 的方法体，仅替换接收者/记录类型；使用原生成选择器与 Connect 错误，显式安排 Range 访问次序，Load/Store/CompareAndDelete 仍用真实 sync.Map。四组检查已在固定 Go 1.26.8 / 64 位本地通过：12 个存活表用例覆盖唯一目标漏查、重复目标顺序、nil/空 tag 和错误文本，5 个退出表用例核对首匹配，另测读取后替换记录的清除保护及 PID/缺选择器路径。入口为 `cd tests/compat/connect-reference && GOTOOLCHAIN=local go test -mod=readonly -run '^TestFixedGo' -count=1 -v .`；普通 CI 不自动运行 Go 观察。它固定的是原方法在受控遍历下的结果，不运行原 Start、计时保留、HTTP、真实进程或 SDK，不作为 Rust 新规则的相等性判据；owner 已批准的确定性差异在模块合同维护。

[确定性 tag 登记检查](../sandbox-guest-service/src/command/registry/tests.rs)以线程屏障核对并发预约只产生一个成功者、取消释放、空 tag 与未提供字段的区别、元数据/容量拒绝不留下预约，以及 owner 丢失后查询/复用均拒绝。[真实 worker 连接](../sandbox-guest-runtime/tests/preparation/pid_registry/tags.rs)在原持久 PID 文件、原 pidfd 和独立私有 gate 上检查两个不同 tag 均准确命中、重复预约发生在准备之前、正常退出后的保留、取消或错会话 publish 不清旧结果、新命令复用清旧 PID/标签而不影响其他命令、旧控制句柄只持有原进程，以及 29/30 秒边界和实际容量/目录收尾。原取消准备测试另核对 tag 可在实际清理后重新预约；真实 I/O 段使用正常时钟，保留边界使用可推进时钟。入口沿已有 service lib/preparation 检查；登记层场景已在 [#528 SDK CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38878297848) 通过（源 85d487ec34fcf144999b1064140350a1f8f9d48d、checkout 45e0ed8f23a3fc2dc3ecf225e20fd87770d30429），后续公开 HTTP 扩展须独立重跑。该登记层检查不包含公开 tag HTTP/SDK，不计为完整 E03/E04 或 VM 重启验收。

[公开 tag HTTP 检查](../sandbox-guest-runtime/tests/preparation/tag_http.rs)通过原 CommandHttp、持久 PID 文件和真实 worker，核对空/特殊 tag 的 Start、List、Connect、全部 byte 输入、CloseStdin、信号、旧终局与复用；重复 Start 不新建 gate、不写 PID 或产生用户副作用，坏 token 不读正文，数字字符串不转换成 PID。另覆盖按 tag 的 PTY 输入和窗口调整、29/30 秒保留及原容量/目录实际归还。公开构造及 daemon 配置检查拒绝 29 秒、30 秒上下纳秒及 31 秒保留；这些 Linux 场景尚待当前候选实跑。可移植[重复回复](../sandbox-guest-service/src/command_requests/http/start/tests/tag.rs)与[缺失回复](../sandbox-guest-service/src/command_requests/http/operation/tests/tag.rs)检查原错误事实、转义、格式、编码预算和最后字节所有权；后者另显式通过固定 Go 的 100 组状态/头/解压正文对照，不运行原 tag 查询或 duplicate 仲裁。上述回归不表示原 SDK tag 调用、正式 MMDS/daemon 或真实 Go/Rust VM 已验收。

#311 来宾替换先复用 S02/S03/S11/S12：固定同步/异步 SDK 对固定 envd 与 Rust 实现执行同一合同样例，区分主进程退出、管道结束、输出排空、请求超时和 VM 停止；保留 stdin/PTY、非零退出、部分文件写入、断线/重启/旧 PID 及预算边界。真实模板切换还须通过 S01/S04/S05/S09/S13 的相关路径和原身份清理，不用组件样例代替实机。

[PTY 实进程检查](../sandbox-guest-runtime/tests/pty.rs)通过真实终端核对登记前无副作用、主 PID/session/三个标准描述符、窗口尺寸及 SIGWINCH、原字节大块双向传输、错误输入通道、CloseStdin 拒绝和 canonical Ctrl+D。取消输入保留实际前缀，放行诊断丢失保持未知且不重执行；终端 EOF 先于主进程退出时不得 End。管理组件覆盖断线/重连、输入堵塞时独立 resize、原期限及挂断后的收尾，防止满终端输入上的就绪忙转。SIGSTOP/waitid、受控 socket 和可推进时钟建立观察顺序。基础检查和 Linux 兼容脚本均运行该组；固定 SDK 另核对 PTY 字节回调和非零退出，不代替 Go 双端对照、全部公开 PTY 路由、后代/整机压力或 KVM。

[准入错误检查](../sandbox-guest-runtime/tests/preparation/guest_http/admission.rs)消费共享合同的 15 条请求池满样例，覆盖七个命令 RPC、五个文件 RPC 和 `/files` GET/POST/HEAD。直接调用原 GuestHttp，使用错误令牌、畸形 Basic 与一旦读取即失败的正文，核对固定 429、完整消息、HEAD、无进程/文件效果和各池归还。[固定 SDK 检查](../tests/compat/guest_admission_sdk.py)经真实有界 Listener/GuestHttp 读取同一组样例：同步/异步生成 RPC 客户端检查 ConnectError 和 RateLimitException；原始文件由 httpx 发送并交原 SDK 文件错误解析器，HEAD 单独检查无正文。Linux `check-compat.sh` 运行此 feature 用例；本地已实测 30 次同步/异步观察，不是原 SDK files.write 发送、Go 负载对照、中心/宿主网关或 KVM 验收。普通 Rust 检查仍运行直接调用回归和原控制池/字节持有测试。

[Rust 命令编解码检查](../sandbox-guest-runtime/tests/protocol.rs)消费原共享样例，覆盖字段存在性、非法/多重选择器、二进制字节、畸形/多余帧及每消息预算。[SDK/真实进程组合](../sandbox-guest-runtime/tests/sdk_protocol.rs)由 Linux `check-compat.sh` 运行：固定同步/异步 SDK 经一次性回环测试路由启动当前账号的真实 worker/命令，检查管道二进制/非零、大输出、信号退出、PTY 原字节/非零及 SDK 终端回调，以及 End 帧截断后实际 `CommandHandle.wait()` 报错。测试路由只接受各场景预先确定的一条命令，无公开登记/鉴权；检查实际主进程退出、输出排空和私有 socket 回收。它不是正式 Rust HTTP 服务、固定 Go envd 双端对照、全部命令操作、PTY 一元操作的 SDK 对照、降权或 KVM 证据。

[SDK/已鉴权 PID 组合](../sandbox-guest-runtime/tests/sdk_registered.rs)及[同步/异步调用集](../tests/compat/guest_registered_sdk.py)另经受限回环路由连接 CommandRequests、Basic 账户预检、无 tag Start、持久 PID 登记和原 PID 控制。八轮预选命令覆盖默认/显式账户、原始 List 元数据、初始订阅断开后重连、空输入与全部 byte 的管道往返、CloseStdin、PTY Update/输入、TERM/KILL 和真实 CommandHandle 终局解析；结束后的 List 为空，原 PID 再连接只取原终局。每轮核对 shell 只执行一次、实际 wait/输出排空、30 秒原保留期、名额和私有 socket 归还。路由只断言固定 SDK 的 JSON/identity 与正 10000ms 头，拒绝任意 Start 配置；该历史批次不覆盖通用 HTTP 协商、公开错误映射、tag 策略或生产监听。Linux `check-compat.sh` 已登记该入口，原提交已有下述 #439 CI 实跑证据；不替代原低层/响应所有权测试、Go 双端 E03、特权账户、压力或 KVM。

固定 SDK 的同一受限入口另增加同步/异步各四轮 tag 场景，保留原八轮无 tag 调用。使用原生成 ProcessClient 和 CommandHandle，检查空字符串、数字字符串、特殊字符及 Unicode tag 的 List 字段存在性、断线重连、管道/PTY 输入、窗口调整、CloseStdin、TERM/KILL、原终局解析及保留期后的 PID/tag 失效。存活期间只发送一次重复 Start，原客户端须解析 already_exists；原 PID 文件、gate、效果标记及登记身份保持。五种按 tag 操作的缺失错误、数字 PID 字符串不作 PID、终局不能供存活控制同样由原客户端解析；已有非法 keepalive 和无 tag 错误回归继续运行。测试只允许预定命令，不新增通用测试执行入口；当前八轮 tag 场景尚待该候选 Linux 实跑，静态检查不代替原客户端/worker 执行，也不构成完整 E03 或正式 daemon/VM 验收。

[Start 登记容量回归](../sandbox-guest-service/src/command_requests/tests/start/capacity.rs)通过原鉴权、账户解析、HTTP 选择和 Start 预约，核对条数满额与单条元数据超限先于 worker，原期限/错误事实匹配及响应最后副本占额；[错误编码回归](../sandbox-guest-service/src/command_requests/http/start/tests/capacity.rs)核对 identity/gzip 的精确响应预算。上述 SDK 入口的八轮无 tag 命令在原进程存活时再发一次相同 Start，核对原 resource_exhausted 响应由锁定 SDK 解码并经其错误映射得到 RateLimitException，且 PID 文件、gate、身份与一次性效果保持。此组件检查不代替真实 guest 满额/保留期回收及九组 KVM 验收。

[keepalive 周期检查](../sandbox-guest-service/src/command_clock.rs)增加可移植虚拟时钟回归：延迟保活仍沿原周期、Data 重置丢弃旧 tick、亚秒精度、巨大跨期无循环补发、零或不可表示时间拒绝。[原 Go ticker 检查](../tests/compat/connect-reference/keepalive_clock_test.go)在固定 Go 1.26.8 testing/synctest 中已显式通过，观察接收 10/34/40 秒对应 tick 10/20/40 秒，43 秒 Data 重置后为 53 秒，77 秒重置丢弃旧 tick 后为 87 秒；入口为 `cd tests/compat/connect-reference && GOTOOLCHAIN=local go test -mod=readonly -run TestDelayedTickerAndDataReset -v .`。这是原时间库与受控发送完成的对照，不是原 envd HTTP 运行。新增 [Linux 原命令响应检查](../sandbox-guest-runtime/tests/owner/body_clock.rs)覆盖持有 Start/保活/Data 最后副本、释放后延迟轮询、禁止预读和保活突发、丢弃响应后原命令仍可输入/退出；两项已在 [#461 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38610815482) 实跑通过：源 ccb286834627b6cb87ce89f8615e0e9e6bc953af、checkout 0ebd7b12ad7ba0290e7b37bea75d1db68508ccf1，owner 22 passed、preparation 44 passed、service lib 395 passed / 63 ignored；固定 SDK 77 项、八轮 PID/56 个错误调用、封存 Redis 与四项检查通过。测试按暂停时钟下 Tokio 驱动的毫秒粒度允许最多 2ms 唤醒窗口，仍拒绝额外等待整个周期；原期限/所有权回归继续保留。该历史回执不包含 wire keepalive 头；后续批准政策的接入检查见下文，完整慢网络时序仍未验收。

[keepalive 头回归](../sandbox-guest-service/src/command_keepalive/tests.rs)增加三项可移植测试，核对原首字段、默认 90 秒、符号及长前导零、i64 解析边界、有符号乘法回绕和原构造器非正周期分类。[Go 参考 I 操作](../tests/compat/connect-reference/keepalive.go)保留固定 permissions/keepalive.go 的 Header.Get、strconv.Atoi、time.Duration 乘法及真实 NewTicker，仅增加周期输出、已知 panic 捕获和立即 Stop；不运行 envd 或等待真实周期。固定 Go 1.26.8/64 位及原依赖下，38,166 组显式对照已本地通过：首字段及四种后续字段、全部单 byte/插入 byte、符号/边界/长零前缀与 seed 0x311 的有限向量；其中不符合 HTTP 字段语法的原始 byte 只测试选择运算，合法字段另由实际 HeaderMap 核对。入口为 `SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference cargo test -p sandproof-guest-service --locked --lib fixed_go_header_selection_and_actual_ticker_constructor_match -- --ignored --nocapture`。原 533 项 Connect timeout 解析与命令 HTTP 头对照使用新增参考程序重跑通过。普通 CI 运行单位回归，不自动执行 Go 对照；原 Go 观察与下述批准政策分开报告，有限对照不代表完整 HTTP/SDK 时序、E03 或 KVM 验收。

批准的 keepalive 有界正秒政策增加两项[选择回归](../sandbox-guest-service/src/command_keepalive/tests.rs)，并在既有回退用例核对缺失、首空值、非法字段及 i64 解析溢出；特别拒绝 Go 回绕为正周期的负值和乘法溢出。新增[头选择顺序检查](../sandbox-guest-service/src/command_requests/http/tests/keepalive.rs)覆盖 Start/Connect、原头错误优先级、原期限与格式及一元忽略；[错误响应检查](../sandbox-guest-service/src/command_requests/http/reply/tests/keepalive.rs)覆盖 identity/gzip EOS、固定文本、独立原/压缩预算和最后字节副本容量。三项 [Linux 组合检查](../sandbox-guest-runtime/tests/preparation/keepalive_http.rs)覆盖 HTTP 在读正文前拒绝且 PID 集/gate/进程效果不变、token/Basic 先于新拒绝、一元/控制请求不受该头影响，以及原 HTTP 选择经真实 Start/Connect 到命令响应体的 1/50/90 秒周期与存活/输入/退出。周期测试使用暂停时钟和原未压缩命令体，不涵盖 HTTP 压缩发送完成或实际网络 50 秒等待。三项已在 [#520 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38849424091) 通过：源 15d0817f11a2be58dfd70b47f82a84a9b4cf5c81，实际 checkout 22a861ab81f01ff188e3e5f2fe9189800dcf5b78（目标基线 45946b0c61348cb7a344512ae98ef812641c9d80）；preparation 63 passed / 3 个 helper ignored、service lib 452 passed / 74 ignored，固定 SDK 231 项、封存 Redis 与四项普通检查成功。首轮失败源于显式账户错误的测试预期：原 Start 使用 HTTP 200 的 unauthenticated EOS，令牌错误仍为 HTTP 401；修复仅核对原响应差异，未改变鉴权行为。

[HTTP 编码后的保活时钟检查](../sandbox-guest-runtime/tests/preparation/keepalive_encoded_http.rs)将真实 CommandHttp、PID 登记与 worker 接到原 identity/gzip 响应编码，输出按 13 byte 分块。分别核对 Start 的 1 秒与 Connect 的 50 秒选择，持有 Start/Data 最后编码块的副本时禁止预读，释放后才开始或重置周期，延迟不能形成保活突发；完成后仍须原命令输入、真实 End/EOS、一次效果、精确回收和请求容量归还。当前线程测试运行时只设一个阻塞 worker，用明确的完成屏障跨接真实编码与暂停时钟，避免等待阻塞编码时自动推进时间；保留 Tokio 毫秒定时器的 2ms 分辨率窗口。此候选尚未 Linux 实跑；交叉静态检查不记作通过。它不运行网络发送、原 SDK 或 Go envd，也不代替真实 50 秒网络等待、慢连接、完整 E03/E05 或 KVM 验收。

[固定 SDK 保活线协议回归](../tests/compat/test_sdk_keepalive_wire.py)使用原同步/异步 SDK 的实际回环 HTTP，分别调用前台/后台命令 Start、命令 Connect、PTY Start/Connect，核对原头 50 秒与 15 秒 Start 期限/不限时 Connect 的独立性。原生成 schema 的受控应用在 Start/Data 后插入预定保活消息，检查跨 UTF-8 分片的文本与 PTY 原 byte 回调、正常 End、只有保活而缺 End、非零 End，以及每次只发原 RPC、无重试/补发 Start/信号。两项回归共 30 次流式调用，不执行命令、文件或产品 VM；没有真实 50 秒等待或定时发送，也不验证 Rust/Go 服务、代理超时、慢网络或完整 E03。这两项客户端测试不构成异常输入政策或实际服务时钟的证据；C02 固定监督调用集未变。

#517 的头选择组件已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38845354172) 通过：源 31fed17cb8b9a341a89c5301d4a3e63a57dd8b35、checkout 4174f7064755e35aa3a816b76275224966fdf1b5，三项新增 Rust 回归、固定 SDK 222 项、原 Linux 组合及封存 Redis 通过；显式 Go 对照仍按原规则跳过。#518 的 SDK 保活回归在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38845839421) 通过：源 6dd968b988bbbe3c91db15d3a863bf5bc9a02ad1、checkout c20bf6158094b41338c84a10eaced9a880640339，新增两项包含在 SDK 224 项中。两次四项 CI 均成功；均未执行实际产品 VM 或 KVM。

#519 多订阅探针的七项回归在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38847645030) 通过：源 cf0e80a12181aac72ea017641e3e8e05bcefdd16、checkout 40569477a0e99dd653d5727a8f762d12a8ec7b18，固定 SDK 231 项、原 Linux 组合、封存 Redis 及四项 CI 成功。该回执不包含后续 keepalive 政策接入或实际 Go/Rust 四 VM 多订阅执行。

[固定 SDK 登记组合](../sandbox-guest-runtime/tests/sdk_registered.rs)在原同步/异步、管道/PTY/两种信号的八轮调用前，分别通过[原生成 RPC 客户端](../tests/compat/guest_registered_sdk.py)发送六种 keepalive 非法整数的 Start/Connect，共增加 96 次请求。核对原 ConnectError 的 invalid_argument 与固定文本；每个拒绝之后检查 PID 持久文件、登记容量、gate、执行标记及启动状态未改，后续原合法 Start/输入/重连/退出及额度回收仍必须成功。显式 Basic 仍每请求查询，默认账户仅在实际 Start 查询；全部拒绝计数核对无额外 RPC。异常值由受控客户端显式设头，不是高层 commands/PTY API 默认的 50 秒，也不包含新策略对原 Go 的等价断言。该扩展已在 [#521 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38849548611) 通过：源 975bc0512511d6cc860d20b36379916eb044672e，checkout f0060dffb6df9203bfea0affc497c13165e41b8e；日志逐轮确认 12 次保活拒绝及后续原合法操作，共八轮 96 次拒绝。固定 SDK 231 项、preparation 63 passed / 3 个 helper ignored、service lib 452 passed / 74 ignored、封存 Redis与四项普通检查成功。这两份回执分别绑定各自目标基线，后续共享基线同步仍须复验，不代替实际 Go/Rust VM、完整 E03/E05 或形式化验收。

该登记组合的受限 Axum 夹具只在已知非法 keepalive 调用中，先以原 16 KiB 上限和独立 5 秒期限接收有限正文，再交原 CommandHttp；不解码、不重放、不改错误断言或产品限额。它固定组件组合测试的输入交付时序，不能据此声称生产监听器的提前拒绝已验收。[独立传输回归](../sandbox-guest-runtime/tests/sdk_registered/transport.rs)直接连接原 Listener 和 CommandHttp，绕开该预读：固定 SDK 已发送完整消息、尚未发送 HTTP 结束标记时，原处理器须先交付相同 invalid_argument，PID 文件、gate、进程效果与原容量保持。同步/异步用明确屏障控制，并核对屏障确实执行；原“头拒绝不读取正文”的准备检查继续保留。该回归只覆盖上述可复现时序，不是任意网络交错、完整 GuestHttp/VM 或 Go 对照的证明。

[共用命令 HTTP 检查](../sandbox-guest-runtime/tests/preparation/http_dispatch.rs)增加四项 Linux 场景：持有 Start 流使普通请求满额时控制请求仍可进入、控制回复最后副本继续占额；同一装配克隆的后续 Start 使用新内部编号；token/Basic/方法/压缩拒绝顺序且不轮询正文；原 Ready 到期仍持原期限/容量；缺失默认账户及坏 gzip 沿原回复，tag/未知路径保持未分类；别名池、错 VM/会话和无效回复配置在工作前拒绝。原 worker、PID 表、账户与有限回复均为实际组件，检查真实退出、私有 socket、命令只执行一次及容量归还；service lib 另用并发检查编号耗尽不回绕。[SDK/PID 夹具](../sandbox-guest-runtime/tests/sdk_registered.rs)改为调用同一 CommandHttp::prepare/Ready::execute，继续只准入预选命令及原 JSON/identity 头，并检查普通/控制两池归还；八轮原调用集和 56 个错误调用不变。新增四项共享入口场景、编号耗尽检查及迁移后的八轮 SDK/56 个错误调用已在 [#454 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38598746529) 实跑通过：源 19dfb9b66b012ffa4eef7ad663e03cea9b9e16da，checkout f886b44146079dd641e20f07cb17f2f770a0f3c8；preparation 34 passed，service lib 386 passed / 60 ignored，固定 SDK 77 项、封存 Redis 与四项检查均通过。

[正式入口配置检查](../sandbox-guest-service/src/daemon/tests.rs)增加完整显式配置、每层未知/缺字段、重复字段、错版本/类型、超限文档、各原组件预算拒绝及跨接口工作目录一致性；Prepared 构造使用不存在安装路径仍能完成，证明静态阶段未以路径存在或 MMDS 可用作为隐含前提。[CLI 检查](../sandbox-guest-service/src/daemon/entry/tests.rs)核对唯一动作/绝对配置参数和同一 FD 的有界读取；[装配检查](../sandbox-guest-service/src/daemon/tests/composition.rs)以受控元数据及原真实 PID 状态连接同一 Prepared::assemble，验证健康独立、三类接口初始化前拒绝、初始化后空 List 和错误令牌拒绝、关闭后编号递增及旧 PID 不复用。六项在下述 #487 Linux CI 实跑通过，执行入口为 `cargo test -p sandproof-guest-service --locked --lib daemon::`；不冒充实际 Firecracker MMDS、正式子进程启动/重启或 KVM。

[正式二进制启动拒绝检查](../sandbox-guest-service/tests/daemon.rs)直接启动 Cargo 构建的服务二进制，覆盖 `check` 在安装路径不存在、监听地址已占用时仍只做静态验证，三个动作对错版本/预算/owner/worker 协议的先行拒绝，以及配置缺失、链接、权限、超限、无写者 FIFO 的明确失败。`initialize`/`serve` 的缺状态/gate 目录、缺 worker、worker 权限/长度/摘要及快照失败均须在元数据阶段之前退出；已有真实 PID 文件的字节与 inode 不变，重开后旧 PID 仍为 Reused、服务编号只按原持久值递增，worker 效果与 gate 为零。每个子进程有独立期限，完整 stdout/stderr 核对只允许无载荷阶段诊断。夹具在检出目录内创建自己的 0700 临时树，并使用原启动文件检查核对整条祖先；检出路径不满足可信权限即失败，不修改祖先、不放宽生产路径、不以跳过计通过。入口为 Linux `cargo test -p sandproof-guest-service --locked --test daemon`，已纳入原 `--all-targets` 检查；三项已在下述 #495 Linux CI 实跑通过。这是正式 CLI 的拒绝路径验证，不包含成功 MMDS 交换、已启动服务的重启、真实模板安装或 KVM。

#495 的三项正式 CLI 拒绝测试已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38788690795) 执行成功：源 9f71ef6ecd68ef3ae829ae75dd81aec42453355e、checkout e334e7904a132e0eca37ffea87c4813c0240ea6b；固定 SDK 84 项、封存 Redis 与四项 CI 全部成功。该回执只推进上述启动拒绝覆盖，不包含成功启动或下述运行身份观察工具。

[受信任启动文件检查](../sandbox-guest-runtime/src/startup_files/tests.rs)的四项普通单元测试使用当前账号拥有的私有目录和文件：原 FD 在路径替换后仍读取原内容，组/其他可写祖先、非私有状态/配置、不可执行或可写 worker、硬链接/软链接/错类型/错 UID 拒绝，FIFO 无写者时非阻塞打开后拒绝，以及路径/预算先行。目录遍历主体使用私有锚点，不依赖系统二进制属主，也不把公共临时目录祖先当成可信安装路径。普通定向入口为 `cargo test -p sandproof-guest-runtime --locked --lib startup_files::`。

从真实 OS 根开始的 root 安装场景单独保留为显式测试，`SANDPROOF_GUEST_ROOT_WORKER` 必须指定已存在、属主 UID 0、祖先受保护且满足原 worker 检查的可执行文件。测试原样调用生产 opener，核对返回 FD 的 root 属主及错误 owner 拒绝；不执行该程序、不修改系统文件或模拟 UID。缺少参数、权限或属主不符均失败，不自动跳过。Linux Rust CI 在完整基础检查后独立执行此项；普通账号拥有的自建 rootfs 不满足该前提时，需要另在满足条件的文件树实测，不能将普通单元测试通过计为 root 安装验收通过：

```sh
SANDPROOF_GUEST_ROOT_WORKER=/usr/bin/false \
  cargo test -p sandproof-guest-runtime --locked --lib \
  startup_files::tests::root_owned_installation_is_opened_from_os_root \
  -- --exact --ignored --nocapture
```

这些检查不覆盖并发挂载替换、root/同 owner 对抗、worker 摘要或整套安装验收。

#486 的精确组合已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38756082071) 通过：源 e75966a0bf0d9917c436ca538fb91753ec698fd8，checkout 99b8d56a547810cfe95f9930dbdef01e9e5f7b20；离线 tz-rs 0.7.3 已可用，runtime PID 11 项、preparation 56 passed / 2 helper ignored、process 24 passed、service lib 429 passed / 71 ignored、固定 SDK 77 项、Harbor 262 项、封存 Redis 与四项 CI 均成功。该提交包括下述启动状态/持久会话和先前时区组合；当前正式 daemon 入口是后续改动，不借用该回执声称新入口通过。

#487 的正式入口候选已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38762634753) 通过：源 53f56c4504296e650212b132a9e28390df799fe2，checkout 9129df731ed335e1fd233f91cc3fd1ca4e16ce4b；六项 daemon、四项 startup_files 均执行成功，service lib 435 passed / 71 ignored、preparation 56 passed / 2 helper ignored、process 24 passed，固定 SDK 与封存 Redis 成功，四项 CI 全绿。该回执不覆盖后续 worker 指纹与配置 v2 改动。

[worker 指纹检查](../sandbox-guest-runtime/src/artifact/tests.rs)在真实私有文件上覆盖已知 SHA-256、大于两个缓冲的输入、初始 FD 非零位置、路径替换后仍读取原 inode、错长度/内容/类型、各读取阶段取消及检查之间增长/截断/同长度改写。三项已在 macOS 实跑；版本标识测试将实际 Rust 常量与原输入锁核对。配置原检查扩展旧 v1/错包版本/错协议/错摘要先行拒绝；Linux [服务版本](../sandbox-guest-service/tests/version.rs)与[worker 版本](../sandbox-guest-runtime/tests/process/worker_protocol.rs)新增独立子进程 --version、无配置/gate 与额外参数拒绝，两项已在下述 #488 CI 实跑。复用普通 service/process 入口，不新增 CI 权限；配置/HTTP 装配夹具不假称实际运行了初始化/serve 或 Firecracker MMDS，不把自报包版本当成源码、ELF 或模板发布证明。

#488 的版本/指纹组合已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38765251683) 通过：源 0bbcea4a6e5ab237e41dbb5cf1e25515235fcb78，checkout 1be104c8f3e998261c4255b5a4fe48b616d6160b；三项 artifact、两项 build_info、服务/worker 两项 CLI 均执行成功，service lib 436 passed / 71 ignored、process 25 passed，原 SDK/封存 Redis 与四项 CI 成功。不覆盖下述新离线 release 准备工具或其实际产物。

[来宾清单检查](../tools/tests/test_guest_artifacts.py)以真实私有文件核对独立 manifest pin、双二进制/归档变更、缺失/额外/旧字段、布尔冒充数字、错架构/版本、文件类型/权限/链接/字节预算与重复 JSON 字段。[准备检查](../tools/tests/test_prepare_guest.py)使用真实临时 Git 仓库及归档、实际有界版本查询子进程，覆盖脏/未跟踪源码排除、运行配方错配、锁/工具链错配、混合版本、离线构建失败、原期限、编译器变化、原目录不复用和环境白名单；编译阶段是受控替身，版本程序是夹具，不把这些用例算作 Rust release 已构建。原命令回归另实际验证 cwd 仅作用于子进程，保留 EOF 后期限检查。可移植定向组通过；正式准备须在隔离 Linux 环境对精确提交运行原命令，独立核对输出清单及再读拒绝，并继续真实模板/SDK/KVM 验收。普通 PR CI 未增加构建权限或新的安装步骤。

#489 源提交 966ca455ec8043dc2066d90c9b71619952c0ffe4（checkout 7e1361f7fede1767992d21b5253cc913393e44c0）的四项普通 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38769618672) 已通过；这不是该工具的真实 Linux release 构建或模板安装回执。

[共享来宾身份样例](../tests/compat/guest-template-identity.json)由工具端真实私有夹具字节、独立清单/config pin 产生，并由 [Rust 模板检查](../controller-service/tests/templates.rs)读取：核对旧描述字节不变、每个新字段参与摘要、错类型/范围/未知字段拒绝，以及只改路径/owner 的合法投影和混合身份拒绝。[原 SQLite 准入](../controller-service/tests/admission.rs)覆盖提交前失败、提交回执丢失后重开、同编号改配置/二进制/清单或删除 guest 均拒绝；[中心权威块](../sandbox-center-service/tests/business_authority.rs)沿原真实固定 Redis 回归携带新身份，检查恢复和本地证据错配；宿主原元数据预算用例纳入新描述字节，该 Linux 专属用例尚未在本轮 macOS 执行。工具端另覆盖配置独立 pin 失配、版本/worker 混配、布尔、重复字段与有限字节。样例是测试产物，未执行安装或正式服务；配置完整有效性仍归原 daemon 检查，真实构建/安装/SDK 双端/KVM 验收仍待完成。


#490 的模板身份源提交 12070f83bd1996163d12c4c375f6950062575694（checkout 87698b989accca687c3450a71fabea654a41d388）已在四项普通 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38773547217) 通过，包含实际固定 Redis 的 18 项中心权威块回归。该入口不执行 Linux 专属宿主元数据预算测试，不背书下述安装连接。

[模板安装回归](../tools/tests/test_template_guest.py)在真实私有候选目录核对复制后的 service/worker/config/manifest 指纹、权限、保留路径及静态 check 失败/原期限耗尽。实际 `/bin/sh` 执行生成的 boot/agent，挂载/网络/BusyBox 操作及 daemon 是受控程序：覆盖一次初始化、respawn/重启、PID 状态缺失、initialize 前后失败、重命名/sync 失败、符号链接/重复制备标记。原 opaque 暂存验证独立 pin 必需、身份持久化和重复恢复拒绝变更，并实跑源文件名已消失的精确交接；这些用例不检查 ext4 稀疏程度或假装 Linux 挂载。Go 原安装回归保持，平铺模块导入和额外 applet 拒绝另测试；不把 stub 的 check、sync 或 PID 文件当作真实 daemon、持久磁盘或 KVM 验收。真实 release、镜像 ABI、实际 builder 上传与完整 E06/E09 仍须使用下文的 Rust 来宾实机入口独立验收，具体版本结果在[#311](https://github.com/VeriInfra-Lab/sandproof/issues/311)记录。

#491 安装源提交 a3301f449382ab91175d00079f71f05ea39c2466（checkout 5916ce0d714849bdb8995e9482cdc68cfc7dd5e7）的四项普通 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38777145956) 已通过，包含实际固定 Redis 的 18 项中心权威块回归；不表示实际模板安装/启动通过。

[监督器材料回归](../tools/tests/test_template_build_guest.py)使用私有文件和原 SQLite 配置守卫核对同路径换 pin 拒绝旧库重开、源码归档只在宿主必需、运行材料独立校验、必要容量边界、实际流字节/EOF/文件替换与期限；原 opaque 暂存检查候选身份、下载名已消失后的恢复、发布未知结果保留和已知原回执观察。[固定 SDK 监督器检查](../tests/compat/test_template_build_backend.py)覆盖 OCI/Dockerfile 两条上传调用、原 VM 会话与身份头、固定目标路径、worker 参数、候选身份错配时不下载磁盘，以及配置变化/上传超期时不开始后续阶段；该组使用真实 SDK 构造器与受控文件/命令端口。[实际 SDK 回环上传](../tests/compat/test_template_upload.py)通过原异步文件客户端发送普通文件，核对实际字节/保护头，服务器等待回复时原期限取消、连接释放且不重发，以及迟到成功拒绝。回环测试未启动 VM、执行产物或调用 E2B 云；线程/内核文件 I/O 的依赖仍按模块合同区分，不把它当作 E06/E09。

[持久服务会话检查](../sandbox-guest-runtime/src/pid_claims/tests/sessions.rs)补五项真实私有文件回归：多次重开/丢弃结果后编号递增且原 PID 拒绝保留；最大 u64 只发出一次，耗尽不写、不回绕；计数写前/写后/摘要后/同步后故障共同毒化会话与 PID 写者，只有完整图像能重开；篡改与 panic 保持拒绝；校验正确的原 v1 及伪装长度的 v1 均原样拒绝。该组连同既有位图场景共 11 项，在 macOS 实跑通过，并已包含在上述 #486 Linux 回执；入口仍为 `cargo test -p sandproof-guest-runtime --locked --lib pid_claims::`，不把软件故障点等同断电或真实故障磁盘。

[正常启动状态检查](../sandbox-guest-service/src/startup/tests.rs)使用真实私有目录及原 PidClaims，覆盖固定身份摘要样例/字段无歧义、预算与配置先行、初始未授权、两次重开服务编号递增且已烧 PID 仍拒绝、独占写者、缺失/损坏/错 VM/错范围不新建或重置、令牌轮换保持绑定及外来元数据拒绝。八项回归的定向入口为 `cargo test -p sandproof-guest-service --locked --lib startup::`；该组在 Unix 运行，macOS 可执行真实文件部分。控制元数据源不代替实际 MMDS，原 PID 没有启动用户进程，也不声称服务重启后的活跃进程已收尾。

[启动取消所有权](../sandbox-guest-service/src/startup/tests/ownership.rs)用单阻塞线程、通道屏障与可推进时钟，把原文件打开固定排在占用任务之后；取消和原期限到期后，克隆协调器仍立即 Busy，直到原排队任务和取消结果实际退出。期限检查直接轮询并要求已就绪，跨过 Tokio 毫秒舍入，不等真实看门狗触发；释放阻塞后重开成功，原位图保持不变；排队取消/过期未烧会话编号，只有成功重试分配 1。另检查元数据故障/等待取消及名额归还。测试不创建新的 PID 责任、不替代 Linux 真实 daemon 重启、FD 压力、KVM 或持久存储故障矩阵。

命令装配配置回归在同一 [HTTP 场景](../sandbox-guest-runtime/tests/preparation/http_dispatch.rs) 扩为 20 类拒绝：已有别名池/VM/会话/回复错误，进程 chunk/launch/握手预算、准备/清理/重试期限的零与溢出、worker/gate 路径语法及另一 VM 的持久登记器。每次核对无 worker gate/用户效果、请求与进程容量可用、原及外来 PID 文件字节不变；异 VM 存储本身合法，不能仅以注册表正确推断组合一致。新增 Linux 构造断言已在 [#479 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38678622537) 通过（源 f58b66682897090169aaa17c96ac01bd5e21d7a1，checkout 69376d3430a715d3e71efd6a378699ba30dc8d0e）：preparation 56 passed / 2 helper ignored、service lib 418 passed / 70 ignored，固定 SDK 77 场景、封存 Redis 与四项 CI 均通过；原真实 Start/SDK 调用继续覆盖有效配置；不证明 worker 安装、当前存储完整性或正式 daemon 启动。

#439 的八轮 SDK/PID 组合已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38564633601) 通过：源 a0329fa9a899059f4a2ef56ced87d66f8f389e40，checkout 06fb36868fb3a37f39c3296cce50d56793241e0b；四项 CI、原命令/文件与封存 Redis 均通过。首轮测试错误要求 identity 头缺失而失败，修正为原客户端显式 identity 后实跑通过，生产代码未为测试改变语义。后续迁移到共用 HTTP 选择的候选须独立重跑。

[命令空消息回归](../sandbox-guest-runtime/tests/protocol.rs)核对零长度完整帧、最小正预算、缺失/截断/错误 flags/多个消息和一元空正文的区别；EOS 回归另核对先解码首消息、两个载荷独立预算和不采纳客户端 error。[固定 Go F 对照](../sandbox-guest-runtime/tests/command_requests.rs)通过原 server-stream handler 回显生成 Start/Connect 请求，覆盖全部 256 个空帧 flags、JSON 形状、截断和多个数据消息。[后续 EOS 对照](../sandbox-guest-runtime/tests/command_requests/streams.rs)新增 3,948 组首消息存在性、所有尾帧 flags、完整 EOF、已知字段形状/null/重复/折叠、UTF-8/surrogate、base64 和深度边界；只比较接受性与成功解码字段，错误码/全文、压缩及完整服务未据此验收。

[来宾 HTTP 装配检查](../sandbox-guest-runtime/tests/preparation/guest_http.rs)增加七项 Linux 场景：三个组件分别拒绝不同认证状态；初始化前拒绝、同 token 初始化、失败初始化与 MMDS token 轮换共同作用于命令/文件 RPC/原始文件；各回复最后 Bytes 副本仍占原池，health 可执行，init 等待原文件授权至最后所有者释放；未知路径及未映射错误不读取正文或泄露诊断。[真实回环组合](../sandbox-guest-runtime/tests/preparation/guest_http/transport.rs)在同一有界监听器上执行当前账号真实 Start/输出/CloseStdin、文件上传/Stat/下载；命令流占满普通池时文件及控制仍可用；失败连接不污染后续请求；取消监听器后通过原表重连并结束同一命令，检查一次效果、原 wait、私有 socket 与容量归还。输入为合成受控请求，账户映射当前 UID/GID、元数据/MIME 为夹具；七项已在 [#460 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38609558853) 实跑通过：源 b48d4f0c301d67483006ac81ef6bcf3a2c3754ab、checkout 405c500195afcf935d4188be90a7617c768dca01，preparation 44 passed、service lib 393 passed / 63 ignored；固定 SDK 77 项、八轮 PID/56 个错误调用、封存 Redis 与四项检查通过。现有固定 SDK 夹具的预选命令/路径限制保持不变，不计为正式 daemon、特权切换、SDK 双端或 E06。

[帧错误完整回复检查](../sandbox-guest-service/src/command_requests/http/reply/tests/framing.rs)新增原接收顺序、有限标量与资源/JSON/压缩错误区分、精确原始/压缩帧预算、过期原失败和最后副本所有权检查。固定 Go E 入口以 7,568 组 Start/Connect、identity/gzip、charset、全部 flags、空/非空帧及首/尾截断比较实际 HTTP 状态、头、flags 和完整错误字节；共用 SANDPROOF_GUEST_CONNECT_REFERENCE 并显式 --include-ignored，没有新增 Go 模式。[Linux 共用入口](../sandbox-guest-runtime/tests/preparation/framing_http.rs)另有 32 个组合：八类坏帧先于默认账户/PID 查询，返回唯一错误 EOS，原鉴权释放、最后副本容量、无进程/私有 socket 副作用，随后合法 List 仍成功。该 Linux 场景已在 [#455 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38602449419) 通过：源 b4dc0263c47f15a589b486431027c88694d83d9f、checkout cfacf01d01fd1142e52015b8ac571f8a8e916577，preparation 35 passed、service lib 388 passed / 61 ignored；固定 SDK 77 项、八轮 PID/56 个错误调用、封存 Redis及四项检查通过。首轮 Linux 旧断言仍期待通用错误而失败，改为精确 ExtraBytes(1) 后实跑通过。合成坏帧不是固定 SDK 原调用，不计为完整 E03。

[命令 JSON 完整回复检查](../sandbox-guest-service/src/command_requests/http/reply/tests/json.rs)显式使用同一固定 Go E 入口，覆盖七种方法、identity/gzip、charset、嵌套字段、重复/oneof/null 顺序、args/envs、数值/bytes、Unicode/词法突变和深度边界。5,107 组合法输入比较接受性，12,505 组拒绝比较原 HTTP 状态、头、flags 与完整错误字节；只在固定 proto: 前缀归一化构建相关空格，Content-Length 先核对实际字节长度。普通回归另检查精确预算、原期限、Debug 隐私、跨命名空间拒绝和最后字节副本容量，共用文件 JSON 对照独立重跑。[Linux 共用入口](../sandbox-guest-runtime/tests/preparation/json_http.rs)增加 28 个方法/编码/charset 组合，检查 JSON 先于默认账户/PID、鉴权释放、普通或控制槽位直到最后副本才释放、无进程副作用及后续合法 List；新增场景已在 [#457 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38603700657) 实跑通过：源 809a5d634d3eb8cf77471165966692ce2575489f、checkout d498273b6b1d9e308479ae135168f889c10ae471，preparation 36 passed、service lib 391 passed / 62 ignored；固定 SDK 77 项、八轮 PID/56 个错误调用、封存 Redis 与四项检查通过。合成输入不记作原 SDK 请求或完整 E03。

[EOS 完整错误回复检查](../sandbox-guest-service/src/command_requests/http/reply/tests/end_stream.rs)通过固定 Go E 原 Connect 入口比较 2,052 组合法 EOS 接受性和 21,076 组完整 HTTP 状态/头/flags/错误字节。覆盖整个 JSON 语法优先、普通/自定义字段错误竞争、嵌套类型、逐字节 base64/CRLF 偏移、截断、Unicode、深层未知值后的已知类型错误及 10,000 层界限；不归一化错误正文。普通检查另覆盖首消息/解压/结束后字节的优先级、精确回复预算、原期限、最后副本和一元误用拒绝。[Linux 共用入口](../sandbox-guest-runtime/tests/preparation/end_stream_http.rs)新增 24 组 Start/Connect、identity/gzip、首帧或消息后 EOS 的语法/类型/base64 拒绝，检查先于默认账户/PID、鉴权及原容量释放、无命令副作用和后续合法请求。新增 Linux 场景已在 [#459 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38605926166) 实跑通过：源 6e88ea8ee2e06c1ffae9af8c9d42ab6696133284、checkout 66f999a0b4d5069673cc5ce2023a8e9dfd19de45，preparation 37 passed、service lib 393 passed / 63 ignored；固定 SDK 77 项、八轮 PID/56 个错误调用、封存 Redis 与四项检查通过。合成 EOS 不记为原 SDK 发送或完整 E03。

[PID 操作回复检查](../sandbox-guest-service/src/command_requests/http/operation/tests.rs)覆盖原始/压缩精确预算、原观察/期限、缺失分类和最后字节副本。固定 Go R 入口以 164 组 Connect/控制方法、JSON 类型、协商编码、PID/信号边界、四种通道错误及关闭 stdin 比较完整状态/头/flags/错误字节；使用原 fmt、生成枚举和 Connect 渲染固定 getProcess/handleInput/CloseStdin/SendSignal/Update 与 handler 表达式，查询及通道错误为受控事实，不执行原服务方法或证明其并发顺序。共用 SANDPROOF_GUEST_CONNECT_REFERENCE 并显式 --include-ignored。

[通道模式 HTTP 检查](../sandbox-guest-runtime/tests/preparation/channel_http.rs)新增管道和 PTY 真实命令场景，核对四种错误的原 code/message、原普通/控制容量保留至最后副本、鉴权释放后可轮换、原 PID 仍可合法输入/结束和真实 wait/socket 收尾。分类回归拒绝把模式正确的 InvalidArgument 或其他失败套为通道错误；回复回归覆盖精确预算、到期、流式误用和最后副本。固定 SDK 每轮追加两个原生成客户端调用，八轮合计 72 个错误调用，保留原预选命令/JSON/identity 限制及后续合法操作；该 Linux 场景与八轮 SDK/72 个错误调用已在 [#463 首次 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38612383324) 实跑通过：源 b408e14f6ab3b68b3aa318b882d70bf337d6045e、checkout e30527eb6d2bbab5021a8eb94749091c0c7000bc，preparation 45 passed、service lib 397 passed / 63 ignored；固定 SDK 77 项、封存 Redis 与四项检查通过。编码 worker 最后释放断言修正为等待实际归还，持有字节期间不得释放的要求不变；[修正提交 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38613612997) 在源 7614cc5b77a49cb1f93d86a009be33524439183c、checkout ae0f70871a3b23650e107e8adeb16bc76c850d0f 重跑同组检查通过。Go R 的 160 组为该批本地显式渲染对照，不执行原 handler，也不计为完整 E03。

[关闭 stdin HTTP 检查](../sandbox-guest-runtime/tests/preparation/closed_stdin_http.rs)在私有 FIFO 保持原进程存活期间分别检查 stdin=false、两次 CloseStdin 和子进程关闭读端。空/非空写入核对确切关闭错误；空写入与真实 EPIPE 不伪报端口关闭，真实 errno 的后续回复见下文；空写入仍按原写端返回成功。检查原请求/最后副本容量、已完成鉴权可轮换、旧组件错误不变、FIFO 放行后原 wait/socket 收尾。固定 SDK 的 TERM/KILL 预选命令改为等待私有 FIFO，在重复关闭后核对空/非空写入拒绝，再通过原 PID 重连并发信号，八轮共 80 个错误调用；路由的精确预选命令及 JSON/identity 限制保持。新增 Linux 和 SDK 场景已在 [#464 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38614732751) 实跑通过：源 fc09c0ccc923483949c2f6514fdcf8016c936390、checkout ff664b21c1d802940b190f1dc7b711485a2e4e1d，preparation 46 passed、service lib 397 passed / 63 ignored；固定 SDK 77 项、八轮 PID/80 个错误调用、封存 Redis 与四项检查通过。Go R 新增关闭事实后共 164 组渲染对照，仍不执行原 handler 或计为完整 E03。

[原生输入回复检查](../sandbox-guest-service/src/command_requests/http/operation/tests/native_input.rs)以固定 Go R 新增 192 组 stdin/PTY、三种 PID、八种 errno 符号和编码/charset，比较原状态/头/完整错误字节；运行原 fmt/os.PathError/Connect 表达式，不执行原 handler。Go TestNativePipeErrorUsesOriginalWriterName 另以真实 os.Pipe 关闭读端、原写端写入核对 |1 和 EPIPE，不能代替 Linux 进程或 PTY 对照。共享回复回归把原生输入纳入精确预算、期限、流式误用拒绝和最后副本检查；分类检查区分实际 errno 与本地合成错误，保持原 PID/通道及旧组件错误。既有 FIFO 场景改为核对真实 EPIPE 的准确 500 回复，空写入仍成功，随后释放原 FIFO 并等待真实结束。[PTY 挂断检查](../sandbox-guest-runtime/tests/preparation/input_native_http.rs)通过原私有 FIFO 证明子进程已关闭全部 slave fd 且仍存活，检查无 raw errno 的就绪拒绝保持未分类、空写入/原容量/令牌轮换及最后真实 wait/socket 收尾。新 Linux 场景已在 [#470 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38667719360) 实跑通过：源 0d0650eaea62f03e40aabe5f72c1fc5c2a31435d、checkout 8391780b81d74959244a65143057592b5cfb3eab，process 22 passed、preparation 53 passed、service lib 410 passed / 67 ignored；固定 SDK 77 项、八轮 PID/80 个错误调用、封存 Redis 与四项检查通过。不计完整 E03、全部写入故障或资源上界验收。

[原生信号回复检查](../sandbox-guest-service/src/command_requests/http/operation/tests/native_signal.rs)以固定 Go R 新增 32 组 errno/编码/charset，比较原 HTTP 状态/头/完整 JSON，并在原 SendSignal 表达式前执行 Go 1.26.8 的 convertESRCH 规则；该对照不向进程发信号。共享回归覆盖 raw/wire 预算、原期限、流式误用拒绝和最后字节副本；分类回归拒绝无 errno、非正 errno、不同粗错误和身份/取消伪装。此前 [#471 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38669428549) 实跑了原信号错误和等待后代 EOF 的旧策略（源 602bcd871c44c548476978f91bcad9df91d2df18、checkout 55f22ed3e057161625dc1f854dc78f8733e209b3）；它不验证以下改动后的收尾策略。

[Linux pidfd 场景](../sandbox-guest-runtime/tests/preparation/signal_native_http.rs)现对 TERM/KILL 各自创建原父子进程，以私有 FIFO 和精确 pidfd 确认主进程已由 owner 回收、后代仍持输出。未知信号不解除等待；合法信号从实际 ESRCH 返回原错误后，流取得真实主进程 End，后代仍存活，最后错误 Bytes 副本仍占控制容量。另以忽略 TERM 的主进程验证信号后继续交付新输出，不能提前制造终局。[执行期限场景](../sandbox-guest-runtime/tests/owner/settlement.rs)推进原假时钟，验证发送失败仍收尾且保留真实 code 0，不伪造 SIGKILL 退出。[PTY 场景](../sandbox-guest-runtime/tests/pty.rs)用忽略挂断、仍持 slave 的后代验证主进程 wait 后撤销保留的控制/输入副本；[描述符检查](../sandbox-guest-runtime/src/pty.rs)验证关闭唤醒同时等待的读写及实际 FD 最后引用释放。组件均使用真实进程/终端与受控屏障，不靠随机 sleep；这些信号/期限/PTY 新版回归已在 [#493 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38784370080) 实跑通过：源 d3a964c9abcce2ca6ec60f45c7abcd44a330af4f、checkout 85b99b4f01e5b5280658e6bfd9a89953bb4b910b；四项 CI、固定 SDK 84 项及原封存 Redis 通过。此证据不覆盖后续管道准备改动。固定 Go Handler 源码顺序已核对，未执行全 Go Handler 对照，也不计强制 PID 复用或完整 E03/E04。

[原生窗口调整回复检查](../sandbox-guest-service/src/command_requests/http/operation/tests/native_resize.rs)以固定 Go R 新增 32 组 errno/charset/编码，对照原 Update 包装及完整 HTTP 状态、头和 JSON；errno 为受控输入，不执行原 handler/ioctl。分类回归区分原 ioctl EINVAL 与无 PTY 的同名粗错误，并拒绝非正 errno、身份、终局和不同粗错误。共享回归继续检查预算、原期限、流式误用拒绝和最后副本。[真实 PTY 检查](../sandbox-guest-runtime/tests/preparation/input_native_http.rs)在私有 FIFO 证明全部 slave fd 已关闭且原进程仍存活后，分别经空 PTY 配置和越过 uint16 的尺寸执行 Update，核对原 master 的实际成功、控制容量/最后副本、鉴权轮换及后续写入挂断拒绝；释放后等待原输出/进程/登记/socket 收尾。新增 Linux 场景已在 [#473 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38670168001) 实跑通过：源 b8ec9f1b9cb333670330b9947b5ebd864577081f、checkout 1e90b4eba0efc688e5fecd627ba7be4c3e2ad2fd，process 22 passed、preparation 54 passed、service lib 414 passed / 69 ignored；固定 SDK 77 项、既有 PID 调用集、封存 Redis 与四项检查通过。未制造真实 ioctl 失败；受控错误对照不能计为全部终端故障或完整 E03/E04。

[启动包装检查](../sandbox-guest-runtime/tests/process/wrapper.rs)覆盖登记放行前工具/用户无副作用、原 PID 贯穿 shell/工具 exec、OOM=100、工具参数及原始空/空格/替换符号 argv、缺失用户程序的真实 127、包装后预算拒绝及原 worker/socket 收尾。[HTTP 检查](../sandbox-guest-runtime/tests/preparation/launch_http.rs)核对缺失/null/空 process 和缺失程序均为 Start→实际 stderr→非零 End→EOS，原 wait 与容量最终归还；已有 pipe/PTY、身份和固定 SDK 调用复用新包装；现有完整环境断言计入 shell 生成的准确 PWD。新增 Linux 场景已在 [#465 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38618795769) 实跑通过：源 e60344e563a4196eb67b20b404136fc4ba4fe340、checkout 753a16d2ebdf9bfc86ad5687f3310259bbcff92d，process 19 passed、preparation 47 passed、service lib 399 passed / 63 ignored；固定 SDK 77 项、八轮 PID、封存 Redis与四项检查通过。可移植检查分别覆盖缺失配置与执行默认值分离、精确元数据预算、脚本上限和路径词法规则；Linux 另检查工具缺失/非执行文件/目录/首个相对命中。显式 Go L 入口以 288 组可选工具、nice 值和路径比较固定 handler 表达式及原 filepath.Join，共用 SANDPROOF_GUEST_CONNECT_REFERENCE；只对照构造，不执行原查找/handler 或计作完整 E03。

[工作目录底层检查](../sandbox-guest-runtime/tests/process/wrapper.rs)现要求不存在路径/悬空链接在私有目录及 worker 均不可用时仍优先报告原 MissingCwd；此路径未创建 child，不再等待一个用于检查目录的 worker。管道/PTY 各以 spawn 返回、握手尚未发送上下文的边界删除已通过预检的目录，要求 READY 成功且实际放行后观察原 ENOENT、实际 wait 和 gate 收尾，拒绝第二次 Stat 改写错误阶段。现存文件/非目录父节点和低层 raw exec 保持原实际 chdir 错误、不得误分类。[HTTP 场景](../sandbox-guest-runtime/tests/preparation/cwd_http.rs)核对首帧直接为带准确路径的 invalid_argument EOS、无 Start/副作用、原鉴权释放、最后副本/进程/登记容量与 socket 收尾，随后合法 List 仍可执行。原 worker 侧目录检查曾在 [#466 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38619889503) 实跑通过：源 699056aba0ba3f9acb77486c7f4e041e0e50f9bd、checkout 30b97b3fe613974be8b176a66e39e08a385069ad，process 21 passed、preparation 48 passed、service lib 402 passed / 64 ignored；固定 SDK 77 项、八轮 PID、封存 Redis 与四项检查通过。[回复回归](../sandbox-guest-service/src/command_requests/http/start/tests.rs)覆盖转义、identity/gzip、精确预算、原期限、缺失观察、一元误用和最后字节副本；固定 Go R 另以 20 组 Start 路径/编码/charset 比较原状态/头/flags/完整 JSON。该显式对照只渲染受控 ENOENT 事实，不执行原 handler 或计作完整 E03。准备观察回归另拒绝把超时、owner 丢失和未知收尾改写成目录缺失。 新版 cwd 顺序回归另复用[私有子进程资源故障](../sandbox-guest-runtime/tests/preparation/pipe_http.rs)：只降低该 child 的 NOFILE，将同一 stdout/stderr/stdin 管道或 PTY master 失败条件与缺失 cwd 组合，要求目录错误优先、PID 文件字节不变、无用户效果/残留 gate，原容量与最后 Bytes 责任保持。原无 cwd 故障组继续核对真实资源错误；新版已在 [#502 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38804563032) 实跑通过：源 7211c05df1a432657b87d4507d7205847850cba4、checkout 948e47a536b6ad5378938ec4fbf77fd9319daab9；process 26 passed、preparation 60 passed / 3 个实际调用的 helper ignored、固定 SDK 109 项、封存 Redis 及四项 CI 成功。该证据不包含之后新增的双端探针。

[私有 worker 协议回归](../sandbox-guest-runtime/tests/process/worker_protocol.rs)对真实 worker 发送错误放行字节、额外字节和截断，核对新握手及独立内部故障标记、实际非零 wait 和无用户副作用。[原生启动 HTTP 回归](../sandbox-guest-runtime/tests/preparation/native_http.rs)分别在 pipe/PTY 触发真实 ENOTDIR，核对完整启动错误文本、原 PTY 宽值、无 Start/副作用、原鉴权/容量与 worker/socket 收尾、随后合法 List。新增 Linux 场景已在 [#468 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38665519619) 实跑通过：源 5a8f1234689d877d36cecd9989b3fc5ea61c5b7c、checkout b5ce75c7b2dcf3168c12b94a0c977c2209c7fcea，process 22 passed、preparation 49 passed、service lib 404 passed / 65 ignored；固定 SDK 77 项、封存 Redis 与四项检查通过。[原生回复检查](../sandbox-guest-service/src/command_requests/http/start/tests/native.rs)覆盖字面 argv、PTY 原尺寸、错误码、精确 raw/wire 预算、到期、缺失观察和最后字节副本。Go R 新增 408 组原表达式、os.PathError 和 Connect 完整回复对照，按 errno 符号名连接 macOS 参考进程与 Linux 数值，不把 Darwin 数字当作 Linux 数字；含 17 种错误、管道/PTY、空/特殊/长参数和编码/charset。该显式渲染不执行固定 Go 的真实启动或资源耗尽，不计为完整 E03。

[管道准备回归](../sandbox-guest-runtime/src/process/preparation/pipes/tests.rs)在独立子进程内把自身 NOFILE 软限制降为 64，预留精确描述符数以分别触发原 stdout/stderr/stdin 的真实 EMFILE；逐次检查原观察、FD 数和私有目录回收，另核对缺失 worker 不能携带旧管道事实。[HTTP 组合](../sandbox-guest-runtime/tests/preparation/pipe_http.rs)以同样隔离方式经过原账户/Start/登记准备/回复，检查 internal EOS、无 PID/用户副作用、原记录回收、最后 Bytes 仍占请求额度、轮换与后续 List；不修改父测试进程或宿主的限制。两项真实 Linux 回归已在 [#494 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38787146112) 执行成功：源 ead5c90107cc2398b7f74aa2ab073b82d5ee97fd、checkout 30ab3869759257a30981e418ab1ce5474bb958c5；runtime lib 115 passed / 7 ignored、preparation 58 passed / 3 ignored、service lib 440 passed / 72 ignored，固定 SDK 84 项、封存 Redis 与四项 CI 成功。两个新增 ignored 入口由原父测试实际调用，显式 Go 对照仍不计作普通 CI 执行。[回复检查](../sandbox-guest-service/src/command_requests/http/start/tests/pipes.rs)覆盖精确 raw/wire 预算、期限、最后副本、PTY/非正 errno/不同粗错误拒绝；固定 Go R 的 48 组通道/errno/编码/charset 完整回复对照已本地通过。Go 参考使用固定 Linux os.Pipe 的 pipe2 包装及 Handler.New/Connect 表达式，errno 是受控事实，不执行原 Go 管道耗尽或整个 Start，也不代替 E03/E05。

上述隔离子进程回归已扩展 PTY master 的实际 EMFILE，并检查 peer 打开/描述符复制失败不能带入旧 master 事实、FD 与 gate 数恢复；HTTP 增加独立 PTY 父测试，要求 resource_exhausted EOS 带原窗口尺寸、无发布 PID/用户效果、原记录及请求最后副本责任保持。[PTY ioctl 检查](../sandbox-guest-runtime/src/pty.rs)在真实 /dev/null 上观察 ENOTTY，明确区分故障观察与真实终端初始化失败；这些新增 Linux 场景已在下述 #498 CI 实跑。[响应回归](../sandbox-guest-service/src/command_requests/http/start/tests/pty_preparation.rs)覆盖 master/解锁/窗口阶段、原字段、raw/wire 边界、期限及不匹配事实拒绝；显式 Go 对照比较固定 Handler/PTY 错误包装和 Connect 的完整回复，errno 是受控输入，不执行原 Go 终端分配。没有耗尽宿主 PTY、修改全局限额或制造内核 ENOSPC，也不计作完整 E03/E05。

#498 的 PTY 准备错误已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38795688737) 通过：源 12b51f519d1bb58c4c33a88b6b1b78ca9a1d9e7a、checkout d84dc411bd2dc6dcc8ec3af2a7393d735e8ad23d；master/peer/复制的实际 EMFILE、真实 ioctl、HTTP Start 与事实匹配回归均执行成功，固定 SDK 98 项、封存 Redis 和四项 CI 成功。此证据不包含原 Go 终端资源耗尽、全部 PTY 初始化故障或 KVM。

[Start NUL 检查](../sandbox-guest-runtime/tests/preparation/nul_http.rs)在真实当前账号管道/PTY 各运行八种输入：cmd、argv、cwd、环境键/值、组合错误、缺失 cwd 优先及展开失败优先。检查仅错误 EOS、无用户效果、持久 PID 文件字节不变、已创建的 worker/目录实际收尾、最后 Bytes 占额、轮换及后续 List；[raw 回归](../sandbox-guest-runtime/tests/process/nul.rs)要求五类 NUL 在 spawn 前拒绝。观察状态测试把三类拒绝同取消/期限/未知清理/owner 丢失分开。新增场景已在 [#477 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38675155298) 通过（源 4ca2c7d7d97229ce2c033702d4b8ce0f33239ac2、checkout 060f4305f9faa768141026244fafe1f6a13315df）：process 24 passed、preparation 56 passed / 2 个实际调用的 helper ignored、service lib 415 passed / 70 ignored；固定 SDK 77 项、八轮 PID/80 个操作错误调用、封存 Redis 及四项检查成功。初始化默认环境和空键/含等号键不在此兼容扩展中。

[固定 Go NUL 对照](../sandbox-guest-service/src/command_requests/http/start/tests/nul.rs)通过 Connect 参考 R/nul，实际调用 Go 1.26.8 Cmd.Start 的参数/环境校验，要求 Process 始终为 nil，再使用固定 handler.New/Start 包装和真实 Connect 输出；参考显式设置非 nil SysProcAttr，避免 os.StartProcess 的额外 cwd 检查改变错误。16 个管道/PTY × 环境优先/参数 × charset × gzip 组合比较完整状态/头/帧/JSON；不运行原 handler 的 cwd Stat、账号/cgroup 或 PTY 创建，不代表完整 E03。定向入口沿 Start 原对照，连同 20 个 cwd、36 个展开和 408 个原生错误渲染重跑。

[Start 路径展开检查](../sandbox-guest-service/src/execution/tests.rs)区分请求/默认 cwd、有效覆盖、选定账户与环境 USER/HOME，以及独立的 NUL 校验与其他上下文错误；[请求连接](../sandbox-guest-service/src/command_requests/tests/start/expansion.rs)核对 Basic/默认账户仅查询一次、精确拒绝且未准备 worker。[回复检查](../sandbox-guest-service/src/command_requests/http/start/tests/expansion.rs)覆盖完整文本/转义、identity/gzip 精确 raw/wire 预算、期限、最后字节副本和观察与原粗错误匹配。固定 Go R 增加 36 组请求/默认路径、用户名和编码/charset，实际运行已固定的 originalExpandAndResolve 与 Connect 完整回复；不运行原进程 handler，不计完整 E03。[Linux HTTP 场景](../sandbox-guest-runtime/tests/preparation/expansion_http.rs)核对六组 Basic/默认账户及 cwd 拒绝、缺失 worker 仍不被调用、原 PID 文件字节不变、无 socket/用户副作用、容量直到最后副本才释放和后续合法 List；另核对默认账户错误优先及有效 cwd 覆盖坏默认目录后原命令真实结束。新增 Linux 场景已在 [#469 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38666731692) 实跑通过：源 4eed2f31f6ad6d5a8205e4cce3362b405163361c、checkout d1b00ca02492ef248be0ddde6907f74e40f61b44，process 22 passed、preparation 52 passed、service lib 408 passed / 66 ignored；固定 SDK 77 项、八轮 PID/80 个错误调用、封存 Redis 与四项检查通过。

[Linux PID 拒绝连接](../sandbox-guest-runtime/tests/preparation/operation_replies.rs)新增真实 worker/登记、缺失 PID 优先于参数、缺失选择器/输入与未知信号不影响原进程、后续合法 TERM、保留 Connect 与终局过期拒绝，以及 tag/无格式保持未分类。每次核对完成鉴权释放和原回复最后副本容量。[固定 SDK 调用集](../tests/compat/guest_registered_sdk.py)每轮新增七个原生成客户端错误调用，八轮共 56 个，使用原 ConnectError 的 code/message 后继续原命令的重连/输入/PTY/终止；它不等同于高层异常映射或 Go 双端 E03。两个 Linux 场景及八轮共 56 个 SDK 失败调用已在 [#453 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38595883687) 实跑通过：源 cbb27f2781777d57e52b7a3a9774bda66b1ee42f，checkout e1ed246c35b3566766bd91ad8e1c203548baa758；preparation 30 passed，service lib 385 passed / 60 ignored，四项检查、原固定 SDK 和封存 Redis 均通过。固定 Go R 对照仍为本地显式证据，不计作普通 CI 已执行。

F 与 Q 的一元 JSON 回显分开报告，共用 SANDPROOF_GUEST_CONNECT_REFERENCE 并显式 --include-ignored；参考二进制须包含 F 入口。新增 EOS 本地对照及可移植规范化检查已执行；Linux 默认账户顺序测试加入零长度 Start 帧及带客户端 error 的合法 EOS，确认仍查询原默认账户，已在下述 #448 CI 通过。

[正常命令回复检查](../sandbox-guest-service/src/command_requests/http/response/tests.rs)覆盖协商 charset/identity/gzip、短消息压缩、每帧预算与无累计上限、真实编码长度头、原期限、截断/非法帧、最后副本反压和排队编码取消后实际工作退出。固定 Go N 入口复用原生成消息及 Connect handler，以 164 组 List/控制回复、Start/Connect 事件、Unicode/长文本和两种编码/类型比较真实头、flags 与解压后的 JSON 值；压缩字节流及 protobuf JSON 空白/键序不要求一致，长度头先核对各自实际字节再比较是否存在，不据此宣称跨编码器 2048 临界字节完全相同。N 共用 SANDPROOF_GUEST_CONNECT_REFERENCE，须重建并显式 --include-ignored。

[Linux 正常回复连接](../sandbox-guest-service/src/command_requests/tests/http.rs)核对原选择/账户预检、List、gzip 回复到原期限和请求名额，旧未选择入口保持未分类；固定 SDK/PID 测试路由改用 Reply::into_http_response 输出成功头和正文。该连接和八轮真实命令 SDK 回归已在下述 #450 CI 通过；不把可移植编码、交叉静态检查或受限测试路由记为请求 gzip、正式服务或完整 E03。

[命令请求 gzip 检查](../sandbox-guest-service/src/command_requests/compression/tests.rs)覆盖 wire-empty/展开为空、首消息与尾帧错误顺序、首头/正文故障、原始和展开预算以及取消/期限；排队解压另以单阻塞线程屏障核对原鉴权与容量直到实际退出才释放。固定 Go E 入口复用原生成消息和 stream/unary handler，以 3,116 组七种命令、拼接/空/截断/损坏 gzip、全部 flags 和压缩 EOS 比较接受性及解码值；不把粗分类当作公开错误全文。E 共用 SANDPROOF_GUEST_CONNECT_REFERENCE，须重建参考二进制并显式 --include-ignored。

[Linux gzip 请求连接](../sandbox-guest-service/src/command_requests/tests/http/gzip.rs)核对七种方法、分块输入、格式/期限和原容量；[Start 顺序](../sandbox-guest-service/src/command_requests/tests/start/gzip.rs)核对 gzip/framing/预算失败先于默认账户及准备，合法压缩请求才查询原默认账户，gzip 账户拒绝保持最后副本责任。SDK/PID 路由迁移到 Selected::decode，固定 SDK 自身仍发送 identity，不能把该回归计为原 SDK 发送 gzip。两项 Linux 连接与迁移后的 SDK 回归已在下述 #451 CI 通过；组件对照不代替完整 E03/E05。

[命令 gzip 回复检查](../sandbox-guest-service/src/command_requests/http/reply/tests/gzip.rs)覆盖真实解压阶段到固定回复、原始/压缩精确预算、原期限、最后字节副本及未分类故障。Go E 的可选 Reply 观察与原接受性入口分开，以 828 组七种命令、两种 JSON 类型、首消息/压缩 EOS 和首头/正文/拼接成员失败比较真实状态、头、flags 与完整解压错误字节；不要求 gzip 位流相同。须重建参考二进制并显式 --include-ignored；它不执行命令或比较任意 DEFLATE 诊断。Linux 原鉴权/选择/解压到 into_response 的连接另核对默认格式、完成鉴权释放和最后副本容量，已在下述 #452 CI 通过，独立于 #451 请求解码证据。

[Start 默认账户回复检查](../sandbox-guest-service/src/command_requests/http/default_account/tests.rs)覆盖原始/压缩载荷独立限额、原期限、缺失观察/选择与最后字节副本。固定 Go D 入口用原生成 Start 消息及 Connect handler，比较 20 组 JSON 类型、identity/gzip、空/普通/转义/控制/长用户名的真实 HTTP 头、flags 和 JSON 原字节；失败表达来自固定 GetAuthUser，账户查询为受控失败，不运行进程。D 共用 SANDPROOF_GUEST_CONNECT_REFERENCE，须重建并显式 --include-ignored。独立 gzip 编码通过不代表 gzip Start 连接已经 Linux 实测。

[Linux Start 连接检查](../sandbox-guest-service/src/command_requests/tests/default_account.rs)另核对原 token/Basic/选择/解码到默认查询的顺序、原格式与期限、查询次数、鉴权释放、Bytes 副本、未分类源故障和无选择旧入口；缺失 worker 路径与空闲进程/登记名额确认这些失败不进入准备。两项 Linux 场景与原 Start/SDK 成功回归已在下述 #447/#448 CI 通过；不从可移植错误编码推算 Linux 组合、完整 E03 或资源峰值通过。

[命令 token 回复检查](../sandbox-guest-service/src/command_requests/tests/token.rs)覆盖原 Session 到回复的拒绝顺序、未初始化/重复 token 的既有规则、HEAD 完整预算与长度、最后 Bytes 副本、原期限和未分类失败。原 WithAuthorization 的 R 入口扩展可选路径，命令测试在七个 process 路径间轮换，以四种 HTTP 方法及 134 种 token 输入比较 536 组真实 HTTP 状态、头和 JSON 原字节；受控初始化 token store 不等于真实 MMDS/命令执行。SANDPROOF_GUEST_AUTH_REFERENCE 须按固定 Go 重建并显式 --include-ignored，原文件路径缺省和测试保留；SDK/PID 路由改用 authenticate_http 后的八轮 Linux 成功回归已在下述 #447/#448 CI 通过，不计完整 E03。

[命令头错误响应检查](../sandbox-guest-service/src/command_requests/http/reply/tests.rs)覆盖最后字节副本/取消/原期限、逐块背压、原始及压缩帧上限、未分类拒绝和 Debug 脱敏。显式固定 Go W 入口对七种命令、两种 JSON 类型、两种协商和七种拒绝组合比较 196 组真实 HTTP 状态/编码广告/长度、唯一终结帧 flags 和解压后的 JSON 原字节，包含超过 Go 自动长度阈值及非 ASCII/HTML 转义；不要求 gzip 压缩流逐字节相同。共用 SANDPROOF_GUEST_CONNECT_REFERENCE，必须重建并显式 --include-ignored；不计作普通 CI 已执行。[原准入连接检查](../sandbox-guest-service/src/command_requests/tests/http.rs)另覆盖选择失败到实际响应/失败的名额与鉴权释放，该 Linux 场景已在下述 #442/#447 CI 通过。尚无正式监听、完整错误覆盖或 Go/Rust 同 SDK E03。

同一 W 入口另有 126 组非 POST 对照：七种命令、六种 HTTP 方法与三种媒体类型，同时叠加坏压缩/版本/期限，核对最先返回的 405、Allow: POST、空正文及 HEAD 的长度头差异。可移植检查覆盖空 body 在 EOF、丢弃或原期限到期后的名额释放；Linux 原 Basic 预检到 HEAD/GET 回复的连接用例已在下述 #442 CI 通过，后续候选独立重跑。

[显式账户响应检查](../sandbox-guest-service/src/command_requests/http/account/tests.rs)覆盖中间件独立格式、空/错误版本、动态类型头和正文副本共同持有容量、原期限/预算与未分类故障；共享 MIME 规范化另在 runtime 检查额外参数、RFC 2231、原坏语法及表示边界。固定 Go Y 入口通过原 authn/Connect 对照 720 组方法/类型/版本/用户名组合，比较真实 HTTP 状态、头、终结 flags 和 JSON 原字节，包含重复字段首值、控制/非 ASCII 字符、空名及超出自动长度阈值的错误；账号查询失败为受控结果，不等于真实 NSS 对照。Y 共用 SANDPROOF_GUEST_CONNECT_REFERENCE，须重建并显式 --include-ignored。

[Linux 账户连接](../sandbox-guest-service/src/command_requests/tests/account.rs)检查原 token/Basic 到回复的拒绝顺序、查询次数、鉴权释放、头/正文名额和未分类源故障；原阻塞账户超时场景及八轮 SDK/PID 测试路由迁移至 prepare_http。新增及迁移后的 Linux 执行已在下述 #447/#448 CI 通过，本地 Go 和静态检查另行报告；不覆盖默认账户 Start 错误、正式服务或完整 E03。

[命令 HTTP 选择检查](../sandbox-guest-service/src/command_requests/http/tests.rs)覆盖七种方法的一元/流式 JSON、重复头首值、MIME 参数、压缩字段分离及 q 非权重、拒绝顺序、期限与失败格式。显式固定 Go V 入口经原生成消息和 Connect v1.18.1 真实回环对照 329 组选择/状态/错误文本；handler 为受控回复，不执行命令，也不证明原 Start 在零/负期限没有副作用。[Linux 所有权检查](../sandbox-guest-service/src/command_requests/tests/http.rs)核对失败持有原请求但释放完成的鉴权锁，gzip/过期不读正文，成功解码保留同一 Ready/期限；该提交已在下述 #440 CI 实跑，后续候选仍须独立重跑。旧 identity 入口仍拒绝请求 gzip，新 Selected::decode 的 gzip 检查见上文；完整错误映射尚未完成，不能计完整 E03。V 共用 SANDPROOF_GUEST_CONNECT_REFERENCE，需重建参考二进制并显式 --include-ignored。

#440 的 HTTP 选择和八轮 SDK/PID 组合已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38567961659) 通过：源 411c9fec8f511eedb3141dc01ff60fe098cbe050，checkout 8756de4a0c3ec2b20892e88fd966c16d2bb7d1d8；新增两项选择所有权检查、preparation 28 项、service lib 349 passed / 51 ignored、八轮 SDK 实进程及四项 CI 均通过。显式固定 Go 对照另在本地运行，不能由 CI 的 ignored 推算通过；此父提交不覆盖新增头错误响应。

#441–#448 的头/方法、显式账户/token/默认账户与空帧/EOS 连接已有累积 Linux 证据：[#442](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38571233313) 源 065b70f0b6e7f19bf6d84f95ad207e1dc6be38b1、checkout 736bf8496a94526419af12e0fcda3218f7deedf3；[#447](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38583740598) 源 db327e9ea840be23d6db69a5224b7b8968d2b58e、checkout a15e028333dcacc078c0bedca138e65fb52e0d18；[#448](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38586175865) 源 ad6fbfb3ea226f0a54c7a358892851d6e094adb8、checkout 4d06bfa9e90c219fe3cd5930e7840c623e86928b。三次四项 CI 均通过；#447/#448 的 service lib 各 368 passed / 56 ignored，含账户顺序/所有权和八轮 SDK/PID 真实进程，#448 另覆盖合法 EOS 的默认账户顺序。固定 Go 显式对照仍另行执行，不由 ignored 推算通过。

正常回复连接另在 [#450 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38589161073) 通过：源 f5a8eb8db49def00b9cce4d1e69cc4dae9b21a3e、checkout 53060df12ae418172753eb6a2a74bb37aa976c6c；四项 CI、77 项 SDK 合同、八轮 SDK/PID 真实命令、List gzip 原容量测试及封存 Redis 均通过，service lib 374 passed / 57 ignored。该证据不包含后续请求 gzip 连接，也不是正式服务、Go 双端或完整 E03。

请求 gzip 在 [#451 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38592180107) 通过：源 530227c14c5a4bea64174536646feb5242bb2ac0、checkout 9041fdd0c8b2c34758286e8dcc95381fdc68eeb7，service 380 passed / 58 ignored，含七方法解压、Start 失败顺序及排队取消。已知 gzip 回复在 [#452 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38593528212) 通过：源 0e15d65664d9c6573a47f02de1a5764b1e5388b5、checkout f5793b488ddf28baa5bfe482b6410517de904e73，service 383 passed / 59 ignored，含原选择到错误响应及最后副本。两次四项 CI、77 项 SDK、八轮真实 PID/SDK 与封存 Redis 均通过；固定 Go 显式对照独立报告，不包含后续 PID 操作错误或完整 E03。

上述 Start 组件的六项账户/准入/实进程回归已在 [#438 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38562089660) 实跑通过（源 550ca46ecb595d25932936a33999fe4fec8cf5fa，实际 checkout e596ebabe93f29dc90504b2e8d2d4d6f34145bb1）：preparation 28 passed、service lib 344 passed / 50 ignored，SDK 与封存 Redis 检查通过。该精确父提交证据不覆盖新增 SDK/PID 组合或完整公开端点。

[命令响应检查](../sandbox-guest-runtime/tests/command_responses.rs)补充 List/空回复和事件默认值、optional/oneof 存在性、完整长度及转义后预算边界。显式 SANDPROOF_GUEST_CONNECT_REFERENCE 入口已在 macOS 与原始生成 process.pb.go（Git blob bdfc62a0aa95980ee9c15ec03a106ceaf9ff2449，SHA-256 e35da822dfb2870b1a6441c536b215f7a2503b37674e12a197f45f9caa083cf5）、Go 1.26.8、Connect v1.18.1 和 protobuf v1.36.12 完成 259 项真实回环 HTTP 对照：五种一元回复以及 Start/Connect 的 Start/Data/End/keepalive 和独立终结帧，包含全部 byte 值、整数边界、空值/存在性与 Unicode；只规范化 JSON 空白和对象键序，验证原帧长度，不声称字节级 JSON 相同。Go 方法为受控回复，不执行用户进程或 List 活跃表。Linux 原 SDK 组合另新增每轮三种合成 List 回复（同步/异步共 30 次），已在 [#429 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38534107339) 通过（源 0ea617e03aa54bd4165ef7ed7547362097eeb396，实际 checkout 800be298e5be7772ba6664f92ef903988217b0e6）；该次四项 CI、原命令/文件及封存 Redis 也通过；不把合成列表当作已接通公开登记。普通兼容入口运行两项独立响应测试及原协议测试，Go 对照仍需显式执行。

[命令请求 JSON 检查](../sandbox-guest-runtime/tests/command_requests.rs)有三项普通回归和显式固定 Go Q 对照：以原始生成 process.pb.go 与默认 Connect codec，在真实回环 HTTP 上比较 2,824 组解码值或 invalid_argument 分类；包含七种请求、未知字段/枚举、重复 null 字段及环境键、精确有符号/无符号整数和裸指数、深度边界、每个原始 byte 及 base64 字母表/尾位/换行/padding。Q 使用七种请求消息的一元回显，包括 Start/Connect 请求类型，只隔离 JSON 行为，不代替 Start/Connect 流 framing、错误全文或进程操作。已在 macOS 显式实跑通过；Go 1.26.8 参考二进制 SHA-256 cfb9d72db3915368f6d268953902c37165857c92246c12727f6d3d31e192a484。三项普通回归已在 [#432 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38540335634) 通过（源 bdc6a31da456605095a6ef790eb8d1b8c6a11f05，实际 checkout 13c002cbaa3ae95484cadf2e100631c427501378），四项 CI 与封存 Redis 均通过；Go Q 在该 Linux 运行中仍忽略，需 SANDPROOF_GUEST_CONNECT_REFERENCE 并显式 --include-ignored。共用文件扫描器保留原文件哨兵行为，原文件 Go 对照独立回归；不把这批解码结果记为完整 E03。

[文件协议检查](../sandbox-guest-runtime/tests/file_protocol.rs)消费同一文件共享样例，核对重复/畸形消息、缺省/null、uint32 深度、int64 尺寸、可选链接字段、时间戳精度/边界和编码预算。[文件上下文测试](../sandbox-guest-service/src/files/tests.rs)覆盖 Basic 与 HTTP username 的分离、一次 Move 查询、默认值/环境不改变身份、词法路径、无效 UID 和展开后预算。Linux 兼容入口还运行[固定同步/异步 SDK 组合](../sandbox-guest-runtime/tests/sdk_files.rs)：原会话授权与路径解析接到文件编解码，核对各一元方法、大尺寸/纳秒/元数据/空列表/可选链接及超额错误。该测试只返回预定合成属性，没有实际 stat、目录修改或文件传输；真实文件权限、链接竞争、磁盘满、部分读写、Go 双端对照和 KVM 仍须独立执行。

[查询绑定检查](../sandbox-guest-runtime/src/file_query/tests.rs)及[错误回执检查](../sandbox-guest-service/src/file_query/tests.rs)覆盖原字节/空值、畸形字段局部丢弃、固定字段顺序、int64 边界与 unsigned 溢出优先级、完整原始输入预算、非 UTF-8 显式拒绝、Go Quote 线长预算及原守卫/期限。可移植定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_query`。固定 SDK 文件测试路由也迁移至共用绑定：原 SDK 读写带加号/百分号/空格的文件名，合成 GET/HEAD/POST 检查重复/非法过期时间先于鉴权和文件效果，HEAD 核对空正文与原长度头，GET/POST plain text 400 经原 SDK 错误解析器核对；Linux 组合待实跑，不能计为完整 E03 或签名验证。

[查询参考适配器](../tests/compat/query-reference/main.go)使用独立锁定的 Go 模块，直接调用 Go 1.26.8 URL.Query 和 oapi runtime v1.6.0 原绑定函数，字段顺序、类型及错误包装来自固定 envd `f455fd58fa3f289e78ce1de690c35968d8d29322` 的 GetFiles/PostFiles 生成代码；main.go 的 HandlerFromMux 使用原默认 ErrorHandler。对照包含种子 0x311、任意百分号字节、非法/Unicode 输入及 2047/2048/2049 字节响应；HTTP 模式比较 GET/HEAD/POST，只调用 http.Error 的真实自有回环服务，不执行完整 envd 或文件操作。已本地显式运行，默认 ignored，普通 CI 不制备 Go 或下载模块。先按 go.mod/go.sum 准备固定依赖，再执行：

```sh
cd tests/compat/query-reference
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -mod=readonly -trimpath -buildvcs=false \
  -o /absolute/path/to/query-reference .
cd ../../..
SANDPROOF_GUEST_QUERY_REFERENCE=/absolute/path/to/query-reference \
  cargo test -p sandproof-guest-service --locked --lib file_query -- --include-ignored --nocapture
```

[文件鉴权检查](../sandbox-guest-service/src/session/file_authorization/tests.rs)和[会话回归](../sandbox-guest-service/tests/session.rs)可在原 macOS/Linux 库运行；会话与初始化 HTTP 组合不依赖 Linux 系统调用，现不再按平台隐藏。覆盖令牌优先/空头回退、非法签名先于过期、相同到期秒、当前快照后读时钟、字段/操作绑定、全部头和保留字节预算、无初始化拒绝、候选轮换失败/取消、原许可阻止轮换和原绝对期限。401 JSON 与最后字节副本保留原守卫另行检查。请求准入组件另检查 HEAD 的完整编码预算、空正文、原长度头、丢弃和到期释放；Linux 公共文件入口核对拒绝前不读正文/查账户。固定 SDK 组合增加合成 HEAD 鉴权拒绝，尚未 Linux 实跑；HEAD 不经 SDK 文件错误解析器断言不存在的响应正文。定向入口为 `cargo test -p sandproof-guest-service --locked --lib session::file_authorization` 与 `cargo test -p sandproof-guest-service --locked --test session`。

[固定 Go 鉴权参考](../tests/compat/guest_authorization_reference.go)保留原 generateSignature/validateSigning 函数体和 SHA-256/base64 方法；仅将令牌存储/日志依赖换为测试替身，清零函数名及 Unix 时钟注入改变，不执行原存储实现或完整 envd。显式组比较签名和允许/拒绝文本，包括读写、缺省/空用户、Unicode/分隔符路径、无/负数/int64 边界到期时间、错误令牌与错误签名；另经原 jsonError 和真实 Go 回环 HTTP 比较 GET/HEAD/POST 四种原签名拒绝；这些有限对照本地已运行，默认 ignored。未初始化匿名回退和重复令牌差异由受保护配置测试单独核对，不能用此对照宣称两种配置完全一致。

```sh
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/auth-reference tests/compat/guest_authorization_reference.go
SANDPROOF_GUEST_AUTH_REFERENCE=/absolute/path/to/auth-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  session::file_authorization -- --include-ignored --nocapture
```

[文件请求准入检查](../sandbox-guest-service/src/file_requests/tests.rs)覆盖共享池无排队、非法预算、头/查询预算及绑定先于鉴权、专用许可持续占额、400/401 错误对象/正文/最后字节副本的所有权、超时与编码失败后的原名额释放。[会话鉴权检查](../sandbox-guest-service/src/session/file_authorization/tests.rs)另用实际写锁和可推进时钟检查取消/到期的鉴权等待释放原容量且不遗留读者。可移植定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_requests` 及上述会话定向入口。它不代表套接字/请求头读取、正式拒绝策略或系统内存峰值已经验收。

新增 Linux [下载许可组合](../sandbox-guest-service/src/file_transfer/download/tests/request/authorization.rs)和[上传许可组合](../sandbox-guest-service/src/file_transfer/batch/tests/authorization.rs)检查错误操作/到期先于账户和文件效果、原 query 到实际路径/用户、鉴权期限不在正文重新起算，以及在途下载字节保留原槽位；入口请求名额另随阻塞账户查询取消、上传成功/拒绝的最后响应副本和下载原期限保留，这些实际执行待补。固定同步/异步 SDK 文件测试路由全部经共用请求准入取得文件专用许可，另用原 download_url/upload_url 生成签名，仅将 URL origin 改到自有回环夹具，经原查询访问 raw/multipart 上传及下载，检查修改路径/用户/操作、过期、错令牌及令牌优先的 401 文本；这是新增 Linux 组合入口，未执行不得计为通过，不代表公开直连模式、中心/宿主签名 URL 转发、E03 或 KVM 验收。

[共用文件 HTTP 编排检查](../sandbox-guest-service/src/file_http/tests.rs)新增真实目录内 framing 已知/未知长度上传、实际下载、多范围动态边界和完整线长、拒绝时不读取正文/不查账户、满池/错误路由无副作用，以及未映射账户错误保留原请求责任；定向 Linux 入口为 `cargo test -p sandproof-guest-service --locked --lib file_http`，实际运行待补。SDK 文件回环夹具已调用共用 FileHttp，保留原目录前置条件与调用集，未映射错误不再由夹具生成通用 JSON。可移植 [boundary 检查](../sandbox-guest-service/src/file_download/boundary.rs)、[计划检查](../sandbox-guest-service/src/file_download/tests.rs)及[流执行检查](../sandbox-guest-service/src/file_download/stream/tests.rs)核对 30 字节编码、熵失败无回退、仅实际多范围请求调用熵源，以及 worker 生成边界后的实际正文/Content-Length；OS 熵 smoke 不证明熵质量。这些不代替真实 socket framing/连接预算、Linux 文件/SDK、完整 E03 或监听器部署验收。

[文件 HTTP 配置检查](../sandbox-guest-service/src/file_http/tests/configuration.rs)在装配入口覆盖 16 类工作目录/上传/下载/响应拒绝，包括未被请求选择的 gzip/MIME 模式、整数上界与固定 read lookahead 溢出；[一元文件配置检查](../sandbox-guest-service/src/file_rpc_http/tests/configuration.rs)覆盖八类上下文及回复拒绝。核对零账户调用、无文件效果及请求/传输容量保持，合法的微小回复预算、零字节配额和不存在的绝对目录仍可装配。低层已有编解码回归继续验证共享校验器；方法拒绝的发送失败回归使用合法配置，以独立单 blocking-thread runtime 和明确屏障排队原响应编码，再推进假时钟越过文件池的原 5 秒期限（6 秒，避开计时器毫秒舍入），立即轮询必须得到超时，不能等真实阻塞线程看门狗释放编码任务；要求 DeadlineExceeded、零账户/文件效果、原两池在失败及丢弃失败后仍由未结束编码任务占用，释放屏障后实际归还并可完成下一次 MakeDir。同一路径的可移植排队编码回归已在本地通过；新增构造及方法失败组合已在 [#478 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38678604725) 通过（源 45f3960bcd6405e7b82671dd09dba3fae2c03e7d，checkout 9006158270db76df1b3a71495e07541aa9109eac）：service lib 418 passed / 70 ignored、固定 SDK 77 场景、封存 Redis 与四项 CI 均通过。没有实现正式 daemon 配置解析/加载，也不计为 E05 实测峰值或 E03 完整双端对照。

[未知文件账号响应检查](../sandbox-guest-service/src/upload_response/account/tests.rs)覆盖 GET/HEAD/POST 的固定 401、JSON 转义、Go 小响应 Content-Length 边界、HEAD 抑制、精确响应预算、原期限及最后字节副本释放；可移植测试及固定 Go 有限对照已本地运行。对照复用原 upload reference，显式注入 Go `user.UnknownUserError`，只核对原包装/jsonError/实际回环 HTTP 写出，不代表真实账户数据库或完整端点。Linux FileHttp 检查新增空/缺失/特殊字符用户名、无正文读取/文件效果、编码首头先于账户、账户先于后续阶段、不可用源保持未映射分类；实际执行待补。SDK 文件组合新增原同步/异步 read、raw/multipart write、gzip write 的未知账号 AuthenticationException；SDK 会省略空 user，因此显式空 username 另由合成请求经原 SDK 解析器核对。该扩展也待 Linux 实跑，不能记为 E03 通过。

[路径展开错误检查](../sandbox-guest-service/src/upload_response/path/tests.rs)对照固定 permissions 的原 expand/ExpandAndResolve 和 GET/POST 错误包装、真实回环 HTTP jsonError，覆盖特殊字符、默认工作目录触发时原查询为空、HEAD、响应字节上限及长错误头/正文；该有限可移植对照已本地运行，不执行真实账号或失败路径 I/O。Linux [上传路径检查](../sandbox-guest-service/src/file_transfer/batch/tests/upload_path.rs)覆盖 raw 拒绝不轮询 identity 正文、旧粗分类、identity/gzip multipart 前文件保留且后文件不执行、查询覆盖非法文件名；[下载请求检查](../sandbox-guest-service/src/file_transfer/download/tests/request.rs)覆盖账户先于展开、展开先于 stat/协商、默认值与 HEAD。这些仅已交叉检查，实际运行待补。固定 SDK 组合新增原同步/异步 multipart 批次失败后回读已完成文件，Rust 检查后续目标未创建；其 Linux 实跑待补。原 SDK gzip=True 会强制 raw 上传，因此压缩 multipart 只归入组件场景，不能标成 SDK 发出的压缩 multipart。

[一元真实文件检查](../sandbox-guest-runtime/tests/filesystem.rs)使用独占临时目录，覆盖属性/特殊类型、断链与长链接/循环、xattr 与 UTF-8、词法父目录、深度/别名/顺序、覆盖移动及失败后的父目录、递归删除大批次与部分删除预算。普通 CI 禁止创建 SUID/SGID 文件，真实属性夹具使用普通权限；[权限字符串向量](../sandbox-guest-runtime/src/filesystem/metadata.rs)核对固定 Go 的特殊位/类型格式，实际 SUID/SGID 属性读取保留在下面的显式 Go 对照中，不算普通 CI 的内核权限实测。[删除交错检查](../sandbox-guest-runtime/src/filesystem/remove_tests.rs)在真实目录打开后精确替换为链接，核对目标未被遍历；这是指定交错，不能覆盖全部并发文件系统变化。[文件工作池检查](../sandbox-guest-service/src/file_io/tests.rs)以通道屏障和可推进时钟核对取消/超时后授权与额度、结果保留、异常和释放，未轮询请求无副作用。新增[一元请求组合检查](../sandbox-guest-service/src/file_io/rpc/tests.rs)核对满额/到期先于账户查询、取消/到期后的账户与授权保留、Move 部分效果及结果/错误保留原名额；文件工作池另以单线程阻塞池和屏障核对排队到期不执行及运行中到期不提前归还请求所有权。这些一元编排组件已在上述 #408 的同一 Linux CI 运行；普通 Linux 基础检查与兼容入口执行这些组件。

[真实文件 SDK 组合](../sandbox-guest-runtime/tests/sdk_file_io.rs)通过原授权、真实本地账户、路径解析及文件池，让固定同步/异步 SDK 在各自临时树上实际执行五种一元方法，核对链接/元数据、错误、创建与移动、失败后父目录、删除及配额失败后的正常请求。夹具已迁移到共享 FileRpcHttp 的 prepare/execute 入口，内部复用 FileRpcRequests→FilePool::prepare_rpc→ReadyRpc::execute 编排，在原始 Body 提取后、正文读取前准入，再经受保护鉴权、Basic 前置查询、有界正文解码和同一封装进入文件池；固定调用集使用 JSON；原 40 个文件场景外，新增同步/异步各一次真实显式账户查询成功，以及显式空/未知/转义用户名与未知缺省账户的固定 ConnectError code/message，共 10 个账户场景。未知缺省账户有独立会话和路由，Rust 检查拒绝请求未创建目标。成功响应使用 FileReply 的有界 identity/gzip 编码、原期限和需求驱动正文，账户及已映射 Stat/MakeDir/Move 错误使用实际 RpcFailure::into_response，原 SDK 同步/异步调用逐一核对完整 code/message，另新增缺失目录/普通文件 ListDir 失败，另加同步/异步 Remove 的末尾点/非目录父路径失败，再加同步/异步 Stat 与 Move 源路径的展开失败，再加同步/异步各一次 gzip 成功解码与协商 gzip 后的 identity 错误，再增加同步/异步各一次显式 gzip 请求成功/错误，共 60 文件 + 10 账户 + 2 timeout 场景；目录配额仍使用测试合成错误并在 Bytes 中保留实际失败所有者。原 40 文件场景及 10 账户场景已在 [#408 的 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38503390750) 通过（源 b125b717307380f549635815ce134da2384aed61，合并测试 9b76a3879072bedd3d0292b2f6d8abc9b2f5992f），不计为完整双端对照。仅测试路由开放回环端口、接受预定目录内路径；没有 HTTP 上传/下载、特权账号切换、正式监听器或 KVM。

[共享一元 HTTP 检查](../sandbox-guest-service/src/file_rpc_http/tests.rs)核对五种真实操作、规范化 charset、prepare 阶段无文件效果、拒绝前不读取正文、显式账户先于方法/头拒绝、共享入口满额、最后字节副本、取消正文后初始化/容量恢复，以及成功 mkdir 后回复编码失败保留原期限和已完成效果。六项 Linux 组件及 SDK 同一 52 场景的新入口迁移已在 [#411 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38506866432) 执行通过（源 0871dcbb35d2238a0324d3631f848180c6a6c33f，合并测试 6614fd276bce9924ba34f6bdb97888e880117e52）；定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_rpc_http`。SDK 夹具仍在 prepare 后检查临时树内的不可变路径，Stat/MakeDir/Move 文件阶段错误、rename 前置检查、worker 错误自身保留容量、SDK 精确文本已在 [#412 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38508480755) 执行通过（源 de22025ea9d861b3df9634728474c47485117a20，合并测试 e773c89066729ceb4bc426e64f46c6ab366b74f8）。目录阶段六项 runtime 与一项 HTTP 组件、SDK 56 场景已在 [#413 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38509964858) 通过（源 89b67bdee3e022d34952e068e2b5686a33eb823f，合并测试 204705dd87aa85c08bd49e0b231a5d52c44a2d63）。Remove 四项新增 runtime、一项 HTTP 组件及 SDK 60 场景已在 [#414 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38511250356) 通过（源 d270b4ec7de406c425468551618c500bc8d21a6d，合并测试 4c27a17823e0fd60859fba73f5e9cfd4182a803b）；路径展开四项新增 service 组件及 SDK 64 场景已在 [#415 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38512018326) 通过（源 042b96e015652b6517f880620c345181a4db4ea4，合并测试 d0e1ba520b50554661d9a576c190c0161aae8140）；gzip 响应和请求的后续 Linux CI 证据分别见下文；新错误响应候选须另跑适用检查。

[一元文件错误检查](../sandbox-guest-service/src/file_rpc_response/io/tests.rs)可移植核对 lstat、MakeDir 初始 stat/已存在、rename、父目录 stat/mkdir/chown/非目录九种阶段的固定文本、Connect code/status/头、特殊字节与 Go JSON 转义、精确编码预算、缺失观察和最后字节副本的容量保留。权限/磁盘满/chown 的 errno 向量是受控格式检查，不是特权或磁盘故障实测。工作池另检查实际错误结果自身继续保留名额，返回后文件被替换也不重查/改变原诊断；共享 HTTP Linux 组件新增真实缺失路径、非目录、已有目录/文件、循环链接、Move 部分效果及错误编码超限后的原期限/双槽位检查；以上 Stat/MakeDir/Move 新增组件及原 SDK 52 场景中的精确 message 断言已在上述 #412 Linux CI 执行。

目录错误新增 13 组可移植格式向量，覆盖解析/目录检查/遍历各阶段、内部 Connect code 前缀与 HTTP 状态，以及原路径 Go %q 组合转义、上下文角色错配和精确预算。Linux [路径解析交错](../sandbox-guest-runtime/src/filesystem/path/tests.rs)核对中间路径未提前清理、断链/循环、尾斜杠、readlink 前链接被替换，另以受控 errno 检查原 too many links 文本优先级；[目录交错](../sandbox-guest-runtime/src/filesystem/directory/tests.rs)核对解析后根消失、根 lstat 不跟随替换链接、遍历后段失败不返回已有结果及打开目录失败。共享 HTTP 新增七种目录失败及后续正常请求，检查原请求/文件容量。六项 Linux runtime 组件、一项 HTTP 组件及原 SDK 同步/异步各两项 ListDir code/message 检查已在上述 #413 CI 通过，不冒充权限、磁盘或全竞争矩阵已通过。

Remove 新增五组可移植阶段格式向量及上下文角色拒绝；Linux 删除检查覆盖真实末尾点、非目录父路径、NUL、首尾/中间斜杠，受控子项失败核对首次错误、兄弟部分删除、最终成功覆盖旧错误及诊断预算。受控读取错误核对外层完整路径与内层 basename，ENOENT 不执行最后 rmdir；注入 errno 只验证编排，不计为真实内核 I/O 故障。共享 HTTP 核对六种失败、原容量及后续缺失/正常删除，SDK 各新增两项精确 code/message。这些 Remove Linux 检查与 SDK 60 场景已在上述 #414 CI 通过；原 Go Remove 方法显式对照、权限/磁盘/全竞争矩阵仍未执行。

[一元展开拒绝检查](../sandbox-guest-service/src/file_rpc_response/rejection/path_tests.rs)核对特殊用户名的固定正文、精确转义预算、错误编码失败及最后字节副本的原容量；固定 permissions 原 expand/ExpandAndResolve 经原 Connect HTTP 的 12 组对照含显式路径、空请求触发默认路径、转义/NUL 与长用户名，已本地执行。Linux [HTTP 接线检查](../sandbox-guest-service/src/file_rpc_http/tests/path.rs)覆盖五种方法/Move 两端、显式/默认账户只查一次、默认工作目录拒绝、响应持有期间会话更新不改变原用户名、账户失败优先和编码失败原期限；旧 execute_rpc 入口另核对观察保留及路径预算错误不伪造展开诊断。以上四项 Linux 组件及原 SDK 同步/异步各新增两项错误已在上述 #415 CI 执行；临时路由只额外接受预定不可展开字面量，并核对 Move 没有创建父目录。

[JSON 类型选择](../sandbox-guest-runtime/src/file_protocol/content_type.rs)和[选择顺序检查](../sandbox-guest-service/src/file_rpc_requests/selection.rs)可移植执行，覆盖两种原 JSON codec、重复字段、参数/大小边界、方法先于类型先于压缩、原大小写及编码字段规则。共享 Go MIME 解析器的类型小写改用已有 Go 简单 Unicode 映射，含 U+0130；原 multipart 对照继续执行。固定 Connect 参考 C 操作在真实 HTTP handler 中观察 JSON 选择及规范化回复头，729 组有限向量含大小写、Unicode、quoted/extended/continued 参数、冲突重复值及逐字节变异，已本地通过。测试只观察 JSON codec，不能将其他结果理解为本模块已实现其他协议或固定 415；原始非 UTF-8 类型字段尚未接通。

[一元准入与正文检查](../sandbox-guest-service/src/file_rpc_requests/tests.rs)覆盖非法配置、共享池无排队、全头/鉴权拒绝、排队写锁下的鉴权取消/到期、已知与未知长度、逐帧/总量/trailer 预算、错误帧与不完整 EOF、完整 JSON 后等待 EOF、结果/失败保留名额，以及到期解码 worker 仍在单阻塞线程队列时保留授权与名额。使用实际锁、通道屏障和可推进时钟，取消后的初始化须能完成；这些可移植检查已本地运行。复用固定 Connect 参考工具，另将原 SetTimeout 的空值/长度判断和 Go strconv.ParseInt 决策投影成解析结果，比较 533 个有限向量（含正负号、十字节边界、非 ASCII 和每个字节）；该对照不创建真实请求期限、不靠 1 ms 竞速，也不代表完整 Connect 头/错误相容。定向执行见下方，显式参考不计为普通 CI 自动执行。

[Basic 解析检查](../sandbox-guest-service/src/basic_auth/tests.rs)可移植执行，覆盖缺省/畸形/显式空的区分、密码非 UTF-8、重复头与预算拒绝，以及非零尾部填充位不能误选默认账户。固定 Go 参考新增 B 操作直接调用原 http.Request.BasicAuth，1171 个有限向量核对 scheme、填充、每个字节、密码及原始 CR/LF；原始字节比较不代表非法 HTTP 头可被网络层接受，也不执行账户查询或 authn 中间件。普通三项与显式参考已本地运行。

[Linux Basic 编排检查](../sandbox-guest-service/src/file_io/rpc/tests/preflight.rs)覆盖显式账户仅查一次、缺省延后、换源也不刷新已有账户、原设置/期限、查账户先于非法正文/方法/timeout、满额/已过期零查询、未知用户错误保留容量而释放授权、运行中查询取消/到期、正文取消及已到期解码任务仍在阻塞队列时的授权/双槽位保留。真实临时目录、通道屏障和可推进时钟检查原请求/文件名额及初始化锁；该前置连接已在 #408/#409 Linux CI 运行。入口 `cargo test -p sandproof-guest-service --locked file_io::rpc::tests::preflight`。

[一元响应检查](../sandbox-guest-service/src/file_rpc_response/tests.rs)在 macOS/Linux 使用同一原文件 JSON 编码器，覆盖三个响应形状、编码边界/转义膨胀、原期限、背压、最后字节副本、取消编码 worker 及失败对象的容量保留。共享纯命令/文件协议测试也可移植执行，新增 Vec 扩容不越过线长预算的检查。Linux 一元组合另检查真实 mkdir 完成后编码超限、实际池期限先于原请求期限、初始化不被结果保留阻塞，以及到期正文副本继续持有请求/文件两个槽位；这些 Linux 所有权组件已在上述 #408 CI 运行，SDK 执行范围见上方；不用可移植测试代替。

[一元 gzip 响应检查](../sandbox-guest-service/src/file_rpc_response/tests/gzip.rs)覆盖三种成功形状、空对象也压缩、JSON/压缩后双字节预算、压缩载荷背压/取消/到期与最后副本占额，原屏障编码取消检查同时覆盖 identity/gzip。固定 Go Connect v1.18.1 参考新增 G 操作：12 组第一字段/重复字段/精确大小写/权重/空白输入 × 四种完整成功载荷和一个原路径错误，共 60 组本地回环 HTTP 对照，比较状态、选定响应头和完整解压正文，并核对存在的 Content-Length 等于实际编码字节数；不要求 DEFLATE 字节或跨压缩库的近 2048 字节边界一致。passthrough codec 不执行 protobuf，也不是完整 envd 双端。[Linux gzip 接线检查](../sandbox-guest-service/src/file_rpc_http/tests/gzip.rs)新增五种方法、charset、错误 identity、实际删除后压缩超限仍保留原双槽位/期限的两项组件；固定 SDK 工厂构造原同步/异步生成客户端时，显式设置 accept_compression=[GzipCompression()]，新增四项协商成功/错误检查；普通工厂默认关闭压缩且会覆盖每次调用的压缩头，不能仅传头冒充已协商。Rust 核对实际 gzip,identity 请求头和两次 gzip 成功响应计数。这些 Linux 组件及 SDK 68 场景已在修复客户端构造选项后的 [#416 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38514457822) 通过（源 945c2df0d8b2fbc54d453c16a982f97572f49e93，合并测试 136e4792110f2f4e8986e78f948210247148f779），service lib 286 passed / 41 ignored，封存 Redis 通过。初次夹具只有调用头而无构造选项，实际 gzip 计数为零的失败不计为压缩证据。

[一元 gzip 请求检查](../sandbox-guest-service/src/file_rpc_requests/tests/gzip.rs)新增四项可移植组件：五种方法与拼接成员、独立压缩/展开精确预算、校验/后续坏头/空输入、HTTP 完结与头拒绝顺序、取消/到期后排队 worker 的原授权与槽位。固定 Connect 参考 Z 操作对照 100 组：五组 Content-Encoding 第一字段/重复字段 × 五组 Accept-Encoding × 四种原始载荷，包含零字节、拼接空成员与跨成员 JSON；比较原 handler 解压内容和响应编码。passthrough codec 不执行 protobuf 或文件方法，畸形流公开文本不在此对照内。[Linux 请求接线检查](../sandbox-guest-service/src/file_rpc_http/tests/gzip_request.rs)新增两项组件，核对五种真实方法、显式账户不刷新、gzip 请求覆盖 identity 接受头、完整校验先于默认账户/文件派发、失败原双槽位及后续恢复。SDK 工厂保留标准环境/传输，构造原生成客户端时另显式启用 send_compression=GzipCompression()，新增四项真实请求及成功/错误解码；Rust 另核对四次 gzip 请求。八项同步/异步客户端选项探针已在本地合成 HTTP 验证；两项 Linux 请求组件及 SDK 72 场景已在 [#417 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38515049974) 通过（源 45163772170ddc5644a620546b3118fb2717757a，合并测试 643a0d54f92d36b8c46e771727672c683f7c7915），service lib 292 passed / 42 ignored，封存 Redis 通过。不是普通工厂的默认压缩行为，也不是完整双端/KVM 验收。

[一元 gzip 错误检查](../sandbox-guest-service/src/file_rpc_requests/tests/gzip_errors.rs)以两段压缩正文的所有非空截断前缀（排除完整首成员）、坏首头、CRC/ISIZE 和后续坏头核对实际 Header/Body 阶段，覆盖固定 400 JSON、精确响应预算、编码失败与最后副本占额、授权释放；预算/传输/期限和未知 DEFLATE 错误不能伪造公开诊断。固定 Connect G 操作另执行 46 种损坏输入 × 两种 Accept-Encoding，共 92 组实际回环 HTTP 对照，比较状态、选定头、实际 Content-Length 与完整 identity 错误正文；有限损坏流不代表全部 codec 错误或完整端点。Linux HTTP 组件将原损坏请求检查接到已知响应，并增加错误编码失败保留原双槽位/期限、无账户/文件派发及恢复检查；新增组合的精确提交证据见下文。SDK 夹具另增加同步/异步各五次合成损坏 gzip HTTP 输入，由固定 ConnectWireError 原解析器检查 code/message，Rust 核对十次已知拒绝及无目标文件效果；这十次独立于原 SDK 72 场景，不称为 SDK 自己编码的损坏请求。本地五种合成 JSON 解析探针通过；上述 Linux HTTP/文件组合已在 [#418 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38516493555) 通过（源 cb8ed7087ca8aedec8a7f54d41b4f6dba0e568b6，合并测试 733ebbf15a8c3c2b386155d5a6b8054431fe10d7），SDK 72 原调用及十次合成坏 gzip、service lib 295 passed / 43 ignored、封存 Redis 均通过。

[一元未知压缩检查](../sandbox-guest-service/src/file_rpc_requests/tests/unsupported_compression.rs)覆盖未知第一字段先于版本/超时/正文、合法首字段忽略后续未知值、固定 501 / unimplemented、精确回复预算、原收紧期限、到期不刷新和最后副本占额。固定 Connect H 操作新增原始压缩头，134 种未知编码（含全部 0x80–0xff 字节及 Unicode/转义）× 两组版本/超时，共 268 组实际 HTTP 对照，逐项比较状态、头和完整 identity JSON。Linux [入口检查](../sandbox-guest-service/src/file_rpc_http/tests/unsupported_compression.rs)核对显式账户错误优先、缺省账户不查询、正文不读取、无文件效果和编码失败保留双槽位/期限；原选择拒绝测试同步迁移。SDK 夹具新增同步/异步各两次合成 br/GZIP 请求，同时携带坏版本/超时，由固定原错误解析器核对四次 501 及无目标文件；与 72 个原 SDK 场景和十次坏 gzip 输入分开统计。本地七种错误 JSON 解析探针通过；新增 Linux 组合已在 [#419 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38517259405) 通过（源 d171ca79a81caa41520d1fd11ade4158b89e82f9，合并测试 13d785d7654bb307d5e0788a0444d12e48b14c4b）：SDK 72 原调用、十次坏 gzip 和四次未知编码、service lib 298 passed / 44 ignored 及封存 Redis 通过。

[一元账户回执检查](../sandbox-guest-service/src/file_rpc_response/rejection/tests.rs)核对两个拒绝阶段的文本/响应头、转义后精确预算、原期限、最后字节副本、编码排队取消及失败对象保留。Go 参考新增 A/U 操作使用原 authn v0.1.0 中间件和原 Connect v1.18.1 handler，显式路径同时给出非法 JSON/timeout 仍先返回账户错误；16 个实际回环 HTTP 向量含空/特殊字符及 2047/2048/2049/8192 字节边界，已本地执行。账户缺失是受控观察，不运行 Go 账户数据库或完整 envd；原 SDK 错误解析组合另行验收。

[HTTP 方法拒绝检查](../sandbox-guest-service/src/file_rpc_response/tests/methods.rs)覆盖空回复在 EOF、丢弃、原期限到达时的容量归还，以及无效配置/过期失败保留原所有者和期限。固定 Go Connect 参考 M 操作使用原生成消息及 schema，在实际回环 HTTP 比较五条一元方法 × 十种非 POST 方法 × 五组媒体类型，共 250 组状态、Allow/Accept-Post、其他选定头和完整空正文，HEAD 的 Content-Length 缺省不作归一化；同时携带非法压缩、版本、超时和 JSON，确认方法拒绝先行。显式入口为 `SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference cargo test -p sandproof-guest-service --locked --lib file_rpc_response::tests::methods -- --include-ignored --nocapture`。三个可移植测试及 250 组对照已本地通过。Linux [入口检查](../sandbox-guest-service/src/file_rpc_http/tests/methods.rs)检查五条路由/八种方法的缺省、有效显式和缺失显式账户矩阵，正文被轮询即失败；核对双槽位、初始化不被回复保留阻塞、失败的较早期限、无文件效果和后续恢复。这些 Linux 方法组合已在 [#424 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38525263351) 通过（源 8f32d7727de6082c77fb25a54a7ebcc794d64b9f，合并 checkout 2047c538bc409861eb7882f2bc032b577f8591e6）：service lib 306 passed / 49 ignored、原 SDK 72 场景及既有 10/4/8 次 gzip/编码/JSON 拒绝、封存 Redis 均通过，四项检查成功。普通 CI 未运行显式 Go 对照；方法输入不是 SDK 正常调用，不代表完整 E03 或正式监听器。

[一元令牌回执检查](../sandbox-guest-service/src/file_rpc_requests/tests/token.rs)由实际 Session 拒绝触发响应，覆盖缺失/空/错误/非 UTF-8/超令牌长度/重复字段及初始化前严格拒绝、成功令牌回归、Debug 隐私、精确响应预算、HEAD 完整预算与正文抑制、原期限/最后字节副本、初始化锁释放和未映射头预算。固定 [Go 鉴权参考](../tests/compat/guest_authorization_reference.go)新增 R 操作，保留原 WithAuthorization 与 jsonError，在受控已初始化令牌存储上执行 536 组真实回环 HTTP：四种方法 × 缺失/空/错值/超长/错误首字段重复项及所有 0x80–0xff 字节，比较状态、五个响应头和完整正文，无正文归一化。该对照不含 MMDS、Basic 或文件；原 Go 的有效首字段重复项/未初始化放行不被冒充为 Rust 等价行为。三个可移植测试和该对照已本地通过，显式入口为 `SANDPROOF_GUEST_AUTH_REFERENCE=/absolute/path/to/auth-reference cargo test -p sandproof-guest-service --locked --lib file_rpc_requests::tests::token -- --include-ignored --nocapture`。Linux [入口组合](../sandbox-guest-service/src/file_rpc_http/tests/token.rs)检查令牌先于账户/方法/类型/超时/正文、不查询账户/预占文件槽位、编码失败原期限与合法并发请求继续；原同步/异步 SDK 各新增三次实际缺失/空/错令牌调用，携带缺失 Basic 账户和非法超时，核对 UNAUTHENTICATED/完整消息及六次拒绝、无 token-refused 文件。Linux/原 SDK 实际文件组合已在 [#425 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38527056907) 通过（源 6541b8e6135dd0e9deb6e441fc4d007eda019115，实际合并 checkout 3eec8ec39304ee18751fcb5570224ebf557869c5）：两项 Linux 令牌组件、原 SDK 72 场景及新增六次令牌拒绝、既有 10/4/8 次 gzip/编码/JSON 拒绝、service lib 310 passed / 50 ignored 和封存 Redis 均通过，四项检查成功。普通 CI 不执行显式 Go 对照，不代替完整 E03。

[有界监听检查](../sandbox-guest-service/src/listener/tests.rs)含十一项可移植测试：真实回环 TCP 检查容量满额不派发、完成后恢复、头字段/缓冲/非法长度拒绝、处理器失败和响应截断、单连接 panic 后继续服务、外层取消关闭监听与传输；独立工作持有容量直到显式退出。内存双工流、通道屏障和可推进时钟检查慢头部、正文/处理/响应/keep-alive 共用原连接期限、慢消费者占满传输缓冲、停机排空和不接收后续请求。定向入口为 `cargo test -p sandproof-guest-service --locked --lib listener::`，这些组件已在 macOS 本地通过，Linux 交叉 Clippy 及固定 SDK 文件夹具迁移另行验证。固定 SDK 文件夹具现使用该监听组件，配置只属于回环测试；原调用集不变，结束时核对无拒绝/失败/到期、全部连接回收及峰值不超过 32。该监听组件和 SDK 迁移已在 [#426 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38529328112) 通过（源 c558044b94a07ab4768cab1b864f01cbc4c04747，实际合并 checkout f5471a53b07cc7178e3e7a3fb4ba2346877f59f7）：十项 Linux 监听测试、service lib 320 passed / 50 ignored、原 SDK 全部文件场景及封存 Redis 通过，监听报告 admitted/completed 各 4、peak_connections 3、refused/failed/expired 均 0，四项检查成功。没有进程 RSS/内核缓冲峰值、完整来宾 daemon 或 KVM 通过声明。

构造容量回归只调用 Listener::new：拒绝两组头解析记录合计超出 isize::MAX、乘法溢出及字节缓冲超过 isize::MAX；同时检查算术边界，不为这些极值创建连接、启动 Hyper 或分配数组。该回归在修复前明确失败、修复后通过；真实回环仍使用原小额度，不能把构造接受最大值当成内存可用或巨型请求实测。Linux 当前提交实跑与部署 RSS 验收仍待补齐。

[#552 提前拒绝收尾检查](../sandbox-guest-service/src/listener/tests/early_rejection.rs)通过原连接函数和内存双工流固定“收到拒绝响应及发送结束后才继续上传”的顺序，旧实现明确出现 BrokenPipe；补丁须让同一测试通过。另覆盖部分 Content-Length/chunked 正文、延迟结束标记、Expect 不发送 100 Continue、清理字节上限、清理期限及原连接截止点、完整请求不清理、后续字节不派发和收尾期间原连接名额不归还。定向入口仍为 `cargo test -p sandproof-guest-service --locked --lib listener::`；不靠随机等待重试。

[控制执行容量回归](../sandbox-guest-runtime/tests/preparation/control_capacity_http.rs)用屏障占满三条普通阻塞线程，保持第四条供控制请求使用；原七种拒绝的状态/完整字节、显式 Basic 账户预检和真实 PID 的 CloseStdin 在原一秒观察窗口内完成，普通请求仍在等待。准备出的 Ready 换到另一任务执行仍须使用原控制执行器，收尾核对单次命令效果与请求名额恢复。任务计数另覆盖分区执行器的排队取消、panic 和超时后实际退出，[装配回归](../sandbox-guest-service/src/daemon/assembly/tests.rs)占住普通阻塞线程后逐一提交原配置允许的全部账户查询，核对预留控制执行器不会提前拒绝第四个查询、超额仍拒绝及释放后可恢复；这些组件检查不代替正式 daemon 慢文件压力、同提交 KVM 或 E05 整体验收。

[固定 SDK 回归](../sandbox-guest-runtime/tests/sdk_registered/transport.rs)保留原错误码、完整消息、无 PID/gate/命令效果和容量归还断言，增加同步/异步发送首字节后暂停余下消息的时序。Linux `scripts/check-compat.sh` 先构建[回环辅助入口](../sandbox-host-service/examples/guest_rejection_gateway.rs)，再通过 `SANDPROOF_GUEST_GATEWAY_HELPER` 连接原 `data_gateway::handle`、请求/传输名额、真实 stdio relay 和原来宾 Listener/CommandHttp。辅助入口仅替换持久授权来源与命名空间放置，不复制网关业务；退出时核对一次授权/派发、原 relay 排空和全部请求名额恢复。它不代表中心 mTLS、真实 namespace/VM、完整 GuestHttp/daemon 或外部代理已验收。[生成器关闭回归](../tests/compat/test_sdk_rejection_body.py)固定消费者取走尾块后不再恢复迭代器的顺序，尾块在 yield 前记录为已提供；夹具等待同步/异步生成器实际关闭或耗尽后才读取事件，上传取消后仍等待控制通道的服务端 EOF，不要求拒绝后的客户端继续拉取。[事件循环收尾回归](../tests/compat/test_sdk_event_loop.py)固定“响应结果已交付，但原生线程仍停在跨线程唤醒”的顺序，夹具关闭循环时等待已进入的通知返回，避免解释器先退出；不替换原 SDK 传输、协议字节或错误解析器，也不表示所有 SDK 后台任务已结束。SDK 迭代器屏障及服务端响应体结束不等于网络数据已抵达对端；确定性失败机制、选定 SDK 时序及完整部署验收分开报告。


[一元头拒绝检查](../sandbox-guest-service/src/file_rpc_requests/tests/headers.rs)核对可选版本的第一字段、重复头、不修剪原值、版本与超时格式错误顺序、正文未读取、有效期限先收紧及拒绝后的原容量。实际初始化、响应字节副本和编码失败验证授权与名额分别释放；未映射错误不能补造 400。固定 Go 参考 H 操作使用原 Connect handler，229 组真实回环 HTTP 对照比较状态/头/正文，含 Unicode、非法 UTF-8、特殊转义、重复字段和大字段，已本地执行；不代表其他媒体类型或压缩协议。Linux 前置连接新增版本/超时拒绝的显式/缺省查询次数、无目录副作用和双槽位保留检查；SDK 同步/异步各新增一次超长 timeout 拒绝，显式缺失账户同时携带超长 timeout 仍先返回 401。新增头拒绝和 SDK 52 场景已在 [#409 的 Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38504921791) 通过（源 0cbcb4d42f3dd076df7edfacfdbd5acf318144b5，合并测试 970f72de981f9077f55c6aa0d56f45b627299af2）。

[Linux 一元账户回执连接](../sandbox-guest-service/src/file_io/rpc/tests/preflight/rejection.rs)检查显式/缺省来源、固定文本、无文件效果、实际授权释放、原双槽位到字节副本/编码失败/到期的保留，以及不可用源/重复 Basic 保持未映射。账户回执及头拒绝连接已分别在上述 #408/#409 CI 运行。文件传输 SDK 的 Range 检查读取原始 MIME 字段，保持精确 Content-Type 值及重复字段拒绝，避免 Python email.policy 对参数补引号造成假失败；仍逐项核对 range、两个 part 内容及总长度，并保留该夹具的五项回归。

有限响应收尾回归 `known_length_rpc_reply_releases_on_last_bytes_without_an_extra_eof_poll` 检查 identity/gzip 及单块/多块载荷：完整收到 Content-Length 指定字节后保留正文对象，不再轮询 EOF，最后字节副本释放才归还原名额。Linux `final_rpc_bytes_release_both_pools_without_an_http_eof_poll` 另覆盖实际 mkdir 成功及 stat 失败、请求/文件双名额和随后的正常请求；本地可移植回归先复现旧实现名额不归还，修复后通过，Linux 新组合尚待该汇总候选实跑。上传原编码取消/实际退出、最后字节及真实解码收尾回归保留；这些用例不更改容量或期限，也不算 KVM/完整来宾服务验收。

[文件属性 protobuf 对照](../sandbox-guest-service/src/file_rpc_response/tests/protobuf.rs)使用上游固定生成的 [filesystem.pb.go](../tests/compat/connect-reference/filesystem/filesystem.pb.go)，Git blob 7724e46497b97b24e4238068360f7244b93ee2cb，与 runtime f455fd58fa3f289e78ce1de690c35968d8d29322 的 packages/envd/internal/services/spec/filesystem/filesystem.pb.go 字节相同，参考程序启动时另检查嵌入源码 SHA-256 94e27e6aa36a685ea38c1ab636f2823ade538da494b476cd241f445866887b7f，[原许可证](../tests/compat/connect-reference/filesystem/E2B-LICENSE)随源码保留。参考 P 操作直接构造原消息并经默认 Connect HTTP codec 编码，不使用透传 JSON codec；48 组向量覆盖空/单/多属性、零值、枚举、int64、mode、optional 空值/epoch、时间戳边界、空元数据值和控制/Unicode 字符。比较状态、选定头和完整紧凑 JSON，双方先独立核对实际 Content-Length；原 protojson 可变空白被压紧，不宣称临近 2048 字节时头阈值相同。记录是合成属性，不运行 Go 文件方法、legacy 转换或整个 envd；现代固定 SDK 使用其正常 User-Agent。Rust 另有精确预算及存在性回归，原 SDK 18 场景夹具现在断言空目录正文为 {}、链接零 type/mode 缺省但空 symlinkTarget 存在；上述属性/空目录 Linux 组合已在 [#420 SDK CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38518473350) 通过（源 8b0b964db1e56521da8f6a1edfc9eed6342e28d2，实际合并 checkout 64b1de1eb60f58d9fd64e5b7f64b1fe8db4bbf50）：原 SDK 18 个合成属性场景、文件组合、service lib 298 passed / 45 ignored 及封存 Redis 通过；该运行两项流程检查曾在 checkout 被取消，随后仅重试未通过项，四项检查现均成功；重试没有改变源提交。原文件方法参考改用默认 MarshalOptions，并移除空 metadata 的额外归一化；Linux 原文件方法对照本候选未执行，不能沿用先前结果。显式入口为 `SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference cargo test -p sandproof-guest-service --locked --lib original_generated_properties -- --include-ignored --nocapture`。

[文件请求 protobuf 对照](../sandbox-guest-service/src/file_rpc_requests/tests/protobuf.rs)的 J 操作经同一固定生成消息与默认 Connect HTTP codec 解码五种原请求，测试 handler 仅回显解码结果，不查账户或执行文件操作。6,280 组本地向量比较解码字段及 invalid_argument 分类，覆盖缺省/null、重复已知/未知字段、转义字段名、所有单个原始字节、代理项、嵌套边界、超大未知数字、精确小数/指数及 uint32 边界；该 J 操作只比较解码值/分类，错误全文由下述 K 操作另行比较。裸指数矩阵另覆盖正/负零、非法前缀、小数/范围边界、e/E/符号、分隔符与空白、已知字符串/数值字段及未知嵌套值；先前四个 `{"depth":1e}` 与未知值 `1e`/`1e+`/`1e-` 差异已修复，并保留逐项相等断言。按需兼容副本的长度/容量不超过原正文，正常数字直接借用，长数字段一次推进；转换不改变原始字节预算、字符串、重复字段与结构拒绝。可移植回归另检查原消息预算先行、扫描深度、精确整数与同一准入名额/期限；不存在浮点舍入或先构造整棵 Value 树。显式执行 `SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference cargo test -p sandproof-guest-service --locked --lib file_rpc_requests::tests::protobuf -- --include-ignored --nocapture`；普通 CI 不自动执行 Go 对照。这些组件结果不能代替 Linux 实际文件、原 SDK 双端 E03、正式监听器或 KVM。

前序请求 JSON 与裸指数实现已分别在 [#421 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38519983349)（源 7ccc7f109a40571172993fb2bf561b29138230a6，合并 checkout a5e2487a0aec1ce5582db48de2e358a89a5225d0）和 [#422 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38521089460)（源 d61d0896222e707d3ee44b1c26c7aeaab0aded01，合并 checkout ad0d66105704b8225005e82620347db5448c3a2f）通过四项检查；两者 service lib 均为 299 passed / 47 ignored，原 SDK 72 场景、十次坏 gzip、四次未知编码及封存 Redis 通过。原 Go 显式对照未在这些 CI 执行；这些父层结果不覆盖后续 JSON 错误映射。

[JSON 错误响应检查](../sandbox-guest-service/src/file_rpc_requests/tests/json_errors.rs)经已鉴权请求、原阻塞解码和实际有界发送器执行 7,303 组固定 Go K 操作对照，比较完整错误消息、选定 HTTP 头及 JSON 字节。覆盖五种方法、字段/结构/重复项优先级、Unicode 行列、每个转义位置的全部原始字节、代理项、裸指数与深度边界；每方独立核对实际 Content-Length。只将原 protobuf 按二进制摘要选出的 proto: 后 NBSP 转成普通空格，保留其他原始 JSON 转义（包括无效 UTF-8 的替代表示）；临近 2048 字节的头阈值仍取各自实际编码长度，不承诺两种前缀构建具有相同阈值。空载荷不增加 protobuf 类型前缀。可移植回归核对 Debug 隐私、精确响应预算、最后副本/错误对象占额和可推进时钟下的过期拒绝；未知方法/其他分类继续未映射。显式入口为 `SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference cargo test -p sandproof-guest-service --locked --lib file_rpc_requests::tests::json_errors -- --include-ignored --nocapture`。

Linux [JSON 入口组合](../sandbox-guest-service/src/file_rpc_http/tests/json_errors.rs)检查显式/缺省账户拒绝顺序、无文件效果、错误超限保留双槽位和恢复；固定 SDK 夹具另执行同步/异步各四次合成非法 JSON HTTP 输入，由原 ConnectWireError 解析器核对完整 code/message，Rust 断言八次拒绝及未创建 json-refused。四种合成错误的原 SDK 解析探针已本地通过；Linux 实际组合已在 [#423 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38524031451) 通过（源 92977a220fe068bf097c5da80cf39240a1d9ad83，实际合并 checkout 947d0ed67a7408296df575deb1f29afd28bfe748）：八次 JSON 拒绝、原 SDK 72 场景、十次坏 gzip、四次未知编码、service lib 302 passed / 48 ignored 及封存 Redis 通过，四项检查均成功。合成坏输入与原 SDK 自身编码的场景分开报告，不作为完整 E03、正式监听器或 KVM 验收；普通 CI 未执行上文显式 Go 对照。

[Connect 传输参考](../tests/compat/connect-reference/main.go)以固定 Go 1.26.8、原 Connect v1.18.1、authn v0.1.0 和 protobuf v1.36.12 在本机真实 HTTP 上返回同一个已编码载荷，比较八组状态/头/正文，包括 2047/2048/2049 字节边界。参考使用 passthrough 测试 codec，隔离原 Connect 传输；成功传输对照不执行 protobuf 编解码、认证、文件操作或完整 envd，不能视为 E03 双端通过。该显式 ignored 对照已本地执行，普通 CI 不自动安装 Go 依赖。新增 X 操作使用 [固定路径函数](../tests/compat/connect-reference/path.go)的实际展开错误交给原 Connect handler，透传 codec 不校验 protobuf，也不访问真实账户/文件；显式执行 `SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference cargo test -p sandproof-guest-service --locked --lib fixed_go_rpc_expansion_rejections_match_http -- --ignored --nocapture`，已有本地结果。

```sh
(cd tests/compat/connect-reference && GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build \
  -mod=readonly -trimpath -buildvcs=false -o /absolute/path/to/connect-reference .)
SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference \
  cargo test -p sandproof-guest-service --locked --lib file_rpc_response -- --include-ignored --nocapture
SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference \
  cargo test -p sandproof-guest-service --locked --lib file_rpc_requests -- --include-ignored --nocapture
SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference \
  cargo test -p sandproof-guest-service --locked --lib basic_auth -- --include-ignored --nocapture
cargo test -p sandproof-guest-runtime --locked --test protocol --test file_protocol
```

新增[流式文件端口检查](../sandbox-guest-runtime/tests/file_transfer.rs)覆盖二进制/空文件、父目录/链接、长度和元数据预校验、截断/超额后保留前缀、元数据替换/清空及打开后路径替换、文件增长和缩短。[传输所有权检查](../sandbox-guest-service/src/file_transfer/tests.rs)覆盖原授权阻止初始化、上传结果/下载块及传输引用占额、消费前不预读、取消后不可重用、显式 EOF 与工作者丢失、空闲期限和过期就绪回复。可推进时钟和明确块释放建立顺序；未模拟内核永久阻塞、真实磁盘满或整机峰值。

[原 SDK 文件传输组合](../sandbox-guest-runtime/tests/sdk_file_transfer.rs)与[同步/异步调用集](../tests/compat/guest_file_transfer_sdk.py)经一次性回环路由连接原授权、真实账号/路径解析和传输池，检查原始字节上传、普通/流式下载、大于块预算的内容、元数据替换、目录/缺失错误及先前成功写入不回滚；新增单文件/多文件 multipart、字节/二进制流/文本流、Unicode 完整文件名、批次元数据、重复路径/目录失败后的前缀及后项不处理。Linux `check-compat.sh` 已登记以上入口，已有原提交的 Linux 组件检查；可定向执行 `cargo test -p sandproof-guest-runtime --locked --test file_transfer`、`cargo test -p sandproof-guest-service --locked file_transfer::tests` 和 `cargo test -p sandproof-guest-runtime --features sdk-contract-tests --locked --test sdk_file_transfer -- --nocapture`。后续改动须重跑，交叉类型检查不能算运行通过。测试适配现支持 raw/multipart POST 和 identity GET；multipart 已有原提交 Linux 组件证据，后续候选仍须重跑；不代表正式路由、下述新增 gzip 组合、range/conditional、固定 Go HTTP 双端对照、E03 或 KVM 验收。

新增[批量上传检查](../sandbox-guest-service/src/file_transfer/batch/tests.rs)覆盖单槽位顺序多文件、一次账户查询、元数据/结果保留、相对路径判重、查询空路径覆盖、合法空批次、取消中间文件、总量/数量/结果预算、后续文件失败与整批绝对期限。结果预算拒绝可发生在实际写完之后，测试要求文件保留而整批失败。Linux 基础及兼容入口中的 service 全测试会执行该组，也可定向 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests`；该组已有原提交的 Linux CI 运行验证，后续改动须重跑。此处直接向组件交入已解码文件名和正文块，没有执行 multipart MIME 解析、网络请求或固定 Go HTTP 对照。

[multipart 解码检查](../sandbox-guest-runtime/src/multipart/tests.rs)可在 macOS/Linux 运行，覆盖逐块和全部切点、近似边界二进制、CRLF/LF、前导/尾随内容、空 part、逐字节截断、前一 part 结束后边界行失败、折叠/重复头、完整文件名、RFC 2231 跨续段 UTF-8、quoted-printable 的部分错误/4096 字节临界值，以及全部输入预算、错误后不可复用和大流中固定解析缓冲容量。执行 `cargo test -p sandproof-guest-runtime --locked --lib multipart`；普通 Linux 兼容入口已有 runtime lib 测试，因此会执行可移植组。该解析组件已有本地实跑，不代表下述上传组合或 Linux 全服务验收。

[multipart 上传组件检查](../sandbox-guest-service/src/file_transfer/batch/tests/multipart.rs)接原批次和真实临时文件，覆盖跨帧/跨写块、QP 二进制/前缀错误、未知字段、查询空串覆盖/缺少文件名、空表单、重复路径、后续边界失败、声明长度/实际正文/空帧/尾随内容/trailers 预算及异常顺序。明确通道信号确认已有文件写完且正在等待下一 HTTP 帧，再测试实际 EOF 前的授权/容量保留、结束标记后的传输错误、中途取消和可推进的原期限。服务全测试与 Linux 兼容入口会执行该组，定向入口为 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::multipart`。该组和扩展固定 SDK multipart 已有原提交的 Linux 组件证据，后续候选仍须重跑；完整基础检查不能由组件绿灯替代，没有磁盘满/故障文件系统、正式监听器或 KVM 的通过声明。

[gzip 检查](../sandbox-guest-runtime/src/gzip/tests.rs)在 macOS/Linux 可运行：`cargo test -p sandproof-guest-runtime --locked --lib gzip`。覆盖空/二进制/多段、所有截断点、CRC/长度损坏、逐项预算、精确上限后的真实 EOF/源错误、错误后不可复用、非法 DEFLATE 回溯距离及大流的小缓冲编码/解码。首个头部失败与正文错误分开核对，后者检查实际可见前缀。本地组件及下述固定 Go 对照已运行；Linux 候选、HTTP gzip/文件副作用与 SDK 压缩组合仍待验证。

[Go gzip 适配器](../tests/compat/guest_gzip_reference.go)仅调用固定 Go 1.26.8 `compress/gzip`。显式组比较所有 flag、额外头、名称长度、截断/校验/尾随错误和种子 0x311 变异的初始化结果、字节前缀和终态，并双向检查 Go/Rust 压缩结果；不要求压缩字节相同。默认 ignored，不把普通 CI 绿灯算作执行过 Go 对照：

```sh
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/gzip-reference tests/compat/guest_gzip_reference.go
SANDPROOF_GUEST_GZIP_REFERENCE=/absolute/path/to/gzip-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  gzip::tests::fixed_go_gzip_reference -- --ignored --nocapture
```

[HTTP gzip 所有权检查](../sandbox-guest-service/src/upload_body/tests.rs)可在 macOS/Linux 执行：`cargo test -p sandproof-guest-service --locked --lib upload_body`。原库代码直接接受受控 HTTP Body，测试首个头部之后按需读取、`Bytes` 副本背压、头部/正文等待取消、同步源轮询阻塞期间保留守卫、原绝对期限、真实 HTTP EOF/尾部错误、编码/展开/帧预算与错误穿过 Body 包装。同步通道和明确轮询信号控制交错；可推进时钟只在等待点确认后使用。该组已有本地运行，只验证传输/守卫连接，不替代 Linux AuthPermit/文件实际副作用。

[gzip 文件组合](../sandbox-guest-service/src/file_transfer/batch/tests/gzip.rs)覆盖一批多段 raw/压缩 multipart、无效头部不打开目标、文件内容前缀与旧 metadata、MIME 结束后 CRC 错误、后续 gzip 段失败、真实 EOF 前授权/容量保留，以及取消/期限/展开预算。入口为 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::gzip`，Linux 基础与兼容入口包含该组；本候选实际运行待补。固定 SDK 文件测试路由另加合成 gzip POST、同步/异步原 SDK 回读、首头错误 400 及校验失败后的数据保留。固定 SDK 支持 gzip=True 上传；此组的压缩 POST 由测试 HTTP 客户端构造，原 SDK 压缩上传在下述错误响应驱动另行检查，不能将合成输入记为原 SDK 上传或完整 E03；扩展组合仍待 Linux 实跑。

[上传头检查](../sandbox-guest-service/src/file_upload/tests.rs)及[共享验证](../sandbox-guest-runtime/src/file_upload/tests.rs)覆盖首字段、忽略空 metadata 后缀、无效/超额值与整个 HeaderMap 的独立预算，已有本地运行。定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_upload`。显式 [Go 上传头适配器](../tests/compat/guest_upload_reference.go)调用固定 Go 1.26.8 ParseMediaType，复制固定 runtime 的 metadata 提取/验证原函数；比较媒体类型选择及 metadata 接受值，覆盖不透明字节、Unicode 大小写/空白、无效可选参数与冲突重复参数。不比较 Go map 随机迭代下的首条错误文本，不代表完整 Go HTTP 上传端点；默认 ignored，已有本地显式运行：

```sh
GOTOOLCHAIN=local /absolute/path/to/go1.26.8/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/upload-reference tests/compat/guest_upload_reference.go
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  file_upload::tests::fixed_go_upload_selection_and_metadata_match_original_methods -- --ignored --nocapture
```

[上传故障检查](../sandbox-guest-service/src/file_upload/tests.rs)使用同一 Go 上传参考的 F 操作，复制固定 runtime 的 processFile 原函数，仅替换日志类型与 I/O 调用名；受控返回不同阶段的 EIO/EACCES/ENOSPC/EDQUOT，对照阶段状态，不自行重写预期表。macOS 参考按 Linux errno 身份适配宿主常量；它不执行文件系统操作，也不验证完整错误字符串。显式入口为：

```sh
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  file_upload::tests::upload_fault_status_matches_original_process_file -- --ignored --nocapture
```

[详细上传 FD 检查](../sandbox-guest-runtime/tests/file_transfer.rs)覆盖目标目录、父路径是文件、目标/父链接循环、悬空链接、NUL 路径与遍历预算前置拒绝；[内部写入检查](../sandbox-guest-runtime/src/filesystem/transfer/upload/tests.rs)覆盖真实临时文件短写前缀后注入 ENOSPC、Interrupted 重试、失败后不可重放、零写、只读 FD 的实际 EBADF 和 O_PATH 的 xattr 列表失败。所有真实资源仅位于各用例私有目录，不填满宿主磁盘或改动设备文件。这两组已交叉检查，Linux 执行待补；受控 ENOSPC 不计为真实磁盘满验收。Linux runtime 全测试包含它们，定向入口分别为 `cargo test -p sandproof-guest-runtime --locked --test file_transfer` 和 `cargo test -p sandproof-guest-runtime --locked --lib filesystem::transfer::upload::tests`。

[上传错误回执对照](../sandbox-guest-service/src/upload_response/errors/tests.rs)通过同一 Go 参考的 J 操作，调用原 processFile、EnsureDirs、os.PathError 和 pkg/xattr.Error，再由真实回环 HTTP 执行原 jsonError；只替换实际 I/O 观察，不启动完整 envd。Linux errno 文本固定，核对全部阶段、普通/空间/配额及未知 errno、原请求未知/零/正/最大有符号长度、控制/HTML/Unicode/无效 UTF-8、xattr 单字节与 2047/2048/2049 字节边界；可移植预算、未分类拒绝及原容量/期限检查一起运行，已有本地证据。显式入口为：

```sh
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  upload_response::errors::tests -- --include-ignored --nocapture
```

[Linux 错误响应连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_errors.rs)验证 raw identity/gzip 的真实父路径/目标目录错误穿过 worker、先前 multipart 文件保留且之后文件未打开、Bytes 克隆持有原槽位、实际 I/O 完成后初始化可继续、错误编码预算失败保留文件并归还容量。定向入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_errors`；已交叉检查，实际运行待补。原固定 SDK 驱动另精确检查同步/异步 raw/multipart 目录异常文本，并新增原 SDK gzip=True 的成功写入/回读及目录拒绝，和合成 gzip 场景分别计数；不模拟磁盘满，该 Linux 组合也待实跑。

[上传头拒绝对照](../sandbox-guest-service/src/upload_response/rejection/tests.rs)使用 Go 参考 P 操作：原 encoding.go 选择、metadata 提取/验证，以及 PostFiles/handleRawUpload 的相应拒绝表达式，再经原 jsonError 的真实回环 HTTP 比较 400、头和正文。覆盖首字段/重复值、无效 UTF-8/HTML/Unicode、键长/单字节值、原字节偏移、单条失败先于整体大小、完整总大小及 2048 字节响应边界；不将摘取分支当作完整端点执行。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::rejection -- --include-ignored --nocapture`，预算与响应副本所有权检查同时运行，已有本地证据。

[Linux 上传头拒绝连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_headers.rs)验证四类错误、首值/拒绝顺序、正文零轮询、无文件副作用、账户调用次数、原槽位/期限、初始化与失败收尾；入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_headers`，已交叉检查，实际执行待补。SDK 驱动另用同步/异步测试 HTTP 客户端构造异常头，交给锁定 SDK 原文件错误解析器检查异常类型/文本；普通 SDK 会在客户端校验 metadata 且不发送异常媒体类型，所以此项明确记作合成输入与 SDK 解析组合，不记原 SDK write 发出的请求，Linux 待实跑。

[gzip 首头/响应对照](../sandbox-guest-service/src/upload_response/gzip_header/tests.rs)使用上传 Go 参考 G 操作：固定 getDecompressedBody 调用 Go 1.26.8 gzip.NewReader，成功只报告首头准备完成，失败走 PostFiles 的原包装表达式和 jsonError 真实回环 HTTP。覆盖全部 flag、首头截断、魔数/方法的单字节、名称/注释 512 字节边界、extra 最大长度及 FHCRC，按多种 HTTP/解码分块比较准备结果或完整错误状态/头/JSON；明确不读取 DEFLATE/正文/trailer。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::gzip_header -- --include-ignored --nocapture`，已有本地运行；空输入与长度/源/预算/期限拒绝、首读 Interrupted 重试另有可移植检查。

[Linux gzip 首头请求连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_gzip_header.rs)核对 raw/multipart 首头拒绝先于账户/metadata/文件，原容量/期限/初始化和最后副本释放，以及传输/预算不被伪装成 EOF；入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_gzip_header`，已交叉检查，实际运行待补。SDK 驱动另加合成空/截断/损坏 gzip raw/multipart 请求，经原错误解析器核对异常文本并由 Rust 检查目标未创建；不记作原 SDK 压缩器产生的输入，该 Linux 组合也待实跑。

[multipart 路径拒绝对照](../sandbox-guest-service/src/upload_response/rejection/tests.rs)使用上传 Go 参考 B 操作：复制固定原 resolvePath 与 CustomPart 方法，仅替换命名空间，路径展开注入已解析观察，不调用真实账户/文件系统；缺失文件名经原 FormName 闸门和原 MIME 参数解析。比较完整 400/HTTP 头/JSON，覆盖完成列表中各重复位置、其他路径数量 0–4 与原 >1 列表阈值、完成顺序、Unicode/转义和 2048 字节头边界。沿用上述 upload_response::rejection 显式 Go 入口，已有本地运行，不代表完整原路由或 Linux 路径展开。

[Linux multipart 路径拒绝连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_multipart.rs)覆盖 identity/gzip 的词法重复、先前文件保留/后项不处理、缺失文件名与查询空串覆盖、详细旧入口迁移、原槽位/最后字节副本/期限/初始化、响应预算失败和文件数量拒绝优先级；入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_multipart`，已交叉检查，实际运行待补。固定同步/异步 SDK 驱动增加原 write_files 重复路径的精确异常与已完成顺序检查；另用合成 identity/gzip 无 filename 表单经原 SDK 错误解析器验证，不能记作 SDK write 发出的输入。Linux SDK 组合仍待实跑。

[multipart 准备对照](../sandbox-guest-service/src/upload_response/rejection/tests/setup.rs)使用上传 Go 参考 T 操作，直接调用 Go 1.26.8 Request.MultipartReader/NextPart：受控 reader 记录首次正文读取，进入该 reader 仅报告准备就绪；零读取错误按固定 handleMultipartUpload 原包装和状态经 jsonError 输出。比较各类型/参数、引号/续段/扩展 boundary、ASCII 单字节与 Unicode、重复 Content-Type 首值的就绪或完整 500/头/JSON；不将注入的 reader 错误作为真实服务错误。沿用 upload_response::rejection 显式入口，已有本地运行；独立检查旧 boundary 粗分类、资源预算和非 UTF-8 参数拒绝。

[Linux multipart 准备连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_multipart_setup.rs)核对 identity 正文零轮询、gzip 首头后的拒绝、旧文件不变、首头字段、账户调用、metadata/类型/预算优先级、正文截断不映射为准备失败，以及原槽位/期限/初始化/编码失败/旧入口迁移。定向入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_multipart_setup`，已交叉检查，实际运行待补。SDK 驱动用合成 identity/gzip 三种准备失败经原错误解析器核对 500 和 SandboxException 的完整文本，并由 Rust 夹具检查文件不存在；不记为原 SDK write 产生，Linux 组合待实跑。

[raw gzip 正文对照](../sandbox-guest-service/src/upload_response/gzip_read/tests.rs)使用上传 Go 参考 D 操作：原 getDecompressedBody 创建固定 Go gzip reader，原 processFile 的 ReadFrom 调用由内存目标实际消费 reader，并保留错误前前缀；文件系统调用仍为受控观察，未执行真实写盘/metadata。完整成功只比较 END 和前缀，失败比较前缀及原 500/头/JSON。覆盖首头就绪后的逐字节截断、全部 trailer 字节逐 bit 损坏、后续成员头/正文、空/二进制/多段，使用多种输入/解码分块；未知 DEFLATE 细节不纳入公开诊断。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::gzip_read -- --include-ignored --nocapture`，已有本地运行；转换检查拒绝首头空输入和源/预算/期限等无对应错误。

[Linux raw gzip 错误连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_gzip_read.rs)覆盖三类 500、已写前缀及旧 metadata、multipart 完成文件的独立错误、identity 输入不套用 raw gzip 包装、长度/展开预算保持原分类、旧入口错误不变、原槽位/期限/初始化/字节副本及响应编码失败。入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_gzip_read`，已交叉检查，实际执行待补。固定同步/异步 SDK 驱动的合成压缩请求新增截断和后续头错误，与原校验损坏一起核对原 SDK SandboxException 的精确文本，再由原 SDK 回读前缀；这是测试客户端构造的错误输入，不能记为 SDK 压缩器发送或 Linux 已执行。

[multipart framing 对照](../sandbox-guest-service/src/upload_response/multipart_read/tests.rs)使用上传 Go 参考 N 操作，运行原 handleMultipartUpload、handlePart、resolvePath、processFile；真实 Go MIME reader 读取正文，ReadFrom 的内存目标记录文件前缀，原 handlePart 到达 metadata 回读时标记完成。仅替换日志/文件系统和既有路径观察，不宣称真实写盘/metadata。比较 EOF 阶段、两类坏边界行、CRLF/LF、忽略字段、后继 part 和已完成文件的观察及完整失败 HTTP；Go Q 操作另核对所有 Unicode 标量值和全部字节的 Quote 转义。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::multipart_read -- --include-ignored --nocapture`，已有本地运行；普通组件检查另核对诊断行预算、失败后不可复用、粗入口兼容及转义后精确响应预算。

[Linux framing 连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_framing.rs)覆盖 identity/gzip 中的原始/QP 文件截断、前一个完成文件/旧 metadata、忽略字段 EOF、两类坏边界行后不打开后继文件，以及头/QP/源/预算的独立分类、旧入口兼容、原槽位/期限/初始化/最后副本和响应编码失败。入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_framing`，已交叉检查，实际运行待补。固定同步/异步 SDK 驱动增加合成 identity/gzip MIME 错误，核对原 SDK SandboxException 文本并由 SDK 回读前缀；输入由测试客户端构造，Linux HTTP/文件组合待补，不记为原 SDK write 发送。

[MIME 头诊断对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/headers.rs)沿用上传 Go 参考 N 操作，比较首行缩进/80 字节边界、缺少冒号的原物理行、非法键/值与折叠后的逻辑行、键大小写/空格、控制和非 UTF-8 字节、无终止空行及真实 EOF、较大响应和前一个完成文件；逐项比较文件观察及原 500/头/JSON。入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::multipart_read::tests::headers -- --include-ignored --nocapture`，已有本地运行。参考 V 操作另在原 MIME reader 的底层注入实际 Reader 失败，核对坏头验证与失败包装顺序，不代称真实 HTTP 断网。可移植 runtime 检查另核对未读 EOF 的提前拒绝、逻辑行未完成时等待、真实源失败参与已有头验证且不合成 EOF、失败后不可复用、头字节/字段数预算及合法头截断的独立分类。

[Linux MIME 头连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_mime_headers.rs)核对 identity/gzip 原消息、前一个完成文件与 metadata、当前文件未打开、明确物理行错误后不再轮询输入、真实源失败时已有坏头诊断优先、本地长度/预算/不支持文本/合法头 EOF 的独立分类、旧入口迁移及原槽位/期限/初始化/最后副本/编码失败。入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_mime_headers`，已交叉检查，实跑待补。固定同步/异步 SDK 驱动新增合成坏 MIME 头，经原错误解析器核对 SandboxException 的精确文本，并由 Rust 夹具核对目标不存在；不记为原 SDK write 生成，Linux 组合待实跑。

[QP 诊断对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/quoted_printable.rs)沿用原上传参考 N/V，比较裸控制字节、短/非法转义、可接受的字面等号、软换行、4096 字节临界值、已完成前一个文件和实际源失败，逐项核对解码前缀、完成标记与原 HTTP。原满行缓冲对齐差异已改为一致回归；新增 L/K 操作在原方法的底层 Reader 限制单次读取大小，独立于 Rust 内部事件分块，核对普通/软换行/带前一物理行的 4096 临界值，以及长 MIME 头/折叠缩进和 NextPart 满行失败。入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::multipart_read::tests::quoted_printable -- --include-ignored --nocapture`，已有本地运行。runtime 普通检查覆盖错误优先级、不可复用、旧粗入口和源/块预算分类，响应检查覆盖转义后精确预算。

[Linux QP 连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_quoted_printable.rs)核对 identity/gzip 的实际文件前缀/旧 metadata/前一个完成文件、坏行后零轮询、真实源失败、本地长度/预算与文件写预算优先级、旧入口及原响应槽位/期限/初始化/最后副本/编码失败。已交叉检查，执行待补。固定同步/异步 SDK 驱动增加合成 QP 失败请求、原 SandboxException 精确文本和 SDK 文件前缀回读；输入并非原 SDK write 生成，本地只验证固定 Go/SDK 错误解析，Linux HTTP/文件/SDK 组合待实跑。

[源读缓冲组件检查](../sandbox-guest-runtime/src/multipart/read_buffer.rs)核对分段提交一次实际读取的调用顺序、原输入预算、物理头行末 CR、数据最后片段才携带 EOF、满行不再读取，以及尾随输入处理；原大流用例同时核对两种解析入口的固定缓冲容量。QP 普通测试另区分同次返回字节/EOF、字节/错误和之后才到来的终态。[Linux 读取边界连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_read_boundaries.rs)按明确源读取大小和事件块配置比较满行完成/失败、文件前缀与 metadata，使用 panic-tail 确认 buffer full 不多读。已交叉检查，实际执行待补；它不替代真实 HTTP/gzip 分块与完整双端 SDK 验收。

[multipart gzip 阶段对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/gzip.rs)新增上传 Go 参考 Z 操作，原 getDecompressedBody 的真实 gzip reader 接原 handleMultipartUpload/handlePart/processFile。Rust 使用实际 GzipBody 与 MIME/QP reader；覆盖 MIME 各阶段切点、三类 gzip 失败、前一个完成文件、忽略字段、坏头/QP 诊断优先级、歧义及近似边界尾部，比较内存文件前缀/完成标记及失败 HTTP。原方法遇 MIME End 即结束，故对照不代称 Rust 后续严格排空与 Go 等价。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib gzip_source_stage -- --include-ignored --nocapture`，已有本地运行。普通 runtime 测试另核对 fail_source 的顺序、不可复用及 EOF/失败互斥，响应测试核对精确预算和不接受 Complete 阶段伪造响应。

[Linux multipart gzip 连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_multipart_gzip.rs)检查三个错误阶段、已完成前项/当前文件前缀和 metadata、QP/头/忽略字段优先级、原编码门控、直接批次/旧入口迁移、长度/展开预算和原响应槽位/期限/字节副本。已交叉检查，执行待补。固定同步/异步 SDK 驱动增加合成压缩错误，使用原异常解析器和 SDK 回读核对文本/前缀；Rust 夹具的 QP 文件检查路径按驱动的实际创建目录修正。这些输入不来自原 SDK 压缩器，Linux 组合仍待实跑，不能计作完整 E03。

[忽略 QP 字段对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/ignored.rs)核对 Close 遇编码错误后保留的原始预读位置、后续文件打开/完成及原边界诊断。固定 Go L 操作独立限制源读取，Rust 内部事件分片另设；包含短/非法转义、软换行、裸控制字节、满行及后续内容，并保留前一个完成文件。gzip 组使用原上传参考 Y：Rust 记录实际解压块序列，Go 在原 gzip reader 外以 ReadFull 按序提供同样字节，末尾零项必须读到真实 gzip 错误，不注入诊断；两端必须消费同一来源序列。仅设相同块上限不足以建立该事实。入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib ignored_qp -- --include-ignored --nocapture`，已有本地运行。

[Discard 普通组件检查](../sandbox-guest-runtime/src/multipart/discard.rs)覆盖有界预读而非全字段排空、错误/顺序/来源/预算、不可复用和 4 MiB 忽略 QP 流中的固定解析容量。[Linux 忽略字段连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_ignored.rs)核对 identity/gzip 固定源读取及内部块下的后继文件/旧 metadata、先前文件保留、原错误与响应预算、旧入口和等待中的授权/取消/原期限。已交叉检查，执行待补。SDK 驱动另加入对网络分块稳定的合成成功/满行失败，核对原异常解析和后继文件回读/未创建；Linux HTTP/文件/SDK 组合未实跑，不记作原 SDK write 生成或完整 E03。

[上传顺序检查](../sandbox-guest-service/src/file_transfer/batch/tests/upload.rs)连接原授权、实际账户池与临时文件，覆盖编码/首头/用户名/metadata/媒体类型/path 的拒绝优先级，raw/multipart × identity/gzip 四种组合的一次准入/一次账户查询、首 metadata 值和 raw 短正文前缀。条件变量屏障与可推进时钟核对账户查询取消/超时后原授权和上传槽位仍保留，实际查询退出后才允许初始化及容量复用。定向入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload`；Linux service 全测试包含该组，已交叉检查，实际运行待补。固定 SDK 文件夹具所有 POST 已改接同一组件，原 raw/multipart 与合成 gzip 回读组合须在此候选重跑；无正式监听器、签名/完整上传错误 JSON、特权身份切换或 KVM 通过声明。

[上传回执检查](../sandbox-guest-service/src/upload_response/tests.rs)纳入可移植 service lib，覆盖空批次、空 metadata 省略、字段/键顺序、Go 转义、精确响应线长、无效预算、原期限、逐块背压和响应丢弃后最后 Bytes 副本占额，已有本地运行。原上传 Go 参考适配器新增生成类型的成功数组与固定 PostFiles 的写头/正文调用，通过真实本机 HTTP/1.1 服务核对状态、Content-Type、Content-Length、nosniff 缺省和完整正文；含 2047/2048/2049 字节边界、控制/HTML/Unicode 字符及多文件。参考仅发送合成内存响应，不访问实际文件或启动完整 envd；运行后关闭自己的回环服务。显式入口为：

```sh
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  upload_response::tests::fixed_go_upload_success_bytes_and_actual_http_headers_match -- --ignored --nocapture
```

[Linux 回执连接检查](../sandbox-guest-service/src/file_transfer/batch/tests/response.rs)验证已完成文件/metadata 在响应预算失败后保留，实际 BatchReply 与响应字节使用同一原槽位，文件 I/O 完成后初始化仍可推进，空批次响应后容量可复用。SDK 文件夹具已移除成功数组的独立 JSON 拼装；同步/异步合成 POST 检查空 metadata、Unicode/HTML 文件名和大 metadata 响应头/精确正文，再用原 SDK 回读实际文件。这些 Linux 组已交叉检查，实际运行待补；可移植响应对照不能替代它们或完整端点验收。

[gzip 编码所有权检查](../sandbox-guest-service/src/upload_body/encode/tests.rs)纳入上述可移植 service lib 入口，覆盖打开后不预读、空/二进制/大文件块、固定源缓冲、Bytes 克隆背压、打开和读取阻塞期间的取消/原期限、两种总量边界及源错误不能变 HTTP EOF。本地已运行；不模拟 Linux 文件权限或授权实现。

[Linux gzip 下载检查](../sandbox-guest-service/src/file_transfer/gzip_download/tests.rs)和原 [FD 检查](../sandbox-guest-runtime/tests/file_transfer.rs)增加增长/缩短、路径替换、链接、procfs 非 stat 长度读取、真实 AuthPermit/响应 Bytes 持有、容量/期限及原始/压缩预算。定向入口为 `cargo test -p sandproof-guest-service --locked file_transfer::gzip_download::tests`，基础和兼容入口会发现该组；当前候选待 Linux 实跑。固定 SDK 文件路由另以原 `files.read(..., gzip=True)` 检查同步/异步 bytes、stream、Unicode text、空/二进制/大文件与缺失/目录错误，合成 HTTP 客户端检查未自动解码的 gzip 正文、无 Content-Length 及 Vary；该扩展也待 Linux 实跑。路由已接下述 stat→编码选择→open 组件，完整正式入口与 E03 双端验收仍待补。

[内容编码测试](../sandbox-guest-runtime/src/file_encoding/tests.rs)和[HTTP 字段测试](../sandbox-guest-service/src/file_encoding.rs)可在 macOS/Linux 原库运行，覆盖 q/大小写/Unicode 空白/不透明字节、重复参数、identity/wildcard、原始字节/项预算、第一重复头、条件字段非空规则和 Vary 设置阶段。13 项同权重列表回归要求保留原 Go 选择，不能稳定排序后误选 gzip。定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_encoding`；普通 runtime/service 库检查会发现这两组。组件没有 stat/open/实际范围响应或 HTTP 错误顺序的执行证据。

[编码 Go 参考适配](../tests/compat/guest_encoding_reference.go)复制固定 runtime `f455fd58fa3f289e78ce1de690c35968d8d29322` 的原选择函数，仅改变 package/import 和输入输出驱动，固定 Go 1.26.8。显式组比较浮点 bits/错误、Content-Encoding、Accept-Encoding 和条件 identity 选择，包括种子 0x311 的十进制/十六进制/畸形数值与长列表；本地已运行，默认 ignored，普通 CI 不代为执行，也未启动完整 Go envd。原样函数及 Apache/BSD 许可证随源码保留：

```sh
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/encoding-reference tests/compat/guest_encoding_reference.go
SANDPROOF_GUEST_ENCODING_REFERENCE=/absolute/path/to/encoding-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  file_encoding::tests::fixed_go_encoding_reference -- --ignored --nocapture
```

[下载决策检查](../sandbox-guest-runtime/src/file_download/tests.rs)及[响应计划检查](../sandbox-guest-service/src/file_download/tests.rs)纳入可移植库测试，定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_download`。覆盖条件优先级、强弱标签、第一重复字段、旧 HTTP 日期、零时间、Range 解析顺序、空文件/后缀零长度、单段 HEAD seek 要求、多段 MIME 字节/长度、错误头保留及组件预算。已有本地运行；输出为读取计划，不等于真实文件读写、原授权保持、错误收尾或网络 HEAD 行为已验证。

[时区快照检查](../sandbox-guest-runtime/src/file_download/local_zone/tests.rs)覆盖 TZif v1/v2/v3 选中块、无 transition、四种首类型分支、同名缩写、最后类型保持、原始闰秒 transition、北/南半球 POSIX 延伸、原输入释放及非法/超额材料拒绝。配置零值/上界、两个头的计数和截断在解析前拒绝；没有从失败结果制造 UTC 快照。入口为 `cargo test -p sandproof-guest-runtime --locked --lib file_download::local_zone`。四项普通回归纳入基础检查；[显式 Go 对照](../sandbox-guest-runtime/src/file_download/local_zone/tests/reference.rs)共用下述下载参考二进制的新 L/Z 操作，直接调用固定 Go `LoadLocationFromTZData`、实际 location 查询和原 `http.ParseTime`，17 组显式材料比较 5,848 次查询与 17,136 次日期解析，已有本地通过。执行前重建参考二进制，设置 `SANDPROOF_GUEST_DOWNLOAD_REFERENCE` 并追加 `-- --include-ignored --nocapture`；普通 CI 不自动执行该 ignored 组。尚未覆盖系统时区文件读取、daemon 启动配置、完整 E03 或真实 VM。

[时区文件读取检查](../sandbox-guest-runtime/src/file_download/local_zone/file/tests.rs)在真实私有临时目录覆盖普通文件/可信 symlink、后续改写/删除、原名替换后同一 FD、配置/路径先行、目录/空文件/元数据已超额，以及无写端 FIFO 不等待。受控 Read 覆盖短读、Interrupted、满额 EOF 探测、探测失败、早期 I/O 错误、增长与截断；FIFO 用独立线程和有界超时观察，失败路径释放误阻塞的 open 并 join，结束删除本轮资源，不靠 sleep 调度。六项新回归已纳入上述 local_zone 入口；不代表系统 TZ 搜索、正式 daemon 或整套 E05 验收。[#480 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38679279467) 首轮在 Rust 构建前因离线黄金盘缺少 tz-rs 0.7.3 失败；按该源提交 a492c8b481a5e65cd7ce86a552190c10360442b7 的锁文件受控预热、两轮全新禁网 QA 并切换后，attempt 2 的四项普通检查全部成功，见[维护记录](https://github.com/VeriInfra-Lab/sandproof/pull/480#issuecomment-6195347453)。该结果不覆盖后续新增的文件读取场景；它们随 #486 汇总候选验收。

[时区与下载条件组合](../sandbox-guest-service/src/file_download/tests/reference/timezone.rs)把真实 `Snapshot::from_tzif` 接入原响应计划器，覆盖非整分钟偏移、同名缩写的历史切换、TZif v2/v3 的未来夏令时、未知缩写、GMT/旧日期与小数秒；普通回归明确核对 200/206/304/412 和 UTC Last-Modified。显式 Go 组通过同一参考二进制的新 C 操作，先由原 `LoadLocationFromTZData` 设置 Local，再运行原 `ServeContent`；7,168 组 GET/HEAD、三类日期条件、标签优先级、完整/单段/多段/无交集范围比较状态、11 个头和正文，已有本地通过。入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::tests::reference::timezone -- --include-ignored --nocapture`，须设置并重建上述 `SANDPROOF_GUEST_DOWNLOAD_REFERENCE`。该组的预期结果来自 Go，Rust 只选择比较时刻，不另实现预期条件算法。

[Linux 文件时区组合](../sandbox-guest-service/src/file_http/tests/timezone.rs)从真实可信 symlink 读取快照，随后删除原时区文件与链接，再让原 FileHttp 对固定 mtime 的真实文件执行 12 次 GET/HEAD 请求；检查 200/206/304/412、未知缩写、GMT Last-Modified、范围长度/字节与请求容量归还。该组纳入普通 Linux service 测试，定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_http::tests::timezone`；仅交叉检查不算实跑。[#482 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38746966384) 的 SDK Python 77 项通过，但该次 Linux Rust 与 Harbor 因离线材料缺少 tz-rs 0.7.3 未执行（源 ce2bd5148d4f04b225ebe49247f9a3e41cd77e7f，checkout dce7218bbd5875d5681fb7f623854f4a2a50264d）。该历史失败不反复重跑，新增场景与原完整 Rust 检查统一在同步 integration 的 [#486](https://github.com/VeriInfra-Lab/sandproof/pull/486) 精确汇总候选上验收，并在 PR 记录源提交、目标基线及最终日志；未完成项不能先算通过。本组合不选择系统 TZ 来源，也不表示正式 daemon、原 SDK 双端 E03、KVM 或形式化验收通过。

[范围正文执行检查](../sandbox-guest-service/src/file_download/stream/tests.rs)纳入可移植 service lib；定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::stream`。覆盖条件拒绝前不 seek/read、完整/单段/多段/零后缀与 HEAD、准备期和正文期的 seek 失败、短读前缀、无伪造结尾、匿名 HTTP 正文错误、原期限以及 Bytes 克隆背压。阻塞 seek/read 用屏障控制，超时用可推进时钟；确认取消后守卫仍保留到实际调用与源析构结束。本地已运行；共享 gzip worker 重构须同时运行原上传/下载所有权回归。

[Linux identity 范围检查](../sandbox-guest-service/src/file_transfer/identity_download/tests.rs)增加真实临时文件、空/二进制、原 FD 替换/增长/缩短、AuthPermit/容量保留及所选文件预算；定向入口为 `cargo test -p sandproof-guest-service --locked file_transfer::identity_download::tests`。固定 SDK 文件夹具的 identity GET/HEAD 已接该组件，合成 HTTP 客户端在原 SDK 写入后检查单/多段、无交集、空文件、304/412、If-Range 回退及 HEAD，原同步/异步 SDK 文件回读继续同路执行。该组还检查原查询扩展名与解析后文件名分离，以及同一 FD 的嗅探/范围读取。测试路由使用受控内建 MIME 表、UTC 和测试边界；identity 自动嗅探，gzip 只查扩展名，合成 HTTP 检查类型/Disposition 和多段各 part 类型，不表示实际模板 MIME/时区/协商已接通；新增 Linux 组尚未实跑，不计为通过或 E03 全端点对照。

[下载顺序检查](../sandbox-guest-service/src/file_download/stream/tests/negotiation.rs)覆盖 stat/目录拒绝先于编码、编码拒绝不 open、两类 Vary 阶段、原四类条件回退、gzip 不嗅探、HEAD 实际消费但抑制正文，以及 stat/open 阻塞取消时原守卫保留。普通组验证准备 seek 的匿名 500 和头清理；显式 Go 组比较嗅探/长度两阶段故障的状态、11 个头和正文，Go httptest 的 HEAD attempted body 按真实 net/http 抑制规则规范化。入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::stream::tests::negotiation`；原 gzip 所有权及计划器回归须同时运行。新增显式组复用下述参考二进制与环境变量，运行该组时另加 `-- --include-ignored --nocapture`；普通 CI 不代为执行 Go。

[下载错误检查](../sandbox-guest-service/src/file_download/stream/tests/errors.rs)覆盖 stat/目录/编码/open 的 JSON 阶段、初始单段 seek 的 416、HEAD 抑制、错误头清理、精确转义字节预算、未知错误不泄漏任意诊断，以及错误 Bytes 克隆保留原守卫。正文开始后的多段 seek 失败保持正文错误，不补成功结尾。普通定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::stream::tests::errors`；显式 Go 对照沿用下述参考二进制及 SANDPROOF_GUEST_DOWNLOAD_REFERENCE，并加 `-- --include-ignored --nocapture`。对照直接调用原 ServeContent、PathError、JSON Encoder，复用固定 envd jsonError/Error 和下载分支文本；Linux errno 表复制自 Go 1.26.8 原文件，只有运行于 Linux amd64 时再核对原 syscall.Errno。macOS 表驱动比较已运行，不能当作实际 Linux 文件错误、完整 HTTP 连接或 envd 双端验收。

[Linux 协商下载检查](../sandbox-guest-service/src/file_transfer/download/tests.rs)检查实际文件/目录错误优先级和 gzip 请求下的范围回退；原[文件端口检查](../sandbox-guest-runtime/tests/file_transfer.rs)检查延迟 open 的路径替换、保留旧 stat 信息及删除后 open 失败。SDK 文件夹具已接统一下载，增加同步/异步合成 HTTP 的大小写/q、范围/条件、重复 Accept-Encoding、HEAD 及 stat/406 优先级；这里只统计固定 SDK 原调用和合成请求分别承担的证据；下载错误 JSON 已由同一组件交付，夹具核对 code/message、UTF-8 Content-Type、nosniff、换行及解析路径。真实 ENOTDIR/ELOOP、NUL→EINVAL、延迟 open 后删除和 File seek errno 另有 Linux 定向回归。以上 Linux 新组已交叉检查，实际运行待补。

[下载账户准入检查](../sandbox-guest-service/src/file_transfer/download/tests/request.rs)覆盖同一槽位内的账户→路径→identity/gzip、range/HEAD，初始化默认值与显式用户名、原查询扩展名与解析路径分离、账户/路径失败先于 stat/编码。条件变量和显式通知确认账户调用已进入，再用取消/可推进时钟核对原授权与文件槽位保留、账户期限与请求期限取早、账户耗时不退还、最后正文副本占额及释放后的正常请求。入口 `cargo test -p sandproof-guest-service --locked file_transfer::download::tests::request`；已交叉检查，Linux 执行待补。固定 SDK 文件路由的全部下载已改用该入口，原调用集和请求计数不变；Linux SDK 组合须在此候选重跑，不借可移植编码检查核销该组。

[下载 Go 参考适配](../tests/compat/guest_download_reference.go)直接调用 Go 1.26.8 原 `http.ParseTime` / `http.ServeContent`，使用合成字节、显式本地时区及 httptest writer；默认错误头开关固定为 `httpservecontentkeepheaders=0`。显式组逐项核对日期秒/纳秒，以及状态、选定实体头和完整正文，只把 Go 生成的随机多段边界等长规范化以便比较。覆盖旧日期/时区、条件组合、GET/HEAD/POST、零时间、范围边界和种子 0x311 的畸形输入。本地已运行，默认 ignored；不是完整 envd/SDK 双端、FD 失败、Linux 或 KVM 验收：

```sh
GOTOOLCHAIN=local /absolute/path/to/go1.26.8/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/download-reference tests/compat/guest_download_reference.go
SANDPROOF_GUEST_DOWNLOAD_REFERENCE=/absolute/path/to/download-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  file_download::tests::reference::fixed_go_download_reference -- --ignored --nocapture
```

[下载 MIME 检查](../sandbox-guest-runtime/src/file_mime/tests.rs)和原[流执行检查](../sandbox-guest-service/src/file_download/stream/tests.rs)覆盖 basename/扩展名、filename 编码与精确头预算、内建/系统表优先级、大小写、空表/错误/超额，以及 304/412 和显式 Content-Type 跳过嗅探、512 字节上限、前缀读取错误、回绕与长度顺序、取消阻塞嗅探时守卫保留。本地可移植检查已运行。Linux 固定路径加载及实际 FD MIME 组已交叉检查，尚未实跑；受控源对照不能代替实际来宾 MIME 数据库证据。

[下载 MIME Go 参考适配](../tests/compat/guest_file_mime_reference.go)直接调用固定 Go 1.26.8 的 DetectContentType、FormatMediaType、filepath.Base/Ext；目录查表复制该版本 mime/type.go 和 type_unix.go 的原函数体，仅变更 package/import 并增加驱动，六个路径指向适配器自己创建的临时文件，运行后仅删除自有目录，不读取宿主 MIME 表。显式组含签名截断/变异、种子 0x311 随机输入、原始文件名字节及受控数据库。已本地运行，默认 ignored，普通 CI 不代为执行；不是完整 envd/SDK 双端或 Linux 实测：

```sh
GOTOOLCHAIN=local /absolute/path/to/go1.26.8/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/file-mime-reference tests/compat/guest_file_mime_reference.go
SANDPROOF_GUEST_FILE_MIME_REFERENCE=/absolute/path/to/file-mime-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  file_mime::tests::fixed_go_file_mime_reference -- --ignored --nocapture
```

gzip 新增固定 Cargo 依赖须按[CI 可信预热](ci.md)制备，普通任务仍离线且缺料失败。源码/本地缓存更新不表示黄金盘完成制备；不临时联网、改弱检查或切换现役模板。

[Go MIME 参考适配器](../tests/compat/guest_multipart_reference.go)仅调用固定 Go 1.26.8 标准库，不调用 envd HTTP/文件端点。它无需外部模块，必须使用经过校验的固定工具链；程序会拒绝其他版本。显式对照保存种子 0x311，以相同输入比较四种 Rust 分块下的文件名、正文前缀、part 完成与请求终态；MIME 头 EOF 的严格拒绝单独核对，其余差异使测试失败。普通 CI 不制备 Go，此参考组默认 ignored，不能把普通绿灯记为已运行；本地已显式运行该组。入口为：

```sh
CGO_ENABLED=0 GOTOOLCHAIN=local go build -trimpath -buildvcs=false \
  -o /absolute/path/to/multipart-reference tests/compat/guest_multipart_reference.go
SANDPROOF_GUEST_MULTIPART_REFERENCE=/absolute/path/to/multipart-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  multipart::tests::fixed_go_multipart_reference -- --ignored --nocapture
```

[固定 Go 方法对照](../sandbox-guest-runtime/tests/file_reference.rs)为显式 opt-in，不执行时必须报未运行。以固定 runtime `f455fd58fa3f289e78ce1de690c35968d8d29322`、该版要求的 Go 1.26.8 和锁定模块，采用 Go `-overlay` 把仓库[参考适配器](../tests/compat/guest_files_reference.go)映射为 `packages/envd/sandproof_files_reference.go`，使用 `CGO_ENABLED=0 GOTOOLCHAIN=local`、`build -mod=readonly -trimpath -buildvcs=false` 编译该虚拟文件。记录原源码/工具链与适配器二进制摘要，原 Go 源树须保持未改。该程序直接调用原文件服务方法，不启动 envd 主程序、MMDS、cgroup 或监听器。编译后显式运行：

```sh
SANDPROOF_GUEST_GO_FILES_REFERENCE=/absolute/path/to/pinned-reference-adapter \
  cargo test -p sandproof-guest-runtime --locked --test file_reference -- --ignored --nocapture
```

参考适配器新增可选 Detailed 字段，直接返回原 connect.Error 的 code/message，未启用时维持原类别输出。[错误原方法对照](../sandbox-guest-service/src/file_rpc_http/tests/reference.rs)在相同临时路径及初始状态下比较原 Go 方法与 Rust HTTP 组件的十三种真实失败，含特殊字符、NUL、非目录、循环链接、父目录部分效果和 rename 目录冲突/同路径/源错误优先；显式运行 `SANDPROOF_GUEST_GO_FILES_REFERENCE=/absolute/path/to/pinned-reference-adapter cargo test -p sandproof-guest-service --locked --lib original_go_methods_match_observed_unary_error_messages -- --ignored --nocapture`。需用新适配器重新构建固定源二进制；当前未实跑，不计为普通 CI 或双端 HTTP 验收。目录另有 original_go_list_dir_errors_match_http，比较十种 ListDir 原方法错误（含未清理路径、NUL、断链、循环与非目录）；同样显式运行并仍待 Linux 实跑。Remove 另有 original_go_remove_errors_match_http，比较九种末尾点、非目录父路径、循环、NUL/转义与斜杠失败；使用同一固定原方法适配器，尚未 Linux 实跑。

对照在同一路径重建相同文件树，核对属性、错误和最终路径/类型/模式/字节或链接目标；读取时间戳预先固定且精确比较，新建目录返回的动态 mtime 不比较。参考现使用原 Connect 默认 protobuf JSON 选项，不再归一化空 metadata 与空 map，其他字段不宽松忽略。它是原 Go 方法到 Rust 端口的组件对照，不能替代 E03 同 SDK 的完整双端 HTTP 对照，也不核销权限矩阵、磁盘满、流式传输、进程/VM 重启和 KVM 验收。

[初始化会话检查](../sandbox-guest-service/tests/session.rs)消费同一宿主初始化样例，检查默认值/环境替换、元数据身份、令牌原子更新、同令牌重试、旧授权拒绝、重启、恰好预算和取消后恢复。[HTTP 路由检查](../sandbox-guest-service/tests/http.rs)覆盖真实回环请求、慢正文、断流、共享槽位、重复鉴权头及错误脱敏；[MMDS 交换检查](../sandbox-guest-service/src/metadata/tests.rs)以实际 TCP 服务核对两次请求并接到初始化路由，拒绝重定向、畸形令牌/元数据及截断。屏障与可推进时钟控制取消/期限，服务端观察原连接 EOF，验证无遗留连接驱动。基础检查和 Linux SDK 合同脚本均运行这些组件；它们不是 Firecracker MMDS 实测、命令鉴权集成、固定 Go envd 双端对照或 KVM 验收。

首批[命令/文件纯核心测试](../sandbox-guest-core/tests/completion.rs)由 workspace 基础检查发现，覆盖退出/双管道 EOF 的全部顺序、未消费输出、旧内部身份、持续输出的有限驻留、短写/截断和关闭失败；不启动真实进程或操作来宾文件。[固定协议样例](../tests/compat/guest-service-contract.json)由 [SDK schema 检查](../tests/compat/test_guest_service_contract.py)读取，核对命令、PTY、二进制输入、文件属性及编码。它们不是完整 Rust 服务的协议对照，尚未核销 #311 的 E01–E10。

[Linux 进程端口测试](../sandbox-guest-runtime/tests/process.rs)使用真实私有 worker、pidfd、管道和命令，核对放行前无用户副作用、当前主进程 PID、原字节/非零退出、超过单块预算的持续输出、消费前不得 End、stdin 取消与关闭、独立输入句柄的大块双向传输、后代持管道时主进程退出、旧内部身份与 exec 失败。SIGSTOP + waitid 建立已观察的暂停点，验证放行回复取消仍可能执行一次但绝不再次放行，以及取消输入后的实际前缀；不使用随机 sleep 建立顺序。这是普通 Linux 当前账号的组件检查，不是 E2B 对照、账号降权、强制 PID 复用、服务重启、整机资源压力或 KVM 验收。

请求入口（#290 R1）用[管理路由合同](../sandbox-api/tests/http_contract.rs)、[真实应用锁夹具](../sandbox-api/tests/managed_port/admission.rs)、[取消/回应期限测试](../sandbox-api/src/lib.rs)及 [Linux 网关端口夹具](../sandbox-host-service/src/data_gateway/port_tests.rs)验收：受控宿主持锁时请求满额即 429，不新增后端调用；有效/无效数据令牌均受派发预算限制；控制及后台名额保留；HTTP 取消、回应期限耗尽后阻塞工作继续占额，实际结束/异常退出才释放。固定 SDK 的既有 429 映射与重试检查仍须通过，重试不意味着客户端具备幂等键。以上为真实 SQLite/HTTP 加受控宿主的组件证据，不是 KVM 性能或完整拒绝服务防护证明；远程部署还须验收模块合同列出的 TLS 前端约束。

- 接受意图及 outbox 先提交，再执行外部动作；提交前故障不得产生 VM。启动成功而回执未落盘时，重启认出原 VM，不创建替代身份。
- 启动期限不冒充运行寿命；运行起点与 Running 同事务发布。续期、到期和终止竞争不能由旧定时工作复活或误销毁实例。
- 单写者缓存与独立完整重放比较；SQL-only 事务和不确定提交使关系成立或失效，不能发布未确认状态。
- 编号高水位跨重开保持单调，u64/i64 边界不回绕；准入前耗尽无宿主动作，准入后未知保留原操作。数据库损坏、绑定缺失或旧格式不安全时失败关闭。
- 已接受工作的后续转换、重试和清理额度先预留；新请求容量不足被拒绝，不能抢走收尾额度。物理 Free、逻辑退休和结果过期是不同事件。
- 有限位置重用拒绝旧 incarnation；未知对象仍占用。检查点根先提交再删除覆盖前缀；一个长期活跃或未知对象不阻止无关终态退休。
- 窗口关闭/结果删除后旧公开、私有构建键不能创建替代工作；候选交接、私有结算和公开确认分别检查。
- 按完整部署配置核对 DB/WAL/temp、各表、claim/fence/secret、登记、日志、模板缓存和稀疏盘未来增长。条数下降不等于真实字节有界。

组件入口包括[缓存差分](../controller-service/src/controller/cache_tests.rs)、[身份分配](../controller-service/src/controller/allocator_tests.rs)、[API 耗尽](../sandbox-api/tests/managed_port/allocators.rs)、[有界位置与维护](../sandbox-api/tests/managed_port/bounded_slots.rs)及[模板发布](../sandbox-api/tests/managed_port/publication.rs)。实际宿主观察不能仅由这些测试替身代替。

#292 的[原子本地映射与事实组件](../controller-service/src/controller/internal_inbox/tests.rs)纳入 `scripts/check-distributed-contract.sh` 的原收件测试步骤：准入提交前后与两种未知提交结果均重开真实 SQLite，检查实例/租户映射、原配额、固定位置和编号同进退；模板修订/摘要/形状/owner、期限及配额拒绝不产生本地责任；多租户映射单射且重复请求不重分配。销毁收件与原核心终止同事务，准备动作回执丢失时也不得继续旧创建。原工作流的受控证据只用于组件测试：Ready 使用原发布时刻，事实提交前后故障可重建原结果；Destroyed/逻辑回收还不是 Cleaned，必须核对原 ConfirmRelease；该实例的终局确认及保留位置退休完成前，本地维护不能删掉映射依赖。旧收件格式明确拒绝且不修复。这些不是原应用执行组装、KVM 或两题通过的证据，实机仍按 DK 场景验收。

[宿主应用执行组件](../sandbox-host-service/tests/durable_https/execution.rs)在原 ControllerApplication 和真实 SQLite 上验收创建/重复/重开、丢激活回执恢复、销毁先到、原数据许可即时撤销及排空、物理释放未知保留 Stopped、中心断线后的原到期处理；取消由明确宿主屏障控制，实际工作结束前入口许可不归还。[真实 Redis/HTTPS 组合](../sandbox-center-store/tests/redis_ledger.rs)再检查原本地执行事实已持久而中心丢回执/重启，重发原请求得到同一 VM/寿命，只有收到原 Cleaned 才释放中心额度。宿主物理动作在这些组件中受控，不能记为 KVM、SDK 生产入口或 DK 用例通过。

凭据隔离另以[同库索引故障](../controller-service/src/controller/data_credentials/tests.rs)、[管理层恢复与授权](../sandbox-api/tests/managed_port/credential_isolation.rs)及[两台真实虚拟机](../tests/system/server_credential_isolation.py)分层验收：单个宿主凭据缺失、损坏或有效内容改变只拒绝该对象；健康对象仍可执行命令、读写文件。共享索引缺失、损坏或重复绑定须拒绝数据授权，不自动补造登记；原管理清理仍按生命周期合同进行。实机入口保留原凭据 inode，故障后恢复它并逐台核对精确资源，不代表恶意宿主存储已验证。

## 3. Linux/KVM 验收

### 正式来宾服务的隔离重启组件

[Start 参数分配回归](../sandbox-guest-runtime/tests/protocol_argument_budget.rs)检查完整字段、参数边界、超额后的非法元素/重复字段/尾帧错误，以及独立 Linux 子进程中的五百万空参数。子进程将地址空间硬限设置为其原有映射加 64 MiB，输入仍低于原 16 MiB 正文限额；该限制是回归测试条件，不是产品内存预算。父测试实际调用并回收标为 ignored 的子入口。[服务组合](../sandbox-guest-service/src/command_requests/tests/start/argument_budget.rs)同时检查 identity/gzip、显式/默认账户的原拒绝顺序、不进入登记/准备，以及失败的最后所有者退出前请求额度不归还。以上不替代正式 daemon 压力、全服务内存峰值或 KVM 验收。

[环境合成回归](../sandbox-guest-service/src/execution/tests.rs)核对覆盖后的精确条数/UTF-8 字节边界，以及账户、目录和非法变量名的原拒绝顺序。[独立 Linux 内存回归](../sandbox-guest-runtime/tests/context_environment_budget.rs)预热原账户工作线程，在已构造超额环境之后限制额外地址空间，要求原公开上下文合成返回 ResourceExhausted 而不复制完整环境；父测试实际执行并回收 ignored 子入口。此测试不启动用户进程、不经过正文解码，地址空间限制不是请求额度或产品预算；正式 daemon 的环境解码及并发压力仍须独立验收。

[环境解码回归](../sandbox-guest-runtime/tests/protocol_environment_budget.rs)检查原合法请求等价、数量边界、分块边界前后及跨块转义键与未转义键的精确重复拒绝、超额后的非法字段/尾帧及参数拒绝优先；独立 Linux 子进程在原映射加 32 MiB 的地址空间条件下解码低于 16 MiB 正文的 120 万个唯一环境键。另用单 glibc arena、原映射加 64 MiB 的独立子进程对照需要兼容数字规范化的同规模输入，约束副本与键表的同时驻留；修复前该用例因分配失败退出。父测试实际执行并回收两种 ignored 子入口；这些是回归条件，不是产品预算。压缩组合另将重复键早于畸形值/尾部、字典顺序与输入顺序不同、多处重复及长转义键的原错误观察逐项对照，验证临时排序不能重排公开首个错误。完整 Start JSON 样例集另将紧凑路径与原路径逐项对照；固定 Go 全文入口还对全部案例启用零参数/环境额度，核对所有原失败及合法消息的不可执行标记分支；转义键内联/字节块边界、UTF-8 偏移、增长、换块及超单块完整键均覆盖，分块粒度不能成为新的公开输入限制。固定 Go 错误对照另追加多处重复、重复早于坏值/词法错误和跨块 token 位置样例；原有样例的编码/charset 组合保持，新增样例跑完整交叉组合。[服务组合](../sandbox-guest-service/src/command_requests/tests/start/environment_budget.rs)同时核对 identity/gzip、显式/默认账户、原 cwd/执行时限/非法变量名顺序、额度匹配、无登记/准备及失败最后所有者退出前不归还请求容量。这些检查不代替正式 daemon 的混合满额压力、错误诊断分配或全服务 E05 验收。

[键表示回归](../sandbox-guest-runtime/src/file_protocol/request_json/key_arena.rs)覆盖空/NUL/Unicode、UTF-16 surrogate pair、15/16 字节内联边界及跨表示的精确相等/排序，并限制键结构不得因优化而增大。内联阈值只是存储选择，不拒绝长键；长键仍按原规则解码。[原压缩观察回归](../sandbox-guest-service/src/command_requests/compression/tests.rs)比较超额环境伴随转义重复键、gzip 损坏和坏尾帧时的完整原错误观察；命令 JSON 的固定 Go 全文对照仍沿原入口显式执行。

[服务任务计数回归](../sandbox-guest-service/src/task_accounting/tests.rs)用明确屏障检查未轮询取消、排队阻塞取消、实际返回与 panic、等待者丢失、runtime 退出超时、不同时间峰值、多 runtime 共同计数，以及算术/毒化异常不可恢复成精确观察。返回值另有存活责任，不能因任务归零视为内存释放。[真实监听器组合](../sandbox-guest-service/tests/task_accounting.rs)在独立进程用原 Listener 和有限响应编码器检查排队期间的异步/阻塞同时计数、连接满额不新增任务及实际退出归零；[正式 CLI 拒绝](../sandbox-guest-service/tests/daemon.rs)同时核对退出报告、原阶段诊断及 PID 状态不变。它们纳入完整 Linux 基础检查，不执行 KVM 或满额业务压力；整服务预算、依赖库开销、命令/文件组合峰值和 E05 仍须独立实测，不能把这些计数回归记作全服务资源验收。

正式装配的任务压力验收按[模块合同](module-contracts.md)的原容量和同步析构重叠计算各类别及合计上界，保存精确源码、配置与二进制摘要。使用退出报告的同一 `total.peak`，逐类核对 `peak`，并要求 `exact=true`；不能相加不同时间的分项峰值。正常收尾另要求 live 归零及独立进程/文件清理；强杀、未知或仍存活任务保留原失败，不补造退出报告。工作负载应同时覆盖保留记录满额、普通/控制请求、文件 RPC/慢传输和取消后的排队工作，保持原期限与配额。环境解码还须覆盖压缩、数字规范化和接近正文限额才发现的重复键，原完整错误与后续合法请求均应保留。有限压力通过只支持已执行的组合，任务上界不代替整服务内存声明；daemon RSS 样本、worker/用户进程及整 VM 的观察范围分别报告。

公共网关的混合内存验收同时核对[模块合同](module-contracts.md)声明的整来宾和原 VMM 预算。使用同一候选的不可变模板、固定配置及原 SDK，先确认实际运行身份；按原登记容量同时保留大元数据终局和活跃命令，再叠加慢上传、慢下载与接近公开正文上限的未完成请求。填充并发、参数/标签大小、保留期限、文件长度和控制请求入口在运行前固定，不因失败降低压力或重放未知 Start。普通传输满额时要求原明确拒绝，仍有原控制容量时核对 CloseStdin 等实际控制请求及期限；Update 不属于公共网关预留控制入口。

该场景须核对部分上传的实际前缀、完整下载摘要、同一 daemon 身份、来宾及原 VMM OOM 计数、销毁前的内核 memory.peak，以及压力释放后的合法业务。daemon RSS 采样注明间隔与次数，不当作连续峰值；任务峰值来自独立的原计数器，不能从采样线程数推导。直接来宾入口的压缩/多请求诊断保留其独立环境和预算，不折算为公共网关等价负载。失败时先保存原错误及可取得的现场观察，随后按原身份销毁并独立核对进程、UID 范围、网络、磁盘、暂存目录及 cgroup；清理成功不能改写失败，后一轮通过也不能证明前一轮未知错误的根因。各项通过只支持已执行的工作负载，E01/E05 整体仍须对照完整验收清单复核。

[单写登记容量回归](../sandbox-guest-runtime/tests/preparation/registration_capacity_http.rs)用一个阻塞线程和明确屏障保留已取消但尚未执行完的 PID 登记工作，再通过原 CommandHttp 发起另一个 Start：检查明确 EOS 拒绝、无用户命令效果、原 registry/worker 收尾、最后响应字节占额，以及真实登记工作退出后的单次合法执行。拒绝编码另覆盖 identity/gzip 的精确预算、原期限和存储 Busy/WriteUnknown 不可误映射。[共享样例](../tests/compat/guest-service-contract.json)同时由真实 Rust 编排回归及固定同步/异步 SDK 错误解析检查读取；后者使用合成回环响应，不代替 SDK 经真实 VM 的组合。该分支修复不能仅因一次实机未复现就被认定为此前所有 502 的根因；同提交实机与原失败证据须分别保留。

[正式 daemon 重启检查](../tests/system/guest_daemon_restart.py)在新的 Linux x86_64 PID、网络和挂载命名空间中执行实际 `initialize/serve` 二进制与 worker，固定 MMDS 地址由隔离的受控服务提供；不改生产地址、监听器或处理器。固定 SDK 的同步/异步原 RPC 客户端分别保持一个真实命令，强杀原 daemon 后核对原 pidfd 仍存活且订阅没有伪造 End；重开原持久文件后，未初始化及错误令牌拒绝，重新初始化后旧 PID/tag 的 Connect、输入、关闭输入、信号和 resize 全部明确拒绝。新命令可复用 tag 正常结束而不接管旧进程；原进程只由测试的原 FIFO 释放并按 pidfd 等待。错误 VM 身份启动拒绝且原文件内容/inode 保持。测试进程本身必须为隔离空间的 root/PID 1，网络只含 lo，普通直接执行及 `-O` 均拒绝。失败收尾仅操作本轮捕获的 pidfd，PID 命名空间退出是未知后代的额外收尾边界。

原强杀场景要求没有退出报告；恢复后完成的新命令再经 SIGTERM 正常收尾，分别保存 initialize、serve 退出及错 VM 启动拒绝的精确任务报告。读取器拒绝缺失、超长、重复字段、未知类别、非整数、不一致峰值、仍存活或 `exact=false` 的观察；核对实际命令各类任务曾被计数。这只验证上述有限调用集的观察与任务退出，实际进程仍由原 pidfd 独立确认，不能用计数归零代替它。

这是正式入口的特权组件检查，**不代替真实 Firecracker MMDS、模板启动、中心/宿主网关、VM 寿命/销毁或 E04/E07 整体验收**。先构建同提交的 `sandproof-guest-service` 和 `sandproof-guest-worker`；在批准的隔离 Linux 测试机，使用 `tests/compat` 锁定 SDK 解释器运行下列入口，超时监督必须在命名空间外。配置样例只提供显式测试预算；脚本仅在自己的私有目录改路径、owner 及实际 worker 指纹，禁止把测试值作为生产默认值。输出包含实际二进制、脚本及配置摘要，结果必须与源码提交一同封存。

```sh
sudo timeout --signal=KILL 150 unshare --mount --net --pid --fork --kill-child --mount-proc \
  "$PWD/tests/compat/.venv/bin/python" "$PWD/tests/system/guest_daemon_restart.py" \
  --service "$PWD/target/debug/sandproof-guest-service" \
  --worker "$PWD/target/debug/sandproof-guest-worker" \
  --configuration "$PWD/sandbox-guest-service/src/daemon/tests/configuration.json"
```

### Rust envd 的组合验收（#311）

[原监督链的来宾阶段](../tests/system/envd_acceptance.py)复用同一中心/宿主、Redis、私有构建和不可变发布入口，不更改现役默认模板。固定支持清单和故障清单由[同一策略](../tools/envd_acceptance.py)维护；CI 接线和受控安装要求见[CI 环境](ci.md)。所有结果仍须在关联 PR 绑定实际提交、清单/配置/镜像摘要、原退出与精确残留；新增入口存在不等于已经实跑或 #311 可以关单。

复用原 Go/Rust 同步/异步对照和资源观察、单次大输入与内部块、PTY、命令/RPC/文件期限及文件权限场景，不删目录大小、原字节或错误断言。[故障片段](../tests/system/sdk_guest_faults.py)只在自己新建的 VM 注入：不消费 Start PID 即断开并核对一次性效果；真实 pidfd 绑定的 daemon 强制退出/原 init、原用户进程存活、旧 PID 拒绝、原持久位和会话递增；独立小 tmpfs 写满与部分文件；不读第一订阅时其他请求和生产者仍推进；跨原登记容量的有限回收与原 30 秒保留；连接/续期，以及 PID 文件丢失后无法服务但宿主仍到期清理。写满挂载仅在来宾内，不能改宿主全局磁盘或网络。重启测试由原受保护入口新建实例，通过停止原宿主后的收件审计核对全局到本地身份，仅将该实例原生 guest secret 对应的 init 负载写入同一来宾私有目录；保留模板默认用户、工作目录和创建环境覆盖，不重放启动命令。SDK 公共网关令牌不能代替原生凭据；不传中心/宿主或 CI 管理凭据。原 init 仅由来宾原生故障夹具调用，不新增公开初始化旁路。原探针期限包含准备阶段，失败仍输出有限场景、读结果错误类型及原实例清理记录，并保留首个失败和清理失败。

生命周期片段另通过原 SDK 在新的 Rust VM 执行 S03：使用镜像内 dpkg 离线安装受控的最小 Python 依赖，分别以 root/ubuntu 导入实际安装位置，并写入/回读包含非文本字节的 `/etc` 测试文件。随后才创建同模板的新 VM，核对包、模块及系统文件均未泄漏；原 daemon 身份保持。宿主入口按原模板缓存描述读取实际共享 rootfs，在整个调用集前后比较完整摘要与原 inode 身份，并检查对应宿主系统路径不存在。它不访问外部软件仓库、不安装宿主包、不减少原场景，也不把来宾文件 user 当作 DAC 边界；失败保留原结果，由原 VM/服务收尾处理，未知安装不重放。

各组均检查原 SQLite 终局和进程/网络/磁盘精确残留，服务名额及 cgroup 也须实际收尾。容量显式拒绝可在确认无效果并等待原保留期后发起新尝试，未知 Start/写入不重放。资源采样记录实际 RSS/峰值/线程及固定 PID 文件长度，并核对原 VM 和控制服务预算；有限观察不推出无限历史或整进程内存证明。强制 PID 数字回绕、任意故障交错和外部内核正确性不因该组变成已验证；原单元/故障点证据仍单独保留。

服务存储清单在扫描期间遇到已打开但被替换后解除链接的文件，必须记为未知，并等待原期限内的稳定观察；不能误记为永久硬链接违规，也不能把未知当作零占用或通过。硬链接、错误权限/归属及预算超额仍拒绝并保留失败。外层收尾失败也保存原场景、资源清理与有限采样诊断；诊断只含原根索引、固定错误分类、计数及已观察峰值，不含路径或内容，不能替代成功的最终存储清单。首个场景失败、资源/服务收尾失败和报告写入失败保持各自责任，不重置采样历史或回写旧运行成功。

工程交付按 #311 的每一验收项核对原始证据，而不是把一个组或绿色检查扩展成整单通过：

- E02 的组件证据来自完整 Linux 基础与固定 SDK 检查；确认相关用例实际执行，父测试调用的隔离 helper 与单独未运行的 ignored 用例分别记录。
- E03 保存固定同步/异步 SDK 的 Go/Rust 四端原调用、响应/事件/错误和最终比较；动态 PID 等只按预先冻结规则处理。`sdk_files` 在同一 SDK 模式比较，已批准 tag 等差异另列，不省略支持项或改写原字节。
- E04/E06/E07 将真实 Start 回执丢失、旧 PID/会话、daemon 重启、寿命/销毁和其他实例观察与原资源审计关联；组件的拒绝复用用例与真实 VM 故障各有范围，不用空 List 或停止信号代替原进程/VM 消失。
- E08 的受控安装先在同一候选完成监督器的原 DK、Rust 阶段和原题集链路，并按[CI 安装规则](ci.md)验证真实失败、取消及精确恢复。普通 CI、`--local-only` 安装预演和原 App 正式回写分别记录；预演不能冒充正式检查，也不能降低原题集或分支门槛。
- E09 核对 release/profile、安装载荷、实际启动与新不可变模板的同一身份，保留错版本/摘要和初始化失败的拒绝证据；旧模板保留/退休及默认别名政策只在[模块合同](module-contracts.md)维护。
- E10 在原权威文档同步能力、迁移与未证明边界，交付记录绑定源码/模板/宿主/依赖及成功、失败、退出和精确清理。结果进入 main 后才可按全部适用项关工程单；不能把 #101 的待证义务改写为外部正确性假设。

来宾监听器的[合并正文回归](../sandbox-guest-service/src/listener/tests/read_budget.rs)使用真实固定 HTTP 解析器和保持连接的确定性输入，不依赖 TCP 时序；核对原 16 KiB 读取边界及原上传帧检查。原版本可能交付 24 KiB 帧并触发断流/上游 502；必须保留失败断言，修复不能增大正文/块/期限。另核对等待和 I/O 错误不能变成成功 EOF。组件通过后仍须用同提交来宾产物在真实沙箱执行原 SDK 文件矩阵，不能用带诊断日志的一轮未复现代替原失败归因。

#311 的[固定 SDK 输入回归](../tests/system/sdk_guest_input_limits.py)读取实际安装的版本 3 配置及摘要，保持原内部写块数值，覆盖同步/异步、root/ubuntu：恰好写块、写块加一和 65,792 字节均以单次 SDK 调用成功，再保留多次调用传输同一二进制的检查，最终核对全部长度/摘要和原 PID/服务身份。它不替换或修改既有 `sdk_command_input.py`。原 HTTP 正文超限的写前拒绝、后续合法调用、一次性效果及容量归还继续由[真实 worker HTTP 回归](../sandbox-guest-runtime/tests/preparation/input_limit_http.rs)验收；不能把取消/超时算成精确预算拒绝。

[gzip 网关回归](../sandbox-data-transport/src/tests/gzip.rs)覆盖压缩与展开双预算、零字节、精确边界/超限、坏头/CRC/截断、实际 HTTP 转发、最后字节副本背压及取消后真实工作退出再归还容量；原固定 SDK 的 `gzip=True` 进入实际网关组合检查。完整 KVM 仍运行原 `sdk_guest_compare.py`，要求新不可变 Rust 模板和固定 Go 模板分别通过同一调用集，保留失败与精确销毁/残留结果。组件、SDK 组合与实机结果分别记录，未执行不得标为通过。

对应组件回归检查[原工作门的有界健康采样](../sandbox-host-service/tests/driver.rs)、[无上传 EOF 的来宾断流](../sandbox-data-transport/src/tunnel.rs)和[真实 worker 输入拒绝后的继续读写](../sandbox-guest-runtime/tests/preparation/input_limit_http.rs)。定点诊断先保留 503 与节点健康期限、原中心/宿主无载荷故障分类，不因单轮未复现就宣布旧故障已解决；不得延长有效期、增加测试容量或移除原检查。上述入口仍须按精确候选分别记录组件、SDK 和 KVM 实跑证据。

私有构建数据路由也输出原节点新鲜度拒绝的固定 `CenterNodeFreshness` 分类，仍返回 Busy；仅有通用 `CenterDataAuthorization` 不能直接断言是健康过期。实际 Redis 回归核对过期拒绝不修改权威状态、其他租户拒绝和原观察恢复后的同一路由，不自动重传失败上传或创建替代 VM。

公开卸载拒绝沿原 `center_wait` 的每进程 64 条预算记录固定类别：原内部传输错误、观察未就绪/过期、完整重放未命中及最终返回错误。每次同步卸载调用用 `u` 加单调序号关联原 Redis/服务错误，不记录用户、会话、凭据或请求正文。诊断不新增网络请求、不重试、不修改拒绝条件；没有分支记录不能据此认定根因。

原中心状态观察另保留有限的 `center_observation` 诊断：区分状态请求未取得回复、原回复被拒绝、取得合格回复时旧健康期限已经过去。只记录本轮节点序号、原调用前后及宿主观察时间、旧期限和原时间策略，不输出节点字符串、路径、凭据、错误正文或模板数据。每个中心进程至多尝试写入 64 条，每条少于 512 字节，最后一条标明 `remaining=0`；写入失败也消耗名额且不重试。记录来自原调用及既有时间值，不新增状态请求、不改变准入、期限或权威提交；取得合格回复不等于后续提交成功。缺日志也可能是限额已用尽，不能当作健康或请求成功证据。

中心健康/生命周期后台轮次按“本轮开始时刻 + 原扫描间隔”等待，原调用耗时计入间隔；慢轮次结束后不再额外等待整段间隔。仍只有原 Background 许可和一个实际阻塞工作，错过的轮次不积压、不重放，下一次短轮次恢复原间隔；关闭请求等待实际在途工作退出且优先于已到期的下一轮。[可推进时钟回归](../sandbox-center/src/runtime.rs)用屏障控制 1 秒/6 秒实际阻塞任务在原 2 秒间隔下的调度，并检查许可保持、无追赶队列和关闭收尾。健康窗口、状态请求期限、节点接受/过期判据及 Material 后台调度保持原值；本修正减少额外调度空档，不保证长于健康窗口的状态调用可用，也不代替实机成功与清理证据。

使用隔离 Linux x86_64 主机，KVM API 12、cgroup v2、固定工具及镜像摘要；通用配置见[示例](../tests/system/host.example.json)，真实地址、SSH 名称和凭据在仓库外提供。使用 root 私有目录、独占 UID/网络范围、空且已委派的 cgroup 父组；不能修改共享祖先或全局网络来让测试通过。确需全局变更须先获批准并恢复原值。

先运行[宿主预检](../tools/check_kvm_host.py)，它只验证环境，不能代替产品测试。系统脚本参数以各自 `--help` 为准，不复制某台机器的绝对路径。新建私有 Git 检出与构建使用 `umask 077`；受保护可执行文件及祖先不能被组或其他用户写入。环境拒绝须修正执行环境，不能降低身份检查。

所有带断言的执行入口必须在 `-O`、`-OO` 或 `PYTHONOPTIMIZE` 下于资源创建前拒绝；[统一守卫](../tests/system/_assertions_required.py)也要绑定源码摘要。C02 的只读安全导出可例外，但不得借导出模式恢复或清理资源。

#311 的候选来宾可沿[原同步/异步 SDK 数据入口](../tests/system/sdk_data_contract.py)显式传入 `--template`；Rust 身份核对还须同时提供 `--guest-release`、`--guest-manifest-sha256`、`--guest-configuration`、`--guest-configuration-sha256`，对应已审核的原 release 目录、独立 manifest pin、实际安装配置及其独立 pin。缺项或材料失配在创建 VM 前拒绝；选定模板须已由原受保护流程发布，脚本不创建模板或改别名。两个 SDK 模式仍经原中心/宿主网关创建、连接与销毁自己的实例，原 root/非 root 调用集和撤销后令牌检查不变。该选项用于获准的隔离实机验收，不纳入普通 CI 的 VM 执行。

[运行身份观察](../tools/guest_probe.py)在调用集前后通过原已鉴权命令执行只读 Python 观察：核对 `/opt/sandproof` 中服务、worker、配置、manifest 的有限长度/摘要及同一 inode，持有 `/proc/PID` 目录 FD 核对实际 `exe`、原 `serve --config` argv、start_ticks，以及该进程持有的 `0.0.0.0:49983` 监听 socket。文件正确但实际运行 Go/其他程序、监听者缺失/不唯一、参数/可执行 inode/进程或 socket 变化均不能通过；不靠服务自报版本或安装文件存在推断正在运行。观察程序不创建服务/PID 状态、不安装或发送信号，使用固定 `/usr/bin/python3 -I`，要求候选测试模板在该路径具备 Python 3、root 观察 `/proc` 的能力及原命令预算；每次 20 秒，程序材料最多 24 KiB、回复最多 4 KiB、进程及候选 FD 扫描分别最多 4096，文件按原产物上限以 64 KiB 块读取。这些是验收观察边界，不是生产资源默认值或任意调度保证。

该观察已接入上述组合入口；实际 VM 结果须绑定该次原报告及精确候选。[私有文件/proc 夹具](../tools/tests/test_guest_probe.py)核对摘要/权限/链接、路径替换、读取到的异常字节及读取前后可观察的元数据变化、错可执行文件、同字节不同 inode、错监听与重复进程、观察中身份变化及有限解析；[固定 SDK 编排检查](../tests/compat/test_guest_observation.py)使用受控 SDK 操作验证双模式候选选择、调用前后观察、无重放、拒绝后的原实例销毁及旧默认报告不变，不执行真实 create 或 KVM。原 C02 不传这些新选项，也不会导入或执行该可选观察程序；默认 base 调用及严格报告接受范围保持不变，可选 Rust 报告不能代替其精确资源审计或发布回执。来宾 root、内核和可信安装仍是工程观察前提。散列与前后 stat 检查不是原子快照，不能保证发现时间戳粒度内发生在已读区间的全部同大小改写；夹具用明确不同的 mtime 建立读取后元数据变化，不以快速写入必定改变时间戳为前提。同字节回滚、恶意来宾伪造回复、完整 SDK 双端对照、真实 MMDS/服务重启及 E06/E07/E09 仍须分别验收。

原数据入口的后台重连和 kill/wait 现共用 KnownCommand 收尾：原 Start 返回后立即登记，错误 Connect 回复只关闭订阅，失败信号仅发给原已知且未观察到终局的 PID；已尝试 kill 后不重发。重连 wait 的完整 stdout/stderr/exit_code/error 和一次性标记均须匹配；公开 wait 已给出终局后，即使这些验证失败也不补 kill。所有取得的订阅各关闭一次，保留原异常和关闭异常。原命令、Start/Connect 的 30 秒期限、异步 ready 的 15 秒界、VM 数量与寿命和报告字段保持，显式 kill/失败收尾使用 15 秒请求期限；未知进程结果仍依赖原 VM 销毁及独立资源审计，不以断开订阅证明进程退出。[数据收尾回归](../tests/compat/test_data_probe_cleanup.py)使用固定同步/异步 SDK 的原 Commands/句柄消费预定 RPC 事件，覆盖正常收尾、未知 Start/Connect/释放命令/kill、错误 PID、缺 End、结束后输出/标记失败及关闭异常，不执行 envd 或 VM。该修复改变 C02 数据探针源码及收尾请求，原批准摘要和旧实机回执不能覆盖修改后的候选，须沿原监督器重新审核和实测；报告接受范围与发布门槛不放宽。

同一工具的 `resource_command(expected, daemon)` / `resource_receipt(result, expected, daemon)` 可在已获原身份回执之后只读采样；Rust 的 expected 沿四项产物 pin，Go 为 `{'fixed_go': pinned_file}`，daemon 必须是此前原回执的 PID/start_ticks/socket_inode。调用方仍使用原 SDK 的 root 命令和 20 秒期限，新增 API 不自动插入原 C02 或双端调用集。采样前后重新验证完整安装/实际进程身份；持有原 proc 目录 FD，检查原身份、argv、exe 和监听未变，拒绝重启后新进程或同号替代。记录 `smaps_rollup` 的 RSS、status 中单列的 VmHWM、线程数、FD 条目数及读取耗时；proc 文件各一次最多 16 KiB 加 EOF 核对，FD 扫描与线程材料各至多 4096，shell 引号展开后的命令另外限制 32 KiB，原 24 KiB 程序和 4 KiB 回复界保持。缺字段、重复字段、错误单位、溢出、链接、超界及身份改变均拒绝，不降级到 VmRSS/statm。

这些读数不是同一时刻的原子资源快照。[Linux proc 文档](https://www.kernel.org/doc/html/v6.12/filesystems/proc.html)说明 RSS 快速统计有精度限制，smaps 类观察扫描实际页表；[Linux man-pages](https://www.man7.org/linux/man-pages/man5/proc_pid_status.5.html)另将 VmHWM 标为不精确。因此报告将其命名为 `reported_peak_rss_bytes`，不能据它证明精确内存峰值；线程/FD 只有当前采样，没有全时段峰值，亦不包含 worker、用户进程、内核缓冲或整 VM。采样本身经来宾命令执行，会增加工作；压力判据仍须单独配置并记录影响，不把观察器上限当产品预算。私有夹具回归核对原目标、异常 proc 材料、采样中身份替换、严格回执和优化模式拒绝；独立 Linux procfs 用例读取当前测试进程的真实 status/smaps_rollup，由完整 Linux 基础检查执行。实际候选 VM 的有限采样在组合报告中保留；它仍不是完整压力矩阵，不能单独据此勾选 E05。

#496 的身份观察编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38791264714) 通过：源 386e3ebb6584f941f19d747937474e071b67e0d7、checkout 5f864540c204b03ac8ea3cf379267f336b3c68e4；固定 SDK 89 项、封存 Redis 与四项 CI 全部成功。该回执不表示真实 VM 中的身份观察已经执行。

[同输入双端数据探针](../tests/system/sdk_guest_compare.py)在同一固定 SDK 2.51.0 的同步/异步模式下，各创建固定 Go 与 Rust 候选两台独立 VM。必须显式传入 `--go-template` 和上述完整 Rust 候选参数；同一实例 API/网关、用户、临时路径、未知账户名、输入字节及超时计划只生成一次，复用[原命令/文件样例](../tests/compat/sdk_data_samples.py)。两端分别核对调用前后的实际 daemon 身份；Go 限定[固定 base 输入](../tests/system/base-inputs.json)的二进制摘要/长度、`/usr/local/bin/envd -verbose`、root/0755、原 exe inode/start_ticks/监听 socket，源码须匹配原 runtime 锁。它不接受其他安装路径、不同摘要的同版本文件或 generic Go 模板作为这个基线，Rust 继续使用四项原产物 pin。

本入口的预定比较规则：逐项比较实际 SDK 调用参数、返回值、错误的准确类型/文本及非零退出字段；stdout/stderr 分别保持回调内顺序，原分块仍记录，比较时只拼接各自通道；文件流比较完整字节，同时记录块长度。ListDir 保持原词法深度优先顺序，不排序、不丢目录属性，并拒绝重复路径；文件和目录的实际 mtime 在读取前统一设置为 1700000000 秒并校验，返回的时间、大小、模式、归属及 metadata 均保留。JSON 比较区分布尔与整数。路径、错误和时间字段保持原样；后台命令、PTY 与 List 的 PID 采用下述按原 Start 验证的显式字段规则；仅 PTY 测试程序的 FIFO 身份结果按后述严格规则处理，其他前台结果不做 PID 替换。daemon 运行身份放在独立证据中，不作为跨 VM 相等字段。此规则与样例在首次获准实跑前供技术复核，不据本地替身成绩冻结完整 E03。

该片段覆盖 root/ubuntu 的前台命令、cwd/环境、非零退出、缺失 cwd/账户错误，以及目录创建/重复创建、原批次上传、gzip 覆盖、普通/流式二进制和零字节下载、属性/递归列表、移动/删除及缺失文件错误；最后检查合法命令仍可执行。前台案例命令执行及文件请求各 15 秒、身份观察命令执行 20 秒、原 SDK 请求默认 30 秒、每台 VM 300 秒；命令回调与下载各限制本测试输入的 128 KiB 观察材料，不改变产品输出上限。创建结果未知只记录该原尝试，不重试或补造实例身份；失败报告保留已经取得的两端结果、尚未完成的调用和已知 VM 的销毁确认，全部已知实例各尝试一次收尾并保留原错误与所有收尾错误。SDK 销毁确认不代替宿主物理资源审计，未知创建也不能因已知 VM 销毁而消失。

[双端探针回归](../tests/compat/test_guest_compare.py)核对输入/输出/错误/类型/列表顺序变化不能通过、分块差异不改变内容、超限与截断不能成为完整结果、部分证据与未知创建不重试、前后身份变化拒绝和所有已知 VM 收尾。原固定同步/异步 SDK 另通过既有合成回环服务实际执行记录器的回调、非零退出、文件流及错误解析；其余编排使用受控 SDK 操作，不启动产品 VM。该前台/文件片段与下述 List 等扩展共用双端入口，实际 Go/Rust VM 结果按上述组合验收记录；这些组件回归不能代替完整故障/资源矩阵、正式服务重启或完整 E03/E06。C02 固定监督调用集与发布门槛保持原样。

#503 的前台/文件双端探针编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38807809473) 通过：源 4eeba251b499ce2c643fc0cc31a81aea75e779d2、checkout bdf96feec3f0f443bcb74bfa209d25bd93554cda；固定 SDK 121 项、封存 Redis 与四项 CI 成功。它不包含以下后续后台扩展，也不表示已执行产品 VM 对照。

[后台双端片段](../tests/system/sdk_background_compare.py)复用同一个入口、实例和输入计划，分别对 root/ubuntu 运行空输入 EOF/正常退出、文本和全部 byte 输入/断线原 PID 重连/退出 17，以及输入等待中的 SIGKILL。案例目录由原用户创建，原命令独占写入一次性标记并通过 FIFO 发出 ready，再放行控制请求；不靠 sleep 猜测命令是否已执行。同步模式在原 handle.wait 消费回调，异步模式在原 run/connect 注册回调；逐通道比较实际回调和完整终局，原 CommandExitException 的类型、文本、字段均保留。关闭 stdin 仅执行一次，保留终局 Connect 必须仍是原 PID 且不重放输出；随后读取一次性标记。Start/活跃 Connect 的流期限为 60 秒，保留终局 Connect 为 20 秒，控制/文件请求 15 秒，VM 仍为 300 秒。该片段不调用 List，也不把 End 解释为 List 已同步删除；固定 Go 的 End 发送先于 OnExit 移除/保留操作。

后台 PID 的预定比较规则限定为每个案例唯一原 Start 返回的正整数 PID：执行时先验证每个 Connect 返回值，控制请求只使用已知原值；原报告保存 Start/Connect、句柄、订阅和控制参数中的全部数字。比较副本只将该案例顶层 PID、Start/Connect 的 result.pid、Connect/SendStdin/CloseStdin/Kill 的第一个参数、句柄操作的 handle_pid、订阅的 pid 替换为同一个逻辑引用，且每项必须先与该端原 Start PID 严格相等。布尔 PID、错误引用、缺失终局、重复 Start 或未完成调用均拒绝；stdout/stderr、错误文本、命令字符串、文件内容、其他数字和元数据不按 PID 搜索或替换。各订阅的回调只合并同一通道分块。此规则仍须随入口在真实执行前技术复核，不能据局部回归冻结完整 E03。

后台失败保留部分调用与订阅；未知 Start 不补造 PID，不重试 Start。错误 Connect PID 只用于关闭返回的订阅，失败信号只发向原已知 PID；Kill 或 disconnect 的未知回执不重复发送。所有已知句柄逐个收尾，保留原失败及全部收尾错误，外层仍销毁已知 VM。[后台回归](../tests/compat/test_background_compare.py)使用原固定同步/异步 Commands/CommandHandle 和受控 RPC 事件核对上述步骤、输入字节、错误 PID、输出损坏、缺 End、历史重放及未知结果；setup/FIFO 读者在该夹具中受控返回。另在真实私有子进程运行同一完整命令程序，实查 FIFO 屏障、空/二进制 stdin、真实退出和 kill 后 wait；不读取宿主 shell profile、不切换身份、不执行 envd。共享[记录器](../tests/system/sdk_observations.py)继续接受原前台/文件回归。该扩展已接入真实 Go/Rust 双端入口，实机结果须绑定原报告；上述组件回归不能代替进程树、完整 E04 或 KVM 验收。

[多订阅双端片段](../tests/system/sdk_subscriber_compare.py)在同一入口、实例和输入计划上，为 root/ubuntu 分别建立原 Start 与两个同 PID Connect，共四个命令案例；候选配置须容纳每命令至少三个同时存在的订阅。三个 Start 事件均返回后，通过原 FIFO 确认程序已执行，再分别断开原 Start 或一个 Connect，送入既有文本/二进制并关闭 stdin；余下两条订阅须各自获得完整 stdout/stderr 和退出 0/17 的同一终局。输出很短并在 stdin EOF 后产生，等待者顺序消费；本场景不声称测试慢消费者压力或没有订阅时的输出竞争。随后原 PID 的 late Connect 只返回终局，一次性标记仍须为 once。固定 Go 的 [Connect](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/services/process/connect.go)在发送 Start 前分别 Fork Data/End，原 [multiplex](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/services/process/handler/multiplex.go)独立取消订阅；这仅是场景依据，尚无真实服务执行回执。

多订阅报告保留原数字 PID、每次调用、各订阅回调、断开对象和部分失败。比较副本沿用上述显式 PID 引用/同通道分块规则，每案要求一个原 Start、三个 Connect 和四个具名订阅，不归一化输出/异常/输入中的数字。共用原 KnownCommand 在实际 End 到达后即标记结束，后续验证失败不补 kill；未知 Start 不认领，错 PID 的返回订阅只关闭，未知信号/关闭均不重试，原异常和全部收尾失败共同保留。[订阅回归](../tests/compat/test_subscriber_compare.py)使用固定同步/异步 SDK 及受控 RPC，覆盖两种断开位置、共享输出/终局、缺 End、历史重放、错误 PID、相反终局类别、未知 Start、第二次 Connect 失败/取消及多重收尾失败；已有私有程序回归继续执行同一 FIFO/stdin 程序。实际 Go/Rust 的广播与订阅回收、真实四 VM 对照、完整 E03/E04/E05 和 KVM 尚待执行，C02 固定监督调用集未改。

双端编排在每种 SDK 模式内，先完成一侧的全部调用、资源采样和最后身份/环境观察，再确认销毁该实例，随后创建另一侧；后续比较使用保留的原完整记录。已完成的实例不等待另一侧执行而继续消耗原 300 秒寿命，不增大寿命、探针期限或 VM 数量。提前销毁只尝试一次，回执拒绝/未知时保留原身份与 `early_destroy_acknowledged=false`、停止后续创建且整轮失败；已销毁的身份仍参与重复 ID 检查。其余已知实例仍按原路径收尾，比较和收尾的双重错误共同保留，SDK 确认不替代宿主物理审计。[编排回归](../tests/compat/test_guest_compare.py)用显式寿命时钟核对各侧 180 秒工作仍在原 300 秒内完成及销毁，不依赖随机等待；组件通过不表示整轮 KVM 已通过。

双端入口的可选 `--observe-resources` 将[有限资源采样](../tests/system/sdk_resource_observations.py)接到同一四 VM 计划，默认关闭。每台 VM 固定 14 次：调用集之前、root/ubuntu 各两种多订阅场景的 subscribed/detached/terminal 三阶段，以及命令与文件调用集之后；共 56 次观察命令，不增加 VM 或后台采样循环。subscribed 位于三条原 Start 事件及 FIFO ready 之后，detached 位于唯一选定订阅断开之后且尚未送输入，terminal 位于原终局、late Connect 和 once 标记核对之后。每阶段仅调用一次原 SDK root 命令，执行/请求期限均 20 秒，stdout 回调按 4 KiB 拒绝超额、stderr 拒绝非空，还须实际 End 和严格资源回执，不能用完整 JSON 替代 SDK 完成。观察命令摘要、阶段、部分失败、宿主请求等待耗时及原回执放在各 VM 的独立 resources 证据中；原命令/文件比较不丢字段，资源读数不跨 VM 判等，`budget_verdict` 固定为 `not_evaluated`。

阶段乱序、重复、并发或前次失败后继续均在额外 RPC 前拒绝；缺任何阶段不能完成采样。失败/取消不重试，保留未完成阶段及原错误类型，继续原已知命令/订阅与 VM 收尾；已经观察实际 End 后的采样失败不补发 kill，未知 Start 不认领。宿主等待耗时不证明被取消的同步底层调用已经退出。候选需另留观察命令的请求/进程/元数据容量，采样仍消耗原 60 秒多订阅期限和 300 秒实例寿命，不自动增大产品预算或刷新原期限。固定同步/异步 SDK 回环检查核对 14 次准确调用、超额/stderr/错身份/缺 End、失败无重试；受控 RPC 检查三个采样点的真实订阅状态及终局前后收尾，四 VM 编排夹具核对同一身份、默认调用集不变和采样/销毁双重失败保留。该选项的真实 Go/Rust VM 采样按上述组合报告记录；这些有限采样不测试大输出、慢消费者或完整峰值/压力矩阵，也不改变 E05 的通过判据。 多订阅案例同时保留三个普通订阅，并用第四个普通请求读取 FIFO 或采样；后续 List 案例保留三个 Start 和一个 Connect，再发起第五个普通 List 请求。因此完整调用集每 VM 至少需要五个普通名额加原控制保留。共同 registry 夹具默认仍为总数 4、控制保留 1；操作者可在新建测试部署前显式选择 `runtime_transfers_per_vm=6`，全局 8、构建池 4、控制保留 1 及其他物理/期限预算不变。选项仅接受包含控制保留且不超过原全局 8 的整数，不在 429 后自动扩容或改写原配置。默认小容量的拒绝结果与新配置的对照必须分别记录，不能将后者记作原容量通过。

后台双端探针与独立 [PTY 探针](../tests/system/sdk_pty.py)均将“原 SDK 已返回真实终局”与“终局符合案例预期”分开判断：收到 CommandResult 或 CommandExitException 后即记录已结束，再核对成功/非零类别、退出码、error 与输出。实际终局不符合预期仍失败并保留实际结果/诊断，只关闭原订阅，不再补发 kill；缺 End、流截断或等待失败仍属于结果未知，按原已知 PID 进行一次收尾。对应[后台回归](../tests/compat/test_background_compare.py)与[PTY 回归](../tests/compat/test_pty_probe.py)以原固定同步/异步 SDK 注入相反退出类别和错误字段，旧代码分别复现 4/8 个失败分支，修正后全部通过；另检查缺 End 时原收尾仍执行。这是探针失败处理修正，不改变生产进程终止语义、End 所代表的主进程范围或物理资源审计要求。

#504 的后台双端编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38825340099) 通过：源 a216e3b6c8b0f2d214641325151b53a5f6688cf9、checkout 752d92c3057036fa4eb3b9f8ab1a0824e467d0b4；新增 8 项后台回归、固定 SDK 129 项、封存 Redis 与四项 CI 成功。它不包含以下后续 PTY 扩展，也不表示产品 VM 双端验收通过。

[PTY 双端片段](../tests/system/sdk_pty_compare.py)复用同一输入计划和固定 SDK 的原 Pty/CommandHandle，对 root/ubuntu 分别检查直接退出、断线原 PID 重连后退出 17，以及输入等待中的 SIGKILL。通过 root 的原命令查询所选账户 UID；使用同一私有目录、上传程序、短 bootstrap、环境值和全部 byte 载荷，检查实际 PID/UID/cwd/环境/三个 tty 描述符、初始尺寸和 resize；重连案例保留 65536 偏移的 uint16 尺寸窄化检查。结束后原 PID Connect 只返回同一终局，不得重放数据，一次性标记必须仍只有一次执行。每次 SDK 调用、结果/错误与原始 PTY 字节分块均保存，含失败前的部分观察。

双端工作目录统一位于 `/var/tmp`：两个固定测试模板的 `/tmp` 分别可能是内存文件系统与 ext4，目录的真实大小不能直接混比。最小镜像缺失 `/var/tmp` 时，初始化只创建这个测试父目录；不覆盖、替换或改权限修复已有目录。每台新 VM 读取实际 mountinfo 与 statvfs，要求工作父目录确实位于 ext4，并记录挂载点、块/片段大小、名称上限及目录权限；两端和同步/异步模式均须一致。初始化只在本轮 VM 中设置共同主机名 `sandproof-paired`，不修改模板、envd 或系统资源限制；全部案例后再次只读核对同一环境，变化即失败，不修复或重放初始化。目录大小、mtime、完整 PTY 前缀及其主机名仍保留并逐字段比较，不能改写返回值来消除差异。原不同环境的失败证据与统一环境后的验收分别记录。

共用来宾程序新增可选 report_ready：写好一次性标记并进入 raw 模式后，先向 PTY 写固定的 NUL 分隔 ready 标记，再写 FIFO 中的完整身份 JSON；旧独立 PTY 探针不启用此选项，原行为保持。同步 SDK 通过公开迭代接口消费到该字节标记，后续 wait 继续原事件流；异步 SDK 使用原 on_data 回调并等待标记，最长 20 秒。同步读取受原 60 秒流期限限制。只有已观察标记才启动 FIFO 读者，FIFO 身份完整匹配后才断线、调整窗口或发送载荷；不用 sleep 猜测启动时序。所有启动提示与命令回显原字节都保留并精确比较；不忽略未知前缀，不为模板登录配置、hostname 或任意输出差异设置宽松替换。

双端 PTY 在原 Start 后先读取固定来宾 Bash 的提示符终止字节（root 为 `# `，ubuntu 为 `$ `），再发送原 bootstrap；仅收到 PID 不等于 shell 已读取输入。提示符和程序 ready 共用原 20 秒观察期限，原 60 秒 PTY 期限、VM 寿命、参数和输入字节不变。两端所有启动输出、回显和分块仍进入原记录与逐字比较，不删除重复前缀来消除差异。实际私有 PTY 回归用启动前已发生的行规程回显控制交错，核对早写回显两次、等待提示符后一次；固定 SDK 回归核对分块提示符、未就绪不得发送、早期 EOF、期限拒绝及原 PID 收尾。这些组件证据不替代修改后候选的完整实机对照。

PTY 的预定 PID 规则只处理已严格匹配原 Start 的顶层/句柄/订阅 PID、Pty 控制参数、Start/Connect 的 result.pid，以及测试程序 FIFO JSON 和最终二进制帧中 ready.pid。FIFO stdout 必须逐字节等于该上传程序的规范 JSON 加换行；二进制帧必须完整匹配 NUL 头、准确长度、规范 JSON、窗口值及原载荷。验证后仅在比较副本中重建这个确定的 PID 字段及由其位数决定的帧长度，原 JSON/帧字节仍留在证据中；不接受重复 JSON 键、额外空白、替代编码、截断或多余内容。所有其他输入、UID/环境/尺寸、错误文本、startup 前缀和载荷保持精确相等；只合并回调分块。未知 Start/信号/disconnect 不重放，错误 Connect 仅关闭该订阅，失败收尾只控制原 PID，并汇总原错误与所有收尾错误。它不调用全局 List，也不把 End 当成同步列表移除或整棵进程树停止。

[PTY 对照回归](../tests/compat/test_pty_compare.py)通过原固定同步/异步 SDK 和受控 RPC 事件检查相同步骤、不同位数原 PID、启动前缀、分块、窗口、输入、真实 SDK 终局解析、错误重连/ready 身份、坏帧/历史重放、未知回执与部分收尾失败；setup/文件/身份读者在该夹具中受控返回。原[真实私有 PTY 检查](../tests/compat/test_pty_probe.py)另同时运行旧模式和可选 ready 模式的同一上传程序，检查完整字节、规范帧、resize、退出和 kill/wait；不读取宿主 profile、不执行 envd 或用户切换。此扩展已接入双端实机入口，实际结果与上述组件回归分别记录；它们不能代替完整故障/期限/资源矩阵。比较规则的技术复核与精确 KVM 验收仍须独立记录，C02 固定调用集和发布门槛未改。

#505 的 PTY 双端编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38827811936) 通过：源 927b13d8ceb654b3b127db4d575ba151d2180c02、checkout e7de93b1853a5f3d51fbcf863a3db3fc0be7e50d；新增 10 项 PTY 对照回归、原真实私有 PTY 检查、固定 SDK 139 项、封存 Redis与四项 CI 成功。这不包含以下文件扩展，也不表示已执行 Go/Rust 产品 VM。

[文件双端片段](../tests/system/sdk_file_compare.py)沿用同一实例、用户、路径和 mtime 计划，对比 root/ubuntu 的相对路径与 `~/` 主目录、文件归属、未知用户、缺失路径、文件/目录类型错误、不支持的 `~user` 写入和拒绝后的合法调用。私有目录案例明确核对 ubuntu 命令不能遍历 root/0700 目录，但指定 ubuntu 的文件 API 仍可读、列举、覆盖并改变文件归属；新建子目录不改变父目录归属。它检验固定 envd 的文件行为，不承诺来宾用户 DAC 隔离。实际 mtime 在属性检查前用 root 命令设置，路径、权限、归属、大小、时间和 metadata 仍完整比较；未把返回的时间字段直接改成常量。

同一调用集的 multipart 部分失败继续参加 Go/Rust 及同步/异步的严格对照：中间项为已有目录，首项完整保留，后项必须不存在。octet-stream 使用原 SDK 的 bytes 输入及显式模式，每项保留 15 秒 HTTP 请求期限；不用会省略客户端期限的 IO 对象。固定同步 SDK 顺序发送，异步 SDK 并发发送，因此报告单列 `sdk_files`，在每种 SDK 模式内逐项比较 Go/Rust，不能将两个 SDK 的部分失败结果强制相等。输入、原异常类型/文本、最终文件字节及所有属性仍精确比较，原共同调用集仍做跨 SDK 对照。

octet 异常之后不重发整批，只查询原路径。同步确认首项，异步确认首/末两项的完整内容与所选用户归属；最终再读取全部预期文件和错误目录，并核对顺序模式的末项缺失。观察最多 15 秒、128 轮，每次读取使用剩余期限；首次可见后消失、大小回退/超长、非文件、错误字节、超时或观察次数耗尽都不能通过。观察中的调用/结果/错误原样留在 `settlement.calls`，失败保留部分记录。比较前重新验证这份记录的路径、方法、错误类别、单调大小、请求期限、最终归属与完整字节；这些有限只读采样的次数、时点、缺失错误文本和瞬时属性单独留作证据，不作为两台 VM 必须同速完成的相等字段。该排除仅适用于 `settlement.calls`；场景中的错误、输入和最终快照没有此排除。文件状态完整不证明并发 HTTP 请求均已收到回执、请求资源已释放或数据已持久化。

[文件对照回归](../tests/compat/test_file_compare.py)通过固定同步/异步 SDK 的真实回环 HTTP 检查 multipart 原顺序与错误解析、同步 octet 遇错停止，以及屏障控制下异步错误先返回、其他原上传仍待完成。另用受控文件结果和可推进时钟检查消失/回退/错内容/期限、完整证据与比较排除范围；私有目录的错误身份或初始归属必须在覆盖前失败。[入口编排回归](../tests/compat/test_guest_compare.py)检查四台实例复用计划、各模式 Go/Rust 文件结果失配拒绝、未完成文件观察保留及原实例收尾。回环夹具不实现业务文件系统；这些测试不证明来宾权限、真实磁盘行为、生产网关或跨 VM 隔离。该扩展及下述 List 场景共用真实 Go/Rust 双端入口，实机结果按上述组合验收记录；完整故障/资源矩阵、服务重启及 E03/E06/E07 须分别核对，不能从组件通过推断，比较规则的技术复核也须独立记录。

#506 的文件双端编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38831403800) 通过：源 e07d2c0cedc7a91482249f44e69bd1ae3fbfdf5b、checkout f3ddcced9cedb92ca85fe69db27d2b9a494af51e；新增 12 项文件回归、2 项编排回归、固定 SDK 153 项、原 Linux SDK/Rust 组合、封存 Redis 与四项 CI 成功。它不包含后续 List 或终局收尾修正，也不代表 Go/Rust 产品 VM 对照。

[完整 List 双端片段](../tests/system/sdk_list_compare.py)在原实例中同时保持 root/ubuntu 两个普通命令和一个 ubuntu PTY，读取完整列表，增加同一 ubuntu PID 的订阅后再次检查列表没有增加条目。随后依次检查 PTY SIGKILL、root 正常退出、ubuntu 退出 17；每次移除后及保留终局 Connect 后，仍核对全部存活项和全部字段，最后要求空表。记录每次原 Start/Connect 的 PID、输入与回调分块，保留终局不重放输出；一次性文件检查程序仅执行一次。不使用 tag 启动或仲裁，此处 tag 必须保留原 None，不能用空字符串代替；已批准的确定性 tag 规则不改变本片段的无 tag 调用范围。

原程序独占创建案例标记，然后通过 stdout/PTY 发出 NUL 分隔的 ready 字节，再等待一个明确的输入字节。同步 SDK 使用公开迭代接口消费到标记，后续 wait 继续原事件流；异步 SDK 等待原回调，最长 20 秒。PTY bootstrap 保持在 512 字节以内并进入 raw 模式。取得全部 ready 后才观察活跃列表；没有依赖 sleep 判断进程已经启动。原 Start 和活跃 Connect 流期限为 60 秒、保留终局 Connect 为 20 秒、普通请求为 15 秒；所有输出（包括 PTY 启动提示/回显）保持原字节并比较。未知 Start/输入/信号/disconnect 不重发；只向已知原 PID 发失败收尾信号，收到真实 End 后即使退出类别或字段错误也不追加 kill。所有返回的订阅都各尝试一次关闭，原失败和全部收尾错误共同保存。

固定 Go 的 [List](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/services/process/list.go)遍历[原 sync.Map 包装](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/utils/map.go)，没有顺序保证。预定比较规则先检查每个 PID 是唯一原 Start 的正整数引用，整个回复恰好包含全部预期存活项；未知、额外、重复或缺失 PID 均失败，cmd/args/envs/cwd/tag 按固定 SDK 的原公开输入逐字段严格核对。仅比较副本的这些已核实 PID 引用转换为 root/ubuntu/pty 三个逻辑名称，并按逻辑名称排列列表；原顺序与数值仍保存在报告。命令参数、环境中类似 PID 的数字、输出、错误文本和其他字段不搜索替换。初始 cwd 缺省与显式路径、空 env 与 PTY SDK 补入的 TERM/LANG/LC_ALL 均保留。该规则不适用于文件 ListDir 的词法深度优先顺序。

准备/身份命令已经返回终局后，先有限查询至登记表为空，才开始上述受控命令集合；初始观察只作为前置条件，完整记录原条目，不控制未知 PID，也不据空表推断来宾所有进程已停止。固定 Go 的终局缓存和列表移除可能晚于 End，因此每次终局后有限观察原已结束条目消失：其余存活项必须始终完整且元数据正确，只允许这一个已知结束项仍在表中或已移除；观察到移除后的稳定快照和后续查询均拒绝它重新出现。初始及移除观察各最多 15 秒、128 轮，请求使用剩余期限，单份列表观察最多 16 项/128 KiB；这是探针材料边界，不是产品列表或内存上限。原采样保留在 observations.calls，比较前重新核对结构/唯一 PID、有效期限、准确终局边界及移除阶段的全部字段；仅这些前置/移除采样的次数、时点与初始条目不跨 VM 判等。各阶段固定期限的完整快照仍严格比较，不能过滤不匹配条目来通过。

[List 回归](../tests/compat/test_list_compare.py)通过原固定同步/异步 Commands、Pty 和 CommandHandle 消费受控事件与预定 List 回复，验证不同列表顺序/不同 PID、全部元数据和条目、活跃多订阅、延迟移除、额外/重复/缺失项、存活项丢失、结束项重新出现、错误重连、坏 ready/缺 End/历史重放、未知输入和部分关闭失败。可推进时钟另核对期限及次数上限；同一程序在真实私有 pipe/PTY 子进程上检查 ready、等待输入、退出和 kill/wait，不读取宿主 profile、不切换身份、不执行 envd。该片段已接入真实 Go/Rust 双端入口；原固定 SDK 下的全部 List 字段覆盖不等于完整 E03–E07，正式服务重启、tag 和故障/资源场景须分别核对原实机报告。比较规则的技术复核仍须独立记录。

#507 的 List 回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38833503900) 通过：源 117e7e9e7a767b1f18fc7e99a412806b3004f346、checkout 9d133d3ed35412bf30db67fbd38adc0a2233afe2，9 项 List 回归包含在固定 SDK 162 项中。#508 的后台/PTY 终局后不补发 kill 回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38834237283) 通过：源 db570135ee33a391394c17a37aa84c66726e9e58、checkout 59088f9d58a716778ba7bd2606c0df090b717936，固定 SDK 165 项通过。两次封存 Redis 及四项 CI 均成功；均不包含真实产品 VM 或之后的 stdin/期限收尾改动。

[单存活 tag 双端片段](../tests/system/sdk_tag_compare.py)在原四 VM 调用计划中，为 root 的空 tag 和 ubuntu 的特殊字符 tag 各执行两个顺序命令。每个命令先独占写入 once 标记并发出原就绪字节，消费就绪后关闭初始订阅，按 tag 重连、写入空输入和全部 256 个 byte、关闭 stdin，核对 hash/stdout/stderr 与实际 End。随后按 tag 控制须返回原 not_found，保留 Connect 只返回原终局；第二次 Start 复用同一 tag 后，旧 PID Connect 必须返回原 not_found，原标记不能被重放。活跃 List 的全部字段与 tag 存在性仍严格核对，终局移除共用上述有界观察检查；保留 Connect 后再次要求完整空表。固定 Go [Start](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/services/process/start.go) 在新进程启动成功后、登记前清除非 nil tag 的旧终局，空字符串也进入该分支；本片段据此核对成功复用，不推断失败 Start 的清除时序相同。

固定 Commands.run 不暴露 tag，因此该片段通过同一 sandbox 已配置的[原生成 ProcessClient](../tests/system/sdk_tag_rpc.py)发送 tag 字段，仍使用原同步/异步 CommandHandle、原鉴权和传输；不修改 SDK、不重新配置路由或泄露连接头。Start/活跃 Connect 为 60 秒，保留/旧 PID Connect 为 20 秒，一元及初始就绪等待为 15 秒；外层原 VM 寿命不刷新。仅同一时刻一个存活 tag 的成功/保留/复用行为进入 Go/Rust 相等比较，不把已批准的重复存活 tag 拒绝和多个不同 tag 的 Go 遍历差异混为相等。

比较只改写已核实的两次 Start PID、对应句柄/列表引用，以及精确核实的旧 PID not_found 文本；原错误类别、tag、配置、输入、输出及原证据均保留。原 List 前置/移除观察的采样差异按同一检查排除，实际固定快照仍比较；不搜索替换任意数字。未知 Start 不重试；失败收尾只作用于尚未取得真实 End 的已知 PID，收到终局后不补发 kill。预期旧 PID 拒绝却返回句柄时只关闭该订阅，不按其 PID 发信号；原错误和关闭错误共同保留。

[tag 对照回归](../tests/compat/test_tag_compare.py)以原 SDK 句柄和预定事件检查重连、旧 PID/字段错误、未知启动/输入、缺 End、错误退出类别及精确收尾；实际生成客户端经回环 HTTP 核对空/特殊 tag、原 Basic/token、期限和流式/一元错误解析。相同私有程序另在真实 pipe 子进程检查 ready、输入摘要、stderr 与 once。上述回归不执行 envd；该片段在真实 Go/Rust 双端入口中的结果须独立记录，不能表示正式服务重启、完整重复 tag 差异矩阵或 E03/E04/E05 已通过。比较规则的技术复核仍须独立记录。

[已批准 tag 差异片段](../tests/system/sdk_tag_policy.py)接在同一四 VM 计划的命令/文件调用集之后，为 root 和 ubuntu 分别发送相同的三个原生成 Start：两个不同 tag 的命令保持 stdin 打开，再尝试与第一个 tag 重复的第三个命令。每次成功 Start 都先消费实际 ready，完整 List 核对所有存活 PID、配置及 tag 字段。重复 Start 前后分别查询两个 tag：Rust 必须返回对应原 PID，固定 Go 允许精确 not_found 或该 tag 已知的原 PID；有重复项时仅允许该 tag 的两个原 Start 之一，不接受另一 tag 或未知 PID，也不假定各次 sync.Map 遍历次序相同。此片段不通过重试等待某种 Go 次序。

固定 Go 的重复 Start 必须成功并实际执行一次；Rust 必须返回精确 already_exists，之后核对重复程序的独占标记不存在。所有成功启动的命令均通过各自原 PID 关闭 stdin、等待真实 End、核对输出及 once 标记；不按歧义 tag 发送结束/清理操作。终局移除继续使用同一有界完整 List 观察，其他存活项必须保持。未知 Start 不重放；不符预期但成功的 Start 先保留原句柄用于失败收尾，重复返回同一 PID 不产生第二个发信号责任；错误 Connect 回复只保留并关闭订阅。所有公开可观察的关闭/清理错误与原失败共同保留，SDK disconnect 返回不代表服务端订阅或 VM 已物理收尾。

这些已确认差异保存在每台 VM 的独立 `tag_policy` 原始证据中，由各端规则逐步断言，不改写成两端相等。流式和一元原 ConnectError 另保留原 RPC code 与完整 details 字节；预期拒绝同时核对代码、全文和无额外 details。原相等调用集仍逐字段比较。每台 VM 新增两次准备命令、六次带 tag 的 Start 尝试（Go 六次执行，Rust 四次执行及两次拒绝）、八次 tag Connect；不增加 VM、不刷新原 300 秒寿命或改变可选 14 阶段采样计划。Start 原流期限为 60 秒、Connect 为 20 秒，初始 ready 等待及一元请求为 15 秒；同步首帧仍受原 60 秒流期限约束。

[tag 差异回归](../tests/compat/test_tag_policy.py)以预定列表/事件和成功/拒绝回复驱动固定同步/异步 SDK 原句柄，检查两端 Start/Connect 请求逐字相同、Go 漏查/不同重复项选择保留、Rust 错误接受/漏查、未知启动/EOF、错误 PID/元数据、重复副作用、缺 End 及多个清理失败；[双端编排回归](../tests/compat/test_guest_compare.py)核对同一输入和 kind 传递、部分差异证据及已知 VM 收尾。它们不重新实现 Go/Rust 登记算法，也不执行 envd；真实双端 VM 的策略结果在同一入口的 tag_policy 证据中记录。并发 tag 预约和全部故障差异仍需独立验收。

[命令参数双端片段](../tests/system/sdk_argument_compare.py)在同一输入计划上执行前台/后台两组案例。每组分别检查 root/ubuntu 的公开目录、root 私有目录以及指向它们的符号链接；Python 程序核对实际物理 cwd、有效用户、空字符串/引号/中文参数和含空值/换行的环境变量，stdout/stderr 与完整终局逐字段比较。两组各有 13 类拒绝：未知账户及未知账户伴随缺失 cwd、root/ubuntu 的缺失路径/悬空链接/普通文件/非目录父路径，以及 ubuntu 无权进入的目录/链接/私有目录下的缺失子项。每次拒绝必须是指定 SDK 异常类型，保留并比较原错误全文；随后检查该次唯一标记不存在，再执行合法 ubuntu 命令。准备时确认账户不存在，读取目录/文件归属、权限和固定实际 mtime，最后核对普通文件内容未变。

每台 VM 的该片段包含 66 次原 Start：两次准备、12 次合法上下文、26 次拒绝及 26 次拒绝后的健康调用；正常路径得到六个后台句柄。命令及其请求期限显式为 15 秒，准备沿用原命令默认请求期限，VM 寿命仍由外层限定。后台比较仅将已验证的原 Start PID、result.pid 及 wait/disconnect 的 handle_pid 转成同一逻辑引用；只合并同通道回调分块，原数字和分块仍保存。命令、参数、环境、输出及错误中类似 PID 的文本均不得替换。预期拒绝却成功返回的后台句柄先纳入所有权，再按原 PID 尝试一次失败收尾；未知 Start 不补造 PID、不重发，已经取得公开终局不追加 kill。原失败和全部收尾失败共同保留。

[参数对照回归](../tests/compat/test_argument_compare.py)用固定同步/异步 Commands 和真实公开句柄消费预定 RPC 事件，覆盖完整调用矩阵、精确输入与回调、异常全文、严格 PID 引用、非法 PID、缺 End、相反结果及多个收尾失败；文件属性和查询回复由受控操作提供，不实现业务文件系统。另在私有 Python 子进程执行同一上下文程序，检查参数、环境和符号链接物理 cwd，不切换身份或读取宿主 shell profile。该片段的真实 Go/Rust VM 结果由双端入口独立记录；上述受控回归不代表 Linux 账户/目录权限、完整畸形参数矩阵、重复 tag 政策或 E03 验收通过，C02 固定调用集保持原样。

#513 的参数双端回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38840358332) 通过：源 59ad1cd9a64592170d4aa44c09c9d1af175da7e5、checkout 81fd6689002e2482c7f421824e6f966201ec387c，新增 9 项包含在固定 SDK 204 项中，原 Linux SDK/Rust 组合、封存 Redis 及四项 CI 成功。它不包含下述独立参数探针的收尾修正，也不代表真实 Go/Rust 产品 VM 对照。

[stdin](../tests/system/sdk_command_input.py)、[命令参数](../tests/system/sdk_command_arguments.py)和[文件权限/部分批次](../tests/system/sdk_file_permissions.py)探针也接受上述同一组候选参数：同步/异步模式均在原调用集前后核对身份，文件探针分别核对两台实例，报告将每份观察绑定到原 sandbox_id；原调用输入和 VM 寿命保持不变。已知实例按逆序各尝试一次销毁，某台拒绝、报错或取消不能跳过另一台；全部收尾异常连同原调用异常保留，创建结果未知不重试。[受控编排回归](../tests/compat/test_candidate_probes.py)覆盖这些顺序、身份变化拒绝和部分失败；它替换 SDK 操作及案例函数，不执行真实案例、主机 canary 或产品 VM。SDK 销毁确认仍须由获准的实机监督器补充精确资源审计。

文件权限探针在失败时也输出有限报告：逐模式保留已完成案例组、失败阶段和错误类别，创建前登记原尝试序号，只有原 create 返回才补充 sandbox_id；未知结果不会因已知实例的销毁确认而消失。未完成案例组不计通过，原调用和全部清理错误共同保留。组合监督入口在探针进程失败后、恢复和清理前，经原宿主固定 TLS 来源做一次中心状态观察，并记录原服务 cgroup；最多使用原 900 秒探针期限内剩余的 5 秒，不扩展期限、不重试创建或停止服务。私有诊断记录观察时间和观察失败类别；它是失败后的状态，不代表请求发生时的状态或新的准入判据，观察失败也不能覆盖原探针失败或跳过清理。[监督回归](../tests/compat/test_guest_faults.py)覆盖一次观察、期限已尽、观察失败和报告写入失败。

#497 的候选探针编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38793635811) 通过：源 586a8e4fb0473c813d8baec9601a548be8e353d6、checkout 4140d26ad2780573401422311406b8177185af93；新增 9 项编排包含在固定 SDK 98 项中，封存 Redis及四项 CI 成功。此处仍为受控 SDK 操作，不是三组案例在真实候选 VM 上的执行回执。

独立[命令参数探针](../tests/system/sdk_command_arguments.py)的合法后台上下文和预期拒绝却成功返回的句柄同样使用 KnownCommand：先验证原 PID，等待缺 End/超时/取消时只向原已知 PID 尝试一次信号，信号失败也继续关闭订阅；非法 PID 仅关闭返回订阅。公开终局已经返回后不补发 kill，并严格核对 stdout/stderr/exit_code/error。结果验证在所有权范围内完成，原失败、信号失败及关闭失败共同保留；Start 结果未知仍不重试。文件存在性和最终内容调用先取得结果再断言，原 Start/文件输入、案例顺序和 12 项报告保持。[原 SDK 收尾回归](../tests/compat/test_argument_probe_cleanup.py)覆盖完整 30 次 Start 编排、原同步/异步句柄、未知 Start、非法 PID、缺 End、期限错误、受控 wait 取消及多个失败；它不执行 envd、真实 Linux 用户权限或产品 VM，不能代替真实资源清理验收。

#514 的独立参数收尾回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38841612428) 通过：源 04a7d70c3528ba9ac36740409dc59ab121a218f4、checkout 50c749c8ebcd2aa6731d9196418fc657aef29102，新增 8 项包含在固定 SDK 212 项中，原 Linux SDK/Rust 组合、封存 Redis 及四项 CI 成功。它不包含下述文件期限证据改动，也不代表真实产品 VM。

[命令流期限](../tests/system/sdk_command_timeouts.py)、[文件请求/空闲期限](../tests/system/sdk_file_timeouts.py)与[一元 RPC 期限](../tests/system/sdk_rpc_timeouts.py)同样接受上述候选参数，覆盖原 run/connect/wait、文件传输和已选命令/文件一元操作矩阵。身份观察在整组案例前后执行，使用自己的 20 秒命令期限；原案例参数、未知结果观察、无重放及 300 秒 VM 寿命保持。可选报告绑定原 sandbox_id 和前后相同的运行身份，默认报告保持原字段。[同一编排回归](../tests/compat/test_candidate_probes.py)还核对期限参数不被观察预算替换、文件初始写入晚于身份核对，以及案例失败与销毁失败同时保留；这些受控操作不构成真实超时/终止或传输验收。

#499 的期限探针编排已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38797587711) 通过：源 7795e782440e731aea50276aa82292ef240d82aa、checkout 7b937bfb8017a92dc84693e0464994dd620647a5；固定 SDK 100 项、封存 Redis 与四项 CI 成功。该结果仍不包含真实候选 VM 的期限矩阵。

文件期限探针只读取自己的固定 69,000 字节内容或未知上传后的前缀；其流式收集器逐块检查原字节前缀和长度，超长/损坏立即失败并关闭原读取器，不先累计任意正文到 EOF。文本/bytes 模式仍由原 SDK 缓冲，返回后才检查；这个探针内容边界不是产品文件上限、SDK 内部缓冲或实际 RSS 保证。read_observation 记录公开返回的 public_bytes/public_sha256 和公开完整返回的 public_eof，超时不把前缀提升为完整结果；零公开字节不表示网络未接收数据。未知上传的单次只读观察另保留 observed_sha256，与原 observed_lengths 区分当次不存在、空文件、部分或完整内容；这些不是写入停止、持久化或回滚回执，不重发写入。成功批量回复必须保持原路径顺序，不能只比较集合。

探针自身的完整性失败与读取器关闭失败共同保留；固定 SDK 在处理传输错误/取消时若又被关闭异常覆盖，只有 ReadTimeout、TimeoutError 原因以及没有另一原始失败的异常链才可归为预期期限。其他失败不能借关闭超时通过，失败后不继续健康调用。原请求参数、上传源类型/编码、负值期限分类、VM 寿命和候选身份观察保持。[文件期限回归](../tests/compat/test_file_timeout_evidence.py)让原固定同步/异步 Filesystem、HTTP 序列化和公开流读取器消费预定 HTTP/RPC 回复，覆盖每种客户端 16 组读取和 64 组上传参数、原 timeout 选择、流关闭、前缀/超长/截断、异常链、取消、乱序回复及上传源关闭。HTTP 回复与时限错误由受控传输提供，未验证 pyqwest 的真实计时、服务文件效果或网络；异步负 idle 参数仍由原 SDK 的 wait_for 拒绝。它不执行 envd、不构造业务文件系统，也不代替真实 VM 文件期限、E03/E05 或实际资源峰值验收。

#516 的文件期限证据回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38843146836) 通过：源 b98a44c51dd1b0935553f92daa6c94f0fabd5756、checkout 1b24f70a0327158129f9820e0ae3097c12c51e13，新增 10 项包含在固定 SDK 222 项中，原 Linux SDK/Rust 组合、封存 Redis 及四项 CI 成功。该回执不包含上述新增 keepalive 选择组件或真实 VM 文件期限矩阵。

stdin 和命令期限探针共用原 Start 身份的收尾记录：只有已返回并通过正整数范围检查的原 PID 可以收到单次失败收尾信号；每个返回的订阅各尝试一次 disconnect，未知关闭回执不重试。错误 Connect 的订阅仍关闭，但其 PID 不用于控制；原异常、信号异常和全部关闭异常共同保留。原 wait 返回 CommandResult 或抛出 CommandExitException 即记录已收到终局，后续错误类别、完整字段或标记读取失败仍判失败，但不再补发 kill。缺 End 或流期限失败不伪造进程已结束。默认成功结果也要求 error 为 None；原 timeout/request_timeout 输入和 SDK 错误映射保持，进程期限与流期限的竞态按下述分别记录。

命令期限 Start 未返回 PID 时，只读取一次 List，记录 cmd/args/envs/cwd/tag 全部精确匹配原提交的候选条目；多个匹配也不代表取得 Start 身份，不根据标记子串或匹配结果连接、发送信号或重试 Start。报告新增 terminal_observed、original_pid、unacknowledged_candidates；原实例仍由外层销毁并等待实机资源审计。已收到命令期限终局后，最多 15 秒/128 轮观察原 PID 从 List 移除，每次查询使用剩余期限，尚未移除时必须保留原元数据；不要求 End 后瞬时消失。单份列表最多 128 项/128 KiB，拒绝重复或非法 PID；这些是探针观察边界，不是产品容量保证。post_end_list_observations 保留原 PID 的各次观察，不据它证明整张表或进程树已空。

[收尾回归](../tests/compat/test_command_probe_cleanup.py)通过固定同步/异步 SDK 的原 Commands 和句柄消费预定 RPC 事件，覆盖 stdin 默认/二进制/重连、成功及错误终局、缺 End、标记失败、期限恢复、延迟移除、未知 Start 候选与错误 Connect；受控句柄故障补充重复引用、无效 PID、取消及所有收尾错误保留，可推进时钟核对截止时点和次数上限。不实现另一套来宾注册表，不运行产品 VM，也不把本地通过计为真实期限或销毁验收。此改动不涉及 C02 原固定 sdk_data_contract 调用集。

#509 的收尾回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38835915317) 通过：源 dfe563a64202aad0ece5a41832c14ced87f760cc、checkout 6b1c74e81ed789b5fd331812342fe1fcd7e9a42b，新增 14 项回归包含在固定 SDK 179 项中，封存 Redis 及四项 CI 成功。该证据不包含下面新增的期限结果修正或真实 VM。

固定 Go 的 [Start](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/services/process/start.go)从同一 Connect 头另建进程上下文，其期限与 RPC 流上下文分别管理；[Handler](https://github.com/e2b-dev/runtime/blob/f455fd58fa3f289e78ce1de690c35968d8d29322/packages/envd/internal/services/process/handler/handler.go)通过真实 exec.CommandContext/wait 产生终局。固定 SDK 把负 timeout 量化为 1 ms。run 的该案例保持 stdin 开启且程序等待输入，只接受原 DEADLINE_EXCEEDED 映射，或 CommandExitException 的准确四字段 `exit_code=-1, stdout='', stderr='', error='signal: killed'`；意外成功、其他退出码/信号/输出仍失败。expected_outcomes 记录两种预期，outcome 记录本次公开调用实际返回的类别，terminal_result 保留原 SDK 公开终局字段；不把两种结果归一化为同一观察。Connect 的新流期限不会改变原执行期限，其案例仍不能把进程死亡作为预期超时。

wait 记录 initial_wait_outcome，再通过同一原 PID 的恢复订阅验证后续终局。执行期限可能在用户程序写入一次性标记之前到期，所以 Start 期限案例允许观察到 0 或 1 次标记，报告实际计数；已有标记仍必须完整且恰好一次。Connect 期限后主动关闭 stdin 并成功完成的案例仍要求标记为 1。若 SDK 在消费 End 后又收到 RPC 流错误，两个固定客户端均优先抛出流错误；探针按这个公开失败记录，不读取 SDK 私有缓存来改判成功。[期限结果回归](../tests/compat/test_command_deadline_outcomes.py)用预定 RPC 事件控制两种结果、尾随错误和标记时点，核对原同步/异步 SDK、原 1 ms/1 s/60 s 参数、同 PID 恢复、全部终局字段和单次收尾。受控事件不证明真实 Go/Rust 的竞态频率或执行期限精度；真实 VM 矩阵以相应期限探针的原报告绑定精确候选，不能由这些受控事件回归代替。

#511 的期限结果回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38836815182) 通过：源 36b6864f95679238c9368f9248042fe9e521b5ef、checkout 58951e4dc42d501721908a67b39d392734fbe9a8，新增 7 项包含在固定 SDK 186 项中，封存 Redis 及四项 CI 成功。该证据不包含下述一元命令收尾改动或真实 VM 期限矩阵。

一元命令期限探针同样使用原 Start 的收尾记录。被测试的 kill 在调用前记为已尝试；成功、失败、取消、无确认或超长头拒绝后都不再补发第二次 kill。未知 CloseStdin 不重发，必要时只对原 PID 尝试一次独立的收尾信号；收到公开 wait 终局后不再发信号。每个返回订阅各关闭一次，原错误及信号/关闭错误共同保留；仅案例与收尾成功后运行健康命令，避免后续健康失败覆盖原错误。原 VM 销毁和实机资源审计仍承担最终停止责任，不将未知 kill 或信号确认记作进程退出。

对超长头明确拒绝的 kill/CloseStdin，不能只凭 List 条目证明原命令仍可用：探针向原 stdin 发送一次独立输入，再正常关闭并核对原流的完整十六进制输出及终局。该恢复只用于已确认拒绝，未知调用不重放。其他输入案例也严格核对完整 stdout/stderr/exit_code/error，不能用 strip/fromhex 忽略额外空白或重复输入；未知输入仅允许原输入完整出现一次或未出现。报告将旧 observed_live 改名为 observed_listed，明确只是列表成员观察；新增 original_pid、terminal_observed、signal_attempt_source 和 rejection_recovered，分别记录原身份、公开终局、信号尝试来源与拒绝后恢复结果。首个被测 RPC 的参数保持不变，后续 List 和收尾控制请求显式使用 15 秒期限；wait 仍继承原 Start 的 60 秒期限。

[一元命令回归](../tests/compat/test_unary_command_probe.py)使用固定同步/异步 SDK Commands 和句柄，按预定 RPC 回复并在明确控制调用后放行 End，覆盖全部六类命令 API 与四类期限参数、负值成功/未知的两种结果、旧 List 条目、缺 End、未知 Start、错误终局、错误十六进制输出、取消、false 信号回执以及原错误和收尾错误同时保留。同一等待输入程序在私有 Python 子进程验证空/二进制输入的准确输出。它不实现来宾注册表，不执行 envd、真实期限竞争或产品 VM；一元文件案例和 C02 固定调用集不在此修改范围。

#512 的一元命令收尾回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38838312267) 通过：源 c9c7f61b652ea91dd4bb19ee7bf15e2797a23306、checkout b27803302ae6a5be4b747208585a79cfbdf58fb6，新增 9 项包含在固定 SDK 195 项中，原 Linux SDK/Rust 组合、封存 Redis 及四项 CI 成功。该证据不包含上述新增的参数双端片段或真实产品 VM。

[PTY 候选入口](../tests/system/sdk_pty.py)通过原固定 SDK 的公开 `pty.create/connect/send_stdin/resize/kill` 和同步/异步句柄，对 root、ubuntu 各执行直接退出、断线重连后 code 17、等待输入时 SIGKILL 三种案例。使用 SDK 原 `/bin/bash -i -l`，在私有目录写入有限 Python 程序，通过 FIFO 就绪屏障核对原 PID、UID、cwd、环境及三个终端描述符；之后发送空输入及分块的全部 256 种字节，验证 2304 字节原样合并输出、初始/调整窗口、超出 uint16 的尺寸收窄、准确终局、立即晚连只返回 End、单次启动标记及存活时 List 包含原 PID。结束后的 List 状态只记录观察值：固定 Go 先广播 End 再调用终局保存/删除钩子，不能额外承诺 End 后立即查不到 PID。交互启动命令小于 512 字节，输出上限 128 KiB、元数据上限 4 KiB；登录 shell 的前缀单独记录长度/摘要，不当作固定二进制帧，也不擅自归一化为 Go/Rust 完整对照。原 shell profile 须保留请求的 cwd/自定义环境；模板还须具备 `/usr/bin/python3`。每个 PTY 60 秒、晚连 20 秒，VM 300 秒，不延长寿命以掩盖超时。

该入口采用同一候选参数和前后运行身份观察，Start 结果未知不重试；错误 PID 的连接只关闭该订阅，失败收尾只尝试原已知 PID 的信号，信号结果未知不重放，随后关闭全部已知句柄并销毁原 VM，保留全部收尾错误和原异常。它已接入上述组合验收，产品 VM 的通过结论须来自对应候选的原报告。[本地回归](../tests/compat/test_pty_probe.py)分别运行真实私有 PTY 上的原上传程序/短命令，以及受控 RPC 事件下的原固定 SDK Pty/CommandHandle，核对二进制损坏、截断/多余字节、历史重放、错误重连 PID、未知 Start/信号和部分收尾失败。私有 PTY 不读取宿主 shell profile、不切换身份、不操作宿主终端；受控 RPC 不执行 envd。它们不能代替真实登录配置、Linux 身份切换、网关/来宾服务、进程树收尾、完整 PTY 故障/期限矩阵、双端 E03 或 KVM 验收，也未加入 C02 固定监督调用集。

#500 的 PTY 探针回归已在 [Linux CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/38800712959) 通过：源 359ced1c8edfa474057f518eb8aaf07654edf76d、checkout d5f12c63110c46fca05ee094bd6f500b28d0d2d7。新增九项回归包含真实私有 PTY 程序和原 SDK 的受控 RPC 调用，固定 SDK 共 109 项、封存 Redis和四项 CI 成功；不包含产品 VM 上的 PTY 探针或此后的阻塞准备改动。

| 实机入口 | 必查内容 |
|---|---|
| [C02 固定监督](../tests/system/sdk_c02_acceptance.py) | 批准的精确提交、中心/宿主双产物与固定 Redis/TLS 清单、宿主/探针配置；单轮私有身份、实际角色进程回执、失败收尾和原资源审计 |
| [基础 SDK 服务](../tests/system/sdk_server_smoke.py)、[查询列表](../tests/system/sdk_list_queries.py)、[详情](../tests/system/sdk_sandbox_details.py) | create/connect/get/list/kill、字段默认值及归属；真实启停，不以接口成功冒充资源消失 |
| [寿命参数](../tests/system/sdk_lifetime_values.py)、[运行起点](../tests/system/sdk_running_start.py) | 同步/异步默认与显式参数、延长/缩短、总上限、到期及同库重启 |
| [进程崩溃](../tests/system/server_process_crash.py)、[未就绪中止](../tests/system/server_unready_abort.py)、[启动恢复隔离](../tests/system/server_prepare_recovery_isolation.py) | 宿主动作前后退出、回执丢失、恢复同一 VM、单实例故障不阻塞其他实例 |
| [私有系统盘](../tests/system/sdk_private_root.py)、[文件权限](../tests/system/sdk_file_permissions.py) | root/普通用户、两实例隔离、基础镜像不变、失败初始化不污染新实例 |
| [公网规则](../tests/system/sdk_public_network.py)、[网络回执崩溃](../tests/system/sdk_network_receipt_crash.py) | 禁网/公网/静态 IPv4 的真实 TCP/UDP；宿主管理、私有目标及其他沙箱拒绝；丢回执恢复原规则 |
| [网关慢流](../tests/system/gateway_slow_boundaries.py)、[上传中断](../tests/system/transfer_interruption.py) | 命令/文件超时、断线、大输出、迟到拒绝响应、撤销后中继停止并回收 |
| [磁盘预留与耗尽](../tests/system/server_disk_reserve.py)、[物理文件系统组件](../sandbox-server/src/storage.rs)、[固定物理配置](../sandbox-node/src/physical_profile.rs)、[有界轮转](../tests/system/server_bounded_churn.py) | 空间不足仍能收尾、同文件系统要求与跨文件系统拒绝、部署 profile 与真实字节、位置复用及旧动作拒绝；组件通过不代替新入口的实机压力验收 |

网络测试须按当前合同报告 IPv6 与 DNS 边界，不把来宾配置或仅拦 TCP/53 当作完整 DNS 防绕过。静态规则使用合同中的部署配置，不虚构固定 SDK 没有的创建字段。DNS 域名过滤/严格转发控制等未交付能力不能计入通过。

C02 在交给中心/宿主装配前显式声明原启动期限 300 秒和禁用公网，保持原 NativeSettings 缺省行为；装配发生在 Rust 反序列化之前，不能依赖它补字段。执行时必须提供 `--cgroup-parent` 指向批准的空且已委派 VM cgroup 父组，宿主内存上限沿用实际模板的 `host_memory_bytes`；服务进程仍使用独立的 `--service-cgroup-record`。[准备回归](../tests/compat/test_c02_preparation.py)把原 C02 池配置交给实际共享渲染器，核对两端收到同一配置及模板对应的宿主资源声明；只构造私有文件，不启动服务或 VM。

C02 的公开租户与宿主本地 owner 分别核对：公开身份来自原中心审计，本地 sandbox/owner 来自同一完整创建身份的原宿主 inbox。保存这份映射后，SQL 状态、退休证书和物理资源回执必须匹配各自的本地 owner；不能将公开租户编号当成本地 owner，也不能从公开 sandbox 编号推导本地编号。[监督回归](../tools/tests/test_system_acceptance.py)覆盖不同本地 owner 的活动/退休记录及错租户、错 owner 拒绝；原 SDK 调用集和 5 秒到期场景保持不变。

C02 原始目录含凭据、配置及完整日志，只在私有目录保存；`--export-run` 输出有限脱敏摘要。安全导出须确认原执行结束/锁状态，不因丢失结果补造通过。监督器无法确认收尾时保留未知占用和现场；禁止全局杀 Firecracker、宽泛删除 jail/网络/目录。清理后核对原 PID/启动时间、目录身份、namespace、cgroup、磁盘及源镜像摘要。

## 4. 通用模板与 Terminal-Bench

当前使用[冻结的 2.0 兼容回归](../benchmarks/terminal-bench-2.0-suites.json)，题集与批次大小由配置选择，使用通用模板链，不按题目增加产品逻辑。已登记解答修补只在评测层显式启用、核对原脚本摘要并单列结果，原评分/资源/时限不变。2.1 原题全部 89 题为发布前待办，其完整锁和[配置](../benchmarks/terminal-bench-2.1-suites.json)保留；不把旧成绩或修补版结果合并为原题通过。[评测入口](../benchmarks/README.md)维护版本锁与命令。

- 先核对固定 Harbor 与 SDK 联合依赖，再固定全题集 ID、任务文件、环境/原解答/原评分和资源/阶段时限。
- 用户 Dockerfile/公开 OCI → 通用构建转换 → 不可变 Ready 模板 → 全新沙箱 → 原初始化 → 标准解答 → 原评分 → 销毁与精确残留检查。不得为某个题目改产品行为。
- 产品功能以[自助模板链](../tests/system/template_self_service.py)、[公开 OCI](../tests/system/server_public_oci.py)和[通用题目监督](../tests/system/harbor_generic_chain.py)验收；选定真实 Dockerfile 多阶段、ENV/WORKDIR/USER、CMD/ENTRYPOINT、错误产物和发布/清理故障也须覆盖。
- 原题资源与阶段时限不能静默放宽；资源不足、网络外部依赖、原解答/评分问题和系统缺陷分别归因。初始化/标准解答退出 0 不足以通过，还须原评分 reward=1。
- 所有固定任务均进入报告，不删失败或未运行题。版本、原始/本地修订模式、环境变体和跨提交结果分开；修订必须列出改动与原因。
- 正式题目与构建服务在同一宿主配置下核对资源争用、同租户/跨租户容量拒绝及收尾；普通串行实跑不表示满载压力通过。
- 先以少量诊断任务打通完整链路，再跑固定全集；不要求接付费模型。每次重构重跑受影响合同与系统场景，阶段晋升按开发规则执行。

本阶段先验收少量 2.0 兼容题与真实自动检查，全部兼容题和 2.1 原题全量未运行不计完成。人工诊断可按 owner 要求跳过外部网络阻塞并在独立身份中换题，不分析或修复网络；原非成功状态、实际范围和精确清理仍由自动报告保留。固定自动回归不动态换题，不能靠跳过得到正式绿灯。特殊单次晋升豁免只记录于对应 PR。

稳定晋升配置：2.0 `ci` 配置两题 polyglot-c-py、db-wal-recovery；原解答和原测试断言保留，只在独立派生任务中用预装 Python/pytest 替换联网评分启动。使用不可变干净模板、全新沙箱和私有工作盘，禁用公网；不复用解答后的磁盘或旧成绩。结果为 `prepared-ci` 环境及 `ci-preinstalled-v1` 评分启动变体，不是原环境、原评分启动或通用模板构建的验收。鉴权、故障恢复、流式边界仍由相关定向测试检查；构建/镜像/平台安装工具改变时补通用构建实机测试并刷新受影响模板。完整 2.1 原入口 89 题在网络合格主机上作发布前验收，不阻塞每次晋升。

当前检查为 `E2E / terminal-bench-ci`，专用 App 已接通，两级晋升的实际规则见[开发指南](../CONTRIBUTING.md#5-github-配置)。部署/更换时须在同一精确提交上实测两题成功、真实评分失败及随后干净成功，绑定模板/评分依赖清单摘要、候选二进制和基线，独立核对清理；缺包不回源、不跳题、不伪造通过。证据留 #14，不在本页维护另一份运行进度。2.0 smoke 四题、十六题及全量入口保留供显式扩展验收，不混用预装和通用构建报告。

### 配置题集自动验收判据

- 每批大小由批准配置确定，题内串行；使用锁定的 OCI 或原 Dockerfile/context 经通用模板链准备。2.0 的原解答/已登记修补模式固定且分批隔离；2.1 只能使用原解答，不接付费模型，不修改原评分/资源/时限。
- 每题评分 reward=1，且原沙箱、构建、服务、代理的进程/网络/cgroup/jail/磁盘均精确收尾。少题、非零退出、超时、失败或清理未知均不得绿灯；检查名称独立为 `E2E / terminal-bench-2.0-compat` 与 `E2E / terminal-bench-2.1`，手动覆盖另加 `/ diagnostic`。仅配置全部选中题通过才成功；完整锁全部通过才记全题集，兼容版不能称原题全部通过。
- 候选/基线提交、配置、工具及任务逐文件摘要固定；每批报告记录可启动模板摘要。分批汇总结果先持久保存，再回收该批已确认的私有产物。
- 普通 CI 与 KVM 共享准入锁；先暂停新准入，等待全部原任务退出才独占，不能取消已有任务来冒充空闲。只有所有验收资源确认停止后才释放自己的标记。
- 准备与批次使用独立 systemd 单元；管理服务重启接管原身份，不盲目重复提交。启动回执未知且单元无法认出、清理未知时保留原占用，发布失败并要求核对；回写终态丢失只重试同一检查，不重新跑 VM。
- 实测真实成功、失败和随后干净恢复，并验证原单元接管、管理凭据不进入准备/批次/来宾、外来资源不被清理。组件故障夹具不能代替这些宿主验收。
- App 授权、受控安装和两级晋升门槛部署以对应 issue/PR 实测为准；保护要求专用 App 的精确检查名称，手动 `/ diagnostic` 或其他 App 同名检查不能代替。未返回成功不得晋升，不配置空绿色工作流。安装、私有配置与安全边界见[CI 运行环境](ci.md)。

## 5. 中心与宿主既有工程验收

本节定义 #292 的必需验收；下方区分已接入的组件检查和仍需实测的完整链路，脚本存在不表示场景已通过。合同依据见[中心与宿主高级操作](module-contracts.md#中心与宿主的高级操作合同)，性质与外部条件见[验证设计](formal-verification-design.md)。首先在同一物理 KVM 主机运行中心、Redis 和一个真实节点，各自独立进程/账号/目录，经版本化双向认证 HTTPS 通信；不使用同进程调用、免鉴权或免持久化替身冒充真实闭环。多台真实宿主另立后续任务，关单范围见蓝图。

普通隔离 CI 已在现有 SDK 合同任务内接入 [check-distributed-contract.sh](../scripts/check-distributed-contract.sh)，使用固定真实 Redis、专用回环数据目录和受控节点/时钟/证书运行下列组件检查；不增加任务检查名称，不访问 KVM 或生产凭据，Redis 不能全由假库替代。脚本包含共享身份、真实 TLS、Redis/SQLite、中心服务与唯一产品入口的定向检查，并明确输出未验收项；固定依赖制备与普通执行环境见[CI 运行环境](ci.md)。这些组件检查不能代替下列 DK 故障组和两题的候选实机验收。

KVM 继续使用可信外部监督器，候选同一提交离线构建 `sandproof-center` 与 `sandproof-host`，实际核对两个 `contract-versions`，绑定二进制/协议/业务信封/宿主格式、固定 Redis、证书配置、模板和完整宿主预算摘要。每轮独立 Redis/SQLite/登记/端口/实例身份；固定受保护测试证书须核对摘要、双方用途与名称，不能关闭证书验证。中心与 Redis 分别为不同非 root 身份，宿主为 root；SDK/Harbor 只指向中心，经真实双向认证宿主启动运行。下列 DK 故障组在两题前执行；只有故障组、两题原定判据、精确收尾与报告均通过才发布 `E2E / terminal-bench-ci` 成功。工具合同源码和自测不代表整条链已安装/验收；新监督链完成真实正负例及取消收尾前，当前已安装门槛保持，不以旧链绿灯替代新架构结果。

两层预算计入原受控准入/独占，不取消他人任务或清空全局资源。故障/缺材料/超时/未知与未运行不能绿灯；结果留 CI 附件和 issue，不每次回写文档。

### 已实现的通信基础检查

独立通信基础的组件入口为 `bash scripts/check-distributed-contract.sh`：同一脚本运行 `cargo test --locked -p sandproof-protocol --test internal_contract` 和 `cargo test --locked -p sandproof-internal --all-targets`，对应测试也随工作区基础检查执行。它们使用独立回环端口和临时测试 CA/证书（仅测试依赖 rcgen 0.14.10），实际交换 TLS/HTTP 字节，不使用生产凭据；核对完整身份/计数器拒绝、双向证书链与登记摘要、证书名/用途、只读路由、两端头/正文边界（含错误响应和分块超额）、握手/完整请求期限、超额即拒及调用方取消后的原连接回收。定向运行不代替开发指南的完整基础检查，实际结果绑定 PR 提交。

本段传输测试只覆盖 DC02 的认证/身份基础与传输预算；高级操作、持久映射和真实 Redis 的组合检查见下一节，不表示 DC02 整体、S1 或任何 DK 用例通过。只读探测也不作为节点健康/可接单证据；双服务实机部署及新 KVM 监督链的正例、负例与取消收尾仍须独立验收。

### 已实现的存储基础检查

同一组件脚本还执行 `sandbox-center-store` 的默认检查和 `redis-tests` 真实引擎用例。运行前把输入锁中的官方 Redis 归档下载至独立临时目录、核对 SHA-256，再编译 `make MALLOC=libc BUILD_TLS=no`；通过 `SANDPROOF_REDIS_SERVER` 提供该构建的绝对可执行路径。版本不符或缺失立即失败，不自动联网安装、使用系统版本、mock 或 skip。此测试 feature 只控制夹具，不提供产品运行模式。测试创建私有目录、独立回环端口和非生产凭据，仅停止/回收本轮子进程及目录。

存储用例覆盖单块 CAS/旧修订冲突、原字节替换拒绝、缺账本不初始化、格式/部署/TTL/损坏/超额拒绝、超过 Lua 整数精度的修订及耗尽、必需持久化配置、实际 noeviction 拒绝、访问凭据/写者身份、引擎卡住时的期限、Redis 崩溃恢复、截断 AOF 拒绝且不修复，以及实际提交后丢失回执时的围栏和耐久重确认。单独的存储测试不宣称无半额度/无重复 VM 的组合结论；实际结果绑定 PR 提交。

同一脚本也运行中心纯决策与原 SQLite 持久收件：多节点确定性负载选择、租户/平台额度、健康/物理空闲/模板否决、未知保留绑定、销毁超越迟到就绪、停止不释额、固定寿命、续期与销毁槽边界；收件包含全文/摘要去重、错身份/窗口/非规范化参数、销毁先到、提交前回滚/后重启以及 SQLite 提交结果未知的两种耐久结果。真实 Redis 与真实 SQLite 既使用明确标注的组件传输替身，也通过真实双向认证 HTTPS 组合，验证提交→异步派发→持久收件、写库后丢回执保留原身份/额度、原请求重放与迟到隔离；HTTPS/SQLite 另验证重开原库及 HTTP 取消不提前返还实际阻塞工作名额。它们不是已部署双服务或 KVM 验收。DC01–DC06/DC08–DC09 的组件覆盖不是整条链路完成；唯一产品入口、本地执行映射、确认/退休与容量仍须在同一候选上通过对应实机矩阵，全部 DK 和监督器受控安装验收不得省略。

### 5.1 组件用例（DC01–DC11）

最小真实创建／销毁的组件证据可通过[独立实机入口](../tests/system/internal_lifecycle_smoke.py)手动验收：普通用户以 `cargo build --release --locked -p sandproof-center-store --example kvm_lifecycle` 构建[组件装配](../sandbox-center-store/examples/kvm_lifecycle.rs)，配合同源码 `sandproof-host relay`；保护精确提交/固定产物后按 `--help` 提供批准配置。它沿用原独占、真实 Redis/mTLS/SQLite/VM 及精确资源审计，不下载镜像、不接付费模型、不改全局网络；仅为组件装配，不是另一生产入口或旧公开服务降级。

正式 SDK、Harbor 与晋升必须使用唯一中心/宿主 `initialize|serve|audit --config` 入口及[共同角色夹具](../tests/system/_center_host_services.py)，不能用组件示例代替。合并运行池与构建池静态目录时，前缀别名不得遮蔽模板 ID；与目标 ID 同名时直接使用该 ID，指向另一模板时拒绝装配。故障明确作用于中心、某个宿主或 Redis，重启保持原受保护配置与账本；公开客户端只访问中心。全局实例与本地 VM 编号通过原已校验宿主收件审计映射，不解析编号后缀。只读中心审计须停止该中心并取得原写者租约；无法观察原请求是否提交时保留未知责任，不将缺记录猜成 Cleaned。成功要求同一 Ready/Cleaned 身份、正确寿命与额度、实际进程账号/目录/cgroup、原 VM PID/时钟/网络/磁盘以及最终残留核对。缺角色、故障组、两题或精确收尾不能通过，安装与结果绑定 issue/PR 精确提交，不仅凭源码关闭 #292。

[发布 owner 退休回归](../sandbox-center-service/tests/business_authority/owner_retirement.rs)使用固定真实 Redis 与原验证证书，依次执行发布、终局确认和窗口确认退休，再为同一全局租户登记新本地编号；历史记录保持原样。退休前更换编号、退休后由另一全局租户复用旧编号均拒绝，原活跃编号和另一租户的新编号仍可使用。该组件不代替双模板顺序发布与原待结算候选的 KVM 恢复验收。

[宿主缓存 Linux 组件](../sandbox-server/src/linux/host_entry/cache/tests.rs)另用真实 inotify 观察验证：同一进程重复回执不重开/读取已封存产物，重启恢复则实际重新读取；不以耗时断言或替换 hash 函数代替该观察。替换 inode、符号链接、硬链接、属主/权限、可写父目录、截断和同 inode 内容变化均拒绝原回执，保留清单且不重新下载；正确字节的替换只能在重启完整重验后接受，坏字节重启仍拒绝。另沿原驱动和 SQLite 准入核对：正确的完整校验记录会传入节点提示，但节点校验失败时仍无物理动作和实例配置，同一原请求须经校验成功才可推进；错误修订、摘要或变化的产物不提供记录。原[节点文件回归](../sandbox-node/src/linux/publication_tests.rs)核对真实读取量、重启重算、内容变化拒绝和资源检查仍必需。[产物 HTTPS 组件](../sandbox-center/src/artifacts/tests.rs)通过真实 TLS 传输带中间非零数据及尾部空洞的镜像，回读核对完整字节、逻辑长度和物理占用，另拒绝错误零区域摘要、非空目标和非零起始偏移。沿原 root/Linux 隔离组件入口执行这些 ignored 用例，普通 CI 不取得宿主权限；它们不代替大镜像、真实模板构建或 KVM 验收。

| 编号 | 注入/输入 | 必需结果 |
|---|---|---|
| DC01 | 两个受控节点、多租户、并发创建及额度边界 | 固定绑定不冲突；平台/租户总额度不超限；本机拒绝不误记成功 |
| DC02 | 重复同操作、同编号异参数、错证书/租户/节点代际/协议 | 原结果复用或明确拒绝；无新增副作用 |
| DC03 | 中心提交前/后中断、提交回复丢失、发送后回执丢失 | 未提交不派发；未知只查/重发原请求；不分配第二个实例 |
| DC04 | 销毁先到、迟到创建/旧回执、过期窗口 | 不复活；旧请求明确拒绝；新实例不受影响 |
| DC05 | Redis/CAS 冲突、无空间、持久化异常、截断/错误类型/格式 | 无半准入/额度丢失；停止新派发；不静默初始化或造成功 |
| DC06 | 心跳过期/节点失联、清理回执未确认 | 不换宿主、不提前释放额度；恢复后按原身份确认 |
| DC07 | 全局 get/list 筛选/排序/分页、模板别名变化；续期确认后从另一入口查询 | 现有固定 SDK 合同保持；租户隔离；原不可变模板不改变；查询对应调用期间合法中心视图，已确认续期不被旧缓存覆盖 |
| DC08 | create/connect/set_timeout 重复/乱序、时钟偏差与期限边界 | 不重置原起点；未确认续期不报成功；到期撤销保持 |
| DC09 | 生命周期未知与退休、有限请求窗口/结果、身份耗尽及已结束键/引用回收 | 未知不回收；保留结果不提前驱逐；旧请求不再执行；驻留条数/字节随声明预算有界，不随历史总请求无限增加 |
| DC10 | 数据鉴权/路由、断线、慢流、超额、终止与流并发；受控端点登记丢失/消费者失效 | 两端资格和预算生效；不重执行或仅凭 PID 接管命令；不将通道结束当 VM 停止；局部失败按范围处理，其他合格消费者在声明条件下推进；无任意地址转发 |
| DC11 | 模板下载中断/损坏/无权限/无空间与重新登记 | 不发布错误模板，不替换 base；仅回收原临时资源 |

测试使用明确故障点、屏障及可推进时钟，不靠随机 sleep。两节点替身使用相同协议和本机端口合同，不另造一套业务规范。存储适配必须使用真实固定 Redis；内存假库不能代替 DC05。

### 5.2 单台真实 KVM 用例（DK01–DK06）

| 编号 | 场景 | 必需结果 |
|---|---|---|
| DK01 | 统一入口创建→get/list→命令→文件→续期→销毁 | 固定 SDK 实际调用及字段符合合同；真实停止；原资源无残留 |
| DK02 | VM 已就绪、节点事实已持久化，丢弃发往中心的回执并重启中心 | 原 VM 仍是同一个；恢复连接后可访问；不重启/不双跑、不重算寿命 |
| DK03 | 已接受启动/停止动作的节点服务强制中断及重启 | 保留原 SQLite 和资源身份，本地对账后向中心确认；未知不虚假成功 |
| DK04 | 受控网络中继阻断中心→节点、销毁竞争及到期 | 中心不换节点/不提前释额；节点本地期限执行；恢复后精确结算 |
| DK05 | Redis 正常重启及中断窗口；保持 VM/节点独立存活 | 新控制写入按合同暂停，恢复原账本；无重复创建/无权限降级 |
| DK06 | 两用户、固定活跃上限的连续创建销毁及历史退休，含两层重启和迟到请求 | 资源/日志/记录符合部署预算；旧请求拒绝；收尾预留有效 |

DK06 的循环次数、采样点、数据规模、期限及预算在验收配置中固定并纳入摘要，不写死为生产上限；不得只跑一次创建销毁就记作长期容量验收。采样包含两层受管驻留键/订阅/池/关联及队列规模，报告内存/条数/字节与预算的对应；库/内核剩余开销单列，不以计数稳定宣称整进程内存已证明。进程、网络、磁盘、数据库/Redis/AOF和日志残留按原身份观察，失败保留有限脱敏证据再精确清理。清理未知保留占用，不宽泛停机/删目录。

两台真机后续任务至少覆盖：同一用户跨节点、两节点全局额度竞争、真实模板传输、节点整机关闭而中心存活、网络分区/恢复、时钟与资源差异。其结果另记，不能由 DK 用例替代。

### 5.3 旧公开入口退出验收

以下是 #292 公开链路切换及关单的必需检查，不表示已经完成。新链路通过对应验收时，同批清除被替代的生产组装和工具引用；在新链路未接通前不提前删除仍在使用的服务。只保留一种生产架构，不提供开发阶段旧库升级或双模式部署。

- 公开 SDK 生命周期、命令/文件及模板用户入口均指向中心；宿主不安装公开用户鉴权表，不接受用户 SDK 管理请求。复用 DC02/DC10、DK01 的身份和端到端场景增加直连宿主拒绝断言，不新建重复故障矩阵。
- 中心或 Redis 不可用时，沿用 DC05/DK05 的失败关闭、固定绑定及本地收尾检查，并确认没有旧公开入口回退；不把节点可独立执行到期/恢复误当降级公开服务。
- 查清并删除原公开 SDK→本地应用组装、宿主全量用户密钥字段、旧模式开关/别名、被替代的配置样例和启动夹具引用。模板构建工具也迁移；保留唯一的本地状态机、SQLite、执行/恢复算法及其组件故障测试，不以清理旧入口为由删掉覆盖。文档/脚本/依赖引用与真实服务产物一致。
- SDK/Harbor 正式验收和两级晋升均运行同候选中心、Redis、真实宿主，经中心进入，报告绑定全部实际产物与协议；不能用旧链路的绿灯代替。普通组件 CI 与可信监督器的分批切换、固定材料制备及安装门槛只在[CI 运行环境](ci.md#中心与宿主拆分的-ci-扩展)维护。

## 6. 证明验收

按[证明工作流](proof-workflow.md)执行固定工具链、重新抽取、全部 Lean 编译与依赖审计。当前已证明输入与缺口只看[覆盖记录](proof-coverage.md)；基础、SDK、KVM 或题集绿灯均不能替代证明。
