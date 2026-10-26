# 原收件执行谱系校验候选

[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [返回位置组合](../inbox-position/README.md)

生产固定 `a9c9967889c4cf298e4378c7106948018835fc64`，父证明为 #717 的
`f548e9a6abfdbac67a5c4040b433ea02c2bdeed2`。本包直接抽取原
Session::validate_execution_lineage，只在隔离副本添加一次调用的透明薄根；不应用
record 重构，不改生产。源与完整 Session/Entry 类型及实际依赖由 inventory 和
两遍 Charon 核对；原 ContractError 完整枚举通过核对后沿用既有 Foundations 别名。

FullLineageContract 对任意已解码 Session 给出完整校验结果。LineageState 保存当前
incarnation、完整前项 Entry 和 capture frontier；scan 按原顺序递归遍历任意长度列表：

- Create 先要求 incarnation 为空，再要求 previous 为空，然后记录目标代。
- Unload 先检查来源代，再要求 capture 严格大于全程 frontier；Resume 不重置 frontier。
- Resume 先检查来源代、checked_next 的精确下一代、前项确为相同来源/capture 的 Unload，
  然后仅在本条目 local_operation 非空时更新 incarnation。
- Renew 要求本代匹配；Destroy 不修改代或 capture frontier。所有成功步保存完整前项。
- 遍历完后要求 Session.incarnation 与计算结果相同。

Rejection 是规格中的检查位置标签；真实 Rust 所有拒绝都返回 CorruptStore，没有可观察
的不同错误类别。body_next/body_end 对应原生成循环体，loop_spec/loop_refines 使用
固定 Aeneas loop 的递减长度规则，覆盖所有有限列表和有效游标。有限正常/错误见证
不能代替此一般证明。旧 checked_next/Option 比较证明正文原样复用，仅将其原 Shape
导入换成此视图已有的 ReplayEquality；没有重写或重复计作新算术定理。

正常见证使用同一 ValidEnvelope，包含 Create、待附加/已附加 Resume、跨代递增 capture、
Renew→Destroy。协议 Resume 的 source=1/target=9 仍合法；本内部检查拒绝它，不能据此
把公共协议接受条件加强为必须加一。local_operation 是已解码元数据，不证明附加事务
已提交、当前 VM 确实切换或原执行已经退出。

三种真实 Rust 错误版分别无条件更新 Resume 代、将 capture 的严格递增放宽为允许重复、
删除最终 incarnation 比较。它们只改原校验方法内部，使用同一 FullLineageContract、
Model、Values 和正常封套规格；负例不导入正确循环/调用者定理。native.py 要求每份
错误版实际编译、Clippy 成功，并且只在指定的同一组断言中失败。formal.py 重抽取
各变体；Lean 反例逐步展开该实际错误生成体，编译失败/超时/导入失败不算性质拒绝。

## 库接线与边界

本闭包有七个外部函数符号。字符串相等、Vec.is_empty、Bool.ne、Option 字典比较、
Option.is_none_or 和 Result.ok 沿用已登记的逐符号合同。借用 Vec 的
SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter 是新增的
**待 V0 采用**调用接线；其最小义务 BorrowedVecBinding 只说明原 Vec 内容被转换为
从零开始的同一 Slice 游标，不假设任何业务谱系结论。

library_binding.py 核对冻结的生产/抽取 std Vec 源码及精确 self.iter() 薄体，候选绑定
只调用固定 Vec.deref 和 Slice.iter 模型，记录固定后端源码摘要。源码检查不等于
stdlib 机器证明，也不代替 V0 批准。caller_with_binding 显式接受该条件；candidate_*
仅在本私有绑定下实例化。原透明 std wrapper 的两次抽取尝试因固定 Vec 表示与原始
布局操作不兼容而失败，日志保留，不能计为业务反例或成功的库抽取。

原 current_execution_origin 的反向迭代/查找抽取另外遇到固定工具生命周期限制，
本包不替代其实现、不将它设为业务 opaque，也不认领来源选择已证明。本校验不等于
完整 Inbox.validate，不提供 V7 独立认证来源、V1/V2 当前意图/耐久发送、V4/V5 本地
接受与结果或 V6 文件/VM 事实。完整 codec、提交未知/重启、GLOBAL→LOCAL、网络外壳
及系统结论继续开放；32 字节 ContentDigest 只是值。

## 执行与交接

外层持有共享重锁，每个视图有独立 .lake/build；保留缓存、原生断言失败、抽取失败
和 Lean 探索诊断。native.py 的 original/三个错误版/restored 使用同一输入闭包，
核对实际编译的协议库/service 测试、manifest/profile、锁定依赖及变异前后源码。
formal.py 使用原 actual_files/proof_services 检查全部真实 Lean 文件、完整且非
stale/partial 的服务诊断和每项内核依赖审计。prepare-only 始终 complete=false。

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lineage/native.py --output /private/lineage-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lineage/formal.py --development-native /private/lineage-native/result.json --variant correct --output /private/lineage-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lineage/formal.py --development-native /private/lineage-native/result.json --variant advance-unattached-resume --output /private/lineage-resume
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lineage/formal.py --development-native /private/lineage-native/result.json --variant repeat-capture --output /private/lineage-capture
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-lineage/formal.py --development-native /private/lineage-native/result.json --variant skip-final-incarnation --output /private/lineage-final
```

正式 --require-native 仍要求 ning 同批 functional 和 stage1-inbox-lineage 完整回执。
公共登记、原入口的新目标、覆盖及 V0 最小接线只交 coordinator-candidate.patch，接在
inbox-record→inbox-append→inbox-record-total→inbox-state→inbox-position 公共候选之后。
主控统一验收和内容复用 verify-controller.sh/proof_native_suite，不以局部检查代替
全套，亦不自动刷新历史冻结。实际执行结果与精确输入由本切片 PR/证据记录提供。
