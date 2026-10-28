# 原收件保留历史的版本关系


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**最终统一候选的抽取与整套原生、Lean 验收尚待完成**；局部结果只按原回执的精确提交保留，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [原执行谱系](../inbox-lineage/README.md)

旧证明最初固定 `a9c9967889c4cf298e4378c7106948018835fc64`，父证明为 #718 的
`a636f26ec26d12e32c8099c7ae4c3d89a687b2b5`。本包直接抽取原
renewal_confirmation::follows_retained，在私有副本加单次调用薄根；生产保持只读。
不改 V1 的 can_discard_confirmed_sdk，也不把删除规则重新实现为本包模型。

FullRetainedContract 与 RetainedOrder 只描述本函数对已解码值的接受条件：

- Absent 只允许前项版本为零。
- Version 与前项相等时，前项必须大于零，不读取压缩/确认元数据。
- 其余 Version 必须严格大于一个正前项，同时不超过保存的 compacted_sdk_through；
  本 intent 的 accepted_version 不超过完整 confirmed_renewal 中的接受版本。
  两个可选字段均须存在。原函数不验证这些元数据本身是否合法或真实发生。

follows_refines 覆盖任意 Session、previous 和完整 AcceptedLifecycleIntent；接受与拒绝
充要条件引用同一规格。合法请求见证复用原 ValidEnvelope；空条目见证专门说明本检查
不建立历史存在或可达性，不能称它满足完整 Inbox.validate。原 caller 另检查接受版本
递增、Destroy 特例、封套、确认形状和执行谱系；这些检查不能从本函数成功推得。

三份真实 Rust 错误版分别反转压缩上界、反转确认上界、允许零版本相邻。
正确/错误/恢复版执行同一组原生断言，所有 Lean 视图共用同一 Model/Values；负例
不导入正确版对应定理。错误 Rust 必须实际编译、抽取并到达同合同反例，编译失败、
导入失败或超时均不计作性质拒绝。

外部函数精确为已登记 String.eq、Vec.is_empty、Bool.ne、Option.eq、Option.as_ref
和 Option.is_some_and，复用 inbox-record 的原 FunsExternal。两个自有闭包保持透明。
本纯布尔闭包不引用 ContractError，不需要别名；两遍固定 Charon 复查同一完整 IR，
仅复用原 llbc_digest 对无序 short_names 排序，拒绝重复键并保留所有有序 AST；
再运行固定 Aeneas。没有新增库绑定或业务假设，没有手改生成体或导入公理模板。

formal.py 复用原 actual_files/proof_services，检查全部实际文件、完整且非 stale/partial
的服务诊断与内核依赖；prepare-only 始终 complete=false。独立 .lake/build，外层持
共享重锁，保留缓存及失败记录。以下为组件开发入口，结果以实际产物为准：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-retained/native.py --output /private/retained-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-retained/formal.py --development-native /private/retained-native/result.json --variant correct --output /private/retained-correct
# 错误视图分别为 reverse-compaction-bound / reverse-confirmation-bound / zero-version-adjacency。
```

正式 --require-native 需要 ning 的完整封口及本 case；本机开发结果不能代替。
coordinator-candidate.patch 是 #713–#718 公共候选之后的增量，只交主控采用，
新增原生 case、四个形式目标和覆盖范围；不直接修改公共入口或冻结。
verify-controller.sh 与共享 proof_native_suite 由主控同批验收和内容复用。

原 validate_lifetimes 的本次预检在固定 Aeneas 报循环提前返回不支持，日志与两遍
Charon 证据保留；未把失败算作业务反例，也未把该函数改为 opaque。本包不证明它，
也不证明完整 Inbox.validate、确认/压缩转换保持性、真实 SQL 读取/提交未知/重启、
GLOBAL→LOCAL 或网络链。V1/V2 提供当前意图与耐久发送，V4/V5 提供实际接受/结果，
V6 提供资源事实，V7 提供独立认证来源；没有假设整个收件事务或 Serde 正确。
