# 来源观察的实际全值相等

[首个完整匹配与判定](../observation-slot/README.md) ·
[原 SourceWork 相等](../../source-work-match/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。接入 #963 实际选择的两个
Rust 比较字典。从原已解码值开始。

- `journal_lookup_returns_first_complete` 在同一生成载体上复用
  `SourceWorkMatch.full_derived_contract`，接入原 `SourceWorkRecord.work`。
  对任意有限向量，当前实际扫描的可选索引等于第一个完整匹配的索引。
- `raw_full_contract` 覆盖 `RawSourceObservation` 派生相等的八个实际字段，
  包括配置字节向量、两个完整 32 字节值以及保存的内核与磁盘字节数。
  `journal_guard_refines` 使用该字典及编译器选择的默认 `PartialEq::ne`，给出
  空记录新写入、全文相等重放和全文不同冲突。手写合同前提现已由实际字典对应证明提供。
- 正误版共用同一完整重放规格和见证。真实 Rust 错误版只在比较中遗漏
  `executable_sha256`，同 binding、同其余七字段而异可执行摘要时实际返回重放，并违反
  `sameObservation`。真实 SQLite 原入口的正常提交、重放及冲突均继续用 #963 原夹具。

两个薄探针各自调用原扫描或判定，机械核对完整实参与所选字典。两份生成视图均与
#963 的实际父 IR 核对三种原载体、查找与判定的共用定义以及三个真实比较符号。
归档、源和工具指纹对齐后，原生成 `Types.lean` 和 `Funs.lean` 按字节使用。
本视图沿用已登记的布尔不等、Option 字典及完整字符串比较模型。配置 Vec、数组
与其它结构复用既有精确值定理。

本批提供比较层接线。前置认证与读回的完整首错、完整父方法、行级 SQL 活事务的
组合、公共 Stage1 和物理观察事实仍需各自验收。摘要定理的输入是 32 字节值，
不会借其相等判定实际内容、授权或资源存在。

## 形式复验

`prior_parent` 指 [#963](../observation-slot/README.md#形式复验) 的已完成
`parent` 捕获，由原报告和工具命令核对。正反闭包各 21 个物理 Lean 文件，
使用固定工具、共享锁和独立构建目录。各自的 `plan.json` 含原输入和复用文件的摘要。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in core skip-executable; do
    bash scripts/python-entry.sh "$package/observation_comparators_capture.py" --mode "$variant" \
        --output "$capture_root/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
for variant in core skip-executable; do
    project="$capture_root/lean-$variant"
    bash scripts/python-entry.sh "$package/prepare_observation_comparators.py" --mode "$variant" \
        --captures "$capture_root" --parent "$prior_parent" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    root="$(jq -r .root "$project/plan.json")"
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
        --output "$capture_root/audit-$variant" --lock "$SANDPROOF_LOCK_FILE"
    jq -e '.complete and .physical_files == 21' "$capture_root/audit-$variant/result.json"
done
```

## ning 原生复验

普通检查使用原无参数入口。原生正错验收分别用干净固定集成 checkout、隔离
`CARGO_TARGET_DIR` 和原离线编译环境。两个版本装载 #963 的同一 SQLite 夹具。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$proof_checkout/$package/stage_observation_comparators_native.py" \
    --source "$PWD" --mode "$variant"
native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
case "$variant" in
    core) "${native[@]}" source_observation_slot_ ;;
    skip-executable)
        code=0
        "${native[@]}" source_observation_slot_preserves_fresh_replay_and_conflict \
            > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'Err(JournalError::Conflict)' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

统筹可将证明根、真实错误版、父 IR 符号对应及精确清单纳入现有 Stage1 检查。
[覆盖候选](public-registration.patch)在同批完整验收后采用。
