# 来源元数据读取与原预留行扫描

[原预留行守卫](../source-record-bounds/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

生产输入固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，
树 `7ca91afd9cd79601d562f3c636066d192fc25fc5`。本包叠在 #978，
接 `codec::load` 的格式、schema、元数据守卫、配置检查和原 `load_records`。

## 实际输入与性质

抽取保留原格式 24、两份完整 DDL、原 SELECT 和列回调、配置的六项范围检查、
会话数范围以及 `Sources` 的所有返回字段。两个 JSON 调用和摘要调用使用逐符号
接口，原有业务判断留在透明生成体。配置解码专门化为原 `Configuration`；
编码薄包装只搬入原 `?` 的错误转换，仍调用真实 Serde。

- `actual_metadata_contract` 在原格式、schema 和查询回复的显式前缀条件下，
  证明元数据行数为 1、预留量恰好 8192、长度在 0 到 8192、
  部署单值容量至少为 8192 时进入原载荷读取。容量查询先于数值守卫。
- `version_error_is_first`、`presence_error_precedes_version_rejection` 和
  `legacy_tables_determine_rejection` 保留原前两次查询及旧格式分支。
  旧格式且两张来源表均不存在时返回 `None`，存在任一来源表则拒绝。
- `schema_reads_complete_text` 比较完整表定义。`checked_decode_error_is_first`
  和 `checked_requires_digest_and_complete_encoding` 保留解码、摘要及全文重新编码
  比较的顺序，所有字节均参与比较。
- `actual_success_retains_full_configuration` 保留 config、node、max_sessions
  和原记录集合。`withOriginalRows` 接回 #978 的守卫和 #977 的同连接游标。
  `original_guarded_rows_keep_transaction_keys` 复用原完整工作键登记结论。

V5/A4 继续提供原 SQLite 连接、prepare/bind/step、真实列值、两张表的实际 DDL、
容量读取及同一事务的聚合和游标关系。新文本适配只用于上述新建范围和原单 TEXT
参数的 schema 查询。UTF-8 转换、标准 Serde 解析与渲染、SHA256 运算的正确性按
原逐符号合同提供。定理显式使用两次库回复及原字节，`Configuration` 的具体
派生字段、原 JSON 到值及重新编码字段接线仍是 V3 的后续义务。
摘要的 32 字节比较本身不证明材料真实性。SQLite 引擎执行、
物理耐久、异步收件、端点认证和完整系统性质继续由所属模块交付。

## 形式复验

`bounds_project` 与 `bounds_audit` 是 #978 原版的精确项目和完整审计。
两版各有独立生成体和 Lake build，使用固定 Charon/Aeneas 和同一 Lean 服务。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
for variant in original allow-inflated-reserve; do
    extracted="$metadata_evidence/$variant"
    project="$metadata_evidence/$variant-project"
    audit="$metadata_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/source_metadata_capture.py" \
        --variant "$variant" --output "$extracted" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare_source_metadata.py" \
        --variant "$variant" --actual "$extracted" --prior "$bounds_project" --audit "$bounds_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    count=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/source-metadata/registration-candidate.json")
    jq -e --argjson count "$count" '.complete and .physical_files == 55
        and (.audited | length) == $count and (.warnings | length) == 0
        and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
jq -se '.[0].fragments as $correct | .[1].fragments as $wrong |
    ($correct["-- ACTUAL LOAD"] != $wrong["-- ACTUAL LOAD"]) and
    (($correct | del(.["-- ACTUAL LOAD"])) == ($wrong | del(.["-- ACTUAL LOAD"])))' \
    "$metadata_evidence/original-project/plan.json" "$metadata_evidence/allow-inflated-reserve-project/plan.json"
```

## ning 原生复验

两个干净的固定生产工作树分别使用独立 Cargo target，先编译测试程序，再执行同一
断言。错误版仅把 `reserve != sql(METADATA_BYTES)?` 换为 `reserve < sql(METADATA_BYTES)?`。
原测试建立合法来源记录，只把原 SQLite 元数据预留从 8192 改为 8193，逐字节确认
配置与摘要未变。原版应拒绝，回滚后读回完整来源。错误版应在同一拒绝断言失败。
格式、编译失败或超时均须保留，不能算性质反例。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh - "$proof_checkout/$package" "$variant" <<'PY'
from pathlib import Path
import sys

package, variant = Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(package))
from source_metadata_capture import allow_inflated_reserve

source = Path('controller-service/src/controller/lifecycle_inbox/source_work')
if variant == 'allow-inflated-reserve':
    codec = source / 'codec.rs'
    codec.write_text(allow_inflated_reserve(codec.read_text()))
else:
    assert variant == 'original'
with (source / 'tests.rs').open('ab') as handle:
    handle.write(b'\n' + (package / 'source-metadata/native.rs.in').read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
test_name=source_loader_rejects_inflated_metadata_reservation
if test "$variant" = original; then
    "${native[@]}" "$test_name" | tee "$native_positive_log"
    rg -F 'test result: ok. 1 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" "$test_name" > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'metadata reservation must be exactly 8192' "$native_negative_log"
    rg -F 'Ok(Some(' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
fi
```

普通完整入口、原生正反、Stage1 与 Controller 公共登记仍交最终采用批次统一验收。
[登记候选](registration-candidate.json) 和 [覆盖候选](public-registration.patch) 由主控采用。
