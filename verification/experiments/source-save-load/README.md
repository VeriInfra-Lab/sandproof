# 来源保存沿用实际首次元数据读取

[证明工作流](../../../docs/proof-workflow.md) · [来源保存沿用实际维护策略读取](../source-save-policy/README.md)

生产输入固定在 `60aad8af23e317f5b1ba913518603d1fda015f4c`，接在 #1037。
把 #984 的元数据查询与首错合同接到原 `codec::save` 的第一步。
实际 `JournalSourceSave.read` 将保存收到的同一个连接直接传给 `codec::load`。
联合根沿用当前生成的配置、节点、错误、来源记录、连接和集合接口。

- `same_read_connection` 和 `save_returns_original_first_read_error` 把原加载错误
  传播到保存，早于保存头部比较、容量检查、编码、责任保留检查和写入。
- 查询顺序覆盖格式版本、表是否存在、两张表的原始建表文本、行数、元数据长度及
  预留数和容量读取。其首错合同保留各分支原错误，并保持旧版本对有表和无表的区别。
- `PreservesNumbersFailure` 同时检查真实保存的正本和提前检查行数的真实错版。
  同一缺行样本上，正本返回 `Sqlite(QueryReturnedNoRows)`，错版返回 `CorruptStore`。
  两个结果都由实际抽出的保存和读取声明推出。

本切片实例化 #984 已审计的查询回调和相同 SQLite 方法。容量配置查询、SQL 整数转换、
JSON 解码、重编码、摘要计算及深层记录读取沿各自的具名接口。首次读取的成功载荷与
记录扫描连接、真实字节及文件事实和 SQLite 持久提交继续沿各自提供者完成。

## 证明机复验

输入包括 #1037 完整形式结果与正确捕获，以及精确 #984
`c887260527b5071fd78a68f3f8d84698fd0c9bd2` 的双捕获、原版项目和完整诊断。
统一复用之前的 Charon、Aeneas、Lean、已注册的同服务检查器和共享重锁。当前正反
闭包保留 #1037 的所有生成声明，核对 #984 的加载相关声明，只允许实际错版
`codec::load` 的函数体改变。

```bash
set -euo pipefail
entry=verification/experiments/source-save-load
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$save_load_captures"
for mode in correct early-count-guard; do
    bash scripts/python-entry.sh "$entry/load_capture.py" --mode "$mode" \
        --output "$save_load_captures/$mode" --reference "$load_errors_captures/original" \
        --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/load_check.py" --captures "$save_load_captures" \
    --parent-output "$save_policy_formal_output" --parent-capture "$save_policy_captures/correct" \
    --reference-captures "$load_errors_captures" --reference-project "$load_errors_original_project" \
    --reference-audit "$load_errors_original_audit" --tools "$fixed_aeneas" \
    --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

为本 PR 的精确提交分配既有 `ci_retention allocate --kind acceptance` payload。
原版、正确联合版和提前检查行数版均在独立 Cargo target 编译，运行同一真实 SQLite
断言。夹具先在一笔事务里验证健康保存成功，随后删除唯一元数据行，再对原候选调用
保存。两正确版保留 `QueryReturnedNoRows`。错误版必须编译并执行到同一断言，
报告 `Err(CorruptStore)`。完成后按既有工作流回传证据并为 payload 执行 `finish`。

```bash
entry=verification/experiments/source-save-load
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/load_native.py" --output "$native_output" --work "$allocated_payload"
```
