# 来源初始化的容量前缀

[维护读取](../source-policy-read/README.md) · [行预留](../source-row-layout/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

接续 #990，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。本包重新捕获实际
`source_work::initialize`、`codec::initialize`、原读取、配置校验、行数、字节预算、五道容量门
及完整后段。生产源码、公共配置、总登记和覆盖记录只读。

## 性质与表示

`SourceInitCapacitySpec.Contract` 同时规定完整预算和五道检查，正确版与错误版共用同一文件。
预算为 `8192 + 48 + rows * (reserve + 80)`。泛型定理保留 Rust 的全部算术顺序：
即使 rows 为零，也先计算 reserve+80。内层或总和溢出先于任何容量拒绝；预算成功才保证
后续 rows+1 可表示。五个条件分别是单值上限、原 16 MiB 常量、元数据上限、总行数和总字节。

父调用先检查六项来源配置和 1..4096 会话，再读取原维护配置并计算行数，计算实际
required_bytes(1)，再执行第二次容量读取和预算门。两次读取使用独立参数，没有假设同一
连接就意味着同一快照。第二次完整读取及其状态/首错沿既有 FullConfigReadContract 证明重接；
没有将完整 Serde、配置读取或事务设为可信。

同一生成体的后容量门续体由工具逐字截取，父调用的分解等式由内核检查。配置、会话、计数、
第二次读取或容量门拒绝时，保留原首错，不进入此续体的 clone、JSON 编码、DDL、摘要或 INSERT。
正常时进入原完整续体；这条等式不保证续体成功。所有后段库端口均为任意有类型结果，包含失败
和非返回，没有假设成功编码、成功写入或正确哈希。

复用 #987 的真实四/五列预留结论：元数据预留另计 48，每个工作行的未来 key 也计入 80。
复用 #988 实际 required_bytes、#989 实际减两格及乘会话数、#990 完整校验后的 3..128 范围。
合法来源配置和会话范围导出 rows≤516096、reserve≤5464064、总额≤2820022870064，
并与实际生成的父调用、计数及字节计算连接。此总额只描述来源两表的逻辑预留；完整数据库、
索引、页、WAL、其它表及最终事务容量检查仍是原调用方的义务。

原 LifecycleInboxConfig 的八项字段、NodeIdentity、HostEpochRegistration、32 字节摘要值、
三项 Configuration、维护及容量配置、完整错误和字典均保留。共享类型、原函数及父物理闭包
逐字核对后复用，不另造协议模型。32 字节值或字段相等不表示认证、计算过哈希、耐久接受或资源存在。

## 抽取隔离与未闭合接线

原 for 循环的提前返回、动态 Params、固定 Step 接口和递归 Serde/摘要字典的失败证据保留。
隔离副本将连续预算/检查和单行写入移到透明小函数，保持原调用位置、参数和首错；循环仍由
原 1..=rows、原 next 和原 INSERT 执行，没有替换算法。Params 宏改用真实 rusqlite 三/四元组，
保持参数值和顺序。编码、next、摘要各有一个实际库委托；摘要委托只将同一 Digest 输出转为
32 字节数组。lib.rs 属性只消除生成名称冲突，不改错误转换体。生成体不手改，外部公理模板不导入。

证明对象是从固定归档构造的隔离候选。预算和拒绝前缀保留原表达式/分支；整个生产初始化到
该候选的等价连接仍未封口。三/四元组绑定、原 u64 RangeInclusive::next、借用及薄函数采用，
交 V0/主控按逐符号合同审查，不能由有限原生对照替代一般等价证明。完整 JSON 实例、实际
SQL 快照/事务及耐久连接交 V5；独立认证来源交 V7；材料、VM 和物理事实交 V6。
成功后的编码、逐行写入、完整初始化和系统组合继续开放。

## 同一完整合同的错误版

omit-row-overhead 只把原预算的 reserve+80 改为 reserve+0；全部其它生成函数和类型必须一致。
正常容量仍可接受。合法的完整维护配置仅将 max_sql_bytes 调成正确预算少一字节，仍通过
原完整校验；正确合同拒绝，错误生成体接受。零行且 reserve 为 u64::MAX 时，错误版又遗漏
原内层溢出。零行边界属于 helper 的全输入域，不作为已接受服务器配置。
这些是同一合同的反例，正常样例不替代全量输入定理。

父调用反例还用同一完整维护配置实例化两次原读取。只有编码口返回 panic，后段其它口返回
另一种失败；错误父调用越过预算门并到达编码失败。该端口反例说明控制流，真实 JSON/SQLite
错误版另由 ning 执行下面的原生断言，不能互相冒充。

## 证明机复验

policy_project/audit 是 #990 的完整正确原件。policy_inputs 是私有 JSON，恰含 actual、reference、
prior、audit、count_inputs 五个物化参数；本包重做原准入并核对完整父计划，不只信一份摘要。
工具、缓存、四核预算、重锁和独立 .lake/build 沿原交接，失败与缓存保留。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-capacity
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original omit-row-overhead; do
    bash scripts/python-entry.sh "$package/extract.py" --variant "$variant" \
        --output "$init_evidence/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    project="$init_evidence/$variant-project"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$init_evidence/$variant" \
        --reference "$init_evidence/original" --prior "$policy_project" --audit "$policy_audit" \
        --policy-inputs "$policy_inputs" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$init_evidence/$variant-audit" --lock "$SANDPROOF_LOCK_FILE"
done
```

按 registration-candidate.json 核对两个视图的全部实际文件、审计项、完整非 stale/partial
服务结果和依赖。该登记仍是候选；公共 verify-stage1-protocol、proof_native_suite、总 Lake
及覆盖补丁由主控统一采用，不以此开发入口签发公共整套验收。

## ning 原生与普通验收

native.py 只准备输入，不执行测试。四个副本分别为原始正确/错误和隔离正确/错误，使用相同
三个断言。隔离副本恢复原 lib.rs，不带 Charon 属性。正常测试核对实际 JSON 全字段、原元数据
和每一行的 id/null/摘要/预留/key，并回滚；边界测试检查完整预算恰相等可接受、少一字节必须
在源表 DDL 之前拒绝。原正确及隔离正确各应 3/3；两种错误各应 2 通过、1 个指定性质拒绝。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-capacity
for variant in original adapted-original omit-row-overhead adapted-omit-row-overhead; do
    case_root="$init_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib source_init_capacity_ --no-run
done
```

ning 继续使用原执行身份检查，绑定实际同 crate libtest 的 manifest/src/fresh、编译归档、
执行程序摘要及私有执行副本。四组全部编译后，执行上述同一过滤器的三个测试；只接受错误版
在 source_init_capacity_rejects_one_byte_short_before_any_source_ddl 的原断言失败，文本为
`source initialization must reject one-byte-short complete source reservation`，其它两项必须通过。
编译失败、超时、导入失败或其它断言失败不算反例。再从原归档新建 original 恢复副本实际
编译/执行 3/3，保存完整输入、输出、实际程序和清理记录；不清共享缓存。

最终精确候选普通验收仍用原固定 Redis 和无参数 bash scripts/check-function.sh。
公共 proof_native_suite 与 verify-controller 批次交主控统一运行和复用内容相同的记录。
本片无 KVM 场景，不改变 #641 实机队列；原生、普通、公共完整证明验收及最终生产采用分别记录。
