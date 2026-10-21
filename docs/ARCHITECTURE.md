# Sandproof 系统架构

[文档目录](README.md) · [项目蓝图](project-blueprint.md) · [模块合同](module-contracts.md) · [验收清单](testing-plan.md)

当前设计面向单固定宿主上的有状态卸载与恢复。integration `56b14d64` 已有中心/宿主、Redis/SQLite、执行/清理和网关代码；真实 snapshot 协议及下述新增状态仍待实现。结构不是已部署或已证明声明，准确证据见[覆盖记录](proof-coverage.md)。

## 1. 保留唯一产品链路

```text
客户：首次发送前取得操作键，授权查询原结果
                    │
                    ▼
sandproof-center ↔ 权威 Redis（意图、固定绑定、预留、结果）
                    │ 真实 mTLS 高级生命周期消息
                    ▼
sandproof-host ↔ 原 SQLite（本地状态、原操作、执行尝试、引用）
                    │
          sandbox-node / sandbox-runtime
             │                  │
      jailer / Firecracker    受保护本地 snapshot store〔待实现〕
      当前运行位置/私有盘     全量 RAM、设备状态、匹配磁盘、manifest
             │                  │
       固定来宾 / envd      独立于可复用运行位置，跨管理服务重启
```

中心是全局身份、用户授权、固定节点、意图与全局预留的权威；宿主原控制器是本机阶段、执行资格与回收的权威。只有一个有效中心写者，一个节点；允许多会话、多用户竞争。公开请求仍走中心，不提供绕过中心的验证专用宿主 API，不新造控制器。

## 2. 模块与状态所有权

| 实现 | 当前职责与本阶段增加内容 | 证明归属 |
|---|---|---|
| sandbox-api / sandbox-center-service | 解析、鉴权；新增带操作键的生命周期及授权结果查询 | V7；网关 V8 |
| sandbox-center-core | 意图、身份、准入、终止；新增 unload/resume 的全局预留转换 | V1 |
| sandbox-center-store | 原字节/CAS、耐久确认、outbox、恢复和有限历史 | V2 |
| sandbox-protocol / sandbox-host-service | 高级操作及原持久收件；新增协议版本和完整绑定 | V3 |
| controller-core | 本地阶段与纯决定；重审旧 pause/resume 和通用补偿 | V4 |
| controller-service | 原 SQLite、工作/回执、重启；新增 capture/claim/pin/retire 登记 | V5 |
| sandbox-node / sandbox-runtime | 原身份、围栏、启动停止；新增全量捕获、先暂停加载、激活、文件持有与精确删除 | V6 |
| 两端 gateway / sandbox-data-transport | 原授权、撤销及排空；恢复的新代重授权 | V8 |
| center / host 正式入口 | 同一配置、版本和真实驱动的组装 | V9 |

V5 管持久引用和是否允许清理，V6 管文件/进程事实；两者以显式事件连接。新增 snapshot store 是原宿主的组件，不是新的数据库或独立服务。模板构建仍在边界外，但运行态 snapshot 决策不在该排除范围。

## 3. 三条关键路径

暂停：持久接受并预留空间→关闭新访问并排空捕获相关效果→暂停原 VMM→捕获同一时间截面的 RAM/设备/磁盘→封存发布并登记完整 bundle→停止旧 VMM→确认运行资源可复用→保存并发布 Unloaded。捕获提交前失败必须保原唯一状态；提交未知先对账，不能先杀 VM。

恢复：核对会话/来源/寿命→原 SQLite 唯一认领并持久持有材料→新运行代装载为暂停→配置当前网络和凭据→先持久记录“可能执行”再激活→核验原候选就绪并发布。激活结果未知只恢复同一候选，不能销毁后再从旧快照装一台。

回收：撤销材料的恢复资格→关闭新引用入口→等待已登记引用及原 VMM 使用结束→记录精确删除责任→删除并核对→释放存储额度。恢复资格被消费不代表 RAM 文件不用了；File 后端可能在 VMM 运行期间继续读取它。详细状态、失败与并发规则唯一见[模块合同的生命周期合同](module-contracts.md#第一阶段生命周期合同)。

## 4. 存储与预算

| 对象 | 位置 / 权威 | 回收条件 |
|---|---|---|
| 全局会话、原操作、固定节点、全局预留、结果 | 原 Redis 控制块 | 对应事实结算；有限窗口过期规则，不凭超时删除未知责任 |
| 本地会话、执行尝试、捕获/认领/引用、待删工作 | 原 SQLite | 同一事务建立责任和工作；原结果确认后按协议退休 |
| 运行 VMM、网络、私有可写盘、运行位置 | 原受保护运行目录 | 原 VMM/助手退出且旧工作失效，按精确身份复用 |
| snapshot bundle | 新受保护本地存储，路径与运行位置分离 | 无恢复资格、无 pin/迟到任务可重新取用、原对象删除确认 |
| 初始模板 | 原受保护模板与有界缓存 | 保留既有合同；不把运行态 bundle 当模板缓存逐出 |

Redis、SQLite、文件提交和 VMM 动作不是一个事务。先登记计划/额度再发生外部效果，允许“文件存在但尚未登记成功”等中间状态；恢复按原意图认领/清理，不能靠目录扫描直接发布成功。物理观察与持久记录可以暂时不同，但每个可能存在的效果都要有责任人。

卸载只释放运行分量，保留会话、快照、结果和清理分量。恢复预留新私有盘、VMM 和旧 bundle 映射；下一次捕获还要计入旧 backing、新 bundle 与临时副本重叠峰值。具体限额是受控配置，不写死测试值。容量不足先拒绝，未知效果持续占账。

## 5. 数据面与来宾

中心和宿主各自核对 owner/session/incarnation、寿命、权限和撤销状态。暂停关闭新访问，排空会改变捕获结果的在途操作；需要维持的 guest 后台进程与传输连接分离，断开订阅不能隐式杀死它。V8 必须沿真实路径证明或实测此条件，不能假设整个自有 envd 正确。

恢复不保证旧 TCP/vsock 连接继续；客户端经新的当前授权重连。装载时先保持网络隔离和数据门关闭；必要的固定 guest 凭据更新钩子完成后才开放。保存状态中旧凭据不能因为 RAM 恢复而重新被接受。完整 PTY/文件/流协议证明后置，必要的撤销、排空、重授权和状态见证不能后置。

## 6. 扩展与实现差距

当前内部 `Intent` 仅 Create/Renew/Destroy；真实 `PrepareVm` 对 `source.is_some()` 明确拒绝；纯核心虽然有 Pause/Resume，但补偿和 manifest 计数器不满足本阶段完整快照合同。V3/V4/V5/V6 需要工程实现与新的表示关系，不能只接通枚举或迁移旧定理。

会话身份、运行代、capture、operation、物理位置分别编码；manifest 的内容摘要独立于旧逻辑计数器。协议版本和完整源/目标绑定为将来多宿主、增量链或 fork 留接口，但这些扩展不进入本次实现和证明。

以下保留既有部署办法，描述当前实现基线；新增 snapshot 目录、预算、schema 和 profile 尚未实现，不能照现有样例就声称已部署本阶段功能。阶段启用前由 V9 核对固定 CPU/内核/设备/文件系统与新协议。

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
