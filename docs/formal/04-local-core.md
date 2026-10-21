# V4 本地生命周期状态与纯决定

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#636](https://github.com/VeriInfra-Lab/sandproof/issues/636)。状态：第一阶段实施与证明计划；代码定位基线 `56b14d64`。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 从旧核心到新合同

对应 [controller-core](../../controller-core/src/lib.rs)。旧核心存在 Pause/Resume、快照状态和定理，但当前真实执行未启用恢复；`handle_compensation` 对 Pause/Resume 可走终止收尾，`reserve_pause_snapshot` 使用逻辑 manifest 计数。必须重审这些分支，不能靠放开旧产品子集就宣称满足状态连续性。

本模块拥有逻辑 `Session`、`Operation`、`Capture`、运行资格、预留和逻辑工作；V5 提供持久性，V6 提供物理效果。不把持久记录和真实 VMM 状态定义为同一对象。

## 2. 关键投影与不变量

- `Recoverable(c)`：对应会话/源代的完整不可变 bundle 已按合同提交且尚有合法恢复资格/责任；包含内容匹配与外部持久条件，不只是 `stage=Durable`。
- `MayHaveExecuted(i)`：在任何可能激活前先设的单调事实；可保守为真，不可由随后停止/paused/超时改回假。
- `SafeToReuse(slot,old)`：原 VMM/助手与相关工作不再能读写该运行位置或取得新资格；不是只有逻辑 phase 终止。
- `Tracked(effect)`：可能发生的效果有持久计划/身份/额度和恢复责任；V5/V6 分别提供存储/物理连接。
- `RestoreEligibility(c)` 与 `Pins(c)` 独立。资格只产生一个可能执行的后继；资格消费不能隐含清空文件引用。

## 3. 必须证明的转换

| 转换 | 规则与错误边界 |
|---|---|
| 接受暂停 | 原 Running、当前版本、关闭新授权、预留捕获与收尾；不能先释放原运行额度 |
| 捕获/提交/卸载 | 同一次冻结截面、指定目标、完整不可变来源先接管，旧 VMM 后退出；真实释放后才 Unloaded |
| 暂停失败 | 普通失败保住唯一原 live/frozen 状态；撤销新 capture 资格且迟到任务失效后才回原 Running；提交未知先对账 |
| 接受恢复 | Unloaded、原 VMM 已退出、当前来源/寿命、唯一 claim 和材料 pin；新运行代不等于新逻辑会话 |
| 装载/激活 | LoadPaused 后先记 MayHaveExecuted 再激活；标记后只继续原候选，不回退重载；原 VMM 非预期死亡报告不可恢复 |
| 发布 | 精确原对象 Ready 与当前资格同时成立；销毁/到期否决新成功发布，不抹掉在途责任 |
| 终止 | 单调撤销未来执行/访问并收尾；显式 destroy 可放弃状态，但不能把普通暂停错误伪装为用户终止 |
| 材料退休 | 关闭新引用，与 pin 获取同原子决定；删除必须资格无效、引用为空且旧工作无法复活 |
| 预算转换 | 运行、保留材料、临时重叠、操作结果及收尾逐分量保持；未知不能释放 |

具体 API 行为唯一以模块合同为准。本模型不重写文件存储或调度器，也不假设所有动作一次成功。

## 4. Rust 对应和反例

对实际新增/重构的决定、回调与补偿生成 Lean，建立字段完整 `R_local`；初态非空、每步保持不变量、拒绝/错误路径保持责任、支持域正常接受。旧 `local_product_subset_closed` 排除 Pause/Resume 的结论不能沿用到新范围；旧定理可按前提重用，覆盖记录必须换精确候选。

至少让三个真实错误版本被同一性质拒绝：暂停捕获失败杀原 VM；激活未知后重载旧 capture；GC 检查后不受保护地删除。负例必须编译且能走到错误，不以抽取失败为安全证明。V4 负责 LC-P1/2/6/7/8/9/10/11 的纯决定，V5/V6 接真实效果，V9 证明全历史。
