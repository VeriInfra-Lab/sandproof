# 卸载决议校验候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**最终统一候选的抽取与整套原生、Lean 验收尚待完成**；局部结果只按原回执的精确提交保留，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。

标准编码实例仍调用真实 `serde_json::to_vec`。通用证明只接收同一完整值对应的编码结果，并保留失败、长度及调用顺序；用于标准实例时仍须提供这一次编码调用到结果模型的窄连接。它不假设整个校验函数正确，也不覆盖持久事务或任意 JSON 解析。
[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

旧证明从 a9c 的已解码 Entry/UnloadResolution 值开始，连接
unload_resolution::validate_entry 的[隔离候选](production-candidate.patch)。
原函数在固定工具的 Serde 参数翻译处失败；仅分离编码又遇到 matches! 借用上下文
及 Box 隐式借用限制。候选把编码作为任意 UnloadEncoding 参数，将原操作判断移入
具名函数，并显式借用同一个 Box 内容。标准实现仍调用真实 serde_json::to_vec。
该窄重构已被当前生产采用，本轮直接证明采用后的实现合同，不另声称旧实现的一般等价。
标准编码实例的窄连接仍需落实；有限原生对照不能消除这项义务。

[共同规格](UnloadModel.lean.in)与[完整对应](UnloadProofs.lean.in)覆盖任意 Entry
和任意编码计算。没有决议时直接接受，不检查操作种类或调用编码。有决议时先要求
Unload，再运行编码；原 Json 错误先于后续检查返回。成功编码至多 4096 字节，随后
检查决议元数据和可选结果。编码参数不假设成功、字段正确或长度合规，并保留外层
失败与发散；这里不宣称任意回调就是实际 JSON 编码器。

BeforeAdmission 要求条目没有 local_operation、快照为 Running、非 terminal、
current_vm 与 lifetime 均存在且 uncertain_actions 为空；AfterCapture 要求
local_operation 存在。结果存在时精确要求 started ≤ observed < end ≤ max_end
及 started ≤ startup_deadline。相等边界保持原语义；不擅加快照 owner/代/当前操作、
lifetime 内容、applied_deadline 或 previous 字段检查。字段存在不表示实际 VM、
授权、历史可达性或持久接受；历史结果也不能代替新的物理 Ready 观察。

[合法见证](UnloadValues.lean.in)复用完整封套规格，并包含两种决议、无结果、相等
边界，以及原函数允许的弱快照字段。[实际返回](UnloadWitnesses.lean.in)由同一完整
合同导出。ignore-snapshot-phase、ignore-capture-operation、ignore-result-maximum
三种错误版分别漏查运行阶段、捕获操作及结果上界，必须实际编译、抽取并触发同一
[原生断言](native_tests.rs)和完整合同反例；错误视图不导入完整正确证明。
[回调检查](callback_tests.rs)核对短路、错误优先级及 4096/4097 边界，属于候选接口。

[库绑定检查](library_binding.py)逐字节复用既有 Bool.ne、Option.as_ref/eq、String.eq
和 Vec.is_empty 五个函数模型，核对固定生产/抽取标准库源码。Phase 的六个逻辑标签
由完整 LLBC 核对，比较函数来自真实生成体；这不涉及物理布局，也不证明标准库实现。
没有新增 Serde、handler 或事务公理。字符串、容器、算术及错误值载体仍依赖
[逐符号合同](../../../../docs/formal/dependencies.md)及固定转换工具的责任边界。

原 Inbox.validate 的其它子校验、ready_facts 的有符号转换和寿命条件、实际状态
转换保持性、持久 codec/提交/未知结果/重启仍需 V3/V4/V5 连接。V1/V2 提供当前意图
与耐久发送，V6 提供实际文件/VM 事实，V7 提供独立认证来源。完整 manifest、实际
JSON 字节链、网络外壳和系统结论均不由本切片完成；摘要值不证明计算过哈希或材料真实。

## 检查

持有共享重锁后执行；每份源码副本使用独立 .lake/build，保留缓存及失败证据：

```sh
python3 verification/experiments/stage1-protocol/inbox-unload-resolution/native.py --output artifacts/inbox-unload-resolution/native
python3 verification/experiments/stage1-protocol/inbox-unload-resolution/formal.py --variant correct --output artifacts/inbox-unload-resolution/correct
python3 verification/experiments/stage1-protocol/inbox-unload-resolution/formal.py --variant ignore-snapshot-phase --output artifacts/inbox-unload-resolution/wrong-phase
python3 verification/experiments/stage1-protocol/inbox-unload-resolution/formal.py --variant ignore-capture-operation --output artifacts/inbox-unload-resolution/wrong-operation
python3 verification/experiments/stage1-protocol/inbox-unload-resolution/formal.py --variant ignore-result-maximum --output artifacts/inbox-unload-resolution/wrong-maximum
```

正式模式使用 --require-native 和 ning 的有效普通/native 回执；开发证据只能用
--development-native 指定，不能替代正式回执。prepare-only 仅准备视图。形式驱动
重新抽取正确与错误版，运行准入回归、全部实际文件的同服务诊断与依赖审计。
[清单](inventory.json)绑定生产输入和声明；完整源码、工具、依赖闭包及原始生成体
收录在执行记录中。[公共登记候选](coordinator-candidate.patch)接在 #713–#721 之后，
由主控同批接入 verify-stage1-protocol.sh、proof_native_suite 和覆盖记录，保留全部
旧入口；局部通过不表示 ning 或 verify-controller.sh 整套验收通过。
