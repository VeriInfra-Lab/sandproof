# 首次登记的保存与提交调用

[Unload 父调用](../README.md) · [证明工作流](../../../../../docs/proof-workflow.md)


本候选固定 integration `7b27876458a7212feb98a2aaf36560fe6443fb73`。#935 纳入的
原验收夹具和原生产输入分别校验。隔离改写把原 `codec::save`、`BeforeCommit`、
`WorldTransaction::commit`、`AfterCommit` 提取到 `finish_source_work`。保存字典只转发
原连接、完整来源值与 Inbox 容量；提交消费同一个事务。完整父方法的 IR 核对实际保存
字典、`ControlFlow`、成功分支的原记录返回和错误分支的原残差返回。

窄视图证明保存、提交前钩子或提交任一步失败时保留首错和钩子状态，以及正常提交后
才接收后置钩子的结果。先核对 #936 前缀的原始输入及函数正文。完整生成父方法须把
同一个已打开事务和同一 Inbox 的容量传入后段。前缀和提交模块的生成错误类型实例同名，
两份生成文件各在其原环境检查。两侧经 #930 的原事务表示连接，保持原连接和 Drop 行为、
World、已提交值及输入前沿。在既有 V5 验证和同次提交读回条件下，`CommitUncertain`
保留原 SQLite 错误且 committed memo 的读回为空。正常提交后钩子失败时，定理同时
给出钩子的原错误和该次提交对原候选 World 的读回。数据库的耐久结果由实际 SQLite
观测提供。真实原生用例覆盖提交后钩子失败仍可从独立连接及重开
读回一份原记录，以及丢失同一提交回包时实际回滚和实际提交的两种情况。

调序版把 `AfterCommit` 移到原提交之前。其 Lean 反例在同样的两个可能错误条件下
得到错误的钩子首错，实际 SQLite 夹具则在同一提交后钩子断言处发现原记录未耐久。
原 `codec::save` 的内部 Serde/SQL 关系仍由 V5 提供。本批未证明保存实现、全部数据库
状态、Drop 的物理回滚或完整父方法 Lean。原保存体的完整抽取在工具的嵌套循环提前
返回处停止，独立保留该诊断。

```bash
package=verification/experiments/stage1-protocol/unload-parent-link
export SANDPROOF_ACCEPTANCE_ROLE=formal
for mode in tail parent reordered; do
  bash scripts/python-entry.sh "$package/finish_capture.py" --mode "$mode" \
    --output "$new_finish_capture/$mode" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
for mode in tail reordered; do
  root=UnloadFinish
  if test "$mode" = reordered; then root=UnloadFinishReordered; fi
  bash scripts/python-entry.sh "$package/prepare_unload_finish.py" --mode "$mode" \
    --tail "$new_finish_capture/$mode" --parent "$new_finish_capture/parent" \
    --prefix "$original_prefix" --prefix-parent "$original_prefix_parent" \
    --begin "$original_begin_capture" --output "$new_finish_lean/$mode" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
  (cd "$new_finish_lean/$mode" && LEAN_NUM_THREADS=4 flock -n "$SANDPROOF_LOCK_FILE" \
    taskset -c "$four_cpus" lake build "$root")
  bash scripts/python-entry.sh "$package/diagnose.py" --project "$new_finish_lean/$mode" \
    --output "$new_finish_audit/$mode" --lock "$SANDPROOF_LOCK_FILE"
done
```

`original_prefix`、`original_prefix_parent` 与 `original_begin_capture` 分别来自 #936
两份原前缀捕获及 #930 的已准入 Begin 捕获。各输出父目录事先创建，子目录须全新。
ning 在各自独立、干净的 `7b278764` 原生 checkout 中设置 `variant=original`、
`variant=candidate` 或 `variant=reordered`。证明工作树由 `proof_checkout` 指向，
调序版把 `native_negative_log` 指向本次独立日志。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
  bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
import finish_capture as probe
from prepare_unload_finish import CANDIDATE, REORDERED

variant = sys.argv[1]
path = Path(probe.prepare.SOURCE)
assert probe.digest(path) == probe.prepare.SOURCE_INPUTS[probe.prepare.SOURCE]
if variant in ('candidate', 'reordered'):
    path.write_text(probe.factor(path.read_text(), reordered=variant == 'reordered'))
    assert probe.digest(path) == (REORDERED if variant == 'reordered' else CANDIDATE)
else:
    assert variant == 'original'
PY
fixture=87845bc241fe09c7803c2f9e0a1cdebf4c823b07
fixtures=verification/experiments/stage1-protocol/unload-prepare-transaction
target=controller-service/src/controller/lifecycle_inbox/source_work/tests.rs
git fetch --no-tags --no-write-fetch-head origin refs/pull/929/head
{
  printf '\n'
  git show "HEAD:$fixtures/native-tests.rs.in"
  git show "$fixture:$fixtures/begin-contention.rs.in"
  cat "$proof_checkout/$package/current-inputs/native-first-errors.rs.in"
  cat "$proof_checkout/$package/finish/native-finish.rs.in"
} >> "$target"
native=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case "$variant" in
  original) "${native[@]}" proof_first_prepare_ ;;
  candidate) "${native[@]}" controller::lifecycle_inbox::source_work::tests:: ;;
  reordered)
    "${native[@]}" --no-run
    rc=0
    "${native[@]}" proof_first_prepare_after_commit_error_preserves_original_durable_work \
      -- --nocapture > "$native_negative_log" 2>&1 || rc=$?
    test "$rc" = 101
    rg -F 'AfterCommit failure must leave the original work durable' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
    ;;
esac
```

## 公共登记候选

主控在最终采用的真实源码确定后，把 `tail`、`parent`、`reordered` 三份原捕获、
两份 `plan.json` 列出的全部实际 Lean 文件、八条正例定理和调序反例加入现有
`verify-stage1-protocol.sh` 的正式范围。正例及调序版使用独立的 Lean 库和审计；
两者各有 59 份文件，含 #930 和 #770 的真实生成体与对应证明。原生当前候选须在
`proof_native_suite` 登记原实现、候选和实际编译后到达断言的调序版。完整普通入口
只协调执行一次，原捕获、工具、配置、原生回执和全部审计均按当前输入校验。
[覆盖候选补丁](public-registration.patch) 供主控复核。共享登记仍沿用
[父包说明](../README.md) 中 #776 的准入条件。
