# 原 Create 附接与事务使用点预检

[V3 组件](../README.md) · [权威范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖](../../../../docs/formal/dependencies.md) · [工作流](../../../../docs/proof-workflow.md) ·
[任务 #635](https://github.com/VeriInfra-Lab/sandproof/issues/635)

固定基线为 `ae8e5a564ab3d81652a23acb2b99e242193fa3a2`。本包提供实际入口的
固定 Charon/Aeneas 预检和固定格式参数模型的信息丢失引理；没有原附接/身份构造的
语义证明、新可信合同或原生验收。
不修改生产实现、公共配置/登记/冻结/覆盖或当前 AE+65 封口，不创建迁移 PR。

## 真实入口与复用

| 模式 | 原函数与单次转发 |
|---|---|
| attachment | `lifecycle_inbox::attach_create(tx, namespace, attachment)` |
| world | `WorldTransaction::world(&self)` |
| identity | `identity::sandbox_id(namespace, sandbox)` |

每个私有源副本只在原模块末尾增加对应转发根，不重写业务函数，也不将自有 codec、
事务或 handler 设为 opaque。准入要求原调用唯一、参数数量/返回位置保持、真实被调
函数有非外部体；翻译成功还必须保留原生成函数，不能只保留载体或转发别名。
所有生成文件原样保留，含 axiom 的外部模板不导入，不据此签发 Lean 成功。

全文查询复用 `InboxLookup.find_contract`：原会话/条目顺序下，首个完整 key 命中后
比较完整 intent；不同全文立即 Conflict。它不检查 current/terminal 或证明查询来自
同一持久事务。原 `ProtocolRecord.full_flow_contract` 已覆盖的同步 record/save
错误和后态分类保留，不重新列作未证明。`owner-mapping` 的独立双循环候选结论仍绑定
原候选，不能迁记为本原 `attach_create` 的证明。

## 必须继续连接的责任

- 原 load 的完整 Inbox、原 find 的位置和实际拒绝顺序：terminal、refusal、当前版本、
  已有 local，再检查 Create 操作。
- 原 expected-owner 的全文查找；缺失时从完整事务 World 排除所有相同 sandbox ID，
  再取其余 owner 与 high-water 的最大值和受检后继。
- 完整 local owner/sandbox/operation 绑定；只在原 owner 缺失时追加并更新 high-water，
  对原 session/entry 写入原 local 和 local_operation，其余字段保持。
- 原 `i64::try_from(startup_deadline_ms)`、原 sandbox ID 构造、原 SQL 和
  `[deadline, sandbox]` 参数顺序、错误传播及随后原 save。

寿命 UPDATE 的成功行数在本函数中被忽略；不能另造“函数检查恰一行”的性质。
deadline 对实际行的结论需要连接原 admission 已在同事务插入该行的事实和原 schema。
`attach_admission` 只在准备原 native 行后、单次 commit 前调用本函数；此源码顺序
不等于同事务/提交未知/重启/持久接受已经证明。

原 `WorldTransaction::world` 使用 candidate 的 `RefCell<Option<World>>`，期望 live
Some 后克隆完整 World。旧 B2 的 set/take 合同不能直接扩大成该读取保证。
String/Option 克隆和 as_ref 已有逐符号合同，完整 World 克隆已有
[共享证明](../../../../SandboxModel/Access/Cloning.lean)，保留其适用版本及表示连接，不重复证明。
`identity::sandbox_id` 的原字符串格式化、十进制 u64 和 ID 校验/错误也须逐符号连接。
标准 Serde/SHA/SQL 的既有最小请求由 #632 协调；本包仍负责自己的传参与顺序，
不请求“整个事务/codec 正确”作为前提。认证、当前意图、本地结果和资源事实继续由
V7、V1/V2、V4/V5、V6 各自提供；本包不从字段相等推出这些事实。

## 执行和证据

在获准证明环境，复用固定工具和已有共享重锁；每次 output 必须不存在：

```sh
python3 verification/experiments/stage1-protocol/create-attachment/preflight.py \
  --mode attachment --output artifacts/create-attachment-original \
  --lock /private/existing-proof-heavy.lock
# world 和 identity 各使用新的独立 output；普通/原生检查仍在 ning。
```

原源码字节/Git 模式、独立 source/target/.lake/build、固定二进制、实际 Cargo 依赖、
原参数/目标/borrow/typecheck 选项和完整输出在结果中保存并起止复核。
等待重锁和实际执行分别计时。失败、超时、未完成翻译或缺根都不算性质反例。
`complete` 仅表示所选原根的捕获/翻译能力准入成功；semantic_proof、Lean/native/
formal/shared acceptance 和 full_v3 始终 false。完整入口和公共登记由主控另批协调。

## 固定格式参数模型的不足

原 sandbox ID 生成体调用固定 `Argument.new_display` 两次，分别捕获原 namespace
和 u64 sandbox；随后传入原九字节格式片段及两个参数，调用 `Arguments.new` 和
`alloc.fmt.format`，再运行原 SandboxId 校验。这些自有函数均保留真实生成体。

固定后端的 `Argument`、`Arguments` 为 Unit，`Arguments.new` 忽略全部输入。
`FixedFmtArguments.lean.in` 直接导入这个未改动的库定义，证明所有输入合并，以及
任何仅接收该参数值的格式化函数都无法同时确定地产出两个不同目标字符串。
这是库模型充分性的反证，不是 Rust 业务错误版或原格式化实现的反例，也不证明
Rust sandbox ID 的具体结果。未填生成模板不参与该证明。

```sh
python3 verification/experiments/stage1-protocol/create-attachment/format_review.py \
  --output artifacts/create-format-model-review \
  --lock /private/existing-proof-heavy.lock
```

该独立入口检查全部实际文件、内核、规定同一服务的完整非 stale/partial 诊断及三项
依赖审计，起止核对固定 runtime、库源和全部本地源码。它不运行原生测试或原附接函数。
格式化需要由主控/V0 选择能保留这两个实际参数的有限绑定；不更换工具、不假定整个
formatter/handler 正确，也不以给 Unit 填一个常量结果冒充完整 ID 对应。
