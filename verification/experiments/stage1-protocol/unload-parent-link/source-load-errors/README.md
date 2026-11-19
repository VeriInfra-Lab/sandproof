# 原来源读取的首错顺序

[父配置合法域](../source-config-guards/README.md) ·
[元数据读取](../source-metadata/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包接续 #983，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
抽取根仍是实际 `codec::load`。重新抽取的原函数逐字节插入私有视图，
`actual_loader_is_parent` 将其接回 #981/#983 已审计的完整类型和调用链。
不改生产 Rust、公共 Lake/Cargo 配置或原证明工具。

## 原顺序与合同

| 先到阶段 | 失败结果 | 后续阶段 |
| --- | --- | --- |
| 版本与存在性 | 原 SQL 错误；旧版本按原表存在性处理 | 已由 #981 证明并原样复用 |
| 来源表 schema | 原 `JournalError` | 不查询记录表 schema |
| 记录表 schema | 原 `JournalError` | 不查询元数据计数 |
| 元数据计数 | 原 SQL 错误 | 不查询长度/预留 |
| 长度/预留 | 原 SQL 错误 | 不读取容量，不提前按计数拒绝 |
| 容量 | 原 `JournalError`，或缺配置的 `CorruptStore` | 不进入计数/预留/长度/容量守卫及载荷读取 |
| 数值守卫 | 原 `CorruptStore` | 已由 #981 证明，失败不读取载荷 |
| 元数据载荷 | 原 SQL 错误 | 不解码配置 |
| 完整配置检查 | 原 `JournalError` | 不检查配置合法域，不调用记录读取 |
| 配置/会话合法域 | 原 `CorruptStore` | 已由 #983 证明，失败不调用记录读取 |

新增首错定理对任意原类型值、连接和端口成立。只要求已到达阶段的前缀成功，
不假设后续端口会成功、返回或无效果；因此结果等式保留原 Aeneas `Result` 的
错误及效果语义，后续调用不能贡献效果。`Contract` 精确要求计数成功后，
长度/预留查询错误在数值计数拒绝之前原样返回。两版共用同一 `Contract`、
同一 `FormatPrefix`、同一 SQLite 表表示和反例。

错误 Rust 只在原长度/预留查询前增加 `if count != 1` 的 `CorruptStore` 返回，
原后置守卫保持。它实际编译、抽取并产生反例：表的唯一元数据行不存在时，
`count(*)` 为 0、后续 `WHERE id=1` 为 `QueryReturnedNoRows`。原函数先返回该
SQL 错误，错误版提前返回 `CorruptStore`。这组回复可由同一真实 SQLite 快照产生；
没有使用“计数为零却成功读到 id=1”的不可达见证。健康元数据前缀另有正常到达
原载荷查询的见证，完整正常读取沿用 #981 的完整配置/记录保留定理。

## 依赖与范围

本包复用 A4 的逐符号 SQLite 绑定、查询、列值/错误身份和同连接合同，
复用原 `TextDecode`、`Codec`、容量及记录读取端口。真实事务快照和耐久性由 V5/A4
提供。配置 JSON 字段与完整 map/seq 装配仍按 #982 的剩余义务推进；
本批没有把整个 Serde、SQLite 事务或业务处理器设为可信。
摘要值比较不证明材料真实性，配置相等不提供授权或实际文件/VM 存在事实。
认证、中心意图、物理资源和系统组合继续由原 V7/V1/V2/V4/V5/V6 提供。

## 形式复验

父 `guards_project`、`guards_audit` 必须是 #983 原版的 58 文件、175 项完整审计。
工具、归档、Rust 重建、实际根/字典、完整生成清单和每份日志都由物化器核对。
父文件逐字节保留，两个新视图各用独立的 `.lake/build`。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-load-errors
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original early-count-guard; do
    captured="$load_errors_evidence/$variant"
    project="$load_errors_evidence/$variant-project"
    audit="$load_errors_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/extract.py" \
        --variant "$variant" --output "$captured" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare.py" \
        --actual "$captured" --prior "$guards_project" --audit "$guards_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    count=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/registration-candidate.json")
    jq -e --argjson count "$count" '.complete and .physical_files == 61
        and (.audited | length) == $count and (.warnings | length) == 0
        and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
```

抽取复用 #982 的有界运行器：单视图 4 GiB target、四 CPU、Charon 240 秒、
Aeneas 120 秒上限，共享原重锁，不预留无关的全盘容量，也不清理共享缓存或失败证据。

## ning 原生复验

ning 持原 native-heavy 锁，对两个固定生产的干净副本使用独立 Cargo target。
夹具使用原 ready/prepare 和真实 SQLite。先正常读取全部 `Sources`，
事务内删除唯一元数据行并确认计数为零，再调用原 `load`。
正常版须保留原 `QueryReturnedNoRows`，回滚后完整来源逐字段相同。
错误版必须到达同一断言并显示 `CorruptStore`。受控 Host 夹具沿原原生范围使用，
该用例不启动产品 VM，也不声称数据库损坏是正常业务输入。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link/source-load-errors
bash scripts/python-entry.sh - "$proof_checkout/$package" "$variant" <<'PY'
from pathlib import Path
import sys

package, variant = Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(package))
from extract import early_count_guard

source = Path('controller-service/src/controller/lifecycle_inbox/source_work')
if variant == 'early-count-guard':
    codec = source / 'codec.rs'
    codec.write_text(early_count_guard(codec.read_text()))
else:
    assert variant == 'original'
for filename, fixture in [('tests.rs', 'fixture.rs.in'), ('codec.rs', 'native.rs.in')]:
    with (source / filename).open('ab') as stream:
        stream.write(b'\n' + (package / fixture).read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
if test "$variant" = original; then
    "${native[@]}" source_load_numbers_error_precedes_count_rejection | tee "$native_positive_log"
    rg -F 'test result: ok. 1 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" source_load_numbers_error_precedes_count_rejection > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'original numbers query error must precede count rejection' "$native_negative_log"
    rg -F 'CorruptStore' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
fi
```

编译失败、超时和环境失败独立保留，不计作性质反例。
完整普通、原生正反和公共整套验收交 ning/主控；本包提供
[登记候选](registration-candidate.json) 和 [覆盖候选](public-registration.patch)，不自行写公共登记。
