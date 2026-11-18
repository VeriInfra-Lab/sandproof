# 来源保存的同类型提供者

[来源首个完整匹配](../first-match/README.md) · [完整工作比较](../../source-work-match/README.md) ·
[已存事实核对](../../source-preservation/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定 integration `7b27876458a7212feb98a2aaf36560fe6443fb73`。隔离副本从 #944 的
精确 Rust 调用取出键查询接口。父 `retains_saved_records` 仍传入两份原记录向量与
同一集合。父选择的 `JournalSavedKeys` 仍用原键字节调用 `BTreeSet::contains`。
共享业务根 `retains_with_saved_keys` 使用同一生成 `SourceWorkRecord`，完整工作比较和
已存事实检查在同一生成环境均有原透明正文。原 IR 核对父参数、键查询接线和两个提供者的
实际操作数。两个生成环境比较全部共用定义，另与 #944 核对原六个扫描定义。

`original_providers_refine_retention` 复用 `SourceWorkMatch.full_derived_contract`、
`SourcePreservation.preserves_exact` 和 #944 首匹配定理。对任意当前和后继记录向量，
每个旧记录先查原 32 字节 binding，成功时选择首个完整 `SourceWork` 匹配，并用同一
旧记录和同一选中记录验证旧 producer、consumer 与 retirement。该结论的唯一键式输入
是 `SavedKeyCheck.contains` 对同一载体与完整字节的正常响应，对应的 `known` 由 V0
提供。A6 现有 Global 插入合同范围见[逐符号依赖](../../../../../docs/formal/dependencies.md#a6-btreeset-普通键插入)。
需要的下一条最小合同是固定 `Global`、普通 `[u8; 32]` 的 `BTreeSet::contains` 对同一集合
值返回是否含有完整 32 字节键。写入阶段另需同一键域的插入、重复拒绝和回写关系。

两个 Lean 根共用正常与改写配置长度的完整记录值和同一比较规格。仅按 binding 的 Rust
变体只变更内层选择的比较方法，保留同一外层、键端口及实际事实提供者。复验材料按
各闭包的 `plan.json` 逐文件列出摘要。旧证明模板只替换生成命名空间和导入绑定，原生成
`Types`、`Funs` 保留原工具字节。Option、Bool、String 仅使用已登记的 A6 值模型。

结论从已解码记录和键接口响应开始。原 SQL、JSON/SHA、事务持久性、认证及物理材料状态
继续按各自的接口核对。原生、公共 Stage1 入口及合并候选由主控和 ning 验收，
[覆盖候选](public-registration.patch)供公共记录采用。

## 抽取和 Lean 复验

在 msra 使用原锁、固定工具、四核、独立 `.lake/build`。下列变量沿用
[上一批形式复验](../first-match/README.md#形式复验)。`prior_capture` 指 #944 带原检查器完整
收据的捕获目录，其中 `factored` 和 `binding-only` 来自其原命令。新捕获遵守原 80 GiB
启动、持久工作区 64 GiB 停机及四核预算。保留原始证据和失败记录。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in providers binding-only parent; do
    bash scripts/python-entry.sh "$package/source_retention_providers.py" --mode "$mode" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE" --output "$new_capture/$mode"
done
for mode in providers binding-only; do
    root=RetentionProviders
    reference=factored
    if test "$mode" = binding-only; then
        root=RetentionProvidersBindingOnly
        reference=binding-only
    fi
    bash scripts/python-entry.sh "$package/prepare_retention_providers.py" --mode "$mode" \
        --generic "$new_capture/$mode" --parent "$new_capture/parent" \
        --baseline "$prior_capture/$reference" --output "$new_lean/$mode" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    (cd "$new_lean/$mode" && LEAN_NUM_THREADS=4 flock -n "$SANDPROOF_LOCK_FILE" \
        taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$new_lean/$mode" \
        --output "$new_audit/$mode" --lock "$SANDPROOF_LOCK_FILE"
done
```

## ning 原生复验

在两个新 `7b278764` 干净 checkout 中分别运行 `variant=providers` 和 `variant=binding-only`。
`proof_checkout` 为本 PR 的精确根。两侧使用独立 Cargo target 和原正常预算。保留完整普通
入口回执和以下实际 Rust 对照。既有 `native-first-match` 在原 SQLite 上重读保存后的同一行。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
from source_first_match import HERE as first_match
from source_save_capture import HERE as save
from source_retention_providers import stage_port

stage_port(Path.cwd(), sys.argv[1])
fixture = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
with fixture.open('ab') as file:
    for case in (save / 'native-save-order.rs.in', first_match / 'native-first-match.rs.in'):
        file.write(b'\n' + case.read_bytes())
PY
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case "$variant" in
    providers) "${native[@]}" controller::lifecycle_inbox::source_work::tests:: ;;
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
