# 来源保存的完整编码、原写入列及原读回往返

[证明工作流](../../../docs/proof-workflow.md) · [完整编码](../source-save-encoder/README.md) · [实际规范字节验证](../source-save-load-canonical/README.md)

叠在 #1046，固定生产输入 `60aad8af23e317f5b1ba913518603d1fda015f4c`，复用 #1035
`dd82c2cdf2921bfdc04f11d032a2fcddd5771132` 的原正反固定捕获及前序实际原保存、
有序读回和全字节校验闭包。这里不另造编码、SQL 或协议算法。

- `original_encoder_success_keeps_all_anchor_and_budget_evidence` 对任意正常完成的
  原实际编码循环加强此前的 `CompleteEncoding`：每个有序输出具有原全键、实际记录
  序列化回复、原 anchor 序列化及完整摘要相等、实际容量守卫及同一要求预留回复。
- `actual_encoder_full_evidence_contract` 和真实 `zero-row-key` 错版共同接受完整增强合同，
  正常原实际编码见证通过，错误版原实际循环使合同不成立。
- `sharedCodec` 和 `sharedEncoder` 将真实同名 anchor 与唯一 SHA 提供者接到原读写入口。
  检查器核对原 payload 摘要薄包装和原 SHA `digest` 精确调用正文、同输入和类型。
  `original_sha_reference_on_supported_domain` 复用既有标准 SHA 数学及候选库接口，仍不证明
  真实库实现。V3 具体 Serde 提供者的两个具名条件 `sameSome`、`readsWritten` 对实际
  `SourceWorkRecord` 与 `Option<SourceWorkRecord>` 完整字节/值关系保留。
- `original_write_and_read_preserve_same_row` 接通实际一个行写入的同连接、选槽、原 SQL
  参数、SHA 值、单行成功回复，与原 SELECT 三列投影及原记录 decoder。`WrittenProjection`
  是 V5 需提供的**原物理列投影及顺序**合同，不假定提交成功或凭相等推出资源真实。
- `original_encoder_and_reader_roundtrip_complete_records` 从任意实际原编码成功输出，
  经过同一具名 Serde/SHA、对应原物理有序列和实际收集循环，返回完整原记录向量，
  无丢项、重排或弱键匹配。`original_load_roundtrips_full_sources` 把此关系接入原同连接
  `load`，在具体原配置、节点、会话等元数据回复相等和原守卫正常时返回完整 `Sources`。

## 保留的原输入与复验

原正反 #1035 捕获和当前 #1046 原本，真实 encoder 相关三个声明逐字相同。其全部 131
个数据类型、198 个函数/字典中仅原标准 `Array.PartialOrd` 字典新增 `lt`/`le` 两个方法地址；
检查器精确限制这一差异，不更换 `BTreeSet` 合同。历史正反清单一致，仅原输出键循环
函数体不同；在当前完整类型及相同规范下重用原反例，重查实际全部文件及审计。

本切片不需要重复旧成功 Charon/Aeneas 捕获。证明机从原 #1035 及 #1046 捕获与完整
检查结果复验；若缺原捕获，按两个父 README 实际重捕获而非补 hash。检查期间复用原
同一服务、固定工具、共享重锁及物理闭包独立 `.lake/build`。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-roundtrip
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
bash scripts/python-entry.sh "$entry/roundtrip_check.py" --encoder-captures "$encoder_captures" \
    --parent-output "$canonical_formal_output" --parent-capture "$canonical_captures/correct" \
    --collector-capture "$collector_captures/correct" --reference "$load_errors_captures/original" \
    --bounds-reference "$record_bounds_original_capture" --cursor-reference "$sql_cursor_original_capture" \
    --collector-reference "$collector_original_capture" --tools "$fixed_aeneas" \
    --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

复用精确 #1035 原、正确、真实零键错版的 Rust 暂存选择，新隔离 SQLite 回归同时核对
完整键、anchor/摘要、实际 Serde Some 字节和值、原行预留以及同连接 `save`/`load`
成功回读完整 `Sources`。同一原生断言的正确版实际通过、错误版实际编译且运行到完整键
拒绝才算回归，编译失败或超时不算。准备输入不等于实际原生验收。

```bash
entry=verification/experiments/source-save-load-roundtrip
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/roundtrip_native.py" --output "$native_output" --work "$allocated_payload"
```

不修改生产、公共登记或覆盖根，提供候选登记交主控。真实物理事务、未知提交/恢复、
具体 Serde/SHA 实现符合性、授权和材料存在仍由各自模块负责；原生、普通和共享完整
验收由 Ning/主控完成。
