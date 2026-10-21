# Full snapshot 捕获/封存切片

本目录跟踪 `sandbox-runtime` 的真实 Full capture/原 FD 复制与封存。
普通测试只在普通测试机运行；抽取和 Lean 检查仅在固定证明运行时运行。
目前新增代码没有接入默认节点命令，不表示产品 pause/unload 已启用。

实际同步路径为 `Runtime::pause_original_vm` → `capture_full_into` →
`SnapshotStage::copy_artifact` → 节点核对完整 manifest → `publish`。
`publish` 提供文件和同步事实，不认证原始 manifest 的业务字段，也不授权 stop。
完整身份、profile 盘集合、许可和持久交接由 V3/V5 与节点连接，不能作为外部假设。

`SnapshotStage` 无删除/kill/thaw 的 Drop；任一错误都可能已留下部分材料。
尤其 rename 后的目录 sync 失败可能已可见，必须保留原 capture 意图，不能另造键
覆盖重试。已有目标一律拒绝自动收养；相同请求重放应先查原持久工作，冲突仍拒绝。

`snapshot_policy::accepts` 是真实复制路径调用的纯检查，检查原 Metadata 的
文件类型、属主、group/other 权限、单硬链接、正长度和预算。其局部证明不能替代
路径打开/原 FD 对应、冻结期间无写者、完整 manifest 或文件持久性合同。
后续抽取必须选择该实际函数，不另写验证用判断算法。

首 profile 只含原 `stage_and_boot` 配置的 rootfs→rootfs.ext4；bundle 映射
为 state.bin、memory.bin、disk-0.img。以后增加驱动必须连接 profile 全盘集合。
尚未完成：完整节点 manifest 连接、受保护打开的完整抽取证明、真实 KVM 验收、
File LoadPaused/恢复及 GC。不同阶段的成绩分别报告。
