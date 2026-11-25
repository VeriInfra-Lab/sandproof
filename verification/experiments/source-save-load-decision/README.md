# 来源保存完整可空槽判定及实际读回首错

[证明工作流](../../../docs/proof-workflow.md) · [完整记录收集](../source-save-load-collector/README.md) · [原读写往返](../source-save-load-roundtrip/README.md)

叠在 #1047，固定生产 `60aad8af23e317f5b1ba913518603d1fda015f4c`。
复用 #1045 `cd6f6b3c8365e1dd05fe350443f0ff848e33c557` 的真实正反收集器和 #1046
实际规范 JSON/摘要转发，以及 #1047 同一 SHA/Serde 接口。无需再次抽取相同 Rust；
完整旧正反生成体和当前实际输入由检查器逐一核对。

- `actual_decode_refines_complete_nullable_decision` 对任意已查载荷的回复和任意可空键，
  证明实际 `decode_loaded_record` 的完整分支等式。错误先返回；空记录仅允许空键且不追加。
  有记录须有完整匹配键，然后才执行原 anchor 编码；原编码错误保持，完整摘要不匹配拒绝，
  成功返回原完整值。真实 key-length 错版违反同一个完整 `CompleteDecisionContract`。
- `original_decode_has_complete_byte_and_nullable_order` 接上实际规范 JSON 解析、摘要、
  序列化和完整字节比较，实际 anchor 及 SHA 委托不再作为黑盒业务决定。仅现有具名
  Serde、SHA 库原语保留其逐符号责任。正常空槽、非空槽及原实际反例都用同一判定。
- `actual_reader_has_complete_nullable_first_error` 对任意有限原始游标，将真实三列投影和
  真实收集循环接到上述完整决策；合法空槽只跳过当前记录并保留后续原顺序，错误空槽
  原样停止当前循环，不遍历后续记录。
- `original_load_refines_complete_precise_read` 对已通过实际头、元数据与记录容量守卫的
  任意原游标，证整个实际同连接 load 等于完整读回规格。这是 Result 等式，包含具名原语
  的返回错误及其他效果，不只列正常样例。`original_save_preserves_first_complete_precise_error`
  将任何这段普通首拒绝原样传回原保存入口，后段 encoder/retention/writer 不执行。

本包保留当前 #1047 完整往返结果，复用 #1045 的完整 155 类型/291 函数声明（当前
156/297 全部保留）及实际错误版，仅原 `decode_loaded_record` 函数体不同；错误版省掉
原未使用的标准切片数组比较字典。无生成体改动、无生产改动。V3 具体记录 Serde 字节/值
关系和真实 SHA 库符合性、V5 实际物理列投影、同一事务及 durable commit，以及业务
资源事实、认证授权、真实 JSON/TLS 完整系统仍显式待各自提供者。

## 证明机复验

使用 #1047 与 #1045 的完整真实检查结果、当前正确联合捕获、原 #1045 正反捕获及前序
的原固定捕获选择。缺捕获时按父 README 重捕获，不迁记结果。固定义务和同服务检查
重新验证所有真实文件及原反例，闭包独立复现。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-decision
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
bash scripts/python-entry.sh "$entry/decision_check.py" --parent-output "$roundtrip_formal_output" \
    --current-capture "$canonical_captures/correct" --collector-output "$collector_formal_output" \
    --collector-captures "$collector_captures" --cursor-capture "$cursor_captures/correct" \
    --reference "$load_errors_captures/original" --bounds-reference "$record_bounds_original_capture" \
    --cursor-reference "$sql_cursor_original_capture" --collector-reference "$collector_original_capture" \
    --tools "$fixed_aeneas" --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

复用 #1045 原、正确、真实键长度错版暂存；新隔离 SQLite 夹具同时覆盖原正常空槽、
非法键/空记录组合、非法非空记录/空键组合及完整键正常和错版。原版和正确版须实际通过，
真实错误版需编译后到达同一完整键拒绝，不能以编译/超时失败代替。三源预检与实际
原生执行、普通验收分开；真正验收由 Ning。

```bash
entry=verification/experiments/source-save-load-decision
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/decision_native.py" --output "$native_output" --work "$allocated_payload"
```

公共登记和覆盖候选交主控，未修改公共文件，不声称完整公开验收或物理系统完成。
