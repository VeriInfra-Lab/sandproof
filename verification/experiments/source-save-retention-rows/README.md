# 完整旧责任检查与逐行保存

[证明工作流](../../../docs/proof-workflow.md) · [完整旧责任检查](../source-save-retention/README.md)

本切片固定生产源码 `60aad8af23e317f5b1ba913518603d1fda015f4c`。在同一生成类型中连接
#1033 的完整旧责任检查和 #969 的多行写入。SQL 提供者使用 #970 的精确提交
`8f94cb4dc947c09c46981931f5e7d98b144de66d` 中保留的 #956 与 #969 定义。
读取这些提交的原对象，避免跟随可变分支。全新克隆须保留本 PR、#1032、#945、#970 的原提交对象。

`original_save_retains_then_writes` 从原读取、头部、容量、编码及键查询结果出发。
对任意两份已解码完整记录，查同一完整键，按第一个 `SourceWork` 全文匹配核对旧事实。
失败返回 `Conflict`；通过时把同一连接及编码后的完整键/字节向量交给原多行 writer。
其返回是 #969 的 `writeList`。`first_row_error_after_complete_retention` 保留任意已完成前缀
后的首个错误，后续行无需条件。两边复用同一个 SQL 错误载体，既有较早拒绝定理继续适用。

正确与错误视图共享 `JointSpec.expected`。正常见证经原 SELECT、SHA 结果模型和 UPDATE
执行一行；仅按 binding 匹配的 Rust 错误版使同键不同配置的记录进入原 writer，产生与
同一合同相矛盾的结果。正反均以固定 Charon/Aeneas 重新抽取，父视图用于核对转发。
已审计的原 SQLite 定义及原循环证明在未修改的联合生成文件上复用。生产分段对应仍由精确
源码审查及原生测试支持。读取、容量、编码和集合构造为具名待证供给，标准键查询、SQL
原语与 SHA 采用各自既有条件。跨行物理效果、回滚、耐久提交和提交未知由 V5 继续连接。

## 证明机复验

`original_row_capture` 是 #956 的完整 `core` 捕获。缺失时可从 #970 的固定模块入口
`source_row_sql_composition.py --mode core` 重新获得。使用固定工具、四核、原共享重锁
和独立 `.lake/build`，两台机器的验收分工沿用工作流。

```bash
set -euo pipefail
entry=verification/experiments/source-save-retention-rows
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$capture_root"
for mode in correct binding-only parent; do
    bash scripts/python-entry.sh "$entry/joint_capture.py" --mode "$mode" \
        --output "$capture_root/$mode" --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/joint_check.py" --captures "$capture_root" \
    --row-reference "$original_row_capture" --output "$formal_output" --tools "$fixed_aeneas" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp" --lock "$shared_proof_lock"
```

## Ning 原生复验

用现有 `ci_retention allocate --kind acceptance` 为本 PR 的精确提交分配空工作目录。
运行现有普通验收及本切片。原版和组合版运行原 source-work 测试组及原 SQLite 保留断言。
组合版另外执行 #956 的真实既有行和最小空槽 writer 断言。错误版须成功编译，并命中原
全文匹配断言。入口核对独立 Cargo target 与当前实际测试二进制。原始证据目录须在回收根
之外，原执行者确认进程及资源实际收尾后用现有 `finish` 处理临时目录。

```bash
entry=verification/experiments/source-save-retention-rows
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/joint_native.py" --output "$native_output" --work "$allocated_payload"
```

公共登记与覆盖候选交主控，原共享验收同批协调。原生和公共验收结果绑定精确提交。
