# 协议 4 封套的完整字段表示

此候选接续 [角色叶子](../schema-leaves/README.md)与[实际接线](../json-wire/README.md)，
固定原生产提交 a9c，复用原 Request 生成类型、Replay、Validation、Accepted 和 Recorded
合同。范围与提供方见 [V3 计划](../../../../docs/formal/03-protocol-inbox.md)及
[依赖登记](../../../../docs/formal/dependencies.md)。完整 V3、JSON parser 和共享验收未由此完成。

`EnvelopeSchema.lean.in` 为实际生成值定义声明顺序的输出字段表示，复用公共 JsonOutput
及已有 Node/Create/Resources 字段配方。它不复制业务算法，也不另造协议状态模型。
一般性单射定理覆盖所有实际字段、五种操作、全部摘要/选项字节及 Recorded 的 terminal；
不以有限见证代替全文相等。相等表示连接原完整 replay 合同，保留独立 context 下的原
守卫结果和首错；不同 intent 表示由原 recorded 合同拒绝。正常五分支及遗漏 capture、
有符号时间、布尔值、retention window 和 terminal 的见证使用同一表示与原规格。

表示定义适用于全部生成类型的可表示值，不把解码后普通整数加强为非零。
Renew 的 end_at_ms 是 i64；ExpectedSession::Version(0) 仍是合法表示；slot 和
accepted_version 的业务拒绝仍由原守卫负责。retention.window 不要求等于 key.window。
terminal 被序列化保存，但 Recorded::matches 不因此认证它。32 字节摘要不证明计算了哈希。

这是**完整字段表示证明**，尚非实际 Serde 派生到该表示的一般性实现证明。
源码/属性清单、原生回归和原 LLBC 是接纳 A6/B4 精确使用点的审查材料，不是新的业务
公理。固定原 Charon/Aeneas 对真实泛型 encode/decode 另做预检；翻译失败原文保留，
不记为性质拒绝。没有把整个 Serde、HTTP handler 或收件事务设为可信。

标准输出均为对象；合法输入不能据此限制为对象。当前固定 Serde 对五种内部标记操作
也接受以 kind 开头、其余字段按声明顺序排列的序列，Destroy 为 `["destroy"]`。
ExpectedSession 的 Absent 可带显式 `version:null`，Version 可用 `["version",0]`。
真实回归核对这些形状、长短序列、对象未知/缺失/重复字段、每个原字段变异、所有自有
叶子类别及守卫首错。有限回归不替代任意输入的解析证明；容器收尾、错误位置、具体
SliceRead 字典与字段现场、正常编码完成、字节预算、真实 Writer 接线仍须后续闭合。

## 检查与交接

`shared-candidate.patch` 只给公共输出词汇增加 signed/boolean 两个构造器，在私有副本
应用，交 V0/主控采用；不直接写公共文件或冻结。普通完整和原生验收仍归 ning：

```sh
cargo test --locked --offline --manifest-path verification/experiments/stage1-protocol/envelope-schema/Cargo.toml
cargo clippy --locked --offline --manifest-path verification/experiments/stage1-protocol/envelope-schema/Cargo.toml --all-targets -- -D warnings
```

证明机持共享重锁，使用分配的固定工具、包缓存和私有构建目录：

```sh
python3 verification/experiments/stage1-protocol/envelope-schema/formal.py --output /private/envelope-fields
```

驱动复制完整源码、核对字段/类型/顺序和审计清单，重新抽取原 request 视图，比较完整
冻结 LLBC/生成物，保留真实 Serde 两根的预检，再经原完整物理文件服务和内核检查导入
闭包。旧 `frozen.json` 仍保留协议 3；#686 已重新生成当前协议 4，差异为版本常量和
原源码行号。本候选重新核对当前完整 LLBC/生成物，记录旧冻结差异，保留公共冻结原字节，
不替主控签发迁移或整套验收。`complete=true` 只表示该局部字段表示候选通过。`--prepare-only` 不运行 Lean/
抽取，返回 `prepared=true, complete=false`；输入变化、缺失审计或差异不符均失败。
它不签发原生封口，不重跑主控的共享完整套件，也不删去原四种 Rust 错误版。

新目标须由主控加入原 `verify-stage1-protocol.sh`、公共模块/原生登记和覆盖记录。
这些接纳项未完成时，不能仅运行旧入口宣称新版通过。与 #702 的七角色回调和原
SliceFieldReader 的完整组合、标准派生原字典实例仍是下一步；持久收件及外部事实仍按
V1/V2、V4/V5、V6、V7 的原所有权分别交付。
