# 来源行数与原读取决策

[预留计算](../source-reservation/README.md) · [原记录守卫](../source-record-bounds/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

接续 #988，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
保留实际 records_per_session、reserved_records 和原 load_records 生成体，连接原维护
配置回复、两格保留、完整会话数及 #988 的实际字节预留。生产、公共配置、根导入和冻结只读。

## 数值与首错

同一个 Contract 对任意原 PolicyRead、连接和 u16 会话参数规定：

1. 调用原配置读取，保留错误、非返回和已发生的效果，不要求总返回成功。
2. 缺失配置返回原 CorruptStore；成功配置先受检减去两格。
3. 减法下溢先失败，即使会话数为零也不能提前返回。
4. 再乘完整会话数；成功连接自然数总量，溢出保留原整数失败。

维护配置的 3 到 128 条件由 V5 的实际读取/验证连接提供，不信任整个读取服务。
在该条件和原会话域内计算正常返回，再接原 load_records 的计数与容量决策。
真实 SQL 聚合、DDL、物理事务、初始化写入及完整 sql_usage/整库预算仍另行交付。
零会话是 helper 的类型域；正式 max_sessions 校验仍为 1 到 4096，不扩大服务器接受域。
无效维护配置的数值边界也不冒充已通过原 validate。值计算不证明认证、JSON/SHA、材料/VM 或系统结论。

## 抽取与错误版

三版分别从原生产归档编译和固定 Charon/Aeneas 抽取，复用 #978 的生成视图、扫描边界和准入。
全部类型、政策/错误字段、字典及非目标函数须与 #978 原件一致。
keep-one-position 仅将原减 2 换为减 1；zero-skips-policy 仅在原 reserved_records 首部加零参数返回。
两版使用同一 Contract。编译、抽取或导入失败不算反例。
原读取继续沿用 PolicyRead 连接，不新增业务不透明函数、外部合同或不同的协议模型。

## 形式复验

reservation_project/audit 是 #988 原版完整原件，bounds_capture 是 #978 原版完整捕获。
reservation_inputs 是私有 JSON，列出 #988 物化器所需十三个原输入路径，键名沿原参数：
actual、layout_prior/audit、usage_actual/prior/audit、charge_prior/audit、config_actual/prior/audit、
metadata_prior/audit。路径不是成功凭据；本包重跑原准入，要求完整父计划逐字节相同，记录位置清单摘要。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-record-count
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original keep-one-position zero-skips-policy; do
    actual="$count_evidence/$variant"
    project="$count_evidence/$variant-project"
    audit="$count_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/extract.py" --variant "$variant" --output "$actual" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$actual" \
        --prior "$reservation_project" --audit "$reservation_audit" \
        --bounds-actual "$bounds_capture" --reservation-inputs "$reservation_inputs" \
        --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    audits=$(jq -r --arg variant "$variant" '.formal_views[] | select(.variant == $variant) | .full_audits' \
        "$package/registration-candidate.json")
    jq -e --argjson audits "$audits" '.complete and .physical_files == 72
        and (.audited | length) == $audits and (.errors | length) == 0
        and (.warnings | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
```

三套独立 Lake build、原共享重锁和失败/缓存保留。公共完整入口须登记本包真实目标，不能只跑旧根。

## ning 原生与普通验收

native.py 准备三个原生产副本，不安装抽取用的扫描/保存改写。三个测试直接调用原私有计数函数，
使用原 v3::open、维护读取和真实 SQLite。不初始化巨大来源池，不运行 VM；Host 仍为原组件夹具。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-record-count
for variant in original keep-one-position zero-skips-policy; do
    case_root="$count_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_record_count_ --no-run
done
CARGO_TARGET_DIR="$count_native/original/target" cargo test --locked --offline \
    --manifest-path "$count_native/original/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_record_count_ -- --nocapture > "$count_native/original.log" 2>&1
for variant in keep-one-position zero-skips-policy; do
    set +e
    CARGO_TARGET_DIR="$count_native/$variant/target" cargo test --locked --offline \
        --manifest-path "$count_native/$variant/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_record_count_ -- --nocapture > "$count_native/$variant.log" 2>&1
    native_case_status=$?
    set -e
    printf '%s\n' "$native_case_status" > "$count_native/$variant.exit"
    test "$native_case_status" -eq 101
done
```

正确版应 3 项通过。keep-one-position 应 1 通过/2 失败，命中
`source count must retain both original lifecycle positions` 及
`source count must multiply the complete session value after reserving two positions`。
zero-skips-policy 应 2 通过/1 失败，命中 `zero sessions must preserve the original policy query error`。
第三项在真实事务内删除唯一维护配置行，核对 QueryReturnedNoRows，回滚后读回完整原配置。
编译、环境失败或退出 101 本身不算性质拒绝。源码准备和格式通过不等于原生已执行。

ning 持原 native-heavy 锁，使用原固定 Rust/Python/离线 Cargo，保留溢出与调试检查。
普通完整入口显式设置原已校验 SANDPROOF_REDIS_SERVER 后执行无参数 bash scripts/check-function.sh。
公共 proof_native_suite、登记和完整 V3/Controller 由主控统一调度及复用，只提供候选 diff。
#985/#986 的原投影/Range/借用条件保持待采纳；局部通过不表示正式原生、完整普通、公共整套或完整 V3 通过。
