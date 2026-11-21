# V3：原 Full 请求与原来源绑定守卫

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包以原 full::validate::full_binding 为入口，复用 #1004 的完整 Full/Pause
合法性规格和证明、父 canonical Record/Full/请求类型与完整字段比较。
接续 #1008 精确6b59a6eed377e6bcf411c772c3d622ba96700950，生产固定
b0d791ccc4d4e058b710dbf8f46766652b2853df。独立抽取/形式开发、原生和公共验收
分别记录；本包规格和输入准备不表示已经证明或验收。

## 同一完整合同

SourceFullBindingModel.Contract 对任意保存记录与 Full 值量化。唯一附加输入域是父
FullRepresentable，表示普通有效 Rust 值/共享借用，不要求 FullAllowed；非法但可表示
的 Full 仍由合同覆盖。非 Unload 分支的提前返回另对任意 Full 保留，不把合法性检查
挪到操作标签之前。

原顺序为：

1. 取原 SourceWork::request，操作不是 Unload 则先返回 Conflict。
2. 调用原完整 Full.valid；不合法返回 Conflict，不另造替代算法或外置业务布尔值。
3. 比较完整 operation binding、来源代、capture、完整来源 work digest、完整节点、
   完整 source_resource 和完整 source process，全部符合才返回单位成功值。

完整 operation binding 比较继续包含版本、完整节点、session、完整键和 expectation。
32字节摘要的值比较不表示计算过哈希。协议/进程/ASCII/profile/路径/长度/超时条件
来自同一个父 FullAllowed，正反两版不更换合法性或较弱绑定规格。

原守卫不读 capture 是否存在、原 native generation/current VM、当前 World、认证或
数据库；这些字段的任意值不被悄悄加进接受条件。成功只表示本次普通值匹配，不产生
捕获、Stop、发送、观察、发布、释放或持久接受资格。

实际错误只删除原 source_work_binding 与 work.requirement.work_binding 的比较。
Full.valid 和其余六个绑定比较保持。源摘要变化仍是合法的 Full 值，正常对照必须
保留，错误接受由同一个完整 Contract 反驳；编译/环境/超时/导入失败不是性质反例。

## 实现和外部边界

实际入口及被调用的 Full/Pause 方法保持透明。重新使用固定 Charon/Aeneas，不手改
生成体，不导入外部 axiom 模板，不把 Full.valid、原 Getter 或完整守卫变为可信端口。
14个逐符号标准接口和 ASCII 原语/回调/profile 证据复用父的原范围，实际调用采用和
普通 Rust/共享借用输入域继续交 V0；String.len/is_empty 的旧采用缺口不得迁记为完成。

普通值开始的本包不提供任意文件集、真实 JSON/TLS、来源实际存在、当前 World、完整
父调用、事务/未知提交/耐久或整个系统结论。原终止 question 的 inode/identity/assemble
连接另列；本守卫返回单位值，本身没有构造观察请求。

## 完整开发入口与精确闭包

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-full-binding
for variant in original omit-source-work-binding; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$full_binding_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$full_binding_inputs" \
    --output "$full_binding_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入为 correct、wrong、reads、reads_audit、retention、retention_audit；后四项是
#1008 完整正确项目/原审计和 #945 原完整保留项目/原审计。全部父181文件/931审计与
原保留26/127保持；本包正确186文件/958唯一审计，错误188/963。新增局部7文件/32
审计不代替完整双视图。各版独立 .lake/build，固定工具/共享重锁/预算不变，保留缓存
和失败，检查全部实际文件、完整审计、精确模板、父原件、十个依赖及新目录逐字重建。

两版111个完整类型全部逐字对应父同一 canonical 定义；正确51/错误49个完整函数和
字典，正确46个原体逐字对应父。新增原 getter、原守卫和三个实际使用 ne 的完整字典，
ne 直接用原 trait default 加已有一般 eq 定理，不造弱字段模型。错误仅改变根并剪掉
不可达摘要比较/字典；核对精确后续三个局部变量改名和缩进，不做一般 alpha 归一化。

原 Unload 的 capture 模式名遮蔽两个完整 callee 名；仅在其原定义后加相同全名的局部
notation，原函数体/参数/生成物不改。原 Full 字面量复用父已审计的有界 toStr 适配，
其原模板也列入精确输入闭包：17个本包文件加1个父模板，共18项。缺失该适配的初版
虽然类型通过，依赖审计拒绝了默认 native decide 假设；原失败保留，不加入白名单或
放宽内核，修正仅逐字采用原父适配。类型通过不能冒充完整证明验收。

## ning 原生交接

native.py 只准备四份完整源码，不编译或执行。保留父40项，再加原 Unload 六类绑定
拒绝及 capture/native 未检查字段控制、非 Unload/非法 Full 拒绝，以及单独的来源摘要
遗漏反例。真实 API/SQLite/JSON 和受控 Host 仅提供原组件夹具，前后核对完整 SQLite
行和 Sources，不作为真实 VM/进程/Writer/授权事实。临时 cfg(test) 桥只调用原守卫。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-full-binding
for variant in original adapted-original omit-source-work-binding adapted-omit-source-work-binding; do
    case_root="$full_binding_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立 source/target。实际编译后核对同 crate libtest 的 manifest/src/fresh、全部
源输入与真实程序，再逐项 --exact 执行全部43个完整名字；只筛 source_full_binding_
会漏父40项。正确两版各43/43；错误两版各42通过/1失败，唯一拒绝必须到达
`full binding must retain original source work digest`。正常、其它六字段和父控制保留。
两错误后另建 original、重编译执行43项、恢复43/43并保留失败。准备/编译/环境/超时/
其它断言或退出101本身不算性质拒绝。

最终采用候选显式设置原已校验 Redis，运行无参数 bash scripts/check-function.sh。
公共 proof_native_suite、verify-stage1-protocol/verify-controller、根/登记/冻结/覆盖
只交主控采用候选并同批统一验收，不能跑旧根报告本包通过，不自行合并。
