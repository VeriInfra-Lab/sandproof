# Create 排除自身后的 owner 后继：当前真实 Rust

[V3 设计](../../../../docs/formal/03-protocol-inbox.md) · [公共验收](../README.md#固定-create-交付) · [依赖合同](../../../../docs/formal/dependencies.md)

当前检查器直接核对生产 `create_owner_successor`，不再对正确版应用旧 `candidate.patch`。
原 Create 在没有现存 owner 行时，从实际 World 的全部 sandbox 记录中排除新 sandbox，
取原记录 owner 与 inbox 高水位的最大值，再受检加一；耗尽返回 IdentifierExhausted。
薄根传递原完整 Inbox/World 的两个共享借用及 sandbox 值，不另造简化业务实现。
准入检查真实 MIR 参数、透明业务函数、完整错误、生成闭包和每份实际证明文件。

## 同一完整合同

`CreateOwnerSuccessor.FullSuccessorContract` 对任意完整 Inbox、World 与排除 ID，
要求真实生成 helper 返回原 `OwnerMapping.allocation` 对 `OwnerMapping.maximum`
的结果，输入为排除该 ID 后的原记录表及原高水位。`successor_complete` 证明任意表的
终止、索引/计数边界、完整过滤、高水位参与和最终正常/错误结果；不假设记录唯一、
ID 非零或物理资源存在。

原正例覆盖空表、排除最大行、排除 ID 不存在、高水位、被排除行达到整数上限、
其它行达到上限和高水位耗尽。四个错误版反转排除条件、误用 owner 字段、遗漏
高水位或使用 wrapping；须实际编译、抽取，再证明同一合同的否定。
错误视图共用逐字节相同的规格和值文件，不导入正确版的完整对应定理。
编译、导入、准入失败和超时均不能算业务反例。

## 依赖与交付边界

复用已有 OwnerMapping 模型、算术、完整值见证、有限 U64 Ord 字典投影和错误值别名，
检查原字节及标准库/固定后端输入。零外部函数生成库在 TypesExternal 导入同一
Ord 绑定，不修改生成函数、工具或填充 axiom 模板。条件限正常普通值；不保证
OOM/abort、物理文件、VM 或可观察自定义 Drop 行为。

普通侧九条原断言用于正确版、四个真实错误版及恢复版共六个视图。所有变异限制在
已核函数范围，旧 visibility/相邻函数文本不再作为业务锚点；仍要求实际编译和
同一断言的精确失败集合。正式交付只由公共固定 Create profile 与完整原始封口提供。

其它安装、寿命、ID/SQL/save 包独立检查；物理事务同一性、提交不确定、重启、
JSON/TLS 与持久收件的责任不会从这个值合同推出。字段相等不等于授权或资源存在。
原 AE/candidate 证据保留原身份，不迁记为当前版本通过；当前候选、RUN/REUSE 与
实际验收结果见 #635 和交付 PR。
