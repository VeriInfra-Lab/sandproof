# 实际 Create owner 守卫到完整写入的高水位

[组件入口](../README.md) · [V3 设计与边界](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md)

本切片把实际 load/完整校验、选择、existing/new owner 守卫、owner_for、successor 和整个 Inbox 写入放在同一次抽取的表示下。原 FullExistingContract、FullNewContract 和 FullInstallContract 保持不变，原证明仅作生成库命名空间的适配。新 FullCheckedInstallContract 同时要求原完整 Inbox 写入框架和 owner_high_water 不下降。load 成功时的 owners 数量界直接来自上一切片，不能由调用者另加一个没有落实的空间前提。

正确版对所有完整 Inbox/World/local binding 证明组合合同。已有 owner 分支保持高水位；缺失分支的实际 new 守卫成功意味着新编号严格大于原高水位。Resume 的编号策略不在这里加强为加一，也不从字段相等推出授权、真实资源或已持久接受。

正常见证包含单独的完整守卫/写入输入，以及原完整 validator/load 已接受的 Inbox 与完整 Create intent。它们连接同一个实际 load 结果、session/entry、两个守卫和完整安装结果。后一见证只证明这段前缀，后续 save 仍可能拒绝写后的 Inbox；不声称任意安装保持整个 Inbox 合法。

两个错误视图分别只把真实 successor 的初始高水位改成零，以及把真实安装中的新 owner 高水位写成零。每个错误版保留同一个组合合同：前者实际接受较小编号且安装后下降，同时违反原 FullNewContract；后者正确守卫接受后实际安装下降，同时违反原 FullInstallContract。错误视图不导入对应错误实现的正确证明。编译、导入、超时或服务失败不算业务反例。

ning 原生入口对实际、两错和恢复实际运行同三项断言，要求协议库与服务测试程序新编译并核对实际 source/manifest/profile 和精确失败集合。错误 seed 同时使原 inbox 形参未使用：严格 Clippy 仍用 `-D warnings`，仅将该固定源位置的唯一 `unused_variables` 诊断另记为预期变异 lint 拒绝；它不算业务反例，其它诊断仍失败。正确、错误写入和恢复实际必须严格 Clippy 成功。原生入口尚未执行。

当前存储格式为24、生命周期协议为4，继续复用上一切片对五份旧 Store23 模板的有限已审查更新；旧冻结、模板和执行身份不改。生成体来自固定 Charon/Aeneas，原完整 ContractError 先独立抽取核对，再使用原逐值 carrier 绑定。仅依赖登记中逐符号的标准库模型；ContentDigest 值往返不证明计算过哈希。

公共 Lake/Cargo、根、原生/正式登记、输入与覆盖由主控单写。本包独立入口与完整物理文件结果仍需由主控统一接线；普通与原生由 ning 执行。实际父函数的惰性 World 读取、原 deadline/保留两个输入的十进制 ID 格式、两个 SQL 参数、save 与同事务、提交未知/重启，以及 JSON/TLS/producer/driver/恢复等义务继续另列。原寿命 UPDATE 不检查更新行数，不额外声称创建了实际资源。已有 Manifest、载体、查找和 FullFlow 的证据保留原适用范围。
