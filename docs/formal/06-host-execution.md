# V6 真实捕获、装载、激活与精确回收

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#638](https://github.com/VeriInfra-Lab/sandproof/issues/638)。状态：第一阶段实施与证明计划；代码定位基线 `56b14d64`。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实现范围

对应 [sandbox-node](../../sandbox-node/src/linux.rs)、[sandbox-runtime](../../sandbox-runtime/README.md)。当前 `sandbox-node/src/linux/execution.rs` 的 `PrepareVm` 明确拒绝 snapshot source，不能将历史 Freeze/Seal/Commit 命令或模型记为真实支持。本阶段新增固定 Firecracker/jailer 1.16.1 的完整路径，原身份/围栏/受保护目录及清理继续复用。

新增外部接口合同仅为[依赖登记 S-C1–6](dependencies.md#第一阶段新增依赖合同)。选择哪个 manifest、原 VM 是否允许停止、何时激活和哪些文件允许删除属于自有业务。外部工具可返回错误但留下部分效果。

## 2. 实际动作与证明接口

| 动作 | 真实输入/输出 | 必须对应的义务 |
|---|---|---|
| PauseOriginal | 原 VMM/API/运行代/围栏，确认 paused 或未知 | 不操作同名新进程；暂停确认和当前停止意图分开 |
| CaptureFull | 原 paused VMM、唯一 capture 目录、Full RAM/device 和所有匹配 disk | 整个截面无运行/宿主写者；不把内存文件等同完整恢复材料 |
| SealBundle | 原文件句柄、长度/摘要/配置/manifest、sync 与受保护发布 | 短写/替换/同步失败不成功；逻辑计数器不能替代内容摘要 |
| LoadPaused | 唯一 claim、新候选/私有盘、File backend、resume_vm=false、已取得 pin | 使用指定完整 bundle；新 jail 保持设备相对路径；不自动运行、不写坏来源 |
| ActivateOriginal | 已持久 MayHaveExecuted、原 candidate、当前围栏/寿命/凭据 | 只激活该候选；重复投递不创建新候选；旧任务不能绕过终止 |
| Observe/StopOriginal | 精确 VMM/辅助进程及工作身份 | Exit/Stopped/排空/Released 分开；PID/路径缺席不能证明无迟到作用 |
| DeleteBundle | V5 删除许可、精确对象、无 pin/映射/可复活任务 | 只删授权原对象；部分删除保持原责任，实际释放后才退额度 |

物理效果必须接原日志/尝试/进程身份的持续观察。文件从路径取得原句柄到校验/使用是一个安全链；Metadata 条件证明不能代替 open_checked 的路径/FD/竞态证明。需先解决 V0 T8 的可抽取拆分或明确阻断，不能扩大可信业务层。

## 3. 特殊失败规则

完整 bundle 接管前不得停止原唯一状态；回滚暂停前先让迟到 capture 工作失效。LoadPaused 返回后，内存 backing 仍被 VMM 使用，文件不能改写或逐出。设置 MayHaveExecuted 后，即使后来看到 paused 或 stopped，也不能允许重载旧来源。

停止/回收以原身份驱动且可重试。manager 重启保留运行 VMM，不能以服务停止代替 VM 清理。实际进程父子关系、退出信号和 service/cgroup 配置必须核对：声明中的管理服务崩溃不能被自有停止钩子转换为未授权的 VMM 终止；对应故障注入需要真实验证。unexpected VMM death 后的状态保证按声明故障域报告失败，不以冷重启补成功。

## 4. 验收与边界

同步守卫、参数、identity/path/manifest 校验、分类与精确清理决定经固定抽取；系统调用/FC/文件持久性用显式外部合同加实机证据。Rust 业务分支仍需机器对应，不能仅源码检查后称实现已证。

真实 KVM 必须观察 RAM-only nonce、磁盘内容、长生命周期 guest 进程、旧 VMM 退出、新 VMM 身份以及后续修改；无旧 VM 留存占额。故障窗口覆盖捕获/发布/停止/加载/激活/删除；File backing 生命周期和磁盘释放单独观测。提供 LC-P4/5/6/7/9/10 的物理部分、LC-I6/7/8；结果绑定固定 CPU/内核/FC/文件系统。无 UFFD、增量链、迁移或任意设备组合支持承诺。

### 捕获与封存部件的接口

`Runtime::pause_original_vm` 绑定原进程的 PID、启动时间、boot 身份和原 jail，只有 API 确认为 paused 才产生 `PausedOriginalVm`。句柄只是观察结果，不授予当前操作资格；丢弃它不会停止或恢复原 VM。

`capture_full_into` 发送固定 Full 请求，从原目录句柄打开 state/memory 和白名单 rootfs，保持暂停并复制到独立 store。首 profile 对应 `stage_and_boot` 的 rootfs→rootfs.ext4。调用方必须持续持有原工作/围栏及无写者条件，不能用过期观察代替它。

`SnapshotStage` 检查原 FD 的类型、属主、权限、链接数及长度，逐块复制并计算真实 SHA-256。发布前核对保留的目标 FD、替换/修改及全部测量，随后同步文件、目录并执行不覆盖的同目录 rename。任何错误都保留材料；rename 后的同步错误可能已有可见效果。已有名字不自动判为同键成功，须查询原持久工作。

`CapturedFullSnapshot` 与普通文件复制结果分型；节点 `full_snapshot::seal_full_capture` 再核对原 VM 身份、完整 capture 目录键和原 profile，并从实际测量构建完整 manifest。`PublishedFullCapture` 只表示文件与同步事实；V5 持久登记前没有 stop 许可。此模块无删除/kill/thaw 的失败补偿，尚未接入默认节点命令，不等于完整 pause/unload 已验收。具体提交和证据在 #638 记录。

纯 `snapshot_policy::accepts` 是复制路径实际调用的 Metadata 接受检查；其局部证明不能替代原 FD 获取、冻结期间无写者、完整 manifest/事务连接或 S-C 文件持久性合同。

`SnapshotLayout::initialize` 只在受保护的空父目录首次创建 `v`、`s`，通过同一个父 FD 创建、检查两原句柄，记录布局版本、原部署配置绑定及父目录/两子目录的设备、inode 和出生身份，先同步再发布。`reopen` 读取原持久记录，允许目录已有运行材料，逐一核对原身份与绑定；缺失、部分初始化、替换或版本不符均拒绝，不转为重新初始化或另建编号。Full 捕获消费这个私有构造的布局，并重查原 runtime root，任意两个 Path 不提供同等保证。这里仍采用受控静态挂载、无无关特权写者的部署条件；自有初始化/重开判断没有被整体列为可信依赖。

运行清理只针对 `v` 下原 VM，不删除共同父目录或 `s`。布局格式 1 是新 stage1 的有限布局，不收养或迁移旧 profile。此布局已接真实组件和故障回归；其完整 Rust 对应证明及与正式物理 profile 激活的连接仍须补齐。root/0400 封存文件供 jailer File 后端加载时的权限路径、是否需要私有 RAM 副本及相应峰值预算属于恢复批缺口，当前恢复仍不启用。

### 原捕获动作与已发布材料观察

`Runtime::pause_original_vm` 与 `full_snapshot::capture_paused_original` 分成两个实际动作，供 V5 在两段之间持久提交原暂停确认与 Full 工作；后者只对独立核对的原 `PausedOriginalVm` 执行 Full、Seal，随后重开原 bundle 核对文件，再确认原 VM 仍暂停。它不重发 Pause，只返回 `FullCaptureObservation`，没有停止、释放、登记恢复资格或激活副作用。调用方必须提供原已持久工作、当前围栏、预算与无写者条件。捕获/发布后错误仍可能已有材料，不得换键重捕获。原 `WorkExecution` 接线及持久 Stop 工作由 V5 提供；本组件不等于实际 unload 已完成。

`SnapshotLayout::open_capture_bundle` 和 `full_snapshot::observe_stored_capture` 只读取原发布键，通过保留 FD 检查受保护原目录、完整白名单、root/0400/单链接/长度及实际字节摘要；节点严格核对原 CaptureBinding、受保护 profile、manifest 与全部测量。已有回执摘要必须相同；尚未取得回执时返回的只是待 V5 对账的物理事实，不能凭完整文件赋予资格。它不再次 Pause/Full，也不制造原 VM 的历史来源证明。

新观察器及调用链的整体 Rust 对应证明仍须补齐；既有 `snapshot_policy::accepts` 定理只覆盖其中原 Metadata 守卫，不能代替目录/FD、全字节读取、JSON、完整工作或事务连接。普通回归和真实错误版的复现见[组件说明](../../verification/experiments/capture-observation/README.md)，实际版本及验收状态仅在 #638/PR 与覆盖记录维护。

### 原进程来源字段

新的 VM claim 在原 launch gate 开放前持久保存 `ProcessRecord.boot_id`，它来自同一被记录 PID/启动 ticks 的宿主 kernel boot identity。新 Prepared/Activated 回执经原 claim、PID/ticks、jail/cgroup 检查后保留该字段；不是从 manifest、guest 或恢复时的全局 boot 值补造旧身份。`HostEvidence::source_process_identity(command, expected_resource)` 同时核原 command 和调用者从原 action 独立取得的完整 resource，再原样投影来源，不能传自己的 resource 自比。owner/operation/原工作与多份回执来源一致仍由 V5 的原持久事务建立；动作前仍须重核原进程，getter 不保证未来存活。

当前唯一 claim 与 Prepared/Activated 回执格式必须保存原 boot 字符串。缺失或 null 的旧开发字节直接拒绝，不保留旧形状往返、不补当前 boot、不收养或迁移旧库。字段齐全仍须核对完整 command、独立 resource、PID/start/boot 和当前原进程；解析成功不表示来源或执行资格已成立。新部署使用隔离数据，历史实现和证据只从 Git 查阅。整个进程记录/系统调用/回执/SQLite 的机器对应仍是后续义务，不把字段回归记成完整来源链证明。

### 捕获前原配置的只读事实

`Runtime::observe_original_configuration` 在已接受的原只读工作及关闭新准入条件下，沿原 PID/start/boot、claim、jail/cgroup 和保留目录 FD 读取。它只发送 GET，核对固定工具的原 instance、两次配置字节一致，并测量实际执行 inode 与原私有内核副本的字节摘要，观察原可写盘长度；前后仍须是同一来源。进程、文件、回复或上界不能确认时返回未决；错误不会 Pause、Full、Stop、创建或删除材料。返回类型私有构造，但只代表读取时的事实。

当前冷启动来源观察先将实际 GET `/vm/config` 按固定 Firecracker 1.16.1 的返回形状解析，再与启动实际使用的同一 `BootPlan` 对照：vCPU/RAM、内核路径和参数、单 rootfs 的可写/缓存/同步 I/O 配置、零或一个原网卡、MMDS 版本/地址/接口，以及不使用 balloon、hotplug、pmem、vsock、entropy。额外或重复字段、额外设备、未知配置、缺失必需字段均拒绝，不把观察值回填成期望值。固定返回投影的依据是 [VmmConfig::from](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/src/vmm/src/resources.rs)、[machine](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/src/vmm/src/vmm_config/machine_config.rs)、[drive](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/src/vmm/src/vmm_config/drive.rs)、[network](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/src/vmm/src/vmm_config/net.rs) 和 [MMDS](https://github.com/firecracker-microvm/firecracker/blob/2038188f145fb81b8d098147a10e9d9f392fd22f/src/vmm/src/vmm_config/mmds.rs)。该检查不是任意 VMM 或恢复来源的通用兼容验证器；恢复批须按原候选来源和固定工具真实返回继续连接，不能用此冷启动检查直接背书多周期。

特别是该固定工具的 GET 投影会将 custom CPU、logger、metrics 返回为 null，并隐藏 custom CPU 的 machine 字段；null 不能证明它们从未配置。原唯一启动路径、受保护工具字节及没有未登记 API 写者的部署条件仍不可省略。这个可见配置判断不认证 CPU/初始镜像兼容、编码堆界、捕获前物理预留，也不授予捕获或停止许可。节点仍须与第一次 Create 前的受保护 epoch、原 CreateOptions/template 和实际原文件测量对照；缺少事实不得填默认值。原可写盘仍在变化，当前长度不替代后续连续暂停区间内的完整测量。V5 接受回传后须重核原工作/版本并在原事务保存，同一责任接管成功后才发物理捕获许可；不在 SQL 事务内等待文件或 API I/O。manager 重启不能仅凭历史 epoch 观察继续启动，须重新核对当前受保护部署。当前只读组件不代表这些连接已经实现或证明。

### 原来源绑定的停止与退出观察

`Runtime::stop_original_vm(resource_id, expected_source)` 只读取一次原 claim，要求独立调用方保存的 PID、启动 ticks、boot 与原 claim 完全一致，并要求当前宿主 boot 仍相同，再按该已核记录取得 pidfd、发送 TERM/KILL 和等待退出。不再调用会重新读取当前记录的通用 stop 接口；当前同名 claim 被替换、旧来源字段缺失、宿主 boot 变化或读取失败均保持未决。该方法只实现物理停止，V5 必须先确认原材料接管和原 Stop 工作提交，并持有原操作准入/助手排空条件，不能从可调用性推导允许停止。

`observe_original_stopped` 用同一原来源只读观察，不重发信号。它与原通用停止接口的 `Stopped` 一样只表示原进程不再执行；尚未收割的 zombie、存活助手、运行位置占用和文件 backing 仍有各自责任。停止和退出观察不删 jail、网络、claim、bundle 或父布局，也不退额度。运行清理和位置释放必须继续使用各自精确身份、无使用者和实际清理事实；不得把 VMM 退出回执直接转换为对外 Unloaded。

### 运行位置释放前的 UID 观察

原 `retirement_manifest`、`retire`、`observe_retired` 与 `observe_released` 的物理检查还核对 `/proc` 下每个进程的每个线程：real、effective、saved、filesystem 四个 UID 任一仍为原位置 UID，都不能确认释放；zombie 不跳过。非 UTF-8 进程名称不影响 UID 字段解析。缺/重复/损坏 UID 字段、权限和其它读取失败保持未决；status/task 缺失只有对应目录也确实消失时才接受为该任务已离开。

这次观察在原持久 Releasing/当前运行代已封闭、独占 UID 映射及原助手/route 门的条件下使用，不能独立证明将来不会有旧代新作用。运行代关闭由 V5 原 work 和节点 Occupied/Releasing 准入接线保证，已登记助手须实际结束；没有关闭新效果之前，不允许先扫描再退位。该检查不删除进程、claim、bundle 或保护根，不把一次 KVM 样本当所有运行情形的保证。UID 观察的实现及整个回收链仍须补真实 Rust 对应证明，不能沿用旧 Metadata 守卫定理。

UID 扫描限于必需身份元数据，不读取 cmdline/environ；每次最多 262,144 个目录项、单 status 128 KiB、挂载信息 1 MiB，并检查同一 2 秒观察期限，超额/超时均保持未决。系统调用仍依赖固定 Linux/procfs 的完成语义，不是任意文件系统的硬实时保证。只允许真实 procfs 挂载，拒绝可检测的 hidepid/subset、多层 NSpid 与 PID namespace 不一致；这些检查不能从进程内部独立证明所在 namespace 就是整台宿主。受保护部署必须将服务放在原宿主 PID namespace，挂载完整宿主 proc 视图，并由本轮准入核对；不能在私有 PID namespace 内重新挂 proc 后宣称看到了所有宿主任务。该前提与独占 UID/已登记 spawn 一起维护。

UID 的数值编号域也必须相同：扫描者分别将 `/proc/self/ns/pid`、`/proc/self/ns/user` 与原宿主 PID 1 的对应 namespace 身份（设备号/inode）核对，不匹配或任何读取失败保持未决。配置 `vm_uid`、原 claim 的文件属主、jailer 使用的 UID 和 `/proc` 显示的四 UID 必须来自同一个受保护的宿主 user namespace，部署与整个原扫描/释放 worker 期间都不得切换 namespace。仅 PID 视图相同不足以保证 UID 数字相同；未映射编号可能在读取者域显示为 overflow UID，依据 [Linux user_namespaces(7)](https://man7.org/linux/man-pages/man7/user_namespaces.7.html) 的 Unmapped user and group IDs。此路径不支持任意 UID 映射或私有 user namespace，不新增编号翻译器；PID namespace 内重新挂 proc 不能自证外层宿主的限制仍保留。取消或超时也必须保留实际仍运行 worker 的许可与责任，不能因调用方不再等待就 Free。


### 共享宿主执行与并发读取

当前候选将同一个 `NodeHost` / `LinuxWorker` 通过共享引用执行，不复制宿主或物理 profile 的租约。每次执行返回原动作身份的回执；诊断由该次调用单独返回，不通过全局“最近一次错误”字段传递。执行错误仍保持 Unknown，不视为没有发生效果。可共享引用本身不提供同会话协调，V5 的持久领取、进程内活动工作和原助手门仍须共同保证同一会话不会出现两个物理执行者；取消等待不释放仍在执行的工作责任。

不可变模板缓存只在查询、插入时持有短锁，完整摘要读取及前后文件身份检查在锁外进行。并发首次校验可能重复读取同一文件，必须由实际执行名额限制；缓存不保证文件不可变，固定部署仍须禁止未登记写者。物理 profile 使用原租约 FD 的有界偏移读取，不通过共享游标上的 seek/read 校验配置；共享对象最后一个所有者退出前不得释放租约。上述候选改动尚待本批普通回归、系统组合与真实 Rust 对应验收，不是新增已证结论。


正式初始化从 `LinuxWorker::host_epoch_registration` 取得既有受保护文件的完整 SHA-256、boot 和原节点/namespace 绑定，并重新核对同一 provider 的真实宿主信息。`ProtectedHostEpochSource::provision` 仅供显式操作者命令生成全新当前格式文件：原父目录 FD 下 create-new、root/0400、全字节写入及文件/目录同步；不覆盖、修复或自动升级。错误可留下未登记文件，不能把错误当“没有写入”。服务重启必须核对已存原 registration，不能用这次观察自动改写旧值。实际工具字节与固定 Firecracker/jailer 语义的对应、完整宿主 proc/sys 视图及 epoch 内配置不变仍是部署条件；该接口不产生快照兼容、捕获预算或停止许可。正式 CLI/初始化/重启连接和本批验收由唯一服务装配完成，单个生成函数不代表这些连接已经通过。


重启后 `Runtime::observe_original_pause_state` 只观察持久工作指定的原来源：原 claim、PID/启动 ticks/boot、宿主 PID/user namespace、jail/cgroup 与保留目录必须匹配，GET 使用该目录下同一个 Unix 连接并核对实际 peer PID/UID/GID。Paused 只返回此次观察的私有句柄，Running 保留未决责任，Exited 需要原记录一致及内核退出依据；API/格式/权限/连接/超时错误和单纯 proc 路径缺失都不生成 Paused/Exited。观察没有 PATCH、Full、Stop、重新初始化或位置释放。Exited 也不证明旧捕获 worker 已结束，管理重启不能据此无条件重发 Full。该观察与后续分段捕获接线属于本批运行组件，尚待真实固定 VMM endpoint 和完整卸载验收，不声称获得新机器证明。


### 原 Pause 的 Node 接线候选

`NodeHost::pause_original(action, pause, original_source)` 只消费已持久领取的原 `FreezeVm`，并要求独立的 `SourceObservationRequirement` 来自同一已存 source-work：完整 work digest、node、resource、PID/start/boot 必须与 Pause 输入相同。服务仍须独立核外部操作到原 native action 的映射、原 plan、当前寿命/资格和领取/回执；Node 不从 DTO 重新生成 bootstrap registration，不把外部 incarnation 当作本地 generation。

Node 沿原 Occupied 位置、owner/VM/generation、原 helper 独占门和未终止当前围栏执行；在 Pause 两侧重新核对原 epoch、实际源配置和文件事实，确认同一私有 paused 句柄后，才构造原 action 的 `Frozen` 回执。任何错误仍可能已经 Pause，原责任不退。`observe_pause_original` 复用相同 guard 和 source 输入，只读重开原 paused 观察，绝不重发 PATCH 或 Full；Running、Exited 或不能确认均不产生 Frozen。通用 execute/observe 在缺少这些原输入时仍拒绝 Freeze，不靠当前 PID/全局 boot 补字段。

此候选尚待统一 ning 编译/针对性回归、真实固定 VMM 端点、对应 Rust 抽取和完整卸载验收。Full 仍是后继独立 `SealSnapshot`，须额外接 protected layout、实际来源匹配的兼容配置和原持久预算；不能从请求里的 profile/尺寸直接制造期望或预留。Linux 文件大小限额与原 cgroup 只提供各自实际约束，不能保证唯一源 VM 在撞限时存活，也不是正常支持条件充分的证明。尚未闭合这些条件之前不启用物理 Full，更不因此赋予 Stop、Unloaded 或释放资格。
