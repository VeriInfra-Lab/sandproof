# 原 Create 附接与事务使用点预检

[V3 组件](../README.md) · [权威范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖](../../../../docs/formal/dependencies.md) · [工作流](../../../../docs/proof-workflow.md) ·
[任务 #635](https://github.com/VeriInfra-Lab/sandproof/issues/635)

固定基线为 `ae8e5a564ab3d81652a23acb2b99e242193fa3a2`。本包只提供实际入口的
固定 Charon/Aeneas 能力预检；没有新增语义模型、定理、可信合同或原生验收。
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
