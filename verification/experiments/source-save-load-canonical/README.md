# 来源保存的实际规范字节验证与摘要转发

[证明工作流](../../../docs/proof-workflow.md) · [实际记录收集器](../source-save-load-collector/README.md)

叠在 #1045，固定生产输入 `60aad8af23e317f5b1ba913518603d1fda015f4c`。
实际原 `checked<T>` 在原 `Option<SourceWorkRecord>` 调用处特化，只将真实
Serde 解码、Serde 编码和 SHA 计算命名为具名叶子，各叶子保留原参数、类型和返回；
原 `map_err`、`?`、短路、完整比较及原值返回全部透明。实际 `encode_load_anchor`
复用已抽取的原 `encode_anchor_json` 调用，原 Json 错误转换透明。实际 `source_work::digest`
仍调用唯一生产 SHA 叶子及原 `ContentDigest::from_sha256` 完整值构造器。

- `actual_checked_refines_first_error` 对任意叶子回复给出实际完整检查等式：
  先解析，解析错误映射为 CorruptStore，再完整摘要比较；仅摘要匹配才序列化，
  保留原 Json 错误，最后完整比较所有编码字节。接受保留原完整可选记录值。
- `original_full_canonical_contract` 证明完整原始字节及摘要关系。正常见证接受，
  真实 `omit-canonical-byte-check` 只改变规范比较本身，违反同一性质。
- `actual_anchor_preserves_serialization_reply` 及
  `actual_work_digest_preserves_every_byte` 保留原 anchor 编码和所有摘要字节。
  在明确的两个 `NormalHash` 提供者条件及支持的完整输入域内，连接已存在的 SHA 字节规格；
  不冒充密码库实现证明、不以摘要相等推断材料真实性或唯一性。
- `actual_decoded_record_preserves_canonical_bytes_and_full_key` 和
  `actual_cursor_records_have_complete_original_byte_evidence` 将前一切片任意记录的
  完整持久键、真实原三列、规范值字节、anchor 字节及摘要回复串联起来。
- `original_load_keeps_canonical_row_evidence` 连接同一原加载入口成功返回的每个记录；
  `original_save_keeps_first_canonical_row_error` 证明原行规范校验的首拒绝原样传播到保存，
  此失败不进入该行键或 anchor 检查，也不继续收集后续行。

联合抽取保留父 #1045 的完整 155 类型、291 函数声明，新增一个实际 closure 类型、六个
实际函数/字典。正确与错误库类型和函数清单相同，只有原特化 `checked_source_payload`
生成函数体不同。检查器还核对原 `source_work.rs` 的真实 SHA 委托、`sha256.rs` 和
`digest.rs` 源位置、两条原调用顺序及生成体。原 Serde 具体类型的字节/值实现、真实 SHA
库的符合性、物理游标与事务耐久性继续保留在各自具名提供者；本包不声称完整 JSON/TLS
业务链、授权、原材料存在或系统完成。

## 证明机复验

复用 #1045 完整形式结果和正确捕获及前序原捕获选择；使用原来固定工具和同一服务，
不修改生成体。完整物理闭包与依赖固定输入、父文件准入、逐文件严格诊断及闭包重现。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-canonical
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$canonical_captures"
for mode in correct omit-canonical-byte-check; do
    bash scripts/python-entry.sh "$entry/canonical_capture.py" --mode "$mode" \
        --output "$canonical_captures/$mode" --reference "$load_errors_captures/original" \
        --bounds-reference "$record_bounds_original_capture" \
        --cursor-reference "$sql_cursor_original_capture" --collector-reference "$collector_original_capture" \
        --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/canonical_check.py" --captures "$canonical_captures" \
    --parent-output "$collector_formal_output" --parent-capture "$collector_captures/correct" \
    --reference "$load_errors_captures/original" --bounds-reference "$record_bounds_original_capture" \
    --cursor-reference "$sql_cursor_original_capture" --collector-reference "$collector_original_capture" \
    --tools "$fixed_aeneas" --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

对精确候选构造独立验收 payload，原版、正确联合版和实际错版编译后均执行同一隔离
SQLite 回归断言。正常版保留完整原记录，拒绝非规范记录编码；真实错版须到达同一合同
拒绝而非编译、导入或超时失败。三份实际 native 收据待 Ning。

```bash
entry=verification/experiments/source-save-load-canonical
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/canonical_native.py" --output "$native_output" --work "$allocated_payload"
```

本包不修改生产、公共配置、根导入、共享登记或覆盖根；候选差异交主控。普通与原生
验收、共享入口采用待 Ning 和主控。形式证明结果不等于公开完整验收。
