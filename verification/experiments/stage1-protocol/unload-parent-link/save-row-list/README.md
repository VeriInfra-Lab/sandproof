# 来源集合保存接入逐行 SQLite 写入

[保存顺序](../save-order/README.md) · [实际选槽和更新](../row-sql-composition/README.md) ·
[保存观察的父方法](../observation-parent/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定生产 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。`codec.rs` 与 #956 的 `7b278764` 基线
为同一 Git blob `7dc0dbb6471fb398193b4e263da30e73d8c436d3`。在当前精确归档中重用
#956 的已核对隔离抽取，保留 `codec::save`、具体实例、原 Vec 消费循环和单行写入。
Charon 和 Aeneas 重新运行。保存适配器逐个核对原读、容量、编码、旧责任和原遍历调用。

## 对应与性质

材料化时逐个比对 #956 选槽、绑定和写入的全部生成声明。Lean 使用 #956 的同一个
`rusqlite.Connection` 和 `JournalError`，以及其实际单行对应定理。新视图复制原保存、
五个具体适配方法及其字典，以及原 writer 循环的声明字节。Vec 的 `into_iter` 与
`Iterator::next` 使用固定 Aeneas 后端的相应定义。列表归纳证明对整个有限编码集合成立。

- `loop_writes_original_order` 和 `rows_use_same_connection` 将原 Vec 的每个完整键和
  编码字节按原顺序交给 #956 的实际 SELECT、选槽及 UPDATE 对应，始终使用同一连接。
- `codec_rows_return_first_error` 允许前面任意个成功行，在首个失败位置原样返回错误。
  定理不要求后续行满足任何条件。
- `save_after_retention` 将这些性质接回具体 `codec::save`。已读取原来源、配置和容量
  检查通过且取得原编码时，旧责任不成立先返回 `Conflict`。旧责任成立才用同一连接和同一
  批编码 Vec 进入实际写入。

定理从已载入值和编码回报开始。`load`、`reserved_records`、`encode_candidate_records`
和 `retains_saved_records` 仍各自提供回报条件。A4 提供具体 SQLite 原语，A9 提供该字节串
的摘要。#968 的父调用还需把该保存符号接进同一活事务；每步 SQL 对下一步的库状态影响、
写入提交和失败回滚均依赖后续 V5 组合。#956 和 #950 的原生对照按各自候选结算。

## 复验

`original_row_capture` 是 #956 复验的 `core` 捕获。过程保留真实 LLBC 和生成器输出。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/source_save_row_list_capture.py" \
    --output "$current_capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/prepare_source_save_row_list.py" \
    --current "$current_capture" --prior "$original_row_capture" --output "$project" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build SaveRows)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 24 and (.audited | length) == 39' "$audit/result.json"
```

ning 统一按 #956 的真实 SQLite 选槽和零行/第二行失败夹具、保存顺序包的留存调序对照
复验原生。原有正错结果各保留其确切源码来源。新增 `SaveRows` 的物理闭包与
[覆盖候选](public-registration.patch) 由统筹核对并登记到公共验收。
