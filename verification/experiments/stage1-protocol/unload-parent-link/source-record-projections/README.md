# V3：原来源记录的完整读取投影

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包从已解码值开始，交付状态与验收回执在 #635 / 本包 PR 维护。下面的库存、
登记和命令不是公共或原生验收回执。固定依赖 #1014 的精确提交
29a97e6441167ccbf0abd8f27655303c63e9925d，生产 b0d791cc；不自动追随新头。
最新 integration faa5b502 的本来源协议路径不变，Core 等其它输入变化另列，
不以本固定证明冒称新版完整通过。仅新增独占包，不改生产或公共配置。

本包对应性质量化任意完整已解码 SourceWorkRecord，给出九个原只读方法的精确字段结果：
source_sandbox、capture_has_end、full_capture、capture_worker、capture_action、
capture_request_end、captured_manifest、captured_material_bytes、capture_operation。
无 capture 的缺失传播、三类 End 的 observation 选择和完整 Full/action/worker 都保留。
有 capture 不等于已有 End；材料测得字节不等于预留额度。两个真实错误版分别混淆
这两组事实，并由同一完整 Contract 检查；不得只为反例改用弱性质。

复用父 canonical 类型、原透明标准方法和逐符号依赖，不重复 #1018 的 End 回执或
六字段 continuation 构造证明。本包读取值不证明授权、已持久接受、材料实际存在
或物理观察真实。完整父 World/SQL/事务/未知提交/耐久、V1/V2 意图发送、V6 物理
事实及 V7 独立认证、真实 JSON/TLS/持久收件及系统结论保持原提供者责任。
普通和原生验收交 ning；公共登记、冻结、根与覆盖只交候选。私有证据位置及报告
哈希保留本地，不追加公开 issue。

## 给值层调用方的接口边界

读取规格从同一份已解码记录开始，完整 Full、worker、action、observation 和 digest
值全部返回，不按 key、session 或摘要替代整个值。无 capture 时八个捕获相关结果
分别为 false 或 None，sandbox 仍取原来源；已有 End 也只有 WorkerEnded 产生
request-end observation，NeverReleased 和 Terminated 都不产生该观察值。

本包一般目标覆盖所有类型正确的记录，读取不调用 validate_record；因此读取成功
不能作为 FullAllowed 或当前 World 资格。共享借用采用原固定工具的值表示，不把
投影等式扩大成物理地址、认证来源或耐久事实。后续调用方仍须连接其原记录来源、
同一事务和原独立校验。实际完成状态以本包 PR / #635 为准。

## 实际抽取和完整入口

当前三版固定 Charon/Aeneas 编译与抽取均已完成并通过本包严格准入。117个完整类型、
195个函数中，116类型及183个完整父函数逐字复用；新增一个原Unit闭包别名和12个
函数/字典，含九个原方法、原谓词/字典及透明Option.is_some_and。Option.is_some使用
固定Aeneas.Std既有内置模型，父原体已有同一调用，无额外库接口。外部仍精确29项。
首轮两工具均0但准入器错误期待内置函数生成声明，原失败保留；修正该期待后实际
重新捕获三版，没有换工具、过滤真实方法或迁记失败。

End错误版只将原谓词返回true，保留is_some_and及原类型/字典，另有一条明确的类型
源码位置注释106:25-106:56变为106:25-106:65。全部117个完整类型体相同；材料错误版
连原类型文件都逐字相同。各版195个函数只允许一个完整函数的确切原式改变，其余
194个完整原体相同；不采用一般alpha/空白归一化。全部4906源文件及执行位、13处
staging、固定二进制/完整命令/日志/LLBC/映射/五份生成文件都由validation.py回读。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-record-projections
for variant in original capture-presence-as-ended reserved-as-material-bytes; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$projection_capture/$variant-01" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$projection_inputs" \
    --output "$projection_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入JSON仍为correct/wrong/reads/reads_audit/retention/retention_audit六项：correct是完整
正确捕获目录；wrong是同时包含两个固定“变体名-01”捕获目录的只读父目录，不是单一
错误版。reads使用#1014原207文件/1175审计的完整正确项目，retention保留26/127。
新目录中的逐字全计划/文件复现、全部真实文件同服务完整诊断、全部审计及固定依赖
核对均为必需；局部Lake通过不能冒充完整三视图或公共验收。完整终态及未通过/未运行项在 #635 / 本包 PR 维护。

## ning 原生交接

native.py只准备六套完整原始/适配输入，不编译或测试。保留父60项，新增3项分别覆盖
完整字段/End观察选择、capture与End存在区别、记录材料字节与预留容量区别。原API、
SQLite、JSON及受控Host夹具前后比较全部原行和Sources，不新增产品算法或桥。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-record-projections
for variant in original adapted-original capture-presence-as-ended adapted-capture-presence-as-ended reserved-as-material-bytes adapted-reserved-as-material-bytes; do
    case_root="$projection_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

逐版检查同crate真实业务库/libtest的manifest/src/fresh与源码绑定，逐项--exact执行
registration-candidate.json全部63个完整测试名，不仅运行新增前缀。正确两版各63/63；
两个错误各原始/适配两版均62通过/1失败，唯一拒绝须分别到达
`capture presence must not stand for saved End presence` 或
`observed material bytes must not be replaced with reserved capacity`。每个错误后独立
original源码/target重新编译并恢复全部63/63，共四次；不复用错误库或只换测试二进制。
编译、工具、导入失败、超时、其它断言或退出101本身不算性质拒绝。普通Linux完整
入口需固定已校验真实Redis；共享proof_native_suite及公共verify入口由ning/主控同批
统一验收与严格内容复用。私有证据和报告哈希保持本地，未运行项不标通过。

## 定理、表示与提供方

SourceRecordProjectionsProof 有16项审计：九个原方法的精确全值等式、同一完整Contract，
以及End/WorkerEnded/Full/材料字节的一般充要关系、缺capture全部结果和忽略字段关系。
Values有15项审计，复用父合法Full及原验证入口；保留三类End、任意optional u64、
预留量无关性，以及不合法Full仍可读取的见证。两个Negative各5项审计，均量化实际
错误根并反驳正例同一Contract；缺失控制与已有End/字节偶合控制不是主性质的前提。

Types只加入原Unit闭包别名，全部协议/来源类型保持父canonical定义；View/两个Wrong
通过工具从实际生成体选择全部12个新完整声明。三个标准调用的名字仅绑定到父
SourceEndReceiptActual中的Option.as_ref、Try.branch、FromResidual.from_residual；它们
原样复用父定义，不复制另一套算法、不引入公理或整个业务opaque。成功结果采用项目
原ITree结果语义及已证明的Result.ok_injective，不将它当作普通有限枚举强行消去。

固定工具的普通共享借用、Option内置/逐符号模型和生产/staging连接条件沿用V0父登记。
原记录来源及同一World/SQL/事务/未知提交/耐久仍由V4/V5及父调用方提供；V1/V2提供
当前意图与耐久发送，V6提供物理资源/材料/进程/Writer事实，V7提供独立认证来源。
没有新增完整Serde/HTTP/收件事务可信假设。getter不做validate_record，读取字段或
得到Some/true均不表示已授权、已持久接受或物理事实成立。真实JSON字节到值、TLS、
持久收件和完整系统保持范围外。最终公共采用仍须把本包全部目标接入真实共享入口，
不能只跑旧根报告新版通过。
