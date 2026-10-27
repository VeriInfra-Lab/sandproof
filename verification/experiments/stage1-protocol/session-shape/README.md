# 会话形状的值校验候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包固定于 a9c 的 `Inbox::validate_session_shape`，生产只读。原 slice 迭代器在
固定 Aeneas 的生命周期约束处抽取失败；直接 Serde 路径及 first/last 查询也不能在
该视图闭合。`prepare.py` 在隔离副本中使用透明索引遍历，复用已有末项查询，
以及 native-retirement/retention-lane 的窄编码接口。标准实例仍调用真实编码和
摘要实现；`production-candidate.patch` 仅供工程负责人审查。

原实现与候选的一般等价、生产采用、标准 Serde 字节/错误及实际 SHA 连接仍开放。
原生对照不是一般等价证明。形式结论从真实生成类型表示的已解码值开始，唯一正反
规格为 `ShapeModel.FullShapeContract`；不表示 JSON/TLS/handler 或完整 V3 已证明。

## 精确合同

`ShapeProofs.validate_shape_spec` 对任意 Inbox、Session、Usize 索引及两种编码参数，
连接实际生成的 `validate_session_shape_using` 与 `ShapeModel.validateView`：

- 先执行完整保存收据合同，保留其首错、编码/digest 调用顺序、失败与发散。
- 再检查非空 entries、slot 非零且不超过容量、首个同槽 lane 的 current_window，
  local 存在时 host observation 必须存在，以及操作条数上限。首个 lane 不匹配时，
  后续同槽 lane 不能补救；此函数没有假设 lane 唯一。
- 以上条件通过后，检查 `sessions[..index]`。索引越界保持原 panic；前面已经拒绝时
  不触及切片。仅前序会话参与冲突检查：相同 id、slot 或双方 local 的相同物理
  sandbox 均拒绝。物理冲突不要求 owner/operation 相同。
- refusal 存在时，不允许 local；首条必须是 accepted_version=1 的 Create；所有
  entries 都不能含 local_operation、applied_deadline、previous_effective_end，且只
  能是 Create 或 Destroy。没有只检查首条而漏掉后续条目。
- confirmed_terminal 存在时，要求 terminal=true、Cleaned、终态完整 self-match，
  并与最后一条完整 intent 相等；然后才编码，传播 JSON 错误，长度须不超过 8192。

`ShapeConsequences` 给出接受充要条件、各层拒绝次序、越界和失败传播；没有假设
编码器总成功。`ShapeValues`/`ShapeBoundaryValues` 复用已有合法 Create→Destroy
封套，给出正常、遗漏字段、首次匹配、前序范围、8192/8193 及编码错误次序见证。
`ShapeWitnesses` 将这些见证连接到实际候选；任意输入定理由完整对应证明承担。

本函数自身不检查所有 accepted 封套、owner 成员关系、认证 observation 的来源、
完整 incarnation/版本执行链或实际资源。local 分支这里只要求 observation 存在，
没有额外的节点匹配条件。完整 Inbox.validate 与耐久接收的证明继续单列。
ContentDigest 的 32 字节比较不表示已计算哈希，也不推出材料真实完整。

## 正反检查与输入

四个真实 Rust 错误版分别把首 lane 改成任意匹配、漏掉物理 sandbox 冲突、漏掉
refusal 的 previous_effective_end 字段，以及把末条完整 intent 相等弱化为 session
相等。它们在相同 `FullShapeContract` 下给出实际接受和违反完整合同的定理；不为
错误版更换规格。错误视图不导入对应变异方法的正确证明。

`native.py` 在独立源码和 target 中检查原版、候选、四个错误版及恢复版；共同测试
观察实际标准实例，候选另检查编码边界和收据/terminal 的回调次序。检查格式、严格
Clippy、实际协议库和服务测试的 fresh 来源、精确断言失败及输入摘要。编译失败、
工具超时或导入失败不算性质拒绝。

`formal.py` 重新抽取每个视图，核对透明根到实际方法的单次调用、目标和选项、全部
必需查询及循环体、外部符号和生成声明，再通过规定同服务客户端检查完整导入闭包
及其所有声明审计。原始 LLBC、生成体和失败证据保留；生成体不手改。

本视图只复用六个已登记的逐符号值模型：String.eq、Vec.is_empty、Bool.ne、Option
字典比较及 is_some_and/is_none_or。物理冲突错误版不再使用 is_some_and，其外部集合
单独精确核对。现有 A6 定义和属性逐字生成导入视图，错误值表示沿用已有登记；没有
为整个 Serde、校验器或收件事务引入可信模型。保存收据和终态匹配证明按原模板复用。

开发入口是 `native.py --output ...`，以及
`formal.py --variant correct --development-native .../result.json --output ...`；错误版
指定对应 variant。执行时持有共享重锁，使用独立 `.lake/build` 并保留缓存。
`--require-native` 验证 ning 的完整功能和 `stage1-session-shape` 封口证据；开发
结果不能代替该入口。公共脚本、原生登记、审计清单及覆盖记录只提交候选补丁，供
主控在前 18 个协调补丁之后应用；该补丁保留旧命令并登记本包及其复用证明的审计。
其他父包的公共审计登记及根级输入冻结仍由主控统筹。

本包局部通过不能背书尚未登记本包的旧 `verify-stage1-protocol.sh`，也不替代
`verify-controller.sh`、共享 `proof_native_suite` 或 E2E。

## 未闭合义务与提供者

V3 仍负责原候选等价、真实编码接线、完整 Inbox 合法与转换保持、codec、同事务
接受、未知提交和重启。V7 提供独立认证上下文；V1/V2 提供当前意图和耐久发送；
V4/V5 提供实际本地接受与结果/收据；V6 提供真实文件、VM 和清理事实。字段相等、
形状合法和 Confirmed 标签均不提供这些业务事实，完整系统结论继续单列。
