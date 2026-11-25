# 来源保存的实际记录收集器与完整工作键

[证明工作流](../../../docs/proof-workflow.md) · [实际有序 SQLite 游标](../source-save-load-cursor/README.md)

叠在 #1044，固定生产输入 `60aad8af23e317f5b1ba913518603d1fda015f4c`。
复用 #975 `c0e7fe698ce1ade6e752bcac8af73e3d9b7a55fc` 实际收集器和完整键关系，
本切片把原 `decode_loaded_record`、`read_loaded_rows` 和生成循环接到同一原游标、
同一连接的记录守卫及保存入口，使用完整当前生成的来源记录类型。

- `original_decode_enforces_full_stored_key` 要求任意成功解码记录对应完整持久键，
  不以键长度代替内容相等。`accepted_decode_preserves_actual_checked_value` 保留
  原三列的已查值、编码及摘要回复，且不声称 SHA 或 JSON 本身已验证。
- `actual_loop_refines_ordered_first_error` 对任意有限游标回复和任意初始缓冲，证明
  实际循环等于有序读取规格，保留首个迭代、解码、追加失败；正常空记录不追加。
  `actual_cursor_collector_keeps_original_row_values` 给出成功列表的逐项原行、完整
  原三列和提供者回复关系。
- `original_load_returns_exact_collected_records` 连接之前的同连接守卫，保留整个
  实际收集结果；`original_save_preserves_actual_first_collector_error` 将同一个首错
  传播到实际保存入口。
- 用同一个 `FullStoredKeyContract` 审查正常见证及正反两种实际 Rust 抽取；
  正本保持完整键合同，`length-only-key` 真实错版违反同一合同。

固定联合抽取保留 #1044 的全部 155 个类型及 286 个函数，#975 的四个收集器相关生成
声明逐字相同。正反仅实际解码器函数体不同，且错误版不再使用标准切片/数组比较字典，
因此只允许这一标准字典声明消失。标准 Vec、Option、比较、迭代器及行转换复用前序具名
合同；`Ports.checked`、`Ports.anchor`、`Ports.digest` 是三个未闭合的实际提供者，
保留原调用类型和完整值，不把 JSON、序列化或哈希实现声称为已证。物理 SQLite 游标及
同一事务耐久性仍由 V5 提供和运行验收，不推出已授权或真实来源材料存在。

## 证明机复验

复用 #1044 完整形式结果和正确捕获、#984/#978/#977 原捕获选择及 #975 的原捕获、
原检查项目和完整诊断。检查器校验历史服务请求哈希逐一对应实际物理输入。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-collector
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$collector_captures"
for mode in correct length-only-key; do
    bash scripts/python-entry.sh "$entry/collector_capture.py" --mode "$mode" \
        --output "$collector_captures/$mode" --reference "$load_errors_captures/original" \
        --bounds-reference "$record_bounds_original_capture" \
        --cursor-reference "$sql_cursor_original_capture" --collector-reference "$collector_original_capture" \
        --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/collector_check.py" --captures "$collector_captures" \
    --parent-output "$cursor_formal_output" --parent-capture "$cursor_captures/correct" \
    --reference "$load_errors_captures/original" --bounds-reference "$record_bounds_original_capture" \
    --cursor-reference "$sql_cursor_original_capture" --collector-reference "$collector_original_capture" \
    --collector-project "$collector_original_project" --collector-audit "$collector_original_audit" \
    --tools "$fixed_aeneas" --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

使用精确候选的独立验收 payload，原版、正确联合版和 `length-only-key` 错版均需编译，
执行同一真实 SQLite 夹具中的正常保存及持久键完整性断言。两正确版应返回原记录或
指定拒绝，错误版应实际执行到合同拒绝；编译失败或超时不计。真实原生收据待 Ning。

```bash
entry=verification/experiments/source-save-load-collector
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/collector_native.py" --output "$native_output" --work "$allocated_payload"
```

本切片不改生产、公共登记或覆盖根文件；两个候选登记文件交主控协调。普通检查、三份
原生回执和公共完整验收仍待 Ning 及主控。
