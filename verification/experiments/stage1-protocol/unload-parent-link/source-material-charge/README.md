# 完整来源材料计费与闭包绑定

[证明工作流](../../../../../docs/proof-workflow.md) ·
[V3 详细计划](../../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包固定生产 `b0d791ccc4d4e058b710dbf8f46766652b2853df`，
树 `7ca91afd9cd79601d562f3c636066d192fc25fc5`，接续 #635 / #997。
复用同一完整来源类型、原克隆/比较、SQL 及原 SHA 模型，生产源码保持只读。
不把历史输入迁记为整个生产事务或系统完成。

## 实际根与完整合同

`SourceMaterialChargeSpec.Contract` 对正确和错误视图使用同一个性质：
对全部已解码 `SourceWorkRecord`，计费函数恰好等于完整有序的 `charge`。
`SourceMaterialChargeProof.actual_contract` / `actual_full_contract` 连接实际生成体。
它保留原 Rust 错误、语言失败和非返回，没有把业务校验器作为假设。

1. 无 capture 时，只有退休状态为空且 producer/consumer 释放事实都为空才返回零。
2. 有 capture 而无退休状态时，原函数直接返回原预留，不额外检查释放或恢复状态。
3. 有退休状态时，先编码完整状态并执行原 `69,632` 字节门，才读原计划。
   空计划和超过 `16,384` 字节的计划先拒绝。
4. 原 request 完整克隆来源和捕获字段。所有命令、网络/驱动列表及全部嵌套字段都保留；
   绑定编码接收原 sandbox、generation、full、source、seal_action、worker、manifest_digest 七字段。
5. 工作绑定、计划摘要和 material_use 依原顺序检查；闭包先检查 producer/consumer
   存在条件，再检查完整释放事实摘要和原恢复绑定。恢复绑定编码保留 intent、local、
   generation、activation 四字段。无 restore 时不执行其编码。
6. Retiring 返回原预留。Settled 依次检查 work_binding、plan_binding、
   released_material_bytes 与原捕获预留相等，全部保持才返回零。

原 Retiring 预留为零时可以返回零。零计费不等于已结算，不授予删除、授权、持久接受
或物理退休许可。material_charge 的结构检查不能替代外层 validate_record、关系校验、
合法来源、实际释放和物理闭合。

计划、释放、恢复摘要沿原已登记的 StandardDigest SHA；它们与记录编码的摘要端口及
请求绑定的两个适配分别处理。只复用原输入域、表示和库合同，不证明整个 sha2 实现、
完整 Serde 或认证来源，也不把 32 字节往返解释成材料真实。

## 正常控制和实际错误体

`capture.py` 复用 #997 的完整抽取环境。捕获根仍为原逐记录清槽，选取其中透明的
material_charge、request、bounded、restore_binding、validate_bindings 实际声明；
原父循环的嵌套借用及整个生产到隔离的一般等价仍未闭合。
两版都重新实际编译、用固定 Charon/Aeneas 抽取，不更换工具或手改生成体。

`ignore-settlement-bytes` 只删原 material_charge 的结算释放字节比较。
完整类型、外部接口和函数清单保持；只有该函数体改变。删去一行使状态编码适配的
源码注释位置由 551–553 变为 550–552，检查器只允许这一处注释差异，接口声明不变。

`SourceMaterialChargeCases.Ready` 为正常及反例提供同一完整记录的精确逐调用条件：
完整状态编码及大小、七字段请求编码/摘要、原计划与释放摘要、无恢复及完整释放/结算
字段。原请求克隆、大小门、闭包和材料计费都是已证明的实际函数，不是假设。
Ready 没有实例化正式 JSON 模式、合法数据库来源或物理释放；不构造冒充真实编码的
字节，也不将这些条件当成整个 Serde 或事务正确性的证明。

原根在匹配预留时返回零，结算释放字节不符时返回 CorruptStore；实际错误根两者都返回零。
同一正常输入在两版接受，同一坏字节输入的错误体被同一个完整 Contract 否定。
这份有条件生成体反例不替代下面的真实原生执行。

## 证明机复现

沿原固定角色、工具、四核、内存预算和共享锁分别捕获两版：

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-material-charge
for variant in original ignore-settlement-bytes; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$charge_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$charge_inputs" \
    --output "$charge_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

`charge_inputs` 恰含六个私有目录键：correct、wrong 为本包两版完整捕获；reads、
reads_audit 为 #997 完整正确项目及原实际文件审计，retention、retention_audit 为
#945 完整原项目及原审计。父项目分别为 130 文件/675 审计、26 文件/127 审计，
后者已完整包含于前者。缺少完整原服务响应、源码、审计或日志摘要必须拒绝。

物化器逐文件重构并核对完整生产归档、五份 staging、LLBC 和全部生成物。
原父七个视图从当前正确生成物重新物化并逐字比较；93 个共享类型和 105 个共享函数
仍须对应。保留父全部 130 份实际文件，再增加本包完整模块。正确版共 133 文件/
752 项唯一审计，错误版共 135 文件/756 审计。错误体只从当前工具输出原样插入。

check.py 仅为父完整检查器供应当前物化器、固定两变体和登记位置，保留其全部角色、
锁、构建、实际文件诊断、审计、新目录逐字复现和固定依赖检查；不提供缩减清单或只验
正确版入口。检查器自己持一次共享锁，不能在外层重复加同一把锁。两版使用独立
.lake/build，缓存只能复制到独立目录，不能删除 .lake。原同一服务必须返回完整、
非 stale/partial、零错误/警告及失败依赖；不能只构建父根便报告本包通过。
本入口始终标开发检查，不签发公共原生或整套验收。

## ning 原生与普通验收

native.py 只准备原始/隔离正确及两个实际错误副本，恢复原 lib.rs，另记原适配字节后
由固定 rustfmt 规范换行。每版包含 #997 原七条清槽断言和本包四条相同断言，
共十一条。源码准备不执行测试，不运行 VM。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-material-charge
for variant in original adapted-original ignore-settlement-bytes adapted-ignore-settlement-bytes; do
    case_root="$charge_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

ning 使用各自独立 source/target，先实际编译并核对同 crate libtest 的 manifest/src/fresh、
实际程序、输入及二进制摘要，再逐项执行 registration-candidate.json 的十一项完整名字，
使用 --exact。只过滤 source_material_charge_ 会漏掉七条父控制。

本包四条新检查使用原 settled_terminal 夹具、真实 SQLite/JSON 和原受控 Host：
无 capture 的释放为空条件；Retiring 的原预留及零预留不等于 Settled；结算 work/plan/
released 字段完整对应；实际超大状态编码须先于计划形状报 MaintenanceCapacity。
记录只在本地克隆中改动，每次还核对完整原 SQLite 行不变，不拿受控事实当物理释放。

正确两版各应 11/11；错误两版各应 10 通过/1 失败，唯一拒绝必须到达：
`material settlement must match every released byte of the original capture`。
原正常结算、工作/计划错绑定以及全部父控制仍须通过。编译、环境、超时、其它断言或
退出 101 本身不能算反例。两个错误之后独立重建 original，实际编译并执行十一条，
恢复结果须为 11/11，全部失败保留。

最终采用候选显式设置原已校验 Redis 后，运行无参数 bash scripts/check-function.sh。
共享 proof_native_suite、verify-stage1-protocol/verify-controller、公共原生/形式/根导入
和覆盖由主控同批统一验收及内容复用，不各自重复全套。V0 的库采用、V3 的完整 JSON
模式、V5 的合法来源/外层关系/事务和耐久、V6 的物理闭合、V7 的独立认证仍各自提供。
局部开发结果不表示公共整套、完整 V3 或系统已证；实机仍沿原分工交 ning。
