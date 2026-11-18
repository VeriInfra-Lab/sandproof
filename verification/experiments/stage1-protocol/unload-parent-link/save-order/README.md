# 来源记录保存的顺序证明

[保存入口抽取](../save-codec/README.md) · [首次 Unload 提交调用](../finish/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定 integration `7b27876458a7212feb98a2aaf36560fe6443fb73`。隔离候选保持 #940 的原
保存分段，将五个已命名的读写调用接成 `SourceSaveEffects` 的参数。实际 `Sources`
通过默认类型参数保留原配置、节点、会话上限和完整记录。原 `codec::save` 传入生产
实例。原始 IR 核对适配方法按原参数调用各阶段、将原结果直接返回；父视图的
`save_using` 正文也必须与独立 Lean 视图逐字一致。

在已载入的值及具名调用结果上，正例证明读取错误、缺少来源、配置、节点、两处会话
上限、容量读取、记录数超限、编码及旧责任拒绝的首错。通过时把同一连接和编码记录
交给原写入并返回其完整结果。错误表示通过原请求视图的共同类型逐个保存构造器和
payload，可双向往返。正常见证用空记录表取到写入成功；拒绝见证复用同一接口。
调序版实际把写入放在旧责任核对前，证明它会优先返回 `Sqlite` 错误；原 SQLite 夹具
用相同输入实际触发该错误。定理与原始 IR 的分支拒绝是不同证据，物理状态由原生观测提供。

## 形式开发复验

输出父目录须已创建，子目录须全新。在独立抽取目录及独立 `.lake/build` 使用规定固定
工具和原锁。抽取文件系统须满足至少 80 GiB 启动空间，持久工作区继续遵守 64 GiB
停止线。使用临时文件系统时应把原归档、修改后源、IR、生成体和各阶段日志保存在
持久证据目录。生成器的外部公理模板只用于核对输出清单。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in factored reordered parent; do
    bash scripts/python-entry.sh "$package/source_save_order.py" --mode "$mode" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE" --output "$new_capture/$mode"
done
for mode in factored reordered; do
    root=SourceSaveOrder
    if test "$mode" = reordered; then root=SourceSaveOrderReordered; fi
    bash scripts/python-entry.sh "$package/prepare_source_save_order.py" --mode "$mode" \
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

在两个精确的干净 `7b278764` checkout 中，分别令 `variant=factored` 或 `reordered`。
`proof_checkout` 指当前证明 PR 的固定 checkout。按工作流设置两个独立 target 和
`native_negative_log`。原实现回归沿 [#940 原生入口](../save-codec/README.md#原生复验) 核对。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
from source_save_capture import HERE
from source_save_order import stage

stage(Path.cwd(), sys.argv[1])
fixture = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
with fixture.open('ab') as file:
    file.write(b'\n' + (HERE / 'native-save-order.rs.in').read_bytes())
PY
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case_name=proof_source_save_validation_precedes_any_row_update
case "$variant" in
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

普通和正式原生由 ning 复验。两份材料化计划的全部物理 Lean 文件、独立错误视图和
原生对照须一并进入主控正式登记。[覆盖候选](public-registration.patch) 给出适用范围。
原读库、完整值首次匹配、编码和逐行写入仍需各自连接已验证的 V0/V5 接口。SQLite
持久效果、真实 JSON/SHA、#939 的完整父调用组合和整套验收继续分别核对。
