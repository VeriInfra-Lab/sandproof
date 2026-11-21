# V3：原 worker 绑定值的构造与形状校验

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包接续 #1004 精确3544c987b1b84d2e52a80ea680410917ea199189，生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df。只读原 capture/launch.rs 的
CaptureWorkerBinding::new 和其直接调用的 valid；不自动追随工程头。公共配置、
根/登记/冻结/覆盖仅交候选，普通和原生验收交 ning。

## 同一完整合同

SourceWorkerGuardModel.ValidContract 量化任意可表示原 worker 值：valid 当且仅当
原 process 的 PID/ticks 都为正，boot_id UTF-8 字节长1..128，所有字节均为33..126。
ConstructorContract 量化任意原 process 与完整32字节 job_digest：正常表示下，形状
不符返回 UnsupportedInput，符合则准确返回整个原 process 和原 digest。摘要不另加
非零或哈希正确条件；值往返不证明计算过hash。

原 Rust 先构造局部 value 再检查 valid。outcome 规定返回结果，不声称失败前没有
构造局部值。successful_payload/fields 覆盖完整 process 的三个字段及完整 digest，
digest_does_not_change_valid 则精确说明原分类器没有比较 digest。

实际错误只删原 process.start_time_ticks > 0。两版正常输入仍接受；零 ticks 输入被
错误 valid 和原错误构造器接受，分别违反与正确版相同的 ValidContract 和
ConstructorContract。wrong_valid_fixed_ticks 是两套实际生成函数的等式辅助引理，
没有另造弱性质；错误构造器仍返回其原零 ticks 数据，未用改过的过程替代返回值。

## 表示与原输入闭包

Representable 仅采用父 StringRepresentable 的正常 Rust 分配/有效共享借用范围，不
预设合法 worker、来源归属、实际进程存在、保持启动门或释放权限。复用父
SourceProcessIdentity/CaptureWorkerBinding/ContentDigest/JournalError 等19个完全相同
真实类型，只插入原 Unit 回调类型；不各造另一套协议记录。

两版20个完整类型体相同；原守卫删除令回调源行52变51，验证器仅允许这一条生成
注释的准确变化。7个完整函数/字典中仅 valid 改变，new、ASCII原语、回调和字典
不变；两版全部7个原体分别逐字物化，保留原 reducible 属性。valid、new、回调和
实际 ASCII 原语必须透明，无自有业务 opaque。

精确五个标准接口为 String.deref/is_empty/len、str.bytes、Bytes.all。两视图逐符号
转发父同一套解释，不导入外部 axiom 模板；字节回调和 allBytes 引理直接复用父实际
证明。String.len/is_empty 精确调用采用保持父 V0 待办，不以模型通过声称原标准库
实现已证明。整个生产到既有 staging 的一般等价也仍开放。

capture.py 保留原五处隔离，仅登记第六处 launch.rs；正确版该文件逐字不变，错误版
只删一个原守卫。父 #1004 的158实际文件/850审计及其中 #945 原26/127完整保留。

## 固定抽取与完整开发入口

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-worker-guard
for variant in original allow-zero-start-ticks; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$worker_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$worker_inputs" \
    --output "$worker_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

六个输入键恰为 correct、wrong、reads、reads_audit、retention、retention_audit；分别是
本包两套完整捕获、#1004 原完整正确项目/原审计、#945 原完整项目/审计。核对全部
4906源码/执行位、六处 staging、固定归档和二进制、完整命令/日志、LLBC、映射及五份
生成物，缺一拒绝。共享重锁、四核和80GiB准入/64GiB运行停机线不变，保留失败和缓存。

完整目标正确164文件/867唯一审计、错误166/874；局部8文件/24审计不能替代本包
完整入口。check.py 复用未改父检查器的两套独立构建、每个实际文件的同服务诊断、
公理依赖审计、新目录逐字复现及十个固定 Lean 依赖核对。检查器自己持锁，不重复
外层持有；错误/警告、失败依赖、stale/partial或仍在检查均不通过。资源、编译、准入
或导入失败不算性质反例。本入口不签普通、原生或公共整套验收。

## ning 原生交接

native.py 只准备四份完整副本，不编译/执行。保留父28条，新增三条：完整原 process/
digest保留且不同摘要仍可通过形状校验；PID与ASCII字节边界；唯一零启动 ticks 拒绝。
直接调用原公开 new/valid/两个 getter，不加测试桥。非法私有字段值通过原 Serde 的
值解码取得，有限对照不表示完整 JSON 字节/TLS 链已证。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-worker-guard
for variant in original adapted-original allow-zero-start-ticks adapted-allow-zero-start-ticks; do
    case_root="$worker_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

ning 各版独立 source/target，真实编译后核对同 crate libtest 的 manifest/src/fresh、
源/输入及实际程序摘要，逐项 --exact 执行 registration-candidate.json 登记的31个
完整名字；只筛 source_worker_guard_ 会漏父28项。正确两版各31/31；错误两版各30通过/
1失败，唯一性质拒绝须到达 `worker constructor must reject zero original start ticks`。
父控制、正常构造及PID/ASCII控制仍须过。两错误后另建 original，重新编译执行31项，
恢复31/31，保留所有失败；准备、编译、环境、超时、其它断言或退出101本身不是反例。

原 API/SQLite/JSON夹具调用前后比较完整 SQLite 行及 Sources 不变。受控 Host/克隆
片段不产生实际 worker/VM/门或 Writer 事实。最终采用候选显式设置原已校验 Redis，
执行无参数 bash scripts/check-function.sh。公共 proof_native_suite、verify-stage1-protocol/
verify-controller、根/冻结/覆盖由主控同批统一验收与内容复用，不能跑旧公共根便报
本包已验收。

V1/V2 当前意图和耐久发送、V3来源/返回借用/完整父调用、V4/V5当前World/授权/同事务/
未知提交/耐久、V6实际进程归属/启动门/材料/VM/Writer、V7独立认证保持开放。
FullCaptureLaunchPermit 的原 confirmed-commit 产生与单次消费不在本包；构造数据、
getter或valid的true不产生该许可。没有任意manifest文件集、完整JSON/网络或系统
结论，不新增KVM、不改#641队列、不自行合并。
