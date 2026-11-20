# 来源元数据写入与原循环入口

[容量父调用](../source-init-capacity/README.md) · [完整单行](../source-init-row/README.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md) · [证明工作流](../../../../../docs/proof-workflow.md)

接续 #993，固定生产 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。本包连接原初始化通过
容量门之后的完整续体：克隆节点、配置编码和大小门、两个源表 DDL、完整元数据 INSERT、
空行摘要及原循环。它保留完整原循环体，但没有证明任意行数的循环或物理初始化完成。

## 完整合同与表示关系

`SourceInitMetadataSpec.Contract` 对任意原 Ports、连接、六字段来源配置、原八字段 Inbox、
u64 行数和预留成立。编码输入是完整 Configuration 的三项：来源配置、按原顺序克隆的
完整 NodeIdentity、原 max_sessions；不丢部署、节点或运行代。克隆返回原字符串时，三个
节点字段全部保留。字符串克隆、认证来源及真实配置的提供方不由字段相等代替。

编码错误运输为原 Json，失败和非返回保持。编码字节严格大于 8192 时，在任何源表 DDL
之前拒绝；恰好 8192 不被此门拒绝。随后按顺序创建原元数据表和工作表，调用原摘要口，
再执行精确 SQL：

```sql
INSERT INTO controller_source_work VALUES(1,?1,?2,?3)
```

三个完整参数是原编码字节、原摘要返回的完整 32 字节 slice、受检的 8192。另核对同一
TupleABC Params 的三值绑定。SQL 成功回复中的影响行数不被检查，零也进入后段；错误、
失败、非返回及先前已经发生的效果沿原 Result 绑定保持，不能从后来 Err 推导早前 DDL
没有效果或事务已经回滚。

元数据 INSERT 成功之后才请求原 null 字节摘要，随后以相同行数、预留、null 字节和完整
摘要进入原 `write_initial_source_rows`。前面任一步失败不进入后一步；摘要返回 32 字节
只提供值，不证明已计算哈希。原 #992 的父调用和公开转发直接组合到同一完整合同，保留
两次读取的独立条件；#993 的一般性单行合同和全部原证明继续复用。

正常见证使用合法的六项配置、单会话/单行以及完整 JSON 数据，实际原单行函数和循环的
一次 Some/None 路径终止。提供的两个摘要值和编码字节只是端口见证，未证明 Serde/SHA
计算；真实实例另由 ning 执行。有限正常见证不替代任意输入合同或一般循环证明。

## 捕获复用与同合同错误版

正确捕获复用 #993 原件，并重做归档、staging、工具日志、LLBC、全部生成物、类型/调用体
准入；完整续体逐字等于 #992 原生成函数中截取的内容。父 93 文件/408 审计闭包重建，
完整计划逐字相同。没有重新运行未变化的正确 Rust 捕获，也不把旧结果改记为新执行。

`metadata-reserve-short` 的真实 Rust 只在 initialize 的元数据参数把
`sql(METADATA_BYTES)?` 改为 `sql(METADATA_BYTES - 1)?`，加载器、预算、大小门、其它参数、
SQL 和所有其它函数保持。错误版实际编译并经固定 Charon/Aeneas 抽取；全部生成类型及
函数名相同，只有原 initialize 体变化。物化器截取完整实际续体，生成体不手改。

两个视图共用同一 Spec、正常见证和正确证明。错误续体在同一个正常输入上把 8191 送入
同一 typed 参数观察器，否定原完整 Contract；这个端口反例不声称真实 SQLite 会 panic。
下述实际 SQLite 完整列断言另验该错误，不能将编译、导入或归约失败当作性质拒绝。

隔离改写、Params、原 next、借用和摘要返回委托沿 #992；本包没有增加业务 opaque 或
工具规则。整个生产到隔离候选的一般等价仍开放，不能因这一段对应成立而自动完成。

## 证明机复现

`row_project/row_audit` 为 #993 正确原件，`row_capture` 为它已完整捕获的原版。
`row_inputs` 私有 JSON 恰含 actual、reference、prior、audit、parent_inputs 五个原物化
参数。工具、缓存和路径使用既有私有交接；四核、原重锁和独立 .lake/build 不变。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-metadata
parent=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/extract.py" --variant metadata-reserve-short \
    --output "$metadata_evidence/wrong-capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
for variant in original metadata-reserve-short; do
    actual="$row_capture"
    if test "$variant" != original; then actual="$metadata_evidence/wrong-capture"; fi
    project="$metadata_evidence/$variant-project"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$actual" \
        --reference "$row_capture" --prior "$row_project" --audit "$row_audit" \
        --row-inputs "$row_inputs" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$metadata_evidence/$variant-audit" --lock "$SANDPROOF_LOCK_FILE"
done
```

核对 registration-candidate.json 的全部实际文件、唯一审计、依赖和完整非 stale/partial
结果。缓存只可在完整父源码一致后复制到独立目录，不能共写 build、删除 .lake 或改工具。
此开发入口不签发公共整套成功；公共根、正式登记、原生与覆盖候选仍交主控统一采用。

## ning 原生与普通验收

native.py 只准备输入，不执行测试。四版为原始正确、隔离正确、原始错误、隔离错误；
隔离副本恢复原 lib.rs，另记原适配摘要后用固定 rustfmt 换行，捕获原件不动。各版追加
同一新元数据断言、#993 原完整行断言及 #990 原三条策略控制，共五条。

新用例用原 v3::open 和实际 SQLite 事务执行真实 initialize，读取全部元数据行及四列。
完整 tuple 必须是唯一 id=1、完整 Configuration 的原编码、实际 Sha256(编码) 和 8192。
另解码并核对完整六字段配置、完整节点及 max_sessions。错误的 8191 仍满足原表约束，
必须到达完整 tuple 断言，不能靠准备错误或 SQL 约束失败提前停止。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-metadata
for variant in original adapted-original metadata-reserve-short adapted-metadata-reserve-short; do
    case_root="$metadata_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run
done
```

ning 核对每个实际新编译同 crate libtest 的 manifest/src/fresh、实际程序及输入摘要，再
按 registration-candidate.json 的五个完整名字逐项 `--exact`；单一 metadata 过滤会漏控制。
正确两版各应 5/5；错误两版各应 4 通过/1 失败，唯一性质拒绝必须是
`initial source metadata must retain the complete configuration, digest and 8192 reservation`。
错误之后另建 original 恢复副本，编译并执行五条，应 5/5。失败、超时、环境或编译错误
分别记录，退出 101 本身不算反例。保留失败、原始日志和实际资源收尾。

最终采用候选仍用原已校验 Redis 运行无参数 bash scripts/check-function.sh。
proof_native_suite、verify-stage1-protocol/verify-controller、公共登记与覆盖由主控同批
统一验收及内容复用；本机未运行这些普通/原生入口，本包不新增 KVM 或改变 #641 队列。

## 后续提供方

V0/主控继续审查原 Row/Range/借用、tuple Params 和薄委托采用；V5 提供真实 SQL 效果、
同事务/快照、实际 JSON/摘要实例、一般循环、耐久初始化及整库预算。V7 提供独立认证与
配置来源，V6 提供实际材料/VM/物理事实。局部值/调用对应不能替代这些事实，也不完成
整个初始化、故障恢复、V3 或系统组合。
