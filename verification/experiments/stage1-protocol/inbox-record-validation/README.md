# Inbox 记录、完整校验与保存的组合边界

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包固定 a9c 的生产源码，组合既有 record 与 validation/save 隔离重构候选，
重新抽取同一根闭包。生产只读，原候选一般等价、标准提供者连接和生产采用仍待闭合。
合并保留原检查、字段、顺序和返回值；唯一人工合并点是在同一 Session 位置保留两组
既有 helper。`production-candidate.patch` 只供工程审查，不含证明转发根。

原 `InboxRecordTotal.FullRecordContract`、`InboxValidation.FullInboxContract` 和
`InboxSave.FullSaveContract` 保持不变。新的 `PreservesValidation` 是额外的转换保持
命题：完整合法前态、合法请求封套及 record 成功，是否足以推出后态完整合法。
`record_does_not_preserve_validation` 给出其反例，不将错误实现的性质换成另一套规格。

完整见证包含受保护配置值、匹配的 host observation、owner 映射与 high-water、
retention lanes、Create 条目、本地绑定以及已解析的 Unload 条目。前态完整校验通过，
新请求完整封套通过，实际 record 返回精确的整份后态和索引。后态的所有条目扫描也
通过；最终 lineage 在新 capture 不大于既有 frontier 7 时拒绝。大于 7 的任意 capture
通过这组见证的完整校验，7→10 是允许编号空洞的正常对照。这是一族完整状态见证，
不是任意收件状态的转换保持定理。

`stale_capture_save_rejected` 对任意保存效果字典和初始效果状态成立：此类后态被
保存校验拒绝并返回原效果状态，未调用编码、digest 或 UPDATE。它只描述 save 调用
内部，不能推出其调用事务更早没有效果，也不能从外层失败或发散推出回滚。
`fresh_capture_saved` 使用显式编码和效果提供者给出合法后态的完整保存轨迹。
`successful_save_requires_validation` 对表示范围内的任意 Inbox 证明保存成功必须先
通过同一完整校验。回调满足性见证不表示标准 Serde、哈希或 SQLite 已被证明。

## 开发与验收入口

`native.py --output ...` 检查原版、组合候选、四个真实 save 错误版和恢复版，复用
既有 record、append 与 save 断言，另检查 capture 6、7、8、10 的实际记录、完整校验
和 SQLite 行。错误版须真实编译并到达精确失败断言，格式或编译失败不计为性质拒绝。
本地开发测试和 ning 的普通／原生验收分别记录。

`formal.py --variant correct --development-native .../result.json --output ...` 在私有
目录重新抽取 record、完整 Inbox 校验、save 和既有独立原语根。四个错误视图复用
相同完整保存合同，不导入被变异函数的正确证明。每个视图检查三条透明转发、完整
生成函数清单、全部真实文件的内核／同服务诊断与依赖审计。执行者持有共享重锁，
使用独立 `.lake/build`，保留失败和缓存；完整检查拒绝 stale/partial 结果。

`--require-native` 只消费 ning 对同一输入的完整功能和组件回执。公共登记、根入口、
冻结及覆盖记录由主控采纳候选增量后统一验收；本包局部检查不代表共享整套、E2E 或
完整 V3 通过。原有反例与历史候选证据继续限定于各自精确输入。

公共候选同时补足本次组合闭包所需的既有 record/append 审计登记。复用的声明不计作
新增定理；`InboxAppendPredicates` 只补直接导入，算术和完整请求规格保持不变。

## 后续义务

重复或较旧 capture 被 record 追加不表示已经耐久接受。V1/V2 需要提供当前意图和
捕获编号相对保留 frontier 的业务条件；仅不重复不足以保证严格增加，也不要求必须
加一。V7 提供独立认证来源及受保护配置。V4/V5 提供本地接受、结果与同一事务中
校验、保存、提交、未知提交和重启恢复的事实；V6 提供实际文件、VM 与清理事实。

本包继续使用依赖登记中的逐符号合同。已解码值和显式效果接口不包括真实 JSON/TLS
链、完整 HTTP handler 或收件事务；32 字节摘要值也不证明计算了哈希或材料完整。
