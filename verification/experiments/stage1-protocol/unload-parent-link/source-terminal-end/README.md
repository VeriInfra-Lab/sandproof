# V3：原终止末态的构造与完整重放

本包接续 #1002。生产固定 b0d791ccc4d4e058b710dbf8f46766652b2853df，原
full::drain::terminal_end 直接透明，不跟随工程新头。生产只读；公共 Lake、根导入、
登记、冻结及覆盖只交候选。普通/原生验收交 ning，固定抽取与 Lean 在 msra/dsp5。

## 同一个完整合同

SourceTerminalEndModel.Contract 量化任意普通已解码 old、完整 StopCoverage 和
TerminatedCaptureObservation，规定原根等于 outcome。old 为 None 时构造 Terminated，
准确保留输入 stop/observation，prior=None；NeverReleased 保留完整 binding/seal_action；
WorkerEnded 保留完整原 observation，分别成为对应 PriorCaptureEnd。已有 Terminated
先比较完整 stop，再比较完整 observation；任一不符返回 Conflict，否则克隆整个已存
end，连同原 prior，而非用新请求重建 prior。

actual_contract、actual_full_contract 及一般构造/携带/replay/冲突引理连接原生成根；
actual_comparison_order 保留原 stop→observation→clone 的单子顺序。完整 stop 比较含
operation 的 sandbox/operation、epoch、attempt、result；观察比较复用父所有九字段及
嵌套完整身份/命令/Option/来源/网络/列表/摘要比较，含 helper gate device/inode。
clone_prior、clone_end 及完整原派生克隆引理覆盖所有构造子和任意载荷，不靠有限样例。

实际错误 Rust 仅删原 `|| **previous != observation`，保留 stop 比较。正常完整重放
两版接受，stop 错误两版拒绝；相同 stop 而任意不同完整 observation 时，正确 Conflict、
错误返回原 end，rejects_same_full_contract 否定同一个 Contract。inode_counterexample
单独给出实际第九字段不同的实例族。没有为错误版另换弱性质。

## 表示、复用及精确生成闭包

capture.py 保留父五处既有隔离，加登记 full/drain.rs。正确版该文件逐字不变，错误版
只删上述条件。所有自有 clone/equality/root 必须 Structured，无业务 opaque 或替代算法。
整个生产到既有 staging 的一般等价继续开放，不能称完整父调用/事务已经证明。

两版实际生成五份文件：Types.lean、Funs.lean、TypesExternal_Template.lean、
FunsExternal_Template.lean、translation.json。58 个完整类型体及两份类型文件逐字相同，
均匹配父真实视图。正确102函数/字典、错误50；共同50中仅原 root 改变。验证器限制为
删除观察比较及未用 binder、一层缩进；52 个剪除声明恰为观察相等依赖闭包与字典。
两版输出不是相同函数清单。

正确87完整函数体匹配父实际视图，15个其它声明包括原末态/先前末态/观察的派生克隆、
root 和四个显式 default ne 接线字典。字典与父同名但接线不同，故完整102/50声明分别
逐字插入独立 Actual/Wrong 命名空间，保留原 reducible/impl_def 和后端 linter 元数据；
不会只补root而误用另一版字典。共享完整类型及父相等/克隆引理保留，无新协议模型。

只绑定 dependencies.md 已有逐符号 String.clone/Option.clone、Bool.ne、String.eq、
Option.eq，复用 SourceRetireLibrary 和父具体库解释/证明；错误版只剩两项克隆外部符号。
外部模板只核对而不导入其 axiom。派生业务克隆全部透明。结论以正常分配、有效安全
借用及原库合同采用为条件；值克隆不证明指针身份，32字节摘要也不证明算过hash或材料真实。

父 #1002 正确142文件/779审计及其中 #945 原26/127完整保留。

## 固定抽取与完整开发入口

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-terminal-end
for variant in original stop-only-replay; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$terminal_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$terminal_inputs" \
    --output "$terminal_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

六个输入键恰为 correct、wrong、reads、reads_audit、retention、retention_audit；前两项为
本包完整捕获，中两项为 #1002 完整正确项目/原审计，后两项为 #945 原完整项目/审计。
重新构造全部4906源文件/执行位及六处staging，核对归档、固定二进制、完整命令/日志、
LLBC、映射、全部生成物及原服务证据。缺任一原件必须拒绝。

沿固定四核、80GiB抽取准入/64GiB运行停机线及共享重锁，保留缓存和全部失败。资源
失败不是性质反例。完整入口目标为正确145文件/804唯一审计、错误147/812。check.py
仅向未改父检查器提供物化器/变体/登记位置，保留完整独立构建、每个真实文件同服务
诊断、标准依赖审计、新目录逐字复现和十个固定 Lean 依赖核对。检查器自己持锁，外层
不重复持有；stale/partial、失败依赖、仍在检查、警告或错误均不计通过。本入口不签
公共/普通/原生验收；单次局部五文件/33审计也不代替本包完整形式入口。

## ning 原生交接

native.py 只准备四份完整副本，不编译/执行。继承 #1002 二十一项，新增三项：
None/NeverReleased/WorkerEnded 的精确 prior 携带；含非空 prior 的整值重放和五个 stop
叶字段逐一拒绝；实际 Terminated 观察的九字段逐一变更拒绝。先变第九字段 gate inode，
确保错误版唯一性质断言确实针对原遗漏字段。有限原生对照不代替上述一般 Lean 性质。

夹具用原 ready、sealed_before_end、actual_api_end、真实API/SQLite/JSON及原应用配
受控Host取得实际末态。私有 capture.end 只由 cfg(test) 投影/薄桥读取；drain 桥调用
原 terminal_end，full 桥只把返回值放入记录克隆用于观察，未写数据库。所有调用前后
比较完整SQLite行与 Sources 不变。输入克隆片段不冒充合法整账本或真实VM/Writer事实。
正确/错误夹具、桥、性质完全相同。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-terminal-end
for variant in original adapted-original stop-only-replay adapted-stop-only-replay; do
    case_root="$terminal_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

ning 各版独立 source/target，先真实编译并核对同crate libtest的 manifest/src/fresh、
源/输入及实际程序摘要，再用 --exact 逐项执行 registration-candidate.json 登记的
二十四个完整名字；只筛 source_terminal_end_ 会漏二十一项父控制。正确两版各24/24，
错误两版各23通过/1失败，唯一性质拒绝须到达 `terminal replay must compare all original observation fields`，
field=8。正常重放、五个stop字段和全部父控制仍须过。编译/准备/环境/超时/其它断言或
退出101本身不算反例。两错误后另建 original，重新编译并执行二十四项，恢复须24/24，
保留所有失败。最后采用候选显式设置原已校验Redis，执行无参数 bash scripts/check-function.sh。

公共 proof_native_suite、verify-stage1-protocol/verify-controller、原生/形式/根/覆盖由
主控同批统一验收和内容复用。此纯末态数据函数不验证当前授权、原来源选择/返回借用、
父调用/OLD World/同事务、未知提交或耐久接受；V3/V4/V5 接这些业务义务，V6 接实际
Stop/Writer退出/材料/VM，V7 接独立认证来源。返回/字段相等不能授予退休/执行许可，
没有 JSON/TLS 或完整系统结论。不新增KVM，不改变 #641 实机队列，不自行合并。
