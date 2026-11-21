# 已选来源记录的完整退休请求校验

[证明工作流](../../../../../docs/proof-workflow.md) ·
[V3 详细计划](../../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包固定生产 `b0d791ccc4d4e058b710dbf8f46766652b2853df`，
树 `7ca91afd9cd79601d562f3c636066d192fc25fc5`，接续 #635 / #998。
生产、公共配置、冻结、根导入及覆盖保持只读；公共接线只提供候选。
完整来源类型和请求构造复用父包，不另建协议模型。

## 完整合同和首错

`SourceRetirementQuestionModel.Contract` 对正确与错误根使用相同性质：任意已解码
记录和完整退休请求上，结果等于有序的 `selected`。实际原请求构造先复用
`SourceMaterialChargeProof.request_contract`；无 capture 返回 UnresolvedAction，
先于任何请求不匹配。存在 capture 时，实际派生比较必须当且仅当整个请求相等才接受，
不匹配返回 Conflict，匹配返回同一完整记录。

七字段为 sandbox、generation、full、source、seal_action、worker、manifest_digest。
`request_eq` 复用父包的完整 Full/来源、命令、网络/驱动列表、Option 和摘要比较；
没有以有限样例代替任意嵌套值。`actual_tail_contract` 连接原生成体，
`actual_full_tail_contract` 直接满足同一个 Contract。

本片从已经选中的记录开始。`actual_full_tail_contract` 对任意 question.source 成立，
因为这一段确实没有再次检查 source；不能从请求相等反推出来源选择正确。
相同记录的正常请求在两版接受；只改变代数的请求被正确根拒绝、被实际 sandbox-only
错误根接受，后者由同一完整 Contract 否定。

全部字段和返回值只描述普通已解码值。在现有逐符号克隆/比较合同及正常分配范围下
使用这些结论，不覆盖任意 panic/unwind/OOM/abort 或别名可变堆。返回完整值不能产生
授权、持久接受、物理释放或退休许可；也不把值相等当作物理地址或原事务同一性。

## 原函数片段与未完成查找

固定原工具对真实私有 `retirement::find` 的预检得到 Charon 0 / Aeneas 2：
原 slice::Iter.find 的 higher-ranked/free lifetime constraint 不受支持
（slice/iter/macros.rs 338–341，TypesAnalysis.ml 968）。只展开原 find/next 后仍失败；
宽泛库展开另在 Charon 变换后的 trait-clause 类型检查失败。原始输入和失败保留，
这些失败不计作业务反例，不更换工具、关闭类型检查或隐藏整个查找。

capture.py 只将原 find 中完整的 request/不匹配/返回段原样移入
check_question_record。原选择前缀保持不动，原父随后调用这段一次；没有实现替代扫描。
继承的五处 staging 仍是父包原库调用适配。本片没有闭合整个生产到隔离副本的一般等价，
也没有将未抽取成功的整个父调用标为已证。

V0/主控待提供或协调最小原库接口：同一有效共享切片和原谓词下的 slice::Iter.find
首次匹配、原返回借用及其来源。V3 的完整 SourceWork 比较已有提供者，但其实际查找
调用、原 selected record 与 question.source、原父返回借用仍需连接。原始缺行首错、
任意重复行下的选择及查找后不再调用的结论仍未在本片形式闭合。下列真实原生用例
只提供有限输入对照，不能替代这条一般证明。

## 固定抽取与完整开发检查

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-retirement-question
for variant in original sandbox-only-request; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$question_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$question_inputs" \
    --output "$question_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

沿原四核、80 GiB 抽取准入及 64 GiB 运行停机线和共享重锁。输入 JSON 恰含六键：
correct、wrong 为本包完整捕获，reads、reads_audit 为 #998 完整正确项目及原实际文件
审计，retention、retention_audit 为 #945 原完整项目及原审计。后者全部包含于父项目。
缺少源码、原服务响应、审计、日志或摘要必须拒绝。

输入检查重新构造 4906 份原源码及五处 staging，核对原归档、工具命令/日志、LLBC、
映射及全部生成物；全部自有业务透明。110 个共享类型和 122 个共享函数的完整声明
逐字出现在父实际命名空间视图中，父 133 文件/752 审计保留。新增 question 类型、
完整请求派生比较/字典及实际根均从原输出逐字插入，不导入带 axiom 的外部模板。

错误 Rust 仅将原完整请求比较换成 sandbox 比较。111 个类型声明全部相同；后端只将
未改的 PolicyDigest/ManifestDigest 两个别名移到 ResultDigest 之后，检查器只允许
这一处整块移动。外部类型模板相同。实际函数由 125 变为 60：59 个共同声明只有根体
改变，新增原 sandbox getter，完整请求比较的 66 个依赖/字典因不可达而被剪除。
检查器验证确切比较块、原依赖闭包及所有共同函数，不声称函数清单逐字相同。

正确项目为 136 文件/761 唯一审计，错误项目为 138 文件/765 审计。check.py 仅向未改的
父完整检查器提供本包物化器、两变体和登记位置，保留其全部角色/重锁/完整构建/逐实际
文件服务诊断/依赖审计/新目录逐字复现/固定依赖检查。两版独立 .lake/build；原缓存只可
复制到独立目录，不能清除。检查器自己持一次共享锁，外层不能再持同一把锁。
缺项、stale/partial、仍在检查、失败依赖或警告/错误不能计为通过。
本入口始终为开发检查，不签发公共、普通或原生验收。

## ning 原生与普通验收

native.py 只准备四份完整副本和同一十五条断言，不执行测试。原始两版只改变原 find
中的同一比较及测试桥，隔离两版保留原父适配/片段分离并恢复原 lib.rs；桥仅在
cfg(test) 下调用原完整 find。另记格式化前字节，再用固定 rustfmt 规范，不改捕获原件。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-retirement-question
for variant in original adapted-original sandbox-only-request adapted-sandbox-only-request; do
    case_root="$question_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

每版用独立 source/target，先实际编译并核对同 crate libtest 的 manifest/src/fresh、
源码/输入与真实程序摘要，再逐项执行 registration-candidate.json 的十五个完整名字
并用 --exact。只筛 source_question_ 会遗漏十一条父控制。

- #998 原十一条清槽/计费控制全部保留。当前四条沿原 settled_terminal、真实 SQLite/
  JSON 和原受控 Host：正常全文请求与七字段遗漏；缺行先于请求错误、选中后无捕获
  先于不匹配；首个匹配无捕获不能跳到后一条；同 binding 改完整参数的行不能代替
  完整 SourceWork。正常返回还检查实际所选原行引用，所有用例核对原完整数据库行不变。
- 正确两版各应 15/15；错误两版各应 14 通过/1 失败，唯一性质拒绝必须到达
  `retirement question must match all seven original request fields`。sandbox 错绑定、
  正常请求和全部首错/查找/父控制仍须通过。原生有限对照不代替一般查找或借用证明。
- 编译、准备、环境、超时、其它断言或退出 101 本身都不是性质反例。两个错误之后
  独立重建 original，重新编译并执行十五条，恢复必须 15/15，所有失败保留。
- 最终采用候选显式设置原已校验 Redis，运行无参数 bash scripts/check-function.sh。
  公共 proof_native_suite、verify-stage1-protocol/verify-controller、原生/形式/根导入/
  覆盖由主控同批统一验收与内容复用，不各自重复全套或新增 KVM。

原库的逐符号采用及整个隔离一般等价、原查找/借用及父调用、V5 的合法行/OLD World/
同事务/提交未知/耐久、V6 的实际材料/VM 与 V7 的独立认证继续具名开放。此处没有
JSON 字节解码或网络链证明，也不将父已有 JSON/SHA 结果扩张到完整 Serde、端点或系统。
