# V3：终止观察和已保存 prior 的实际顺序校验

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包准备连接原 drain::validate_record 和 writer_closed，固定父 #1011 的
552dbf5b43789b969f2b2304d801657de96b839d，生产固定 b0d791cc。复用原 #1011
from_saved、#1008 完整终止观察匹配和 #1007 prior 匹配，三个父合同保持原样，
不另造协议类型、较弱匹配或业务端口。准备、抽取、局部证明、完整形式开发检查及
原生/公共验收须分别记录；本包不是物理 Writer 关闭或资源释放定理。

## 原调用及边界

原 validate_record 先用给定 stop/observation 调用 question::from_saved；该错误
原样返回。之后 observation.matches 比较两项原完整绑定及所有原值字段；编码错误
按原 Json 转换返回，不匹配为 CorruptStore。仅当前序通过且原 capture.end 是
Terminated、其中 prior=Some 时才调用原 prior_matches；其错误原样传播，false
变成 CorruptStore。其它 End 形状或 prior=None 不增加检查。from_saved 成功时请求值已经构造；
其后观察或 prior 校验失败，不能声称该请求值从未构造。原早拒绝不调用构造器的
结论仅复用 #1011 的对应分支；这里不把 Unit 错误返回偷换成无构造或无副作用轨迹。

savedPrior 仅选择原 End 中的 prior；不以保存的 stop/observation 替换显式参数。
writer_closed 则只在原保存 End 是 Terminated 时，把其原 stop/observation 传给
validate_record。缺 capture 或其它 End 形状直接 Ok(false)，不先检查 Full；
验证失败原样返回，成功为 Ok(true)。该 Bool 只属于已解码值分类，不证明进程、
门、Writer 或文件描述符实际退出，更不授予 Continue、材料回收或配额释放权限。

同一 canonical Record/Full/Action/Observation/Work/Prior/End 与原父规格复用。
只保留 SourceSavedQuestionModel.Representable 的正常 Rust/共享借用输入域。
两套原 JSON 编码和共同摘要 provider 与 #1008/#1007 完全一致，允许错误、语言
失败及不返回；ContentDigest 的32字节值相等不表示已经计算哈希或材料真实完整。
V0 提供逐符号库、借用和具体采用；V1/V2 提供当前意图/耐久发送；V4/V5 提供
当前 World/授权/接受结果；V6 提供实际进程/门/FD/材料/Writer事实；V7 提供独立
认证来源。SQL/父事务/未知提交/耐久和生产/staging 一般等价仍开放。

## 原件组合与真实错误版

inputs.py 分别重建已公开的原 #1011 和 #1008 staging，只合并不相冲突的原文件
差异；若同一文件有两个不同且非原始的改动则拒绝。原 prior 的 completed/NoRequest
整段子句按 #1007 移到透明原 helper，保留原参数与晚检查位置；不删除业务比较。
原返回共享借用的闭包在固定 Aeneas 因 non-endable abstraction 失败；将其移到
命名返回借用函数也遇到 RegionsHierarchy 的 Unimplemented，两份失败保留。
仅把原纯 Option 选择展开为连续 if-let：先 capture.as_ref，再 end.as_ref，
原 Terminated/prior 模式及其后 prior_matches 调用、错误和返回位置保持。复用已有
SourceRowChargeView.optionAndThen 与共享借用模型核对该纯选择关系；不把整个业务
验证器或回调设为 opaque，也不声称已完成全部生产/staging 一般等价。
所有派生克隆、
完整比较、Full/Pause/profile 和请求工厂继续使用原透明生成体与已有一般证明。

allow-mismatched-prior 只删除原 prior 不匹配分支中的 CorruptStore 返回。
原请求构造、完整观察匹配、prior 调用及其错误传播仍保留；正常控制仍应接受，
false prior 被错误接受。正反版使用同一 Contract 和 WriterContract，不为错误版
改成弱规格；编译/环境/超时/导入失败不算性质拒绝。完整生成物不得手改。

## 完整形式开发入口

两套当前固定 Charon/Aeneas 实编译、抽取及准入已通过。112个完整类型均复用父
canonical 定义；169个函数/字典中167个原体逐字复用父，仅新增实际 validate_record
和 writer_closed。错误版只改变 validate_record 的一条 late prior 返回；全部类型、
接口及其它函数保持。视图只在完整定义后补原有逐符号 IO 绑定和两个名称解析
notation，不编辑原生成体、不做一般 alpha/空白归一化、不剪掉字段或比较。

26个外部接口全部对应既有合同，其中四项仅为原 Full/完整八元组的 JSON 编码和
摘要调用，两处摘要共用同一提供者；其余标准接口逐符号复用原父定义。原两段
Option Try 模型及有界 toStr 适配保持逐字选择。18个本包文件加两份原外部模型
来源，共20项精确输入；全部当前文件及十个固定 Lean 依赖纳入回读。

正确目标196文件/1062唯一审计，错误198/1068；其中父#1011正确191/1014及原保留
26/127完整保持。本包实际7文件/54审计（30条通用/顺序性质、18条见证、6条真实
错误版性质）须以最终当前版本完整检查；局部候选成功不代替完整双视图或公共验收。
两个原生正常控制及同一完整 Contract/WriterContract 的实际错误反例均须保留，
错误版的 positive helper 不得替换为另一套弱规格。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-drain-validation
for variant in original allow-mismatched-prior; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$drain_validation_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$drain_validation_inputs" \
    --output "$drain_validation_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入 JSON 六项为 correct/wrong/reads/reads_audit/retention/retention_audit。reads 与
reads_audit 使用父#1011完整正确项目及原服务审计，另两项使用原#945保留项目和原
服务审计。每版独立 .lake/build、共享重锁、固定版本与预算；所有失败和缓存保留。
检查须覆盖全部当前真实文件、完整审计、源/命令/生成物和父原件，再在新目录逐字
重建两套计划和所有输入，结果非 stale/partial且没有失败依赖才计完整开发通过。

## ning 原生交接

native.py 准备 original/adapted-original/allow-mismatched-prior/
adapted-allow-mismatched-prior 四套完整源码。保留父48个完整测试名字，新增四项：
正常/无 prior/实际 WorkerEnded prior 控制、原请求首错、完整观察及 Writer 早退、
保存 prior 不匹配拒绝。每套52项；正确两版预期52/52，错误两版51通过/1失败，
唯一拒绝须到达 `drain validation must reject mismatched saved prior`。

原 API/SQLite/JSON 夹具与受控 Host 前后比较完整 SQLite 行及 Sources；测试用
JSON 改值只构造解码输入，不修改原库或宣称已提交。新增两条 cfg(test) 桥仅转发
原 validate_record/writer_closed，不重写算法。受控 Host 不提供实际物理退出事实。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-drain-validation
for variant in original adapted-original allow-mismatched-prior adapted-allow-mismatched-prior; do
    case_root="$drain_validation_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立 source/target；实际编译后核对同 crate libtest 的 manifest/src/fresh、
完整源、精确输入及真实程序，逐项 --exact 执行全部52个完整名字，不只筛新增前缀。
两错误后另建 original、重新编译执行52项、恢复52/52，保留失败证据。退出101本身、
其它断言、环境缺失或超时不是性质反例。最终采用候选显式设置原已校验 Redis，
运行无参数 bash scripts/check-function.sh。普通/原生不在证明机执行。

公共 proof_native_suite、verify-stage1-protocol/verify-controller、根导入、公共
登记/冻结/覆盖只交主控同批统一验收及内容复用，不以旧公共入口通过代表本包通过。
保持 draft，由主控采用到 integration，不自行合并、改保护或新增实机任务。
证据位置和报告哈希按用户决定保留本地，不追加到公开 issue。
