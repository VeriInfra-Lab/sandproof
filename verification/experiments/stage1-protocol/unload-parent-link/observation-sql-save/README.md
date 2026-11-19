# 观察保存父调用接入原 SQLite 写入

[父方法](../observation-parent/README.md) · [保存与行循环](../save-row-list/README.md) ·
[原提交尾部](../observation-commit/README.md) · [逐行 SQL](../row-sql-composition/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定生产 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。同一归档内，应用 #963 和 #969
已经核对的隔离抽取改写并使用原 Charon/Aeneas，生产源码保持固定。Charon 核对原父方法的
选中分支、同一事务及所选保存字典。该字典的原 `save` 调用实际 `codec::save`，后者使用
原 Vec 循环和单行写入。

## 生成体及类型连接

材料化检查联合抽取、#968 父生成体与 #969 保存生成体。父生成体的唯一字节差异是所选
保存实例因 `Sources` 三个类型实参而取得的名称。实例声明及适配器只增加这三个原类型的
实参；其它父调用、提交和查找声明保持原字节。当前 `codec::save`、实际保存实例、循环和
单行写入声明与 #969 对应逐字相同。两份独立证明视图的共同物理文件须按原字节一致。

`source_fields_round_trip` 保留配置、节点、容量及完整记录 Vec。`bindCodec` 通过同型等式
在父调用和 #956 SQL 接口间传递连接及错误，从该连接调用实际保存。等式不提供任意状态
或连接替换。父方法取得的同一事务用于原读取、解引用和后续提交；写入调用固定为 #969。

- `first_row_error_precedes_commit` 允许一个任意的成功前缀。首个失败行返回其确切错误，
  原 Begin 回写连接和原 hook 值一起返回。定理不要求余下行、提交前钩子、提交或提交
  后钩子提供回报。
- `sql_success_then_commit` 复用 #968 的事务和 #969 的写入。在原提交及两处钩子成功时，
  返回原选中记录的更新版本。`sql_success_then_commit_error` 保留提交错误和提交前钩子
  的状态。`after_commit_error_is_returned` 在原提交成功后保留提交后钩子错误及其状态。

原观察守卫、完整工作匹配、实际 `Sources` 更新、按序编码 SQL 与原提交通过同一调用链
组合。结论从已解码值开始，编码回报、保留检查以及 SQLite 原语效果仍由各逐符号提供者
落实。`WorldTransaction::drop` 实际处理仍然活着的事务。上述 SQL 错误定理核对的是
显式返回与调用顺序。将 SQLite 每个写入后的状态接到下一行、核对 Drop 回滚、提交后可见
性及 `CommitUncertain` 的两种耐久结局，仍需 V5 状态和事务证据。

## 精确复验

`slot_parent`、`prefix_parent`、`slot_core`、`commit_tail` 是 #968 的四份原输入。
`current_save` 和 `row_sql` 是 #969 的实际保存及 #956 的原单行输入。联合抽取的原归档、
实际 LLBC、生成字节摘要及 35 个物理 Lean 文件进入同一计划。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/observation_sql_save_capture.py" \
    --output "$joint_capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/prepare_observation_sql_save.py" \
    --joint "$joint_capture" --original "$slot_parent" --prefix "$prefix_parent" \
    --scan "$slot_core" --tail "$commit_tail" --current-sql "$current_save" --row-sql "$row_sql" \
    --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build ObservationSqlSave)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 35 and (.audited | length) == 74' "$audit/result.json"
```

ning 正式复验沿用 #950 的第二行失败实际回滚和 #959 的原观察提交可见性、重开与错序
入口。原生及错误版按原候选保存记录，并按工作流核对实际输入后复用。统筹可复核
[覆盖候选](public-registration.patch)，同批将合并后的 `ObservationSqlSave` 根及原证据
登记到公共 Stage1 入口。
