# 来源配置 JSON 的具体字段接线候选

[元数据读取证明](../source-metadata/README.md) ·
[既有 Serde 源检查器](../../serde-binding/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包接续 #981，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
对原 `codec::Configuration`、`SourceObservationConfig` 和 `NodeIdentity`
重新捕获真实 Serialize/Deserialize 派生和两个 JSON 包装。三个结构体分别有
3、6、3 个完整声明字段。配置的磁盘上限和材料上限使用不同数值作为正反见证。

## 已检查的原调用与边界

`review.py` 复用原 `serde-binding.Original` 和 `check_stages`。每个字段的
完整类型、声明顺序、JSON 名称、原字段借用、实际字典、同一序列化容器、
前项成功后才执行后项、原首个错误返回和结束调用均需符合原合同。
解码侧检查三个原 `deserialize_struct` 派发及六个 string/bytes
unknown-field 拒绝 visitor，并保留完整派生子树和原 LLBC 的摘要。
具体 map/seq 的全部字段装配仍须继续连接。

错误版只在原 `SourceObservationConfig` 上交换 `max_disk_bytes` 与
`max_material_bytes` 的 Serde 名称，两个 Rust 字段及其类型保留。
它可以自我往返，仍须被同一完整字段合同拒绝。原断言固定要求
`SourceObservationConfig.max_disk_bytes` 取原字段并使用原 JSON 名称。

本包新增 Lean 定理为零。固定 Aeneas 对完整 JSON 根的实际预检退出 2，
原日志包括 `Inconsistent projection` 和参数类型不一致。全部失败和部分产物保留，
部分生成页没有进入 #981 的证明项目。源接线审查和原生有限对照分别支持
A6/B4 使用点审查，具体 Serde 的机器精化及 #981 `Codec` 的完整实例继续待证。
共享登记与覆盖仅提供候选，由主控协调采用。

## 证明机源码准入

使用独立源码和 target、固定工具、同一共享重锁。输出路径必须尚不存在。
`metadata_json_evidence` 是本包独立证据父目录。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal
package=verification/experiments/stage1-protocol/unload-parent-link/source-metadata-json
for variant in original swapped-limit-names; do
    captured="$metadata_json_evidence/$variant"
    bash scripts/python-entry.sh "$package/capture.py" \
        --variant "$variant" --output "$captured" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/admit.py" \
        --capture "$captured" --output "$metadata_json_evidence/$variant-admission.json"
done
jq -e '.source_admission_complete and .original_contract_matches
    and .new_lean_theorems == 0' "$metadata_json_evidence/original-admission.json"
jq -e '.source_admission_complete and (.original_contract_matches | not)
    and .rejection == "original field/container reference changed: SourceObservationConfig.max_disk_bytes"' \
    "$metadata_json_evidence/swapped-limit-names-admission.json"
```

准入逐字节检查原归档、重建的原 Rust/变异、两份工具日志、实际 LLBC 及
选择器/目标/类型检查选项。Aeneas 的退出码单独保存，源合同结果不将翻译失败
记作性质反例。错误版须实际经 Rust/Charon 完成编译和捕获，再到达同一源合同拒绝。

## ning 原生复验

ning 持原 native-heavy 锁，两个固定生产的干净工作树分别采用独立 Cargo target。
把本包的原生夹具追加至
私有源码副本的 `codec.rs`，以便访问原私有 `Configuration` 和 `checked`。
夹具使用真实 Serde、原 3/6/3 字段及 SHA256，核对完整对象、正常值、全部字段缺失、
三个对象的未知字段、重复、整数范围、原节点合法域、文档尾及非规范编码/错误摘要拒绝。
它不启动产品服务或 VM。

```bash
set -euo pipefail
test "$(git rev-parse HEAD)" = b0d791ccc4d4e058b710dbf8f46766652b2853df
test -z "$(git status --porcelain)"
package=verification/experiments/stage1-protocol/unload-parent-link/source-metadata-json
bash scripts/python-entry.sh - "$proof_checkout/$package" "$variant" <<'PY'
from pathlib import Path
import sys

package, variant = Path(sys.argv[1]), sys.argv[2]
sys.path.insert(0, str(package))
from capture import swap_names

if variant == 'swapped-limit-names':
    source = Path('controller-service/src/capture/source.rs')
    source.write_text(swap_names(source.read_text()))
else:
    assert variant == 'original'
codec = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
with codec.open('ab') as stream:
    stream.write(b'\n' + (package / 'native.rs.in').read_bytes())
PY

native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
name=source_metadata_json_preserves_every_field_and_decode_shape
if test "$variant" = original; then
    "${native[@]}" "$name" | tee "$native_positive_log"
    rg -F 'test result: ok. 1 passed;' "$native_positive_log"
else
    code=0
    "${native[@]}" "$name" > "$native_negative_log" 2>&1 || code=$?
    test "$code" = 101
    rg -F 'original Configuration complete JSON fields' "$native_negative_log"
    rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
fi
```

普通完整入口、实际原生正反结果与公共登记待 ning/主控执行。
SQLite 来源、同事务、提交未知、异步收件、认证、实际资源及系统组合沿原模块分工继续交付。
