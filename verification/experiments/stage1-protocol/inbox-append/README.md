# 已有会话追加候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**最终统一候选的抽取与整套原生、Lean 验收尚待完成**；局部结果只按原回执的精确提交保留，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md) · [父记录切片](../inbox-record/README.md)

旧证明最初固定的生产输入为 `a9c9967889c4cf298e4378c7106948018835fc64`，堆叠 #713。
本包复用父切片的透明 `record` 候选、完整请求/封套合同和任意长度全文查找。
输入是已解码且按生成类型表示的 Inbox/AcceptedLifecycleIntent；没有改生产源码、公共
Lake/Cargo、共享根或冻结。当前生产已采用父切片的窄重构；本包直接核对当前实现的合同，不声称另有旧版到新版的一般等价定理。正式统一验收仍待主控协调。

## 完整合同

`FullExistingContract` 量化任意 Inbox、完整 intent 和有界会话索引；`ExistingPost` 描述
首个拒绝的精确错误及整个 Inbox 不变，或完整 Entry 追加和精确选中会话更新。
成功时只改变 current_version、terminal、entries；其余会话字段、所有其他会话及 Inbox
的配置、owners、lanes 等字段保持。Vec 回写和安全加减通过固定库合同连接。

`existingDecision` 保留实际顺序：owner/terminal、完整 retention、ExpectedSession 或
Destroy 版本、总容量、非终止操作预留末槽、操作形状/原前驱、最后是新 SDK Renew 的
未确认门槛。Create 在已有会话路径被拒绝。Unload/Renew 的可继续运行条件、Resume 的
来源代/紧邻 Unload/source/capture，以及 Renew 非负绝对结束时间均对应原代码。

非 Destroy 路径只检查 ExpectedSession，接受版本加一来自前置 Accepted 校验，不能补成
`record_existing` 自己的守卫。协议 DTO 的 Resume 仍允许编号空洞；当前 inbox 另检查
checked_next。本包同时给出合法 DTO 空洞在该 inbox 被拒绝的见证，不加强公共请求规格。
Destroy 可使用预留末槽，且不受新 SDK Renew 门槛阻挡。

`FullRecordExistingContract` 接回实际 `record` 调用者：先由全文保留 key 查找确认没有
任何旧 key，再选择第一个相同 session ID；索引界由该选择推导。这些容器条件不表示授权。
父切片的保留重放/冲突与新会话证明按原范围复用，不重复迁记。

正常 Unload、Resume、SDK Renew、末槽 Destroy 与 capture 改变、编号空洞、未确认 SDK
拒绝共用原完整封套规格。两种真实 Rust 错误版分别忽略 SDK 门槛、遗漏前驱 capture
比较；它们在相同 Model/Shape/Values/Selections 上否定上述两个完整合同。错误视图
不导入正确追加定理；完整状态断言不更换成返回值或少数字段的弱性质。

## 抽取及依赖

仅在隔离副本应用父包 `production-candidate.patch`，不另复制协议实现；新增变异唯一
定位在原 record_existing 体内。驱动保存原版 Charon，并核对完整 ContractError enum，
再重抽取候选/错误版。原四个协议薄根、真实 record 和两个实际 SDK 谓词均透明；
SDK 谓词作为独立根保留，不因错误版不再依赖拒绝门槛而丢失共同库证明。
工具生成的 Types/Funs 不手改。编译、抽取、导入失败及超时均不算形式反例。

复用 [依赖登记](../../../../docs/formal/dependencies.md) 的逐符号 String、Vec、Option、
实际比较字典、算术及 Result 合同。新增使用 Vec.index_mut 的回写、set 和恒等更新关系，
以及 U16 容量界下的 usize 加减。Incarnation.checked_next 接回同一 U64 checked_add。
自有 Clone 的完整字段保持复用父证明；ContentDigest 只保留 32 字节值，不证明哈希计算。
纯容器模型不提供 OOM/abort、物理容量或资源存在事实。

独立上下文/配置来源由 V7 提供；V1/V2 提供当前意图和耐久发送；V4/V5 提供本地接受、
结果和调用次序；V6 提供真实文件/VM 事实。record 自身不执行认证或封套校验。
SQLite codec/事务、未知提交、崩溃恢复、GLOBAL→LOCAL、完整 JSON/TLS/异步 handler
和完整 V3 继续开放。本包不把纯内存追加称为持久收件完成。

## 重现与交接

命令外层持有共享重锁；每版使用全新输出目录、隔离源码/Cargo target/.lake/build，
缓存保留。复用父 Cargo manifest/lock 和相同固定 Charon/Aeneas，不修改公共配置。
formal.py 通过原正式 actual_files/proof_services 检查全部实际依赖文件、非 stale/partial
诊断与每项审计；前后核对完整输入、工具、库和服务内容。

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-append/native.py --output /private/append-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-append/formal.py --development-native /private/append-native/result.json --variant correct --output /private/append-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-append/formal.py --development-native /private/append-native/result.json --variant skip-sdk-gate --output /private/append-skip-sdk
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-append/formal.py --development-native /private/append-native/result.json --variant skip-prior-capture --output /private/append-skip-capture
```

本机输出仅为开发证据；ning 负责普通/原生正式回执，主控同批复用完整
verify-controller.sh/proof_native_suite。`--development-native` 只绑定同一输入和实际源码，
不签发 seal；`--require-native` 要求当前 functional 与 stage1-inbox-append 登记回执，
两种模式互斥。公共登记、verify-stage1-protocol.sh 新目标及覆盖/依赖采用由候选 diff
交主控；本包 `coordinator-candidate.patch` 是父包同名候选已应用后的增量，
须依次应用，不能覆盖父登记。旧入口通过不能替代本包验收。精确交付提交、正反记录和未完成项见本批 PR/#635。
