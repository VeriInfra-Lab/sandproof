# 维护读取接入来源计数

[来源计数](../source-record-count/README.md) · [完整读取规格](../../inbox-load/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

接续 #989，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。生产源码、公共配置、
根导入、冻结与覆盖只读。本包把实际 checkpoint::config、config::read、read_using、
StandardConfigRead 及完整 Maintenance/Capacity 校验接回同一来源计数与 load_records。

## 已解码值、原类型和顺序

复用 inbox-load 的完整 FullConfigReadContract，不另写配置算法。原维护配置源码与历史冻结
字节相同，但仍对当前归档实际编译、抽取并重新实例化证明。保留完整类型、字段、字典、
两次原 SQL、解码、校验、回调状态、错误、失败与非返回。两个透明根各仅调用一次原方法。
完整维护校验、容量数学、读取首错及正常合法见证的原证明体保留；只重接必要导入。
泛型循环和成功 bind 引理按原声明复用，未使用的容量读取推论不纳入本片。

成功读取推出同一完整配置实际 validate 成功及原独立数学模型接受，再推出 operation
范围 3..128。标准读取的同一回复实际供给原减两格、乘会话数和 #988 字节预留；计数和
loader 不再额外假设这个范围。首条格式查询错误经实际读取直接传给 loader，后续聚合
仍不执行。来源配置/会话接受域沿原合同，不把 helper 的任意类型参数说成已接受服务器配置。

原 Std 适配器的 SQLite 操作复用既有逐符号表示，完整共享类型和六个生成字典与父闭包
逐字节对应。Option 和完整 Formatter/Display/Error 数据复用既有 A6 模型，不增加公理。
原 from_slice 与实际 MaintenanceConfig 派生 Deserialize 仍以完整捕获类型的任意回调保留；
本片对这些回调的全部结果成立，不假定它们正确，也不证明 JSON 字节装配或实际 Serde 实例。
捕获模板公理不导入证明。字段正确、数值可表示不推出认证、持久接受或资源存在。

## 同一合同的错误版

skip-maintenance-validation 仅删除解码后的 config.validate()?；其余函数、全体类型及字典
与正确捕获一致。正常合法配置仍通过。operation=1 的完整配置应为 UnsupportedStore，
错误生成体却返回 Some，原计数随后下溢。仅将 retention 改为零时，同一个完整合同也要求
拒绝；错误生成体接受，原计数继续正常返回。两例使用同一 FullConfigReadContract，
不只检查必要的 operation 范围。正常见证复用原完整配置，不以有限样例代替一般定理。

两版使用相同透明根和仅影响生成名称的属性，消除 SQL/JSON Error 的名称冲突；不修改
原转换业务体。准入核对完整归档、三份隔离输入、原 LLBC、工具日志、完整生成清单、
真实调用及父闭包。模块不靠仅改摘要迁记旧证据，也不改工具、关闭检查或手改生成体。

## 证明机复验

count_project/audit 是 #989 正确版完整原件。count_inputs 是私有 JSON，恰有五个原物化
参数键 actual、prior、audit、bounds_actual、reservation_inputs；位置可以不同，内容须相同。
本包重新执行原准入，要求 #989 完整父计划逐字节相同。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-policy-read
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original skip-maintenance-validation; do
    bash scripts/python-entry.sh "$package/extract.py" --variant "$variant" \
        --output "$policy_evidence/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    project="$policy_evidence/$variant-project"
    audit="$policy_evidence/$variant-audit"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$policy_evidence/$variant" \
        --reference "$policy_evidence/original" --prior "$count_project" --audit "$count_audit" \
        --count-inputs "$count_inputs" --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
    case "$variant" in original) files=84; audits=357 ;; *) files=85; audits=354 ;; esac
    jq -e --argjson files "$files" --argjson audits "$audits" '.complete and (.public_acceptance == false)
        and .physical_files == $files and (.audited | length) == $audits
        and (.errors | length) == 0 and (.warnings | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
done
```

两个视图复用历史证明命名空间，公共登记须沿独立项目执行，不能与另一同名旧生成根直接拼装导入。
独立 Lake build、原共享重锁、失败与缓存保留。公共完整入口及 native 登记只交候选，
由主控统一采用；本开发入口不能签发公共整套或完整 V3 验收。

## ning 原生与普通验收

native.py 从原归档准备两个副本，只加入同一夹具及指定错误。没有抽取转发根、生成名
属性或其它改写。三个测试直接调用原维护读取和来源计数，用原 v3::open、真实 SQLite
及真实 serde_json；不创建巨大来源池，不运行 VM。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-policy-read
for variant in original skip-maintenance-validation; do
    case_root="$policy_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_policy_read_ --no-run
done
CARGO_TARGET_DIR="$policy_native/original/target" cargo test --locked --offline \
    --manifest-path "$policy_native/original/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_policy_read_ -- --nocapture > "$policy_native/original.log" 2>&1
set +e
CARGO_TARGET_DIR="$policy_native/skip-maintenance-validation/target" cargo test --locked --offline \
    --manifest-path "$policy_native/skip-maintenance-validation/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_policy_read_ -- --nocapture > "$policy_native/negative.log" 2>&1
native_case_status=$?
set -e
printf '%s\n' "$native_case_status" > "$policy_native/negative.exit"
test "$native_case_status" -eq 101
```

正确版应 3 通过。错误版应 1 通过、2 失败，实际命中
`original policy read must reject the complete operation bound` 与
`original policy read must retain every maintenance validation`。测试先用完整原配置正常读/validate，
再在真实事务中更新完整 JSON，覆盖 0/1/2/129/u64::MAX、零 retention 和缺失 capacity，
原版须返回 UnsupportedStore；回滚后读回完整原配置不变。编译、环境失败或退出 101 本身
不算性质拒绝。源码准备/格式通过不等于测试已执行。

ning 持原 native-heavy 锁、用原固定 Rust/Python/离线 Cargo，保留调试和溢出检查。
普通完整入口显式设置原已校验 SANDPROOF_REDIS_SERVER，执行无参数 bash scripts/check-function.sh。
共享 proof_native_suite、公共登记和完整 Controller/V3 由主控同批统一验收及内容复用。
#985/#986 薄绑定仍待采纳；真实 JSON 实例、SQL 同事务/快照/耐久性、来源初始化写入及整库预算、
SHA/认证、实际材料/VM、重启和系统组合继续按具名提供者交付。
