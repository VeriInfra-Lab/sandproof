# 来源 UPDATE 四参数

[逐行写入控制流](../row-write/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

固定 `7b27876458a7212feb98a2aaf36560fe6443fb73`，复用 #951 的原行写入适配器。
隔离 `legacy` 保留原 `params!`。`core` 在同一 `Connection::execute`、同一 SQL 和参数顺序上用
四个 `ToSqlOutput::Borrowed` 显式表达完整 BLOB、完整 32 字节摘要、完整 32 字节工作键和
原 `i64` ID。`swap-key-digest` 仅互换两个真实 BLOB 位置。候选 Rust 只注入独立源码副本。

同一原生夹具使用锁定 SQLite 和原 `RECORDS_TABLE`。正常用例在两个预留 ID 中指定实际目标，
核对更新计数、相邻原行和目标的完整 `state_json`、`sha256`、`work_key` 及原预算。步骤错误
用例让同一真实请求命中连接内触发器，核对 `SQLITE_CONSTRAINT_TRIGGER` 的原消息和原行。
物理行测试直接调用原 `SqliteSourceRow::update`。目标槽选择与父事务由 [#949](https://github.com/VeriInfra-Lab/sandproof/pull/949)、
[#950](https://github.com/VeriInfra-Lab/sandproof/pull/950) 分项验证。

## 形式范围

`actual_complete` 对原接收者、任意完整字节、两个 32 字节数组和任意 `i64` 证明：
实际生成的 `SqliteSourceRow::update` 使用原 SQL 及 slot 1–4 的
`state_json / sha256 / work_key / id`。A4 `prepare` 失败时保留原错误并结束，
成功时原顺序和完整值进入同一 `run`；其错误经原 `JournalError::Sqlite` 返回，成功保留
原 `usize` 行数。四项数组的 `Params::__bind_in`、共享迭代与 `ToSql` 都经过原生成体。
`exact_contract_rejected` 在独立生成的交换版上，对同一 `expected` 合同给出反例。
两版共用一个接收者见证，逻辑提供器按完整参数列表返回不同结果。
实际 SQLite 原生对照另观察到错误版仍更新一行，却将两列内容写反。

原 [A4 执行提供者](../../../../../docs/formal/dependencies.md#a4a6-v3-寿命-update-的两个参数)
负责同一活连接的完整 `prepare/run`、首个原始绑定错误、事后参数数目检查及
step/reset。这里的列表描述原始输入，实际原始绑定及其副作用由该提供者承担。
字符串字面量使用原共享 `Str` 构造；三个普通 BLOB 和一个整数的元素投影只用于
这个数组。源行选择、WHERE 对数据库状态的效果、真实摘要计算、事务身份/提交回执和
重启继续由独立切片与提供者落实。

## 证明机复验

在本证明 checkout 运行。输入为集成源 `7b278764` 当前的原 SQL，
固定 `x86_64-unknown-linux-gnu` 与仓库 Charon/Aeneas 版本；
`capture_root` 使用已批准的证明数据盘，`fixed_tools` 指原 Aeneas 工具根。
两个隔离项目共享原只读依赖和同服务客户端，使用各自的 `.lake/build`。
`shared_lock` 指原证明重任务锁，`four_cpus` 是协调准入核组。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in core swap-key-digest; do
    taskset -c "$four_cpus" bash scripts/python-entry.sh "$package/source_row_sql_update.py" \
        --mode "$variant" --output "$capture_root/capture-$variant" \
        --tools "$fixed_tools" --lock "$shared_lock"
done
for variant in core swap-key-digest; do
    project="$capture_root/lean-$variant"
    root=SourceRowSqlUpdate
    if [ "$variant" = swap-key-digest ]; then root=SourceRowSqlUpdateSwapped; fi
    bash scripts/python-entry.sh "$package/prepare_source_row_sql_update.py" \
        --mode "$variant" --capture "$capture_root/capture-$variant" \
        --reference "$capture_root/capture-core" --output "$project" \
        --lake-packages "$lake_packages" --lean-mcp "$registered_lean_mcp"
    (cd "$project" && LEAN_NUM_THREADS=4 flock "$shared_lock" \
        taskset -c "$four_cpus" lake build "$root")
    taskset -c "$four_cpus" bash scripts/python-entry.sh "$package/diagnose.py" \
        --project "$project" --output "$capture_root/audit-$variant" --lock "$shared_lock"
done
```

每个 `plan.json` 固定该入口的 12 份实际 Lean 文件及摘要。物理闭包包含原生成
Types/Funs、逐符号连接、原 SQL/SQLite/Foundations 类型模型和原证明审计。生成体
保留原字节。错误版准入还核对同一生成类型，以及程序正文仅互换两个输入。
`public-registration.patch` 为统筹登记的候选。统筹需将两份必检视图和 #952 原生
三版作为同批义务接到系谱，并以精确输入运行 `verify-stage1-protocol.sh`；
隔离检查回执只描述本切片。

## ning 复验

在三个独立干净的 `7b278764` checkout 中，各以 `variant=legacy`、`variant=core`、
`variant=swap-key-digest` 运行。`proof_checkout` 是本 PR 的精确 checkout。每版使用独立 Cargo target，
错误版需要单独的 `native_negative_log`。设定原普通与原生环境并保留完整普通入口回执。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = 7b27876458a7212feb98a2aaf36560fe6443fb73
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$proof_checkout/$package" \
    bash scripts/python-entry.sh - "$variant" <<'PY'
from pathlib import Path
import sys
from stage_row_sql_update import native

print(sys.argv[1], native(Path.cwd(), sys.argv[1]))
PY
tests=(cargo test --locked --offline -p sandproof-controller-service --features rusqlite/hooks --lib)
case "$variant" in
    legacy|core) "${tests[@]}" proof_source_update_binding ;;
    swap-key-digest)
        "${tests[@]}" --no-run
        code=0
        "${tests[@]}" proof_source_update_binding -- --nocapture > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'source update must bind digest before work key' "$native_negative_log"
        rg -F 'test result: FAILED. 1 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```
