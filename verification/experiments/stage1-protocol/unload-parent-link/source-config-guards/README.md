# 来源配置的完整合法域与读取拒绝

[元数据读取](../source-metadata/README.md) ·
[配置 JSON 接线候选](../source-metadata-json/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包接续 #981/#982，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
实际抽取根仍是原 `codec::load`，准入核对它恰好调用一次同一个
`SourceObservationConfig::valid`。固定工具分别抽取原版和仅遗漏
`max_configuration_bytes` 检查的 Rust 错误版。

## 定理与表示

`Limits` 直接复用 #978/#981 的原六字段类型。以下范围均含上下界。

| 原字段 | 合法范围 |
| --- | --- |
| max_work_bytes | 1024 到 1048576 |
| max_configuration_bytes | 1 到 1048576 |
| max_executable_bytes | 1 到 67108864 |
| max_kernel_bytes | 1 到 268435456 |
| max_disk_bytes | 1 到 i64::MAX |
| max_material_bytes | 1 到 i64::MAX |

- `actual_guard_has_all_six_bounds` 对任意原类型值证明全部六项的充要条件，
  原 `i64::MAX as u64` 转换对应 9223372036854775807。
- `actual_valid_is_parent` 把本次实际生成函数接回 #981 的原函数。
  `session_guard_is_exact` 接原父读取的 1 到 4096 会话范围。
  最小合法配置、零配置长度和会话边界都有正常/拒绝见证。
- `invalid_limits_skip_original_records` 和 `invalid_sessions_skip_original_records`
  对任意记录回调证明直接返回 `CorruptStore`。回调保留完整效果和不返回行为。
  `actual_load_rejects_before_original_records` 再接原格式/schema/元数据前缀、
  原载荷查询与原完整配置，得到同一真实 `load` 的提前拒绝。
- 错误版在配置长度为零、其余五项合法时返回 true，
  `omitted_field_breaks_same_contract` 否定同一个六项 `Contract`。

本批从已解码配置开始。标准整数比较、转换及新建 Range 的合同沿用既有 A6；
JSON 字段/字节实例仍按 #982 的明确剩余义务处理。SQLite 回复与同一连接、
原策略/容量、事务和耐久性由 V5/A4 提供，认证和实际资源沿原模块分工提供。
合法范围本身不提供操作授权、实际材料或资源存在事实。

## 形式复验

`metadata_project`、`metadata_audit` 是 #981 原版的 55 文件、160 项完整审计。
父文件逐字节保留。新原版与错误版均使用独立生成视图和 Lake build。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-config-guards
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original omit-configuration-bound; do
    captured="$config_guards_evidence/$variant"
    project="$config_guards_evidence/$variant-project"
    audit="$config_guards_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/extract.py" \
        --variant "$variant" --output "$captured" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare.py" \
        --actual "$captured" --prior "$metadata_project" --audit "$metadata_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    count=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/registration-candidate.json")
    jq -e --argjson count "$count" '.complete and .physical_files == 58
        and (.audited | length) == $count and (.warnings | length) == 0
        and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
```

## ning 原生复验

ning 持原 native-heavy 锁，对两个固定生产的干净副本使用独立 Cargo target。
新夹具使用原 ready/prepare 和真实 SQLite。普通配置测试逐字段检查下界前一值、
下界、上界和上界后一值。父读取测试在原配置 JSON/摘要正确、元数据形状合法时，
把后续记录预留列写成会使原聚合返回列型错误的 TEXT。非法配置或会话数必须先返回
`CorruptStore`，回滚后重新读回全部原来源。受控 Host 夹具沿原原生范围使用。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link/source-config-guards
bash scripts/python-entry.sh - "$proof_checkout/$package" "$variant" <<'PY'
from pathlib import Path
import sys

package, variant = Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(package))
from extract import omit_configuration_bound

if variant == 'omit-configuration-bound':
    limits = Path('controller-service/src/capture/source.rs')
    limits.write_text(omit_configuration_bound(limits.read_text()))
else:
    assert variant == 'original'
source = Path('controller-service/src/controller/lifecycle_inbox/source_work')
for filename, fixture in [('tests.rs', 'fixture.rs.in'), ('codec.rs', 'native.rs.in')]:
    with (source / filename).open('ab') as stream:
        stream.write(b'\n' + (package / fixture).read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
if test "$variant" = original; then
    "${native[@]}" source_config_guard_ | tee "$native_positive_log"
    rg -F 'test result: ok. 2 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" source_config_guard_ > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'every source configuration bound must be checked: max_configuration_bytes=0' "$native_negative_log"
    rg -F 'invalid source configuration must reject before the aggregate' "$native_negative_log"
    rg -F 'InvalidColumnType' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 2 failed;' "$native_negative_log"
fi
```

编译失败、超时和原断言失败分别保留，错误版必须实际到达上述两个性质断言。
完整普通、原生正反和公共整套登记由 ning/主控统一验收。
[登记候选](registration-candidate.json) 与 [覆盖候选](public-registration.patch) 随包提供。
