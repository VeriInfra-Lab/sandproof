# 来源观察在 SQLite 提交边界的可见性

[准备事务的提交尾部](../finish/README.md) · [来源写入的 SELECT 与 UPDATE](../row-sql-composition/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定集成提交 `b0d791ccc4d4e058b710dbf8f46766652b2853df`、
树 `7ca91afd9cd79601d562f3c636066d192fc25fc5`。临时夹具调用原
`ControllerJournal::save_source_observation`，在同一个真实 SQLite 账本中用第二条连接读取
原槽号、完整 JSON、SHA-256、工作键和预留额度。提交前外部连接读取原值，另一写事务被
SQLite 拒绝。提交后外部连接读取精确新值，且能取得并释放写锁。

分别在 `BeforeCommit` 与 `AfterCommit` 回报注入错误后重开原账本。前者保留原行，
之后用原参数重试时完成写入并依次命中两个钩子。后者保留已提交的新行，重试返回
同一完整记录而不再调用提交钩子。新版复用既有 SourceWork 夹具。错误版只把原
`AfterCommit` 调用放到 `tx.commit()` 前，必须编译成功并到达相同的可见性断言。

本夹具是 V5 真实提交尾部的原生证据。父调用首错的一般性对应、生成类型与真实事务
接线、以及生产 Host 观察的事实仍有独立义务。已有提交结果未知的回归继续覆盖底层
提交已发生与已回滚两种可能。正式登记按原证明工作流由统筹合入。

## ning 原生复验

`proof_checkout` 为本稿精确 checkout。对 `core` 和 `early-after-commit` 分别准备干净
固定集成 checkout、独立 Cargo target 和原始证据路径。运行目录为对应固定集成 checkout。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/observation-commit
bash scripts/python-entry.sh "$proof_checkout/$package/stage_native.py" \
    --source "$PWD" --mode "$variant"
native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
case "$variant" in
    core)
        "${native[@]}" source_observation_commit_publishes_exact_bytes_between_hooks_and_replays_after_reopen
        "${native[@]}" source_work_unknown_commits_keep_actual_original_question_and_result
        ;;
    early-after-commit)
        code=0
        "${native[@]}" source_observation_commit_publishes_exact_bytes_between_hooks_and_replays_after_reopen \
            > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'source observation visibility at AfterCommit' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```
