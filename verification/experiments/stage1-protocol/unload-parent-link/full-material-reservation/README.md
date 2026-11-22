# V3：Full 材料双副本预留

[父回执](../source-end-receipt/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包精确依赖#1014的29a97e6441167ccbf0abd8f27655303c63e9925d，生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df，不自动跟随工程头。实际检查结果在
#635/本包PR维护；下面的库存和命令不是成功回执。只改独占包，公共根/配置/登记/
冻结/覆盖由主控采用候选，不并发写，不自行合并。

## 完整原函数及同一性质

实际入口 source_work/full.rs::reservation 按原顺序：state_bytes加memory_bytes、
再加disk_bytes、乘2、再加2*262144+65536。对任意完整已解码Full，结果精确为：

```text
amount = 2 * (state_bytes + memory_bytes + disk_bytes) + 589824
amount <= u64::MAX  => Ok(amount)
amount >  u64::MAX  => Err(MaintenanceCapacity)
```

每步checked运算保持原首错和短路，不回绕。末次常量仍是实际普通u64乘/加，由原
库正常运算定理证明安全，不偷偷替换生成体为一个预计算字面量。实际Option.and_then、
三个原闭包及字典全部透明。合同量化全部Full，不加入FullAllowed或业务可部署前提；
函数没有读取其余完整Full字段，也不负责验证它们。

FullMaterialReservationModel.Contract 是正确及错误版的同一完整公式。Steps/Proof的11项
审计包括结果双向刻画、固定常量、真实四步展开、四个原checked运算投影小引理，以及
实际全域对应/成功/溢出条件。Values的9项包括复用父合法Full及原valid结果、正常成本、
零值成本且非合法工作，以及四处溢出见证。零值成功不授予执行或接受资格。

正常父Full的三个大小均为1，正确成本589830。真实错误版只把checked_mul(2)改为
checked_mul(1)，实际返回589827，被同一完整Contract反驳；保留零值和原首加溢出控制。
Negative共6项审计。编译/工具/导入失败、超时或仅准入拒绝不算性质反例。

这与#1015的full/end.rs::reserved_record_bytes（未来End的16KiB）不同；保留其原适用
范围，不重复、不迁记为本函数。这里只证明数值算法，不能从成本推出已占额/提交、
已持久接受、材料实际存在、封套/JSON编码成功、授权、发布/删除资格或物理VM事实。

## 实际抽取、复用与完整检查

固定原Charon/Aeneas各自重新编译并抽取两版。119个完整类型、191个完整函数/字典中，
116类型/183原体逐字复用父；只增加三个原闭包类型别名和八个原函数/字典（含实际标准
Option.and_then）。两版全部原始Types/TypesExternal/FunsExternal相同；仅乘法闭包的
完整call_once体中2改1，其余190个完整函数相同。不作通用alpha/空白归一化。

保留父29项逐符号接口，无新增业务opaque或库假设。新算术根自身只读取三U64，既有
29项是保留父根的完整库存。首次工具均成功但只保留receipt导致所需writer_closed
未生成，严格准入拒绝；补齐原material_writer_closed及receipt两个保留根后重抽。
错误工具目录的CLI解析失败亦保留，均不计性质拒绝。

完整正确视图214实际文件/1195唯一审计，错误216/1201，保留完整父207/1175及原
retention26/127。九个新增实际文件共26项审计，全部真实文件诊断、内核依赖审计、
输入内容和新目录全部计划/文件逐字复现均必需；局部通过不代替完整双视图。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/full-material-reservation
for variant in original omit-second-material-copy; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$material_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$material_inputs" \
    --output "$material_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入JSON固定六项correct/wrong/reads/reads_audit/retention/retention_audit：前两项为本次
两版实际捕获；reads及其审计为#1014完整正确项目，retention及审计为原26/127。
严格回读全部4906源文件/执行位、13处staging、固定二进制/完整命令/日志、LLBC、映射及
五份生成物。每版独立.lake/build，固定工具/预算和共享重锁，保留失败与缓存。所有
实际文件零错误/警告，无stale/partial、超时、失败依赖，再核对十个固定依赖才计完成。

## ning 原生交接

native.py只准备四套完整源码，不编译/执行测试。原source-end-receipt/native.py完整
保留父60断言、原API/SQLite/JSON/受控Host夹具及桥；本包新增同一完整公式断言，共61项。
桥只调用真实reservation。用原合法Full、零值、另一组正数及四处溢出边界，对照u128
完整数学公式；前后核对完整SQLite行和Sources。边界字段是显式已解码输入，不声称
建立了超大文件或可部署资源。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/full-material-reservation
for variant in original adapted-original omit-second-material-copy adapted-omit-second-material-copy; do
    case_root="$material_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

每版独立source/target，核对同crate真实libtest/业务库的manifest/src/fresh、来源和程序。
按registration-candidate.json全部61个完整名字逐项--exact执行，不只跑新增前缀。
正确两版各61/61；错误两版各60通过/1失败，唯一断言须到达
`Full material reservation must include both complete copies`。退出101、其它断言、编译/
工具/导入失败或超时不算性质拒绝。每个错误视图后各独立original/source/target重编译
恢复61/61，共两次，保留失败证据。首次准备用例名不符已拒绝；规则变化须重新准备和
回读，不只改哈希迁用旧输入。

最终候选显式设置原已校验真实Redis，执行无参数bash scripts/check-function.sh。
普通/原生及共享proof_native_suite、verify-stage1-protocol/verify-controller由ning/主控
同批统一验收与内容复用，不以旧根通过报告本包通过。公共候选仅在本包登记/补丁交付。

同父World/SQL/事务/未知提交/耐久/授权、V1/V2当前意图和发送、V6材料/进程/Writer/VM
事实、V7独立认证来源及真实JSON/TLS/持久收件/完整系统仍由原提供者闭合。
ContentDigest32字节值不表示已计算hash或材料完整。证据位置和报告哈希按用户决定保留本地。
