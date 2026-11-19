# 同一来源写入调用的 SELECT 与 UPDATE

[逐行首错](../row-write/README.md) · [UPDATE](../row-sql-update/README.md) ·
[SELECT](../row-sql-select/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定源码 `7b27876458a7212feb98a2aaf36560fe6443fb73`。组合沿用 #951 的实际
`write_candidate_record` 与 `write_candidate_record_using`，接入 #954 的四参数 UPDATE
及 #955 的两个 SELECT。唯一额外的隔离改写是把原 SHA-256 表达式委托给同一模块已有的
`sha256::digest`。该共享包装的库表达式与原调用相同，复用 A9 已有数学和有限输入合同。
真实父调用、选择分支和所有自有适配器均保持透明。LLBC 入场核对原 Vec 借用、连接、完整键和
六个原实例方法。生成体保留原字节。错误版只把原空槽语句的 `min(id)` 改为 `max(id)`。

两条 SELECT 和 UPDATE 使用同一组合视图中的 `rusqlite.Connection`。这个值保留一个
canonical Connection，原读取中的五种 SQLite 值按原构造及内容接到生成的 `ValueRef`。
原单 BLOB 和四元素字典通过实际迭代生成逻辑绑定输入，零参数仍走原计数检查。A4 继续提供
实际库的 prepare、原始 bind 首错、最终参数数目检查、首行与列、写入/step/reset 和完整错误。
在已编码字节上，已有键跳过空槽查询，只有明确无行才查询空槽。首错及无槽保留，成功选择后
同一接收者收到原字节、摘要、工作键与选中 ID，成功须恰好更新一行。

只有已有 A9 正常库条件满足时，摘要获得完整数学输出。本切片从已编码 Vec 开始。SQLite
实际选行及写入语义、同一活事务的来源、真实提交与回滚由 A4/V5 和原生证据继续落实。
统筹接受 #955 增加的库使用点后，组合登记及覆盖仍须统一跑原 Stage1 入口。
`actual_composed_write` 连接真实父调用与按序规格。`actual_composed_greatest` 展开同一父调用的
错误分支，保留 #955 的同规格 SELECT 反例。联合父调用的错误选行另用实际 SQLite 测试观察。
主控可采用 [覆盖候选](public-registration.patch)。

## ning 原生复验

`proof_checkout` 指本证明提交的干净源，`fixture_checkout` 指 #950 的精确
`56e28c2859ccdd1e5264bef03e9e35752d062fda`。两个候选分别在独立干净的
`7b278764` checkout 中运行。测试源直接使用 #950 原文件和本包一个额外的低层父调用夹具。
额外夹具用 SQL 独立构造已占 ID 3 与空槽 ID 7、11，使错版可以通过原已有键更新并到达
同一最小槽断言。它读取同一真实事务中的两个完整行，然后实际回滚并在原连接重新读回原状态。
#950 原三个用例另核公开保存、零行拒绝和第二行 SQL 拒绝后的回滚及重开。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$proof_checkout/$package/stage_source_row_composition_native.py" \
    --source "$PWD" --fixture "$fixture_checkout" --mode "$variant"
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
"${native[@]}" --no-run
case "$variant" in
    core)
        "${native[@]}" proof_source_row_composed
        "${native[@]}" controller::lifecycle_inbox::source_work::tests::codec_rows
        ;;
    greatest-vacancy)
        code=0
        "${native[@]}" proof_source_row_composed -- --nocapture > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'combined writer must update the lowest vacant slot' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

## 证明机复验

使用已批准的证明盘、固定工具包和原服务客户端。下面的 `capture_root`、`fixed_tools`、
`fixed_packages`、`registered_lean_mcp`、`shared_lock` 与 `four_cpus` 沿同批交接设置。
两个视图使用独立 `.lake/build`。输出计划列出整个物理 Lean 闭包。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in core greatest-vacancy; do
  taskset -c "$four_cpus" bash scripts/python-entry.sh "$package/source_row_sql_composition.py" \
      --mode "$variant" --output "$capture_root/capture-$variant" --tools "$fixed_tools" --lock "$shared_lock"
done
for variant in core greatest-vacancy; do
  root=SourceRowSqlComposition
  if [ "$variant" = greatest-vacancy ]; then root=SourceRowSqlCompositionWrong; fi
  project="$capture_root/lean-$variant"
  bash scripts/python-entry.sh "$package/prepare_source_row_sql_composition.py" \
      --mode "$variant" --capture "$capture_root/capture-$variant" \
      --reference "$capture_root/capture-core" --output "$project" \
      --lake-packages "$fixed_packages" --lean-mcp "$registered_lean_mcp"
  (cd "$project" && LEAN_NUM_THREADS=4 flock "$shared_lock" \
      taskset -c "$four_cpus" lake build "$root")
  taskset -c "$four_cpus" bash scripts/python-entry.sh "$package/diagnose.py" \
      --project "$project" --output "$capture_root/audit-$variant" --lock "$shared_lock"
done
```
