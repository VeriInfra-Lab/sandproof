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

本夹具是 V5 真实提交尾部的原生证据。已有提交结果未知的回归继续覆盖底层提交
已发生与已回滚两种可能。

## 原提交尾部的形式对应

在隔离源码中复用 [#939 的实际提交辅助函数](../finish/README.md)。将保存观察末尾的
原保存、两个钩子、提交以及成功返回原克隆记录提为同一个具名调用。Charon 父 IR 核对
初始原 `begin` 的同一事务供两个读取、World 和当前来源使用。只有通过原前置分支且
选中的原行需要写入时，同一事务和来源集合才传给尾部。Inbox 容量、钩子和原观察写入
后的克隆记录来自实际原值。保存字典只调用原 `codec::save`。

窄视图把实际生成的记录返回函数接至 #939 的保存、提交前钩子、提交和提交后钩子
首错定理。正常分支只交还原记录，失败按原错误返回。复用 #770 的 V5 事务关系，
在原活事务、前置校验和实际读回条件下，`CommitUncertain` 清空 memo；提交成功后的
钩子失败保留已发表候选。原错序生成体在同一条件下让后置钩子错误掩盖原提交错误。

父 IR 不是完整 Lean 父方法。固定 Aeneas 对原 `iter_mut().find` 的高阶生命周期与自由
生命周期约束报告未实现。查找和前置首错的完整对应、实际 `codec::save` 与 #956 行组合
的活事务连接，以及 Host 提供观察的物理真实性继续分列。记录相等及提交 memo 的定理
不赋予资源存在或授权结论。独立 SQLite 观察提供本文的持久字节见证。

### 形式复验

`prior_tail`、`prior_reordered` 指原 [#939](../finish/README.md) 材料化的正确与调序
项目。逐项校验原计划、工具入口与 59 份 Lean 闭包，复用原提交提供者。生成的新文件
以原字节落盘。比较旧辅助体时只忽略源行号注释。每个新闭包分别包含 61 份物理文件。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4
package=verification/experiments/stage1-protocol/unload-parent-link
for mode in parent tail reordered; do
    bash scripts/python-entry.sh "$package/observation_finish_capture.py" --mode "$mode" \
        --output "$capture_root/$mode" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
for mode in tail reordered; do
    prior=$prior_tail
    root=ObservationFinish
    if test "$mode" = reordered; then
        prior=$prior_reordered
        root=ObservationFinishReordered
    fi
    project="$capture_root/lean-$mode"
    bash scripts/python-entry.sh "$package/prepare_observation_finish.py" --mode "$mode" \
        --actual "$capture_root/$mode" --parent "$capture_root/parent" --prior "$prior" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
        --output "$capture_root/audit-$mode" --lock "$SANDPROOF_LOCK_FILE"
done
```

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

若复验形式抽取对应的隔离改写，在两份新的固定集成 checkout 各自先执行
`stage_native.py --mode core` 装入同一夹具，再从同批 `tail` 或 `reordered` 捕获中
逐字节取用 `source/controller-service/src/controller/lifecycle_inbox/source_work.rs`。
分别按上文正确与错误断言编译运行，证据绑定两种实际源码。

## 公共登记候选

统筹将 `parent` 的 Charon 结构核对、两个独立的实际生成视图、各自完整
`plan.json` 和原生正错结果接入既有 Stage1 检查。原生组件采用原入口及实际
形式改写的当前候选。保留生成的两个根各自的日志、依赖审计和真实反例，公共正确
根仅采用正例。[覆盖候选](public-registration.patch)供统筹复核。完整普通、公共
Stage1 和同批总入口仍按既有工作流执行。
