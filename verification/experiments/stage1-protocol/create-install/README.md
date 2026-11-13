# Create 附接的完整值写入

[V3 设计](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md) · [依赖登记](../../../../docs/formal/dependencies.md)

本入口针对当前真实 Rust 的有限写入函数，正确视图只追加透明调用根，错误视图才局部修改实际语句。数学开发、实际采用、重新抽取和原生验收分别记录；旧执行保留原精确输入，不改写成新头执行。

此入口的输入清单固定于 `8a69398f`；后续 owner 守卫的采用见
[独立入口](../create-owner-check/README.md)。本清单不直接刷新整文件摘要；未变化写入函数的
复用仍须核对当前实际源码、重新抽取、生成/证明/依赖字节及原证据身份。

本切片证明 `create_attachment_install` 对已解码完整 Inbox 的效果，复用原请求、Accepted/Recorded、查找和 Create 选择规格。父 `attach_create` 的 load、owner 检查、SQL 和 save 的整体结论仍开放。

`FullInstallContract` 对任意完整 Inbox、两个有效索引和完整 `InternalLocalBinding` 使用相同性质：完整外部 owner 不存在时恰追加原 session.owner 与 binding.owner 的映射，并把高水位赋为该 owner；存在时保留原表和高水位。选中 session 写入完整 sandbox/owner/operation，选中 entry 写入 operation。后置条件与整个 Inbox 值相等，因此其余配置、host observation、lanes、session 字段、其他 session、entry 字段及其他 entry 全部保留。它没有假设映射唯一、ID 非零或已授权。

该函数不重新验证 owner 与 local.owner 的一致性，也不保证后态通过完整 Inbox 校验；父函数的原 owner 比较及后续 save/validate 责任保留。

高水位在追加分支是赋值；其单调性还需要父函数实际 successor/owner 检查，不能从这个独立写入合同推出。字段相等也不证明已持久接受、事务已提交或实际资源存在。

`selection_success_reads` 从真实选择成功推出实际两个列表的读取位置及完整 entry.intent 相等；`selected_install` 接上写入定理。追加空间由既有 u16 owner 数量上限推出；该上限到实际 FullInbox/FullLoad 接受结果的连接还须连同原表示及提供者条件核对，不能把整个收件事务设为可信。正常见证复用原有效 Create 封套；它们的整个 Inbox 校验与持久事务并未因此获证。

Charon 根转发原完整 Inbox 可变借用、原 s/e 和完整 local Copy。正确入口只追加调用根；错误版只修改实际函数的相应语句。固定 Charon/Aeneas、透明业务体、完整错误载体、未修改的生成文件和原严格全部文件服务/审计检查均须核对。

四个错误版分别丢失 sandbox、写错 operation、丢失高水位、破坏 current_version。每版实际编译、抽取并用同一 `FullInstallContract` 证明反例，值与规格文件共用，错误证明不导入正确 install 定理。编译、导入、超时或服务准备失败均不计性质拒绝。规范多行 sandbox 错误版已独立重新编译抽取，完整生成字节相同后可复用原数学回执；这两次源输入和执行身份分别保留。

复用 dependencies.md 中原 String 全文比较/正常值 clone、Vec 索引/回写/正常追加和算术合同。通用索引循环证明逐字复用原 ShapeIteration 的数学代码，来源摘要在 `reuse.json`；没有增加另一套请求/封套模型或公共库定义。分配失败、panic/abort 与真实进程内存不由这些正常值合同证明。

普通、格式、Clippy 与原生检查交给 ning；native 输入检查实际、四个错误及恢复实际六视图，每视图四个完整状态断言，先经真实 Create 选择取得索引。它们尚未执行，不能计为原生通过。公共 native 登记、Lake/根输入、依赖合同与覆盖记录由主控单一写入，本目录只提供候选和精确证据。

原 Manifest 任意已解码值、查找及 `ProtocolRecord.full_flow_contract` 保留原适用范围。本切片不宣称完整 JSON/TLS、完整持久收件、授权、资源、网络或整个 V3 完成；后续继续实际 owner 检查、i64 deadline、保留原两个输入的 ID 格式、两个 SQL 参数、save 与同事务事实。原 UPDATE 没有检查成功行数，不能把它写成恰好一行的守卫。
