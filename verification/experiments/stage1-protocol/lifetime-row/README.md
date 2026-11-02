# 期限行值校验

[协议证明范围](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包对应真实 `controller::admission::lifetime_from_row`。它由原 `read_lifetime` 的六列校验拆出，原调用仍先执行相同 SQL、相同六次类型读取及 optional，再在有行时映射 owner 并调用 helper。没有行不调用 owner 映射；SQL 和类型错误仍先于业务校验。

`Spec.lean.in` 的同一 `FullRowContract` 用于正确版与三种真实 Rust 错误版。合同保留完整 owner 字节、非负 deadline/startup、正 disk、deadline 上界和可选开始时间，并精确返回全部五个 Lifetime 字段；不要求 startup 小于 deadline。`Core.lean.in` 给出一般返回、接受/拒绝充要条件、总性及完整字段结论；合法行与错误输入见 `Values` 和 `Witnesses`。错误版分别漏 owner 比较、漏 startup 拒绝、丢失 started_at。

`FunsExternal.lean.in` 的 Option 连接复用已有 A6 回调合同；String/&str 的 ne 和 i64→u64 连接是固定标准库使用点候选，公共唯一登记仍交维护方。字符串比较使用同一完整字节表示，转换的使用合同仅要求非负 i64 时返回精确 u64；真实守卫已先拒绝 bytes <= 0。真实两版标准库 TryFromIntError 都含 IntErrorKind，而固定后端使用 Unit。此处错误分支不可达且原 map_err 不观察载荷；模型中的负数错误只作域外定义，不宣称一般错误载荷表示忠实，也不扩展成其它调用点的合同。三类错误值类型复用 BoundedWriter 已有表示；没有给 SQL 或业务函数增加 opaque。库符合性仍是显式 A6 条件，Lean 定理不证明第三方库内部实现。

独立开发检查：

```sh
python3 verification/experiments/stage1-protocol/lifetime-row/native.py --output artifacts/lifetime-row-native
python3 verification/experiments/stage1-protocol/lifetime-row/formal.py --output artifacts/lifetime-row-formal --lock artifacts/.proof-heavy.lock
```

形式入口重新抽取真实函数，逐字核对生成物，检查全部实际文件、原服务完整诊断及依赖审计。Native 入口使用同一组真实 SQLite 行断言检查原版、正确版、三错版与恢复版。普通及正式原生验收交 ning；公共 Lake、共享入口、冻结、依赖和覆盖只提供候选差异。

本切片从已读出的六列和给定 expected_owner 开始。实际 SQL/列来源、owner 映射调用连接、同一事务、提交未知与重启、当前意图和授权、物理资源事实及 native_snapshot 的 World 来源仍需各原责任方闭合。它不证明整个 read_lifetime、持久收件或完整 V3。原实现的失败 IR、原日志与原成功证据保留，不作为性质反例或新源验收。
