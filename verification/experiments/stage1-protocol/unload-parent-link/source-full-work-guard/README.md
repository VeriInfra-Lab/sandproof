# V3：原 Full/Pause 请求的完整值校验

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包接续 #1003 的精确父提交 385d2460c878659348a12665995467e75fff232e。
生产固定 b0d791ccc4d4e058b710dbf8f46766652b2853df，只读原 capture/work.rs 的
PauseWorkRequest::valid、FullCaptureWorkRequest::valid 及其真实依赖。公共 Lake、
根导入、登记、冻结和覆盖仅交候选；原生和普通验收交 ning。

## 同一个完整合同

SourceFullWorkGuardModel.PauseContract、FullContract 量化任意满足表示条件的原生成
请求值，规定实际根返回 ok(decide Allowed)。不使用有限样例代替一般性质，正反视图
共享同一个 FullContract。

PauseAllowed 精确对应协议4、完整节点相等、key.deployment 与 node.deployment_id
相等、ExpectedSession 为 Version、来源资源种类为 Vm、正 PID/启动 ticks，以及 boot_id
的 UTF-8 字节长度为1..128且每字节为33..126。Version 的编号不额外要求大于零。
原方法没有调用完整 NodeIdentity::validate，也不检查 session、owner、资源 ID、来源
work 摘要或实际进程归属，证明不增加这些条件。

FullAllowed 保留原 Pause 校验和 VmCompatibility::validate，随后要求恰好一块可写盘，
drive_id 为 rootfs、jail_relative_path 为 rootfs.ext4；state/memory/disk 三个长度均为正
且可转为 i64；API 超时1000..300000，worker 超时不小于 API 且不超过300000。
profile 校验包括非零 snapshot_format、盘数范围、路径和两种盘标识分别唯一。四个
32字节 profile 摘要在此不被核验；值存在不表示计算过hash或材料真实完整。

Pause/Full 的 actual_contract、complete_contract 连接全部原生成分支。normal_accepted
给出正常合法见证。错误 Rust 只删 Full 原 state_bytes > 0，保留其 i64 转换及所有其它
检查。错误体 evaluated 引理保留正常控制，并对原零状态长度值返回 true；
violates_same_full_contract 由同一个完整 FullContract 导出矛盾，不改用弱规格。

## 表示、历史证明与库采用

PauseRepresentable 仅要求 boot_id 可由既有字符串模型表示。FullRepresentable 再要求
原 profile 所有路径及 drive_id 可表示；不预先假设工作有效、材料存在或被授权。
它们是普通分配/安全借用和字符串大小的表示范围，不是整个收件事务的可信假设。

两版26个完整类型相同，24个逐字对应父实际类型，只增加原 ManifestError 和原 Unit
回调载体。两版29个完整函数/字典中仅 Full 根改变；错误视图复用其余28个逐字相同
函数，单独插入完整错误根。字典保留原混合 reducible 属性；原 ASCII 原语、回调、
路径和 profile 循环均透明。14个标准外部接口精确固定，不接受自有业务 opaque。

历史 Manifest 中20个相同完整类型和19个相同完整函数逐一核对。reuse.py 只变更五份
原通用 Loop/Path/Profile 证明的导入、命名空间和对应符号的限定名，逻辑定义与证明
步骤保持；新实例使用父同一套真实类型，不导入会冲突的另一套 Manifest 生成类型。
原一般 profile/path 定理与原循环见证分别保留，不能以局部循环或正常 manifest 见证
声称任意文件集、完整 manifest、真实 JSON 字节或完整 V3 已完成。

11个现有标准值接口定义只按列出的 provider 名称实例化。字符串/Vec比较、字符串
借用和字节迭代采用既有 A6/逐符号合同；RangeInclusive 复用父实际通用字典及范围
引理，u64→i64 复用原 SourceRecordBoundsView.toSigned。String.len/is_empty 的两项
精确调用接线沿用 inbox-config-validation 提案，仍须由 V0 确认原库采用；不因其值
模型通过便宣称标准库实现已证。外部 axiom 模板只核对，不导入。未新增 JSON/hash 端口。

capture.py 保留原五处隔离，登记第六处 capture/work.rs；正确版此文件逐字不变。
错误版只删上述原守卫。整个生产到既有 staging 的一般等价仍开放，不能称完整父调用
或同一事务已证明。父 #1003 的145实际文件/804审计及其中 #945 的26/127原件完整保留。

## 固定抽取与完整开发入口

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-full-work-guard
for variant in original allow-empty-state; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$full_guard_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$full_guard_inputs" \
    --output "$full_guard_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

六个输入键恰为 correct、wrong、reads、reads_audit、retention、retention_audit；前两项为
本包完整捕获，中两项为 #1003 精确父正确项目/原审计，后两项为 #945 原完整项目/审计。
验证全部4906源码及执行位、六处 staging、固定归档/二进制、完整命令/日志、LLBC、映射
与五份生成物；重新实例化所有标准绑定和原通用证明，缺原件拒绝。

沿固定四核、80GiB抽取准入/64GiB运行停机线和共享重锁，保留失败与缓存。完整入口
目标正确158文件/850唯一审计，错误160/854。check.py 复用未改父检查器的两套独立
构建、全部实际文件的同服务诊断、公理依赖审计、新目录逐字复现及十个固定 Lean 依赖
核对。检查器自己持锁，不在外层重复持有。错误/警告、失败依赖、stale/partial 或仍在
检查均不通过；资源、准入、编译和导入失败不算性质反例。局部15文件/50审计也不
代替本包完整形式入口。本入口不签普通、原生或公共整套验收。

## ning 原生交接

native.py 只准备四份完整副本，不编译/执行。继承 #1003 的24项，新增四项：正常及
原本未检查的普通字段仍可通过而不产生权限；逐项 Pause 字段与 ASCII 边界；合法
profile 下的盘形状、三个 signed 长度和超时边界；唯一的零 state_bytes 拒绝断言。
长度测试以外的三个新增控制在两错误版仍须通过。

夹具通过原 ready、observed_for_full、full_request 和真实 API/SQLite/JSON 建立请求，
所有调用前后核对完整 SQLite 行及 Sources 不变。新增测试直接调用原公开 valid，
没有增加测试桥；继承原父薄桥。受控 Host、输入克隆及未校验的摘要值不产生实际
文件/VM/Writer 事实，也不表示合法整账本或实际授权。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-full-work-guard
for variant in original adapted-original allow-empty-state adapted-allow-empty-state; do
    case_root="$full_guard_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

ning 各版使用独立 source/target，真实编译后核对同 crate libtest 的 manifest/src/fresh、
源/输入及实际程序摘要，再逐项 --exact 执行 registration-candidate.json 登记的28个
完整名字；只筛 full_work_guard_ 会漏24条父控制。正确两版各28/28；错误两版各27通过/
1失败，唯一性质拒绝须到达 `full work validity must reject zero state bytes`。准备、编译、
环境、超时、其它断言或退出101本身不算反例。两错误后另建 original，重新编译执行
28项，恢复28/28，保留全部失败。最终采用候选显式设置原已校验 Redis，执行无参数
bash scripts/check-function.sh。

公共 proof_native_suite、verify-stage1-protocol/verify-controller、根/冻结/覆盖由主控同批
统一验收和内容复用；未登记本包时运行旧公共根不能报告本包已验收。V3 的来源选择、
返回借用与完整父调用，V4/V5 的当前 World/意图/事务/提交未知/耐久，V6 的实际来源、
材料/封印/VM/Writer 退出，V7 的独立认证上下文分别保持开放。valid 的 true 不授权，
不表示持久接受或实际资源存在；没有完整 JSON/TLS 或系统结论，不新增 KVM、不改
#641 实机队列，也不自行合并。
