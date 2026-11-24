# 来源保存的元数据容量和成功载荷

[证明工作流](../../../docs/proof-workflow.md) · [首次读取](../source-save-load/README.md)

本切片叠在 #1038，固定生产输入 `60aad8af23e317f5b1ba913518603d1fda015f4c`。
它把实际 `capacity::config` 和 `codec::sql` 接到原 `codec::load`。
容量读取沿同一 SQLite 连接调用已证明的维护策略读取，原错误直接返回，成功容量来自
通过校验的策略。预留值由实际 SQL 整数转换确定为 8192。

- `original_load_uses_exact_metadata_reserve` 给出原元数据门的精确条件和后续载荷。
- `actual_success_retains_full_configuration` 保留完整配置、节点、会话数和来源记录。
  `save_header_consumes_loaded_sources` 将同一返回值接到真实保存头部。
- 精确 8192 字节的样本到达后续 payload 查询。8193 字节样本的原版先返回
  `CorruptStore`，原历史超额预留错版继续到后续查询，违反同一 `MetadataContract`。
- 真实 JSON、摘要与实际记录扫描继续使用具名接口。实际文件与 V5 物理事务仍由对应层完成。

## 证明机复验

使用 #1038 的完整形式输出及正确抽取。#984 的精确原版捕获只用于合并可复用的选择器。
复用固定 Charon/Aeneas 和同一已注册 Lean 客户端。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load-payload
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$payload_captures"
for mode in correct allow-inflated-reserve; do
    bash scripts/python-entry.sh "$entry/payload_capture.py" --mode "$mode" \
        --output "$payload_captures/$mode" --reference "$load_errors_captures/original" \
        --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/payload_check.py" --captures "$payload_captures" \
    --parent-output "$save_load_formal_output" --parent-capture "$save_load_captures/correct" \
    --reference "$load_errors_captures/original" --tools "$fixed_aeneas" \
    --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

为精确候选分配空的既有验收 payload。三版各自在独立目标目录编译并运行同一断言。
夹具先在真实 SQLite 事务里保存健康候选，然后把唯一元数据行的预留值从 8192 改为
8193。正确两版必须返回原错误，允许超额错版必须编译并执行到同一断言，报告
`Ok(())`。结束后按已有工作流回传结果及关闭临时 payload。

```bash
entry=verification/experiments/source-save-load-payload
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/payload_native.py" --output "$native_output" --work "$allocated_payload"
```
