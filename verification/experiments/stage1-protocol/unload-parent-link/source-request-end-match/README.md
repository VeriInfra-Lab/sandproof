# V3：原请求结束观察的全文匹配

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包连接原 CaptureRequestEndObservation::matches 与透明的
FullCaptureWorkRequest::binding_digest。形式开发、原生和公共验收分别记录；本包
范围说明与候选登记不能代替精确提交的完整验收回执。

接续 #1005 精确7d923214f875bb3bcab226644c277556dad1cd5b，生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df。不自动追随工程新头，公共配置、
根/登记/冻结/覆盖仅交主控采用候选，普通及原生验收交 ning。

## 同一完整合同

SourceRequestEndMatchModel.Contract 对任意原观察、完整 action、Full 请求、来源要求
和 worker 量化，正反视图使用同一 outcome：

1. 完整 action 不同返回 false；该分支不依赖编码或摘要调用的返回结果。
2. action 相同时，将完整原 Full 请求交给原 JSON 编码调用。编码错误先于其后
   worker/source/outcome 的比较传播；成功才将全部返回字节交给原 SHA 调用。
3. 原 full_work_binding、完整 worker、source.source 必须分别相等。NoRequest 标签
   通过；Returned 的状态码须在200..599，包括4xx/5xx。

不擅加 Full.valid、worker.valid、source 其它字段、2xx、所有权或授权条件。
相同32字节摘要也不能推出原 Full 请求相等，更不表示材料真实完整。

只在抽取隔离副本中把原 serde_json::to_vec(self) 及原 Sha256::digest(...).into()
分别移到仅转发完整参数的两处 helper；binding_digest 和 matches 保持透明。
Contracts 保留任意库返回结果，不把整个 Serde/SHA 或业务方法设为可信。
原 A6/A9/B4 精确库调用采用、正常共享借用、字节表示和 SHA 正常输入域仍由 V0
提供；组合定理本身不提供这些外部事实。

实际错误只把原完整 worker 比较换成两个原 process() getter 的比较，遗漏
job_digest。保持相同完整合同与其它原守卫，正常观察和进程字段拒绝仍保留；仅改
worker 摘要的观察由正确版拒绝而被错误版接受。
准备/编译/环境/超时/导入失败不能计为性质反例。

## 原件复用与完整开发入口

两版51个完整类型全部逐字匹配父同一套真实类型，不建协议副本。正确45个和错误
46个完整函数字典分别逐字物化，保留原reducible属性；公共函数只有matches改变，
删去完整worker比较依赖，加入原process getter及对应字典。五个原函数体逐字匹配父
视图；其余实际比较体直接连接父完整action/worker/source/digest证明，不拿有限见证
代替全文结论。原一般U16范围证明由reuse.py逐字选择，仅包装导入和命名空间；原件
也纳入输入闭包，不重新证明、导入axiom模板或手写生成体。

精确五个外部接口是原String.eq、Option.eq、RangeInclusive.contains及本包两个只转发
库调用的helper。前三项逐符号转发父同一解释；后两项显式量化库结果，实际业务
binding_digest保持透明。全局依赖合同和覆盖由主控协调，本包只交候选。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-request-end-match
for variant in original process-only-worker; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$request_end_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$request_end_inputs" \
    --output "$request_end_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

六个输入键是correct、wrong、reads、reads_audit、retention、retention_audit，分别指本包
两份完整抽取、#1005原完整正确项目/原审计和#945原完整26文件/127审计。
核对全部4906源码与执行位、七处staging、固定工具/命令/日志、LLBC/映射/五份生成物。
保留父164文件/867审计，正确完整目标170/885，错误172/890；局部8文件/23审计不能
替代完整入口。两套独立构建、每份实际文件的同服务诊断、唯一审计、新目录全部计划/
文件逐字重建及十个固定Lean依赖核对必须完整、非stale/partial且无失败依赖/警告。
共享重锁、四核、80GiB准入/64GiB运行停机线及失败/缓存保留不变；检查器自己持锁。

## 精确提供方与未闭合连接

| 提供方 | 最小需求 | 本包结果的限制 |
|---|---|---|
| V0/A6/B4 JSON | 固定实际 Serialize 字典与全部字段/标签/次序，原完整 Full 对象的一次 to_vec，原输出或错误 | 任意库结果下的调用参数与拒绝顺序不等于真实 JSON 字节已证明 |
| V0/A9 SHA | 固定 sha2/digest/generic-array 实例、完整原切片、有效共享借用及长度严格小于2^61的正常完成合同 | 不给摘要单射、材料真实性、授权或实际进程身份 |
| V1/V2 | 当前完整意图、原工作绑定与耐久发送的来源 | 普通字段匹配不推出当前授权或业务接受 |
| V4/V5 | 本次原记录、当前 World、完整事务/提交和恢复历史 | 值相等不推出同一物理连接、事务或耐久 |
| V6 | 原 worker 的独立归属、真实退出、请求释放门及受保护终局记录 | NoRequest/Returned 的值形状通过不产生这些事实 |
| V7 | 独立认证上下文的真实来源 | JSON、普通数据构造与 matches 都不发放权限 |

以上依赖保持具名开放；本包不会为了推进而各造另一份协议模型，也不将完整 handler、
收件事务、编码器或业务方法纳入可信边界。

## ning 原生交接

保留父31项，再加三个原生候选，完整34项复用原 API/SQLite/JSON 和受控 Host 夹具，直接调用原公开 matches 与
binding_digest，不加业务测试桥：正常及完整状态码边界；完整 action/binding/source
变更；worker 的PID/ticks/boot/job_digest逐项变更。候选调用前后比较完整 SQLite 行
和 Sources。受控 Host 不产生真实 VM、请求、HTTP 响应或 worker 退出事实。

native.py只准备完整副本，不编译或执行。ning可按registration-candidate.json接手：

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-request-end-match
for variant in original adapted-original process-only-worker adapted-process-only-worker; do
    case_root="$request_end_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立source/target，实际编译后核对同crate libtest的manifest/src/fresh、源码/输入及
实际程序摘要，再逐项--exact执行全部34个完整名字。只筛source_request_end_会漏父31项。
正确两版各34/34；错误两版各33通过/1失败，唯一拒绝必须到达
`request end matching must retain complete original worker binding`，field=3。
正常控制、状态码和进程字段拒绝须保留。两错误后另建original，重新编译执行全部34项，
恢复34/34，保留失败；准备/编译/环境/超时/其它断言或退出101本身不是反例。
最终采用候选显式设置原已校验Redis，执行无参数bash scripts/check-function.sh。
公共proof_native_suite、verify-stage1-protocol/verify-controller、根/冻结/覆盖由主控
同批统一验收与内容复用，不能跑旧根就报告本包验收通过。

NoRequest 是普通已解码标签，不能据其通过断言请求没有发出；实际结果记录的来源、
原 worker 真实退出、未释放/已释放门、提交和耐久都仍是独立义务。
原整个生产/staging一般等价、V1/V2当前意图/耐久发送、V3来源/返回借用/完整父调用、
V4/V5当前World/授权/事务/未知提交/耐久、V6真实进程/材料/VM/Writer和V7独立认证
继续开放。没有完整JSON/TLS、任意manifest文件集或系统结论；不自行合并。
