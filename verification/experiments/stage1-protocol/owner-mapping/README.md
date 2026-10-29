# 完整 owner 映射候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md) ·
[任务 #635](https://github.com/VeriInfra-Lab/sandproof/issues/635)

本包固定工程基线 `ae8e5a564ab3d81652a23acb2b99e242193fa3a2`，
结果范围为原 `owner_for` 的**未采用双循环候选**。生产函数、共享状态/事务、
公共 Lake/根导入/冻结/覆盖和本次 AE+65 封口均未由本包修改。
原 Iterator 根的固定工具 HRTB 障碍保留；候选的一般原实现等价、生产采用和普通验收仍开放。

`FullOwnerContract` 在同一份 `OwnerMappingModel` 中定义，对任意完整 Inbox、World 和
原协议 TenantId 量化：首次全文匹配返回该行原 local；没有匹配时，读取所有 World
sandbox owner 与 high-water 的最大值，成功分配受检后继，耗尽返回 IdentifierExhausted。
不预设 owner 表唯一，不增加原函数没有的非零检查。原匹配优先于新分配耗尽。

模型使用原生成类型和独立列表查询/自然数最大值规格；两个循环的终止、
全部索引与加法边界及完整返回由实际生成体对应。
正常空表、同长度不同 owner、首次重复、全部 World 记录、high-water、耗尽及先查再分配
见证复用同一规格。重复表见证属于原函数的已解码值域，不能据此声称外层 Inbox 校验接受。

输出是调用时值上的 owner 分配关系。配置/字段相等不证明已认证、已有资源、已写入映射
或耐久接受。`attach_create` 的同事务重查、排除刚入库 sandbox、完整
owner/sandbox/operation/deadline 绑定和 UPDATE/save，Create/Resume/Unload 调用组合、
真实 JSON/SQL、提交未知/重启及系统结论继续是独立义务。

## 固定库与表示关系

复用原字符串全文相等、Vec/slice 索引、usize/u64 比较及受检/回绕算术和普通错误值载体。
FunsExternal 只复用一份原 String.eq 片段，避免无关生成 ContractError 标签重名；
三个错误类型是原 BoundedWriter 的普通值身份，无 IO/SQL/Serde 调用保证。

固定生成体把 Ord U64 字典传给 max.default，而固定后端接口接收比较函数。
`OrdDefaultBinding` 是本视图的**未采用透明字典投影提案**，把原 partialOrdInst.lt
传给同版后端原函数。生产版与抽取版 Rust 的该默认方法原体均核对为
`if other < self { self } else { other }`。不修改工具或生成体；实际使用限本 U64 默认实例，
没有新增公理或自有业务假设。共享绑定采用由主控/V0 协调。

固定工具、完整外部/透明函数集合、原始生成字节和所有输入在 inventory.json/结果中核对。
生成的 axiom 模板仅保留为原始证据，不导入；所有证明文件都经原实际文件检查器检查。

## 相同完整合同的真实错误版

| 变体 | 实际 Rust 改动与反例 |
|---|---|
| wrong-owner-guard | 将原全文相等守卫用当前循环边界放行；请求 bravo 错返 alpha 的 local |
| wrong-world-maximum | 扫描时重复 high-water，漏掉 World owner 最大值 |
| wrong-high-water | 从零开始扫描，遗漏原 high-water |
| wrong-overflow | checked_add 改为 wrapping_add，耗尽时错返零 |

每版必须实际编译/抽取并保留真实根，证明 observed 和同一 FullOwnerContract 的否定。
编译失败、导入失败、超时或仅有工具退出零不构成反例。
薄根保留原 Inbox/World/owner 参数及共享借用次序；准入另拒绝缺根、外部业务根、
错误 Rust 根和参数换位。

## 独立开发入口

按证明工作流使用固定工具、四线程预算、私有构建目录及既有共享重锁：

```sh
python3 verification/experiments/stage1-protocol/owner-mapping/formal.py \
  --output artifacts/owner-mapping-development \
  --variant all --lock /private/existing-proof-heavy.lock
```

`all` 实际运行候选及四个错误版；单个 `--variant` 只报告该视图。
每版有独立源码/target/.lake/build；全部实际文件、原服务完整非 stale/partial 诊断与
准确审计清单均检查。原 runtime 在批次前后收录全部固定工具、库和依赖内容，
等待重锁与执行耗时分开。缺原始结果或批次 runtime 变化时不得采用叶子成功。

结果仅为未采用候选的局部开发证明，formal/native/shared acceptance 和 full_v3 始终 false。
这不是当前 verify-stage1-protocol.sh 已登记的正式封口，不能用旧根通过背书本包。
公共登记、覆盖和完整验收由唯一集成人协调；本包不向当前 AE+65 窗口追加任务或新 PR。

## 交 ning 的原生输入

native.py/native_prepare.py 是后续 ning 组件候选，**本包尚未取得其原生回执**。
它保留同一 native-common.rs.in 的十条原断言，World 输入由原 core reducer 构造。
原实现、候选、四个真实错误及恢复原实现分别使用独立源码/target，校验实际
protocol 库与 service 测试的 manifest/src/profile/fresh 及产物摘要。
编译/格式/Clippy 失败不能当作原断言拒绝；错误版须达到准确失败集合。

```sh
# 只在 ning 的固定普通环境，普通完整入口仍由主控同批统一执行/复用。
python3 verification/experiments/stage1-protocol/owner-mapping/native.py \
  --output artifacts/owner-mapping-native
```

原生预期集合仅是待实测计划。完整原生封口、公共检查点和原全项目冻结/旧首链问题分别报告。
有限原生对照也不证明原 Iterator 实现与候选的一般等价，不签发完整 V3 结论。
