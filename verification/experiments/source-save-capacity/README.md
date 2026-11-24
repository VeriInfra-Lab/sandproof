# 原来源保存使用实际预留行数

[证明工作流](../../../docs/proof-workflow.md) · [实际完整编码与原保存](../source-save-encoder/README.md)

生产源码固定为 `60aad8af23e317f5b1ba913518603d1fda015f4c`，叠在 #1035。
复用 #989 精确提交 `405b2e346669af18a00f1aeeadea7f54081e6306` 的
`records_per_session`、`reserved_records`、原完整计数合同与正常政策样例。
原 SQL、保留、编码和完整键返回继续采用 #1035 的同一生成类型和连接。

- `save_preserves_policy_read_error` 与 `save_rejects_missing_policy` 保留原维护读取
  错误，维护配置缺失返回 `CorruptStore`。这些拒绝发生在编码和写入前。
- `actual_rejects_excess_before_encoding` 在原头部已接受及维护策略有效时，以
  `sessions × (max_operations_per_sandbox − 2)` 比较原记录行数，超限先返回容量错误。
- `save_consumes_exact_policy_capacity` 把同一维护读取、连接和会话值计算出的容量接到
  原完整编码及保留写入。通过容量门后的保留与逐行失败继续由 #1035 和 #1034 给出。
- 正反两版共享 `RejectsExcessBeforeEncoding`。正常策略设为三格，一个会话有一行额度。
  对包含两条重复记录的同一候选，正确版先返回容量错误；减一版把额度算成两行，继而
  进入实际编码并返回重复键错误。变异沿用 #989，只减少一个应保留的位置。

本证明从当前维护读取和已解码保存记录开始。维护读取的真实 SQLite 数据和完整
配置校验由原 V5 读取合同连接；本切片使用 `PolicyRead` 的精确返回与字段范围。
完整键集合、JSON/SHA 字节、事务回滚、耐久及提交未知继续沿 #1035 的具名条件落实。
公共注册、覆盖及统一验收由主控协调，候选随本包提供。

## 证明机复验

使用 #1035 的正确版抽取、其完整正式检查结果和 #989 的 `original` 固定捕获。
缺失时从相应固定对象按各自 README 复现。保留 #1035 README 所列 Git 对象及
#989 的精确 Git 对象。检查器核对 #989 的两份类型和两份计数函数，确认与新保存视图
逐字相同；仅允许错误版改原减法。新写入的启动入口重建两份 Lean 闭包，并逐文件诊断。

```bash
set -euo pipefail
entry=verification/experiments/source-save-capacity
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$capacity_captures"
for mode in correct keep-one-position; do
    bash scripts/python-entry.sh "$entry/capacity_capture.py" --mode "$mode" \
        --output "$capacity_captures/$mode" --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/capacity_check.py" --captures "$capacity_captures" \
    --encoder-capture "$encoder_correct_capture" --encoder-output "$encoder_formal_output" \
    --count-reference "$original_count_capture" --tools "$fixed_aeneas" --lock "$shared_proof_lock" \
    --output "$formal_output"
```

## Ning 原生复验

为本 PR 的精确提交，用现有 `ci_retention allocate --kind acceptance` 分配空 payload。
原版、正确联合版及错误版在各自 Cargo target 编译并运行同一个保存断言。夹具使用
真实 SQLite 已读取的维护策略构造超限一行的重复记录。正确两版须返回容量错误，
减一版须成功编译、执行并命中同一断言，实际返回重复键错误。独立证据在回收根之外，
原执行者确认资源收尾后用现有 `finish` 清理临时构建。普通和共享验收同批统一调度。

```bash
entry=verification/experiments/source-save-capacity
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/capacity_native.py" --output "$native_output" --work "$allocated_payload"
```
