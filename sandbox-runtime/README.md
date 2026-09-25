# Firecracker 运行层（B01；B02 网络原型）

[项目蓝图](../docs/project-blueprint.md) · [模块合同](../docs/module-contracts.md) · [系统验收](../docs/testing-plan.md)

此 crate 是节点执行器的起点，负责固定 Linux/KVM 主机上的 Firecracker/jailer 启停、进程身份观察和私有磁盘清理。可选的 B02 原型会为每台 VM 自动创建独立 Linux 网络空间和 TAP，仅配置一条 `172.16.0.0/30` 私有链路，不添加默认路由；停止并核对精确 VM 进程后，按已记录的网络空间 inode 回收。它**尚未**部署 envd，也不提供 E2B API、命令/文件通道或多租户授权；`vmm_running` 仅表示 Firecracker 报告 VM 正在运行，**不等于来宾服务就绪，更不能直接发布控制器的 Running**。后续分别由 [#6](https://github.com/ningZhang-cs/sandproof/issues/6)、[#8](https://github.com/ningZhang-cs/sandproof/issues/8)、[#7](https://github.com/ningZhang-cs/sandproof/issues/7) 和 [#12](https://github.com/ningZhang-cs/sandproof/issues/12) 接通。

运行器要求调用者为 root，jailer、Firecracker、内核、基础 ext4 和 jailer 基目录的每级路径均由 root 拥有且不可由其他用户修改。基础盘在每次启动时复制到独立 jail 中；调用者必须为不同 VM 分配不同非 root UID/GID。当前 CLI 是开发诊断工具，不是常驻控制服务；从短命 SSH 会话单独执行 `launch` 不能保证 VM 在会话结束后继续存活。

Linux 实机可运行 `cargo test -p sandbox-runtime --locked` 和 `cargo clippy -p sandbox-runtime --all-targets --locked -- -D warnings`。准备固定镜像及 root 拥有的 jailer 目录后，使用唯一的全小写资源编号执行：

```sh
sudo ./target/debug/sandproof-vm smoke \
  <资源编号> <jailer绝对路径> <firecracker绝对路径> \
  <root拥有的jailer目录> <root拥有的内核绝对路径> \
  <root拥有的基础ext4绝对路径> <独占的非root UID> <独占的非root GID>
```

`smoke` 在同一个进程内启动 VM、读取串口日志中的来宾启动标志、按 PID 句柄确认精确进程退出，然后删除该编号的私有 jail/磁盘。编号占位记录及仅 root 可读的日志保留，防止旧编号被复用。`observe`、`stop`、`cleanup` 接收同样参数，可检查或收尾确切编号；失败/超时不自动分配第二个 VM。部署前还需要解决崩溃在进程创建与记录落盘之间的孤儿识别，以及常驻运行服务、来宾健康检查和实际资源/网络约束。

若需复现实机私有链路，用同样参数调用 `smoke-net`，并在末尾追加 `/run/netns/<同一资源编号>`。此诊断模式使用代码中固定的**公开测试用**令牌摘要，只能验证 Firecracker 的网络和 MMDS 配置，不能作为正式服务凭据。运行器自行创建网络空间和 TAP，验证来宾可达后按编号清理网络空间和私有磁盘；基础镜像不被修改。实测过的网络模式仍依赖 CI 镜像预装的 `fcnet.service`，它不是可部署任务镜像。

失败清理尚不完整：若进程在记录写入前或网络空间创建后、inode 记录写入前崩溃，编号会保留给人工核对，不能自动重试或宣称资源已回收。网络模式还未接入控制器持久动作、来宾服务和 SDK 数据通道，因此 #6、#8、#7 的验收都不能因此勾选。
