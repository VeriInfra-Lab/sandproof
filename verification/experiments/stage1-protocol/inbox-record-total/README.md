# 纯内存 record 统一合同候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**重新抽取、原生正反例和 Lean 检查尚待完成**，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [已有会话追加](../inbox-append/README.md) · [新会话记录](../inbox-record/README.md)

固定生产输入仍为 `a9c9967889c4cf298e4378c7106948018835fc64`，父提交为 #714 的
`27cc13364deda50f7e77a2b07221909e12ff384a`。本包复用同一个隔离生产重构及已有协议规格，
不复制另一份 record 算法，不修改生产或共享配置。生产采用、原版/候选的机器检验等价、
V0 调用采用和 ning/shared 验收仍待完成。

`FullRecordContract` 对任意已解码且按生成类型表示的 Inbox/完整 intent 量化，不要求
调用者预先给出 replay/fresh/existing 的分支条件。`RecordPost` 先取原全文 key 查找：
首个完全重放返回原位置且整个 Inbox 不变；首个冲突拒绝且整个 Inbox 不变；无 key 时
再按原首个 session ID 选择分支。无会话沿用 `FreshPost`，已有会话沿用 `ExistingPost`，
包括精确错误与完整状态。索引界从原选择定理推导，不假设 key/session 唯一。

`record_contract` 组合父包已有真实 Rust 对应定理。正常 Create/Destroy、完整保留重放、
Unload/Resume/SDK Renew 和预留末槽 Destroy 的合法见证沿用父包，不再换一套性质。
`total_implies_existing`、`total_implies_new_session` 证明统一合同确实包含之前的完整
分支合同。原 SDK 门槛/capture 遗漏 Rust 错误版据此否定同一个 FullRecordContract；
正反视图使用逐字相同的 TotalModel，负例不导入正确的 record_contract。

依赖沿用父切片的逐符号库合同；本包没有新增外部库模型或业务 opaque。独立上下文及
配置来源由 V7 提供，V1/V2 提供当前意图与耐久发送，V4/V5 提供本地接受/结果/次序，
V6 提供实际材料/VM 事实。相等、内存记录成功及 32 字节摘要值都不证明授权、持久提交
或资源存在。非 Destroy 版本关系及 DTO Resume 允许空洞的规格保持父包原范围。

该合同不是 `record_lifecycle_intent` 的事务证明。外层 load/authenticated、Destroy 的
releasing-slot 门槛、terminate_local/operation 映射、save、BeforeCommit/commit/AfterCommit
及未知结果/重启均须另接实际提供者；SQLite codec、GLOBAL→LOCAL、完整 JSON/TLS/handler
和完整 V3 继续开放。

## 检查接线

`formal.py` 复用父包正式驱动，仅追加受输入闭包约束的证明模板；追加不能覆盖现有文件。
父包的原抽取根、业务透明检查、所有旧证明和审计均保留。正确视图另接父包已有 Fresh、
NewSession、WindowComparison 证明；错误视图只加统一模型及对应反例。仍通过原
actual_files/proof_services 检查整个实际 Lean 闭包及完整非 stale/partial 诊断。

原生复用 `stage1-inbox-record` 与 `stage1-inbox-append` 两个现有 case，各检查五个真实
Rust 版本；不重复注册等价原生套件。形式驱动逐字绑定两组输入与共同候选。
新证明文件及检查器同样进入该批完整输入闭包；不能拿 #714 的旧输入回执背书新提交。
同批公共入口候选将三个 append 形式目标升级为本包扩展目标，扩展仍包含全部原追加证明。
公共登记、覆盖和正式验收由主控/ning 协调，本包只交候选 diff，不刷新共享冻结。

外层持有共享重锁，每版使用新的隔离输出及独立 .lake/build，保留缓存和失败证据：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record/native.py --output /private/total-fresh-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-append/native.py --output /private/total-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record-total/formal.py --development-native /private/total-native/result.json --development-record-native /private/total-fresh-native/result.json --variant correct --output /private/total-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record-total/formal.py --development-native /private/total-native/result.json --development-record-native /private/total-fresh-native/result.json --variant skip-sdk-gate --output /private/total-skip-sdk
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record-total/formal.py --development-native /private/total-native/result.json --development-record-native /private/total-fresh-native/result.json --variant skip-prior-capture --output /private/total-skip-capture
```

`--development-native` 是本机开发输入连接；正式 `--require-native` 要求 functional 与
两个原生 case 的当前完整回执；开发模式同样要求两组本机结果，两模式互斥。完整 verify-controller.sh/proof_native_suite
交主控同批复用。局部组合定理通过不等于新抽取、整批验收或完整系统结论。
