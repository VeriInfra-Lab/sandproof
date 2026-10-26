# 原收件查询的完整值合同候选

[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [统一记录合同](../inbox-record-total/README.md)

固定生产提交 `a9c9967889c4cf298e4378c7106948018835fc64`，父证明提交
`cee7f384a90d603e8f39508f90ce30bccfa04ba4`（#715）。本包直接抽取原
`controller-service/src/controller/lifecycle_inbox/model.rs` 的 `Inbox::state`，
只在私有副本加单次调用的透明薄根，**不应用 record 的生产重构候选**。
生成体由固定 Charon/Aeneas 产生，不手改；生产源码及公共配置保持只读。

`FullStateContract` 对任意已解码、按生成类型表示的 Inbox 和两个有效索引量化。
它精确保留七个返回字段：完整 intent、会话当前版本、条目版本等于会话版本的
`current`、terminal、refusal、local，以及被选 Entry 自己的 local_operation。
`state_contract` 接原两次 Vec 索引与父包完整 AcceptedLifecycleIntent clone 证明；
`valid_position_returns` 给出有界读取必返回完整查询值的结论，不对越界读取作保证。
正常当前项、旧版本项的见证共享原封套有效性规格；旧项 current 为 false，
新项可以没有 local_operation，即使会话 local 中已有 operation。

真实 Rust 错误版分别把 current 固定为 true、遗漏条目自己的 local_operation。每版须编译、实际重新抽取并到达同一个 `FullStateContract` 的反例。
正反共用同一 Model、Values、完整 clone 和库绑定；负例不导入正确 state 定理。
有限见证用于说明和发现字段遗漏，通用证明覆盖全部字段与任意有界索引。

本切片只说明查询值，没有执行/释放授权、当前资源存在或持久接受含义。配置和独立
认证上下文来源仍属 V7，当前意图及耐久发送属 V1/V2，本地实际接受/结果属 V4/V5，
文件/VM 属 V6。ContentDigest 仍是值，不声称计算过哈希。
统一 `RecordPost` 到返回索引有效性的推导及 record→state 同一生成视图组合，
留给下一切片；本包没有把它们作为已完成结果。
实际 record_lifecycle_intent 的 load/authenticated、Destroy 资源映射、save/commit、
未知提交/重启及完整 JSON/TLS/handler、持久收件和完整 V3 继续开放。

## 库条件及验收入口

依赖沿用既有逐符号合同：固定 Aeneas Vec.index_slice_index/index_usize_spec，
标量比较，以及原 AcceptedLifecycleIntent clone 所需 String/Vec/Array clone。
五个 FunsExternal 绑定逐字复用已登记的 String 相等/clone、Vec.is_empty、Bool.ne、
Option 字典比较。原查询闭包不含 ContractError，无需错误枚举别名或新业务 opaque。
工具转换和标准库语义仍是原依赖边界；V0/主控负责调用采用及公共登记。

原 native.py 在四份隔离源上编译原版、两错误版和恢复版，复用相同四个完整值断言，
另检查 Clippy、原协议/服务 Cargo artifact、注册库和全输入闭包。formal.py 对三版
重新抽取并核对实际 state 薄根、协议根、透明业务体和固定五个外部符号，再通过原
actual_files/proof_services 检查全部实际 Lean 文件和审计。抽取准入回归不能代替
业务反例；prepare-only 只表示已准备，complete 始终为 false。

外层持有共享重锁，每次使用新的私有输出及独立 .lake/build，不清缓存：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-state/native.py --output /private/state-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-state/formal.py --development-native /private/state-native/result.json --variant correct --output /private/state-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-state/formal.py --development-native /private/state-native/result.json --variant always-current --output /private/state-wrong-current
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-state/formal.py --development-native /private/state-native/result.json --variant drop-operation --output /private/state-wrong-operation
```

本机结果为开发验证。正式 --require-native 必须取得 ning 的同批 functional 和
stage1-inbox-state 回执；完整 verify-controller.sh/proof_native_suite 由主控统一执行与复用。
coordinator-candidate.patch 是接在 #713–#715 公共候选之后的增量，保留全部旧目标，
本包不直接写公共入口、覆盖记录或冻结。局部通过不等于 ning、共享验收或完整 V3。
