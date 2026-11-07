# 主控采用候选

本目录的独立 formal/native 入口和生成视图属于 V3。同一分支继续完整 V3 审查，
不另开碎片 PR、不修改公共登记或重启历史汇总。

公共维护方采用时须同步登记：

- actual/default-world 两个正式视图的全部真实文件与原内核/同服务/依赖审计。
  正反使用同一个 `StandardCreateModel` 和 `StandardCreateFixture`；错误视图
  不导入正确实例定理。新默认值函数是错误 Rust 的透明抽取，不是模型替代。
- `stage1-create-standard-adapter` 原生 case：actual/default-world/restored
  同三个断言、真实新编译和精确失败集合，来自 ning 的有效证据。
- 两个实际 Std 符号的有限库条件：
  `RefCell<Option<完整 World>>::borrow` 与 issued `Ref<Option<完整 World>>::deref`。
  借用计数、同一对象/全值、正常 guard 释放、纯 World 克隆区间和异常范围须明确。
  源码正文核对不等于 Std 内部实现已机器证明。
- 复用原完整 World 克隆、String/Vec/Option、SQL Connection 观察载体和正常字面量
  条件；仅补当前 `OperationPlan.capture` 的透明完整 handoff 克隆引理。
  独立视图的 canonical clone 数学只换导入/命名空间，不并发改共享 Access 文件。
- V5 活对象/事务/lease 历史与 A4 物理连接/事务/行历史保持具名提供者条件。
  不把整个 WorldTransaction、handler、收件或持久事务列为可信函数。

后附候选补丁及精确前后摘要供单写维护方审查；本包不自行采用共享 Lake/Cargo、
根/原生登记、根级冻结/输入、公共依赖和覆盖修改。最终候选必须让现有
`verify-stage1-protocol.sh` 真实包含这些新目标，不能只跑旧根报告新版通过。
普通完整入口及共享 proof_native_suite/verify-controller 与其它组件同批统一
验收，已有证据按实际输入逐项 RUN/REUSE，不重写旧执行身份。

三个零上下文补丁按 create-attachment-flow、create-fixed-identity、
create-lifetime-sql 的对应候选之后生成。检查/采用用 `git apply --unidiff-zero`，
前后摘要见 registration-candidate.json。私有文本副本只检查补丁适用性，不运行
普通 Python/Rust 测试；当前共享文件若已变化，应重整候选而非盲刷摘要。

根分区准入保持至少 64GiB，加本入口每视图 4GiB 写预算的双倍余量，使用现有
共享重锁。保留 `.lake`、固定工具、原失败和原始输入；不以降门槛或换版本通过。
