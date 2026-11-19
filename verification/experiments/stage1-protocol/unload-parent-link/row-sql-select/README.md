# 来源记录的两个 SELECT

[逐行写入](../row-write/README.md) · [四参数 UPDATE](../row-sql-update/README.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

以 `7b27876458a7212feb98a2aaf36560fe6443fb73` 隔离源为准。两个薄根分别调用同一
`SqliteSourceRow::existing` 和 `SqliteSourceRow::vacant`。正确版保持 #951 中的原 SQL、参数个数与
回调，隔离版仅将空参数 `[]` 改为 `()`。固定 rusqlite 0.40.2 的两个 `Params::__bind_in`
均调用 `ensure_parameter_count(0)`。原空数组捕获到 `Dyn ToSql`，固定后端输出的泛型
数组 universe 未通过 Lean。错误版相对该隔离候选只把原 `min(id)` 改成 `max(id)`。两次抽取的生成类型相同，函数体只在该
SQL 字面量不同。实际生产文件由统筹另行处理。

## 两种空值与提供者

- 已有键查询将完整 32 字节 BLOB 送到原 `work_key=?1`，它的回调只读第 0 列为 `i64`。
  `OptionalExtension` 附加 `Some`，且仅将实际 `QueryReturnedNoRows` 解释为 `None`。
  例如原列类型错误保持原 SQLite 错误。
- 空槽查询使用 `min(id)`、`WHERE work_key IS NULL` 与零参数。它保持语句参数数目
  检查，第 0 列用 `Option<i64>` 解码 SQL NULL。库若返回无行错误，仍原样传播。
- 两条调用使用同一原调用方传来的接收者，条件接口保留 #954 的 canonical Connection。
  建议 V0/A4 采用的新增窄项是完整原错误的 `QueryReturnedNoRows` 分类、原语句的
  单 BLOB 或零参数、实际首行和第 0 列原始值/错误。单 BLOB 列表描述原始输入。
  原库仍负责 raw bind、首错及末尾计数、step 和原对象关联。实际 SELECT 对原行集的语义、
  同一活事务和后续 UPDATE 是否命中由 A4/V5 与独立原生证据继续连接。
- 形式视图从该有限接口开始。正式登记时核对 `OptionalExtension`、`Row::get<usize, i64>`、
  `Row::get<usize, Option<i64>>` 及固定 `Params` 的真实符合性，再接入公共 Stage1 入口。
  库提供者是显式值，生成体继续执行真实回调、分支和原 `JournalError::Sqlite` 构造。

共用的 `existing_query` 和 `vacant_query` 先说明两类语句各自的准备、参数与首行次序。
`actual_existing` 和 `actual_vacant` 把原生成根接到这两个合同。反例根在同一个提供者见证上
用 `same_min_contract_rejected` 拒绝最大的槽。形式见证只区分 SQL 及其可观察结果，
真实 SQLite 是否返回最小行由原生对照单独观察。

两份隔离项目的 `plan.json` 各登记 11 个实际 Lean 文件，包括原生成 Types/Funs、
候选读接口及用到的既有 canonical 文件。统筹可参考 `public-registration.patch`。
接受新库使用点后须将三种原生版及两套正式视图接入原 Stage1 脚本。

## ning 原生复验

在三个独立干净的 `7b278764` checkout 中分别使用 `original`、`core`、`greatest-vacancy`。
`proof_checkout` 是本证明提交的工作树。保持原工具链、离线设置和各自 Cargo target。
步骤需保留实际编译记录与原 libtest 结果。三个版本使用同一 Rust 测试。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
from source_row_sql_select import CODEC, HERE, digest, stage

source = Path.cwd()
staged = stage(source, sys.argv[1])
with (source / CODEC).open('ab') as file:
    file.write(b'\n' + (HERE / 'native-select.rs.in').read_bytes())
print(sys.argv[1], staged, digest(source / CODEC))
PY
tests=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
"${tests[@]}" --no-run
case "$variant" in
    original|core) "${tests[@]}" proof_source_select_binding ;;
    greatest-vacancy)
        code=0
        "${tests[@]}" proof_source_select_binding -- --nocapture > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'vacant SELECT must return the minimum unassigned slot' "$native_negative_log"
        rg -F 'test result: FAILED. 3 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

四项分别检查完整键（含同前 31 字节的两个键和超过 32 位的槽号）、多个空槽与没有空槽、
实际表缺失错误、实际 `InvalidColumnType`。最后一个测试单独使用不符合 schema 的表，
用于到达原回调的类型错误分支。错误版也必须实际编译并在原最小槽断言被拒绝。

## 证明机复验

使用仓库固定工具、证明环境和独立数据盘。`capture_root` 是本次已批准的目录，
`fixed_tools`、`lake_packages` 与 `registered_lean_mcp` 指向原组件。
`shared_lock` 是原证明重锁，`four_cpus` 是协调后的四核组。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in core greatest-vacancy; do
    taskset -c "$four_cpus" bash scripts/python-entry.sh "$package/source_row_sql_select.py" \
        --mode "$variant" --output "$capture_root/capture-$variant" \
        --tools "$fixed_tools" --lock "$shared_lock"
done
for variant in core greatest-vacancy; do
    project="$capture_root/lean-$variant"
    root=SourceRowSqlSelect
    if [ "$variant" = greatest-vacancy ]; then root=SourceRowSqlGreatestVacancy; fi
    bash scripts/python-entry.sh "$package/prepare_source_row_sql_select.py" \
        --mode "$variant" --capture "$capture_root/capture-$variant" \
        --reference "$capture_root/capture-core" --output "$project" \
        --lake-packages "$lake_packages" --lean-mcp "$registered_lean_mcp"
    (cd "$project" && LEAN_NUM_THREADS=4 flock "$shared_lock" \
        taskset -c "$four_cpus" lake build "$root")
    taskset -c "$four_cpus" bash scripts/python-entry.sh "$package/diagnose.py" \
        --project "$project" --output "$capture_root/audit-$variant" --lock "$shared_lock"
done
```
