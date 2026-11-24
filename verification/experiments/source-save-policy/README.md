# 来源保存沿用实际维护策略读取

[证明工作流](../../../docs/proof-workflow.md) · [来源保存的容量计算与首错](../source-save-capacity/README.md)

生产源码固定到 `60aad8af23e317f5b1ba913518603d1fda015f4c`，本批接在 #1036。
复用 #990 的精确提交 `9a88b02b4e1937e1a5e469f7a40a5be84d39a877`，把实际
`checkpoint::config` 的维护读取、解码与完整校验接到同一保存调用的容量分支。
该读取与保存调用使用同一个 SQLite 连接。

- `save_preserves_original_policy_error` 保留标准维护读取返回的错误。
- `save_rejects_invalid_maintenance_before_encoding` 由实际读取分支和原校验结果
  推出保存返回的首错。校验拒绝发生在容量计算、编码和写入之前。
- `save_uses_validated_policy_capacity` 从实际读回的策略推出已校验的操作数范围，
  接入 #1036 的原预留行数、容量门及 #1035 的完整编码写入合同。
- 正确版和跳过校验版共享 `RejectsInvalidMaintenanceBeforeSave`。在共同的零留存
  策略和超限样例上，正确版返回 `UnsupportedStore`，错误版越过政策校验并返回
  `MaintenanceCapacity`。错误版的实际 `checkpoint::config` 和
  `read_using` 来自实际 Rust 变异的生成体。

本次接线从已通过保存头部检查的调用开始。标准 SQL 读取在模型中使用同一连接，
反序列化沿用原 `DeserializeMaintenance` 和 `DecodeSlice` 回调。外部 SQLite
库和真实 JSON 字节行为仍使用各自的具名接口。现有来源记录的其他读取、实体状态、
提交和回滚继续由相应层承接。

## 证明机复验

保留 #1036 的完整审计输出和正确原保存捕获，以及 #990 的精确
`original` 捕获、已检查的 Lean 项目及其完整诊断报告。这些输入可由对应提交的
复验入口重建。检查核对 #990 原声明与当前政策生成体，并核对 #1036 父生成体。
唯一额外的父字典变化是 Array `PartialOrd` 显式包含 `lt` 和 `le`。保存接线对
给定集合操作的字典实现保持不变，见 `PolicyOrderProjection.lean`。库行为仍按
完整集合合同处理。两版的实际生成体仅在政策读取的维护校验处分歧。

```bash
set -euo pipefail
entry=verification/experiments/source-save-policy
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$policy_captures"
for mode in correct skip-maintenance-validation; do
    bash scripts/python-entry.sh "$entry/policy_capture.py" --mode "$mode" \
        --output "$policy_captures/$mode" --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/policy_check.py" --captures "$policy_captures" \
    --capacity-capture "$capacity_correct_capture" --capacity-output "$capacity_formal_output" \
    --policy-project "$original_policy_project" --policy-capture "$original_policy_capture" \
    --policy-audit "$original_policy_audit" --tools "$fixed_aeneas" --lock "$shared_proof_lock" \
    --output "$formal_output"
```

## Ning 原生复验

为本 PR 的精确提交，用既有 `ci_retention allocate --kind acceptance` 分配空 payload。
原版、正确联合版与错误版各自在独立 Cargo target 编译并运行同一个保存断言。
夹具在真实 SQLite 事务中写入零留存策略，并对同一超限候选调用原保存。正确两版
返回 `UnsupportedStore`。错误版实际编译、运行，并在同一断言报告
`Err(MaintenanceCapacity)`。原生视图去除仅供 Charon 使用的命名属性。驱动记录
相应抽取投影供逐字核对。原生断言覆盖真实保存整体入口，包括它自己的现有记录读取。
原执行者确认资源收尾后使用既有 `finish` 处理临时构建，公共验收同批协调。

```bash
entry=verification/experiments/source-save-policy
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/policy_native.py" --output "$native_output" --work "$allocated_payload"
```
