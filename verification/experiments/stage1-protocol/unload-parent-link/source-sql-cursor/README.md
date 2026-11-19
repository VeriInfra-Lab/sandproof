# 来源查询的原 SQLite 游标与三列

[原行消费者](../source-load-binding/README.md) · [原选槽及写入](../source-key-history/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

生产固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。原 SQL 为
`SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id`。
隔离捕获把原 `prepare` 到 `read_loaded_rows` 的尾部提为一个根，把原 `query_map` 的
三列闭包提为另一根。原 `query_map([], read_source_columns)` 和 `MappedRows::next`
由保留原借用期的薄转发连接。固定 Aeneas 直接处理原游标时在高阶生命周期转换失败，
因此原库游标及薄转发使用显式 A4/B4 接口。实际查询调用方、三列 mapper 和它们使用的
生成字典在独立视图内透明。读取循环复用 #975 的原生成函数。隔离副本以外的 Rust 原样保留。

## 定理与提供者

- `prepare_preserves_connection_sql_and_arguments` 保留原连接、完整 SQL 与空绑定。
  `scan_prepare_error_is_first` 和 `scan_open_error_precedes_row_read` 保留原 SQLite
  错误，且此两种失败在进入行消费者前返回。
- `mapper_reads_three_columns_in_order` 对所有模型行证明实际顺序为第 0、1、2 列。
  前两列只接收 BLOB，第三列接收 SQL NULL 或完整 BLOB。无效列号或存储类型的首错
  先于后续列及 JSON。原版和错误版用同一 `ColumnContract`，在两个非 BLOB 列均错时
  原版选第 0 列，错误版选第 1 列。
- `cursor_preserves_order_and_errors` 保留原有限游标中的顺序、原始 SQLite 步进错误
  和每项真实 mapper 结果。`scan_uses_original_mapped_cursor` 以原游标的 `next`
  接到 #975 的行消费者，其先错停读和去除空槽的行为由原闭包证明。有限列表描述惰性
  游标的值序列，避免把模型中的 `List.map` 解释为运行时预读或额外 SQL 调用。
- `actual_scan_registers_returned_keys` 从同一 Statement 的原列投影推出返回记录的
  完整键都已登记。`scan_then_save_selects_same_transaction_slot` 复用原事务父调用、
  编码和 SQLite 写入历史，推出观察保存的正常查询使用原登记槽。

`ActiveCursor` 明列 V5/A4 的剩余责任。原 prepare、同一个已发出的 Statement、实际
`query_map` 的有限返回及每条原始 SQLite 行都要有同一活动事务的物理实例。SQLite
存储类、列数和错误由同一库边界给出。证明在此输入上推导行序列及表示关系。实际
`load_sources` 的元数据、容量和配置前缀，原 `checked` 的 JSON 与校验值关系、
SHA-256 提供方以及游标错误后的物理效果仍各有条件。V5 原生复验和全套验收由 ning
与统筹执行。本目录给出 [Stage1 登记候选](registration-candidate.json) 和
[覆盖记录候选](public-registration.patch)。

## 形式复验

`loader_project` 和 `loader_audit` 是 [#975 原行消费者复验](../source-load-binding/README.md)
的精确目录。两版各有独立 Lake build。材料器重建固定 Rust 归档及实际变异，核对生成体、
封存日志与父闭包，再把实际函数体逐字导入各自的 Lean 视图。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in original checksum-first; do
    extracted="$cursor_evidence/$variant"
    project="$cursor_evidence/$variant-project"
    audit="$cursor_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/source_sql_cursor_capture.py" \
        --variant "$variant" --output "$extracted" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare_source_sql_cursor.py" \
        --variant "$variant" --actual "$extracted" --prior "$loader_project" --audit "$loader_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    if test "$variant" = original; then audit_count=128; else audit_count=118; fi
    jq -e --argjson count "$audit_count" '.complete and .physical_files == 49
        and (.audited | length) == $count and (.warnings | length) == 0
        and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
jq -se '.[0].fragments as $correct | .[1].fragments as $wrong |
    ($correct["-- ACTUAL MAPPER"] != $wrong["-- ACTUAL MAPPER"]) and
    (($correct | del(.["-- ACTUAL MAPPER"])) == ($wrong | del(.["-- ACTUAL MAPPER"])))' \
    "$cursor_evidence/original-project/plan.json" "$cursor_evidence/checksum-first-project/plan.json"
```

## ning 原生复验

在两个独立干净工作树用原构建环境，分别设 `variant=original` 或
`variant=checksum-first`，另用各自独立 `CARGO_TARGET_DIR`。夹具走原 SQLite schema、
`codec::load` 和真实 mapper。三个小事务依次将第 0–2 列、第 1–2 列和第 2 列设为
符合原长度限制的 TEXT，核对首个 `InvalidColumnType` 的列号、列名和存储类，回滚后
重新读到原记录。错误版仅提前读取校验列，成功时保持原三元组。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh - "$proof_checkout/$package" "$variant" <<'PY'
from pathlib import Path
import sys

package, variant = Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(package))
from source_sql_cursor_capture import swap_column_reads

source = Path('controller-service/src/controller/lifecycle_inbox/source_work')
if variant == 'checksum-first':
    codec = source / 'codec.rs'
    codec.write_text(swap_column_reads(codec.read_text()))
else:
    assert variant == 'original'
with (source / 'tests.rs').open('ab') as handle:
    handle.write(b'\n' + (package / 'source-sql-cursor/native.rs.in').read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
test_name=source_cursor_reports_first_invalid_column_type
if test "$variant" = original; then
    "${native[@]}" "$test_name" | tee "$native_positive_log"
    rg -F 'test result: ok. 1 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" "$test_name" > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'source cursor must return the first invalid column' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
fi
```

公共登记由主控按现有 Stage1 形式与原生入口统一核对。普通入口及原生正反例的执行
回执依赖 ning，同步根和两个形式视图可独立复验。
