# V3：原终止观察的完整匹配

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包连接原 TerminatedCaptureObservation::matches、透明的
TerminatedCaptureWorkRequest::binding_digest 和 from_source，复用 #1006 的原
FullCaptureWorkRequest::binding_digest。接续 #1007 精确
242306e87fc4f9d8b7b5082f06cbe37beea21d4d；生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df，不自动跟随工程新头。
形式开发、原生/普通和公共整套验收分别记录；源和规格准备不是已证回执。

## 同一完整合同

SourceTerminalMatchModel.Contract 对任意原完整观察、工作和逐符号库结果量化。
两版使用同一 outcome，保持原先后顺序：

1. 将 identity、seal_action、full、source、可选 worker、stop_action、stop、host_slot
   按原八元组顺序交给原 JSON 调用。错误先返回；成功才将全部返回字节交给原 SHA。
2. 原 work_binding 与所得完整32字节值不同时返回 false；该结果独立于后续 Full
   编码结果。相同时才接父 Full 绑定，错误先于其后的字段比较传播。
3. full_work_binding、完整 seal_action/stop_action、source.source、完整可选 worker
   和完整 host_slot 必须逐一相等。SourceObservationRequirement 的其它字段进入
   第一步原完整编码输入，但本函数不额外逐字段比较它们。

from_source 只把所有参数原样保存，包括原 terminal 对的 action/stop。
ConstructorContract 不以注释或工厂可见性推断输入已经被授权、已从耐久账本取得、
真实进程存在、已经停止或已经排空；这些仍是实际调用方及 V4/V5/V6 的责任。

原 matches 忽略 helper_gate_device/helper_gate_inode。改变这两个字段不影响本值
函数；from_saved 的非零 inode 首拒绝和受保护物理 provider 的 FD/退出保证是另两项
义务，不能把任意门值检查偷偷加入本合同。worker=None 也不是物理排空事实。

## 表示关系与复用

所有类型和函数从固定实际 Rust 经原 Charon/Aeneas 重新生成，不手改生成体。
与父相同的类型须逐字对应同一 canonical 定义；只允许加入原完整终止工作类型。
完整 action、进程、可选 worker、宿主槽及32字节比较复用已证一般引理，有限见证
只检查非空正常接受和字段遗漏，不代替普遍结论。

两版60个完整类型中59个逐字复用父 canonical 定义，仅加入原完整终止工作类型。
正确52/错误48个完整函数/字典逐字物化，48个正确原体逐字对应父；另有原构造器、
终止绑定、匹配和只保留实际使用 eq 的 worker 字典，后者直接实例化已有完整 worker
比较定理。错误仅改变匹配根并剪掉四个不可达宿主比较函数。生成器将独立 TenantId
别名移到三个 digest 别名之后；检查器核对该完整原块的唯一移动，其余类型原文不变，
不改生成体、不做一般重排或任意 alpha 归一化。

精确六个外部接口为 String.eq、Option.eq 和两套原编码/摘要调用。前两者复用父同一
逐符号解释；编码器参数分别为完整原 Full 对象与原完整八元组，两个摘要调用共享同一
provider。绑定输入一般单射只说明八元组没有遗漏字段，不给任何摘要单射或物理事实。

仅在隔离副本把原八元组 to_vec 和原 SHA 调用移到两个只转发的薄 helper；父 Full
仍采用同一对原调用分离。两处 SHA 共享同一个逐符号摘要 provider，业务方法保持
透明。不导入 axiom 模板，不把完整 Serde、业务 binding_digest 或持久事务设为可信。

实际错误只移除 host_slot 完整比较，并把其模式绑定改为通配以免制造未使用变量
编译错误；其余守卫和构造器保持。正常控制须保留，只有改变观察宿主槽的值被错误
接受，并由同一个完整 Contract 否定。编译/环境/超时/导入失败不是性质反例。

整个生产到 staging 的一般等价、具体 JSON/SHA/比较调用采用仍待相应提供方；源码
逐字/格式回读和原生对照不冒充一般性机器精化。真实字节、认证、当前 World、完整
父调用、事务、未知提交、耐久、物理进程/门/材料/VM/Writer 和完整系统结论均未闭合。

## 完整开发入口

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-terminal-match
for variant in original omit-host-slot; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$terminal_match_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$terminal_match_inputs" \
    --output "$terminal_match_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入为 correct、wrong、reads、reads_audit、retention、retention_audit；后四项是
#1007 完整正确项目/原审计和 #945 原完整保留项目/原审计。须保留全部父证明与其
源闭包，核对实际文件/唯一审计/每个已提交模板、新目录全部计划和文件的精确重建、
同一注册服务完整非 stale/partial 诊断及十个固定 Lean 依赖。两视图独立 .lake/build，
共享重锁和固定工具/预算不变，保留失败和缓存；不能仅默认构建或只查新增文件。
原父175文件/905审计须完整保留，本包完整目标为正确181/931、错误183/936；
新增局部8文件/31审计不能代替完整双视图。

## ning 原生交接

native.py 只准备四份完整隔离源码，不编译或执行。父37项保持，另加正常完整构造/
可选 worker/未检查门值控制，两个绑定/完整动作/来源/worker 拒绝，以及完整宿主槽
七个字段拒绝。临时 cfg(test) 桥只转发到原 question::from_saved，不重写业务或改库。
复用真实 API/SQLite/JSON 和受控 Host；调用前后比较完整 SQLite 行与 Sources。
受控 Host 不产生真实进程、VM、退出、FD、Writer 或授权事实。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-terminal-match
for variant in original adapted-original omit-host-slot adapted-omit-host-slot; do
    case_root="$terminal_match_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

实际编译后核对同 crate libtest 的 manifest/src/fresh、所有源输入和实际二进制，逐项
--exact 执行 registration-candidate.json 的全部40个完整名字，不只筛本片前缀。
正确两版各40/40，错误两版各39通过/1失败；唯一性质拒绝须到达
`terminal matching must retain complete original host slot`。其它字段、正常控制和父
37项须保留。两错误后另建 original、重新编译执行全部40项，恢复40/40并保留失败。
准备/编译/环境/超时/其它断言或退出101本身不算反例。

最终采用候选显式设置原已校验 Redis，运行无参数 bash scripts/check-function.sh。
公共 proof_native_suite、verify-stage1-protocol/verify-controller、根/登记/冻结/覆盖
只交候选，由主控同批统一验收与内容复用，不跑旧根就宣称本包已验收，不自行合并。
