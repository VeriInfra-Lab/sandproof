# V3：原续行状态分类门

本包接续 #1001。生产固定 b0d791ccc4d4e058b710dbf8f46766652b2853df，不跟随工程头。
原 full::end::continuation_allowed 与原 FnOnce 回调直接透明；生产只读，所有公共登记、
根导入与覆盖仅交主控候选。普通/原生验收交 ning，固定抽取与 Lean 在 msra/dsp5。

## 同一完整合同及边界

SourceContinuationGateModel.Contract 量化任意普通已解码完整记录，规定原函数等于
allowed：无 capture 或无 end 为 false；NeverReleased、WorkerEnded 为 true；
Terminated 为 false。模型复用父全部真实类型，五项通用分支引理覆盖任意嵌套载荷。
actual_contract、actual_full_contract 连接原生成根；另外分别给出缺项、两个正常状态
及 Terminated 拒绝的原根结论。

原函数只分类 end 的构造子。它没有验证 end 的载荷、完整当前身份、World、源行、
授权、Writer 退出、封印或数据库提交；true 不能授予续行/执行许可，WorkerEnded 标签
也不能证明实际进程或描述符已退出。完整外层 continuation::validate_receipt 的其它
合取门继续开放，本片不把它们改成可信业务。返回 false 同样不能证明其它资源不存在。

实际错误 Rust 只在原 matches! 模式中增加 Terminated。它改变原 FnOnce 回调的该一支
返回值，根函数和字典体仍相同。错误视图必须插入原错误回调、字典与根三者，防止因
命名空间回退误用正确字典。normal_control 给两种正常状态的共同接受；同一已记录
Terminated 值上，原根 false、错误根 true，rejects_same_full_contract 否定同一个 Contract。

## 表示、依赖与精确生成闭包

只复用已有 A6 逐符号合同。as_ref 绑定现有 SourceRetireLibrary.optionRef；is_some_and
复用父具体定义，None 不调用回调并返回 false，Some 恰调用给定 FnOnce 一次并保留其
失败、效果或发散。实际原回调透明，空 Unit 状态和全部构造子分支已分别展开，不将
自定义有副作用的 Drop 或任意共享可变堆纳入这一值解释。正常有效共享借用与原库采用
条件保持；模型可计算不证明 Rust 标准库已经验证。

capture.py 保留父五处既有隔离，加登记 full/end.rs；正确版该文件逐字不变，错误版
只改变上述一条模式。原 root 与原回调都必须 Structured，所有自有业务/协议/核心均
不得 opaque。没有替代算法、whole handler/Serde/收件事务假设或新 JSON/hash 端口。
整个生产到既有隔离的一般等价仍未闭合，未扩张为完整父调用结论。

两版实际生成94类型、三函数/字典，完整类型与外部接口逐字相同；三者只有回调的
Terminated 分支变化。93个共享完整类型体逐字匹配父实际视图，一个真实 Unit closure
别名及原三声明从输出逐字插入，保留原 reducible 属性。视图保留后端本就使用的
unusedVariables=false 元数据，以保留原未读 c 参数；不改生成体或关闭内核检查。

Bool 根没有外部类型模板，实际输出恰为 Types.lean、Funs.lean、
FunsExternal_Template.lean 和 translation.json 四份。验证器要求该精确清单及缺失的
TypesExternal_Template，不能伪造第五份。外部函数模板只读取核对，不导入其中 axiom。
父 #1001 正确139文件/767审计及其中 #945 原26/127完整保留。

## 固定抽取与完整开发入口

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-continuation-gate
for variant in original accept-terminated; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$gate_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$gate_inputs" \
    --output "$gate_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

六个输入键恰为 correct、wrong、reads、reads_audit、retention、retention_audit；前两项
为本包完整捕获，中两项为 #1001 完整正确项目/原审计，后两项为 #945 原完整项目/审计。
重新构造4906份源文件、六处 staging，核对原归档、文件/执行位、固定二进制、完整命令/
日志、LLBC、映射及全部生成物。缺原件、摘要、日志或原服务响应必须拒绝。

沿固定四核、80 GiB抽取准入/64 GiB运行停机线及共享重锁；保留全部缓存与失败，资源
失败不能作为性质拒绝。完整入口目标为正确142文件/779审计、错误144/783。check.py
仅向未改父完整检查器提供物化器/变体/登记位置，保留全部角色、锁、完整构建、逐真实
文件同服务诊断、全审计、新目录逐字复现和固定依赖检查。两版独立 .lake/build，缓存
只复制。检查器自己持锁，外层不能重复持同一把。stale/partial、失败依赖、仍在检查、
警告或错误均不计通过。本入口仅签本包形式开发检查，不签公共/普通/原生验收。

## ning 原生交接

native.py 只准备四份完整副本，不编译或运行。保留 #1001 十八项断言，新增三项：缺
capture/end 的值片段拒绝；实际 API 保存 NeverReleased 和 WorkerEnded 后接受；原应用
完成受控 Stop/drain 后实际保存 Terminated，原分类门拒绝。使用原 ready、
sealed_before_end、actual_api_end 和原 v3::application、真实 SQLite/JSON及受控 Host，
不会产生实际 VM/进程退出证据。原类型的 Serde 标签只用于夹具前置条件，未另建协议模型。

cfg(test) 薄桥调用原分类函数；其它两桥仅投影真实 end 数据供夹具核对，以及在克隆上
清空私有 end。所有分类调用前后核对完整 SQLite 行不变。None 值片段不是合法整账本的
替代模型。正确/错误版本的夹具、测试桥及性质完全相同。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-continuation-gate
for variant in original adapted-original accept-terminated adapted-accept-terminated; do
    case_root="$gate_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

四版独立 source/target，先真实编译并核对同 crate libtest 的 manifest/src/fresh、源/输入/
实际程序摘要，再用 --exact 逐项执行 registration-candidate.json 的二十一个完整名字。
只筛 source_continuation_gate_ 会漏十八项父控制。正确两版各21/21；错误两版各20通过/
1失败，唯一性质拒绝须到达 terminated saved end must not pass the original continuation
classifier。正常状态、缺项及全部父控制仍须通过。准备/编译/环境/超时/其它断言或退出
101本身均不计反例。两错误之后独立重建 original，重新编译执行二十一项，恢复须21/21。
保留全部失败，最终采用候选显式设置原已校验 Redis，跑无参数 bash scripts/check-function.sh。

公共 proof_native_suite、verify-stage1-protocol/verify-controller、普通/原生/形式/根/覆盖
由主控统一验收与内容复用，不重复完整套件、不新增 KVM。V3 的完整 end 校验/原父调用/
来源借用，V4/V5 的当前 World/授权/合法行/OLD World/事务/未知提交/耐久，V6 的真实
Writer/材料/VM 与 V7 的独立认证继续开放。没有 JSON/TLS 或完整系统结论。
