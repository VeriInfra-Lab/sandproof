# 来源保存的实际有序 SQLite 游标

[证明工作流](../../../docs/proof-workflow.md) · [实际记录聚合守卫](../source-save-load-row-bounds/README.md)

叠在 #1042，固定生产输入 `60aad8af23e317f5b1ba913518603d1fda015f4c`。
把 #977 的三列读取和有序查询接到实际 `scan_loaded_records`，复用上一切片的
同连接元数据及记录汇总守卫。当前来源记录值使用完整生成的 Rust 类型。

- `mapper_reads_three_columns_in_order` 对任意实际列值及索引错误，保持
  `state_json`、`sha256`、可空 `work_key` 的原读取顺序、完整值和首错。
- `prepare_preserves_connection_sql_and_arguments` 关联调用中的原连接、
  `ORDER BY id` 查询和空参数。原 cursor 打开后按其顺序映射每个原行及错误。
- `original_save_keeps_cursor_prepare_error` 和
  `original_load_returns_same_ordered_collector` 把实际准备错误、审查完整的
  有序 cursor 结果接回同一原保存入口。
- 共同的 `ColumnContract` 在双 TEXT 样例上要求第零列错误。实际
  `checksum-first` 变异先报告第一列，违反同一合同。两个正常 BLOB 见证分别
  保持空和非空键值。

原 #977 的逐符号标准游标边界仍适用，`SourceRows::next` 转发对应固定的 Vec
迭代器表示，保留实际行顺序和错误。联合抽取对这个原 SQLite 标准库转发显式选择
不透明，以避开 Charon 的区域翻译故障。具名 `Library.openRows` 提供同一物理查询
的原始列和值回复，实际记录消费器由后续切片接入。真实 SQLite 运行事实、JSON/SHA
和耐久性由各自模块验收。

## 证明机复验

复用 #1042 的完整形式结果和正确捕获、#984/#978 的原捕获选择，以及 #977
`b56b75ed1f866bfc01cba1cd242d692feebfa020` 的原捕获、原检查项目和完整诊断。
当前抽取保留全部 155 个父类型及 283 个父函数声明，正反只有原三列 mapper
函数体不同。检查器验证历史服务请求哈希逐一对应实际输入。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-cursor
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$cursor_captures"
for mode in correct checksum-first; do
    bash scripts/python-entry.sh "$entry/cursor_capture.py" --mode "$mode" \
        --output "$cursor_captures/$mode" --reference "$load_errors_captures/original" \
        --bounds-reference "$record_bounds_original_capture" \
        --cursor-reference "$sql_cursor_original_capture" \
        --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/cursor_check.py" --captures "$cursor_captures" \
    --parent-output "$rows_formal_output" --parent-capture "$rows_captures/correct" \
    --reference "$load_errors_captures/original" --bounds-reference "$record_bounds_original_capture" \
    --cursor-reference "$sql_cursor_original_capture" --cursor-project "$sql_cursor_original_project" \
    --cursor-audit "$sql_cursor_original_audit" --tools "$fixed_aeneas" \
    --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

使用精确候选的独立验收 payload，原版、正确联合版和 checksum-first 错版均编译
且执行同一个真实 SQLite 保存断言。夹具分别将三列、后二列、最后一列改为合法
长度以内的 TEXT，保持后续的存储类转换错误可区分。两正确版在原列顺序返回首错，
错误版第一组需实际运行并报告第 1 列 `sha256` 的 TEXT 转换错误，违反要求第 0 列
的同一断言。夹具验证每次回滚后原记录保留。

```bash
entry=verification/experiments/source-save-load-cursor
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/cursor_native.py" --output "$native_output" --work "$allocated_payload"
```
