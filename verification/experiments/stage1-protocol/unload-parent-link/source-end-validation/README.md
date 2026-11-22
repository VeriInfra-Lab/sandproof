# V3：原保存 End 的编码预算和分类校验

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包连接原 end::validate_record 和 end::material_writer_closed 的已解码值规格。固定父#1012
809da6344b7d6478650508b521e82310a397eb3c，生产固定 b0d791cc。两版固定抽取及局部对应、首错、边界和错误版反例已检查，完整双视图检查仍待完成；
普通/原生及公共整套验收交 ning/主控；不是物理 Writer 关闭或资源释放结论。

原 validator 缺 capture 先 CorruptStore；缺 End 则成功且不编码；存在 End 则先按
原对象编码，保留 Json 错误，超过16384字节先 CorruptStore，恰等于上限允许继续。
之后 NeverReleased/WorkerEnded 复用原完整 prior 规格，Terminated 复用#1012的原
完整 drain 校验。匹配错误原样传播，false 为 CorruptStore，不增加 worker.valid、
授权、当前 World、物理来源或耐久接受前提。

material_writer_closed 在缺 capture 时直接 true；否则先验证 End，再按原 End
分类。Terminated 会再次调用原 drain::writer_closed，保留这次重复验证及其可能的
失败/效果；不假定两次有状态编码相同。其它形状返回 End 是否存在。该 Bool 是原值
分类，不能推出实际工作结束、最后 FD 关闭、物理清理、Continue 或材料/配额释放。

仅隔离原 to_vec(end) 的精确调用，将原 WorkerEnded 的晚 completed/NoRequest 整段
子句移到透明 helper。固定 Aeneas 首次在 NeverReleased 原短路表达式的上下文合并
失败，移动完整原表达式后该分支通过，下一次失败定位到 WorkerEnded 原短路表达式；
两次失败保留。原空超限分支错误版（包括长度先绑定为标量的尝试）也被固定后端
拒绝，不计性质反例；保留失败并撤回该长度提升。错误版改为仅把原比较阈值误写为
END_RECORD_BYTES的两倍，正确版保留原表达式，继续使用同一16384字节合同。
透明 helper 保留原参数、位置和
全部检查/短路/错误顺序，不复制另一套算法。原父staging和正常借用/
分配条件保持，第三方 JSON/Vec 依赖逐符号复用 A6/B4，不把整个 Serde 或业务函数
设为可信。固定工具、完整原件、实际文件与审计通过前，不计本包证明完成。

真实错误只将编码 End 上限由16384误写为32768；原编码、CorruptStore拒绝分支、错误传播和全部
分支匹配保留。正反版须使用同一完整 Contract/WriterContract，正常控制保持，实际
超限错误必须到达反例；编译、导入、环境和超时不算性质拒绝。

原生候选保留父52个完整测试名字并新增四项，共56项：全部原 End 形状及缺 capture
分类、原 completed+NoRequest 条件、Terminated 原请求首错传播、真实超限 End 拒绝。
使用原 API/SQLite/JSON 与受控 Host，前后比较完整 SQLite 行及 Sources；新增两条
cfg(test) 桥仅转发原 end validator 和分类器。编译/执行留给 ning。

超限反例从未验证的解码值开始：同时加长原保存 worker 与 WorkerEnded 观察中的
boot_id，保持完整匹配；该 worker 特意不满足构造器形状条件。它不制造 launch permit，
不宣称绕过外层 worker 校验或实际受理。这里证明本值函数自己的容量拒绝，不用全局
有效性假设排掉应拒绝的可表示输入。32字节值相等仍不表示算过哈希或材料真实完整。

当前原生输入预期 original/adapted-original各56/56，错误两版各55通过/1失败，
唯一拒绝须到达 `saved End validation must reject oversized encoded End`。两错误后
另建original重编译执行全部56项、恢复56/56，保留失败。最终采用候选显式设置原
已校验 Redis，运行无参数 bash scripts/check-function.sh；普通/原生不在证明机执行。

V0 提供具体 JSON/Vec/字典、正常 Rust/共享借用/分配采用；V1/V2 提供当前意图与
耐久发送；V4/V5 提供当前 World/授权/事务/未知提交与耐久；V6 提供物理进程/门/
FD/材料/VM/Writer事实；V7 提供独立认证。receipt/set_end、完整 coverage/input、
当前 question/terminal_key/successful_stop 和生产/staging 一般等价保持开放。

公共根/登记/冻结/覆盖和 proof_native_suite/verify-stage1-protocol/verify-controller
交主控同批统一验收及内容复用，不并发改公共文件、不跑旧根声称本包通过、不自行
合并或新增实机任务。证据位置和报告哈希按用户决定保留本地，不追加公开 issue。

## 完整形式开发入口

两版实际生成体包含相同112个完整 canonical 类型及175个完整函数/字典，其中169个
原体逐字复用#1012。仅新增原常量、三个透明helper、validate_record和分类器。两版
只有真实上限比较不同；全部类型、外部接口和其它函数不变。27个接口中26项逐字
继承父提供者，新增的仅为原保存End编码调用。符号、属性、完整命令、工具二进制、
LLBC、五份生成物、4906源文件及执行位和12处staging均由检查器核对。

同一Contract/WriterContract不因正反版改变。正确目标201文件/1115唯一审计，错误
目标203/1122；保留父196/1062与原retention26/127。本包7个实际文件共60条审计：
38条对应/首错性质、15条合法及16384/16385边界见证、7条真实错误版性质。完整
检查还须逐一诊断所有实际文件并在新目录重建全部计划/文件；局部通过不代表此门槛。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-validation
for variant in original allow-oversized-end; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$end_validation_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$end_validation_inputs" \
    --output "$end_validation_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入JSON为correct/wrong/reads/reads_audit/retention/retention_audit六项。reads及其
审计须是#1012完整正确项目；retention及其审计为原保留26/127。每版独立.lake/build，
共享重锁、固定版本/预算，不清缓存或覆盖失败记录。全部实际文件、完整审计和依赖
须无错误/警告、无失败依赖、无stale/partial后才计完整形式开发通过。

## ning 原生交接

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-validation
for variant in original adapted-original allow-oversized-end adapted-allow-oversized-end; do
    case_root="$end_validation_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立source/target。实际编译后核对同crate libtest的manifest/src/fresh、完整源
及真实程序，按registration-candidate.json逐项--exact执行全部56个完整名字。
正确两版56/56，错误两版55通过/1失败，必须到达上文原超限断言；退出101本身、
其它断言、编译/环境缺失或超时不算性质反例。两错误之后另建original、重新编译
并执行全部56项，恢复56/56。按既有普通角色和固定离线环境验收；本包不新增依赖。
最终采用候选显式设置已校验Redis，运行无参数bash scripts/check-function.sh。
公共原生/形式入口、覆盖、根及冻结只交主控采用候选，未登记前不能凭旧根宣称本包
通过公共整套。只在完整结果和实际证据回读后更新当前进度，不自行合并。
