# 收件记录返回位置与完整查询组合候选

[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [统一记录合同](../inbox-record-total/README.md) · [原查询合同](../inbox-state/README.md)

生产仍固定 `a9c9967889c4cf298e4378c7106948018835fc64`，父证明提交为 #716 的
`c387b3c0e5b38a134931e0b334c845e0864a83f8`。本包在**同一份**固定 Charon/Aeneas
生成视图中抽取原隔离 record 重构候选与未改动的 Inbox::state；两个透明薄根各调用
真实方法一次。不导入两套同 namespace 的生成体，不复制记录/查询算法，不改生产。

原 FullStateContract 要求两个索引有效。这里从原 RecordPost 的全部成功分支推出
这个前提，并保留完整 intent：

- 保留重放：原 entrySpec/inboxSpec 的首次完整命中给出实际列表前缀和命中项。
  从它推出两个位置可读且 Entry.intent 完整相等，不要求 key/session 唯一。
- 新会话：FreshPost 的 sessions 追加和单项 entries 给出返回旧长度/0 的位置。
- 已有会话：ExistingPost 的有界 Vec.set 和 entries 追加给出返回会话/旧条目长度的位置。

StoredIntent 只是精确位置到原 Session/Entry 的表示关系。stored_intent_bounds 将它
接到已有 ValidPosition，保留完整 AcceptedLifecycleIntent 相等。FullRecordStateContract
同时保留原 RecordPost（包括所有拒绝与整个 Inbox 状态），并对每个成功返回位置给出
索引界、完整 intent 及原 workState 的全部七字段查询值。composed_implies_record 明确
说明新合同包含原完整合同。实际 record_state_contract 在同一生成视图中实例化
record_contract/state_contract；调用者无需再外部假设 record 返回索引安全。

正常 Create、已有会话 Unload、旧 Create 保留重放沿用原 FreshPost/ExistingPost/全文
查找和合法封套规格，分别给出实际记录结果和完整查询值；旧 Create 在版本推进后
current=false，仍保留自身 operation。真实 Rust current=true 与 local_operation=None
错误版沿用 #716 的变异及完整查询规格，并否定同一个 FullRecordStateContract。
负例不导入正确的 state_contract 或 record_state_contract；原 record 正确性仍可复用，
因为这两份变异只改 query。有限见证不代替对任意已解码输入的通用位置证明。

本包仍不证明 record_lifecycle_intent 的事务外层。其 load/authenticated、Destroy
本地效果及 operation 映射、codec.save/commit、未知提交/重启另列；外层在 record
和 state 之间修改本地字段时，必须另接实际状态关系，不能直接拿本组合代替。
查询值不表示授权、执行/释放许可、持久接受或资源存在。

## 假设提供者与验收

沿用父包逐符号 String/Vec/Option/算术/clone 合同；列表前缀和可读位置的数学引理
不新增 Rust 库假设。同一视图仍只有原 record 的九个外部函数绑定；state 本体在
候选准备前后逐字核对，ContractError 原完整枚举在采用既有别名前重新核对。
V0/主控负责库调用与公共登记采用，V7 提供独立认证上下文，V1/V2 提供当前意图及
耐久发送，V4/V5 提供本地实际事实，V6 提供文件/VM 事实。32 字节摘要仍是值。
record 重构的生产采用及原版/候选机器等价仍未闭合；原生对照支持审查，不代替等价证明。

native.py 在五份私有源上检查原版、候选、两错误版和恢复版。测试用实际 record
返回的 (s,e) 调用 state，比较完整字段并核对纯查询不改 Inbox，同时覆盖非零 session
位置和首 key 冲突。local 元数据是值夹具，不声称建立了真实资源。各版必须实际编译、
通过 Clippy；错误版必须到达同合同断言，不能用编译失败、超时或导入失败计为拒绝。

formal.py 保留原记录/追加/统一记录的正确目标，加入两个真实根、完整 query 与本包
目标；正反的 Model/Values/表示关系相同。每版重新抽取，通过原 actual_files 和
proof_services 检查整个真实 Lean 闭包，核对完整非 stale/partial 诊断及依赖审计。
prepare-only 仅用于开发抽取准备，complete=false。新增目标须随公共候选进入原
verify-stage1-protocol.sh；不能只运行旧根来报告新证明通过。

外层持有共享重锁，每版使用新的输出及独立 .lake/build，保留缓存和失败证据：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-position/native.py --output /private/position-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-position/formal.py --development-native /private/position-native/result.json --variant correct --output /private/position-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-position/formal.py --development-native /private/position-native/result.json --variant always-current --output /private/position-current
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-position/formal.py --development-native /private/position-native/result.json --variant drop-operation --output /private/position-operation
```

本机结果为开发证据；正式 --require-native 必须取得 ning 的同批 functional 和
stage1-inbox-position 回执。coordinator-candidate.patch 接在 #713–#716 的公共候选后，
保留所有旧目标，只提供公共原生登记/入口/覆盖/依赖的增量，不直接改公共配置或冻结。
verify-controller.sh/proof_native_suite 由主控同批执行与复用。本切片不是完整 CI、
ning/shared 验收，也不是完整 JSON/TLS/handler、持久收件或完整 V3。
