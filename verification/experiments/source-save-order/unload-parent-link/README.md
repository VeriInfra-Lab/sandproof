# 来源记录保存的首错与写入顺序

[证明工作流](../../../../docs/proof-workflow.md) · [首次卸载父事务](../../stage1-protocol/unload-parent-link/README.md)

本切片的 Rust 来源固定为 #1000 候选源码树 `2941c6b1`。来源记录的生产入口
`controller::lifecycle_inbox::source_work::codec::save` 先读取原库并核对配置、节点、
会话上限和容量。整组编码后，先核对已保存的责任，再调用真正的逐行写入。旧责任核对失败
时应返回 `Conflict`，因此写入失败只有在旧责任通过后才可能成为返回结果。

采用 #940 的真实 SQLite 正反夹具和 #942 的两份 Lean 视图。具名调用仅出现于隔离 Rust
视图。正常 `Result` 路径下的机械分段由固定源码审查和原生对照支持。Lean 定理适用于
该分段实际生成的 `save_using` 及其具名接口条件。两份具体抽取检查正确版与调序版的调用顺序、同一 SQLite 连接和原错误。三份类型化
抽取补充检查生产适配器实际传递原参数、直接转发原结果，父调用和独立视图的
`save_using` 生成体逐字相同。调序版真实编译并被同一 SQLite 断言拒绝，其 Lean 反例
还给出实际生成体返回 `Sqlite` 且不等于 `Conflict`。证明从具名接口结果出发，
首次匹配、逐行写入、JSON/SHA、SQLite 持久化和父事务组合有各自的接口义务。

## 固定源码复验

从同一干净候选工作区分别执行原普通功能入口及以下专用入口。全新克隆需要保留 #1000
的原 `3ac27735` 提交对象，便于复算 Git 归档。例如，可取原始 `refs/pull/1000/head`。普通测试和真实 SQLite
夹具在 ning，固定 Charon、Aeneas、Lean 内核和原注册服务检查在证明机。
输出目录必须全新。临时 Rust 构建目录退出前收尾，保留原源码归档、回执、日志和形式产物。

```bash
entry=verification/experiments/source-save-order/unload-parent-link/verify.py
SANDPROOF_ACCEPTANCE_ROLE=native bash scripts/python-entry.sh "$entry" \
    --output "$native_evidence" native --work "$private_build_directory"

SANDPROOF_ACCEPTANCE_ROLE=formal bash scripts/python-entry.sh "$entry" \
    --output "$formal_evidence" formal --native "$native_evidence" \
    --tools "$fixed_aeneas" --lake-packages "$fixed_lake_packages" \
    --lean-mcp "$fixed_lean_mcp" --lock "$SANDPROOF_LOCK_FILE"
```

ning 的原生切片收据绑定当前候选、原始 Rust 来源树及三份真实编译和断言记录。两个形式
视图分别核对 17 份实际 Lean 文件，正例 13 条、调序版 3 条审计。整批 Stage1 与公共
证明注册仍由其各自的正式入口登记。
