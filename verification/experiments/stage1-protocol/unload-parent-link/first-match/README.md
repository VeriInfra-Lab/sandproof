# 来源记录的首个完整匹配

[保存首错与原写入](../save-order/README.md) · [完整工作比较](../../source-work-match/README.md) · [已存事实核对](../../source-preservation/README.md)

固定 integration `7b27876458a7212feb98a2aaf36560fe6443fb73`。隔离候选将 #942 中
`retains_saved_records` 的 `any` 与 `find` 拆成两个有界向前扫描。实际父调用把原全部
两份记录及原键集合交给扫描。每条旧记录先核对键，再取第一条 `next.work == old.work`。
选中记录用原 `preserves_saved_facts(old, next)` 决定结果。原 IR 核对选取两条记录的完整
`work` 字段、原事实函数的完整参数和原返回，并检查两个真实扫描在父生成视图与独立
证明视图相同。

证明复用 `FullReplayContract`，对任意有界向量给出实际生成扫描的 `List.find?` 和
`List.all` 对应。无匹配拒绝。首个匹配的已存事实若不满足，保留其拒绝结果。完整相等、
原键集合和原事实检查使用具名值级合同，不在这一层重新实现它们。两个已证明提供者的
具体生成类型连接仍需独立落实。证明范围是保存前的已解码值，原逐行 SQL、JSON/SHA、
事务及持久化分别由后续切片接上。

`inventory_using_complete_work` 以完整 `work` 投影和原比较合同作为输入。
`first_selected_record` 约束首个匹配的那条记录，即使后续也匹配，也只使用这一条的
已存事实检查。原生 `BTreeSet::contains` 的关系由键集合提供者承担，`Core.work_eq` 和
`Core.preserves_exact` 分别给出工作比较和事实保留的现有值级对应。

实际错误版把完整工作比较换成仅比较 `work_binding`。真实 SQLite 夹具保留同样的键和
anchor，只改变完整工作中的配置长度限制。正确版返回 `Conflict`、原行保持一致。错误版
返回成功并从原连接读回实际改写的完整记录。

## 形式复验

按[证明工作流](../../../../../docs/proof-workflow.md)固定工具、批准 CPU 与原重锁。抽取
输出所在文件系统启动前至少有 80 GiB，持久工作区保持原 64 GiB 停机线。每份输出目录
须全新，原始归档、调用日志、生成文件、原 IR 和材料化计划随开发收据保存。
每个 `plan.json` 记录六份物理 Lean 输入及各自摘要。两侧共用审计命令、原 `ReplayContract`、
`FirstMatchInputs` 和同一实际生成 `Types`。其余两份是各自真实生成的 `Funs` 与证明根。
父视图只用于原 IR 检查及两个循环定义的逐字节比较。外部模板不进入 Lean 闭包。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in factored binding-only parent; do
    bash scripts/python-entry.sh "$package/source_first_match.py" --mode "$mode" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE" --output "$new_capture/$mode"
done
for mode in factored binding-only; do
    root=SourceFirstMatch
    if test "$mode" = binding-only; then root=SourceFirstMatchBindingOnly; fi
    bash scripts/python-entry.sh "$package/prepare_first_match.py" --mode "$mode" \
        --capture "$new_capture/$mode" --parent "$new_capture/parent" \
        --correct "$new_capture/factored" --output "$new_lean/$mode" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    (cd "$new_lean/$mode" && LEAN_NUM_THREADS=4 flock -n "$SANDPROOF_LOCK_FILE" \
        taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$new_lean/$mode" \
        --output "$new_audit/$mode" --lock "$SANDPROOF_LOCK_FILE"
done
```

## 原生复验

准备两个独立干净的 `7b278764` checkout 和独立 Cargo target。令 `variant` 分别为
`factored`、`binding-only`，`proof_checkout` 指当前证明 PR 的固定 checkout，
`native_negative_log` 指本次错误版测试的独立日志。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
from source_save_capture import HERE as previous
from source_first_match import HERE, stage

stage(Path.cwd(), sys.argv[1])
fixture = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
with fixture.open('ab') as file:
    for test in (previous / 'native-save-order.rs.in', HERE / 'native-first-match.rs.in'):
        file.write(b'\n' + test.read_bytes())
PY
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case "$variant" in
    factored) "${native[@]}" controller::lifecycle_inbox::source_work::tests:: ;;
    binding-only)
        "${native[@]}" --no-run
        code=0
        "${native[@]}" proof_source_save_rejects_same_binding_with_different_complete_work \
            -- --nocapture > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'the same binding must not replace different complete SourceWork: Ok(()), candidate persisted: true' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

正式普通和原生由 ning 复验。两份物理 Lean 清单、三份原始捕获及正反原生归主控统一
登记到现有 Stage1 入口，按精确候选封口。[覆盖候选](public-registration.patch)
提供覆盖文字。当前切片的独立复验入口为上列命令。
