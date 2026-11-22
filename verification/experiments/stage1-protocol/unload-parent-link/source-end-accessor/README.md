# V3：公开 End 回执与 continuation 值接口

[证明工作流](../../../../../docs/proof-workflow.md) ·
[父完整回执](../source-end-receipt/README.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包精确依赖 #1014 的 29a97e6441167ccbf0abd8f27655303c63e9925d，生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df，不自动跟随工程头。实际检查结果在
#635/本包PR维护；下面的库存和命令不是成功回执。不修改生产、公共配置/根/登记/
冻结/覆盖；public-registration.patch 和 registration-candidate.json 供主控采用。

## 八个实际方法与同一完整合同

实际入口为 SourceWorkRecord.capture_work_end_receipt，以及
OriginalContinueWorkRequest.from_ended_source、identity、full、source、end_receipt、
network_policy、file_policy。公开 getter 直接调用原 receipt，复用父同一完整
SourceEndReceiptModel.Contract 和 Representation，保留所有结果、首错及端口失败/
发散。缺 capture/End 直接 None；有值先完整验证，Terminated 验证后 None；独立
末次 Full/End 编码错误、完整 typed end 与两个独立 binding 均保留。

构造器接受任意已解码参数；一般定理量化全部完整 identity、Full、source、
end_receipt、network_policy 和 file_policy。六个实际读取方法及构造后全部读取
保持这六个完整值。不是 key、session、digest 或有限见证代替完整相等。不校验
来源真实性、编码、授权或实际资源，也不产生 OriginalContinuePermit。

正确版117个完整类型/191个函数；116个类型/183个父函数逐字复用已检查父定义，
新增仅原六字段类型及八份完整方法体。与父共享 canonical 类型，不另造协议模型。
严格保留父29项逐符号接口；新增 opaque、遗漏根或实际体均拒绝。不导入带 axiom
的外部模板，不手改生成体。具体29个接口、每次调用环境、正常借用/分配及生产/
staging一般连接保留父提供者义务；不把整个 JSON、receipt 或事务设为可信。

两个真实错误版分别仅将原 NoRequest 回执标为 Returned，及仅交换构造器
end_receipt.work_binding/end_binding 两字段。前者复用父完整实际错误 receipt，
后者保留其它所有完整字段；两者分别被同一完整 getter/ConstructorContract 反驳。
其余190个完整函数及全部117类型/29接口保持相同。保留正常 Returned、NeverReleased、
独立编码错误、完整六字段、不同双 binding 及等 binding 控制见证。

Charon start-from 使用实际可解析 Type::method，另保留定义模块 inherent 匹配根。
首轮工具均成功但生成根未入场、第二轮路径解析失败的原记录保留；均不计性质拒绝。
错误版必须实际编译、抽取并到达同一性质反例；编译/工具/导入失败或超时不算反例。

## 完整形式开发入口

正确版213实际文件/1199唯一审计；回执错误217/1210（含父原两个错误文件/5审计）；
构造错误215/1205。保留完整父207/1175和原 retention26/127。十个新增实际文件共
36项审计：13条一般对应、11条值见证、两个错误版各6条。局部成功不代替完整三视图。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-accessor
bash scripts/python-entry.sh "$package/capture.py" --variant original \
    --output "$accessor_original" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
for variant in receipt-no-request-as-returned swap-receipt-bindings; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$accessor_wrong_store/$variant-01" \
        --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$accessor_inputs" \
    --output "$accessor_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入JSON固定六项 correct/wrong/reads/reads_audit/retention/retention_audit。
correct为本次正确捕获，wrong为上述两种固定命名错误捕获的父目录；reads及其审计
使用父#1014的完整原 original-project/original-audit。该原批次的同级
no-request-as-returned-project/no-request-as-returned-audit 须保留，供严格回读原
错误209/1180，不复制成功标签伪造新凭据。retention及审计是原26/127。

validation.py 回读全部4906文件/执行位、13处staging、固定二进制/完整命令/日志、
LLBC、映射与五份实际生成物。prepare.py 从完整原体生成独立视图；check.py 复用
原完整检查器，逐文件同服务诊断、全部审计、全部内容输入、十个固定依赖及新目录
完整计划/文件逐字复现。每版独立 .lake/build，固定工具/预算/共享重锁，保留失败及
缓存。零错误/警告、无 stale/partial/超时/失败依赖才计完整形式开发检查通过。

## ning 原生交接

native.py 只准备六套完整输入，不编译或测试。原 source-end-receipt/native.py
完整保留所有60断言、真实API/SQLite/JSON与受控Host夹具/桥；本包只追加一条同一
六字段断言。调用真实公开 getter、构造器和全部六个读取方法；等/不同双 binding
和两种完整网络策略共用同一断言，前后核对完整SQLite行与 Sources。替换的 digest
是显式已解码输入，不声称真实编码所得。全部来源文件/执行位和规则/夹具内容登记。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-accessor
for variant in original adapted-original \
    receipt-no-request-as-returned adapted-receipt-no-request-as-returned \
    swap-receipt-bindings adapted-swap-receipt-bindings; do
    case_root="$accessor_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立source/target。按 registration-candidate.json 全部61个完整测试名逐项
--exact 执行，不只跑新增前缀。编译时核对同crate libtest的manifest/src/fresh、
源码及真实程序/业务库，不能换测试程序沿用错误库。正确两版各61/61；回执错误
两版各60通过/1失败，唯一断言须到达 `saved End receipt must preserve NoRequest tag`；
构造错误两版各60通过/1失败，唯一断言须到达
`continuation request must preserve all six complete fields`。
退出101、编译/工具/导入失败、超时或别的断言均不算性质拒绝。

每个错误视图后另用独立original/source/target重新编译执行全部61项恢复61/61，
共四次恢复；保留失败证据。最终采用候选显式设置已校验真实Redis，执行无参数
bash scripts/check-function.sh。普通/原生交ning；公共 proof_native_suite、
verify-stage1-protocol/verify-controller、根/登记/冻结/覆盖交主控同批统一验收/
内容复用，不以旧公共根通过报告本包通过，不自行合并。

## 待证范围与提供者

这里只从已解码值开始。构造/相等没有授权、持久接受或实际资源结论；32字节
ContentDigest值不是已计算哈希或材料真实完整证明。binding_digest、实际父读取
调用/当前World、SQL/同父事务/未知提交/耐久、OriginalContinuePermit及真实派发
继续由V4/V5和相应连接承担；V1/V2提供当前意图/耐久发送，V6提供实际API/进程/
FD/Writer/材料/VM事实，V7提供独立认证来源。真实JSON字节到值、TLS、持久收件和
完整系统结论仍开放。证据位置和报告哈希按用户决定保留本地。
