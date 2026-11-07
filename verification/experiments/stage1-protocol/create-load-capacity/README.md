# 实际 load、完整校验与 Create 写入容量

[组件入口](../README.md) · [V3 设计与边界](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md)

本目录把既有 FullLoad、FullInbox、完整 Create 选择和 FullInstall 放在同一次实际抽取的 Inbox 表示下。容量连接直接复用既有 `InboxLoad.successful_load_requires_validation`，不另做一套 load 成功归纳。`actual_load_owner_limit` 从实际 load 成功返回的同一个完整 Inbox 推出 owners 长度不超过配置上限；`actual_loaded_selected_install` 据此消除完整写入定理的调用方容量前提。结论保留完整 intent、session/entry 位置、local binding 及整个 Inbox 的写入后置条件。它不证明完整父函数、owner 分配单调性、写入后的整个 Inbox 合法性、授权、实际资源或持久接受。

`inventory.json` 逐项绑定生产源、原提供者、四个透明根、生成闭包、物理文件和审计。`binding.py` 核对原可变/共享对象、全部值参数和六类 load 泛型字典；它的准入拒绝检查与业务反例分开。原生、公共 Lake/根/输入/覆盖登记及整套验收由主控接线；本入口始终报告局部开发结果，不能以此替代完整入口。

## 当前格式与历史证据

当前生产 `LIFECYCLE_STORE_FORMAT_VERSION` 来自协议内部的 `HOST_STORE_FORMAT_VERSION = 24`，生命周期协议版本仍为 4。旧 `inbox-load` 模板固定 Store23。本目录仅在隔离工作区对清单中五份原模板应用逐字节审查的 Store23→24 字面量变更，核对原/新内容及位置数量；FullLoadContract 的形状、所有其它字段、拒绝顺序和 callback 行为保持原规格。正反视图用同一份 Store24 规格。

这次重新抽取和检查针对当前生产源，不修改旧冻结、旧模板或其执行身份，也不将旧 Store23 结果迁记成当前通过。生成体直接取固定 Charon/Aeneas 输出，不能手改。全原始 ContractError 枚举先独立抽取并与 Foundations 对照，再采用既有逐值 carrier 绑定；仅复用已登记的字符串、Vec、Option、算术及 Graphic 等逐符号合同。

## 正常与错误见证

`CreateLoadExamples` 复用完整 Inbox 校验已接受的 `recorded`：有非空 owner 表和一条完整 Create intent。实际 load 返回此值后，实际选择取得同一 session/entry，完整写入定理适用于任意传入 binding。callbacks 是显式可观察的合成值，包含完整 encoded Inbox 和两次配置读取；这不证明它们实现真实 JSON、SHA 或 SQLite。

`wrong-owner-limit` 只删除真实 `Inbox::validate_using` 中 `owners.len() > max_sessions` 守卫。错误版实际编译、抽取，在相同完整 FullInboxContract 与 FullLoadContract 下构造超限但其它字段合法的表。反例直接证明实际 validate 接受且实际 load 返回超限 Inbox；错误视图不导入正确的完整 validator/load 对应定理。编译失败、导入失败、超时及不完整诊断不计作性质拒绝。

## 独立开发入口

```sh
# 证明环境：原共享锁、固定工具、独立 .lake/build。
python3 verification/experiments/stage1-protocol/create-load-capacity/formal.py \
  --output /private/evidence/create-load-capacity --lock /private/locks/proof-heavy.lock
# ning：实际、真实错误版、恢复实际，执行同一原生断言。
python3 verification/experiments/stage1-protocol/create-load-capacity/native.py \
  --output /private/evidence/create-load-capacity-native
```

形式入口重新捕获原始枚举、bound Request 和 Graphic 三个独立视图，逐份构建并通过原 `actual_files` 检查器核验全部真实物理文件、完整非 stale/partial 服务结果与依赖。原生入口只复用既有 fixture 的准备/效果代码，核对实际与错误版的完整校验、真实 codec load 以及合法 load→选择→完整写入断言；普通测试仍由 ning 执行，未运行不能声称通过。

持久收件事务、同一快照、实际 JSON 字节到值、实际哈希计算、独立认证上下文的来源、SQL 参数/更新行数、save/提交未知/重启和资源事实仍是具名后续义务。既有 Manifest 与 FullFlow 继续保持各自原适用范围；本切片不重新声称它们已覆盖完整系统。
