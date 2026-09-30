# Firecracker 运行层与来宾联调（B01/B02）

[项目蓝图](../docs/project-blueprint.md) · [模块合同](../docs/module-contracts.md) · [系统验收](../docs/testing-plan.md)

此 crate 负责固定 Linux/KVM 主机上的 Firecracker/jailer 启停、进程身份观察和私有磁盘清理。每台 VM 有独立 Linux 网络空间和 TAP；VM 启动前先安装默认拒绝规则，保留经鉴权的管理通道。可选 IPv4 出口使用每台 VM 独立的链路、地址、宿主 INPUT/FORWARD 规则和两层 NAT；先持久记录资源意图，安装并观察宿主规则，再准备来宾策略，最后原子打开来宾闸门。平台禁区高于用户允许，用户允许高于用户拒绝；动态宿主本机地址再由宿主路由判断阻断。观察绑定网络空间 inode、链路 ifindex、地址、路由和完整规则。配置中断时，未打开闸门的部分资源可由精确动作对账并清理；已准备的配置只在重新观察后继续激活。停止并核对精确 VM 进程后回收出口与私有资源。隔离流量与固定 SDK→真实 Firecracker→测试专用外部网络空间的选定场景通过；这不是任意公网、DNS 隧道、并发故障或完整 #10 验收。`vmm_running` 仅代表虚拟机监控器运行，不能直接发布控制器的 Running。相关工作由 [#6](https://github.com/ningZhang-cs/sandproof/issues/6)、[#7](https://github.com/ningZhang-cs/sandproof/issues/7)、[#10](https://github.com/ningZhang-cs/sandproof/issues/10) 和 [#12](https://github.com/ningZhang-cs/sandproof/issues/12) 跟踪。

[出站规则隔离流量测试](tests/egress_traffic.rs)在测试专用 Linux 主机上以 root 手动运行：`cargo test -p sandbox-runtime --test egress_traffic -- --ignored --nocapture`。它验证来宾侧允许、拒绝、平台禁区、IPv6 封闭及管理回包。[出口安装与恢复测试](src/uplink.rs)另验证完整来宾模拟网络空间→宿主→外部模拟网络空间的允许与拒绝数据包、闸门激活前的封闭、准备后恢复、安装中断后的精确清理。后者需临时开启宿主 IPv4 转发并在测试后恢复；不得并行运行或当作公网/真实 VM 的验收。服务配置还须显式 `public_ipv4_enabled=true`，宿主转发已启用，且新构建的固定 `base` 镜像包含来宾默认路由。默认配置仍拒绝公网上线请求。

运行器要求调用者为 root，jailer、Firecracker、内核、基础 ext4 和 jailer 基目录的每级路径均由 root 拥有且不可由其他用户修改。基础盘在每次启动时复制到独立 jail 中；调用者必须为不同 VM 分配不同非 root UID/GID。当前 CLI 是开发诊断工具，不是常驻控制服务；从短命 SSH 会话单独执行 `launch` 不能保证 VM 在会话结束后继续存活。

Linux 实机可运行 `cargo test -p sandbox-runtime --locked` 和 `cargo clippy -p sandbox-runtime --all-targets --locked -- -D warnings`。准备固定镜像及 root 拥有的 jailer 目录后，使用唯一的全小写资源编号执行：

```sh
sudo ./target/debug/sandproof-vm smoke \
  <资源编号> <jailer绝对路径> <firecracker绝对路径> \
  <root拥有的jailer目录> <root拥有的内核绝对路径> \
  <root拥有的基础ext4绝对路径> <独占的非root UID> <独占的非root GID>
```

`smoke` 在同一个进程内启动 VM、读取串口日志中的来宾启动标志、按 PID 句柄确认精确进程退出，然后删除该编号的私有 jail/磁盘。编号占位记录及仅 root 可读的日志保留，防止旧编号被复用。`observe`、`stop`、`cleanup` 接收同样参数，可检查或收尾确切编号；失败/超时不自动分配第二个 VM。启动门控进程在精确 PID 与启动时刻原子落盘并同步后才执行 jailer；控制进程在此之前退出会关闭门控管道，使 Firecracker 不会启动。常驻运行服务由 `sandbox-server` 经 `sandbox-node` 接入；已有真实来宾健康/鉴权检查、禁网、选定公网/静态 IPv4 出站和回执丢失恢复证据。CLI 的启动标志检查不能替代这些服务验收。实际 CPU/内存限制须显式配置下述 `host_resources`；完整恢复竞争、通用互联网与最终部署预算仍待对应任务验收。

若需复现实机私有链路，用同样参数调用 `smoke-net`，并在末尾追加 `/run/netns/<同一资源编号>`。此诊断模式使用代码中固定的**公开测试用**令牌摘要，只能验证 Firecracker 的网络和 MMDS 配置，不能作为正式服务凭据。运行器自行创建网络空间和 TAP，验证来宾可达后按编号清理网络空间和私有磁盘；基础镜像不被修改。实测过的网络模式仍依赖 CI 镜像预装的 `fcnet.service`，它不是可部署任务镜像。

失败清理仍有边界：门控解决“VM 已运行但无精确进程记录”的窗口；网络空间创建前先同步写入资源意图，若在 inode 记录写入前崩溃，恢复时须用持久操作计划、槽位绑定和资源意图共同核对，确认没有进程记录、虚拟机 jail 或空间内进程后，才能精确删除该网络空间并留下不可复用的编号。有精确进程记录但进程已退出的准备动作，则先清理独立 jail/磁盘及网络空间，再落盘失败标记。进程仍存活、身份不符或资源状态无法判定时继续保持未知，不猜测停止。测试脚本的 `finally` 收尾不能替代服务崩溃恢复。

## 宿主 CPU 与内存限制

服务私有配置可显式提供 `host_resources`；CLI 诊断使用同结构的 `SANDBOX_HOST_RESOURCES` JSON 环境变量。缺省仍保留历史诊断模式，**不能据此声称宿主资源已受限**。此接入已通过选定真实内核、VM/SDK 和回执恢复测试，提交、配置与边界记录在[版本基线](../references/v1-baseline.json)；不能由基础 CI 的忽略用例推断通过。

```json
{
  "host_resources": {
    "cgroup_parent": "/sys/fs/cgroup/sandproof-node",
    "memory_max_bytes": 805306368
  }
}
```

这是 512 MiB 来宾的测试配置示例：`memory_mib` 仍表示来宾 RAM，`memory_max_bytes` 表示整个 Firecracker 的宿主内存上限，包含 VMM 开销，必须大于来宾 RAM 且按 4096 字节对齐。示例不是最终任务预算。CPU 上限按 `vcpu_count × 100000 / 100000` 微秒配置到 `cpu.max`，包含 VMM 的处理开销；不表示独占物理核或调度时延保障。每组关闭额外 CPU burst 和宿主 swap，启用整组 OOM 处理。它不限制整个控制服务/relay 的内存，不提供宿主物理容量预留，磁盘和准入预算仍需各自验收。

部署者预先建立一个独占、root 拥有且模式为 0700 的 cgroup v2 父目录，保证它为空的 domain，向子组委派 CPU/memory 控制器。根或共享祖先须由宿主配置事先提供这些控制器；运行器不会修改共享祖先。每个部署须保持父目录与配置稳定，不能在仍有记录的 VM 存活或待清理时更换父目录、取消配置或改预算。

运行器在放行 jailer 前持久记录创建意图、宿主 boot ID、父目录和子组 inode、限额，再设置并读回限制。jailer 进入这个已配置的精确子组，不临时创建未记录的分组。运行观察核对限额、inode 与 PID 的实际 cgroup 归属；异常不会取得成功观察。停止仍使用原 PID/start-time 的 pidfd；清理要求精确身份且分组无任务，只删除该空子组，保留父目录。创建后 inode 尚未落盘的中断，仅在独占分配器的持久意图匹配、没有启动记录或 jail、分组为空时收尾；删除前补存 inode，防止下一次清理误处理同名替代组。缺少首个槽位、终止标记、丢失清理回执和服务启动盘点也检查 cgroup，不能只因磁盘已删除就报告资源全部回收。

内核专用用例需要独占的父目录与 root scratch 目录，通过 `SANDPROOF_TEST_CGROUP_PARENT`、`SANDPROOF_TEST_SCRATCH` 指定，然后手动执行 `cargo test -p sandbox-runtime linux::cgroup::tests::real_ -- --ignored --nocapture --test-threads=1`。CPU 竞争探针要求至少两个可用宿主 CPU；失败或缺环境不是通过。节点回执恢复用例另通过 `SANDPROOF_TEST_HOST_RESOURCES` 接入同结构配置，仍须按每轮精确资源身份审计残留。上述新增 Linux 资源管理没有现有 Lean 对应证明。

## 固定 envd 与真实 SDK 测试

### 隔离候选构建实验（#123，未接入用户 API）

[`tools/template_worker.py`](../tools/template_worker.py) 必须由外部监督器放到新建、限额、独占且可销毁的构建 VM 中执行；不能在宿主运行它处理用户输入，也不能只用 rootless 替代 VM 边界。调用需显式提供输入归档、全字段预算请求、受信任工具路径/SHA-256 清单和分配 VM 的 boot_id。工作器使用标准 [BuildKit](https://github.com/moby/buildkit/tree/v0.28.0) 与 [umoci](https://github.com/opencontainers/umoci/tree/v0.6.0)，不连接宿主 Docker；BuildKit 的 host 网络模式仅指这台禁网构建 VM 内部，不改变宿主网络权限。工具、输入及缓存都属于该项工作，不能跨租户复用。

[`tests/system/template_build.py`](../tests/system/template_build.py) 是可信操作方的离线最小实验，所需 `--tools` 目录包含 `buildctl`、`buildkitd`、`buildkit-runc`、`umoci`、固定 `envd` 与静态 `busybox`。前三项来自官方 `buildkit-v0.28.0.linux-amd64.tar.gz`（SHA-256 `bc5a158c97c1f7ace1af81c375e8c28bd27e6afeb283fc5b3bfb10777f48fc83`）；umoci 来自官方 `umoci.linux.amd64` v0.6.0（SHA-256 `b51c267ec394499e42c6fde47f240b7b7dba57ea49df0b5acd304378b82a3b71`）。验证摘要后只提取构建所需三项二进制，不把其他架构模拟器放到宿主 PATH。envd/BusyBox 与宿主文件系统工具指纹另随实测清单记录。工作器预算与运行服务额度不是同一配置；最终独立构建准入服务尚未实现。

实验运行还需要 `--server`、`--jailer`、`--firecracker`、`--kernel`、含 Python/文件系统工具的 10 GiB 测试 `--rootfs`、root 独占 `--scratch-parent`、空且已委派的 `--cgroup-parent` 和独占 `--uid-base`；无公共网络、模型费用或生产配置。输出候选磁盘不是 Ready 模板，ENV/WORKDIR/USER/CMD 等仅记录、未自动启用。真正用户入口和发布检查仍待下一批接入；合同与边界见[模块合同](../docs/module-contracts.md)，实际测试提交见[版本基线](../references/v1-baseline.json)。

### 显式预制多模板

常驻服务配置可选 `templates` 目录，结构与检查以[模块合同](../docs/module-contracts.md#模板服务--创建准入1236)及[描述类型](../controller-service/src/templates.rs)为准。这是受保护的内核/ext4 输入，不是 Dockerfile/OCI 构建入口。必须同时配置 `host_resources`；每张模板的 `host_memory_bytes` 覆盖全局默认内存上限，CPU 上限由该模板 vCPU 数推导，父目录仍固定。建议把 `base` 别名指向一张明确共享的预装模板，保留原 SDK 的便利默认值。未配置目录时沿用旧 `base` 参数。

部署者先批准每张镜像的摘要和长度、验证其可启动，再填完整的 `TemplateBinding`，不得把未知名称统统映射为 base。描述例子和双模板运行入口见[实机探针](../tests/system/server_templates.py)；脚本只复制已批准镜像，不能替代镜像构建或认证。旧实例清理、V8 数据迁移、编号不可改写及别名筛选规则见模块合同。源镜像丢失不妨碍已有私有 VM 的观察/停止/清理，但将拒绝新启动；不得把磁盘复制等价为共享卷或快照。

[`tests/system/base-inputs.json`](../tests/system/base-inputs.json) 固定新的离线 base 制备输入：Firecracker 官方 Ubuntu 24.04 SquashFS、软件包清单、6.1.176 内核，以及固定源码构建的 envd 0.9.0。URL、长度和 SHA-256 随代码复核；本工具不跟随 latest，仍不接收用户 Dockerfile/OCI。通用构建/转换由计划中的 [#123](https://github.com/ningZhang-cs/sandproof/issues/123) 单独交付，整体在证明范围外，见[覆盖记录](../docs/proof-coverage.md)。本配置用于可复现联调，其历史证据不能视为通用模板或生产默认值验收。

准备阶段按清单下载并校验输入，解开清单固定的 Go 工具链和 envd 源码，提前缓存 `packages/envd` 所需的 Go 模块（`go mod download`，保留 go.sum 校验）。在源码的 `packages/envd` 目录使用以下构建参数；离线时设置 `GOPROXY=off`，缺少依赖直接失败：

```sh
env GOENV=off GOFLAGS= GOTOOLCHAIN=local GOAMD64=v1 GOEXPERIMENT= \
  CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOPROXY=off \
  /absolute/go1.27.1/bin/go build -trimpath -buildvcs=false \
  -ldflags '-X=main.commitSHA=f455fd5 -s -w -buildid=' \
  -o /absolute/envd
```

该配方在两个独立源码目录构建得到相同的清单内 envd 指纹。它与下述历史 CI 二进制的指纹不同，不能据此沿用旧镜像的实机结论。镜像组装由 [`tools/build_base_guest.py`](../tools/build_base_guest.py) 执行：

```sh
sudo python3 tools/build_base_guest.py \
  --squashfs /absolute/ubuntu-24.04.squashfs \
  --envd /absolute/envd --packages /absolute/ubuntu-24.04.manifest \
  --output /root-owned/new-base.ext4 > artifacts/base-build.json
```

输出目录及其祖先必须由 root 拥有、不可被其他用户写入且不是符号链接；输出文件必须尚不存在。工具先把三个输入复制到私有目录并校验指纹，再解包、组装 1 GiB ext4；不挂载镜像、不执行来宾代码、不访问网络。保留 root/ubuntu 用户供 envd 选择运行身份，恢复上游 all-root SquashFS 中 ubuntu home 的正确归属，锁定登录密码、移除继承的 SSH 密钥和串口自动登录配置、屏蔽 SSH 服务，并清空共享机器标识。安装 envd、启动服务和包清单后，用 debugfs 检查内容与权限、e2fsck 检查文件系统；仅成功时输出 JSON 指纹报告并将镜像设为只读。失败的精确输出路径保留诊断，没有成功报告的文件不能作为已验收 base。

新构建的固定镜像在 `fcnet.service` 配好 `eth0` 后、启动 envd 前，设置到 `172.16.0.1` 的默认路由。旧镜像没有这条路由，即使宿主出口测试通过也不能用于公网模式验收；公开模式只对已重建并重新记录指纹的镜像开放。显式禁网仍由宿主闸门强制，不能靠来宾路由是否存在判断安全。

报告记录 profile、构建脚本、宿主文件系统工具及其配置、输出磁盘的指纹。固定输入与文件时间不代表不同宿主工具或 inode 创建时间下磁盘字节完全一致；每次输出都须单独绑定预检和实机证据。1 GiB 是这份联调镜像的逻辑容量，不是已测定的任务配额、宿主物理磁盘预留或内存保障。系统调用与 envd 均不在现有 Lean 证明范围内。

历史 CI 配方 [`tools/build_ci_guest.py`](../tools/build_ci_guest.py) 只接收已固定指纹的 Firecracker CI 基础盘和 envd 二进制。它新建 ext4 副本，用 debugfs 安装程序及开机服务，并检查文件内容、启动链接和文件系统；不挂载镜像、不运行来宾程序。基础盘和已有输出均不被改写。envd 源码固定为 `e2b-dev/runtime@f455fd58fa3f289e78ce1de690c35968d8d29322`，程序报告版本 `0.9.0`；构建时使用 Go 1.27.1、Linux amd64、CGO 关闭。具体二进制与基础盘 SHA-256 由脚本固定。输出镜像包含新 inode 时间戳，脚本记录每次输出指纹，不承诺磁盘字节逐次完全相同。

```sh
python3 tools/build_ci_guest.py --base /absolute/base.ext4 \
  --envd /absolute/pinned-envd --output /absolute/new-guest.ext4
```

将输出放入运行器要求的 root 拥有、不可由其他用户修改的目录后，在独立 Linux/KVM 测试机运行：

```sh
uv sync --locked --project tests/compat
cargo build -p sandbox-runtime --locked
sudo tests/compat/.venv/bin/python tests/system/guest_smoke.py \
  --runtime ./target/debug/sandproof-vm \
  --jailer /absolute/jailer --firecracker /absolute/firecracker \
  --chroot-base /absolute/jailer-directory --kernel /absolute/vmlinux \
  --rootfs /absolute/new-guest.ext4 --uid-base 22001
```

常驻节点初始化 envd 时固定默认用户为 root，但不覆盖其工作目录解析：省略 cwd 时使用当前请求所选用户的 home，显式 cwd 仍优先。这样普通用户命令不会因继承 `/root` 工作目录而在执行前失败；同步和异步完整服务探针覆盖这两条路径。

测试机须预留指定的两个非 root UID/GID，且不能同时运行另一轮使用相同 UID/GID 的测试。脚本为两个同时存活的 VM 分别生成随机令牌，启动前只把摘要放入 MMDS；原始令牌经测试进程标准输入交给私有空间内的探针，不出现在命令行或结果日志。先确认错误令牌不能抢先初始化，再完成合法初始化；原 SDK 验证命令的目录、环境变量、用户、输出、退出码、后台等待，以及文件读写和目录查询。异步 SDK 路径另行实测，两个 VM 对同一系统路径的写入必须互不影响。最后核验同一进程停止、私有盘和网络回收，以及基础盘指纹不变。

边界探针还检查命令流超时不被当作虚拟机销毁、显式终止、断线后用原进程编号重连、约 1 MiB 标准输出与 256 KiB 错误输出，以及宿主与来宾同名绝对文件路径互不污染。固定 envd 0.9.0 对含空字符的文件路径返回 500；这个实测现象不是预期对外契约，后续网关必须在转发前拒绝空字符并返回明确客户端错误。`/proc/1/root` 和 `..` 路径访问仍位于来宾自己的文件系统，不能因为得到 200 就推断访问了宿主。当前探针验证隔离和可继续使用，不代表已实现网关的请求大小/输出上限。

禁网测试额外创建一个仅供本轮使用的外部网络空间和临时链路，先从 VM 的宿主网络空间确认该测试端点真实可达，再由来宾 root 设置静态路由尝试访问；即使转发路径存在，来宾流量仍须在 TAP 入站处被宿主规则拒绝。该实验只使用隔离的测试网络，不开放真实公网，也不改变宿主全局 IPv4 转发值。测试脚本会恢复 VM 网络空间原转发值并删除临时链路；异常留下的网络空间需要按本轮唯一名称检查，不得批量删除其他运行中的沙箱资源。

`launch-net`、`observe-net`、`stop-net`、`cleanup-net` 为该测试提供诊断入口，参数与 `smoke-net` 相同，但要求环境变量 `SANDBOX_ENVD_TOKEN_SHA512`。`launch-net` 保留串口日志，整轮测试应留在同一个受监督的宿主会话中。该测试直接使用 SDK 构造器连接已创建的来宾，**没有测试公共 create/kill API 或 Harbor**。上述脚本也已使用 `base-inputs.json` 的新 base 执行，精确提交和指纹见[版本基线](../references/v1-baseline.json)；历史 CI 配方与新 base 的证据分别记录。它不包含 Dockerfile/OCI 构建、完整外网策略或最终任务资源验收。新增运行层、制备工具和 envd 不在现有 Lean 证明范围内。


## 独立复现 B01 节点验收

[节点实机用例](../sandbox-node/tests/host_smoke.rs)可独立于公共 HTTP 服务执行，使用真实控制器日志、jailer、envd 与资源观察。先按[宿主预检](../docs/testing-plan.md#4-独立运行宿主预检)确认干净提交及固定产物，再构建该提交的测试可执行文件：

```sh
cargo test -p sandproof-node --test host_smoke --locked --no-run --message-format=json \
  > artifacts/host-test-build.json
python3 -c 'import json; print("\n".join(x["executable"] for x in map(json.loads, open("artifacts/host-test-build.json")) if x.get("executable")))'
```

将输出的 `host_smoke-*` 可执行文件复制到已复核的 root 独占目录，核对复制前后的 SHA-256，并记录源码提交。以下路径和 UID 是需由测试机操作员填写的占位值；UID 基数及后续 1000 个编号必须独占，不能与其他测试并行复用。每条用例单独建立空的 root 私有 state/jails 目录及新的 16 位小写十六进制 namespace；不能把多个用例共用同一数据库。

```sh
sudo env \
  SANDPROOF_TEST_NAMESPACE=<新的16位十六进制编号> \
  SANDPROOF_TEST_STATE_DIR=/root-owned/this-case/state \
  SANDPROOF_TEST_CHROOT=/root-owned/this-case/jails \
  SANDPROOF_TEST_JAILER=/root-owned/jailer \
  SANDPROOF_TEST_FIRECRACKER=/root-owned/firecracker \
  SANDPROOF_TEST_KERNEL=/root-owned/vmlinux \
  SANDPROOF_TEST_ROOTFS=/root-owned/base.ext4 \
  SANDPROOF_TEST_UID_BASE=<独占UID基数> \
  SANDPROOF_TEST_HOST_RESOURCES='{"cgroup_parent":"/sys/fs/cgroup/sandproof-node","memory_max_bytes":536870912}' \
  /root-owned/host-smoke --exact repeated_stop_and_cleanup_preserve_another_live_vm \
  --ignored --nocapture
```

该用例同时启动两台 VM，重复停止、清理其中一台，并在重建 worker 后再次提交相同动作。每步必须保留另一台的 PID/启动时刻、网络/磁盘/cgroup inode，且通过真实 envd 鉴权读取；最后两台均停止、资源均清理。节点测试的来宾 RAM 为 256 MiB，上例 VMM 宿主上限为 512 MiB，仅是测试配置。失败保留现场，不靠析构函数掩盖失败后的残留。

同一入口分别运行 `create_and_kill_use_real_host_receipts`、`real_worker_process_exit_reconciles_prepare_and_stop`、`real_worker_sigkill_reconciles_prepare_and_stop`、`real_worker_sigkill_reconciles_partial_cleanup`、`partial_prepare_without_first_slot_is_reclaimed_without_launch` 和 `partial_prepare_network_intent_is_reclaimed_by_exact_identity`；每次更换独立目录和 namespace。这些用例覆盖已固定的故障窗口，不代表任意位置崩溃。基础检查还运行 `missing_kvm_is_reported_explicitly`，确认缺失 KVM 路径返回专门错误；宿主预检负责检查实际 `/dev/kvm` 能否打开及 API 版本。

报告须记录命令、退出码、测试二进制/配置摘要与固定镜像指纹。测试退出后，独立按 `.claims` 内记录的 PID/启动时刻检查进程退出，核对本轮 jail、网络空间和 cgroup 已消失、没有进程仍持有记录的网络空间 inode，并重新计算基础盘哈希。保留 root 私有编号记录和诊断日志，导出时只提取明确允许的结果字段；不发布 envd 令牌或整份私有配置。普通 CI 中这些实机用例是忽略项，不能把基础检查绿灯称为 KVM 通过。
