# Firecracker 运行层与来宾联调（B01/B02）

[项目蓝图](../docs/project-blueprint.md) · [模块合同](../docs/module-contracts.md) · [系统验收](../docs/testing-plan.md)

此 crate 负责固定 Linux/KVM 主机上的 Firecracker/jailer 启停、进程身份观察和私有磁盘清理。每台 VM 有独立 Linux 网络空间和 TAP，仅配置一条 `172.16.0.0/30` 私有链路。创建 TAP 和启动 VM 之前，在该空间内原子安装宿主控制的默认拒绝规则：只允许宿主发起的管理连接得到已建立连接的回包，其余来宾发起的 IPv4、IPv6 和 DNS 包均丢弃。网络回执核对空间 inode 与规则的完整内容；规则缺失或被放宽不会算配置成功。停止并核对精确 VM 进程后，按已记录的网络空间 inode 回收。`egress` 模块集中生成禁网规则，并预备按固定 SDK 的 IPv4 地址/网段允许、拒绝与公网默认值生成来宾侧规则；不要求用户增加协议或端口字段。平台禁区优先于用户允许，用户允许优先于用户拒绝。此规则生成器**尚未接入宿主出口、隔离观察与恢复**，不能作为已启用功能，也不能把目前的禁网模式称为完整 #10。配套脚本可制备固定 envd 来宾镜像，并用原 E2B SDK 实测来宾命令和文件操作；`vmm_running` 仅表示 Firecracker 报告 VM 正在运行，**不等于来宾服务就绪，更不能直接发布控制器的 Running**。相关工作由 [#6](https://github.com/ningZhang-cs/sandproof/issues/6)、[#8](https://github.com/ningZhang-cs/sandproof/issues/8)、[#7](https://github.com/ningZhang-cs/sandproof/issues/7)、[#10](https://github.com/ningZhang-cs/sandproof/issues/10) 和 [#12](https://github.com/ningZhang-cs/sandproof/issues/12) 跟踪。

[出站规则隔离流量测试](tests/egress_traffic.rs)在测试专用 Linux 主机上以 root 手动运行：`cargo test -p sandbox-runtime --test egress_traffic -- --ignored --nocapture`。它先在独立网络空间中证明公网样式地址、私网地址、宿主样式地址和 IPv6 目标原本有路，再安装生成的规则测试允许与拒绝，并核对宿主发起的管理回包及测试资源清理。所有虚拟链路两端都在本轮私有网络空间，转发只在该空间启用；不连接真实互联网、不启动 VM，也不等于控制器或 SDK 网络功能已验收。

运行器要求调用者为 root，jailer、Firecracker、内核、基础 ext4 和 jailer 基目录的每级路径均由 root 拥有且不可由其他用户修改。基础盘在每次启动时复制到独立 jail 中；调用者必须为不同 VM 分配不同非 root UID/GID。当前 CLI 是开发诊断工具，不是常驻控制服务；从短命 SSH 会话单独执行 `launch` 不能保证 VM 在会话结束后继续存活。

Linux 实机可运行 `cargo test -p sandbox-runtime --locked` 和 `cargo clippy -p sandbox-runtime --all-targets --locked -- -D warnings`。准备固定镜像及 root 拥有的 jailer 目录后，使用唯一的全小写资源编号执行：

```sh
sudo ./target/debug/sandproof-vm smoke \
  <资源编号> <jailer绝对路径> <firecracker绝对路径> \
  <root拥有的jailer目录> <root拥有的内核绝对路径> \
  <root拥有的基础ext4绝对路径> <独占的非root UID> <独占的非root GID>
```

`smoke` 在同一个进程内启动 VM、读取串口日志中的来宾启动标志、按 PID 句柄确认精确进程退出，然后删除该编号的私有 jail/磁盘。编号占位记录及仅 root 可读的日志保留，防止旧编号被复用。`observe`、`stop`、`cleanup` 接收同样参数，可检查或收尾确切编号；失败/超时不自动分配第二个 VM。启动门控进程在精确 PID 与启动时刻原子落盘并同步后才执行 jailer；控制进程在此之前退出会关闭门控管道，使 Firecracker 不会启动。部署前仍需完成残留资源自动对账、常驻运行服务、来宾健康检查和实际资源/网络约束。

若需复现实机私有链路，用同样参数调用 `smoke-net`，并在末尾追加 `/run/netns/<同一资源编号>`。此诊断模式使用代码中固定的**公开测试用**令牌摘要，只能验证 Firecracker 的网络和 MMDS 配置，不能作为正式服务凭据。运行器自行创建网络空间和 TAP，验证来宾可达后按编号清理网络空间和私有磁盘；基础镜像不被修改。实测过的网络模式仍依赖 CI 镜像预装的 `fcnet.service`，它不是可部署任务镜像。

失败清理仍有边界：门控解决“VM 已运行但无精确进程记录”的窗口；网络空间创建前先同步写入资源意图，若在 inode 记录写入前崩溃，恢复时须用持久操作计划、槽位绑定和资源意图共同核对，确认没有进程记录、虚拟机 jail 或空间内进程后，才能精确删除该网络空间并留下不可复用的编号。有精确进程记录但进程已退出的准备动作，则先清理独立 jail/磁盘及网络空间，再落盘失败标记。进程仍存活、身份不符或资源状态无法判定时继续保持未知，不猜测停止。测试脚本的 `finally` 收尾不能替代服务崩溃恢复。

## 固定 envd 与真实 SDK 测试

[`tools/build_ci_guest.py`](../tools/build_ci_guest.py) 只接收已固定指纹的 Firecracker CI 基础盘和 envd 二进制。它新建 ext4 副本，用 debugfs 安装程序及开机服务，并检查文件内容、启动链接和文件系统；不挂载镜像、不运行来宾程序。基础盘和已有输出均不被改写。envd 源码固定为 `e2b-dev/runtime@f455fd58fa3f289e78ce1de690c35968d8d29322`，程序报告版本 `0.9.0`；构建时使用 Go 1.27.1、Linux amd64、CGO 关闭。具体二进制与基础盘 SHA-256 由脚本固定。输出镜像包含新 inode 时间戳，脚本记录每次输出指纹，不承诺磁盘字节逐次完全相同。

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

测试机须预留指定的两个非 root UID/GID，且不能同时运行另一轮使用相同 UID/GID 的测试。脚本为两个同时存活的 VM 分别生成随机令牌，启动前只把摘要放入 MMDS；原始令牌经测试进程标准输入交给私有空间内的探针，不出现在命令行或结果日志。先确认错误令牌不能抢先初始化，再完成合法初始化；原 SDK 验证命令的目录、环境变量、用户、输出、退出码、后台等待，以及文件读写和目录查询。异步 SDK 路径另行实测，两个 VM 对同一系统路径的写入必须互不影响。最后核验同一进程停止、私有盘和网络回收，以及基础盘指纹不变。

边界探针还检查命令流超时不被当作虚拟机销毁、显式终止、断线后用原进程编号重连、约 1 MiB 标准输出与 256 KiB 错误输出，以及宿主与来宾同名绝对文件路径互不污染。固定 envd 0.9.0 对含空字符的文件路径返回 500；这个实测现象不是预期对外契约，后续网关必须在转发前拒绝空字符并返回明确客户端错误。`/proc/1/root` 和 `..` 路径访问仍位于来宾自己的文件系统，不能因为得到 200 就推断访问了宿主。当前探针验证隔离和可继续使用，不代表已实现网关的请求大小/输出上限。

禁网测试额外创建一个仅供本轮使用的外部网络空间和临时链路，先从 VM 的宿主网络空间确认该测试端点真实可达，再由来宾 root 设置静态路由尝试访问；即使转发路径存在，来宾流量仍须在 TAP 入站处被宿主规则拒绝。该实验只使用隔离的测试网络，不开放真实公网，也不改变宿主全局 IPv4 转发值。测试脚本会恢复 VM 网络空间原转发值并删除临时链路；异常留下的网络空间需要按本轮唯一名称检查，不得批量删除其他运行中的沙箱资源。

`launch-net`、`observe-net`、`stop-net`、`cleanup-net` 为该测试提供诊断入口，参数与 `smoke-net` 相同，但要求环境变量 `SANDBOX_ENVD_TOKEN_SHA512`。`launch-net` 保留串口日志，整轮测试应留在同一个受监督的宿主会话中。该测试直接使用 SDK 构造器连接已创建的来宾，**没有测试公共 create/kill API 或 Harbor**。镜像沿用 Firecracker CI 系统及其测试配置，不是生产任务镜像，也不包含 Dockerfile/OCI 构建、外网策略或完整资源限制验收。新增运行层、制备工具和 envd 不在现有 Lean 证明范围内。
