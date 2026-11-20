# 来源预留计算与原读取守卫

[原行布局](../source-row-layout/README.md) · [配置守卫](../source-config-guards/README.md) ·
[原记录守卫](../source-record-bounds/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

本包接续 #987，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
直接抽取原 `SourceObservationConfig::required_bytes`；没有生产重构、抽取适配、
新的不透明函数或外部库合同。类型和三个原预留常量与 #978 原调用者逐字节核对。

## 完整数值合同

对任意六字段原配置及任意 u64 记录数，定义自然数：

```text
per_record = max_work_bytes + 4 * max_configuration_bytes
             + 8192 + 131072 + 69632 + 4096
total = records * per_record + 8192
```

69632 是原退休计划的 `4*16384+4096`，4096 是原材料释放记录；没有删除未来责任的额度。
其余四项配置限制未被原函数读取，不能擅自把它们加入这段容量算法。

`Contract` 使用固定 Aeneas 的 `UScalar.tryMk` 表达完整结果：数学总量可表示时返回
精确 u64，否则为原整数溢出失败。该失败不是 `JournalError::MaintenanceCapacity`，
也不表示已发生的事务或物理责任被撤销。零记录仍保留尾部 8192。

实际原函数的所有 u32 字段在内部加乘阶段均可表示；单记录调用对任意类型值都不会溢出。
原完整六项配置守卫接受后，单记录预留位于闭区间 222212 到 5464064。
原 `load_records` 直接使用这一实际结果，不能继续把完整预留回复当作未落实的业务假设。
原计数、SQL 聚合、首拒绝和后续扫描继续复用 #978 的实际生成体及合同。

错误 Rust 仅将原 `+ super::MATERIAL_RELEASE_RECORD_MAX_BYTES` 改为 `+ 0`。
两版使用同一个完整 `Contract` 和同一正常配置；一记录错误版少计 4096。
乘法溢出及乘法仍可表示、末尾元数据加法溢出的参数域分别核对。这里的大记录数是
原函数的参数边界，不称为合法部署的实际来源行数，也不宣称建立了这么大的数据库。

## 实际输入与复用

两版各自从原生产归档编译并经固定 Charon/Aeneas 生成，函数与三个常量均保留。
原版与 #978 原调用者的生成函数及全部六字段相同；错误版只有上述原表达式不同。
物化器重新运行 #987 与 #983 的原准入，要求完整父计划逐字节相同，
合并共同 52 文件的表示时拒绝任意摘要分歧。服务诊断覆盖整个新闭包，各有独立 Lake build。

完整 initialize 的 source_bytes u64 加乘、原维护策略/会话行数、DDL/INSERT、同一物理事务、
sql_usage 与整库预算仍待 V5 连接。本包没有证明 SQL 引擎、Serde/SHA、认证、实际材料/VM
或完整系统。#985/#986 待采纳的四项 Row 投影及 Range/借用绑定保持原状态，未因数值定理自动采用。

## 形式复验

沿各父包 README 取得其原始完整项目、审计与捕获。`layout_*` 是 #987 原版，
`usage_*` 是 #986 原版，`charge_*` 是 #985 原版，`config_*` 是 #983 原版，
`metadata_*` 是 #981 原版。物化器实际检查这些原件，不用空目录或新的摘要替代。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-reservation
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original omit-release-budget; do
    actual="$reservation_evidence/$variant"
    project="$reservation_evidence/$variant-project"
    audit="$reservation_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/extract.py" --variant "$variant" --output "$actual" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$actual" \
        --layout-prior "$layout_project" --layout-audit "$layout_audit" \
        --usage-actual "$usage_capture" --usage-prior "$usage_project" --usage-audit "$usage_audit" \
        --charge-prior "$charge_project" --charge-audit "$charge_audit" \
        --config-actual "$config_capture" --config-prior "$config_project" --config-audit "$config_audit" \
        --metadata-prior "$metadata_project" --metadata-audit "$metadata_audit" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    audits=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/registration-candidate.json")
    jq -e --argjson audits "$audits" '.complete and .physical_files == 69
        and (.audited | length) == $audits and (.errors | length) == 0
        and (.warnings | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
```

共享重锁、固定工具、独立输出和缓存按原工作流保留。失败、未选中函数、缺失常量或
未完成服务诊断均拒绝复用；不能只跑旧 V3 根并将本包记为通过。

## ning 原生与普通验收

脚本从同一个原归档准备两个独立源副本，仅追加原方法的三项测试；错误版另作同一处
真实 Rust 变异。不安装任何验证用适配。源码准备和格式检查不等于原生已经执行。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-reservation
for variant in original omit-release-budget; do
    case_root="$reservation_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_reservation_ --no-run
done
CARGO_TARGET_DIR="$reservation_native/original/target" cargo test --locked --offline \
    --manifest-path "$reservation_native/original/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_reservation_ -- --nocapture > "$reservation_native/original.log" 2>&1
set +e
CARGO_TARGET_DIR="$reservation_native/omit-release-budget/target" cargo test --locked --offline \
    --manifest-path "$reservation_native/omit-release-budget/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_reservation_ -- --nocapture > "$reservation_native/omit-release-budget.log" 2>&1
native_case_status=$?
set -e
printf '%s\n' "$native_case_status" > "$reservation_native/omit-release-budget.exit"
test "$native_case_status" -eq 101
```

ning 持原 native-heavy 锁，沿原固定 Rust/Python/离线 Cargo；保留原调试断言和溢出检查。
正确版应 3 项通过；错误版须实际编译后 3 项失败，分别命中：

- `source reservation must include every original allowance`
- `source reservation multiplication overflow must not return a wrapped value`
- `source reservation final metadata addition overflow must not return a wrapped value`

测试用 u128 计算完整公式。两个溢出例在原函数执行时分别捕获原乘法和原末尾加法 panic；
错误版少计后返回可表示的值，须由上述原断言拒绝。编译、超时、环境失败或退出 101 本身
不算性质反例。

完整普通入口仍显式设置原已校验的 `SANDPROOF_REDIS_SERVER`，执行无参数
`bash scripts/check-function.sh`。公共登记、同批 proof_native_suite 和完整 V3/Controller
验收由主控统一调度及内容复用。仅提供候选 diff；正式原生、完整普通和公共整套结果仍由 ning/主控交付。
