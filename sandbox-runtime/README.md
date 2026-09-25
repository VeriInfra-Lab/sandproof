# Firecracker 运行层（B01）

[项目蓝图](../docs/project-blueprint.md) · [模块合同](../docs/module-contracts.md) · [系统验收](../docs/testing-plan.md)

此 crate 是节点执行器的起点，只负责固定 Linux/KVM 主机上的 Firecracker/jailer 启停、进程身份观察和私有磁盘清理。它不提供 E2B API、envd、命令/文件通道、网络隔离或多租户授权；`vmm_running` 仅表示 Firecracker 报告 VM 正在运行，**不等于来宾服务就绪，更不能直接发布控制器的 Running**。后续分别由 [#6](https://github.com/ningZhang-cs/sandproof/issues/6)、[#8](https://github.com/ningZhang-cs/sandproof/issues/8)、[#7](https://github.com/ningZhang-cs/sandproof/issues/7) 和 [#12](https://github.com/ningZhang-cs/sandproof/issues/12) 接通。

运行器要求调用者为 root，jailer、Firecracker、内核、基础 ext4 和 jailer 基目录的每级路径均由 root 拥有且不可由其他用户修改。基础盘在每次启动时复制到独立 jail 中；调用者必须为不同 VM 分配不同非 root UID/GID。当前 CLI 是开发诊断工具，不是常驻控制服务；从短命 SSH 会话单独执行 `launch` 不能保证 VM 在会话结束后继续存活。

Linux 实机可运行 `cargo test -p sandbox-runtime --locked` 和 `cargo clippy -p sandbox-runtime --all-targets --locked -- -D warnings`。准备固定镜像及 root 拥有的 jailer 目录后，使用唯一的全小写资源编号执行：

```sh
sudo ./target/debug/sandproof-vm smoke \
  <资源编号> <jailer绝对路径> <firecracker绝对路径> \
  <root拥有的jailer目录> <root拥有的内核绝对路径> \
  <root拥有的基础ext4绝对路径> <独占的非root UID> <独占的非root GID>
```

`smoke` 在同一个进程内启动 VM、读取串口日志中的来宾启动标志、按 PID 句柄确认精确进程退出，然后删除该编号的私有 jail/磁盘。编号占位记录及仅 root 可读的日志保留，防止旧编号被复用。`observe`、`stop`、`cleanup` 接收同样参数，可检查或收尾确切编号；失败/超时不自动分配第二个 VM。部署前还需要解决崩溃在进程创建与记录落盘之间的孤儿识别，以及常驻运行服务、来宾健康检查和实际资源/网络约束。
