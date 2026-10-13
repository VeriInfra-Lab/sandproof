# Sandproof 系统架构

[文档目录](README.md) · [项目蓝图](project-blueprint.md) · [模块合同](module-contracts.md) · [验收清单](testing-plan.md)

本页只解释组件关系、存储用途和运行路径。功能范围由蓝图决定，失败与边界由模块合同决定，工程通过情况绑定 issue/PR 的精确源码与产物；证明状态只看[覆盖记录](proof-coverage.md)。已有代码、组件通过、真实组合通过和已证明是四件不同的事。

下文是 [#292](https://github.com/ningZhang-cs/sandproof/issues/292) 的唯一交付结构；迁移组合、故障组及两级晋升仍须按该 issue 验收，不从结构图推断已部署或全部通过。

E2B 对照使用其 [runtime 架构文档](https://github.com/e2b-dev/runtime/blob/132dadd2ef55bbac301528500e8a4538caf4b163/docs/ARCHITECTURE.md)的固定阅读版本，不升级[兼容基线](project-blueprint.md)。E2B 的 `runtime` 是整套后端；本仓库 `sandbox-runtime` 只是底层宿主工具，两者范围不同。

## 1. 唯一服务链路

```text
用户 / 固定 E2B SDK / Harbor
            │ 公开鉴权、生命周期、命令和文件
            ▼
    sandproof-center（非 root） ↔ 权威 Redis（独立非 root）
            │ 固定登记端点、双向认证 HTTPS
            ▼
    sandproof-host（私有 root 服务） ↔ 原 SQLite
            │ 原本地控制、执行资格、恢复、精确清理
            ▼
    jailer / Firecracker → 来宾 Linux / envd / 用户程序

中心模板目录 → 受保护只读 HTTPS → 宿主有界缓存 → 实例私有系统盘
```

公开入口只在中心。宿主不安装用户 API，不保存全量用户密钥，也不接收客户端提供的宿主地址、路径或成功标志。一个用户可有多个沙箱，一个沙箱只绑定一个固定节点及代际；失联不在别处重建。单物理测试机上的不同进程/账号/目录仍通过实际网络协议连接，不用函数调用代替。

中心和宿主是两个独立服务产物，不是一个程序的旧/新启动模式。`sandproof-center` 来自 [sandbox-center](../sandbox-center/src/main.rs)，`sandproof-host` 来自 [sandbox-server](../sandbox-server/src/main.rs)。包目录保留并不意味着继续提供旧 `sandproof-server` 公开产品。正式命令为 `initialize|serve|audit --config 绝对路径`，契约信息用 `contract-versions`；宿主另有原受保护 `relay` 子进程入口。初始化必须显式执行，重启不会补建丢失数据库或猜测导入旧格式。

## 2. 职责分层

| 模块 | 唯一职责 | 不负责 |
|---|---|---|
| [sandbox-api](../sandbox-api/src/lib.rs) | 原 SDK 解析、鉴权边界、字段/错误/分页编码 | 不选择 VM，不发布状态 |
| [sandbox-center-service](../sandbox-center-service/src/lib.rs) | 全局准入、固定节点、意图、查询、令牌和协调 | 不远程指挥本地 VM 子步骤 |
| [sandbox-center-core](../sandbox-center-core/src/lib.rs) | 纯全局决定、身份、预留、单调终止、回执和窗口退休 | 无 Redis、HTTP、时钟读取或 VM 操作 |
| [sandbox-center-store](../sandbox-center-store/src/lib.rs) | 一个 Redis 控制块、原字节/修订 CAS、耐久确认和恢复 | 不另存业务状态或调度队列 |
| [sandbox-host-service](../sandbox-host-service/src/lib.rs) | 高级意图收件、原映射/应用推进、独立到期与恢复 | 不接受用户全局准入，不信任 HTTP 即 VM 成功 |
| [controller-service](../controller-service/src/application/mod.rs) | 原 SQLite、本地工作/回执、寿命、维护与执行资格 | 不建立第二套 VM 算法 |
| [controller-core](../controller-core/src/lib.rs) | 原纯本地状态机与安全判断 | 不操作数据库或宿主 |
| [sandbox-node](../sandbox-node/src/linux.rs) | 原工作对应真实身份、围栏、观察和精确清理 | 不凭名称或 PID 猜测归属 |
| [sandbox-runtime](../sandbox-runtime/README.md) | 私有盘、网络、jailer/Firecracker、有限辅助命令与回收 | 不决定用户权限或逻辑完成 |

入口只组装模块，纯决定接收显式时间/观察，持久确认与外部派发分开。中心是全局归属、绑定、意图和预留的唯一权威；各宿主原控制器是本机阶段与资格的唯一权威。两层允许暂时滞后，不共享跨库事务。网关、缓存和退出码都不能自行宣布运行或清理完成。

运行/构建节点通过受保护 `role` 分池。公开 create 只能选运行池；私有构建复用同一个准入/生命周期算法，但只能选构建池；模板验证仍走运行池正常流程。角色不是客户端参数，也不是旧/新控制器模式。

## 3. 创建到销毁

1. 中心先取得有限请求名额，再用用户凭据识别租户，解析原 SDK 的模板、寿命、网络、环境和元数据。
2. 归属与别名在当前权威中解析一次。创建绑定不可变修订、完整描述符摘要、获准资源与原选项；未知名称不回退到 `base`。
3. 纯中心核心检查平台/租户/节点额度、真实健康与缓存，分配不复用的全局身份。意图、固定节点、预留和待发送责任同一 CAS 提交。
4. 提交确认后经登记 mTLS 端点发送高级 Create。网络未知保留原请求，不分配替代 VM；用户等待超时不取消责任。
5. 宿主在原 SQLite 事务核对完整身份/参数，映射原本地身份与资源位置，由原状态机完成围栏、私有盘、网络、启动和 envd 初始化。全局描述符用于摘要/身份，本地缓存路径仅由受保护缓存派生用于执行，两者不冒充同一摘要。
6. 原就绪事实满足要求才返回 Ready，中心保存匹配事实再返回会话。运行起点来自宿主首次就绪，不是重试或重启时间。
7. kill/到期先持久提交单调终止并撤销访问，再停止、排空、清理和核对物理释放；确切 Cleaned 后才解除预留。

未知不是失败或成功。停止不等于全部清理；心跳、丢连接、查询缺席和等待超时都不释放责任。节点在中心断线时仍执行自己的启动期限与寿命。

结果确认与历史退休也用原权威：每个实例绑定一个有限保留位置和该位置的代次，最后终态由宿主精确确认后才提交关闭；宿主在原事务核对工作/映射/结果，丢回执只重发同一完整回执。未结算构建/验证引用会否决该实例退休，但长期运行的同伴不阻止其他位置回收；空位置不因扫描反复消耗代次，单位置耗尽不回绕。详细身份、拒绝前沿和预算只在[模块合同](module-contracts.md#调度寿命与终止)维护。

## 4. 命令、文件和网络

公开网关核对当前中心权威、实例令牌、归属、可见性与寿命，才转发到固定 mTLS 宿主。宿主再次检查原映射、阶段、权限、终止围栏与精确实例，再通过原中继连接 envd。用户不能指定宿主 URL，也不能访问 envd 初始化接口。

envd 在 VM 内执行用户进程、信号与文件操作；来宾及来宾 root 不可信，来宾文件路径不是宿主路径。我们不重新实现完整 POSIX/ACL，也不证明来宾系统或虚拟化逃逸不存在。

现役模板复用固定 envd；自主替换由 [#311](https://github.com/ningZhang-cs/sandproof/issues/311) 推进，不作为当前 V1 发布前置。首个 [sandproof-guest-core](../sandbox-guest-core/src/lib.rs) 仅实现命令订阅完成判定和文件传输计费，不含服务监听、进程启动或文件 I/O，不能部署为 envd。后续协议/执行适配使用这些原函数，保持纯决定与系统调用边界；公开路线仍经中心与宿主网关。尚待冻结的 PID 复用策略见[模块合同](module-contracts.md)，抽取试验及未证明范围见[覆盖记录](proof-coverage.md)。

持续命令输出使用有界在途缓冲、分块、并发和背压，不设累计字节上限；执行期限不等于字节上限。文件、一元回复和宿主辅助命令各有有限预算。取消、撤销、实际退出与排空责任见[模块合同](module-contracts.md)。

[sandproof-guest-runtime](../sandbox-guest-runtime/src/lib.rs) 已增加独立 Linux 进程端口：私有 worker 放行、真实 pidfd/管道/PTY/wait 与原纯核心连接。它已有当前账号的组件测试与独立特权身份验收入口，另有文件编解码、授权路径与真实一元文件操作组件；尚无公开监听、持久 PID 注册、完整 SDK 命令或文件传输服务，未安装到任何模板；这些端口不承担中心、宿主或 VM 生命周期职责。

[sandproof-guest-service](../sandbox-guest-service/src/lib.rs) 承接来宾 HTTP 编排，当前提供受保护初始化、默认上下文快照及固定 MMDS 客户端。授权上下文组件可按本地账户构造完整 worker 上下文，数值身份在单线程 worker 内切换。独立准备任务持有原 worker 与预留额度，取消后实际 wait 才归还；登记确认后移交执行任务，后者持有原进程、执行期限和有界订阅，断线不撤销执行，未知结果不重执行。一元文件池持有原授权至阻塞 I/O 结束，结果保留期间继续占额；流式文件池还由在途块持有容量，以实际释放推进下一次读取，已有单文件 Linux 组件验证。顺序批次复用一次授权/账户快照和同一容量/期限，累计结果与各文件部分效果分别处理，已有 Linux 批量回归。独立 multipart 解码器已有本地组件及固定 Go MIME 对照；它已接到批次和 identity HTTP Body，保持同一授权/期限直到实际正文结束，真实文件及 SDK multipart 组合已有原提交 Linux 组件证据，后续候选仍须重跑。独立有界 gzip 编解码已有本地组件和固定 Go 对照，尚未连接 HTTP/文件 worker。等待取消不终止已派发系统调用。公开登记、正式命令/文件路由与生产监听器尚未接通。它不负责中心/宿主权限；完整认证、预算与执行组合仍须验收，细节维护在[模块合同](module-contracts.md)。

宿主通过 Linux 网络空间、TAP 和防火墙强制禁网及静态 IPv4 规则，不依赖来宾自觉。公网需部署显式开启，平台禁区始终生效。DNS 也是出站通道，固定 DNS 地址不等于查询内容过滤。域名策略、IPv6 放行、运行时网络切换、任意服务端口公开不在当前版本。

## 5. 模板与构建

模板是软件环境与资源形状，不是运行中的 VM。用户通过通用 Dockerfile/OCI 构建取得不可变模板编号，再用原 SDK create；不按题目增加产品分支。转换复用标准工具，采用冷启动，不保存运行内存快照。

中心保存归属、别名、描述与发布状态，正文在其服务账号独占目录，Redis 不存大磁盘。只读 HTTPS 按认证节点、修订、摘要和 kernel/rootfs 部分读取，不接受客户端 URL/输出路径。宿主有界下载并核对完整哈希、大小、格式和文件身份后才登记可用；缺失不替换为 base。

完整缓存、下载临时副本、未知残留、元数据和 inode 均计入原物理峰值预算，激活仍预留整体两倍余量。资源不足先拒绝，不删除未知工作或降低题目资源造成功。

自助构建仅在显式部署时提供。原 Python 构建监督器保留唯一作业库、有限队列与工作器，作为受保护私有辅助服务，不持全量用户密钥、不直接向用户开放。中心鉴权后派生租户；私有会话在同一权威绑定原请求窗口、构建编号与全局 Create。构建 VM 走原宿主生命周期，公开用户不能查询或使用其令牌。

交接在可信工作器关闭写者后进入中心独占目录，中心复用原候选校验/封存。发布验证复用运行池正常准入、真实启动与精确销毁；成功和清理结算齐全才 Ready。中心不再造作业队列。整个模板子系统在证明边界外，工程隔离、权限、限额与收尾仍须验收；核心模板绑定/记账仍有后续证明义务。

## 6. 存储用途

| 内容 | 权威/位置 | 作用 |
|---|---|---|
| 全局身份、逐位置代次、预留、意图、事实、模板元数据/引用 | 一个 Redis 控制块 | 同一提交，无缓存决定 |
| 本地核心、去重、映射、工作/结果 | 每宿主原 SQLite | 原事务、容量、恢复与退休 |
| 精确资源/位置/进程/凭据 | 宿主受保护记录 | 实际观察和清理 |
| 模板 kernel/rootfs | 中心目录与宿主有界缓存 | 不可变环境、校验分发 |
| 实例可写盘 | 原运行位置 | 实例独有，不污染模板 |
| 作业、有限构建日志/窗口 | 原构建 SQLite/workspace | 不重复存为中心队列 |
| CI 日志/奖励/配置指纹 | 本轮私有证据及安全导出 | 不参与授权 |

不引入指标/日志专用数据库或云对象存储。审计使用原格式/不变量验证，不重置状态或伪造物理清理；中心离线 audit 取得原写者租约，与运行服务互斥。具体容量、不可变交接和受保护路径条件见模块合同。

## 7. 部署、CI 和验证

中心与 Redis 使用独立非 root 账号/目录，宿主与私有辅助工具按实际权限运行，jailer 将 VM 降到精确非特权身份。证书、配置、可执行文件和目录身份由可信部署固定。可信时钟、Linux/KVM、文件/网络原语、存储引擎和管理员是明确外部前提，不是本批 Lean 结论。

### 单机三角色部署样例

[部署样例目录](../deploy)包含[中心配置](../deploy/center.example.json)、[宿主配置](../deploy/host.example.json)、[Redis 配置](../deploy/redis.example.conf)与对应的[中心](../deploy/sandproof-center.service)、[宿主](../deploy/sandproof-host.service)、[Redis](../deploy/sandproof-redis.service)服务单元。它们使用上述唯一产品入口，没有自动安装或初始化脚本。JSON 由原 Rust 配置解析与校验测试检查，不维护另一份配置规则；该检查不代表单元已经在 Linux/KVM 部署通过。

这些是**必须审核、填料后才可使用的输入样例，不是生产默认值**：示例为一个运行节点、两个资源位置、1 GiB 私有盘和 128 MiB 来宾内存，不包含私有构建助手。部署者须核对实际镜像形状、全部资源预算、摘要、envd/Firecracker/jailer 版本、地址与标识；全零产物摘要和空凭据不得沿用。缺凭据、受保护证书或原存储时拒绝启动，不能用 base 或旧格式补齐。公网示例明确关闭，不自动修改共享网络。

制备和身份要求：

1. 创建不具登录权限的 `sandproof-center`、`sandproof-redis` 两个独立账号。示例物理配置的 UID `1001/1002` 只是明确示例输入；先选定真实不重叠 UID/GID，再同步账号、服务单元和 `service_data.owner_uid`。不得与宿主 root 或 VM 的原 `uid_base` 范围重叠。先检查现有账号，不能自动占用或修改他人账号。
2. `/var/lib/sandproof-services` 由 root 管理为 0711，其 `center`、`redis` 子树分别为对应账号的 0700；`/var/lib/sandproof-host` 及其全部子树为 root/0700。路径必须原生、不可互相别名且同受管文件系统，不能靠放宽宿主目录让中心读 VM 状态。按样例先制备中心 `control/bin`、`control/tls`、`assets`、`writer`、`registry/inbox`、`registry/artifacts`；Redis `control/bin`、`data`；宿主 `control/bin`、`control/tls`、`control/bootstrap`、空 `db`、`state`、`jails`、`template-cache/inbox`、`template-cache/artifacts`。
3. 二进制来自同一固定源码与锁，制备后核 SHA-256 和两个 `contract-versions`；角色二进制为对应账号/0500，配置、密钥和 TLS 材料为对应账号/0600。宿主 bootstrap 镜像为 root 私有预置输入；中心 `assets` 保存单独、中心可读的相同不可变产物，宿主从空缓存通过真正 mTLS 下载后才登记可用，不把 bootstrap 目录当缓存事实。
4. 私有 CA 和两端叶证书由部署者制备，叶子分别包含 `center.test`、`host.test` 及客户端/服务端双用途；私钥不跨角色复制，只复制固定 CA/对端叶的公有 DER 并核摘要。安全生成 API 凭据、独立 Redis 凭据与至少 32 字节的中心令牌密钥；只写各角色私有配置。Redis 的 `control/users.acl` 必须显式制备：关闭 default 用户，给独占引擎的 `sandproof-center` 用户设置独立随机凭据及原存储所需权限（包括持久化配置检查）。缺 ACL 文件不能换成公开样例密码或无鉴权引擎。
5. 首次制备原 `physical-profile` 与缓存 `.cache.lock`，均为 root/0600、空的普通单链接文件；缓存和原 DB/状态/监狱目录必须是本次空身份。不要覆盖现有协调文件、账本、缓存或失败现场。物理配置显式计费实例/清理、宿主缓存、中心模板与登记、Redis/AOF、二进制和控制材料；原 Rust 激活检查全峰值的两倍余量。Redis `maxmemory` 不限制 AOF 总大小，重写与观察也不能被称为长期硬磁盘上限；`service_data.peak_bytes` 是物理预留，不是对 Redis 的写入限额。部署者须为独立 Redis 受管根落实已批准的磁盘限额及 AOF/重写/备份峰值、恢复检查，不能截断 AOF 或削减原预算造通过，也不重挂载整个宿主文件系统。
6. 原 VM cgroup 父目录 `/sys/fs/cgroup/sandproof-vms` 由可信 root 显式制备，必须在 `sandproof-host.service` 单元 cgroup **之外**，保持原 `host_resources` 与位置内存检查。服务控制内存/CPU及宿主总体余量使用同一批准服务预算和实际 cgroup 核验；这些单元不猜测 `MemoryMax` 或另外设磁盘默认值，上线前须明确配置相符的资源 drop-in。样例显式丢弃三服务 stdout/stderr 及 Redis 文件日志，放弃控制台诊断，不是已部署的监控；需要保留日志时须另审核有限收集/轮转及原物理预算，不把日志默认交给无限 journald。

把审核后的配置和单位文件装入样例所示固定路径，先以部署主机的 `systemd-analyze verify` 检查单位，再执行以下**首次**顺序；命令不生成材料或修复缺失状态：

```sh
sudo systemctl start sandproof-redis.service
sudo /var/lib/sandproof-host/control/bin/sandproof-host initialize --config /var/lib/sandproof-host/control/host.json
sudo -u sandproof-center /var/lib/sandproof-services/center/control/bin/sandproof-center initialize --config /var/lib/sandproof-services/center/control/center.json
sudo systemctl start sandproof-host.service
sudo systemctl start sandproof-center.service
```

只有真实健康、模板封存与持久回执后才能创建成功；公共 HTTP 200 本身不是就绪。单元只有 `serve`，没有 `ExecStartPre=initialize`；后续恢复只用同一配置重启，缺库或旧格式保留未知、拒绝，不再运行 initialize。公开接口只监听回环，部署者另提供可信 TLS 前置入口；不能把宿主/Redis 端口暴露给客户。

宿主收到 SIGTERM 后，原入口停止接入并等待 HTTP 连接、持久工作驱动和缓存工作排空。[systemd 的 process 停止语义](https://github.com/systemd/systemd/blob/main/man/systemd.kill.xml)只终止主进程，因此宿主单元刻意使用 `KillMode=process`，不采用 `mixed/control-group`、`ExecStop` 杀 VM 或删除资源；示例 60 秒控制服务停止期限到达后也只杀主进程。残留实例/助手仍占原身份和责任，不能把单元 stopped 当 Cleaned。该方式不让 systemd 管理全部子进程，必须由原持久观察/恢复核对它们；计划维护先走正常销毁并核精确清理，不依赖停服务清场。

在线就绪检查通过宿主的 `observe-center --config` 读取中心保护的实时投影，不停启服务；中心生命周期与模板慢工作使用独立有限名额。接口和期限的唯一合同见[模块合同](module-contracts.md)。

离线审计时显式停止对应写者，保留 Redis 运行；审计不修改状态、不初始化，也不代替 VM 物理清理：

```sh
sudo systemctl stop sandproof-center.service sandproof-host.service
sudo -u sandproof-center /var/lib/sandproof-services/center/control/bin/sandproof-center audit --config /var/lib/sandproof-services/center/control/center.json
sudo /var/lib/sandproof-host/control/bin/sandproof-host audit --config /var/lib/sandproof-host/control/host.json
sudo systemctl start sandproof-host.service sandproof-center.service
```

普通 CI 检查解析、组件与工具，不启动产品 VM。两级晋升采用同一个监督器、同一候选双产物/版本清单和固定两题原解答/评分，SDK 只指向中心。成功、失败、取消均核对原全局→本地映射与真实残留，不能用旧单服务报告满足新链路。DK 与两题范围见[验收清单](testing-plan.md)，部署与离线缓存见[CI 运行环境](ci.md)。

原纯本地核心证明只适用指定版本。新增中心、Redis、内部映射、两端网关和组装只有工程检查；后续分别证明决定、存储/派发、接收/执行及组合。并行宿主工作仍归 [P2 #284](https://github.com/ningZhang-cs/sandproof/issues/284)，本次不改变原串行算法。
