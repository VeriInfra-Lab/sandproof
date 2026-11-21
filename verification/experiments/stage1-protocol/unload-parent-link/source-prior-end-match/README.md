# V3：原先前终局与当前捕获记录匹配

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包通过透明守卫适配连接原 full::drain::prior_matches 的完整已解码值合同。
接续 #1006 精确cfacf73efd7cc39c779dcf2ccb7b72206593697b，生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df，不自动跟随工程新头。
形式开发、原生和公共验收分别记录，候选登记不代替精确回执。

## 统一性质与复用

使用父同一套 Record、Prior、worker、action、Full 类型，以及 #1006 的完整
请求结束匹配和 Full 绑定规格，不另建协议模型或把这些业务方法改为可信函数。

- capture 缺失先返回 CorruptStore。
- NeverReleased 要求保存 worker 和 completed 均不存在，才执行完整 Full 编码/
  绑定；然后比较完整摘要和完整可选 action。action 为 None 本身允许，不擅加必须
  Some。形状短路先于编码，编码错误先于后面的可选 action 比较。
- WorkerEnded 先要求 action，再要求 worker，然后调用原请求结束匹配；即使该匹配
  成功，completed 为 Some 且观察为 NoRequest 时仍须返回 false。没有 completed
  时 NoRequest 可通过，普通 Returned 保留父完整接受条件。

实际错误候选只删除 WorkerEnded 成功匹配后的 completed/NoRequest 拒绝门。
完整同一 Contract 用于两版；正常和早期错误保留，反例必须实际到达错误接受，不能
用编译、准入、导入失败或超时冒充性质拒绝。

旧 exit-observations 包证明恢复候选的三种匹配；source-terminal-end 包证明原
terminal_end 的完整携带与重放。它们均不提供这个 prior_matches 父调用结论，已有
适用证据保留。本包复用 #1006 的 JSON/SHA 两处原调用隔离，保持原 callee 透明。

## 真实生成体与完整开发入口

原未适配函数在固定后端的matches!上下文合流失败，初次标签helper仍失败；完整原
completed/NoRequest布尔子式分离为透明helper后可抽取。completed值在原后置短路位置
求值并作为Bool实参，原matches!表达式和判断不改；helper自己的完整生成体与原
caller都参与证明，没有把业务守卫改成外部条件。两份失败、原LLBC和源码保留，
不是业务反例；整个生产到既有staging的一般等价仍是独立义务。

两版109个完整类型全部逐字对应父原件，51/50个完整函数/字典各自逐字物化并保留
原reducible属性。47个正文逐字对应父视图；新增原需求getter、WorkAction字典、
完整后置守卫与caller。错误只剪掉后置门及其不再可达helper；其余共同正文只变caller。
生成器另将NeverReleased分支的一个临时变量b3改名val2，验证器仅在内存允许这对
准确alpha重命名，不改原生成体。完整callee和Full绑定证明直接复用#1006，不重复实现。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-prior-end-match
for variant in original allow-completed-no-request; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$prior_end_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$prior_end_inputs" \
    --output "$prior_end_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

六个输入键correct、wrong、reads、reads_audit、retention、retention_audit分别指本包
完整两份捕获、#1006完整正确项目/原审计和#945原26文件/127审计。全部4906源码/
执行位、八处staging、固定工具/完整命令/日志/LLBC/映射/五份生成物缺一拒绝。
保留父170文件/885审计，完整正确目标175/905、错误177/912；局部7文件/27审计不能
代替完整入口。两套独立构建、每份实际文件的同服务诊断、唯一审计、新目录全部计划/
文件逐字重建和十个固定Lean依赖核对必须完整、非stale/partial且无失败依赖/警告。
共享重锁、四核、80GiB准入/64GiB运行停机线及失败/缓存保留不变，检查器自己持锁。

## 依赖与验收边界

原 String/Option 比较、as_ref 和可表示共享借用按原逐符号合同采用。as_deref 复用
父 SourceMetadataView.optionalDeref 的完整字典调用模型；本次 Box<WorkAction>
具体调用采用须另交 V0 核对，不能由旧文本借用实例自动声称已采用。未落实部分
继续具名开放，不将整个 Option、Serde、handler 或事务纳入可信。

三个原生候选保留父34项，共37项：两类 prior 缺失 capture；NeverReleased 的
worker/completed 缺省、完整摘要和可选 action；WorkerEnded 的 action/worker 首错、
正常及唯一 completed/NoRequest 拒绝。只在临时 cfg(test) 副本解码 prior 值并调用原
私有方法，不重写其算法或修改数据库。原 API/SQLite 及受控 Host 夹具前后核对完整
SQLite 行和 Sources；准备、编译和执行分别记录，原生/普通验收交 ning。

## ning 原生交接

native.py只准备四份完整副本，不编译或执行。可按registration-candidate.json接手：

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-prior-end-match
for variant in original adapted-original allow-completed-no-request adapted-allow-completed-no-request; do
    case_root="$prior_end_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立source/target，真实编译后核对同crate libtest的manifest/src/fresh、源/输入及
实际程序摘要，再逐项--exact运行全部37个完整名字；只筛source_prior_end_会漏父34项。
正确两版各37/37；错误两版各36通过/1失败，唯一性质拒绝须到达
`prior end matching must reject completed capture with NoRequest`。正常两类prior、
缺省/字段控制及无completed时NoRequest允许仍须过。两错误后另建original，重新编译
执行37项，恢复37/37并保留失败；准备/编译/环境/超时/其它断言或退出101本身不是反例。
临时测试桥仅解码prior值后调用原私有方法；适配版透明helper按原参数/短路位置、
精确beta内联和固定rustfmt回读，不能以有限对照认领整个生产/staging一般等价。
最终采用候选显式设置原已校验Redis，执行无参数bash scripts/check-function.sh；
公共proof_native_suite、verify-stage1-protocol/verify-controller、根/冻结/覆盖由主控
同批统一验收和内容复用，不能跑旧根报告本包验收通过。

NoRequest 是已解码标签，字段匹配并不证明请求未发出、真实结果或 worker 已退出。
V0 精确库采用、生产/staging 一般等价、V1/V2当前意图/耐久发送、V3来源/返回借用/
完整父调用、V4/V5当前 World/授权/事务/未知提交/耐久、V6真实进程/材料/VM/Writer
及 V7 独立认证继续开放。没有完整 JSON/TLS、任意 manifest 文件集或系统结论。
公共配置/根/登记/冻结/覆盖仅交候选，保留失败和缓存，不自行合并。
