# Unload 父调用、来源行与观察请求的连接候选

对应 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635)。本包固定
integration `c265145f5614535c068c464bc37b6162edcdc45c`，复用 #768、#770–#774
的原规格及实际生成视图。生产 Rust、公共 Lake/Cargo、根导入、依赖合同和覆盖冻结均未改。
这是独立的证明开发候选，普通与原生验收交 ning，公共登记由主控协调。
[#776](https://github.com/VeriInfra-Lab/sandproof/pull/776) 负责四批上游包的公共接线，
本包没有重复实现它，也没有修签历史 continuation 输入或将新捕获冒充它要求的旧 LLBC。

## 证明与表示

- `SourceParentLink` 将 #770 的真实 `original_source_using` 接到原父调用的
  `readOriginalSource`。两者直接使用同一生成的完整 World、transaction、namespace、
  sandbox；`.conn` 原样提供给后续选项和凭据位置。成功返回复用原
  `actual_source_success_complete_contract`，保留同游标来源、完整行字节、字典后态、
  两槽、命令、身份和 HostEvidence 检查；不重新假设业务行处理器正确。
- `OptionsParentLink`、`FirstParentReadback` 安装 #774 的实际读函数和选项编码函数。
  原 SQLite 行的完整重编码必须等于原 BLOB；正常首次父调用保留该 BLOB 和完整摘要输入，
  并得到原完整观察工作请求。缺行/default 的原分支仍在上游合同，BLOB 定理限于原行存在。
- `SharedParentCarriers`、`SharedInboxCarriers` 实例化原 transaction、World、请求与错误
  连接。共享值字段的类型等式由 `rfl` 给出。独立生成的 ContractError、CrashPoint 和
  ControllerReject 使用完整构造器变换，双向定理覆盖嵌套 payload，不假设不同归纳类型相等。
  `views.json` 只将原手写 Inbox 桥的三项错误类型等式改为具体变换，原文件保持不变。
- `SameParent` 证明 Inbox/base/startup 与上述实际 source/options 安装作用于同一个
  完整调用字典，实例化 #772 的原 World/record/ledger/sealed-plan 结果；原定理的
  native、ledger、codec、读快照和 Host 条件保留。`LiveParentLink` 安装 #768 的真实 guard，
  精确保持其接受与拒绝合同。未读取字段的代表值只在 #768 已证的不相关字段范围内使用。
- `ContinuedParentLink`、`ContinuedSameParent` 把 #773 的实际 `ready_records` 装入同一
  父调用，覆盖 Err/None/Some，保留原完整工作值、回执、同连接/World、全部 native 字段。
  动作和 native 的结构变换均有完整往返定理。`ContinuedParentSuccess` 将原父函数的
  继续分支接到完整 question，保留原动作/回执前缀并准确追加一次。
- `ParentErrors`、`ParentRejects` 按原顺序证明已选前缀后的拒绝：查找/缺项、非当前/终局、
  已解决项、非 Unload、缺 bootstrap/active/本地绑定/执行来源、base/缺 native/历史缺失或
  不匹配/live、source/继续读取缺项或错误、来源 boot/pid/ticks、startup、options、credential
  缺项/错误/代不符。每个定理只要求到该首错为止的成功读取，不要求后缀成功。
  `NoQuestion` 对任意最终完整 anchor 编码回调保持同一错误；该回调即使不返回也不会影响结果。
  这不声称此前读操作没有效果，不把坏索引的 panic 解释为 JournalError。
- 两个 JSON 错误位置分别检查。选项编码失败跳过最终 question；anchor 编码失败保留
  原 JSON 错误，并在 question 内跳过最终 digest。该结论不消除之前已经执行的 options digest。
  `JournalErrorFromJson` 使用当前真实生成体，错误 payload 原样保留。

`UnloadParentLink` 汇总本包 86 项依赖审计；这些包含类型变换和辅助引理，不能计为
86 个独立业务保证。`prepare.py` 从原带哈希清单装配 279 份实际输入，不重复搬迁上游算法。
原 raw Types/Funs 保存在本包生成目录，类型 helper 的可逆 metadata 隔离及外部薄绑定
单独登记；生成函数体不编辑。`LiveDiscriminants` 使用固定后端的原 helper 生成定义，
仅给实例明确命名，解决多个生成视图的全局辅助名冲突，不改变判别值或工具。

## 依赖与范围

依赖只采用[逐符号合同](../../../../docs/formal/dependencies.md)及
[V3 来源接口](../../../../docs/formal/v3-transaction-source-boundaries.md)。
每个参数是原调用的具体接口实例，不能用“整个 reader 正确”代替业务证明。

| 未由本包消解的条件 | 提供方及限制 |
| --- | --- |
| 原 SQLite 读快照、issued statement/row、同一物理事务及正常读框架 | A4/B4 和 V5；生成句柄同一不证明 BEGIN、提交或错误后仍 active |
| 具体 codec 的完整字节/原值、标准 JSON/克隆/字典/有限循环接线 | 已登记 A6/B4；保留全部原成功、错误与非返回行为 |
| 原 HostEvidence/ResourceEvidence 与真实宿主事实对应 | V6；VM 记录或相同 PID 不证明 OS 存活、Stop 或暂停许可 |
| 独立认证上下文与完整原意图来源 | V7、V1/V2；从已解码值开始，未证明 JSON/TLS 全链 |
| 原父调用的 readCredential、base/native/ledger 及历史/工作校验读条件 | 原具名 V4/V5 提供方；没有把它们藏入一个总正确 reader 假设 |
| 原 Create options 的首次 INSERT/commit、录入到后续读取的事务历史 | V5/#637；本批只接同次读调用及现有封存计划合同 |
| 原 actions/receipts 的 identity/digest 完整收集及最终 anchor 编码/SHA 使用点来源 | V3 自有调用连接与逐符号 A6/B4；当前保留 FirstRead/ContinuedRead 的逐项观察，不把完整收集业务列为外部信任；digest 值相等不证明材料真实完整 |

正常 FirstRead/ContinuedRead 采用上游同一完整请求/工作规格，保留原成功域和正常分配条件。
原正常见证与错误版仍归各上游包，验收需核对它们的当前适用输入；本包没有新的 Rust 错误版，
不以导入失败、超时或有限样例充当一般反例。首次/继续正常返回证明与首错证明并列，
不能据此宣称任意请求必成功、循环必终止、持久收件、完整 manifest 或整个 V3 完成。

## 证明机复现与 ning 交接

先复用原固定工具、Lake 依赖和标准服务，使用独立输出及 `.lake/build`，保留共享重锁。
下列是开发检查，不能替代 `verify-stage1-protocol.sh` 的已登记原生准入和完整验收：

```sh
package=verification/experiments/stage1-protocol/unload-parent-link
# 原已配置的共享重锁；每次命令的输出目录必须尚不存在。
python3 "$package/capture.py" --mode source --output artifacts/unload-parent-source --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/capture.py" --mode parent --output artifacts/unload-parent-current --lock "$SANDPROOF_LOCK_FILE"
python3 "$package/prepare.py" --output artifacts/unload-parent-project
python3 "$package/check_capture.py" \
  --source artifacts/unload-parent-source --parent artifacts/unload-parent-current \
  --project artifacts/unload-parent-project --output artifacts/unload-parent-capture-check
# 在同一外层协调锁及四核预算内构建，再释放该锁供 diagnose.py 取得。
(cd artifacts/unload-parent-project && lake build UnloadParentLink)
python3 "$package/diagnose.py" --project artifacts/unload-parent-project \
  --output artifacts/unload-parent-diagnostics --lock "$SANDPROOF_LOCK_FILE"
```

`check_capture.py` 重新执行 #770 的完整原源码/字典审查，核对 raw 生成体、命令/日志及
父调用除注释、字典参数和已登记 helper metadata 外的全部 token。它不签发原生封口。
`diagnose.py` 使用项目原 `tools/lean_session.py` 服务检查全部实际文件和准确审计，
拒绝 stale/partial、缺文件、错误和本包新页的警告。上游未改页的警告原样记录；
返回成功只表示这份开发检查完整，不代表公共入口已接受这些警告。

主控使用 [登记候选](registration-candidate.json) 和 [公共覆盖候选](public-registration.patch)
接入全部实际输入和审计；先解决 #776 已列的旧输入/原件阻断，不能只跑旧根报告本包通过。
ning 在确定的新提交上完成普通检查及相关上游原生原版/错误版/恢复检查；严格按原收据键、
完整输入与日志决定可复用部分，`functional` 同候选只运行一次。
`proof_native_suite` 和 `verify-controller` 由主控去重，不在本包另签全项目结果。
