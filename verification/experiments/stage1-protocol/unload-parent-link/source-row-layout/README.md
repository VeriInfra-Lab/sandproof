# 来源行布局与固定预留

[完整列计费](../source-row-usage/README.md) ·
[原行预留](../source-row-charge/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

本包接续 #986，生产仍固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
复用 #985/#986 已实际抽取的原行计费、表名分派和来源预留，不新增或替换 Rust 算法，
也不把复用写成本批重新抽取。生产、公共 Lake/Cargo、根导入和共享冻结未改。

## 三种实际布局

`Header` 绑定同一个原 Row 的前四列完整值：Integer id、全部 payload BLOB、
32 字节 digest BLOB、Integer reserve。`Layout` 只描述原 SQL 行布局：

| 布局 | 实际列数及第五列 | 原始计费 | 来源预留 |
| --- | --- | --- | --- |
| metadata | 四列，不读取第五列 | payload 长度 + 48 | reserve + 48 |
| empty | 五列，work_key 为 Null | payload 长度 + 48 | reserve + 80 |
| occupied | 五列，work_key 为完整 32 字节 Blob | payload 长度 + 80 | reserve + 80 |

这里的布局是原字段观察关系，使用既有 Row/Statement/ValueRef/CapacityConfig，
没有另造协议或 SourceWorkRecord。完整 32 字节 Blob 键是表示关系的前提；原 schema
约束长度，原 load 的类型读取检查 Blob，二者到本布局的来源连接仍由 V5 交付。
它不表示 `charged_bytes` 会检查任意非 Null 值的类型或长度。其它值仍沿 #985 的原合同。
摘要长度和完整字节不表示计算过哈希，也不推出 JSON、授权、已持久接受或真实材料事实。

## 性质、正常域和反例

- `actual_layout_contract` 对任意三类原布局，先用 #986 的完整计费消解 current，
  再接原 i64 预留读取、完整载荷替换及未来键计费。没有假设最终 charge 已成功。
- `actual_accepting_contract` 在正常域证明实际 count 和 charge 均返回成功，且预留精确：
  原 signed 预留能转换到 u64、payload 不长于预留、预留不超过原单值上限、
  单值上限至少 32。i64 的真实值域本身足以给 80 字节开销留出 u64 算术空间。
  此正常域不改写原函数的拒绝行为，首错继续由 #985/#986 提供。
- `record_layouts_have_identical_charge` 对同一配置预留的两个已表示记录，证明
  空槽与已占用键两种布局均保留 reserve+80；双方 count/charge 正常返回也由证明给出。
  它不证明一次实际填入/清空事务、授权退休或已释放额度。
- `preallocated_source_total` 在自然数上连接原初始化式
  `8192 + 48 + rows * (reserve + 80)` 的系数。它不证明原 initialize 的 u64 加乘、
  行数/配置提供方、DDL/INSERT、提交，也不替代完整 sql_usage 或整库预算。
- 原完整 `null`、真实 32 字节摘要和 222212 预留的空行是正常见证：实际 current=52，
  最终预留 222292。错误版复用 #986 的真实 `len().saturating_sub(1)` Rust 生成体，
  在同一合法布局、转换、容量及正常返回下得到 current=50、预留 222290。
  同一个布局合同和同一个正常成功合同均被否定，未为两版更换性质。

原 SQLite 同 Row/Statement/缓冲及快照、实际查询/列顺序/表名来源仍由 A4/B4/V5 提供。
完整源池状态、Serde/SHA 具体实例、事务/预算、实际文件/VM 和系统结论继续开放。
#985 的四个库符号与 #986 Range/借用薄绑定仍保持各自待审查状态，本批没有新增外部合同。

## 形式复验

`usage_project`、`usage_audit` 分别指 #986 对应原版或错误版的完整 58 文件/197 项审计。
`charge_project`、`charge_audit` 是 #985 原版的完整 55 文件/172 项审计。
物化器重跑原 #986 源重建和全部准入检查，核对原完整计划、实际 Rust/LLBC/生成物、
全部类型/调用体及父服务日志。完整原件不符则拒绝复用。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-row-layout
parent=verification/experiments/stage1-protocol/unload-parent-link
# 对 original、drop-terminator-byte 分别设置以上对应的 usage/capture 原件。
bash scripts/python-entry.sh "$package/prepare.py" \
    --actual "$usage_capture" --prior "$usage_project" --audit "$usage_audit" \
    --charge-prior "$charge_project" --charge-audit "$charge_audit" \
    --output "$layout_project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
module=$(jq -r .root "$layout_project/plan.json")
(cd "$layout_project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
bash scripts/python-entry.sh "$parent/diagnose.py" --project "$layout_project" \
    --output "$layout_audit" --lock "$SANDPROOF_LOCK_FILE"
audits=$(jq -r --arg module "$module" '.formal_views[] | select(.formal_root == $module) | .full_audits' \
    "$package/registration-candidate.json")
jq -e --argjson audits "$audits" '.complete and .physical_files == 60
    and (.audited | length) == $audits and (.errors | length) == 0
    and (.warnings | length) == 0 and (.rejected | length) == 0' "$layout_audit/result.json"
```

两套目录各有独立 `.lake/build`，使用新输出目录与原共享重锁；保留失败和缓存。
局部开发复验不代表公共登记或完整 `verify-stage1-protocol.sh` 已完成。

## ning 原生与普通验收

原生夹具从原 `ready/prepare` 建立实际源表，先调用原 `Sources::load` 检查完整源状态，
再在同一次原 query_row 中取得当前 Row、实际 cap 和完整列数，依次调用原 row_bytes
及 charged_inbox_bytes。不构造替代计费函数或占位成功数据库，也不启动产品 VM。
Host 事实仍由原组件夹具提供。两个 cfg(test) 入口仅让容量测试复用原私有准备函数。
本包准备源码不等于已执行这些夹具；原 load 检查和后续断言的实际回执均由 ning 提供。

ning 持原 native-heavy 锁，使用原固定 Rust/Python/离线 Cargo，分别重建两个固定生产副本。
脚本只追加三项断言及测试入口；错误版另作 #986 同一处长度变异，不安装抽取适配。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-row-layout
for variant in original drop-terminator-byte; do
    case_root="$layout_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_row_layout_ --no-run
done
CARGO_TARGET_DIR="$layout_native/original/target" cargo test --locked --offline \
    --manifest-path "$layout_native/original/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_row_layout_ -- --nocapture > "$layout_native/original.log" 2>&1
set +e
CARGO_TARGET_DIR="$layout_native/drop-terminator-byte/target" cargo test --locked --offline \
    --manifest-path "$layout_native/drop-terminator-byte/source/controller-service/Cargo.toml" \
    -p sandproof-controller-service --lib source_row_layout_ -- --nocapture > "$layout_native/drop-terminator-byte.log" 2>&1
native_case_status=$?
set -e
printf '%s\n' "$native_case_status" > "$layout_native/drop-terminator-byte.exit"
test "$native_case_status" -eq 101
```

正确版应 3 项通过；错误版须编译后 3 项失败，分别真实命中
`source metadata must reserve exactly 48 overhead bytes`、
`empty source work must reserve exactly 80 overhead bytes`、
`occupied source work must reserve exactly 80 overhead bytes`。
预期少计分别为 2、2、3 字节；初始化/准备/加载、编译、超时或环境失败另列，不能计为反例。

完整普通入口仍显式设置原已校验 `SANDPROOF_REDIS_SERVER` 后执行无参数
`bash scripts/check-function.sh`。同批共享 proof_native_suite、公共模块/原生登记和完整
V3 验收由主控统一调度及复用。本包提供公共登记/覆盖候选，尚未自动采用；不宣称
正式原生、完整普通、公共整套或完整 V3 已通过。
