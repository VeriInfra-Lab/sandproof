# 来源记录逐行写入

[保存首错](../save-order/README.md) · [首匹配及具体提供者](../retention-providers/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定 integration `7b27876458a7212feb98a2aaf36560fe6443fb73`。隔离候选从既有保存入口
拆出 `write_candidate_record_using`。真实父调用传原连接、完整键和原编码 Vec 的共享切片。
父视图逐项核对原 IR 与 `SqliteSourceRow` 的六个方法，两个视图的实际行控制正文逐字节一致。
原已有槽 SELECT、最小空槽 SELECT 和含键保护的 UPDATE 字符串由固定输入核对。

正确版用任意正常读回复证明三个更新前首错。已有槽查错原样返回。只有明确缺键才查空槽，
空槽查询错原样返回，空槽缺失使用原容量错误。两个可用槽分支使用同一连接，摘要获取收到
原完整编码切片，UPDATE 收到同一切片、提供者返回的完整摘要、原键及选中 ID。写入错误
原样返回，正常返回恰好一个影响行才可报告成功。正常见证和真实零行错版共用 `rowOutcome`。
错版仅把 `!= 1` 改为 `> 1`，抽取后两个分支的同一判据随之改变，类型与其他正文保持一致。

定理从已编码值和具名接口回复开始。A4 需要提供同一连接的原两个 SELECT 结果与绑定，
以及固定四参数 UPDATE 的实际绑定、执行结果和可能写入。原 UPDATE 的三个 BLOB 分别是
JSON、其 32 字节 SHA-256 与原工作键，第四项为已选 `i64`。其他语句的 A4 参数合同范围
见[逐符号依赖](../../../../../docs/formal/dependencies.md)。本切片读取摘要提供者的输出；
哈希计算、逐行更新的 SQLite 物理语义、整组事务与恢复仍分别处理。父视图的外部模板不
进入 Lean 闭包。[覆盖候选](public-registration.patch)供主控登记公共入口与覆盖。

## 形式复验

在 msra 使用固定工具、既有重锁和独立 `.lake/build`。参数沿用[前批](../retention-providers/README.md#抽取和-lean-复验)，
抽取输出遵守 80 GiB 起跑、持久工作区 64 GiB 停机、四核及原 4 GiB target 预算。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in core zero-ok parent; do
    bash scripts/python-entry.sh "$package/source_row_write.py" --mode "$mode" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE" --output "$new_capture/$mode"
done
for mode in core zero-ok; do
    root=SourceRowWrite
    if test "$mode" = zero-ok; then root=SourceRowWriteZeroAccepted; fi
    bash scripts/python-entry.sh "$package/prepare_row_write.py" --mode "$mode" \
        --capture "$new_capture/$mode" --parent "$new_capture/parent" --reference "$new_capture/core" \
        --output "$new_lean/$mode" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    (cd "$new_lean/$mode" && LEAN_NUM_THREADS=4 flock -n "$SANDPROOF_LOCK_FILE" \
        taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$new_lean/$mode" \
        --output "$new_audit/$mode" --lock "$SANDPROOF_LOCK_FILE"
done
```

每个 `plan.json` 列出五份物理文件的摘要。正确和错误分别使用原生成的 `Types` 和 `Funs`，
共享 `RowWriteInputs` 和原审计模块，各自导入对应证明根。

## ning 原生复验

以 [#949](https://github.com/VeriInfra-Lab/sandproof/pull/949) 的固定测试文件作原生夹具，
定向检查已有槽、最小空槽、完整物理行和真实零行。使用两个独立干净 `7b278764` checkout，
`variant` 分别为 `core` 和 `zero-ok`，固定各自 Cargo target。`proof_checkout` 为本 PR
的精确 checkout，`fixture_checkout` 为测试提交 `1686f9b8d0cdf6a14363db1715bbd486ed203c9a` 的干净 checkout。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
test "$(git -C "$fixture_checkout" rev-parse HEAD)" = 1686f9b8d0cdf6a14363db1715bbd486ed203c9a
test -z "$(git -C "$fixture_checkout" status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" "$fixture_checkout" <<'PY'
from pathlib import Path
import shutil
import sys
from source_row_write import stage

stage(Path.cwd(), sys.argv[1])
tests = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
fixture = tests.with_suffix('') / 'codec_rows.rs'
assert not fixture.exists()
shutil.copyfile(Path(sys.argv[2]) / fixture, fixture)
text = tests.read_text()
assert text.count('mod continue_application;\n') == 1
tests.write_text(text.replace('mod continue_application;\n', 'mod codec_rows;\nmod continue_application;\n'))
PY
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case "$variant" in
    core) "${native[@]}" controller::lifecycle_inbox::source_work::tests::codec_rows ;;
    zero-ok)
        "${native[@]}" --no-run
        code=0
        "${native[@]}" controller::lifecycle_inbox::source_work::tests::codec_rows -- --nocapture \
            > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'zero updated source rows must be rejected: Ok(())' "$native_negative_log"
        rg -F 'test result: FAILED. 1 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

正式普通与原生仍由 ning 统一复验。[#950](https://github.com/VeriInfra-Lab/sandproof/pull/950)
另外核对第二行 SQL 拒绝后的实际父事务回滚和重开结果。
