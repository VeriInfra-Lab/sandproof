# 完整来源列计费与表名分派

[原行预留](../source-row-charge/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

本包接续 #985，固定生产 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
实际根是 `capacity::row_bytes`，另保留 `capacity::charged_inbox_bytes`。
原 `add`、internal/lifecycle/source 三段计费及真实 Row 实例均保持透明。
生产 Rust、公共 Lake/Cargo、根导入和共享冻结未改；公共登记只提供候选。

## 完整值、首错与正常见证

在下述机械接线和 A4/A6 库条件下，`complete_count` 对任意原 Row、请求列数和容量，
证明原循环对应同一个 `Contract`：
按原 `0..columns` 顺序读取指定列，Null 计 0，Integer/Real 计 8，Text/Blob 计完整
字节长度。TEXT 的 UTF-8 字节、BLOB 的零字节都不扣所谓终止符。
请求列数不自动等于 Statement 列数：零列不读取 Row，请求过多列按原顺序遇到索引错误。

- 原索引错误先于该列大小；单列大小超过 `max_sql_value_bytes` 时先拒绝，
  然后才做 checked 总和。读取、超限或加法失败后不读取后续列。
- `loop_accounts_for_all_columns` 对任意读取实例保持原返回及非返回行为；
  `complete_column` 用实际透明 Row 实例消解完整值条件，没有假设整个计费业务正确。
- `account_success_is_full_sum` 和 `complete_count_is_full_sum` 将正常返回连接到
  所请求各列的数学总和。后者明确要求请求的列在原 Statement 范围内。
- `dispatch_exact` 使用完整表名相等，精确覆盖 internal/lifecycle 两张表、
  source metadata/records 两张表及其它名字；其它名字保留传入值，不读取 Row。
  原 internal 计费与 #985 的 lifecycle 基础计费定义相同，由 `internal_same` 核对。
- `exact_source_after_original_count` 从实际列计费取得原五列空行的 `current=52`，
  再接回 #985 的原来源计费得到 222292。该行保留完整 `null` BLOB、真实 32 字节
  摘要及原最小配置的 222212 预留，不再凭空提供 current。

错误版仅将原 Text/Blob 共用的 `value.len() as u64` 改为
`value.len().saturating_sub(1) as u64`。空值仍计 0；同一个合法空行的两个 BLOB
各少一字节，实际列计费为 50，最终来源计费为 222290。
`truncated_count_breaks_same_contract` 用这份真实生成体否定相同 `Contract`。
编译、抽取和导入失败都不计为性质反例。

## 抽取连接与待审查边界

固定工具直接处理原循环时依次遇到 ValueRef 借用合并、循环嵌套借用问题。
隔离副本只将原五类大小匹配、指定列读取提成透明小函数，再将原循环抽成泛型读取函数。
实际 Row 实例仍调用同一个原 `get_ref(index)`；匹配、错误转换、上限判断和 checked 加法
没有设为 opaque。物化器重新构造完整源副本并核对所有原函数/回调的映射。

保留原 for-range 时，全部透明函数已翻译，固定后端随后因 Std Step 的
`forward_overflowing` 字段不匹配而失败。最终在隔离源中把原 for 显式展开为同一个
Range 的逐次 next；唯一不透明的新薄函数 `source_row_next` 的完整体是 `indices.next()`。
其有限值模型直接使用固定 Aeneas 原 `IteratorRange.next StepUsize`，没有补写 Step、
修改工具/生成体或实现另一套业务循环。

原 for 的 IntoIterator 身份转换、显式 next 连接和借用保持仍须 B4/A4/V5 审查。
[依赖候选](dependencies-candidate.patch) 明确尚未采纳；不能把局部 Lean 成功当作新绑定
已获批准或 Std 源码已机器精化。全文字符串比较复用原 A1/A6 合同；Row、Statement、
ValueRef、CapacityConfig 和错误使用原表示。#985 的四个最小库符号仍保持其待采用状态。

本片证明从同一个已表示的原 Row 开始。原 `sql_usage` 的查询、表名和列清单来源、
特殊表调整、同连接/事务及整库预算仍由 V5 交付。`initialize`、完整 JSON/摘要实例、
实际材料/VM、授权和系统组合继续开放；32 字节摘要值不表示本包计算过哈希或证明材料真实。

## 形式复验

`row_charge_project`、`row_charge_audit` 是 #985 原版的 55 文件、172 项完整服务审计。
物化器验证完整原归档、原容量全部字段、原生成清单及透明函数，并要求三段复用函数和
六个闭包函数逐字节等于原已审计视图。外部生成模板从不导入。两版各用独立 `.lake/build`。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-row-usage
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original drop-terminator-byte; do
    captured="$row_usage_evidence/$variant"
    project="$row_usage_evidence/$variant-project"
    audit="$row_usage_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/extract.py" \
        --variant "$variant" --output "$captured" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare.py" \
        --actual "$captured" --prior "$row_charge_project" --audit "$row_charge_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    jq -e '.complete and .physical_files == 58 and (.audited | length) == 197
        and (.errors | length) == 0 and (.warnings | length) == 0
        and (.rejected | length) == 0' "$audit/result.json"
done
```

原有界捕获入口保持四 CPU、独立 target 4 GiB、Charon 240 秒、Aeneas 120 秒及共享重锁。
所有输出使用新目录，失败、诊断及缓存保留。上述是独立开发复验；公共
`verify-stage1-protocol.sh`、原生凭据和统一完整验收由主控协调，不以旧根结果代替本包。

## ning 原生与普通验收

ning 持原 native-heavy 锁，使用原固定 Rust/Python/离线 Cargo 和 Redis。
`native.py` 创建两份原精确生产归档，只向 capacity.rs 追加相同的三个原生断言；
错误版另做上述唯一长度变异。它不安装抽取 helper、泛型改写或 Range 薄绑定，
也不执行测试或写入成功回执。两份源码使用不同 target，先分别实际编译，再运行相同过滤。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-row-usage
for variant in original drop-terminator-byte; do
    case_root="$row_usage_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_row_usage_ --no-run
done
# 分别保留原始输出；错误版还须检查下表三个实际断言，退出 101 本身不够。
CARGO_TARGET_DIR="$row_usage_native/original/target" cargo test --locked --offline \
    --manifest-path "$row_usage_native/original/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_row_usage_ -- --nocapture \
    > "$row_usage_native/original.log" 2>&1
set +e
CARGO_TARGET_DIR="$row_usage_native/drop-terminator-byte/target" cargo test --locked --offline \
    --manifest-path "$row_usage_native/drop-terminator-byte/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_row_usage_ -- --nocapture \
    > "$row_usage_native/drop-terminator-byte.log" 2>&1
native_case_status=$?
set -e
printf '%s\n' "$native_case_status" > "$row_usage_native/drop-terminator-byte.exit"
test "$native_case_status" -eq 101
```

正确版应为 3 项通过；错误版应实际编译后在同样三项断言失败：

| 原生用例 | 同一断言与真实 SQLite 输入 |
| --- | --- |
| `source_row_usage_counts_full_original_columns` | `original row byte count must include every BLOB byte`；五列空行必须计 52，另覆盖零到五列前缀、Null/Integer/Real/Unicode Text/二进制 Blob 与空值 |
| `source_row_usage_dispatches_original_source_and_other_tables` | `source charge must use the actual complete row bytes`；原空行必须预留 222292，元数据、两种 envelope、完整 source 表名及近似/大小写/后缀名字分别检查 |
| `source_row_usage_preserves_per_value_limit_before_later_index_error` | `complete per-value bytes must reject before the later missing column`；请求六列、上限 31，原 32 字节摘要须先拒绝；错误版才会到达缺失第六列，另覆盖 Unicode 完整字节上限 |

原生、超时、编译和环境失败分别登记，不把未到断言的退出计为反例。
正式普通入口仍显式使用已校验 `SANDPROOF_REDIS_SERVER`，执行无参数
`bash scripts/check-function.sh`。同批 `proof_native_suite` 和完整公共证明由主控统一调度。
[公共登记候选](registration-candidate.json)、[覆盖候选](public-registration.patch) 与
依赖候选随包交付，尚未自动采用；本片不声明完整普通、正式原生或公共整套验收通过。
