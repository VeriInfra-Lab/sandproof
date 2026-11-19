# 来源预留行的聚合检查与原扫描

[原 SQLite 游标和三列](../source-sql-cursor/README.md) · [原行消费者](../source-load-binding/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

生产固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。本批从已解码的 `Configuration` 和
`CapacityConfig` 开始。固定抽取保留原 `load_records`、预留量计算、会话行数、六列
聚合读取、转换和所有后续分支。原扫描接在 #977 的同一连接上。

## 定理和边界

- `reserved_count_comes_from_policy` 接原维护策略回复，再使用
  `max_sessions × (max_operations_per_sandbox − 2)` 的原数值运算。
  原固定 16 MiB 上限及原退休记录预留常量由实际生成体计算。
- `original_aggregate_reads_six_columns_in_order` 检查原六个整数列及最早失败。
  原 SQL 聚合的字段依次为行数、最小 ID、最大 ID、最小预留、最大预留、最大载荷长度，
  空集合使用 SQL 原 `COALESCE`。原 `query_row` 用同一连接、完整 SQL 和空参数。
- `count_error_precedes_aggregate` 和 `aggregate_error_precedes_capacity` 保留原顺序。
  会话行数错误先返回，聚合 SQL 错误先于部署容量守卫。配额或全局上限超限后不进入
  SQL 有符号值比较。其余情况下先做原窄化，再比较完整五元组，最后拒绝负长度和超过
  原每行预留量的长度。恰好等于预留量允许进入原扫描。
- `guarded_scan_registers_full_keys` 接入 #977 的已证游标。原扫描在上述守卫均通过时
  返回记录。在同一活动事务的游标条件下，返回的完整工作键均属于原登记行。

V5/A4 提供原维护策略读取、同一 SQLite 事务的真实聚合回复及物理游标关系。
实际 `codec::load` 的 schema、格式版本、元数据数值与配置 JSON 读取仍各有义务。
原聚合回复的值决定守卫。SQLite 自身如何执行聚合、索引和锁由该提供者连接。
本包给出 [登记候选](registration-candidate.json) 与 [覆盖候选](public-registration.patch)。

## 形式复验

`cursor_project` 与 `cursor_audit` 是 [#977 原版复验](../source-sql-cursor/README.md)
的精确输出。两版分别重建原 Rust 输入，使用独立 Lake build，再经同一 Lean 服务逐文件审计。
错误版只把最大载荷长度的比较对象由每行预留换为部署上限。原版与错误版保留同一 `Contract`。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in original deployment-cap-for-length; do
    extracted="$bounds_evidence/$variant"
    project="$bounds_evidence/$variant-project"
    audit="$bounds_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/source_record_bounds_capture.py" \
        --variant "$variant" --output "$extracted" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare_source_record_bounds.py" \
        --variant "$variant" --actual "$extracted" --prior "$cursor_project" --audit "$cursor_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    count=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/source-record-bounds/registration-candidate.json")
    jq -e --argjson count "$count" '.complete and .physical_files == 52
        and (.audited | length) == $count and (.warnings | length) == 0
        and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
jq -se '.[0].fragments as $correct | .[1].fragments as $wrong |
    ($correct["-- ACTUAL LOADER"] != $wrong["-- ACTUAL LOADER"]) and
    (($correct | del(.["-- ACTUAL LOADER"])) == ($wrong | del(.["-- ACTUAL LOADER"])))' \
    "$bounds_evidence/original-project/plan.json" "$bounds_evidence/deployment-cap-for-length-project/plan.json"
```

## ning 原生复验

在两个固定生产的干净工作树，以独立 Cargo target 分别运行 `variant=original` 和
`variant=deployment-cap-for-length`。用例从原有效记录开始，实际 SQLite 写入比原
每行预留多一个字节、仍在部署上限内的 TEXT。原版须先返回 `CorruptStore`，错误版
越过守卫后在原扫描返回 SQLite 列型错误。原版回滚后须读回原记录。

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
from source_record_bounds_capture import substitute_length_limit

source = Path('controller-service/src/controller/lifecycle_inbox/source_work')
if variant == 'deployment-cap-for-length':
    codec = source / 'codec.rs'
    codec.write_text(substitute_length_limit(codec.read_text()))
else:
    assert variant == 'original'
with (source / 'tests.rs').open('ab') as handle:
    handle.write(b'\n' + (package / 'source-record-bounds/native.rs.in').read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
test_name=source_loader_enforces_per_row_reserve_before_scanning
if test "$variant" = original; then
    "${native[@]}" "$test_name" | tee "$native_positive_log"
    rg -F 'test result: ok. 1 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" "$test_name" > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'per-row reservation must reject before decoding the oversized TEXT' "$native_negative_log"
    rg -F 'InvalidColumnType(0, "state_json", Text)' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
fi
```

完整普通、原生正反以及 Stage1 与 Controller 公共入口由最终采用批次统一验收。
