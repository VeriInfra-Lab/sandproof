# 来源保存的完整编码输入

[证明工作流](../../../docs/proof-workflow.md) · [原保存顺序和逐行写入](../source-save-retention-rows/README.md)

本切片固定生产提交 `60aad8af23e317f5b1ba913518603d1fda015f4c`，沿用 #1034
的完整旧责任检查和 #969 的同连接逐行写入。把 #974 的实际键编码循环抽取到同一
个 Rust 保存父调用中。复用 #974 的两个单态 JSON 转发，两个 `?` 直接使用原
`From<serde_json::Error>` 的构造器及载荷，以避免组合抽取时与 SQLite 的
转换同名。正常版及错误版只在隔离的 Git 归档上暂存。

- `actual_encoder_keeps_complete_input` 对任意成功的原编码循环证明完整行序、
  每条记录实际返回的编码字节、完整 32 字节键以及同一返回集合的成员关系。
- `save_returns_first_row_encoding_failure` 把任意已编码前缀后的首个重复键、
  锚点 JSON 错误、摘要不符、记录 JSON 错误或行超限传到原保存返回值。失败
  之后不再检查后续记录，也未进入旧责任检查或逐行 writer。
- `actual_save_with_complete_encoded_inputs` 将编码器返回的集合与每个有序行
  直接传入 #1034 的原保存定理，复用原首拒绝和同连接写入规格。
- 两版共用 `CompleteEncoding`。错误版只把送往 SQL 的行键改为全零，保留插入
  集合的完整键与 JSON。原抽取入口在正常见证上返回同样的记录字节，错误版返回
  全零的行键并违反原合同。

`KeySetSpec` 的提供者是 V0 对 `BTreeSet<[u8; 32]>` 的构造、成功插入和查询
合同。向集合写入的键与传给 writer 的行键通过原循环证明相等。`EncoderPorts`
保留 JSON 锚点/记录编码的实际字节结果、摘要调用结果、当前 SQL 读取结果及
预留行数作为显式输入。JSON 规范字节由 V3 编码提供者连接，原读取和值恢复由 V3
收件存储连接，SHA 采用既有独立合同。持续存储、回滚和未知提交由 V5 连接。
正常 Lean 见证的 JSON 和摘要由有限测试提供者给出。Ning 原生见证核对实际
Rust 读取的 SQLite 行、真实 JSON 和完整键。

## 证明机复验

使用 #1034 的三份原始提取及其固定 #956 SQL 参考。全新克隆需获取 #1034 README
列出的固定 Git 对象与 #974 提交 `36f551f005e29b960cf1ac0a2638e21c47f4fa2a`。
检查器验证原父闭包及新声明，核对正反只改变原编码循环的行键，复验两份真实
Lean 文件闭包与声明审计。

```bash
set -euo pipefail
entry=verification/experiments/source-save-encoder
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$encoder_captures"
for mode in correct zero-row-key; do
    bash scripts/python-entry.sh "$entry/encoder_capture.py" --mode "$mode" \
        --output "$encoder_captures/$mode" --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/encoder_check.py" --captures "$encoder_captures" \
    --parent-captures "$parent_captures" --row-reference "$original_row_capture" \
    --output "$formal_output" --tools "$fixed_aeneas" --lake-packages "$fixed_lake_packages" \
    --lean-mcp "$fixed_lean_mcp" --lock "$shared_proof_lock"
```

## Ning 原生复验

为本 PR 的精确提交用现有 `ci_retention allocate --kind acceptance` 分配空 payload。
运行普通验收与下列局部用例。原版和联合版须通过；错误版须编译、执行并准确
命中同一个完整行键断言。原始证据保存在回收根之外，原执行者确认实际资源
收尾后用现有 `finish` 回收临时构建。

```bash
entry=verification/experiments/source-save-encoder
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/encoder_native.py" --output "$native_output" --work "$allocated_payload"
```

公共登记由主控同批协调，原生结果绑定精确候选提交。
