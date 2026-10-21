# V6 真实捕获、装载、激活与精确回收

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#638](https://github.com/VeriInfra-Lab/sandproof/issues/638)。状态：第一阶段实施与证明计划；代码定位基线 `56b14d64`。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 实现范围

对应 [sandbox-node](../../sandbox-node/src/linux.rs)、[sandbox-runtime](../../sandbox-runtime/README.md)。当前 `sandbox-node/src/linux/execution.rs` 的 `PrepareVm` 明确拒绝 snapshot source，不能将历史 Freeze/Seal/Commit 命令或模型记为真实支持。本阶段新增固定 Firecracker/jailer 1.16.1 的完整路径，原身份/围栏/受保护目录及清理继续复用。

新增外部接口合同仅为[依赖登记 S-C1–6](dependencies.md#第一阶段新增依赖合同)。选择哪个 manifest、原 VM 是否允许停止、何时激活和哪些文件允许删除属于自有业务。外部工具可返回错误但留下部分效果。

## 2. 实际动作与证明接口

| 动作 | 真实输入/输出 | 必须对应的义务 |
|---|---|---|
| PauseOriginal | 原 VMM/API/运行代/围栏，确认 paused 或未知 | 不操作同名新进程；暂停确认和当前停止意图分开 |
| CaptureFull | 原 paused VMM、唯一 capture 目录、Full RAM/device 和所有匹配 disk | 整个截面无运行/宿主写者；不把内存文件等同完整恢复材料 |
| SealBundle | 原文件句柄、长度/摘要/配置/manifest、sync 与受保护发布 | 短写/替换/同步失败不成功；逻辑计数器不能替代内容摘要 |
| LoadPaused | 唯一 claim、新候选/私有盘、File backend、resume_vm=false、已取得 pin | 使用指定完整 bundle；新 jail 保持设备相对路径；不自动运行、不写坏来源 |
| ActivateOriginal | 已持久 MayHaveExecuted、原 candidate、当前围栏/寿命/凭据 | 只激活该候选；重复投递不创建新候选；旧任务不能绕过终止 |
| Observe/StopOriginal | 精确 VMM/辅助进程及工作身份 | Exit/Stopped/排空/Released 分开；PID/路径缺席不能证明无迟到作用 |
| DeleteBundle | V5 删除许可、精确对象、无 pin/映射/可复活任务 | 只删授权原对象；部分删除保持原责任，实际释放后才退额度 |

物理效果必须接原日志/尝试/进程身份的持续观察。文件从路径取得原句柄到校验/使用是一个安全链；Metadata 条件证明不能代替 open_checked 的路径/FD/竞态证明。需先解决 V0 T8 的可抽取拆分或明确阻断，不能扩大可信业务层。

## 3. 特殊失败规则

完整 bundle 接管前不得停止原唯一状态；回滚暂停前先让迟到 capture 工作失效。LoadPaused 返回后，内存 backing 仍被 VMM 使用，文件不能改写或逐出。设置 MayHaveExecuted 后，即使后来看到 paused 或 stopped，也不能允许重载旧来源。

停止/回收以原身份驱动且可重试。manager 重启保留运行 VMM，不能以服务停止代替 VM 清理。unexpected VMM death 后的状态保证按声明故障域报告失败，不以冷重启补成功。

## 4. 验收与边界

同步守卫、参数、identity/path/manifest 校验、分类与精确清理决定经固定抽取；系统调用/FC/文件持久性用显式外部合同加实机证据。Rust 业务分支仍需机器对应，不能仅源码检查后称实现已证。

真实 KVM 必须观察 RAM-only nonce、磁盘内容、长生命周期 guest 进程、旧 VMM 退出、新 VMM 身份以及后续修改；无旧 VM 留存占额。故障窗口覆盖捕获/发布/停止/加载/激活/删除；File backing 生命周期和磁盘释放单独观测。提供 LC-P4/5/6/7/9/10 的物理部分、LC-I6/7/8；结果绑定固定 CPU/内核/FC/文件系统。无 UFFD、增量链、迁移或任意设备组合支持承诺。
