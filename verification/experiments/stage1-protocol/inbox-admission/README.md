# 收件认证入口组合候选

[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md) · [父组件](../README.md)

本切片从已解码、按原生成类型表示的 `Inbox`、`AcceptedLifecycleIntent` 和独立
`AuthenticatedCenterContext` 开始，连接真实 `authenticated` → `Inbox::verify_context`
→ `AcceptedLifecycleIntent::validate`。生产输入固定于
`a9c9967889c4cf298e4378c7106948018835fc64`，堆叠父项为 #710。

原认证调用可由固定 Charon/Aeneas 直接抽取。本包没有生产重构；不应用父包的
`Inbox::find` 拆分补丁。`roots.rs.in` 仅在隔离副本转发完整三个参数，四个现有协议
薄根仍共同抽取，原 Request/Accepted/Recorded 规格及完整字段相等定理继续复用。
生成的 Types/Funs 原样装入独立 request 视图，不手改生成体。

## 完整合同

`FullAuthenticationContract` 对任意输入要求实际结果等于唯一 `expectedAuthentication`：

1. 保存的完整中心和完整节点均等于连接上下文时，调用原封套规格。
2. 上下文不匹配时立即 `Conflict`，不继续封套检查。
3. 封套成功返回成功，任意封套错误映射为 `Conflict`。

`InboxAdmission` 证明原实现对应、接受/拒绝的充要条件、上下文拒绝优先，以及成功时
intent 的中心、request 的完整节点绑定原配置。内层封套首错仍使用原
`FirstAcceptedError`；外层错误被合并，不能称外层保留具体封套错误类别。
正常输入、独立 retention window 和完整节点代不符见证使用同一规格。

两个真实 Rust 错误版分别只删除完整节点检查、只删除封套检查。`prepare.py` 在
隔离副本中核对唯一原表达式后实施变异；正确版、错误版和恢复版使用同一组原生断言。
错误版必须实际编译、成功抽取并到达断言失败；编译失败或工具失败不算性质拒绝。
两个错误生成视图分别证明错误接受，并否定同一 `FullAuthenticationContract`。
Model、Support 和 Values 在三视图中逐字相同，不导入正确版组合定理到错误环境。
漏节点反例的完整封套仍满足原合法形状；漏封套反例故意含无效 slot/接受版本，原规格
先拒绝 retention。共同 `InboxLookup` 值夹具只表示有限容器，不声称已建立持久状态不变量。

## 依赖与范围

复用已登记 String.eq、Vec.is_empty、Bool.ne、Option.eq、实际 U64 字典及固定数值表示。
错误类型只复用 IO/SQLite/JSON 值载体；ContractError 别名逐项核对原透明 enum。
原调用使用固定 Aeneas 的 Result `Try::branch`、`FromResidual::from_residual`、
`FromSame`、`map_err` 和 PartialEq 默认 `ne`；需 V0/主控逐符号核对本调用采用范围。
错误映射闭包和业务判断保持透明，不能把整个 handler 或认证函数当作可信适配。

这里的匹配是字段关系。V7/连接适配仍负责独立上下文及受保护配置来源，V1/V2 负责
当前意图和耐久发送，V4/V5 负责本地接受、耐久结果与调用次序，V6 负责实际文件/VM。
本包不读取 SQLite，不证明授权、收件持久提交、GLOBAL→LOCAL 映射、未知提交、恢复、
JSON/TLS/异步外壳或完整 V3。`Inbox::record` 的抽取预检仍受固定工具的高阶生命周期
约束限制；失败证据保留，未把它记为业务反例或本包已证入口。

## 检查与交接

输出目录必须不存在；外层取得工作流规定的共享重锁，每版使用独立源码、Cargo target
和 `.lake/build`，只共享只读依赖。开发入口：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-admission/native.py --output /private/admission-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-admission/formal.py --variant correct --output /private/admission-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-admission/formal.py --variant skip-node --output /private/admission-skip-node
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-admission/formal.py --variant skip-envelope --output /private/admission-skip-envelope
```

驱动绑定全部当前输入、锁定依赖源码、实际工具/后端/服务内容，要求原调用和上下文守卫
出现在透明生成清单，经原 `actual_files` 与规定服务完整检查实际文件和审计。
原生检查分别验证真实协议库、service 测试程序的源码/manifest/profile 和新编译产物。
开发结果不签发 ning 或共享验收。正式 `--require-native` 还要求当前完整共享 seal、
`functional` 与新 `stage1-inbox-admission` case，并核对原生/形式副本的实际业务源码。

公共登记、覆盖与最小依赖采用候选只交主控，见 `coordinator-candidate.patch`；需同批
合并父包登记，保留原入口、全部原错误版与 Manifest/Carriers 证据。普通完整入口和
共享 `verify-controller.sh` 由 ning/主控验收；局部通过不表示完整 V3 已验收。
