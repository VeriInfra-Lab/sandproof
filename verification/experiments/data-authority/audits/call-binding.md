# 重新派发的调用对应与剩余连接

[实验说明](../README.md) · [网关设计](../../../../docs/formal/08-data-gateway.md)

此记录针对 integration `0a23b57af60752f61067fc00b0b0b656e98347cb` 与本包隔离 overlay。它是源码审查记录，不是完整调用方的一般等价定理。实际编译/抽取/证明/应用测试由复现入口分别取证；生产未接入。

## 调用次序与同一目标

上一批冻结 `data_access.rs::spawn_data_relay` 仍持有原管理锁，依次检查旧撤销观察、配置、world 中的原记录与 VM/代数，然后执行凭据、寿命和 Access 检查，最后调用 Host 的 relay 派发。新 helper 没有预取后续数据库/Host 观察，也没有把不同读取时刻合成一个虚构的原子快照。

| 调用点 | 隔离核心的实际输入 | 审查到的保持与限制 |
| --- | --- | --- |
| `verify_target` 注册项 | 原 `data_credential(sandbox)` 返回的 `Option<(generation,[u8;32])>` 和许可 generation | 缺失首先记录 ReadCredential/Unavailable；错代数记录 CheckCredential/Mismatch 并在反向 SQL 前返回，保留原短路 |
| 反向绑定 | 原摘要查询返回项、同一 sandbox 数值与 generation | 两字段均比较；None/任一不同均拒绝。原查询错误继续先由 `map_journal` 返回 |
| Host 摘要 | `token_for(sandbox)` 的原字符串字节、原注册摘要 | digest 先检查准确格式，再调用原 SHA-256；比较完整三十二字节，未变更为调用方预先计算的有效布尔值 |
| 寿命 clock | 原派发处 `clock.now_ms()` 返回值 | 负时钟仍在 world 查询前拒绝；时钟提供方及单位是接口合同，不证明真实墙钟 |
| 寿命 record | 同一 sandbox 的查询结果；真实 Actor 匹配投影；六种 Phase 逐构造器对应 | SystemController 投影 None，寿命拒绝；actor 错误先于 phase 错误，缺记录更早。不是先假设 owner 有权 |
| 寿命 row | 原 SQL 六列、原 `identity::owner_id(owner)` 字符串字节 | owner 构造只在有行时执行，错误次序保持；字段位置、值与 Option 均传递，未只接受预验证 Lifetime |
| 到期 | 原同一次 clock 样本和该行校验所得 Lifetime | 缺行先于到期；`deadline <= now` 精确拒绝；不增加原实现没有的 startedSome 或 now>=started 条件 |
| Access | 原 DataKind→Exec/FileRead/FileWrite，Tenant(record.owner)，原 sandbox/VM/generation | 直接调用原纯核心；外层 ControllerInput::Access 包装和 ApplicationError 映射由审查与应用对照连接，未抽取完整控制器 |

隔离调用方 `data_credentials.rs` SHA256 为 `79ace2cacae4ab7ee5391f1123ee92c88e14052d86c38e144ba895e899d4c1c3`；`admission.rs` 为 `59ae7f5349c8543877082fed17d6a3ba8e4a3cbbaac502c68ea3ed71bc8d07ab`。完整 overlay 及其余输入由包内清单绑定，不仅靠这两个摘要背书。

## 原始行和重构行为

`read_lifetime` 的 SQL、WHERE 参数、列顺序与 Optional 错误传播保持原样。核心仍验证：owner 字节相等；deadline/startup 非负；disk bytes>0；deadline<=max；started 为 None 或落在 [0,deadline]。正 i64 值均可精确表示为 u64，原 TryFrom 成功分支与正值检查后的 cast_unsigned 对应。实际字段转换的证明属于同步核心，原 SQL 的关系含义仍单独待证。

原字符串相等改为 UTF-8 字节的全量 XOR/OR 比较，只承诺值相等的功能行为。它不证明原生机器上的常数时间、全部资源成本或密码保密。Lifetime 的字段、派生 trait 和公开再导出路径保持，新的实际类型位于纯核心。新增 phase 投影类型命名为 LifetimePhase，以避免原生成器对同末名类型生成全局同名实例；原 controller-core::Phase 及六种构造器对应不变。

Access 的 exec 自身不检查 phase/current_vm；组合中的这些条件来自外层目标和寿命检查。文件路径检查 workspace/access，不能误说成核对 VM。Access 的 authorized 接受 owner 或 SystemController，但此 App 调用明确构造 Tenant，不能用 SystemController 替代实际输入。

## 明确保留的自有义务

- SQL 选取的是同一 namespace/sandbox/owner；WHERE/参数/列索引、身份格式、代数 parse→print、原创建绑定及完整凭据映射正确。
- 凭据唯一索引、poison、登记与恢复、未知提交、CommittedWorld 发布及重放正确；外部事务原子性不自动证明这些业务。
- world、凭据和 lifetime 观察满足原单 writer/租约与锁合同。`live_lifetime` 的只读事务没有凭据读取那样的末尾 ensure_ready；本包不声称覆盖绕过租约的任意外部 writer。
- NodeHost→LinuxWorker→secret 的原路径/对象绑定、文件权限/替换、描述符配置、真实 relay 参数及进程身份正确；不能把整个自有 Host 算法当作 OS 合同。
- 时钟检查到实际 I/O 间的边界、持续撤销、旧持有者计费、取消后收尾、Unknown、精确退出及物理排空需后续连接。

## 应用对照及其限度

原版和候选安装完全相同的独立测试模块及 cfg(test) 观察桥接，复用原应用夹具与 SQLite。原集成测试文件、原业务断言及默认编译入口保留。受控 Host 统计的是 trait 的真实 relay 派发调用，不启动产品中继。

Closed 负例通过显式测试 feature 克隆缓存 World，只将一个 record.effective 设为 Closed，再核对 Running/VM/代数/旧 watch 保持。原 SQL/index 检查不读取 effective，因而正确版确实到达能力拒绝，错误版确实到达 Host。测试不使缓存失效或从 SQLite 重载，不修改 checkpoint validator，不声称该故障状态可达。

这些有限业务对照与本记录支撑实际参数、拒绝顺序和调用的连接；它们不代替原始实现到拆分实现的任意执行等价证明。可用的最终结论必须显式依赖 A/B 接口合同及上述调用绑定，不能表述为完整异步 Rust 系统已经机器证明。
