# 探测候选的同步判定预检

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包是 `ReadyCandidate::can_probe` 的独立候选，精确源码基线见 `inventory.json`。
正确副本只向原 snapshot 模块追加 `roots.rs.in` 的一次转发；错误副本仅使原
credential 存在性条件恒真。原生产文件及工具生成体不在本包修改。
未接入公共验收入口，也不追加到已冻结的主控交验批次。

`FullProbeContract` 对任意完整候选要求实际方法返回 `ProbeEligible` 的判定：
continuation 为 Some 时恰有一个动作，否则恰有五个；source 与 credential 都为
Some；readiness 为 Pending(HostUnobserved)。类型保留原八字段和完整嵌套载荷。
正常五动作见证、对任意原 continuation 的一动作见证与十七组拒绝使用同一规格。
错误版的实际缺 credential 见证否定同一个合同，两版 Model/Values 逐字相同，
错误证明不导入正确版实现定理。

一般不变引理表明本方法不检查动作内容、Some 载荷身份、receipts、options、ready_fact
或 view 的其它字段。合法探测值可以没有 native、local、历史结果及 receipts；
因此本判定不提供整体 Ready、资源存在、凭据正确、已授权或耐久执行的结论。
这些事实仍须沿真实候选生产与探测调用落实。

## 未采用的值载体

原类型闭包包含 CreateOptions 下的 PathBuf 和 BTreeMap。生成类型全部保留；
`TypesExternal.candidate.lean.in` 仅是待 V0/主控审查的值表示提案，不能当作已登记
外部接口。BTreeMap 复用现有 Storage 的完整键值条目载体；Unix PathBuf 暂提议
用既有有限 Vec<U8> 保存全部原始路径字节，不提供任何路径或 map 操作合同。
真实 Rust 到这两个载体的表示关系和当前视图采用均尚未落实。
最小请求见 `carrier-review-request.json`，不能借“本函数不读取该字段”宣称表示已经证明。

函数级外部绑定只转发已登记 String 全文比较；Vec.len、Option.is_some 沿固定 Aeneas
内建模型。原派生比较保持透明。候选引理的完整物理文件和同服务检查须包括此载体提案，
不能只检查三份手写证明，也不能把该局部结果记作公共正式验收。

本目录交付数学模板和最小接口请求。实际源/IR/生成物、私有 Lake 视图、原工具命令、
完整检查与失败证据沿 #635 的独立开发交接保存；当前尚无本包已登记的正式 CLI，
不能仅跑旧 `verify-stage1-protocol.sh` 报告本包已通过。公共 Lake/登记/覆盖候选由
主控协调；正确/错误/恢复原生验收交 ning。JSON/Serde 字节、标准 SQL/SHA、
事务未知/重启、真实资源及完整 V3 仍保持原义务。
