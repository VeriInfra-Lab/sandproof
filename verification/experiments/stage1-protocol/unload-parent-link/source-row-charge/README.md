# 来源行计费与未来工作键

[原记录容量](../source-record-bounds/README.md) ·
[来源读取首错](../source-load-errors/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包接续 #984，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
实际根为 `source_work::codec::charged_bytes`，同时保留其调用的原
`lifecycle_inbox::codec::charged_bytes` 透明体。后一段业务计费没有改为外部假设。
原配置与 SQL 行类型沿 #978 复用，不新造协议模型或更改生产 Rust。

## 性质与原顺序

通用计费先读取第 1 列的完整 BLOB，再读取第 3 列的 i64 预留。
非 BLOB、负预留、预留短于载荷或超过 `max_sql_value_bytes` 均按原顺序拒绝。
正常值先从 `current` 减去全部载荷长度，再加入完整预留；减法下溢和加法溢出
返回 `MaintenanceCapacity`。SQL 原错误保持，前阶段失败不读取后续列。

来源层只在原 Statement 恰有五列且第 4 列为 Null 时，再用 checked_add 加 32。
四列元数据行不读取第 4 列；非 Null 的 Integer/Real/Text/Blob 均保留原计费，
不擅自加强为该函数必须验证工作键类型或长度。工作/JSON/键的合法性仍由原读取和验证器负责。

- `base_keeps_complete_reservation_and_checked_arithmetic`、完整长度和算术结果定理
  对任意原行、字节、容量及整数证明完整值及精确边界。
- `original_null_match_covers_every_variant` 覆盖实际五种 ValueRef 构造器。
  `original_future_key_contract` 对任意原行证明 Null 键必须加入全部 32 字节。
  其它列数、非 Null、列错误和前段错误分别有对应定理。
- 在 V5 提供 `current = 完整载荷长度 + 48` 时，原实际函数的正常结果为：
  四列元数据 `future + 48`，五列空记录 `future + 80`。这连接原初始化公式中的
  48/80 开销，不声称原 initialize 或整库容量已经证明。
- 正常见证使用完整 `null` 字节、其真实 32 字节摘要和原最小配置的一行预留
  222212。实际 `current=52`，通用计费为 222260，来源计费为 222292。
- 错误 Rust 只把原 `.checked_add(32)` 改为 `.checked_add(0)`。
  两版共用同一个 `Contract`，错误版在同一正常行少计 32 字节并被否定；
  另有加键溢出被错误版放行的算术见证。

溢出见证显式传入函数的 u64 `current` 边界，不把该参数说成真实 SQLite
测出了一行这么大的数据。真实调用者的 `current` 来源及较小物理上界仍由 V5 连接。

## 表示、抽取与依赖

固定工具对原宏的首次尝试在 `matches!(…Null)` 借用分支合并处失败，原始输出保留。
后续仅把同一表达式提成透明 `source_key_is_null(ValueRef)`，保留原取列位置、
错误转换、短路和全部五种分支。新 helper 与两段业务函数均实际抽取并进入完整审计；
未把判定设为 opaque，也没有导入外部模板中的 axiom。

原 Row、Statement、ValueRef、CapacityConfig 与 JournalError 使用原表示。
`get_ref`、Row::as_ref、column_count 和 as_blob 的最小逐符号合同列在
[依赖候选](dependencies-candidate.patch)，交 V0/A4 主控采纳；目前未作为已接受登记。
现有 usize 索引、i64 FromSql、Option/Result、有限切片和 checked 整数合同继续复用。
同一个受保护 SQLite Row/Statement、原列缓冲和有效借用由 A4/V5 提供；
本包不扩大为整个 SQLite、事务或业务处理器可信。

V5 仍须连接 `capacity::row_bytes` 的完整 `current`、`sql_usage` 的表分派、
原配置/同事务及整体预算。此处行形状不提供授权、持久接受、实际资源或完整材料事实。
JSON/摘要校验、实际文件/VM 和系统组合仍沿原模块义务推进。

## 形式复验

`records_project`、`records_audit` 是 #978 原版的 52 文件、145 项完整服务审计。
物化器验证实际归档、源码重建、工具状态、原根和透明 helper、生成清单、原容量全部字段
及父输入闭包；新文件各自用独立 `.lake/build`。所有输出使用新目录，保留失败和缓存。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-row-charge
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original omit-future-key; do
    captured="$row_charge_evidence/$variant"
    project="$row_charge_evidence/$variant-project"
    audit="$row_charge_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/extract.py" \
        --variant "$variant" --output "$captured" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare.py" \
        --actual "$captured" --prior "$records_project" --audit "$records_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    count=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/registration-candidate.json")
    jq -e --argjson count "$count" '.complete and .physical_files == 55
        and (.audited | length) == $count and (.warnings | length) == 0
        and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
```

本包复用原有界运行器，单 target 4 GiB、四 CPU、Charon 240 秒、Aeneas 120 秒上限，
使用原共享重锁。完整公共登记与统一验收仍交主控，不以本局部入口替代。

## ning 原生复验

ning 持原 native-heavy 锁，在两个固定生产的干净副本使用独立 Cargo target。
用例以真实 SQLite query_row 提供原 Row；不新建替代计费实现，不启动产品 VM。
相同代码检查 Null/已占用键、四/六列、全部非 Null 类型、首错和算术边界。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link/source-row-charge
bash scripts/python-entry.sh - "$proof_checkout/$package" "$variant" <<'PY'
from pathlib import Path
import sys

package, variant = Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(package))
from extract import omit_future_key

codec = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
if variant == 'omit-future-key':
    codec.write_text(omit_future_key(codec.read_text()))
else:
    assert variant == 'original'
with codec.open('ab') as stream:
    stream.write(b'\n' + (package / 'native.rs.in').read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
if test "$variant" = original; then
    "${native[@]}" source_row_charge_ | tee "$native_positive_log"
    rg -F 'test result: ok. 2 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" source_row_charge_ > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'empty source row must reserve the future 32-byte key' "$native_negative_log"
    rg -F 'future work key charge must reject overflow' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 2 failed;' "$native_negative_log"
fi
```

错误版必须编译后实际到达上述两条断言。编译、环境失败或超时不能算性质反例。
完整普通入口及原生正反交 ning；[登记候选](registration-candidate.json)、
[覆盖候选](public-registration.patch) 和依赖候选交主控统一采用。
