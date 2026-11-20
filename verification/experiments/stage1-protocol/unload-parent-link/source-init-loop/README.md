# 来源初始化的完整行循环

[完整单行](../source-init-row/README.md) · [元数据续体](../source-init-metadata/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

接续 #994，固定生产 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。
本包只交付实际隔离候选的有条件循环合同及其父调用连接；公共登记、库薄绑定采用、
普通/原生验收和系统组合分别交付，不能由交互检查或一次构建代替。

## 实际函数与统一合同

复用 #992/#993 原捕获中的 `write_initial_source_rows`、完整实际 loop/body 和
`insert_initial_source_row`，不换生成体或写入算法。`SourceInitRowSpec.expected` 及
完整转换/SQL 参数合同直接复用，不给正常和错误版更换性质。

`CursorTrace` 只描述迭代器边界的返回、错误或非返回及其顺序，不包含行写入或 SQL
成功前提。`finite_trace_contract` 对任意有限迭代器记录连接实际循环和原完整单行
合同的顺序组合，保留任一行的错误/失败/非返回。迭代器 None 不插入，迭代器错误先于
插入，原行失败不进入下一轮；行成功只继续同一个返回的 cursor。

任意实际 u64 行数的 `1..=rows` 不靠有限样例展开。复用固定 Aeneas 现有
`RangeInclusive.next_U64_spec` 和 `StepU64`，以剩余项数归纳取得有限 trace，产出的
完整编号值恰为 `List.range' 1 rows.val`，包括 rows=0 的空区间。
`Contract` 在逐 cursor 的原 `next` 与此既有库模型对应条件下，连接完整行序列和
实际循环的返回。该机械采用继续由 V0/主控审查，不能从模型引理自动推出真实 Rust
迭代器已被机器精化，也不另造一份不同的范围模型。
零是 helper 类型域，不扩大正式服务器的合法配置；原合法计数域仍由 #989/#990 提供。
最后编号只产出一次；原模型在最大 u64 终点也不要求再执行超界加一。

全体所需行插入成功时原循环终止成功；其它插入返回仍按原顺序处理。单行转换的合法
配置域和原计数/预留结果沿 #993；本包不把 Ok 当成物理插入、事务提交或耐久事实。
一般等价、实际 SQL、同事务/快照/耐久及完整初始化/整库预算仍分别有提供方。

元数据 INSERT 成功后，任意影响行数均进入同一完整序列，包含原实现接受的零。
完整公开父调用连接原读取、容量、节点克隆、编码、两个 DDL、两个摘要及写入成功
条件，得到全部原行完成；这些条件逐项保留提供方，不把整个初始化或 handler 设为可信。
每行仍使用原完整 id/payload/digest/reserve、精确 SQL 和 NULL key。

## 正反视图

正确捕获继续复用 #993 原件。`skip-last-row` 错误 Rust 在原每行写入前，对最后一个
编号提前结束；原版和隔离副本分别只在原 for/match 处加入同一个遗漏。固定抽取驱动
保留原选择器、库端口、版本、四核预算、重锁、失败日志和独立 target。

错误版已实际编译并经固定 Charon/Aeneas 抽取；全部生成类型和函数名保持，只有原行
循环的 body、loop 和入口三个声明改变。物化器逐字加入这三个完整实际声明，不手改生成体。
正确捕获的原三声明亦逐字核对 #994 完整父视图，原全部文件和审计保持。

两版共用 `SourceInitLoop.Contract`、原完整行合同和合法单行输入。正常控制均成功；
行 SQL 失败的端口见证下，原版保留失败，错误实际循环跳过最后写入返回成功，否定同一
完整合同。此端口反例不表示实际 SQLite panic；真实行集另由原生完整列断言提供。
编码字节、32 字节摘要值或字段相等不提供真实 Serde、SHA、认证、资源或持久事实。
编译、超时、导入或归约失败不算性质拒绝。

## 证明机复现

`row_capture` 是 #993 原正确捕获。`metadata_project/metadata_audit` 是 #994 的完整正确
原件；`metadata_inputs` 私有 JSON 恰含 actual、reference、prior、audit、row_inputs 五项
原物化参数。后者继续指向原 Row/Capacity 输入闭包，不用新目录伪装旧成功。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-loop
parent=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/extract.py" --variant skip-last-row \
    --output "$loop_evidence/wrong-capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
for variant in original skip-last-row; do
    actual="$row_capture"
    if test "$variant" != original; then actual="$loop_evidence/wrong-capture"; fi
    project="$loop_evidence/$variant-project"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$actual" \
        --reference "$row_capture" --prior "$metadata_project" --audit "$metadata_audit" \
        --metadata-inputs "$metadata_inputs" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$loop_evidence/$variant-audit" --lock "$SANDPROOF_LOCK_FILE"
done
```

核对 registration-candidate.json 的全部物理文件、唯一审计及依赖；服务结果须完整、
无错误/警告、无失败依赖和 stale/partial。各自使用独立 .lake/build，父源码相同后只可
复制缓存到独立目录，不能共写 build、删缓存或换工具。此开发入口不签发公共整套成功。

## ning 原生与普通验收

native.py 只准备原始正确、隔离正确、原始错误、隔离错误四份源码，不执行测试。隔离
原生副本恢复原 lib.rs，另记原适配字节后用固定 rustfmt 规范换行，捕获原件保持。
每份追加同一个完整行集断言及 #994 五条未改控制，合计六条。

新断言沿原 v3::open/真实 SQLite 事务执行 initialize，查询全部预留行及五列。完整
编号必须正好为 1..=rows，每行必须有原 null 字节、真实 SHA256、原预留和 NULL key；
rows 来自原实际维护计数，预留来自原 required_bytes。原正常配置含多行，不能只查
count 或前一行。这个有限实例与一般循环定理各自报告，不混为全部物理事务已证。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-loop
for variant in original adapted-original skip-last-row adapted-skip-last-row; do
    case_root="$loop_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run
done
```

ning 须核对实际新编译 libtest 的 manifest/src/fresh、实际程序和输入摘要，再按登记的
六个完整名字逐项 `--exact`。单用 source_init_loop_ 过滤会漏掉五条控制。正确两版各应
6/6；错误两版各应 5 通过/1 失败，唯一拒绝须到达
`initial source rows must include every reserved id with complete values`。
错误之后另建 original 恢复副本，重新编译并执行六条，应 6/6；失败日志保留。
环境、约束、超时、编译或退出 101 本身均不能替代指定性质拒绝。

最终采用候选显式使用原已校验 Redis，运行无参数 bash scripts/check-function.sh。
公共 proof_native_suite、verify-stage1-protocol/verify-controller、原生/正式模块登记、
根导入和覆盖由主控同批统一验收及内容复用；本包只提供候选，不重复无关全套或新增 KVM。

## 后续提供方

普通与原生正确/错误/恢复验收继续交 ning；公共 proof_native_suite、正式模块/根导入、
冻结、依赖和覆盖由主控同批采用，不在本包并发写。生产 Rust、工具和已发布父候选不改。
dependencies-candidate.patch 仅细化原 u64 next 的待采用接线，不新增已批准的库合同。
V0/主控继续审查 Range/IntoIterator/借用、tuple Params、Row/摘要薄委托及整个生产到隔离
候选的一般等价。V5 提供实际 JSON/摘要实例、SQL 效果/同事务/快照/耐久和整库预算，
V7 提供独立认证/配置来源，V6 提供实际材料/VM/物理事实；不据本片声明完整 V3 或系统已证。
