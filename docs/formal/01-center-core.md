# V1 中心纯决策：身份、节点、预留与意图

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：详细证明计划；拟定定理尚待建立。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

中心是全局绑定和资源预留的唯一权威，但不执行 PrepareVm 等本地子步骤。纯决定无网络或数据库副作用，能够独立抽取；将它与 Redis 和宿主分开，修改调度策略时不需要重证物理停止算法。

## 1 实现入口与所有权

| 代码 | 入口与责任 |
|---|---|
| [model.rs](../../sandbox-center-core/src/model.rs) | `CenterState::new/is_valid`（校验定义在 [validation.rs](../../sandbox-center-core/src/validation.rs)）、`reserved/pending/observe_node/refresh_node` |
| [decision.rs](../../sandbox-center-core/src/decision.rs) | `create/create_builder/renew/destroy/apply_response/compact_confirmed` |
| [window.rs](../../sandbox-center-core/src/window.rs) | `begin_drain/confirm_terminal/begin_close/apply_close_ack`；退休位置不等于物理宿主位置 |
| [cache.rs](../../sandbox-center-core/src/cache.rs) | 不可变模板可用事实作为准入输入；模板子系统不在自有证明范围 |

拥有 `Cc`：deployment、bounds、节点登记、租户预算、两类高水位、sandboxes、retention_lanes。Sandbox 的 operations 保存完整原请求/结果，另有 terminating、released、首次运行时间/期限、观察版本和 terminal_confirmation。缓存/Redis 字节由 V2 拥有，物理效果由 V6 拥有；V1 不以本地枚举冒充节点事实。

## 2 输入、规则和拒绝

| 输入 | 接受与更新 | 必须处理的拒绝/未知 |
|---|---|---|
| 创建（含已获批的构建池角色） | 合法租户/形状/模板；选择协议、角色、健康、空闲否决及预算均合格的原节点；分配永久身份、保留位置及清理/结果空间，产生原高级请求 | 无节点、容量、格式、身份耗尽分别返回真实类别；不得静默换模板或池 |
| 续期/销毁 | 原绑定与最新意图；单调版本；保留销毁专用操作位置；destroy 不改绑 | 终止/已释放/错误身份/编号耗尽；旧续期不覆盖销毁 |
| 接收事实 | 全身份、种类、版本、原参数相符；采用合法新事实，原时间起点不重算 | 迟到/冲突事实不能改变其他操作；无回复保留原 pending |
| 确认/压缩/关闭窗口 | 中心已有原结果、节点确认及窗口顺序条件成立；保留必要锚点和拒绝前沿 | 发出确认不等于节点已收；健康过期不释额；闭窗回执错配不退休 |
| 节点观察 | 更新有限健康/可用描述，原节点身份和受保护登记不变 | 探测成功不是执行/释放证据，外部空闲只是否决项 |

模型为 `δc(Cc,input)`，含原守卫顺序及不修改其他绑定的条件。完整字段枚举和 `is_valid` 均需对应，不能只证明一个核心 Map。未知/未完成操作继续计费；上下游事实合法性由 V3/V5/V6 通过 V9 提供，不作为无来源假设。

首批创建按实际 `create_for_role` 固定守卫顺序：状态/描述/选项/启动期限有效 → 实例记录容量 → 租户存在 → 平台和租户各分量可加且不超额 → 合格节点/可用保留位置 → 两种高水位可加一 → 为全部未终止实例保留未来销毁编号。首个失败决定Invalid/Capacity/NotFound/Unavailable/Exhausted，不将它们统一成“容量不足”。节点在合格集合中按最大资源使用比例排序，以node_id确定同分顺序；比较用实际u128交叉乘法，另证表示范围。

定义 `Reserved_c(scope,r)=Σ{shape(b,r) | b尚未released且属于scope}`，分别量化平台、租户和节点scope。接受新增形状要求各分量可表示且 `Reserved_c+shape≤Limit`，物理free仅作额外否决。`pending`投影必须保留原请求完整字节/身份；缺失结果不是确认失败。`R_c`逐字段连接注册、窗口/前沿、原请求/结果、期限/观察版本和高水位；成功输出是 `(Cc', 原LifecycleRequest)`，拒绝时Cc不变。

## 3 待证义务

| 拟定定理 | 结论 |
|---|---|
| `center_init_valid` | 通过真实 new/validation 的状态对应合法 `Init_c`，含未来结果包络与租户/节点配置限制 |
| `center_step_preserves` | 每种接受/拒绝保持唯一固定绑定、完整操作、单调终止、计数器和窗口约束；修改范围精确 |
| `center_reservation_covers` | 所有未释放绑定与未来销毁/回执/退休空间均有预留，平台/租户/节点各分量不超限 |
| `center_response_monotone` | 旧观察/终局冲突不回退结果、期限或终止；原 started_at 只确定一次 |
| `center_retirement_safe` | 压缩保留原创建/当前意图/必要续期锚点；闭窗后旧请求不成为新请求 |
| `center_decision_refines`、`center_decision_complete` | 实际纯 Rust 的完整成功后继、首个拒绝类别及全部合法可表示输入，与模型一致 |

主要性质 S1/S3/S5/S7/S8/S10/S14、F1、DS1/DS2/DS4。V1 只能证明逻辑预留；“节点真实责任被覆盖”和“可真正释放”由 V9 连接，不能由 released 布尔值自证。

## 4 批次与依赖

### 首链回执分类子批次

`CenterState::apply_response` 在原状态校验、租户/实例查询及最后原操作选择之后，实际调用 `Operation::classify_response`。这个只读判定按原顺序检查：回执与原请求的身份/事实兼容性（错误映射为 Stale）→ 完整回执相等时返回 Duplicate → 非重复的旧观察版本拒绝 → 非推进的事实等级拒绝 → Advance。Duplicate 由调用者返回原状态克隆；Advance 才进入原状态更新及后继校验。检查拆分不改变持久化、归属查询或物理证据语义，也不代替原请求的准入合法性检查。

独立 [ResponseModel](../../SandboxModel/FirstChainLinux/ResponseModel.lean) 使用自然数观察版本及完整身份/事实载荷；[ResponseClassification](../../SandboxModel/FirstChainLinux/ResponseClassification.lean) 连接真实生成的相等字典和判定结果。Recorded、Stopped 的等级分别为 0、1，其余为 2；等级只用于同一个原操作内的推进，不作为回执相等，也不阻止之后建立新的生命周期意图。是否已经通过原请求检查作为由既有生命周期定理提供的输入，不是额外的可信自有函数。

本批目标是完整分类结果、重复/推进的接受充要条件、身份优先拒绝、旧观察/非推进事实拒绝及完整身份保持。非空见证只说明此判定存在可接受输入，不说明已通过中心准入或存在虚拟机。具体状态及适用版本见[覆盖记录](../proof-coverage.md)。

完整中心入口仍需独立对应 `is_valid`、首次实例匹配、原操作选择、全字段更新与后继校验；Redis 提交、节点事实真实性及宿主清理由相邻模块提供。本子批次不能替代这些连接，不能由 Duplicate/Advance 推导已提交成功或资源已经释放。

首批 C-create/response/destroy：固定一类已授权模板描述，任意预算内登记节点，涵盖无节点/耗尽、重复/迟到事实、丢回复及销毁先到。纯步骤没有崩溃中间点；V2/V9 建模提交前后，不能为了简化把网络派发合并成纯一步。

后续分两批：续期/查询相关事实次序；每位置结果确认/窗口退休、构建池准入的核心接口和完整容量包络。模板业务本身继续排除。

依赖 V0 的身份/数值；模板 A10；V2 提供持久版本，V3 提供认证事实，V9 证明这些调用前提。并行工作可使用已固定的抽象输入合同，最终必须由对应模块实例化。交付及开工表沿用 [V0](00-foundations.md#5-开工与完成条件)，不把“pure crate 测试通过”记作抽取/证明通过。
