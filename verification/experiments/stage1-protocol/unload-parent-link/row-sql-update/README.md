# 来源 UPDATE 四参数原生对照

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

原双 BLOB 的 A4 执行合同可复用已审的整次执行接口。四参的生成绑定、原参数描述符及形式审计
另行接续。下列命令仅生成当前隔离版和 SQLite 回执。

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
