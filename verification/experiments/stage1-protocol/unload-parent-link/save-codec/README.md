# 来源记录保存的抽取与首错回归

[Unload 父调用](../README.md) · [原提交调用](../finish/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定 integration `7b27876458a7212feb98a2aaf36560fe6443fb73`。隔离候选将原
`source_work::codec::save` 中的编码、旧责任核对和逐行写入提取为具名 Rust 函数。
保存入口仍先读取原库、比较配置及节点和会话上限，再核对可用记录数。整组编码
完成后，原核对按完整 `SourceWork` 找到首个匹配并检查已保存事实。核对通过才遍历
编码记录，在原 SQLite 连接按原键查询既有槽或第一个空槽，执行原带条件的 UPDATE。

捕获驱动固定原始归档、两个补丁的实际输出、原 Charon/Aeneas 工具、调用闭包和生成
文件摘要。它从原始 IR 核对保存入口的调用顺序、三次读写边界使用同一原连接、
旧责任拒绝返回原 `Conflict` 以及正确版只有通过分支才调用写入。调序版将真实
`write_candidate_records` 调到旧责任核对前，仍使用原行写入。在两个独立目录保存
生成的 `Funs.lean`、`Types.lean` 和原捕获报告。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in factored reordered; do
    bash scripts/python-entry.sh "$package/source_save_capture.py" --mode "$mode" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE" \
        --output "$new_source_save_capture/$mode"
done
```

两个输出须全新，共用既有证明重锁。此抽取覆盖真实保存入口的控制流，生成视图将
`load`、`reserved_records`、`encode_candidate_records`、`retains_saved_records`
和 `write_candidate_records` 留作具名阶段，待接上实际提供者。完整 `SourceWork`
相等与 `preserves_saved_facts` 已有值级结果，需连接本调用第一次匹配。行级 SQL、
JSON/SHA、分配失败、事务回滚和生成体的 Lean 对应仍有独立义务。这一层使用原始
生成体和原生对照，未导入生成器的外部公理模板。

## 原生复验

在三个精确的干净 `7b278764` checkout 中使用 `variant=original`、`factored` 或
`reordered`。`proof_checkout` 指本证明 PR 的精确 checkout，`native_negative_log`
指调序版单独的日志文件。各版本使用独立 Cargo target，按原证明工作流固定 Rust 环境。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
from source_save_capture import HERE, stage

stage(Path.cwd(), sys.argv[1])
fixture = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
with fixture.open('ab') as file:
    file.write(b'\n' + (HERE / 'native-save-order.rs.in').read_bytes())
PY
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case_name=proof_source_save_validation_precedes_any_row_update
case "$variant" in
    original) "${native[@]}" "$case_name" ;;
    factored) "${native[@]}" controller::lifecycle_inbox::source_work::tests:: ;;
    reordered)
        "${native[@]}" --no-run
        status=0
        "${native[@]}" "$case_name" -- --nocapture > "$native_negative_log" 2>&1 || status=$?
        test "$status" = 101
        rg -F 'the saved-work conflict must precede the source row update:' "$native_negative_log"
        rg -F 'reached source row update' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

相同 SQLite 夹具使用真实首次 Unload 取得原行。保持原键和 anchor，只改完整工作内的
一个限制时应得到 `Conflict`。再加入第二个相同键时，应先得到 `CorruptStore`。
精确原记录可到达触发器，必须保留真实 SQLite 错误。移除触发器后须按原键正常保存并
读回全值。调序版在第一条断言处实际到达提前触发的写入错误。普通和正式原生验收仍由
ning 执行。
