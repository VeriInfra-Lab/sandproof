# 已解码收件状态与 Recorded 封套

[V3 入口](../README.md) · [协议范围](../../../../docs/formal/03-protocol-inbox.md) ·
[具体提供者合同](../protocol-record/contracts.md) · [依赖](../../../../docs/formal/dependencies.md)

本模块从固定调用方候选 `65cf8a33cb284562f1e986d10db2dfa986bdefea` 继续，生产代码、
实际抽取根、工具和既有 request/accepted/recorded 规格均不改变。公开清单保留原
完整视图的来源，四份模板只增加值级连接证明，不创建替代 Rust 协议或封套算法。

`Assembled` 是真实四字段构造的表示关系：完整 intent、current_session_version、
current 和 terminal 分别对应同名状态字段。它仍是需要生产构造提供的关系；
本模块不把 async `record_lifecycle` 的内联构造、Renew 调用或 HTTP/Serde 当成已证明。
正常见证中的单个 receipt 是一个具体值，不是替代构造函数。

`InboxStateReceiptVersion` 从原条目扫描、会话扫描和完整 Inbox 校验成功推出每条
已存 intent 的接受版本不超过所属会话当前版本。扫描允许任意严格递增版本，
没有把它改成必须加一。失败、发散、计数溢出和遍历索引溢出沿原 Result 语义处理，
不假设回调总成功。完整实际 `validate_using` 对应仍使用已有 Graphic 合同和
ConfigValidation.Representable；具体比较、字符串与容器提供者沿原依赖合同。

`InboxStateReceipt` 连接实际 `Inbox::state` 和现有 `Recorded.accepts_iff`：
对经过实际完整校验的 Inbox，表示关系成立时，匹配返回真当且仅当完整状态意图等于
原意图。实际 `Inbox::record` 成功给出位置与完整意图，再经**record 后的实际校验**和
实际 state 查询，可以得到同一原意图的封套匹配。不能从 record 成功直接删掉后续
校验条件；既有 `record_does_not_preserve_validation` 及其完整合同继续保留。

`InboxStateReceiptValues` 复用既有正常 Create/record/完整 Inbox 见证。字段反例只改
retention window，完整请求仍相同，accepted 封套仍满足原形状条件，全文匹配却拒绝。
因此没有增加 key window 等于 retention window 的条件。terminal 的表示相等也
不能推出实际资源终止，原 Recorded 匹配器并不认证这个标志。

## 检查入口

```sh
python3 verification/experiments/stage1-protocol/state-receipt/formal.py \
  --output artifacts/state-receipt-correct --variant correct --require-native
```

该入口只参数化原 `protocol-record/formal.py` 的正确视图，加入四个真实文件、全部
新增审计及同一表示模型，然后使用原编译/Charon/Aeneas、library_binding、完整物理
文件/服务检查器和源/工具指纹校验。原父结果保存在 `parent-result.json`，没有新造
检查器。`--prepare-only` 只准备，不能与 `--require-native` 合用成为正式通过。

普通/原生由 ning 在最终确定输入上执行原 `protocol-record/native.py` 和已保留组件，
本模块消费同一个 `stage1-protocol-record` case，不另跑重复的三条调用方断言。
新的四个形式目标及输入由主控登记，不能只跑旧根报告本模块通过。公共 Lake/总根、
原生/形式登记、冻结及覆盖均不在本分支改动。局部构建与服务诊断不等于正式或共享封口。

实际 async 构造对应、独立认证上下文来源、具体 JSON/SHA/SQL/事务、未知提交/重启、
V1/V2 耐久发送和 V4/V5/V6 实际资源事实继续作为具名义务，完整 V3 尚未完成。
