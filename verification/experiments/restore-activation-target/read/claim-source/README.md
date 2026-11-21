# G2 的原 Unload 身份条件

[ClaimSourceIdentity](ClaimSourceIdentity.lean.in) 在原查询的
`SandproofRestoreSourceQueryRust.controller.lifecycle_inbox.source_work.SourceWorkRecord`
上提供 `selected_query_row_has_original_capture`。它保留 `PermitReadSelection`
的原行、`matchesRestoreIntent` 和选择条件。以下是证明连接的字段。

| 字段 | 生产与生成视图 |
| --- | --- |
| 已保存原 claim | `row.restore = some claim` |
| 本次 Resume 全文 | `claim.intent = intent` |
| 原 Unload | `SourceWork.request` 返回 `row.work.anchor.request`。其 `request.operation = .Unload source capture` |
| 同一 session | `row.work.anchor.request.request.binding.session = intent.request.binding.session` |

`original_unload_guard_exact` 对原样 Rust `matches!` 表达式给出等价条件。
`validated_claim_binds_original_unload` 由第 75–99 行身份前缀成功和全文相等推出
原 `session`、`source_incarnation`、`capture`。实际守卫原样放进薄函数，
其余身份检查保持原顺序。`selected_query_row_has_original_capture` 显式要求该行
自身保存的 claim 通过身份前缀，所得结论与 G2 的 `originalUnloadCapture` 展开相同。
若要用于 G2 的全行条件，须给出同一查询快照中每个命中行的身份前缀成功。
[生产身份检查](../../../../../controller-service/src/controller/lifecycle_inbox/source_work/restore.rs) ·
[预提交全行入口](../../../../../controller-service/src/controller/lifecycle_inbox/source_work/codec.rs)

## 已验收输入和使用方法

[固定清单](inputs.json) 保留生产源、B 的普通检查候选、A 查询来源、联合生成体、
已验模板及原生和 Lean 收据身份。正确原例来自原 SQLite 已准入恢复夹具。对同一行
独立更换原 session、源代数、捕获编号或 Unload 类型，原函数与前缀均拒绝。
[对照夹具](validate-claim-source-test.rs.in)

在新的私有 A 查询工作树运行 [物化入口](../stage_claim_source.py)。按
[读侧复现说明](../../README.md#可独立复现的原许可读侧)使用固定工具、现有重任务锁、
缓存及同一 Lean 服务。所有工作目录均独立。

```bash
stager="$B_REPO/verification/experiments/restore-activation-target/read/stage_claim_source.py"
python3 "$stager" rust "$RUST_VIEW"
(
  cd "$RUST_VIEW"
  "$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
    --include sandproof_protocol --include sandbox_controller_core --include sandproof_controller_service \
    --opaque rusqlite::error::Error --opaque serde_json::error::Error \
    --opaque rusqlite::types::from_sql::FromSqlError \
    --start-from crate::controller::lifecycle_inbox::source_work::codec::proof_query_source_records \
    --start-from crate::controller::lifecycle_inbox::source_work::restore::proof_validate_claim_source \
    --dest-file "$OUT/validate.llbc" -- --manifest-path controller-service/Cargo.toml \
    --lib --locked --offline --target x86_64-unknown-linux-gnu
)
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files -emit-json \
  -namespace SandproofRestoreSourceQueryRust -subdir SandproofRestoreSourceQuery \
  -dest "$OUT/generated" "$OUT/validate.llbc"
python3 "$stager" lean "$LEAN_VIEW" "$A_QUERY_GENERATED" "$OUT/generated"
(cd "$LEAN_VIEW" && lake build ClaimSourceIdentity)
```

`RUST_VIEW` 和 `LEAN_VIEW` 都以清单固定的 A 查询提交为起点。
`A_QUERY_GENERATED` 使用原 A 查询的已封口生成目录。`OUT/generated` 也可直接使用
已保留且通过清单核对的联合抽取。物化后导入 `ClaimSourceIdentity`。
联合视图共用原 A 的命名空间和已有声明。`SandboxId` 只移动了声明位置，
新增 `InternalError` 供 `checked_next` 使用。查询声明保持原样，四个新外部函数复用原
Option、Result 和 String 值的比较及投影。原生对照若需复现，
可另在固定生产基线私有目录执行 `python3 "$stager" rust "$NATIVE_VIEW" --native`，再精确执行
`controller::lifecycle_inbox::source_work::tests::validate_claim_source::original_validator_and_extracted_cut_bind_the_original_unload`。

原全函数已通过 Charon。Aeneas 停在材料关闭支路的 Serde 参数与原模式守卫的
借用合并。整根成功到身份前缀成功的机器连接仍开放。许可读入口在选中后才调用
`validate_record`，此前的全行事实由提交与读取的同一快照关系另行提供。
