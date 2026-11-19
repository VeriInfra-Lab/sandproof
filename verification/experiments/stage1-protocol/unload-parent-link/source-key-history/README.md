# 来源观察保存的原键与已登记槽

[写入历史及耐久结果](../observation-sql-history/README.md) · [原 SQLite 选槽](../row-sql-composition/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定生产 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。复用 #973 的 41 文件视图及其父调用、写入循环和事务历史。
新视图用固定 Charon/Aeneas 从同一生产归档抽取实际 `encode_candidate_records`、循环和循环体。
隔离副本复用既有保存投影，将 anchor 与完整记录的两个原 `serde_json::to_vec` 各移入一个只转发原参数
和结果的具名调用，以避开固定翻译器的 Serde 循环 trait。生产 Rust 保持不变。

## 结论和接口

- `actual_encoding_keeps_full_keys` 对任意正常编码成功结果，证明实际 Vec 中的键逐项等于
  原记录 `work.requirement.work_binding` 的完整 32 字节，保留次序和重数。证明追踪原迭代器的位置、
  成功分支实际 `Vec::push` 和原累积器，对重复键、JSON、摘要不符或大小拒绝不提供成功结果。
- `observation_update_preserves_key_vector` 复用原观察父方法对选中记录的更新，`work` 保持原值。
  `registered_after_writes` 复用 #973 的原槽位和键保持结果，适用于任意成功前缀。
- `original_encoded_rows_select_registered_slots` 将选中记录、原编码结果、成功前缀和当前读取的 SQL
  槽绑定到同一原 Begin。若起始 load 关联原已登记行，且原 SELECT 正常返回并满足 A4 的完整 BLOB
  等值条件，则下一行用原已登记槽，跳过空槽查询。SELECT 错误仍由原首错定理处理。

`Fields` 保留实际 requirement 和 work 的全部字段。其他载荷由原父库的类型参数和表示关系承载，
编码叶子使用原 Serde 调用的正常值接口。SHA-256、长度预算及固定 Global BTreeSet 回报保持具名输入。
本批从实际编码成功推导键关系，包括任何被调用的集合/编码接口正常回报。已解码记录与 SELECT
快照的完整来源仍由 V5 的实际 `load_records`、同一事务及原 schema 提供。`A4Lookup` 只约束
该 SELECT 的正常结果，SQLite 错误、JSON 字节正确性、摘要真实性和其他写入路径不由它保证。
本批与 #973 重用原来适用的 SQLite 原生对照及各自的错误版。

## 精确复验

`history_project` 和 `history_audit` 来自 [#973 的原始复验](../observation-sql-history/README.md)。
新材料化核对源归档、LLBC、实际原工具日志和全部生成文件的摘要。生成声明以原字节复制到私有 Lean
视图，外部模板仅核对摘要。新的 `plan.json` 收录实际捕获、原 41 文件闭包、原 Lean 服务证据摘要及新根。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/source_key_encode_capture.py" \
    --output "$capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/prepare_source_key_history.py" \
    --actual "$capture" --prior "$history_project" --audit "$history_audit" --output "$project" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build SourceKeyEncodeProof)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 44 and (.audited | length) == 108
    and (.warnings | length) == 0 and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
```

正式普通与原生由 ning 统一验收。复用 #950 的原父方法双行测试，第二行失败前核对同一事务能读取第一行
完整新值及工作键，重开后重试核对原槽、原键、摘要和另一行。复验命令及其余原始正反证据见
[#973 说明](../observation-sql-history/README.md)。公共覆盖候选与当前工作流一起由统筹复核。
