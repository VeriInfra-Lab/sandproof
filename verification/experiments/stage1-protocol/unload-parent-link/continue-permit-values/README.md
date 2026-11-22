# V3：继续运行许可的完整值接口

[证明工作流](../../../../../docs/proof-workflow.md) ·
[父六字段请求](../source-end-accessor/README.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包固定依赖 #1018 精确作者 2bb7a92f5dc61704d906dc81c3425219901fa4ee，生产
b0d791ccc4d4e058b710dbf8f46766652b2853df；不自动跟随分支新头。交付与验收状态在
#635 / 本包 PR 维护。只追加独占证明包，不改生产或公共配置、根、登记、冻结和覆盖；
公共采用交主控。本包尚在 Lean 开发，下面的库存与命令不是成功回执。

## 值合同和表示范围

目标是对任意完整 WorkAction 和 32 字节 ContentDigest，证明真实
OriginalContinuePermit.after_dispatch_commit 精确保留 action/work_binding，两个读取
方法精确返回整个值，构造后联合读取仍保留全部两个字段。复用 #1018 canonical
类型和完整父证明。ConstructorContract 在正反版本完全相同，ReadersContract
量化全部类型正确的 Permit；不靠有限样例代替字段完整性证明。

真实错误版只把保存的 work_binding 替换为零值，并保留原 action。原参数的无副作用
读取用于避免未使用参数警告。必须实际编译、抽取并到达同一完整合同的反例；工具、
编译、导入失败或超时不算性质拒绝。零值输入是错误版也能通过的控制，非零见证与
任意完整 action 的保存分别陈述。见证中类型正确的 action 不提供继续执行资格。

这是值构造和读取：工厂名字 after_dispatch_commit 不证明 SQL 已提交，也不使任意
给定 action 成为当前允许的操作。字段或摘要值相等不提供授权、耐久接受或物理存在。
ContentDigest 输入只是 32 字节值，不表示计算过哈希。借用采用父正常安全值表示；
不将读取等式解释为内存地址或认证来源。联合读取只是实际方法的证明组合。

六项一般对应和组合目标在 ContinuePermitValuesProof，三个正常/零值见证目标在
ContinuePermitValuesValues；ContinuePermitValuesNegative 有五项错误结果、完整
action 控制、非零反例、同一 ConstructorContract 否定和零值控制目标。全部目标必须
进入实际文件检查与审计，完成状态以 PR / #635 的终态为准。

## 实际生成体与完整开发入口

正确版与 discard-work-binding 版的固定 Charon/Aeneas 已实际编译、抽取并通过原
29 项接口的严格准入。118 个完整类型中 117 个逐字复用父，194 个函数中 191 个
完整父体逐字复用；只新增原 Permit 类型、实际构造和两个读取方法。错误版只改变
一个完整构造体，其余 193 个完整函数及 Types/TypesExternal/External 均相同。
错误构造实际生成 Array.repeat 32#usize 0#u8 后调用原 ContentDigest.from_sha256。
局部 notation 精确绑定父 SourceEndReceiptActual 同函数，生成体保持原样。

validation.py 回读全 4906 源文件和执行位、13 处 staging、完整工具命令与二进制、
日志、LLBC、映射和五份生成文件。全部字段使用父 canonical 类型，没有新增业务
opaque 或外部公理。首次物化的 bytes/str 比对错误保留；修正后重新在新目录物化，
不把这个驱动错误记为性质拒绝。开发预览的输入变化后须重新封口，不能刷新旧摘要。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/continue-permit-values
for variant in original discard-work-binding; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$permit_capture/$variant-01" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$permit_inputs" \
    --output "$permit_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入 JSON 为 correct/wrong/reads/reads_audit/retention/retention_audit 六项。correct
是完整正确捕获目录；wrong 直接指向完整 discard-work-binding-01 捕获目录。reads 使用
#1018 原 213 文件/1199 审计的完整正确项目，retention 保留原 26/127。完整两视图
库存为 219/1208 和 221/1213；这些数字不表示检查通过。入口保留所有父文件、全部
实际文件同服务完整诊断、审计、固定依赖检查，以及完整计划和文件的新目录逐字复现。
局部 Lake 通过不代替完整两视图或公共验收。

## ning 原生交接

native.py 只准备四套完整原始/适配输入，不编译或测试。保留父全部 61 项，追加一个
原 API 完整 action/binding 断言，覆盖零值、各字节非零及末字节变化。沿用原 SQLite
夹具，前后比较全部原行和 Sources。它不见证事务已提交、实际派发或执行权限。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/continue-permit-values
for variant in original adapted-original discard-work-binding adapted-discard-work-binding; do
    case_root="$permit_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

逐版核对同 crate 真实业务库/libtest 的 manifest/src/fresh 及源码绑定，并逐项 --exact
执行 registration-candidate.json 的全部 62 个完整测试名。正确两版各 62/62；错误
两版各 61 通过/1 失败，唯一断言必须到达
`continuation permit must preserve the complete supplied binding`。每个错误后都要
独立 original 源码/target 重新编译并恢复全部 62/62，共两次。不能只换测试二进制
而沿用错误业务库；编译/工具/导入失败、超时、其它断言或退出 101 本身不算性质拒绝。

源码、规则或夹具变化后，必须按最终提交重新准备四套源码和模式、完整规则/断言及
精确单文件变异。旧准备保留为旧版本证据，不更新摘要后沿用。普通 Linux 完整入口
需固定已校验的真实 Redis；共享 proof_native_suite 与公共 verify 入口由 ning/主控
同批统一验收和严格内容复用，不能只跑旧根报告新版通过。

## 提供者和剩余义务

本包只提供精确值接口。实际调用方仍须连接当前意图、同一 World/SQL/父事务、提交
未知和耐久派发，以及许可的实际使用。V1/V2 提供当前意图与耐久发送，V4/V5 提供
本地状态和原调用事务，V6 提供物理材料/进程/VM 事实，V7 提供独立认证来源；普通
借用、固定库接口及生产/staging 采用沿用 V0 父条件。没有新增完整 Serde、HTTP 或
收件事务可信假设。binding_digest、真实 JSON 字节/TLS/持久收件及完整系统仍开放；
不重复 #1018 的六字段请求构造，也不推定所给 binding 来自该请求。

public-registration.patch 和 registration-candidate.json 仅供主控审查，不能作为
公共接线或验收回执。普通和原生验收交 ning。私有证据位置和报告哈希保留本地。
