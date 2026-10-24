# 原捕获动作与已发布 bundle 观察

本组件对应 [V6 详细计划](../../../docs/formal/06-host-execution.md)，使用生产 `sandbox-runtime` 和 `sandbox-node` 源码，不创建替代控制器。

`Runtime::pause_original_vm` 单独执行原 Pause；V5 持久登记后，唯一 `capture_paused_original` 入口接原私有 paused 句柄执行 Full/Seal 并验证原 VM 仍 Paused，结果只表示材料事实。`observe_stored_capture` 重开同一原捕获键并校验目录/FD、所有原文件字节和 manifest，不发捕获或停止动作。文件观察、V5 持久接管和允许停止是不同事实；真实生产派发必须接 V5 的原持久工作。

普通检查只在规定 Linux 普通测试环境、原普通重锁下运行：

```sh
cargo fmt -p sandbox-runtime -p sandproof-node --check
cargo test -p sandbox-runtime -p sandproof-node --lib --locked
cargo clippy -p sandbox-runtime -p sandproof-node --all-targets --locked -- -D warnings
```

真实错误版在同一源码的私有副本中，仅将 `full_snapshot_observation.rs` 的实际测量投影比较改为 `if false && manifest != manifest_from_measurements(binding, profile, measured)?`，随后运行原断言：

```sh
cargo test -p sandproof-node --lib --locked full_snapshot::tests::stored_capture_must_match_actual_files_and_the_saved_manifest_digest
```

正确版必须通过；错误版必须先成功编译，再由相同的文件内容一致性断言失败。缺工具、编译失败和超时不算反例；保留两版源码摘要、编译来源和原日志，不能覆盖正确版或清理共享缓存。额外/缺失文件、符号/硬链接、权限、超额、原文件/目录替换及原捕获键与后续操作键混淆另有生产路径回归。

本批没有新的 VM 试验，也没有完整新增调用链的机器对应定理。先前真实 Full/Seal 证据只适用于其精确原候选，不能直接当成新入口实测；既有文件接受守卫证明不能扩张为目录取得、读取、manifest、持久接管或停止的整体证明。新 KVM 批次需要精确候选和范围准入，不能使用 test-discard 冒充产品 unload。

原来源回归覆盖当前 claim 在 launch gate 前保留原宿主 boot、Prepared/Activated 完整字段往返；缺 boot 或 null 的旧开发字节必须拒绝，不保留旧形状接纳路径。`source_process_identity` 的 expected_resource 必须来自原 action。另一个真实错误版只把 `sandbox-node/src/linux/source_process.rs` 的 `self.resource != *expected_resource` 条件禁用，运行原测试 `linux::source_process::tests::foreign_command_resource_and_malformed_source_are_refused`，必须成功编译并触发同一拒绝断言。两个错误版使用独立源码/原日志，不替换正确版。缺失来源不意味着进程已退出；缺字段记录保持拒绝，不自动补当前 boot，不支持滚动混合格式或旧开发数据迁移。
