# 来源保存的实际记录聚合守卫

[证明工作流](../../../docs/proof-workflow.md) · [来源保存元数据容量与载荷](../source-save-load-payload/README.md)

本批接在 #1039，固定生产输入 `60aad8af23e317f5b1ba913518603d1fda015f4c`。
复用 #978 的原聚合读取、长度比较及其首错规则，并接到原保存第一次加载同一
SQLite 连接的实际 `load_records`。元数据完整配置、节点、会话、容量和来源记录
使用上一切片中的实际生成类型。

- `original_aggregate_reads_six_columns_in_order` 覆盖六列读回的原顺序及各列错误。
- `actual_loader_has_exact_first_decision` 关联原预算计算、行数、聚合读取和
  精确记录守卫。行数读取错误先于聚合，聚合错误先于容量拒绝。
- `original_save_rejects_invalid_record_bounds` 把守卫返回的错误接到原保存首错。
  `original_load_returns_exact_scanned_records` 把守卫成功后的完整记录返回值
  接到已校验的来源配置。
- 同一记录守卫合同覆盖正本及 `deployment-cap-for-length` 错版。样例在逐行预留
  222212 时到达后续扫描，增加一个字节则由原版拒绝。真实变异改用部署上限，
  继续到后续扫描并违反原合同。

实际有序记录扫描和真实 JSON/SHA 仍由相应具名提供者承接。物理事务、耐久性和
实际材料事实保留在各自模块的证明范围内。

## 证明机复验

使用 #1039 的完整检查与正确捕获、#984 原捕获选项，以及 #978
`e3e24a6fe7a7a3566b34c84aa27ac7001924474a` 的完整原捕获、已检查项目和诊断。
正反捕获沿用原 Rust 记录读取透明划分。完整闭包核对原 153 个类型及 276 个函数，
并验证两版仅有原记录长度判断的函数体不同。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-row-bounds
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$rows_captures"
for mode in correct deployment-cap-for-length; do
    bash scripts/python-entry.sh "$entry/rows_capture.py" --mode "$mode" \
        --output "$rows_captures/$mode" --reference "$load_errors_captures/original" \
        --bounds-reference "$record_bounds_original_capture" \
        --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/rows_check.py" --captures "$rows_captures" \
    --parent-output "$payload_formal_output" --parent-capture "$payload_captures/correct" \
    --reference "$load_errors_captures/original" --bounds-reference "$record_bounds_original_capture" \
    --bounds-project "$record_bounds_original_project" --bounds-audit "$record_bounds_original_audit" \
    --tools "$fixed_aeneas" --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

为本候选的精确提交分配既有独立空验收 payload。三份视图各自在独立目标目录编译，
运行同一真实保存断言。夹具先保存健康候选，再将其中一行的 `state_json` 改为
超过该行预留一个字节且仍在部署上限内的 TEXT。两正确版返回 `CorruptStore`。
错误版须实际运行到后续 SQLite 列读取，以 `InvalidColumnType(0, "state_json", Text)`
违反同一断言。夹具同时核对回滚后的记录。

```bash
entry=verification/experiments/source-save-load-row-bounds
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/rows_native.py" --output "$native_output" --work "$allocated_payload"
```
