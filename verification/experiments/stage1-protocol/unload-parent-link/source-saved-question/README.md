# V3：从原保存记录重建终止观察问题

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包准备连接原 question::from_saved、original_identity 和 assemble，复用 #1010
完整 Full/来源绑定、#1001 六项原身份条件和 #1008 原八参数构造器。固定父
ee789e4bbdb076ce0499d5a6d4543c0957bb6de6，生产固定 b0d791cc。正确与真实漏守卫两版已用固定 Charon/Aeneas 实编译、抽取，
全部业务与派生克隆保持透明。值定理和反例须经实际文件、完整双视图及公共入口分别
检查；原生、普通及公共整套验收仍交 ning/主控。

## 合同和原首错

同一套 canonical Record/Full/Action/Identity/Stop/Slot/Observation/Work 类型，
不复制另一套协议数据。FullBound 直接复用原完整 FullBindingModel.outcome；原身份
六项比较直接复用 SourceSealEndModel.checks。本包从已解码普通 Rust 值开始。

原顺序为 inode 零先 CorruptStore；缺 capture 则 CorruptStore；完整 Full 绑定失败
原样 Conflict，早于缺 action 的 UnresolvedAction。action 必须 SealSnapshot，且
原 sandbox/native generation/current VM、epoch/generation/attempt 六条件全部符合。
成功身份准确保留原 capture.operation 和原 SealSnapshot 的 epoch/generation/vm。

assemble 保留原十四比较：完整 StopVm 命令、Stop sandbox/异于原 key、严格较大
Stop epoch、两项 attempt、epoch/generation、原 seal 的公开 owner/sandbox，以及
宿主 binding 的 owner/sandbox/generation/vm。允许 epoch 编号空洞，不要求加一。
只有这些条件和前序检查满足才调用原 from_source，准确保存全部八参数。

from_saved 不比较 observation 的 work_binding/full_work_binding、seal_action、
source、worker、device；只读 inode、stop_action、host_slot。它也不检查宿主槽的
position、expected_previous、network；完整观察匹配和当前宿主上下文是另两项义务。
本值函数的接受不产生物理门、退出、完整材料、授权或耐久接受资格。

错误源只删 stop.epoch <= id.epoch 的拒绝子句。同一完整合同必须继续用于正反版，
其它十三比较和前序检查保留。唯一反例令 Stop 与原身份同 epoch，同时保持命令和
公开 epoch 一致，避免用其它拒绝掩盖缺失守卫；合法大跨度 epoch 是正常控制。

## 精确原件与完整形式入口

两版112个完整类型全部逐字对应父同一 canonical 定义，不新增协议类型；145个
完整函数/字典中137个原体逐字对应父，新增四个原比较字典和原 key/identity/assemble/
from_saved。全部当前原体逐字物化，保留原 reducible 属性。两个原完整 assembly
后缀按实际固定生成体核对，仅允许漏 epoch 守卫及随后缩进/换行变化，不做一般
alpha/空白归一化，不改原生成物。完整比较、Full/Pause 合法性、所有自有克隆与原
八参数构造器均用原生成体及已有一般证明；有限见证不代替任意值合同。

精确21个外部标准接口。原14项与 String/Option 克隆、Option 共享借用/完整比较、
Bool.ne 全部转发已有逐符号模型。Option Try::branch/FromResidual::from_residual
逐字选择原 A6-foundations-stdlib 两个完整定义，不导入 axiom 模板、不造业务端口。
有界 toStr 仍逐字复用原父适配；其原模板和两段 V0 模型的原公开文件也进入闭包。
17个本包文件加这两项原件，共19项精确输入；固定库的具体采用仍是 V0 条件。

SourceSavedQuestionModel 的 IdentityContract、AssemblyContract 和 Contract 是同一
完整规格链。失败分支的实际 zero_inode_first/assembly_identity_error/
from_saved_identity_error/bad_stop_before_constructor 不借助克隆或构造器定理就返回；
failed_result_contains_no_request 另证明错误结果不含工作请求。该结论限于原值函数和
正常库模型，不是任意 OOM/abort、HTTP/提交失败、真实退出或系统资源释放保证。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-saved-question
for variant in original allow-old-stop-epoch; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$saved_question_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$saved_question_inputs" \
    --output "$saved_question_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入 JSON 必含 correct、wrong、reads、reads_audit、retention、retention_audit；后四项
分别为 #1010 完整正确项目/原审计及 #945 原保留项目/原审计。全部父186文件/958审计
与原保留26/127完整保持；本包正确目标191文件/1014唯一审计，错误193/1018。新增
局部7文件/60审计不能代替完整双视图。全部实际文件、19项精确模板、父原件、十个
固定 Lean 依赖和新目录逐字重建都须检查；各版独立 .lake/build，共享重锁、固定
工具和预算不变，保留失败与缓存。形式开发、普通/原生和公共整套验收分别记录。

## ning 原生交接

原生准备器保留父43个完整名字，新增五项：准确重建及未检查字段控制、首错、六项
身份、十三个 Stop/宿主字段、严格较新 epoch。源真实错误只删一条比较；正常和较大
编号空洞控制保留。同 epoch 反例同时调整 StopVm 命令与公开 epoch，其余十三条件
一致。四视图预期各48项，正确两版48/48，错误两版47通过/1失败，唯一拒绝须到达
`saved question must require strictly newer stop epoch`。源准备、编译失败、环境缺失、
超时、其它断言或退出101本身不算性质反例。本机不编译或执行普通/原生验收。

复用原 API/SQLite/JSON 与受控 Host，前后比较完整 SQLite 行和 Sources；没有物理
进程/VM/门/FD事实。不新增业务桥，保留三份父桥并复用原 source_terminal_work_for_test
对原 question::from_saved 的转发。不会把整个业务克隆、整个库或持久事务设为可信。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-saved-question
for variant in original adapted-original allow-old-stop-epoch adapted-allow-old-stop-epoch; do
    case_root="$saved_question_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立 source/target；真实编译后核对同 crate libtest 的 manifest/src/fresh、全部
源输入与实际程序，逐项 --exact 执行 registration-candidate.json 全部48个完整名字，
不能只筛本片前缀而漏父43项。两错误后另建 original，重新编译执行48项，恢复48/48
并保留失败。最终采用候选显式设置原已校验 Redis，执行无参数 bash scripts/check-function.sh。

当前 World 的 question::current/terminal_key/successful_stop、SQL/outbox/事务、未知
提交/耐久、独立认证、真实 JSON/TLS 与系统结论保持开放。公共 proof_native_suite、
verify-stage1-protocol/verify-controller、根/登记/冻结/覆盖只交主控同批统一验收及内容
复用，不跑旧根就称本包通过，不并发改共享文件、不自行合并或新增 KVM/改实机队列。
