# 保存来源观察的前置首错

[原来源观察守卫](../../source-observation/README.md) ·
[当前来源与旧观察的全值比较](../observation-comparators/README.md) ·
[后续提交尾部](../observation-commit/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。按真实
`ControllerJournal::save_source_observation` 保存前的语句提取前缀。生产源保持固定。

## 顺序与实际接线

1. 使用原 `RawSourceObservation::matches` 检查已解码的观察和原 work 的 requirement。
   拒绝时返回 `InvalidHostEvidence`。任何后续接口响应都不影响这个结果。
2. 复制原 active epoch，再从原 SQLite connection 和 committed World 调用 Begin。
   Begin 错误原样返回。
3. 读取 Inbox。读取错误原样返回，合法空值返回 `UnsupportedStore`。
4. 使用原 Inbox、完整原请求及已提供的认证上下文调用 `authenticated`。
   其明确错误先于来源读取。
5. 从同一事务读取 Sources。读取错误原样返回，合法空值返回 `UnsupportedStore`。
6. 用同一事务、同一 World、Inbox、原请求、来源限制和 active epoch 重建当前来源。
   原重建错误先于匹配；完整 `SourceWork` 不一致返回 `Conflict`。
7. 完整匹配交回原事务、Inbox、原来源集合。仅前缀成功可进入 #963 的查找与重放
   判定。只有原判定的 `Fresh` 分支可进入 #959 的提交尾部。

薄适配只转发现有操作和原错误构造器。实际父 LLBC 核对所选六个方法、原实参、
三件返回值及严格的成功分支。当前来源比较选用与 #964 全文查找相同的完整工作字典。
局部证明展开真实提取的 Rust 前缀，逐个回报上述首错，并对同一返回值给出成功定理。
`actual_evidence_first` 在当前 view 复用历史 `matches_exact`。

查找、认证、读取及重建接口按真实返回值参数化。V7 提供独立认证上下文，V5 提供
事务及 SQL 行对应，V6 提供物理来源事实。入口接线、局部首错和实际 native 对照
已核对。完整父方法 Lean、回调内部的持久读取及与行级 SQL 的完整活事务组合仍分列。
结论从已解码值开始。

## 证据与复验

正确和错误根共用原完整 guard 规格。真实 Rust 错误版只把 Begin 移到观察守卫之前。
同一正式反例在 guard 原应拒绝且 Begin 也失败时返回另一个错误。真实 SQLite 夹具
用第二连接持有写锁，实际阻塞 Begin 的 WAL 预检。正确版仍返回
`InvalidHostEvidence`，错序版先返回真实 `MaintenanceCapacity`。另一个正确
夹具同时提供错认证上下文和损坏来源行，观察原 `Conflict` 先于
`CorruptStore`。继续复跑 #963 的正常写入、无钩子重放及冲突。

形式复验用固定工具、共享重锁及两份隔离 Lean 构建，从原归档录入同一完整输入。
正例 11 个物理文件，错误版 10 个物理文件。保留原实时诊断与依赖审计。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4
package=verification/experiments/stage1-protocol/unload-parent-link
for mode in parent core begin-first; do
    bash scripts/python-entry.sh "$package/observation_prefix_capture.py" --mode "$mode" \
        --output "$capture_root/$mode" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
for mode in core begin-first; do
    view="$capture_root/lean-$mode"
    bash scripts/python-entry.sh "$package/prepare_observation_prefix.py" --mode "$mode" \
        --captures "$capture_root" --output "$view" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    root="$(jq -r .root "$view/plan.json")"
    (cd "$view" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$view" \
        --output "$capture_root/audit-$mode" --lock "$SANDPROOF_LOCK_FILE"
    jq -e .complete "$capture_root/audit-$mode/result.json"
done
```

## ning 原生复验

普通检查沿无参数入口。正反各用干净固定 integration checkout、独立 Cargo target
和原离线编译环境。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$proof_checkout/$package/stage_observation_prefix_native.py" \
    --source "$PWD" --mode "$variant"
native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
case "$variant" in
    core)
        "${native[@]}" source_observation_prefix_
        "${native[@]}" source_observation_slot_preserves_fresh_replay_and_conflict
        ;;
    begin-first)
        code=0
        "${native[@]}" source_observation_prefix_invalid_evidence_precedes_sqlite_begin \
            > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'invalid observation must precede actual SQLite Begin: Err(MaintenanceCapacity)' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

统筹可将两份精确生成根、父 IR、原生正反结果纳入现有 Stage1 检查。完成正式
验收后采用 [覆盖候选](public-registration.patch)。
