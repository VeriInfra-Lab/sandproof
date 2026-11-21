# V3：原 seal_end 的完整值合同

本包依赖 #999，生产固定在 b0d791ccc4d4e058b710dbf8f46766652b2853df，
不自动追随工程分支。直接入口为原 full/end.rs 的 seal_end，函数体没有分离或改写。
Command、WorkAction、SourceWorkRecord 和完整字段类型沿用父包，不另建协议模型。
生产只读，公共登记及覆盖仅提供候选，正式原生和普通验收交 ning。

## 精确性质与范围

SourceSealEndModel.Contract 对正确和实际错误根使用同一 outcome，量化任意已解码的
完整 record 和 action。Command 的十三个实际变体中只有 SealSnapshot 可以成功；
其它十二个返回 CorruptStore。SealSnapshot 依次检查 sandbox、来源 generation、
当前 VM、动作 identity epoch、identity generation、identity attempt，任一不符返回
CorruptStore，全部符合时返回准确的 attempt、workspace、snapshot、manifest 四字段。

command_accessor 连接原透明 EffectId.command，actual_contract 连接完整原生成根，
actual_full_contract 满足上述同一个 Contract；normal_seal 给正常合法条件见证。
source_generation_mismatch 给来源代不符拒绝。六项检查均返回相同错误，此根没有
进一步可区分的错误种类。合同保留真实命令形状先于绑定检查的分支。

实际错误 Rust 只删除 generation != original.native.generation，保留其它五项。
正常对照两版都接受；其它绑定正确但来源代不同时，正确根拒绝、错误根接受，
rejects_same_full_contract 否定同一个完整 Contract，未替换正反性质。

原根不检查 capture 是否存在、不校验完整 end record，也不做来源查找、授权、Writer
退出或物理封印。输出 workspace/snapshot/manifest 来自命令，不能推出与整个 Full/
World 相符。无 capture 的值片段仍可成功，这是原函数的真实范围，不是完整账本的
合法状态见证。本片不推出持久接受、资源存在、同一事务、JSON/TLS 或完整系统结论。

## 原输入、逐声明复用与固定工具

capture.py 保留父包五处既有库调用/编码隔离，仅将入口指向原 seal_end；第六处登记
full/end.rs 在正确版逐字不变，在错误版只移除该一项来源代比较。没有替代算法，
原 EffectId.command 和本包所有业务/核心/协议声明必须透明。完整隔离对生产的一般
等价及原库逐符号采用仍需主控协调，不能由此声称整个父调用已证。

validation.py 重新构造全部 4906 份源文件，核对归档、文件及执行位、工具命令/日志、
LLBC、映射和全部五个生成文件。两版 110 个完整类型、七个实际函数清单相同，所有
类型和外部模板逐字一致；唯有 seal_end 体改变，检查器只允许删除确切生成条件及
对应一层缩进。所有共同函数完整声明相同。

父 #999 的 136 文件/761 审计及其中 #945 的 26/127 原服务证据完整保留。
109 个共享类型和五个共享函数的完整体逐字匹配父实际视图；只有原 CaptureSealEnd
类型、原 EffectId.command 和原 seal_end 两函数从固定新输出插入。唯一外部接口为
已有标准 Option 相等，由既有逐符号合同和 SourceWorkMatch.CoreEffects.optional_vm_eq
提供；不导入带 axiom 的模板，没有新增业务 opaque、自定义公理或占位证明。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-seal-end
for variant in original ignore-source-generation; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$seal_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$seal_inputs" \
    --output "$seal_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入 JSON 恰含 correct、wrong、reads、reads_audit、retention、retention_audit 六键。
前两项是本包新捕获；中两项是 #999 完整正确项目及原审计；后两项是 #945 原完整项目
及原审计。缺原件、摘要、日志或原服务响应必须拒绝。沿固定四核、80 GiB 抽取准入、
64 GiB 运行停机线及共享重锁；保持原工具和所有缓存，不以资源失败充当性质反例。

正确139实际文件/767唯一审计，错误141/771。本包 check.py 只给未改父完整检查器
提供物化器、变体和自有登记路径，保留全部角色、共享重锁、完整构建、逐真实文件
同服务诊断、依赖审计、新目录逐字复现及固定依赖检查。两版独立 .lake/build，缓存
仅复制。检查器自己持锁，外层不能再持同一把锁。stale/partial、仍在检查、失败依赖、
警告或错误均不计通过。此入口是开发形式检查，不签发公共或原生验收。

## ning 原生交接

native.py 只准备四份独立完整输入，不执行编译或测试。保留 #999 全十五项断言，
再加三项：准确四字段输出及不偷加 capture 前提；逐一变更六个真实绑定；非封印命令
拒绝。用原 settled_terminal、真实 SQLite/JSON 和受控 Host；测试桥仅在 cfg(test)
下调用原 seal_end 及父包原 find，全部用例核对原数据库行不变。

原始两版仅安装测试桥/断言及同一单项错误变更。隔离两版保留捕获的既有库适配并
恢复原 lib.rs，原 find 不分离。该根不调用 find，父十五项仍对原 find 提供有限原生
对照。分别记录格式化前/后的全部字节，使用固定 rustfmt，不修改捕获原件。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-seal-end
for variant in original adapted-original ignore-source-generation adapted-ignore-source-generation; do
    case_root="$seal_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

每版先实际编译，核对同 crate libtest 的 manifest/src/fresh 及源码、输入和程序摘要，
再用 --exact 执行 registration-candidate.json 的十八个完整名字。只筛 source_seal_end_
会遗漏十五项父控制。正确两版各18/18，错误两版各17通过/1失败；唯一失败必须到达
seal end must retain every original sandbox, source generation, VM and action binding，
对应 field=1 的来源代变更。正常、其它五项拒绝及父控制仍须通过。

准备/编译失败、导入失败、超时或其它断言不是性质拒绝。两错误之后独立重建 original，
重新编译执行十八项，恢复必须18/18，保留所有失败证据。最终由主控设置已校验 Redis，
跑无参数 bash scripts/check-function.sh，统一登记/复用 proof_native_suite、公共
verify-stage1-protocol/verify-controller、根导入和覆盖，不各自重复完整套件或新增 KVM。

V3/V5 的原来源查找/借用、完整父调用与合法 OLD World/事务/提交未知，V6 的真实封印、
Writer/材料/VM，V7 的独立认证继续开放。ContentDigest 普通字节值或相等不证明计算过
哈希或材料真实完整；此处也没有扩张父 JSON/SHA 局部结果为完整 Serde/网络结论。
