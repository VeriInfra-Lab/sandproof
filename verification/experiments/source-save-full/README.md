# 来源正常保存：实际读入、同连接容量/编码、保留及有序写入的完整结果组合

[证明工作流](../../../docs/proof-workflow.md) · [精确读取首错](../source-save-load-decision/README.md) · [实际 Serde 读端](../source-serde-ports/README.md) · [原完整编码及读回](../source-save-load-roundtrip/README.md)

本切片固定生产 `60aad8af23e317f5b1ba913518603d1fda015f4c`，叠在 #1050，复用 #1047
已核对的原完整编码正反；不复制生产算法或重新定义字段合同，不修改生产代码。

- `original_encoder_independent_of_read_and_count` 与 `same_keys_after_actual_read_and_count`
  证明原编码不依赖读回/预留数量两字段，原 Empty/Insert/Contains 完整字典不变。
- `actual_header_from_original_loaded_state` 将原 `load` 完整具体结果、原配置及节点
  相等检查和两个实际会话数检查接到同一原保存的 `HeaderAccepted`。
- `original_save_uses_complete_current_value_policy_and_encoded_rows` 将真实维护策略读端
  与原数量计算、原编码的任意完整记录键/锚点/JSON bytes/行预算和原保留责任连接到
  同一个实际 `save` 返回值。保存正常分支为真实原保留检查和按原顺序、同一 `Db` 的
  逐行原 SQL 写入；超过容量仍为原 `MaintenanceCapacity`。
- `original_save_from_actual_precise_read_to_full_write_result` 从 #1048 原完整可空槽、键、
  摘要、规范字节、完整记录的实际读取，以及 #1050 的原具体 typed Serde 读入口出发，
  在原有效头、配置、容量、查询/游标操作与具名原库返回条件下将上述完整结果接到
  同一原 `save`。正常写入结果由 `full_expected_write_result_is_success` 接上，若每个
  原 SQL 操作均成功且原保留/容量成立，则完整 Rust 返回值为成功。
- `original_and_wrong_share_full_row_evidence_contract` 保留 #1047 的相同任意完整行证据合同；
  原编码全称成立，先前实际零化完整行键的独立错误版违反同一合同。保留原正常证据
  及实际错误版反证，不以有限样例代替全称。不重复捕获同一生产、同一实际生成体；
  完整检验原实际正反捕获及当前生成体逐声明对应，重建并审查两个旧根与本次根的并集。

## 证明机复验

```bash
set -euo pipefail
entry=verification/experiments/source-save-full
SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
  "$entry/full_check.py" --parent-output "$serde_formal_output" \
  --roundtrip-output "$roundtrip_formal_output" --encoder-captures "$encoder_captures" \
  --current-capture "$canonical_captures/correct" --collector-capture "$collector_captures/correct" \
  --reference "$load_errors_captures/original" --bounds-reference "$record_bounds_original_capture" \
  --cursor-reference "$sql_cursor_original_capture" --collector-reference "$collector_original_capture" \
  --tools "$fixed_aeneas" --lock "$shared_proof_lock" --output "$formal_output"
```

本批复用两个完整正式产物，先核对原规则输入、实际旧生成捕获和逐文件依赖与复现计划。
完整新根使用固定 Lean 与已登记同服务诊断，不导入外部模板、不以占位证明替代。

## Ning 原生复验与剩余边界

本次没有新生产实现或新的实际错误版。复用 #1047 的原实际 SQLite 保存/读回夹具（正常
原版、正确联合版、真实错版同一完整行键断言）和 #1050 的实际 Serde 字段/bytes 夹具。
两个包先前已准备原生源码视图，本次不声称原生通过；只有在 Ning 实际编译、运行、核对
精确输入及结果后才能复用其收据。各包 README 给出原完整验收入口，不重复登记新套件。

相同形式 `Db` 参数及原操作成功关系不自动蕴含物理 SQLite 快照、并发隔离或持久提交。
V5 要提供实际同事务行/聚合/连接/提交关系；具体内层 Serde `writtenObject` 状态/字段、
原写端库精化（固定工具限制见 #1050）、真实 SHA、授权、实际资源和完整系统仍是具名待证。
本批不将这些条件提升为事实，不改共享覆盖/登记/依赖根；候选改动交主控统一采用。
