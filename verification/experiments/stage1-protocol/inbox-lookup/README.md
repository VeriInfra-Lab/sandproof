# 收件全文查找候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**重新抽取、原生正反例和 Lean 检查尚待完成**，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md) · [父组件](../README.md)

本切片从已解码的真实 `Inbox` 与 `AcceptedLifecycleIntent` 值开始，只覆盖
`controller-service/src/controller/lifecycle_inbox/model.rs` 的全文查找。
原生产输入固定于 `a9c9967889c4cf298e4378c7106948018835fc64`；堆叠父项为 #709。
它不读取 SQL，不认证连接，不证明收件提交、崩溃恢复、当前授权或资源存在。

## 真实入口与候选边界

固定原 Aeneas 拒绝原 `Inbox::find` 的嵌套循环提前返回。
`production-candidate.patch` 仅把原内层循环移入私有 `Session::find_entry`，
外层按原顺序传播首次结果与 `Conflict`。补丁只应用于隔离副本，生产未采用。
原版与候选的同断言测试是回归证据，不是任意输入的 Rust 重构等价证明。

`extract-root.rs.in` 仅转发 `inbox.find(intent)`，由工具在隔离模型文件末尾装入；
这是抽取根，不属于生产补丁。固定 Aeneas 会裁掉没有调用者的外部根，故不能仅因
生成物中含有 `find_entry` 或工具退出为零就报告 `Inbox::find` 已抽取。
驱动明确要求两个真实方法及各自循环/循环体出现在生成清单，且保持透明。
四个现有请求/封套薄根一并抽取，从而原样复用 `ReplayEquality` 的完整字段相等证明。
生成的 Types/Funs 原样装入独立 request 视图；不手改生成体，不导入公理模板。

## 合同与证据

`InboxLookupModel` 定义唯一 `FullLookupContract`：按会话、条目的原顺序扫描，
首个完整 operation key 命中时检查完整 accepted intent；全文相等返回原始下标，
全文不同立即返回 `Conflict`，没有同键项才返回 `None`。
合同不要求 key 唯一，也不检查 session 当前版本、terminal 等元数据。

`InboxLookup` 证明任意有界向量的内外循环终止、全部结果对应、缺失的充要条件，
以及排除所有早先同键项之后的首个命中/冲突结论。下标增加的界来自真实向量长度，
不另加有限样例或“总能成功”的业务前提。`InboxLookupValues` 复用原封套正常形状见证；
同 key、不同 capture 的两个值都满足原封套形状合同。这里不声称构造的收件容器
已经通过完整持久状态不变量。

真实错误版把唯一 `entry.intent != *intent` 守卫替成 key 比较。原版、候选、
错误版和恢复版运行同一组四项原生断言；错误版须实际编译并触发全文冲突和首次冲突
两项拒绝，编译失败不算反例。`InboxLookupWrong` 使用重新抽取的错误生成体，
证明同一正常形状输入被错误接受，并否定正确版的同一 `FullLookupContract`。
正反版本使用逐字相同的 Model、Support、Values。

## 依赖与待采用项

沿用依赖登记的 String.eq、Vec.is_empty、Bool.ne、Option.eq 及固定 Aeneas 的
数值、字节比较与容器表示。错误类型只复用原有 IO/SQLite/JSON 错误值载体，不引入 SQL
或 IO 操作模型；ContractError 别名须与原透明 enum 的所有变体、字段和布局一致。

需 V0/主控逐符号核对并登记本切片实际使用的固定内建：`Slice.iter`、
`IteratorSliceIter.next`、`Iterator.enumerate`、`IteratorEnumerate.next`、
`Vec.deref`、标准 Result 的 Try/FromResidual 与同类型 From。尤其要核对迭代顺序、
耗尽结果、计数递增及失败传播。现有后端模型和本地定理不是这些 Rust 标准库函数的
新机器证明；公共采用前，本切片保留为候选。生产重构采用归主控，独立认证上下文归
V7，当前意图/耐久发送归 V1/V2，本地接受及结果归 V4/V5，物理材料事实归 V6。

## 运行与交接

开发检查可以独立执行；输出目录必须不存在，所有失败记录保留：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lookup/native.py --output /private/lookup-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lookup/formal.py --variant correct --output /private/lookup-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lookup/formal.py --variant key-only --output /private/lookup-key-only
```

按工作流在外层取得共享重锁；每版使用自己的源码、Cargo target 与 `.lake/build`，
只共享只读依赖。形式驱动重新抽取原版限制与候选，绑定全部当前输入、原生注册源、
固定工具及完整后端/服务输入，再经原 `actual_files`/严格服务检查全部实际文件和审计。
开发结果不签发 ning 或共享验收；`--require-native` 还要求当前完整共享 seal、functional
和 `stage1-inbox-lookup` 原生 case。候选公共登记见 `coordinator-candidate.patch`，须由主控
与同批其它包协调，不能只运行旧 `verify-stage1-protocol.sh` 就报告本切片通过。

原普通完整入口、既有四类协议错误版、Manifest/Carriers 证据均保留。
#687 的任意已解码 manifest 验证已有独立候选，应按精确输入复用，不因本切片重证或迁记。
原 JSON parser/字节到值链、持久收件提交/恢复、网络外壳、E2E 与完整 V3 仍未完成。
