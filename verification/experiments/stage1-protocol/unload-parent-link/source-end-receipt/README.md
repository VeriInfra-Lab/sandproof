# V3：原保存 End 到完整值回执

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包连接原 full/end.rs 的 receipt 已解码值入口，固定依赖#1013作者
49aa7267c3d7de156f48005405bf9b710a9d434a，生产固定b0d791cc，不自动跟随工程头。
复用#1013完整End校验、#1001完整seal_end及同一canonical类型，不修改生产。
实际检查结果及未完成验收在#635/本包PR维护；下面的库存和命令不是成功回执。

## 原行为和同一完整合同

缺capture或End均直接Ok(None)，不先验证。否则先完整validate_record；其错误
原样先返回。Terminated必须验证成功后才返回None。NeverReleased仅在保存action
存在时调用原seal_end；WorkerEnded先调用seal_end，再准确保留NoRequest/Returned。
随后再次绑定完整Full、编码原End并交给共享SHA提供者，准确保留work_binding、
end_binding和完整typed end三个字段。正常值、首错和真实错误版都用同一Contract。

前序验证成功不替代末次编码结果。Ports.validation复用父完整规格；末次Full和End
编码有独立回复，SHA仍共用同一提供者。实际视图为原Full.binding_digest的末次调用
选择receiptPhase环境；原业务函数体不变，不复制其算法。直接的两个End编码调用
分别隔离到原typed helper，helper仅调用同一serde_json::to_vec。此处保留值模型中的
实际调用组合；不是任意可变外部状态/堆对象的完整证明。具体调用环境、正常借用、
分配和生产/staging一般连接仍须提供者采用。

首遍16KiB预算不能自动给末次可失败编码的长度或成功保证。SHA具体正常输入域须
逐次落实；这里保留完整输入/32字节结果，不声称计算过数学SHA或提供碰撞单射。
Representable沿用父Full字符串/配置表示域，不偷换成FullAllowed、权限或业务状态
有效性。回执/None/NoRequest/Returned不证明持久接受、真实API调用、工作/Writer/FD
退出、材料释放或当前World资格；JSON字节到值/TLS/物理事实与完整系统另列。

## 实际抽取与精确复用

两版116个完整类型、183个完整函数/字典。113个类型和178个原体逐字复用父201文件
闭包；新增原CaptureWorkEnd、CaptureWorkEndReceipt和闭包类型别名，以及原receipt、
两个闭包函数/字典、实际Option.map/transpose。既有CaptureSealEnd与seal_end、
ContentDigest.as_bytes及source_work::digest复用同一原体，不重复定义协议。

Option.map/transpose是本次真实透明标准函数，回调也是原seal_end调用；不增加这两项
可信模型或导入带axiom模板。29个外部接口为父27项、原末次End编码及已登记的唯一
生产SHA原语。完整4906源文件/执行位、12处staging、固定二进制/完整命令/日志、LLBC、
映射和全部五份生成物由validation.py严格回读；不做一般alpha/空白归一化。

首轮Charon/Aeneas均成功，但冗余保留根不符合原单保留根准入器，未计准入成功。
下一轮仅保留原material_writer_closed根，仍传递保留完整End校验，并显式include两项
标准方法。首份命令/工具/生成物和失败记录保留，不重标为性质拒绝。

no-request-as-returned仅将原NoRequest回执标签写成Returned，其余检查、编码、SHA、
字段和错误传播不变。工具会把纯构造提升到后段；检查器仅接受该确切实际差异。
一般actual_worker_result辅助方程保留任意成功输入、完整绑定和原调用，未另造弱合同。
正常Returned和两种NeverReleased控制保持；原NoRequest见证必须否定同一Contract。
编译、环境、导入失败或超时不算性质反例。

## 完整形式开发入口

正确库存207实际文件/1175唯一审计，错误209/1180，保留父#1013的201/1115及原
retention26/127。本包8个真实文件共65条审计：38条对应/首错性质、22条值见证、
5条实际错误版性质。先用固定工具重新生成，再检查所有真实文件和内核依赖，最后
在新目录逐字重建全部计划/文件；局部成功不代替完整双视图或公共验收。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-receipt
for variant in original no-request-as-returned; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$receipt_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$receipt_inputs" \
    --output "$receipt_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入JSON是correct/wrong/reads/reads_audit/retention/retention_audit六项；reads及其
审计使用#1013完整正确项目，retention及其审计使用原保留26/127。每版独立.lake/build，
共享重锁、固定版本/预算，保留失败及缓存。完整回执须零错误/警告、无失败依赖、
无stale/partial，并回读21项精确输入及十个固定Lean依赖后才计完整形式开发通过。

## ning 原生交接

native.py只准备四套完整输入，不编译或测试。保留父56项，新增4项：缺capture/End
及原形状控制、完整Seal坐标、Terminated先验后None、NoRequest标签精确性。原
API/SQLite/JSON及受控Host夹具前后核对完整SQLite行/Sources。新cfg(test)桥只导出
原End的真实编码字节；断言直接调用原公开capture_work_end_receipt，不替代算法。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-receipt
for variant in original adapted-original no-request-as-returned adapted-no-request-as-returned; do
    case_root="$receipt_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立source/target。编译后核对同crate libtest的manifest/src/fresh、源码和真实
程序，逐项--exact执行registration-candidate.json全部60个完整名字；不能只筛新增
前缀。正确两版60/60，错误两版59通过/1失败，唯一拒绝须到达
`saved End receipt must preserve NoRequest tag`。退出101或其它断言不算性质拒绝。
两错误后独立original重新编译执行全部60项、恢复60/60，保留失败证据。最终采用
候选显式设置原已校验Redis，运行无参数bash scripts/check-function.sh。

普通/原生交ning；公共proof_native_suite、verify-stage1-protocol/verify-controller、
根/登记/冻结/覆盖由主控同批统一验收/内容复用，本包只交候选。不以旧公共根通过
代表本包通过，不自行合并或新增实机任务。公开getter和完整父调用、set_end错误后
状态、reserved_record_bytes、当前World/SQL/事务/未知提交/耐久及V1/V2/V6/V7责任
仍开放。证据位置和报告哈希按用户决定保留本地，不追加公开issue。
