# Unload 动作、回执与同一父调用的连接候选

对应 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)，堆叠于
[#802](https://github.com/VeriInfra-Lab/sandproof/pull/802)。固定父提交
`b6c0190bc92de8dc2476d776264b394fe9a7232a`；不自动移动上游头。生产 Rust、
公共 Lake/Cargo、根导入、公共登记与覆盖冻结保持，由主控统一接线和验收。

## 性质与表示

本包复用原 `Collections` 对任意有限 slice 的循环证明，安装真实生成的
`original_action_identities` 和 `original_receipt_digests`，继续使用 #779 的
同一 `FirstRead`、`ContinuedRead`、完整 anchor/question 及首错性质。

- `CollectionCarriers` 完整往返映射九个 ActionIdentity 字段、资源种类及 ID、
  WorkAction 与全部十二个 Command 构造器、完整 EffectId/logical/attempt。
  六个抽象叶子采用原 SourceQuestion 的相同具体类型；不丢弃命令或填入代表值。
- `CollectionInstall` 只替换同一 ParentCalls 的两个收集位置，保留 source、
  options、startup、base、continued 及其原 transaction/World 接线。
  完整身份按原序列逐项保留；原始回执按原顺序、完整字节接到既有摘要合同。
  `LinkedReceiptDigests` 由真实函数实例化，包含 Result 的效果与非返回语义。
- `BeforeCollections` 是原 FirstRead 的精确投影：只移除两个计算结果和三条
  收集断言，保留其余 58 个字段。继续分支对应保留 71 个字段，包括两次真实
  Vec.push。两个 attach/erase 定理返回原结构，不另造成功规格。
- 首次/继续组合定理从这些前缀构造原完整读取见证，不再要求调用者提供
  `collectActions`、`collectReceipts`、`actionsFull`。继续分支保留原列表，
  并准确追加一次原 continued action/receipt，保留完整 `continued_from`。
- `CollectionErrors` 将原 28 条首错/NoQuestion 定理实例化到这个安装后的
  同一字典；成功前缀、首错顺序和原错误保持。它们是上游性质的连接，不能
  计为 28 个新业务保证，也不声称失败前没有读效果。
- `CollectionJson` 给出从完整原列表确定的 anchor，并连接两个分支的实际
  anchor JSON 错误。第二次 JSON 编码失败保持映射错误并跳过 question 内的
  最终摘要；此前的选项摘要和逐回执摘要可能已经运行。第一处选项编码错误
  仍由原 NoQuestion 分支覆盖。

正常摘要域仍要求每份原回执长度小于 `2^61`，与既有 SHA 字节合同一致。
完整列表定理不由有限样例替代。非空正常见证及原生空/非空、重复和顺序样例
另行保留；它们不表示完整父调用的所有业务前提已无条件可实现。

## 同一合同的错误版

`CollectionContract.CompleteIdentityContract` 对所有输入要求完整身份列表相等，
原版和错误版导入同一文件。错误版只在隔离 Rust 归档内把真实 clone 后的
`ActionIdentity.attempt` 清零。固定原 Charon/Aeneas 必须实际编译、抽取它，
生成体不得手写。`CollectionCounter` 在这个真实生成体上证明：非零 attempt
的单元素输入返回清零身份，从而违反上述同一完整合同。

`native.py` 把同一 `native-fixture.rs` 接到各自隔离副本中的原 private 函数，
正确版、字段遗漏版、恢复版使用独立 Cargo target。先要求真实业务测试二进制
新编译且绑定同一 crate/source，再运行同一完整身份和完整回执断言。错误版
必须到达身份相等断言失败，同时回执断言通过；编译失败、导入失败、超时不算
反例。恢复版仍须重新编译和通过。这里 WorkAction 的实际协议版本是 1，
与 LifecycleRequest 的协议 4 区分；样例不是一次完整生命周期授权请求。

## 输入与可复现开发检查

`prepare.py` 先逐字核对父包的原冻结及全部 279 份装配输入。正确版 collector
的 raw Types 必须相同、raw Funs 的全部非注释 token 必须相同，才能复用原
任意列表证明。仅覆盖该工具输出及新增手写证明，不更改生成算法。
错误版在独立项目导入同一合同和真实错误生成体，不导入原版循环正确性证明。

`capture.py` 使用固定二进制、原 selector、独立源码归档与重锁。
`check_capture.py` 复用 #779 的完整原 source/body 审查，并核对两份 collector
捕获的实际 Rust、LLBC 的透明真实根/selector、完整原生成体、命令/日志和
同一精确候选。原始 LLBC、源码归档、生成体和失败输出留在私有证据目录，
不能只有这份说明或一个成功标签。

下列各输出目录均须尚不存在，使用原已配置的固定工具、依赖及共享重锁：

```sh
parent=verification/experiments/stage1-protocol/unload-parent-link
package=verification/experiments/stage1-protocol/unload-parent-collections
python3 "$parent/capture.py" --mode source --output artifacts/collection-source --lock "$SANDPROOF_LOCK_FILE"
python3 "$parent/capture.py" --mode parent --output artifacts/collection-parent --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/capture.py" --variant correct --output artifacts/collection-correct --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/capture.py" --variant attempt-zero --output artifacts/collection-attempt-zero --lock "$SANDPROOF_LOCK_FILE"
python3 "$parent/prepare.py" --output artifacts/collection-parent-project
python3 "$package/prepare.py" --parent-project artifacts/collection-parent-project \
  --variant correct --output artifacts/collection-project
python3 "$package/prepare.py" --parent-project artifacts/collection-parent-project \
  --variant attempt-zero --output artifacts/collection-counter-project
python3 "$package/check_capture.py" --source artifacts/collection-source --parent artifacts/collection-parent \
  --correct artifacts/collection-correct --mutant artifacts/collection-attempt-zero \
  --project artifacts/collection-project --output artifacts/collection-capture-check
# 构建各用独立 .lake/build，并由外层协调共享重锁和四核预算；不得共写缓存。
(cd artifacts/collection-project && lake build UnloadCollectionLink)
(cd artifacts/collection-counter-project && lake build CollectionCounter)
python3 "$package/diagnose.py" --project artifacts/collection-project \
  --output artifacts/collection-diagnostics --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/diagnose.py" --project artifacts/collection-counter-project \
  --output artifacts/collection-counter-diagnostics --lock "$SANDPROOF_LOCK_FILE"
```

`diagnose.py` 沿用原服务客户端，对精确传递闭包的每份实际文件取得完整诊断，
核对全部真实依赖审计；拒绝 stale/partial、缺项、错误和本包新页警告。父包
保留的四条外部 `__bind_in` 名称警告原样报告，不关 linter，不称公共零警告通过。

ning 在同一确定提交上运行原生检查，并由主控同批去重普通全套和公共证明入口：

```sh
python3 verification/experiments/stage1-protocol/unload-parent-collections/native.py \
  --output <新私有原生目录> --lock "$SANDPROOF_LOCK_FILE"
bash scripts/check-function.sh
# proof_native_suite、verify-stage1-protocol、verify-controller 的正式登记、
# 证据准入与内容复用由主控协调，不能仅跑旧根报告本包通过。
```

## 未消解条件与提供方

只复用[逐符号依赖合同](../../../../docs/formal/dependencies.md)和
[原事务来源边界](../../../../docs/formal/v3-transaction-source-boundaries.md)。

| 仍需的条件 | 提供方与限制 |
| --- | --- |
| 首错之前的具名读取与正常父调用前缀 | #768、#770–#774、#779 对应实例及 V4/V5 的原 native/ledger/读快照条件；本包不把整个 ParentCalls 设为可信 |
| 原 Clone、Vec/Slice、算术、标准编码与 SHA 字节使用点 | 已登记 A6/B4 的逐符号合同；不表示整个 Serde 或服务正确 |
| 真实 SQLite BEGIN/读事务/提交及持久收件历史 | V5/#637；类型上同一句柄不证明物理事务或持久接受 |
| 独立认证、当前意图与耐久发送 | V7 与 V1/V2；仍从已解码值开始，不称 JSON/TLS 全链已证 |
| 来源 OS/VM/文件事实及材料完整性 | V6；32 字节摘要值或相等不证明真实材料存在且完整 |
| 最终 anchor 编码成功、其摘要调用、选定资源和正常分配 | 原 question 具名合同及实际输入；完整收集不保证任意请求都会构造成功工作 |

完整 manifest/任意文件集、持久收件、网络外壳、实机故障矩阵、全部系统结论仍在
所属任务推进。本包不扩展历史证明适用版本，不把局部构建、开发检查或错误版
反例称为正式整套验收；公共登记与覆盖记录仅交候选改动。
