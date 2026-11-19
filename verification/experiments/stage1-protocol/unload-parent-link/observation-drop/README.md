# 观察保存首错与原事务 Drop

[观察与 SQL 组合](../observation-sql-save/README.md) · [原提交尾部](../observation-commit/README.md) ·
[SQLite 提交与缓存](../../../sqlite-commit/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定生产 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。在 #970 的同源联合抽取上保留实际 Drop、
回滚闭包和缓存单元。生产代码保持固定。

## 正常返回路径与状态

Charon 正常控制流检查逐条追踪保存、提交前钩子、提交、提交后钩子和原事务的条件 Drop。
保存或提交前钩子返回失败时，原事务尚未移动，正常返回路径经过它的 Drop。进入 `commit`
时，实参通过 Move 临时项消费同一原事务，外层条件 Drop 对已移动位置不作第二次析构。
进入 `commit` 后其内部校验失败的活事务清理由原 V5 路径处理。

实际 Drop 将原 `transaction` 槽、`committed.value` 和 `candidate` 交给
`rollback_if_live`。该闭包把原事务交给 rusqlite `Transaction::rollback` 并原样返回。
实际单元适配只写原 `committed`，值来自给定实参。联合收据绑定父返回路径、Drop glue、
原事务/单元借用及生成声明。`rollback_if_live` 与已登记 SQLite 核心字节相同。

固定 Aeneas 生成的外层 Drop 仅返回新事务槽，生成的共享 RefCell 适配未返回原 committed
单元的新值。私有证明视图保留实际回滚核心和原 #570 的三个方程，通过已登记 B2 绑定
该单元，并通过 B4 绑定原事务和回滚回调。原生成声明不改写。

- `row_error_and_original_rollback` 复用 #970 的完整首个失败行定理。原 SQL 错误、Begin
  回写连接及 hook 状态保持。V5 提供同一 Begin 事务仍有原活槽；回滚的正常返回决定
  该槽变为空，`Ok` 保留旧 memo，`Err` 清空 memo。原 SQL 错误不被回滚错误替换。
- `consumed_slot_has_no_rollback` 复用空槽方程。它只需要原槽已经为空，无需回滚回报。
- 实际错误版只省略回滚失败后的清空。它同样编译并由固定工具抽取。相同的失败缓存
  方程在存在旧 memo 时被 `retained_memo_violates_failed_rollback` 正面推翻。

A4 提供成功回滚的物理事务语义。回滚失败只得到缓存失效；真实 SQLite 状态、跨行增量、
连接可用性和后续重开仍按 V5/A4 提供者核对。全部定理限正常返回，不覆盖 panic 或进程
崩溃后的析构。条件 `LiveDrop` 绑定同一事务和原已提交单元，由实际 V5 视图实例化。

## 精确复验

`joint_capture`、`joint_project`、`joint_audit` 是 #970 的联合捕获、实际项目及完整服务
收据。新视图重新抽取两份 Drop，并核对实际生成核心、原已采用库及既有项目文件摘要。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
for mode in core keep-cache; do
    bash scripts/python-entry.sh "$package/observation_drop_capture.py" --mode "$mode" \
        --output "$capture_root/$mode" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/prepare_observation_drop.py" \
    --core "$capture_root/core" --mutant "$capture_root/keep-cache" --joint-capture "$joint_capture" \
    --prior "$joint_project" --audit "$joint_audit" --output "$project" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build ObservationDrop)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 39 and (.audited | length) == 82' "$audit/result.json"
```

### ning 原生验收

正反各从干净固定 integration checkout 使用独立 Cargo target。以下入口在原测试机执行。
测试用真实 SQLite 结束事务，再由原 `Drop` 执行自己的 rollback，于任何缓存读取前
检查原 memo 是否失效。这样可区分立即清空和后续一致读重新填充。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from hashlib import sha256
from pathlib import Path
import sys
from observation_drop_capture import MEMO_PATH, MEMO_SHA, memo_source
from check_observation_drop import __file__ as checker

memo = MEMO_PATH.read_bytes()
assert sha256(memo).hexdigest() == MEMO_SHA
MEMO_PATH.write_bytes(memo_source(memo, sys.argv[1]))
fixture = Path(checker).with_name('observation-drop') / 'native.rs.in'
transaction = Path('controller-service/src/controller/transaction.rs')
with transaction.open('ab') as handle:
    handle.write(b'\n' + fixture.read_bytes())
PY
native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
test_name=original_drop_invalidates_memo_when_sqlite_has_already_rolled_back
case "$variant" in
    core) "${native[@]}" "$test_name" ;;
    keep-cache)
        code=0
        "${native[@]}" "$test_name" > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'failed original Drop rollback must clear the memo before any read' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

普通和原生正式回执由 ning 统一取得。#950 的实际第二行回滚和 #959 的原提交可见性继续保留
各自原证据。公共 Stage1 登记由统筹复核 [覆盖候选](public-registration.patch)。
