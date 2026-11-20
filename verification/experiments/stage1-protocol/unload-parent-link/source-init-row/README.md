# 来源初始化的完整单行 INSERT

[容量父调用](../source-init-capacity/README.md) · [维护读取](../source-policy-read/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

接续 #992，生产仍固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。本包新增的是已捕获后段中
单行 INSERT 的完整值/调用合同，不重复此前的配置、计数、字节计算及容量门证明。
生产 Rust、公共 Lake/Cargo、冻结、依赖和覆盖记录只读；公共接线仅交候选补丁。

## 性质与表示关系

`SourceInitRowSpec.Contract` 对任意实际 Ports、连接、u64 id、两个完整字节 slice 和 u64 预留，
规定真实 `insert_initial_source_row`：先按已有精确 i64 上限转换 id，再转换 reserve；任何转换
拒绝都不调用 execute。随后调用原四元组 Params、同一连接及精确 SQL：

```sql
INSERT INTO controller_source_work_records VALUES(?1,?2,?3,?4,NULL)
```

四个完整参数依次为受检 id、原 payload、原 digest、受检 reserve，第五列为 SQL NULL。
一般性对应覆盖任意完整字节值，正常有限样例仅作可实现见证。四元组字典另证明完整值进入
同一 typed bind4；其实际库效果及 Params 宏到元组适配仍沿原逐符号采用义务。

SQL Err 包装为原 Sqlite 错误；失败、可观察效果及非返回由同一 Result 绑定保持。原 API
忽略受影响行数，因此 Ok(0) 也返回 Ok(())。不能从 helper 成功推导已插入一行、持久提交、
已授权、资源存在或计算过摘要。此函数本身也没有重新检查 id 非零或 digest 长度，不能添加
这样的接受条件；零值边界只属于 helper 类型域，不当作正常 SQL 行。

复用 #989 的完整计数/预留边界与 #990 的真实维护读取成功证明，在已接受会话和六字段
来源配置域内，对每个 id≤rows 导出两项原 SQL 转换成功。后者直接从实际完整读取成功
取得策略范围，没有将完整 reader/Serde 作为可信业务函数，也不重复实现计数算法。

## 抽取和反例

两版从相同归档重新实际编译，经固定 Charon/Aeneas 抽取。全部生成类型、函数名及非目标
函数体必须相同。新原版所有生成声明还与 #992 原件逐字相同，故复用同一实际函数和生成类型。
物化器重做 #992 的完整输入准入、服务日志及父计划重建；不是只相信一份旧摘要。

隔离仍沿 #992：透明单行 helper、真实三/四元组 Params、原 for/next、原编码和摘要委托；
本包没有新不透明函数或替代业务算法，也不手改生成体。整个生产到隔离候选的一般等价
没有因此闭合。固定工具、重锁、四核及独立 .lake/build 保持，失败和缓存保留。

`swap-row-placeholders` 只把同一实际 Rust SQL 的 ?1 与 ?4 交换，参数值、求值顺序、
NULL、配置读取及其余代码均不改。实际生成体在相同正常行上被同一个完整 Contract 拒绝。
形式反例使用明确的 SQL 识别端口，说明调用合同被破坏，不冒充 SQLite 的实际解析；
下述原生回归另由真实 SQLite 观察列值，且必须先实际编译。

## 证明机复现

`init_project/init_audit` 为 #992 正确原件；`parent_inputs` 是私有 JSON，恰含
actual、reference、prior、audit、policy_inputs 五个原物化参数。各私有路径从现有交接
配置读取，不在公共文档记录机器地址。所有目录必须是新输出，已有失败与缓存保留。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-row
parent=verification/experiments/stage1-protocol/unload-parent-link
for variant in original swap-row-placeholders; do
    bash scripts/python-entry.sh "$package/extract.py" --variant "$variant" \
        --output "$row_evidence/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
    project="$row_evidence/$variant-project"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$row_evidence/$variant" \
        --reference "$row_evidence/original" --prior "$init_project" --audit "$init_audit" \
        --parent-inputs "$parent_inputs" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$row_evidence/$variant-audit" --lock "$SANDPROOF_LOCK_FILE"
done
```

按 registration-candidate.json 检查完整物理文件集合、全部审计、依赖、非 stale/partial
服务结果。此处是开发证明复现，不签发公共整套通过；公共验收按主控批次采用。

## ning 原生与普通验收

native.py 只准备四份输入，没有执行测试。original 和 swap-row-placeholders 使用实际
生产；adapted 两版使用原隔离改写，但恢复原 lib.rs。原适配的未格式化字节另留摘要，
原生副本按配置中的固定 rustfmt 规范换行，捕获原件不动。四版都追加同一份新行断言和 #990
原三条策略控制，使用各自独立 target 与原执行身份检查，不装 Charon 抽取根。

新用例通过原 v3::open 建真实 journal，在事务内读回完整已校验策略，将每会话操作数设为
合法的 3、会话数设为 1。它调用实际 initialize，读取全部五列，核对唯一行的完整 tuple：
id=1、原 null JSON、实际 Sha256(null)、原 required_bytes(1)、key=None。id 和预留明确不同，
错误 SQL 合法且只创建一行，不会因重复主键或编译失败先停，必须到达同一完整 tuple 断言。
其它三条控制使用原完整合法/非法策略，事务回滚后保持原配置。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-row
for variant in original adapted-original swap-row-placeholders adapted-swap-row-placeholders; do
    case_root="$row_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run
done
```

ning 在上述新编译同 crate libtest 中按 registration-candidate.json 的四个完整名字分别执行
`--exact`，绑定 manifest/src/fresh、实际程序摘要及私有输入；不可只运行 source_init_row_
过滤而遗漏三个策略控制，也不能仅凭 cargo exit 101 认作性质拒绝。

- 正确及隔离正确各应 4 通过、0 失败。
- 两种错误各应 3 通过、1 失败；唯一失败为新行用例的原断言
  `initial source row must preserve all four bound values and NULL key`。
- 必须区分准备、初始化、编译、环境、超时和目标断言。失败证据保留。
- 错误版之后另建全新 original 恢复副本，编译并执行四条，预期 4/4。

最终精确候选普通验收继续使用原已校验 Redis，执行无参数 bash scripts/check-function.sh。
公共 proof_native_suite、verify-stage1-protocol/verify-controller、公共登记与覆盖由主控同批
统一验收和内容复用；本机没有执行这些普通/原生入口。本包不新增 KVM，不改变 #641 队列。

## 剩余义务和提供方

V0/主控审查原 #985/#986 库投影、Range/借用与 #992 tuple Params/薄委托的采用及一般等价；
本包没有扩大外部合同。V5 提供实际 SQL 参数效果、同事务/快照、JSON/摘要实例、完整循环、
元数据写入、耐久初始化及完整数据库预算；V7 提供独立认证/配置来源；V6 提供材料/VM/物理
事实。后继可复用本片精确单行合同，不能据此宣布整个初始化、V3、故障恢复或系统已证。
