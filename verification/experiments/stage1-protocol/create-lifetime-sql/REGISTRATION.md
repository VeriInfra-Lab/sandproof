# 两参数寿命 UPDATE 的公共登记候选

本包沿同一 V3 分支提供一个 native label 和正确／两错形式任务，不另开碎片 PR。
共享文件由主控唯一维护者采用。本目录补丁、源码可读及局部形式检查均不表示普通、
共享、实际事务或完整 V3 已通过。

三个零上下文补丁按 `create-attachment-flow`、`create-fixed-identity` 的对应候选
之后的实际文本生成；前后摘要见 registration-candidate.json。检查和采用均须带
`git apply --unidiff-zero`。已在私有文本副本检查适用性，未修改共享文件，也没有
执行普通 Python/Rust 测试。若主控当前文件已变化，应按本合同重整差异，不能盲刷摘要。

- native 为 `stage1-create-lifetime-sql`：实际、deadline 清零、全文清空、恢复实际
  四份源，每份同六项断言；新编译协议库及真实 service 测试，精确检查失败集合与
  链接源。实际／恢复须完整通过；lint 拒绝不算业务反例。
- formal 包含 actual、zero-deadline、empty-sandbox，必须拒绝 prepare-only、缺视图、
  partial/stale、缺文件、缺审计或未实际到达反例。目标清单分别为 18 份实际文件、
  20／17／17 项审计，共 29 个唯一新审计名；实际执行还须逐项核对 inventory.json。
- 外层单一重锁以同一打开对象传入子进程；本包保持独立构建目录、资源门槛和原内核／
  同服务客户端。未变化的公共 ByteArray／literal 原数学复用，不重新计数。

公共定义仍使用现有 SQL 模型。局部 materializer 以原名称、原函数体生成精确片段；
只避免旧业务错误类型的重复登记并连接当前完整 Params trait，不另造数据库算法。
新增有限纯数组 whole-execute 使用点的 A4/A6 条件及未完事务义务见 README 和
dependencies-candidate.patch。coverage-candidate.patch 仅补当前值／错误范围。

主控还须统筹原 `verify-stage1-protocol.sh`、输入／冻结、累计审计、普通准入及
`verify-controller.sh`；不只运行旧根报告新包通过，不让 V3 与主控重复完整共享套件。
源代码推送与原始证据实际接收分别记录，原生回执须绑定最终精确输入。
