# 来源读取的完整工作键与同事务行绑定

[来源编码及原选槽](../source-key-history/README.md) · [SQLite 写入及耐久结果](../observation-sql-history/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

生产输入固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。复用 #974 的编码、载体和写入闭包。隔离副本保留原
`SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id` 及原三个列读取。
原有惰性 `query_map` 直接交给提取出的原行消费者，每次先读取当前行的 SQL 结果，再做原来的一行校验。
每次调用仍在原借用期内。独立具名端口只转发原 `checked` 和 anchor JSON 调用，前端保留原 `?`
短路、完整 SQL 键、原 anchor 摘要检查及 `Vec::push`。生产 Rust 和生成体都保持原样。

## 结论及边界

- `decoded_record_has_full_sql_key` 证明实际解码返回记录时，原 SQL 第三列为 Some 且其完整 BLOB 等于
  这条记录的 32 字节 `work_binding`。`foreign_blob_is_rejected_before_anchor` 对不等 BLOB 保留原
  `CorruptStore`，与后续 JSON 或摘要提供者的任何返回值无关。
- `collector_accounts_for_complete_order` 与 `read_ordered_rows` 对正常成功的原有限游标按顺序连接
  每条返回记录。同一行的原解码正常返回 None 时跳过，正常返回 Some 时追加完整值。SQL 行错误或行解码
  错误在实际生成体中终止，不会作为正常成功的记录或末尾。
- `original_load_then_save_selects_registered_slots` 从该实际读取结果和同一 Begin 的三列快照关系推出所有
  返回键均已登记，再沿 #974 的实际编码和 #973 的写入前缀得出后续正常查询使用原登记槽。

`Snapshot` 是 A4 对原三列及这次游标的有限观察。每条成功的游标回复与同一个活动事务视图中的真行逐字段
对应，保留完整 JSON、校验值和可空工作键。游标一项 SQL 错误保持实际失败，且没有虚构物理行。
定理通过原行消费者自身推导已登记键关系。V5 仍提供原 `load_sources` 调用和该游标结果的物理实例，
以及快照使用的实际连接和读锁/写锁接线。配置/预留前缀、原 `checked` 的 JSON 往返和校验值、
实际 SHA-256 的提供方、表重开或全库一致性另有义务。BLOB 等值采用已登记的 A6 完整长度与逐字节值。

## 形式复验

`key_project` 与 `key_audit` 是 [#974 原始复验](../source-key-history/README.md) 的全部输入及原服务结果。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/source_load_binding_capture.py" \
    --output "$capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
bash scripts/python-entry.sh "$package/prepare_source_load_binding.py" \
    --actual "$capture" --prior "$key_project" --audit "$key_audit" --output "$project" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build SourceLoadBinding)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 46 and (.audited | length) == 115
    and (.warnings | length) == 0 and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
```

## ning 复验

在两个固定生产的干净工作树，各自使用独立 Cargo target，按原工作流分别用
`variant=original` 或 `variant=skip-sql-key`。坏版只省略原 SELECT 第三列与解码记录工作键的全文比较。
本夹具使用原 SQLite、原准备路径和原 `codec::load`。只翻转最后一个 BLOB 字节，保留原 JSON 与校验值。
正例先见原行，损坏期间拒绝，恢复原键后读取原完整记录。坏版须实际编译并触发同一断言，再保存原始日志。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link/source-load-binding
bash scripts/python-entry.sh - "$proof_checkout/$package/native.rs.in" "$variant" <<'PY'
from pathlib import Path
import sys

fixture, variant = Path(sys.argv[1]), sys.argv[2]
source = Path('controller-service/src/controller/lifecycle_inbox/source_work')
if variant == 'skip-sql-key':
    codec = source / 'codec.rs'
    exact = (
        '(Some(record), Some(key))\n'
        '                if key.as_slice() == record.work.requirement.work_binding.as_bytes()\n'
        '                    && record.work.requirement.work_binding'
    )
    altered = '(Some(record), Some(_key))\n                if record.work.requirement.work_binding'
    before = codec.read_text()
    assert before.count(exact) == 1
    codec.write_text(before.replace(exact, altered))
else:
    assert variant == 'original'
with (source / 'tests.rs').open('ab') as handle:
    handle.write(b'\n' + fixture.read_bytes())
PY
native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
test_name=load_rejects_a_foreign_full_sql_key_for_canonical_source_bytes
if test "$variant" = original; then
    "${native[@]}" "$test_name"
else
    code=0
    "${native[@]}" "$test_name" > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'a different complete SQL key must reject the canonical original row' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
fi
```

正式普通与原生由 ning 取得。公共 Stage1 验收和覆盖记录由统筹复核
[覆盖候选](public-registration.patch)。
