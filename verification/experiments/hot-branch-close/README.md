# 热分支窗口关闭与额度回收

[证明工作流](../../../docs/proof-workflow.md) · [覆盖记录](../../../docs/proof-coverage.md)

本组件固定候选 `1388de447126e025c9080557261d41baca48bbb4`，对应已审阅的 #1030 与 `integration` 合并树及宿主路由回归。`msra` 上的 Charon 抽取覆盖分支关闭状态、额度占用、生命周期窗口的 Rust 事务路径，以及 `retire_closed_parent` 中的 SQL 调用。Aeneas 生成体和已登记的库、模型与审计保持原输入。

`CloseProof` 检查原分支状态转换与兄弟凭据保持。`CloseSql` 在明示的 SQLite 活动连接、扫描、删除、更新和 A4 提交条件下，证明正常提交看到完整新镜像，正常回滚看到原镜像，未知提交可以看到两种完整镜像之一。三份独立原生构建检查正常、提前提交错误版和恢复后的同一真实 SQLite 断言。错误版须在数据库重开后的原额度断言被拒绝。

两 BLOB 更新的生产侧和消费侧分别引入原生成视图。`BlobInvocation` 携带两边共同核对的原执行身份、SQL、两个有序 BLOB 和受影响行数。同一次物理调用、同一笔活动事务和正确的更新行状态是显式前提。

退休抽取检查器沿 `query_map` 的第三个实参和编译器选择的 `FnMut` 字典定位原回调，核对对同一个回调行的 `Row::get<usize, Vec<u8>>` 调用按顺序读取第 0 到 4 列，并将列号写入 `row_callback_columns`。这一字段记录调用点、索引和类型。五列成功值怎样组成返回元组、首错停止、实际游标与 BLOB 值、按原操作键 DELETE 的准确行状态、受保护重开、宿主授权和资源释放仍需各自的实现对应。

## Linux 复验

使用 `msra` 的数据盘、项目固定的原证明工具和独立输出。按工作流取得原 Stage1 锁及可继承的文件描述符，设置 `SANDPROOF_ACCEPTANCE_ROLE=formal`、`SANDPROOF_LOCK_FILE` 与 `SANDPROOF_LOCK_FD`。两个入口各在四个已分配 CPU 上运行。原生入口需要离线 Cargo 缓存。`--checkout` 指向含固定候选及其父节点的干净检出。

```sh
component=verification/experiments/hot-branch-close
candidate=1388de447126e025c9080557261d41baca48bbb4
taskset -c "$proof_cpus" "$proof_python" -B "$component/replay.py" \
  --approved-linux --with-window --with-blob-bridge --candidate "$candidate" \
  --checkout "$candidate_checkout" --tool-root "$proof_tool_root" \
  --packages "$formal_packages" --output "$proof_output"
taskset -c "$native_cpus" "$proof_python" -B "$component/native_witness.py" \
  --approved-linux --candidate "$candidate" --checkout "$candidate_checkout" \
  --output "$native_output"
```

保留原源码归档、生成物、Rust 和 Lean 逐文件日志、证明服务读回及 `result.json`。本组件的记录与完整登记证明及真实 KVM 验收分别查阅。
