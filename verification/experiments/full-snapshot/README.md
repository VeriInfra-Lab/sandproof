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
尚未完成：正式节点驱动与 manifest 的完整对应证明、受保护打开的完整抽取证明、真实 KVM 验收、
File LoadPaused/恢复及 GC。不同阶段的成绩分别报告。

布局使用真实 `SnapshotLayout::initialize/reopen`，不是测试自定义目录约定。首次创建与服务重开分开；重开允许 `v/s` 已有原材料，原绑定或目录身份不符则失败关闭。Full 捕获只接受该布局，运行清理只删原 VM，不删共同父目录或 `s`。

`extract.py` 选择真实生产 `snapshot_policy::accepts`，只在错误副本把 `links == 1` 改成 `links <= 2`。`proof.py --output <独立证据目录>` 在固定证明包装内重抽取完整 IR，逐字节核对未修改的 Lean/名称映射，构建后通过项目标准 Lean 服务检查五个实际文件和六项审计。它是局部组件入口，不替代完整项目验收。公共 Lake/验收注册由整合写者另行接入。

普通组件验收命令为 `cargo test -p sandbox-runtime -p sandproof-node --lib --locked` 和两个包的 `cargo clippy --all-targets --locked -- -D warnings`。实机探针先以 `cargo build -p sandproof-node --example full_snapshot_capture --locked` 编译；只有固定提交、镜像和资源准入审查后才能运行 `tests/system/full_snapshot_capture.py`。探针复用现有 CI/KVM 租约与普通重锁，原 VM 的测试丢弃单独执行；不运行整套题集，也不借保存失败自动 kill 原 VM。

节点 manifest 的实际测量投影和布局初始化/重开已有代码与组件回归，但完整对应证明、正式 V5 驱动/事务交接、受保护路径获取证明和实机验收仍是缺口；File LoadPaused、恢复权限与副本预算、资格和 GC 继续后置。守卫的局部证明不为这些缺口背书。
