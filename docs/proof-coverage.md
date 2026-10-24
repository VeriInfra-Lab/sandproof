# 证明覆盖与剩余义务

[文档目录](README.md) · [验证设计](formal-verification-design.md) · [本地状态机](state-machine.md) · [证明流程](proof-workflow.md)

## 当前唯一协议与正式装配候选：尚未验收

中心单行收敛候选已替换原 `Sandbox.operations` 与 `LifecycleProfile.sessions` 的重复权威：
只保留一张会话表、完整当前操作和原分配前沿；当前 Ready/Renew 事实直接写该行，
不再合成旧 `LifecycleResponse`。注册必填，当前格式不接受旧数据或缺省协议配置。
新恢复检查同批保留全身份、原参数、分配与版本顺序、寿命、额度、位置代次及精确终态确认。
原编号/比较引理可作依据，但改动的真实函数及完整状态尚无当前抽取/证明结论。

同一原账本新增显式 `result_retention_ms`：首次 Cleaned 固定期限，重放与重启不续期；
期限结束、精确确认/关闭回执和业务责任结算是分别保留的删除条件。
时钟回退仍保存 Cleaned 并暂不建立期限；时钟恢复后沿同一收尾 CAS 只建立一次起点和截止点，
重复和重启不续期。真正算术耗尽仍保留事实并拒绝删除，不把已发生的完成改成未发生。
接受时间改为原记录内明确的 Sampled/Unsampled：Create/Renew（以及当前需时钟的Unload）
保留原采样并精确复算期限，Destroy不要求读取时钟且明确未采样，不用0伪装实际时刻。
保留期仍核对历史中的全部已采样时间下界；未采样不产生新时间事实，也不免除确认和关闭条件。
该表示改动和新增恢复回归已纳入下述指定源码的有限检查，不计为已证明或整套验收。
详情删除时先关闭该会话涉及的当前公开窗口，存活旧记录仍先全文重放；
较小的新序号在同一个开放窗口内可乱序接受。已删除的 SDK 键不能凭记录缺失再次受理。
Stale 仅说明这次没有新增受理，不证明这个键历史上未执行。

内部 SDK 续期的有限回收候选在原会话中保存最新完整确认、实际删除版本前沿及被删采样时间最大值；
每次只删原上一个完整且已应用的 SDK Renew，保留 Create、公开输入、当前行、未知结果和终止责任。
恢复保留相邻版本检查，间隙只能落在实际删除前沿及完整确认之前；该结构检查不单独证明被删历史。
Renew 保存原前驱有效期限，不能用 Create 的期限替代；下一 SDK 检查全部保留历史中的未确认 SDK，
不能靠插入公开操作绕过确认，Destroy 不受此门控。原服务/数据库已接入确认与回收并取得下述有限回归；
统一候选和真实 Rust 证明尚未闭合，旧恢复和终态抽取结论不自动适用。

当前中心的引用来自 `ValidationLink` 的原 Create/Destroy、`GlobalBuildAnchor` 的原 Create/Cleaned，
由 `RegistryMetadata::validation_valid`、`PrivateBuilds::valid_for` 与 `publication::terminal_request`
检查；数据投影读取原 Create 和当前操作，令牌签名不建立另一张旧 SDK 引用表。
原 `BusinessEnvelope` 的同一 CAS 仍须验证完整交叉引用，不能从“已确认”推导任意资源或 pin 已消失。
本候选需证明回收保持首行、当前行、终态、位置和所有其它操作，及这些真实引用检查的保持性；
目前是源码核对和待证业务义务。以后新增 capture/claim/pin 引用旧操作时必须在原同一事务补齐检查，
不能直接沿用目前没有该引用的范围。

当前生产中心服务已改用完整 `AcceptedLifecycleIntent` 与真实终态观察；私有创建、验证证据和终止
沿同一原 Redis/CAS，公开查询、卸载和销毁从同一业务包络排除私有实例。
旧 `wait/send/confirm` 派发路径已从生产中心服务移除；中心核心和中心服务有限单元消费者已有下述当前合同迁移结果；其余未纳入下述有限验收的 Redis/SDK 夹具仍需迁移，
保留原安全、未知、迟到和退休竞争断言，未迁移用例不能计为通过。

中心原核心/退休窗口/结果分类消费者在 `ccde8ee010dbef0f4bf1ca216977cb41f9a562bf` 完成当前合同迁移：原11＋13＋2场景仍为26，ning实际24＋2全部通过，严格Clippy与格式通过，3.545秒。使用真实同步核心，保留完整原/当前身份、保留时间、重复终态、SDK确认、32轮位置退休、耗尽和损坏恢复；旧观察版本/rank算法对照改为当前合同的绑定与终态重放检查。此次只改测试，宁实际格式字节按前后摘要导回；未重跑既有decisions8、生命周期36和Host21＋1，未取得新增Lean或完整组合结论。

来源观察慢I/O在 `098e12d9e65a2392e398a3a0f694de6e7f686207` 移出原Application管理锁：锁内核对原SourceWork，解锁调用原Host，再入原journal按当前状态保存；没有写回先前整个World。ning真实Application/SQLite与受控Host的并发目标6/6、严格Clippy通过；原持锁实现重新编译后由同一“另一会话Create/Ready必须推进”断言拒绝，恢复正确实现后该项再次通过，共28.167秒。源码和三版本日志/新编译记录保留；实际格式提交为 `616b60314f3aa5f7e58dcd08cd68d86ab9ab1b0a`。Destroy与宿主epoch变化的迟到结果拒绝，以及仅另一会话推进时合法结果仍可保存，均有组件证据；未完成版本切换情景、Full捕获、KVM或此函数的Lean对应，旧锁范围结论不自动适用。这一批不含后续B授权/schema草稿，原失败记录继续保留。

中心服务消费者在 `a8fceb5e891051cc40fe9ea621361c862f02a76a` 使用当前唯一 record/observe/confirm/close 接口，测试Host保存完整受理请求，MemoryAuthority仍调用真实CenterState。原29＋5＋7＋9迁移场景及共享查询3项共53项全部通过；原库目标实际69项为62通过、7失败，15.414秒，生产库Clippy通过但不认领测试代码Clippy。7失败均因原deployment JSON夹具遗漏必填的 `bounds.result_retention_ms`，在解析阶段先返回InvalidConfiguration，尚未进入TLS/ProtectedInput/模板拒绝；原运行日志保留。更早 `9148f837` 仅因测试比较u64/i64编译失败，零场景执行；后继采用受检查转换，没有截断或放宽确认。

`74a7bdf635e0207bb57aa908d1bbbfee9a88f662` 仅补deployment夹具与原示例相同的300000ms保留期，并在原负向场景保留缺失/零值拒绝；未增大存储或元数据预算、未修改生产校验。ning仅重验受共享夹具影响的deployment10项，全部通过，59项过滤，8.363秒；其余59项复用a8fceb5e未变输入的成功结果，不称新一轮69项全部执行。单文件实际格式按前后摘要导回。旧观察版本/rank专用场景已按当前完整绑定、Pending不覆盖并发已保存Ready、冲突寿命不覆盖原事实、原截止时间和终态精确退休重新表达；时间到期仍不证明清理，公共令牌/未知提交/迟到不复活断言保留。该组是受控Host与原同步核心组件测试，真实Redis消费者、完整SDK/基础入口、KVM和当前Rust形式证明尚未由此验收。

真实Redis消费者迁移在 `7bc02a58` 首次实际运行35项（含共享丢回复4项），31通过、4失败，17.102秒；明确测试目标Clippy另有3项诊断，原记录保留，不称已验收。三个失败与当前夹具/调用次序有关：公共跨租户用例须登记第二真实租户及请求窗口；容量拒绝后先保留原Destroy，再由下一次后台推进；关闭回执先重新确认原包络，丢提交回复的故障点须置于其后的CAS。相应测试修正保存于 `a4cfd01e`，未改原拒绝/恢复后退休断言。

其中缺少终态确认的私有构建恢复用例揭示实际检查遗漏：`PrivateBuilds::valid_for` 的已结算且原会话仍存在分支，已有末操作及Cleaned全文匹配，却未核对同一记录的 `terminal_confirmation`；原 `settle_build` 要求该确认，正常路径在退休前不清除。当前修复仅补同一完整确认相等条件；会话已退休时的原位置/回执规则保持。负例保留并明确中心核心仍可合法保存待确认Cleaned，业务结算状态必须拒绝这种缺确认组合。此函数及真实Redis/结算/恢复的形式对应仍未证明。修复源码 `d087789f56715fe3a856ff7d9073e74f4890d145` 在ning实际35/35、零忽略通过，原恢复负例同时断言中心核心合法而业务包络非法；测试目标Clippy当时仅余一项测试长度限制，17.184秒整批未封口。后继 `7c5f6f2fcf93742061b5a7d5435630c7603aadbf` 只导回三处实际格式并按原顺序提取同一结算断言helper，生产/依赖/共享夹具不变；末轮只执行受影响场景1/1、34过滤，完整business_authority测试目标严格检查和格式通过，16.553秒、无格式差异。其余34项精确复用d087成功，不称末轮重跑35。使用已有固定Redis7.2.16，二进制SHA256 `e1a376d0ea42cb9bcd92ed1fc450871bb7d9e298fc3317a742b2892dfdf06244`；官方归档SHA256 `960a8ec15e34ff40e57ff16837b26b33bd81f2da6d24497bb63de532a323a18e` 对应的1584个源文件逐项一致，二进制另以其执行摘要绑定，不冒称可重现构建证明。它与CI保护产物分别登记。原编译、四断言及Clippy失败完整保留；未重跑服务69、Host或其它模块，未修改四个中心证明根、B批格式或固定工具。真实Redis+HTTPS装配消费者、完整SDK/统一基础入口、KVM与形式化仍未由该组件组验收。

[回执屏障](../tests/system/_internal_fault_relay.py)在 `b3e7bbea47d255794e8bd5acd673b4abb3fc943f` 仅迁移原两个 Python 测试文件，基于统一 `d78c9f59` 接入，没有合入 OS 旧父链。屏障只在真实 create-observation 路径的 200 响应中识别完整 `CreateObservation`；收件成功不冒充 Ready。检查原/当前完整请求、独立登记节点、启动和寿命边界，重复观察只允许观察时间变化，其余原请求与执行事实保持。分区记录保存完整 `AcceptedLifecycleIntent`，只有原 record 路径的 Destroy 设置销毁事件。ning 原 Python 单测 **19/19**，原 `secure_probe` 双向 mTLS 状态查询精确 **1/1**，共 **5.973 秒**；原证书链、ALPN、叶子 pin 拒绝和名额收尾断言保留。这里的新 Ready 检查仍是有限 Python 匹配测试；状态查询穿过真实双 TLS 连接，不表示真实 Ready 已穿过屏障，也不是 Rust/Lean 对应证明。该轮 DK02/DK04、原审计和故障脚本读取器尚待迁移；后继源码迁移及普通单测见下方，真实故障场景仍未执行，不能据此认领整个系统故障矩阵。

[Redis＋HTTPS 应用消费者](../sandbox-center-service/tests/redis_https_application.rs)在 `b891414731937dd14de0d748ed7bd184953bab9d` 复用原当前应用包装、真实 SQLite、固定 Redis 及实际 TLS/独立中心上下文；收件、观察、续期/终态确认和关闭均调用原 Host，后台使用原 `run_driver`。ning 显式启用 `redis-tests`，实际 **5/5**、零忽略，完整该测试目标严格 Clippy 和格式通过，共 **22.461 秒**。唯一格式产物按前后摘要导回 `fec85a8a7a84f9e0fb7386b912ce94262f835545`，没有重复运行未变输入。覆盖租户/选项/寿命、当前观察回包丢失后中心重开仍保留原请求和启动/到期/最大寿命、查询的一次 Redis 快照及真实存储失败后的围栏。并发例保持原 normal4/control2：屏障阻塞 A 的原 Prepare，B 仍完成真实 Create/Ready 和 Destroy，再放行 A；两个原 sandbox 各 Prepare 一次，最终额度归零、原位置收到准确释放调用且工作排空。Host 效果和 PID/boot 仍是受控夹具，不能将原 sandbox 计数解释为物理 VM 唯一性或真实宿主释放证明。

该组首轮 `389528d129353e54b88e8359a0aa9a3fc90c29c9` 已实际编译并运行，但 **0/5**，另有测试长度 lint，原记录保留。旧测试 builder 未设置当前必需的 `lifecycle_observation_policy`，原入口在调用 Host 观察前即拒绝，因而出现 503 和未收到观察通知；真实生产装配已经设置该策略。后继仅补原有限 5000/100ms 时效策略、保持实际时间戳/寿命检查，并按原顺序提取相同 Prepare 断言 helper；没有延长等待或放宽安全断言。这两组未修改公共 `internal_application`/`lifecycle_application` 夹具、生产调度器或 B 的捕获/授权/driver 草稿，六份原 B 草稿未入测试归档或提交；不新增 KVM、Guest、SDK、Lean 或完整基础/系统验收结论，不刷新旧冻结记录。

当前审计消费者在 `9a232919a3a64a692c1e45d6c9cdb5122058f564` 接入原 27 个 Python 测试/工具文件；仅应用 OS `e714556e` 相对 `b3e7bbea` 的已审补丁并逐文件核对摘要，没有合入 OS 旧父链，旧 `2ff` 候选未执行。读取器改用完整当前 Create/观察、中心原会话的 running 寿命及 Host 原映射/native/claim 物理身份，不新增生产 Ready 报文或审计输出。分别检查两端 JSON 字段的整数/布尔类型和必填创建代次，拒绝用 Python 数值相等混过单端类型变化；实际读取的文件描述符还须与回报路径指向同一对象。新增单端变异负例保留其它输入合法，换文件负例覆盖读到替换对象而原路径已恢复的情况。这些是有限解析、文件和关联检查的工程测试，不是完整 JSON、任意文件竞争或系统对应证明。

ning 同一批实际发现并通过七个受影响模块 **193/193**，零失败、零跳过；27 个文件语法与已审清单检查通过，总计 **4.258 秒**，源码前后无变化。原日志和各模块实际数量保留，没有将静态方法数直接当作执行结果。原 relay 19 项、双向状态 TLS 1 项及 Rust Redis＋HTTPS 5 项的源码和实际依赖未变，按原版本证据复用，本轮未重跑。真实 DK02/DK04、SDK、KVM、q1 和高权限服务验收仍未执行；生产审计/协议、两个共享 Rust 应用夹具、Node 与六份 B 草稿和配置保持原样，没有新增 Lean、工具链或冻结结论。

以下是分别绑定源码的有限工程结果，不是统一候选或系统验收：

| 源码 | 实际结果 | 尚不能认领 |
|---|---|---|
| `a5a02d10` | 同表生命周期20项、完整验证报文及独立子限额3项通过 | 后续私有消费者或最新整个工作区 |
| `832924d2` | 显式匹配宿主启动身份后，真实网关端口9项通过；原凭据和正文断言保留 | 物理宿主/来宾、TLS或KVM完整组合 |
| `c694b08c` | 生命周期21项通过，含受保护Builder分配/重放角色；当前中心服务库编译通过 | 旧测试迁移、SDK/Redis完整入口及严格静态检查；严格检查先在中心核心7项诊断处退出 |
| `4d7bfd58` | 同表生命周期27项、完整Create拒绝观察2项、当前本地应用7项、Host实际SQLite拒绝/重启/终态链1项通过；同归档32.703秒 | 后续接受时间改动；各包全部目标编译仍有旧夹具/调用者，四库严格检查止于中心恢复函数103行限制；双二进制构建止于中心审计投影旧字段，实际版本命令尚未执行 |
| `f36f225c` | 中心生命周期36项、当前收件78项、watch 5项、应用7项及Host连接2项通过；双实际程序重新构建并报告内部2/中心8/宿主20 | 统一候选、完整SDK/Redis/KVM、形式证明；严格检查当时止于中心两处函数长度 |
| `20c1593e` | 修复两处长度后，中心原36项再次通过；保留全部原断言 | 后续统一候选；严格检查另有两处文档lint，后继修复不等于已复验通过 |
| `74b295a3` | 有限借用/枚举重构后中心原36项、四个production lib严格检查、定向格式均通过，9.084秒；实际宁格式结果随后按前后摘要导入 | 后续完整中心抽取、统一候选全部测试及系统验收；不重复认领未变化的78项等检查为本轮执行 |

`f36f225c` 的真实 Rust 删除错误版实际编译成功，再由正确版同一公开重放性质拒绝：
`confirmed_sdk_compaction_preserves_public_replay_and_current_deadline` 触发
“public renewal replay must survive SDK compaction”。两版均核对实际业务库和协议库重新编译，
最后恢复正确源码并通过原断言；这属于原生回归，不是SQL事务或Rust实现的形式证明。

`ffbf3158` 的完整中心确认根经固定原Charon生成完整LLBC
（摘要 `e88ba845459587abdc4286ac238e83727da0b53fb37a02496de17c91874f652c`），
但Aeneas在枚举展开、返回借用和分支上下文合并处退出2，尚无完整可检查Lean。
同源Host真实纯删除判据经Charon/Aeneas生成Lean，LLBC摘要为
`da0e5ca06824e147b78e861cd37133b33892b131647c3297e9913afc42be20c7`；
原始生成Types/Funs未改，5个标准库符号连接既有A6定义后，
`source_reference_prevents_discard`（有来源引用不删）及
`public_window_prevents_discard`（非SDK窗口不删）已通过Lean内核、原服务完整诊断和依赖检查。
仅依赖`propext/Classical.choice/Quot.sound`，没有自定义公理；首轮跨生成根重名的失败保留，
后继成功21.468秒，服务退出时的Python清理警告也保留，完整响应及独立内核均退出0。
这是实际纯判据在既有A6模型下的两条局部结论，不覆盖SQL取字段、原事务/CAS、引用保持、
完整中心确认或系统；尚未接入本批正式模块与完整验收，不能把局部通过称整套完成。
后续有限源码重构仅调整枚举分解、查询借用边界、临时唯一性集合和投影后的短写；
不改变持久数据格式、拒绝条件或检查顺序，也不把原恢复校验设为opaque或可信假设。
临时集合持有原值副本会增加有界临时分配，仍依赖既有分配成功条件，未声称无分配失败或新增性能上界。

`0eb78929` 的后继完整中心根仍未通过：Charon退出0，完整LLBC摘要为
`a08b344c0a764753d7d94df448d7318500c298cbbefaab1db167b522b3518823`；
Aeneas在`PrePasses.ml:576`拒绝该次重构新增的`valid_lanes`循环提前返回，
尚未进入透明函数翻译，不能解释成原借用点已经修过仍失败。
复核原Git还发现`terminal`/`released`的原返回借用闭包未实际修改；后继纠错恢复标志控制循环、
使用不提前返回的索引查找并准确处理这两处，保持原短路、首匹配与反向首匹配。
这些纠错仍需重新检查，不从前一轮36项通过或Host两条小定理推导新中心实现已证明。

纠错源码`88ca719f`的原36项、四生产库严格检查及定向格式已在宁通过（10.001秒），
实际格式后继`51728e61`再做一次完整中心根：Charon退出0，完整LLBC摘要为
`ee88c2a8f1646dec8d256afcec545db996852346d6de2c4bce023745597b97ba`。
该轮通过预处理并进入127/210个透明函数，仍在8处分支上下文合并处退出2（8.368秒）；
没有完整可检查Lean。原恢复校验、确认转换及引用保持仍是自有未解义务，
后续按实际生产函数分别连接，不将这些业务整体列为可信条件。

内部测试消费者`905822d5`已切到完整已收请求及独立认证中心上下文：实际宁编译、两目标严格检查通过；
`secure_data`为5/5，`secure_probe`为15/16（整轮13.686秒，仍失败）。
唯一失败是原Python双mTLS测试代理只允许已退役的`/internal/v2/node-status`，
当前Rust发送`/internal/node-status`被该固定白名单拒绝；不是证据表明TLS链或ALPN已失败。
后继将代理有限路径逐项对齐当前生产常量，并保留拒绝旧路径、任意目标及过大正文的检查；
这些测试改动不覆盖生产授权、SQL或物理结果。该轮代理`ReceiptBarrier`仍用旧packet/fact形状；
后继完整请求/观察的迁移及实际结果见上方 `b3e7bbea`，状态查询或纯代理向量通过仍不能算完整故障矩阵通过。

后继`882bd629`只复验受影响的原双mTLS状态查询1项及代理Python原11项，全部通过（7.557秒）；
原证书链、ALPN、未登记叶子拒绝、回调次数和名额归还断言保留。
`905822d5`已过的其它20项与对应严格检查没有重复执行，不将分批组件结果写成整套验收。
中心决定消费者`597bd014`的原8个场景迁到真实当前接口后，8/8及该目标严格检查通过（3.346秒）。
依据现行寿命合同，测试覆盖相对秒输入截顶及伪造超上限Ready的拒绝；
Pending保额、Cleaned待准确确认、晚到拒绝不能抹去已Ready，以及原配额/健康/模板/分池/u64极限均保留。
其它中心单元、窗口、分类及Redis/业务测试仍待迁移，不从这8项推导全部中心测试已通过。
当前Host HTTPS测试的21项与后继精确重放1项来自Core所列两次不变闭包检查，
后继格式头`a68e3c7a`已整合；不能把它写成本轮重新执行22项或真实KVM验收。

`8cdd12f2` 保存上述7项静态修复，首次Create记录校验独立成实际纯函数，原谓词和短路顺序保留；
其后继已取得上表有限回归，不能据此认领全部严格检查。后续 `818cec1` 修复中心审计投影、
保留Create载荷脱敏，并将真实followup校验透明拆分；二进制与受影响检查另按该源码验收。
没有为取得通过降低lint或改变固定工具。

真实终态根 `terminal_result` 在 `c694b08c` 重新经固定原Charon成功生成完整LLBC；
Aeneas已越过早退循环和原切片`find`的生命周期约束，进入192个透明函数的翻译，
但仍报告13处借用返回、分支上下文合并或枚举展开错误。原日志与IR保留，尚无完整可检查Lean产物，
不能称终态实现已证明。后续按真实业务根和恢复校验分别连接；恢复校验仍是自有业务义务，
不得将其整体变为opaque或可信假设。

`codex/feat/641-lifecycle-unified` 从保全草稿 `4cb07020` 与 #659/#661 的完整祖先继续开发，
并完整合入当前审计/配置接口 `a64fcd35`（含 OS `82ba33f8`）。公共身份合同提交
`32118b6d` 新增 `LifecycleAccessIdentity` 和显式上下文检查；它只校验表示与独立注册的中心、
完整节点、部署和接受版本，不提供会话访问许可，也不把外部 incarnation 转成本地 generation。

本候选正在将正式数据入口改为：同一次 TLS 的原 peer→受保护中心注册→原当前收件账本映射，
再检查原本地 VM、凭据、期限与 Access。卸载/销毁登记后的撤销和重新派发前复查归实际业务核心。
新的共享类型、当前 URL、数据路由、Ready 观察竞争、终止意图和宿主 initialize/audit/serve
接线尚未完成组合编译、原生回归或形式验收；没有新增本批已证结论。

宿主的显式 Bootstrap 创建和现场读取依 OS 原对象合同；配置、持久登记与真实材料的连接仍需验收，
不能由登记相等推出 VM 级捕获/恢复兼容。当前原生测试源码保留普通返回、取消持额和 Ready 竞争，
其源码存在不表示已经执行。当前结果确认、有限窗口退休、完整卸载/恢复/GC及多周期仍有未完成项，
不能把终止意图、受理回执或永久保留历史当成物理完成与最终闭环。

旧 Status/JSON/字段目录和任务/存储定理只适用于其冻结的真实函数与输入；它们可以作为复用依据，
不能自动覆盖上述新字段、改动的生产调用或异步外壳。本批不刷新旧冻结摘要、不手改生成体，
后续由统一候选按实际变化重抽取、审计并运行相关验收。


## 第一阶段适用性：重新定目标，不迁记旧结论

本次 `stage1` 是基于 integration `56b14d642c392316043dfbdbe4d26bacc663fb30` 的文档与任务调整，不修改 Rust、Lean、抽取物、工具或冻结摘要。该基线已包含 [#631](https://github.com/VeriInfra-Lab/sandproof/pull/631) 的组合；完整普通检查、证明验收与 CI 证据在该 PR 的精确候选上，合入树对应已核对。下面原有局部证明和限制保留，不能把它们升级为本阶段已完成。

| 第一阶段目标/连接 | 可以复用的基础 | 当前真实缺口 |
|---|---|---|
| LC-G1 状态连续 | 原本地状态模型、身份/回执等引理 | 真实 Full 捕获/磁盘截面/封存不存在；旧暂停补偿须重审；完整连续性未证 |
| LC-G2 唯一恢复执行 | 围栏、代次、原实例观察/授权规则 | 新 claim、激活前不可逆消费、重启继续同候选及真实激活连接未交付 |
| LC-G3 卸载与材料回收 | 位置复用、原清理/任务责任局部基础 | bundle 独立存储、持久 pin、File backing 实际使用、retire/取 pin 竞争未闭合 |
| LC-G4 重试/崩溃安全 | Redis/SQLite、JSON、原回调/持额及首链的有限组件 | 公开操作键、新协议、文件/库非原子窗口、各新状态恢复和实际接线未证 |
| LC-G5 正常接受 | 有界类型/检查与部分正常分支 | 新完整合法域、实际新 handler/状态转换接受性未证 |
| LC-G6 条件完成 | 部分有限循环与模型进展基础 | 新驱动启用/预算/无环等待和实际全历史提升未证 |

新 V0 的 T0–T9 仅按本阶段实际根重新分类：[详细清单](formal/00-foundations.md)。已交付基础不能按定理数量换算阶段完成百分比；只有公共来源登记也不是实现行为证明。当前 snapshot restore 在真实节点拒绝，内部 Intent 没有 Pause/Resume；核心中的枚举/定理不构成产品支持。

原 P1–P4/C1–C7/D 类清单以下保留为**旧范围的证据索引**，不是新阶段无条件前置。生命周期所需 open_checked、资源移动和新 schema 等仍要补；完整 Guest 编解码、无关容器、完整查询、两执行保密等归后续。准确新待办见 [#632](https://github.com/VeriInfra-Lab/sandproof/issues/632) 与 V1–V9，不从关闭旧 issue 推断完成。

本次文档内容涉及旧证明输入清单中的文档文件。保留原冻结摘要作为旧证据；新候选的冻结一致性/全套证明需要后续按工作流重新验收，本次未声称新输入已通过证明。不得只更新哈希消除该差异。

### 原中心生命周期首片：实际实现与证明缺口

新增 [原 CenterState 的受限 profile](../sandbox-center-core/src/lifecycle.rs)、
[同一 Redis CAS 接线](../sandbox-center-store/src/authority/lifecycle.rs)和
[可复现组件](../verification/experiments/stage1-center/README.md)，对应 #633/#634/#635。
本片保存完整相对提交参数与原分配器派生的规范请求、接受版本、owner/固定节点、预留及可枚举 outbox。
已有键按全文重放；Ready 连接原寿命守卫；Unload 暂不释额，Destroy 保留原终止预留。
仍未接正式 HTTP/driver、内部到期责任、完整卸载事实和 manager 实机重启；profile 不默认启用，
只存在 Creating/Running/Unloading/Terminating，不能把本片查询称为 Unloaded 闭环。

新增真实 Rust 的普通/Redis/错误版测试属于工程证据，精确源码、实际数量、耗时和失败以本批 PR 回执为准。
原 Charon/Aeneas 对版本检查和全文重放完成过生成；未运行其 Lean 内核/实际文件诊断或新增定理审计，
不称这些根已证明。完整接受首轮失败于两层原序查询；小拆分后仍在
`valid_lifecycle_session` 的循环预处理失败，原 IR/诊断保留。
完整决定、恢复校验、预留/修改范围与实际调用对应仍待证明，不能整体改成库/可信适配假设。

接口依赖提交 `3342edc12dbae5821656914423cb2c302f0d73ca` 的协议/internal 91 项普通测试及严格 Clippy 已通过；
`AcceptedLifecycleIntent` 是完整数据，不是授权凭证。Context 的 pin/受保护注册连接按
[准确 A9/B4 条件](formal/dependencies.md#当前中心身份的薄连接)采用，正式服务绑定仍待接通。
新增 `ResourceKind::Snapshot` 不改变 VM/Workspace 的含义，也不自动证明 snapshot 材料事实。

旧 #642 及旧 V0 局部定理只覆盖各自固定输入；这里的新 CenterState 字段、恢复 checker、
接受/提交和新增角色不能沿用旧结论背书。全局注册、冻结适用性和完整验收仍须在统一候选完成。

### 第一阶段依赖组合：调用版本与未证明连接

组合的代码输入为 `13cbea4b6856a3449de49832f66ef3ab9cbafd62`，
其中中心组件来自 #649 的 `e16f022338acf6d03c77d645a2e486dcbdf8f39d`，
原本地服务完整依赖截至 #651 的 `e542c7cb9077efa93fda8419192c1e549fee1a5a`，
原运行时完整依赖截至 #650 的 `5e8784429d3f823b79d5be5d2847ec45a9111259`。
本批实际组合和测试适配的结果见 #641 及对应 PR，不以来源 PR 各自通过代替组合验收。

#651 修改了原[分配准入](../controller-service/src/controller/admission.rs)、
[内部收件准入](../controller-service/src/controller/internal_inbox/admission.rs)、
[恢复打开](../controller-service/src/journal.rs)及启动派发的 epoch 检查，
并增加[原 SQLite 的 v3 收件](../controller-service/src/controller/lifecycle_inbox.rs)。
#650 的 [BootPlan](../sandbox-runtime/src/boot_plan.rs)实际用于
[原启动配置调用](../sandbox-runtime/src/linux.rs)。
这些新调用、事务/恢复连接、真实宿主 epoch 提供方，以及 BootPlan 重构前后的行为对应均未由旧定理证明。
下文旧结果仍只适用各条原记录列明的精确提交、冻结输入和条件，不能背书本组合新增或修改的函数；
本批没有新增 Lean 定理，也未迁移旧证明、刷新旧冻结摘要或执行完整证明验收。
普通编译、测试及严格静态检查属于工程证据；正式 profile 保持默认关闭，
真实材料登记、原 VMM 退出、必要 runtime 清理和安全复用事实尚未连接为公开 Unloaded。

### 公开接受与受保护 Recorded 收件的历史组件证据

本批在 `279002a0c7be5e0ec650a501f653516334eb0fd0` 的依赖组合上增加
[显式公开入口](../sandbox-api/src/lifecycle.rs)、
[原业务包络接受](../sandbox-center-service/src/business/authority/lifecycle.rs)和
[原 TLS 收件的当前源码位置](../sandbox-internal/src/server/lifecycle.rs)。
公开认证给出 owner，原中心状态给出 deployment/node/session；客户不能在正文选择它们。
相对参数按全文重放，首次接受才读取当前受保护模板、政策和时钟，并沿原 Redis CAS 保存。
该历史候选使用单独的可见性投影；当前开发已改为唯一生命周期行，不保留该旧运行分支。202 仅表示持久接受。历史组件结果不自动适用于当前重构。

受保护路由显式配置原 peer 注册后，将真实 TLS 提供的 pin 转成 context，
校验完整 accepted intent，再经原 HostFacade/HostLifecycle 工作许可交给
同一 `ControllerApplication::record_lifecycle_intent` 和 SQLite 收件。
Recorded 回执保留完整原请求及当前/终态数据，不表示 Ready、VM 已启动或资源可释放。
未配置路由保持拒绝；收件许可由实际 blocking worker 持有，取消等待不补造退出事实。
这些机械接线沿已登记 A9/B1/B4 条件核对，新的业务接受函数与原应用/事务连接仍须证明，
不因测试或已抽取的旧同步守卫而自动获得实现定理。

`CreateParameters` 逐字段迁到 protocol 并保留 core 原路径重导出；这改变抽取的名义来源，
原生成物及旧类型对应不能直接背书该新版本。没有手改生成 Lean 或刷新旧冻结摘要。
本批新增 Lean 定理为零；相关原生测试、失败及精确适用版本以本批 PR 的回执记录。
公开 HTTP→真实 CenterPort/Redis 与真实 TLS→原 Facade/SQLite 分别验收，
不称公开请求已经由正式后台自动派发。原 worker、中心持久待办的重观测/发布、
当前 Ready、新 Unload 和完整 Unloaded 均尚未接通；正式 profile 不默认启用。
内部到期仅有设计，未实施前不能启用该 profile；Resume/GC 和真实部署输入仍开放。

### 卸载结果连接的开发中范围

从 integration `14cfd421` 开始的卸载功能切片新增真实 `UnloadObservation` / `UnloadedFacts`、
中心原操作行的必填卸载结果、mTLS 观察与原 Redis CAS/授权查询连接。中心格式升至 9，
生命周期 profile 升至 6，旧格式明确拒绝；Host 格式升至 21，新增宿主材料持久责任字段由本地持久层提供。
已保存的完整结果可投影为 Unloaded，但生产提供方还须从原 SQLite 证明材料接管、
原执行及相关工作退出和运行位置安全释放。Pending、接收回执及快照存在均不能代替它。
原寿命不重算，原请求版本与查询时的当前操作另行核对；Destroy/到期不被迟到卸载覆盖。
已确认卸载释放中心运行 instances/CPU/RAM，逻辑记录、保留位置和原磁盘额度保留；
material_bytes 只是原完整 bundle 的计量事实，不证明宿主的事前空间预留。

功能源码 `0157e52af636946771918285e70c59f029352584` 在普通测试机通过本组格式检查、
中心核心/Redis 组件/内部传输的严格 Clippy，以及 `business_authority` 全部 37 项真实 Redis
组件测试（串行执行，零忽略、零筛选）。其中两项新增用例核对 Pending 不发布、完整结果保存、
原 Redis 重开后的原所有者查询与完整重放，以及 Destroy/到期不被迟到结果覆盖。
宿主结果在这些测试中受控提供，不是真实 SQL 材料接管、VM 退出或两个 manager 进程重启验收。
同组默认并行首次有两项在业务断言前因 Redis 端口被占用而失败；原日志保留，
随后保持全部源码和断言不变，仅控制测试线程数通过，未修改共享夹具或身份检查。

正式中心监听器现把原 SDK 与生命周期路由接在同一应用、用户凭据和请求预算上。
源码 `0bc9bbfac44c3738e03b7ec689864574c821d2b0` 在普通测试机通过格式、中心测试目标严格 Clippy
及实际 `boundaries::bind` 的回环 HTTP 组件回归：从原 Redis 登记读取窗口 9，无凭据返回 401，
占满原普通请求额度后返回 429，释放后恢复 200。此回归不使用独立路由替身，也没有启动宿主或 VM。
首次因原审计测试漏填新 `unloaded` 字段而编译失败的结果保留，补齐 None 后才实际运行上述用例。
[卸载重启场景](../tests/system/lifecycle_unload_scenario.py) 已编写公开创建、原所有者详情取得数据令牌、
固定 SDK 原构造器负载、卸载及原令牌拒绝、双 manager 重启后完整查询和原请求重放的断言。
它依赖同目录 runtime helper 的原服务配置、物理见证及精确测试收尾；整条场景尚未实跑，
不能把脚本存在、语法检查或上述监听器组件通过称为快照、双 manager 重启或正式 SDK 负载验收。

宿主连接继续复用原 `SourceWork`、SQL 事务、outbox、领取和完成入口。材料预算在 Pause 前保存；
同一次 Full 的原 worker 身份确定提交后才放行，结果未知时保留责任，不重新派发或退还材料预算。
材料登记后仍须原 VM 退出、VM/网络清理和原位置释放见证，才能保存可查询的 Unloaded；
释放见证属于原沙箱，因此位置被另一沙箱复用后仍可核对原结果。只归还已释放的运行 instances/CPU/RAM，
原磁盘、材料、逻辑记录和保留位置责任不因此消失。SQL 计数不等于文件系统已强制执行硬限额，
`material_bytes` 是所核对完整清单中的有效载荷长度之和，不表示分配块数或构建期间峰值占用。

宿主源码冻结 `f7cb8358fde76e08f77e802f0ea738c0664645e4` 的 ning 格式化快照
（上传包 SHA-256 `8a4c8a55949cd70a6c398459cdfd7fd9f05f306c889bdb4ef686c813d5f8317c`）
通过五项原 SQL 组件回归：Pause 前预算拒绝、来源不一致拒绝、完整请求重放只预留一次、
提交回执丢失后重开区分耐久与未提交结果，以及只允许一份运行额度时 A 卸载、B 复用位置后重开仍可查询 A。
五项均实际执行且未忽略；物理回执由受控宿主提供，不能据此宣称真实 Full、VM 清理或双管理进程重启通过。
后续 CleanupVm/CleanupSlot 分流属于待组合检查的实现，不能沿用上述回归背书其新增路径。

Destroy/到期优先撤销可恢复资格；迟到的原 Freeze/Seal/Commit 只登记其真实责任，不恢复授权，
也不生成旧卸载的 Stop。已经产生或可能产生材料时，本批保留原材料、保留责任和预算，
公开结果保持终止中或未知；没有实现最终 GC，不能承诺 Cleaned 或材料额度全部归还。
组合源码 `db9eb8022d4d8cb81c02bc400b7f47019000abe6` 在 ning 实际通过原 SQL 的单项三阶段回归：
分别在已领取的 Freeze、Seal、Commit 回执到达前记录 Destroy，再从原完成入口登记迟到事实并重开 SQL。
回归核对当前终态行不变、恢复资格撤销、不生成旧 Stop，材料责任、保留责任和预算不释放，
旧卸载不发布 Unloaded、Destroy 不冒报 Cleaned；Seal/Commit 的有效载荷事实仍保存。
该用例使用受控物理回执，未运行 VMM，不构成真实异步交错、物理持久化或 GC 的形式证明。

原纯核心的 workflow、worker 迟到处理、暂停位置释放和检查点校验，以及宿主原 SQL/额度连接均有新增行为，
尚未重新抽取和形式证明。既有 Lean 定理只对原适用源码有效，不能证明这些新路径；
编译、组件回归、实际 KVM、完整普通入口和形式验收分别记录，当前没有新增 Lean 定理。

这些修改尚未取得本功能切片的普通/实机/形式验收；已有同步协议与旧中心核心定理仅对原适用版本有效，
不得用旧生成体证明新增状态、字段或运行额度归还。当前仅按有限功能开发推进，未启动旧证明迁移。

### 当前同步封套切片与正式入口接线范围

本组件已检查的真实 Rust 候选为 `6751508c0862194d0100bedd7eece8514f8a8344`。
原 `stage1-protocol` 的 request 视图保留四个透明入口：`request_validate`、`replay_matches`、
`accepted_validate`、`recorded_matches`；后两项分别委托生产 `AcceptedLifecycleIntent::validate`
和 `RecordedLifecycleIntent::matches`。不并存私有 CurrentEnvelope 库，不增加翻译器或独立解释器。
当前结论来自本候选的实际生成体与原记录，不能仅沿用早期私有原型的结论。

唯一 [ValidationModel](../SandboxModel/Lifecycle/Protocol/ValidationModel.lean) 不导入生成函数，
[ReplayContract](../SandboxModel/Lifecycle/Protocol/ReplayContract.lean) 描述全文相等；正确版与错误版
使用逐字相同的独立规格和输入见证。已证明的范围如下：

| 真实入口 | 精确性质 | 不由该性质提供 |
|---|---|---|
| `request_validate` | Create/Unload/Resume/Renew/Destroy 的全部成功条件及六类首错；协议版本为 3，先核节点、部署和 expected 形状，再进入操作守卫 | 已解码值到真实输入字节的对应、授权和当前账本事实 |
| `replay_matches` | 任意两个完整请求值相等，当且仅当返回真；包含 Renew 全部字段、options 每个字节及完整摘要 | 相等的请求不因此合法、已授权或已经耐久处理 |
| `accepted_validate` | 按原顺序核对独立 context 的 center、部署、内层请求、slot 范围和 accepted 版本。Absent 要求版本为 1；Version(v) 要求 checked_add(v,1) 成功并等于 accepted 版本，耗尽不回绕 | context 确实来自原认证连接/注册、窗口可用和已持久受理 |
| `recorded_matches` | 完整原 intent 相等、current_session_version 不小于 accepted_version，且 current 恰好等于两版本是否相等 | validate 成功、terminal 的真实性、真实账本中的 current 状态或执行/释放资格 |

原请求首错依次为 UnsupportedVersion、ForeignNode、ForeignDeployment、InvalidExpectation；
随后 Create 分支可返回 InvalidCreate，Resume 分支可返回 InvalidIncarnation。
Renew 仍要求 Version 形状，但纯守卫不比较 end_at_ms 与当前时间或业务截止；
原负期限接受见证已保留，寿命政策由业务处理证明。Retention 只检查 slot 为 1..4096，
不证明当前窗口仍可用，也不要求操作 key 的 window 等于 retention window。
[Accepted](../SandboxModel/Lifecycle/Protocol/Accepted.lean) 与
[Recorded](../SandboxModel/Lifecycle/Protocol/Recorded.lean) 分别连接上述顺序与完整匹配合同。

本候选的实际检查与复用范围：

- 原三视图正向检查完成 47 次实际文件检查，去重后为 38 个文件。`2229bdfc` 的重新抽取保留完整
  LLBC，生成 Lean、模板和 translation 逐字相同；LLBC 仅原无序 short_names 表的排列不同，
  其它全部 AST 字段相同。到 `6751508c` 的 42 个实际抽取输入及 38 个证明文件均未变，
  因此复用原正向记录，没有重复运行正向内核检查。
- `6751508c` 在普通测试机的原完整单组件检查通过：两包共 113 项测试，零失败/忽略，包含封套七组；
  八个原阶段全部通过。四个真实单表达式错误版均实际编译成功，被各自未改的原断言以 101 拒绝，
  恢复原源码后重新编译并通过同一断言；四份完整源码均恢复，独立 native_case 收据已保留。
- 同一候选在证明机完成四种真实 Rust 错误版与六份形式模板：same-incarnation 的顺序见证和完整守卫合同否定，
  key-only-replay 的固定 pair 和通用全文重放合同否定，skip-capture-binding 的原 manifest 反例，
  session-only-envelope 对正确版同一 FullRecordContract 的否定。原 95 条入口命令全部成功，
  87 次实际文件诊断完整、非陈旧且无警告/错误；耗时 526.279 秒。审计包含辅助引理，不是独立系统性质计数。

四反例原结果摘要为 `021fdd3f0844fba956683285c02cf2f59cf3aaf71fe03c02a28f4cc24411c9ec`；
候选、源码归档、原生收据、正向复用和原服务返回均在组件原记录中相互绑定并经独立复核。
`0e981ae9` 的旧格式测试失败和 `9fcda217` 的测试辅助 Clippy 失败继续保留，后续成功未改写原失败。

该 V3 同步组件复用四项既有 [A6 标准模型](formal/dependencies.md#a6-v3-同步协议的字符串与完整字节视图)：
String.eq、Vec.is_empty、Bool.ne 和 Option.eq；封套的 Option 实例为 U64，字典调用/失败保持条件明确登记。
固定 Rust 编译器、原 Charon/Aeneas 翻译和模型与标准库的对应仍属显式信任条件，Lean 内核检查未关闭。
实际 bytes/Serde、TLS/注册到独立 context、完整调用者的顺序/取消、handler/账本/耐久回执真实性，
以及正常分配之外的 OOM/abort、资源 Drop/unwind、预算、保留前沿和 VM/release 连接不在本组件结论内。

[组件入口](../verification/experiments/stage1-protocol/README.md) 仍要求当前完整共享 native seal 和当前
functional 文档检查。上述独立单组件收据及六份形式反例没有补造 complete.json；
`verify-stage1-protocol.sh` 与 `verify-controller.sh` 尚未取得本候选的完整成功记录。
本节不宣称 V0、完整 V3、V1/V2 账本或系统闭环已完成。

### 同步协议的历史局部证明与适用边界

[草稿 PR #642](https://github.com/VeriInfra-Lab/sandproof/pull/642) 的生产 Rust 适用
`584e155163f641582e93f5d94538329e257c74f8`。相对首个字段提交，循环/借用写法作保持首错和短路的小拆分，
当时新增未启用的 codec；其[当前源码位置](../sandbox-internal/src/lifecycle.rs)已随唯一实现重构调整。
当时尚无新 HTTP 路由、持久收件或驱动。该历史提交保留旧路径，不表示当前运行实现仍支持两套合同；以下局部结论只认领列明版本，源码审查和对照测试不是一般性行为对应证明。

[独立 Model](../SandboxModel/Lifecycle/Protocol/Model.lean) 不导入被检查的生成 Funs。
三个真实生成视图、原完整 LLBC、准确输入/导入闭包和检查器由
[模块冻结](../verification/experiments/stage1-protocol/frozen.json)绑定；生成 Lean 未手改。
局部性质及其限制如下，精确候选的实际组件结果、耗时与失败记录见本批 PR，不以本表代替验收回执。

| 文件与声明组 | 对应范围 | 不能推广为 |
|---|---|---|
| Carriers，6 项 | CaptureId 的全 U64 构造/get、零拒绝、一个正常接受、最大值耗尽；任意 32 字节摘要值完整往返 | 其它角色构造已逐项接通、任意非最大 checked_next 已证、摘要已计算或文件已校验 |
| Request，5 项 | 任意错误版本首拒绝；其余绑定字段固定合法时任意 source/target 的严格顺序；capture 1/2 的完整请求重放 pair | 任意两请求全文等价、所有请求守卫的充要条件、认证/账本或窗口合法 |
| LoopWitness，6 项；ManifestWitnesses，17 项 | 原生成循环/checked 算术与一个路径/profile/完整文件集见证 | 任意文件集合、所有路径或循环业务都正确 |
| Manifest，4 项 | 任意错误格式版本首拒绝；原 capture 的 sequence 改变时先拒绝，不依赖其它 profile/文件合法；一盘完整清单接受且总长 3；同一 capture pair 合同 | 实际捕获、完整 profile 合法域、兼容性、持久发布或恢复资格 |

正常 manifest 见证使用可表示路径 `r`、一个 rootfs drive、state/memory/disk 三个文件各长 1、
32 字节零摘要；这是合法同步守卫输入，不是可恢复的物理快照。

三个真实 Rust 错误版分别允许同代 Resume、只比较操作 key、跳过原 capture 绑定。
正确版与错误版使用同一个独立 Model 合同，分别给出错误接受见证和合同否定；
编译/导入失败不能作为反例。第三种修改使原工具生成 loop0/loop1：
绑定不匹配的实际分支调用 loop0，私有辅助证明只更换六处真实 helper 引用，
不改错误 Rust、生成实现或 Model；原/新证明摘要随组件记录保存。
每个错误版有两项反例目标，manifest 视图还保留原 23 项辅助目标，不能因公共根未导入而省略。

生产字段/codec 的原普通组件记录包含 88 项测试（无失败/忽略，20.519539 秒），
三份错误 Rust 实际新编译后均被原测试同一断言拒绝（6.387291 秒）。
这些原始记录按精确 Rust/测试输入核对，不能替代新候选的脚本检查、形式组件或完整共享验收。
[复现入口](../verification/experiments/stage1-protocol/README.md)保留原工具重新生成、
完整 IR/生成物比较、所有实际文件内核/原服务诊断、准确审计和真实错误版；
源码恢复、单次 run_code 或裸 success 标记均不能代替该入口。

新增 [ReplayContract](../SandboxModel/Lifecycle/Protocol/ReplayContract.lean) 与
[ReplayEquality](../SandboxModel/Lifecycle/Protocol/ReplayEquality.lean) 把上述固定 pair 扩展为
任意两请求的完整值相等：`replay_complete` 总返回 `.ok (decide (request = original))`，
并给出接受/拒绝充要条件。覆盖 binding 的版本、节点、session、完整 key、expected，
以及 Create/Unload/Resume/Destroy 的全部字段；Create 的 options 比较包括长度和每个字节，
摘要比较包括全部 32 字节。它对所有生成值成立，不以请求预先合法为前提，
但不证明 serde 表示、请求守卫、鉴权、持久账本或跨重启重放。

本扩展的 Rust 输入来自依赖组合 `1c5a085c5df53bdc456c492ff79f9a4488d00e6c`，
未修改生产 Rust。新增[局部复现入口](../verification/experiments/stage1-protocol/replay_formal.py)
在独立源码/构建目录内用固定原工具重新抽取两版，保留完整 LLBC 和原样生成文件；
正确版 15 项审计、key-only 错误版 2 项审计使用逐字节相同的 `FullReplayContract`。
错误版由原 `negative_native.py` 登记的唯一 Rust 修改生成，形式反例明确否定同一通用合同。
实际文件经过内核及标准服务完整非陈旧诊断，源码、模型、生成物和日志摘要在局部回执记录。
此入口只报告局部形式检查：本批未取得 ning 普通检查和原生错误版回执，
共享 `verify-controller.sh` 仍因原冻结输入与当前候选不同而失败；
未刷新原 `frozen.json` 或共享输入摘要，也未完成共享模块登记和整套验收。
仓库保留的旧生成物仍属原冻结版本；上述新候选的对应检查以局部入口的新生成环境为准。

在全文相等扩展之上，[ValidationModel](../SandboxModel/Lifecycle/Protocol/ValidationModel.lean)
独立规定纯请求守卫的接受条件和六类首错，仅读取生成类型，不导入生成函数。
Create 复用原 `Foundations.CreatePlan.valid`；[Validation](../SandboxModel/Lifecycle/Protocol/Validation.lean)
的 12 项审计连接本次真实生成体，证明完整返回值、成功充要条件、每种首错充要条件和总返回。
版本→完整节点→deployment→期望类别→操作形状的优先级保持；Create 包括资源正值、单实例、
非空且至多 65536 字节的 options、期限关系与 checked_add 不溢出，Resume 要求严格更大目标代。
[ValidationExamples](../SandboxModel/Lifecycle/Protocol/ValidationExamples.lean) 另给出四种操作的正常接受见证。

本扩展仍使用 `1c5a085c` 的同一生产 Rust，未增加协议功能；实际 same-incarnation Rust 变体
把 `<=` 改成 `<`，错误接受同代请求，两个形式反例目标明确否定同一个 `FullValidationContract`。
局部入口的 `--property validation` 从当前源码重新抽取两版，检查 31 份实际文件、33 项审计，
其中 15 项是本证明直接复用的全文相等辅助/定理，新增正确版 16 项、错误版 2 项；
原默认 replay 模式也重新验证。实际结果以精确输入回执为准，不把重复审计计为新增覆盖。
此结论只刻画已解码、已表示值上的纯守卫，不证明角色构造器/serde、认证 owner、
账本期望版本确实成立、模板/options 可信装配、资源预留或任何 VM 动作已获授权。
宁侧普通与原生错误版回执、共享冻结迁移/登记及完整验收的缺口仍保持；未刷新旧摘要。

Request 的两个、Manifest 的七个外部函数按[逐符号 A6 登记](formal/dependencies.md)使用。
新增 `str::as_bytes = .ok value` 要求有效 Rust str 在原借用内提供完整原序只读 UTF-8 字节；
标准库与模型符合仍是显式条件，不是库实现已证。正常分配/普通值限制保留；
OOM/abort、资源 Drop、unwind 和 async shell 未覆盖。

新 codec 的实际 serde_json/有界 Writer 调用及自有预算/首错顺序已有普通测试，
其完整生成体/字节接线尚未由本组件证明，不把整段自有代码设为 B4。
认证 owner、原 capture 操作、当前 Resume 键、账本版本和受保护 profile、
原句柄/哈希/截面/同步、claim/pin/激活资格仍由各实际业务提供。
三个生成环境按既有独立审计模式检查；公共消费者只导入纯 Model，
没有同时导入全部生成类型，也没有接通本批的完整共享验收。
T1/T2/V0、完整 V3 及系统闭环均未因此闭合。

## 三项公共组件的组合范围：#626、#628、#630

本组合只接入已封存的三个组件，不增加新接口、不修改生产 Rust 或固定 Charon/Aeneas。来源分别为 #626 `8fa328e6`、#628 `00c0fb83`、#630 `93c12f51`；各组件已有证据，组合后的完整验收及合入适用性在 [#631](https://github.com/VeriInfra-Lab/sandproof/pull/631) 记录并另行核对。不能以来源 PR 的 CI 或独立组件通过代替组合结果。

| 组件 | 已有证明及实际连接 | 仍依赖或不覆盖 |
|---|---|---|
| [BTreeSet 插入](../SandboxModel/Foundations/BtreeSetBinding/Refinement.lean) | 五个真实 Rust 透明委托的原生成体；22项正确/辅助及2项返回值取反错误版目标；新键/重复键、原集合连续回写，原生同性质断言 | 固定普通键/Global/正常分配及实际比较的 A6 条件；不含任意 Ord/allocator/资源键、contains/collect/迭代或完整 validator |
| [Metadata 目录判断](../SandboxModel/Boundaries/ProtectedDirectory/Refinement.lean) | 原 directory 函数的完整比较/掩码/返回，14项正确/辅助及2项真实错误版目标，原生产/抽取工具与16份实际Lean文件 | A3/B4仅同一已有Metadata的3个getter；路径/FD取得、owner选择、替换竞态和完整受保护读取尚未证明 |
| [Tokio Mutex](../SandboxModel/Boundaries/TokioMutex.lean) | 21项条件模型目标，七组原操作绑定，真实Session提前释放副本的原生断言 | A6/B2机械异步接线仍可信；非async Rust机器证明。业务写入/授权/取消清理/IO及退出仍归调用者 |

本批全局入口只共同导入可兼容模块；目录判断错误版和正确版保持独立生成环境，全部实际对应和反例进入原形式任务。保留集合登记新增61个唯一审计目标；最终数量及实际执行/复用以同一候选的完整结果核对，登记数量不表示验收通过。G1/G2 尚未发布的新证明、BTreeSet contains 预检和 open_checked 失败试点均未纳入本组合。

## 旧 V0 剩余义务索引（适用旧目标）

历史收敛决定及验收记录见 [#333](https://github.com/VeriInfra-Lab/sandproof/issues/333)；#626/#628/#630 已由 #631 集成。下表是调整目标前的缺口记录；本节固定剩余义务的证据类别与边界，不以百分比或新增定理数量关单。来源是当前生产编译目录、D1–D11/52个共享字段位置、schema目录及25类/71个OS存储代表位置；这些有限目录仍不是已完成的语义调用图。新增用法须说明原条目为何不足，经单独认领才扩展，不能自动把未来所有Rust用法加入本轮。

| 编号/类别 | 当前尚缺的内容及已有证据 | 完成时必须能说明什么 |
|---|---|---|
| P1 公共能力核对：字节到文本 | `file_query::Query::into_text` 的三个可选字节字段和有符号expiration；现有String域不能直接描述非法UTF-8前态 | 先查原工具现有from_utf8接法；若已有则转入连接项。保留合法原字节、非法拒绝、None/空值、原首错和有符号值，不预造新字符串库 |
| P2 公共能力：剩余容器/借用形状 | 集合contains/collect/迭代、BTreeMap的Entry/get_mut/insert/remove、可变元素定位/反向查找、短路Result遍历；#626仅插入，不等于这些已证 | 以#333列出的实际函数和最小签名为限，给可复用合同或保持顺序/首错/回写的小重构接法；不预建所有标准容器或证明全部业务循环 |
| P3 公共能力：可观察资源移动 | 订阅列表retain_mut/drain、reaper的swap_remove及队列/字段载荷；已有纯值筛选、持额消息和Child观察不足以覆盖所有删除时机 | 表达原资源所有者、转交/替换/移出和实际Drop效果；不把普通值析构或引用计数观察当成资源已归还 |
| P4 公共能力：路径到原句柄 | `protected_files::open_checked` 试点Charon已通过，Aeneas仍在循环/借用状态合并失败，无可用Lean输出；#628只证Metadata快照判断 | 给出具体可抽取接法并保留原flags/身份/首错/部分效果，或单独审查准确新增条件；不把路径安全整体改可信，不扩成通用文件系统证明 |
| C1 已有能力的来源连接 | T0目录尚未把全部保留范围内的实际根/类型/外部API/析构关联到具体接口和业务提供方 | 每个已登记点都有已证公共接口、明确未完成公共项或实际业务任务；来源登记不能写成行为证明，不能删掉未知点造覆盖 |
| C2 类型连接与组件交付 | 已有身份/算术/完整守卫；G1/G2仅两个本地编号和准确版本记录的独立组件通过，未PR/未共享验收 | 52位置按实际域/单位/原参数/定理与条件对账；其它本地编号不得由两个样例自动覆盖，Lean的u64别名不自动隔离身份角色 |
| C3 普通JSON形状与输出连接 | LifecycleRequest/Status代表及有界Writer已交付；Probe、身份头、LifecycleResponse/结果确认、WindowClose请求/ack仍需实例连接 | 原字段/derive属性/叶子/切片/限额/错误顺序与值保持；原json_response实际Writer轨迹及200/500选择；不重证serde引擎或信任整段自有decoder |
| C4 实际回调连接 | 后台分支、Redis codec/clock、SQLite FnOnce及回调状态已有接法 | 原实例、完整参数、次数/顺序、捕获/结果/错误与实际自有函数对应；仅机械调用可信，完整授权/提交/恢复/释放业务留给所属模块 |
| C5 任务与资源接口实例连接 | 已有任务池、撤销、消息、Notify、JoinSet、标准锁/Condvar/RwLock；Mutex公共模型与七组有限接线已接入本页组合入口 | 原锁/池/信号/守卫/载荷和取消清理逐点对应；正常Drop、panic、abort、崩溃分开；发现确实缺规则时定位P3或明确新增最小项 |
| C6 存储公共使用条件 | Redis CAS/未知围栏、SQLite提交/缓存与条件UPDATE代表已交付 | 现有有限目录的版本/配置/原连接/参数/结果及部分效果对账；保留可能已提交与原身份恢复条件，不要求V0先证明全部SQL/Lua |
| C7 TLS/OS/流接口连接 | TLS/relay候选及Child观察已交付，原TLS来源已登记；原生产借用入口和外层复制/转交未获一般对应证明 | 绑定原连接/字节/完整证书快照/pins/摘要/预算及已有A/B条件；普通源码审查与机器对应分开，开始/就绪/停止/清理/排空/释放分别有提供方 |

P项表示公共能力尚未交付或需最小预检确认，C项可能仍需真实实例的小证明，不是只填文档。P2各具体操作及T3/T4余项以#333列明的有限函数族为准，不能用一个普通键接口承诺任意比较器/资源键/迭代形状。上述未完成项不能仅改名“下游”就消除；反过来，下列业务证明也不能全部加入V0前置：

- V1/V2/V4/V5：身份选择/分配、高水位/退休/恢复、全局容量、全部实际SQL/Lua、codec/validator、outbox、耐久确认、派发与生命周期（#334–#338）。
- V3/V7：认证端点/证书角色、实际请求与回执全文绑定、版本/身份/结果资格、窗口关闭/确认、公开输入授权（#336/#340）。公共JSON表示不授予执行权限。
- V6：真实路径/进程/VM/网络对象选择、Firecracker/jailer调用政策、围栏/停止/精确清理、整个reaper与释放资格（#339）。P4的共用打开接法仍属V0，完整宿主算法不属V0。
- V8：自有Guest协议解析与授权、命令Data-before-End、消费者落后/掉线政策、输入/字节传输、撤销和真实排空（#341）；Go/Connect/protobuf兼容解析不能一揽子当Serde可信，也不因此提前证明整个Guest。
- V9：具体初态、邻接业务不变量、容量、双执行保密及条件最终完成性（#342–#346）。业务启用/出队/预算/无环等待不能写成调度假设。

模板子系统和T9继续按既有范围排除；模板相关目录保留来源及核心侧输入责任，不把模板端点全部变成V0新增前置。#337历史生成物迁移不恢复。V0完成要求公共项有交付、连接项有实际证据或明确的业务实例义务；不要求下游整体完成，也不保证以后的新用法永远无需扩展。

## T2/T3/T4/T8 接口组合已集成：原 Child、回调状态与有界 Writer

来源 PR 为 [#617](https://github.com/VeriInfra-Lab/sandproof/pull/617)（封口 `e50f6b5b`）、[#619](https://github.com/VeriInfra-Lab/sandproof/pull/619)（`28facbba`）和 [#620](https://github.com/VeriInfra-Lab/sandproof/pull/620)（`74e9f53d`）。四个来源已由 [#622](https://github.com/VeriInfra-Lab/sandproof/pull/622) 正常 squash 合入 integration `139f4db6990f67c1ed9cd28c4ce28af707d469f9`，与完整验收候选 `22383847` 同树。ning 原入口 1914.273 秒、52份实际成功记录（58次使用含6次同批去重），a6000 原入口 2339.072 秒、27项全部 RUN；1506个唯一命名审计（含辅助/见证/错误版）与预期一致、27份全局实际文件服务诊断完整非陈旧，当前四项 CI 首次通过。实际合入后完整2138个跟踪文件、配置/工具及原始证据核对一致，ning2.631秒、a6000 29.885秒采用证据复用，没有重新测试、抽取或证明；详细摘要及失败见[最终验收](https://github.com/VeriInfra-Lab/sandproof/pull/622#issuecomment-6304788365)。生产 Rust、Charon/Aeneas 和固定工具版本均未改。

- [ChildObservation](../SandboxModel/Boundaries/ChildObservation.lean)：正常返回下，真实隔离同步核心与具体调用方已由原生成函数证明；成功有原 Child 的完整同一退出观察，未知返回前交出原 Child/Slot。15 项核心、3 项具体调用方和4项同性质错误版审计共22个不同目标；重复视图不重复计数。九个机械 API 仍依 [A2/A3/A6/A8/B4](formal/dependencies.md#a2a3a6a8b4-原-child-的退出观察与未知交接)。生产未采用候选，一般新旧等价、异常后态、整个 reaper 和 V6 生命周期未完成。
- [CallbackState](../SandboxModel/Foundations/CallbackState.lean)：真实 helper 生成体及有条件的状态/访问/幸存行组合，24 项正例与2/3项实际错误版审计共29个不同目标。原外部状态/参数接线依 A6/B4；toggle 是隔离代表，trace/calls 有原测试来源。不能据此声称所有生产闭包、panic 堆语义或完整 FnMut 机器精化。
- [OS/存储目录](../verification/experiments/os-boundary-catalogue/README.md)：25类/71位置有限源码与合同审查，沿用原818项/60变体来源记录；未重新捕获 MIR，没有新增程序定理或信任。目录状态是审查基线快照，不能据其 pending_617 历史文字重新打开已交付接口，也不能把 business_obligation 读为已证。

子进程原生驱动在共享接线中加强了同 crate 业务测试程序的 package/manifest/src/fresh 检查，逐变体只重建 sandbox-runtime 包，并在结束时收掉其错误版构建缓存；依赖、原二进制证据和 Lean 缓存保留。加强后的驱动已包含在上述候选的本次完整实际验收中，没有沿用旧驱动结果背书新准入。

[#621 有界 Writer](https://github.com/VeriInfra-Lab/sandproof/pull/621)的来源提交为 `1655c14e`，36 个正确声明和3个同性质实际错误版目标。真实隔离生成体覆盖写入、结束及响应选择；原生产连接仍依 A6/B4 的逐动作原语和精确来源/原生证据。两版已在上述完整入口中通过原抽取、36/3审计和17份专属实际文件检查；namespace适配、完整验收及CI已按同一候选核对，不由宿主或环境探针成功推定。 公共总根复用纯 Model；生成 ContractError 同名声明与 Foundations 分开导入，实际对应及正反例审计未删除，仍进入独立模块、全部文件构建和实际文件诊断。普通值/初始界/正常分配、任意 Serialize 的错误传播以及逻辑字节界与 RAM 的区别见[依赖合同](formal/dependencies.md#a6b4-有界-writer真实写入结束和响应选择)。

本组合已通过完整验收，但不等于 V0 全部完成。T0原生缓存曾遗漏根级合同/配置JSON；该缺口后来由 #629 修复并完整验收。#622 当时52项均实际执行，其历史证据不依赖该缺陷命中。T0 全量连接、其余 JSON/schema、Tokio Mutex、TLS 原借用来源及剩余公共义务继续开放；V1–V9 的真实对象/授权/提交/生命周期/系统组合责任保留，T9 与 #337 历史迁移不作前置。

## T0/T1/T8 来源准入与缓存输入修复：#629 已集成

[#629](https://github.com/VeriInfra-Lab/sandproof/pull/629) 已正常 squash 合入 integration `38dde5c9f00fcbc55b3d516c462a93adfdc8028a`，与验收代码 `6a9f1cba` 的完整文件树一致。ning 完整入口2115.227秒通过，52份记录实际执行、58次使用含6次同批复用；a6000 完整入口2392.938秒通过，27项形式任务全部执行、1506项唯一审计与27份全局实际文件检查通过。最新四项CI及实际合入的2159项源码和原始证据适用性核对通过；后者为只读复用，不称重证。[最终证据](https://github.com/VeriInfra-Lab/sandproof/pull/629#issuecomment-6307535361)保留探索失败和范围。原生记录改为逐字节绑定根级 JSON 合同/环境及候选对应配置，并绑定实际 SQLite/Child/Writer 原源码归档；旧版封口不沿用。根级配置失效缺口的原失败保留；ning 在首个修复提交 `225861bc` 上通过230项定向回归及文档/来源检查（13.613秒），不是整套验收。

来源 [#623](https://github.com/VeriInfra-Lab/sandproof/pull/623) `d4caa358` 的14个原TLS本地体及四入口，以及 [#624](https://github.com/VeriInfra-Lab/sandproof/pull/624) `50690eb4` 的D1–D11/52个共享字段审查位置已纳入候选的必经来源准入和独立冻结叶子。两单原组件/原捕获引用及失败保留；本批只重新连接实际父元数据，不改原15份Rust、39条既有定理引用、TLS原IR或既有模型。#624的375个词法候选不是调用数；其27个已有实现连接、2个条件叶子、23个未预检位置均为原审查快照，后续G1/G2预检不在此冒充已证明。TLS原借用入口仍未机器精化，一般旧新等价未交付；窄A6/A9/A12/B4、原对象/原字节及业务授权/撤销条件保留。本批新增Lean定理为0，不关闭全部T0/T1/T8或V0。

## T5/T6 Tokio RwLock：原读守卫与映射观察

生产 Rust 来源为 integration `e576a8475043c2d21b68231b7f6b1ead09be1952`，两份原源码和固定 Tokio 1.49.0 摘要见[来源登记](../verification/tokio-rwlock-environment.json)。模型首次组件提交为 `924e06a9d2b63d1395cd1edd4189e2d496537d56`；[公共入口](../SandboxModel/Boundaries/TokioRwLock.lean)、[接口条件](formal/dependencies.md#a6b2-tokio-rwlock保留原读守卫的共享观察)和[复现说明](../verification/experiments/tokio-rwlock/README.md)分别提供复用入口、保证与检查方式。生产 Rust、Charon/Aeneas、依赖与固定工具均未改。

42 条条件模型声明已通过 a6000 原服务的组合检查，以及五个实际文件的构建、内核依赖审计和完整非陈旧诊断。Core 提供读写排斥、领取/释放/遗忘及调用者写关系下的不变量保持；Mapping 保留同一个原读持有，失败 Err 仍拥有原守卫；Examples 给出非空多读者历史及同一 `ProtectedRead` 对提前解锁的拒绝。实际 A6/B2 事件、独立 Rust 所有权与整个守卫寿命仍是显式连接条件，不是完整 async Rust 实现证明。

原型的 ning 回归包括原会话与六项 API 夹具共 17 项、严格 Clippy 和实际 `Session::authorize` 的提前释放错误副本。错误版先成功编译业务库及测试二进制，再由正确版原轮换等待断言拒绝，不能称 Rust 形式反例。一次补测检出试验脚本复用上一错误版 rlib 的缓存问题，原失败和库/二进制保留；可复现入口逐变体重建本自有包，同时检查业务库、测试二进制与源副本绑定，并在结束时移除错误版包缓存。其它依赖与 Lean 缓存保留。后续正式入口另运行原文件授权的两个确定性回归；本段不以尚未运行的检查充当通过。

载荷保持只用于原锁继续存活的范围：最后一个 owned Arc 析构可能接着销毁锁与载荷。取消只投影对这次读写权的影响，不把 semaphore 内部预留视为已返回守卫，也不假定整个运行时不变。没有 FIFO、最大读者数、最终调度或硬返回时限定理。

固定 Charon 的原 `Session::authorize` 预检在 coroutine 和 `MetadataSource` GAT 处失败，带错误 LLBC 和原诊断保留。七组有限机械绑定不把三项令牌/签名、默认值/替换业务决定列可信；V8/V9 继续提供业务校验、旧快照使用、守卫寿命、内部进展和最终析构条件。T0 全量连接、其它锁和 T1–T8 剩余公共义务保持开放，T9 排除，#337 不作为前置。

本节记录上述已完成组件的精确范围；本批最终候选、完整双机验收、当前 CI、正式 PR/合入及实际合入适用性由 [#333](https://github.com/VeriInfra-Lab/sandproof/issues/333) 对应回执绑定。局部构建/审计不代替整套验收，不据此认领 V0 或部署系统完成。


## T5/T6 Condvar：原守卫等待与重新持锁

本批在 integration `646fc2c8569083cf4f03554f9acb2ca67bf22e6e`（#615）上开发，代码与正式接线冻结于 `75044cd8b68edbd5ef267364e25d598aadb31cfb`。生产 Rust、Charon/Aeneas、Cargo 依赖和固定工具配置未改。[公共入口](../SandboxModel/Boundaries/Condvar.lean)导出原守卫核心和有限等待协议；[实验说明](../verification/experiments/condvar/README.md)、[依赖合同](formal/dependencies.md#a6b2-condvar等待时释放返回前重新持有原锁)与[七组实际来源](../verification/condvar-environment.json)分别给出复现、适用条件和业务提供方。

46 项条件审计目标由 Core 20 项、Protocol 22 项及 Examples 4 项组成：等待暂停旧逻辑持锁权限而非执行守卫 Drop；正常返回或中毒错误均重新持有原 Mutex，保留被传入的守卫值及最初的 panic 标记；超时错误保留完整元组。协议要求匹配的活跃等待已可返回且原锁空闲，裸 `resume` 只是值构造器。其它线程的写入必须由调用者证明保持业务不变量。合法有限历史同时表明虚假唤醒不建立队列谓词、通知不缓存许可、超时观察不保证已重新取得锁或在硬时限内返回。范围限一次活跃等待，不包含多等待者、notify_all、公平性或一般异常清理。

ning 的原完整原生入口退出 0（1361.049 秒），129 项前置回归通过；48 份完整成功记录、54 次使用中 6 次同批复用。13 阶段 Linux 功能检查实际执行 574.367 秒；新增 Condvar case 包含四项原锁/守卫/中毒/展开用例、原 Reaper 回归、fmt、严格 Clippy 和新编译的错误锁夹具。原生封口 SHA256 为 `afb3317921d88949d11132236f2d179b4ac0b026ab6be3315ef7fd919102fdb0`；完整证据包 SHA256 为 `70181777e43c9e0fb78717eddbadb2e2b9f60f8f1f029934d0a5699196014e07`，a6000 用原校验器核对了全部回执和日志。

a6000 的原完整 `verify-controller.sh` 退出 0（入口 1480.961 秒，批次 1479.124 秒），23 项任务全部实际执行；内部同批公共生成去重不冒充模块跨批复用。1374 项预期唯一命名声明与实际审计完全一致，无缺项或额外项；该数字包含辅助、见证及错误版声明，不是业务定理数量。最终保留集合入口完成全局登记的 19 份实际物理文件诊断，全部完整、非陈旧；既有独立模块的实际诊断及正负检查同样保留。原始完整结果 SHA256 为 `8d9f98ccff20fbfd4072b184d670b013e06403ea497fddafb42c7302831af2cd`，批次回执 SHA256 为 `e1ca5d3859c816af278a615c9ecdbf71a8694c020369fd10add930308d28fdab`。全部新增证明临时和证据实际位于 `/data1/zn`，普通测试与 CI 没有搬到证明机。

固定原 Charon 对未修改的 `Reaper::run/reap_ready` 得到透明体；固定 Aeneas 在原 run 循环借用合并处报告 `InterpReduceCollapse.ml:1137`，预检原文与 LLBC 保留。整个回收循环、实际队列/退出/Slot 释放没有机器精化，也没有被整体列为可信。条件模型中“不重新持锁就返回”被正确版同一 Safe 性质反驳；原生错误锁只修改 API 夹具，两者均不称真实 Rust 的形式反例。七组源码绑定和原生对照支持 A6/B2 符合性的采用，不构成标准库或适配的一般精化证明。

旧 ca804a1c 轮次的权限拒绝、功能子任务完成后受控取消及 CLI 漏登记均保留，不算完整原生通过；75044cd8 同时修正 require-condvar 接线并检查所有正式入口的字面 case。一次性形式调度脚本的语法错误在启动前修正，原文件保留；正式入口、断言、审计和固定工具未放宽。四个 Condvar 实际文件进入全局诊断，24 份冻结清单保留全部原成员。

本批交付单消费者代表的公共条件接口及有限实际接线。V6 仍须证明原 Child/Slot、单一等待消费者、容量、全部业务写入、队列判断、中毒策略、真实退出和释放依据；通知或等待结束不替代这些证据。T0 全量连接、其它锁/消息接口及 T1–T8 剩余公共义务继续开放，T9 排除，#337 历史迁移不作为前置。最终报告提交、当前 CI、正式合入及实际合入适用性以本批 PR 和 #333 精确回执为准。


## 三窗口公共接线：Status、共享查询与 SQLite UPDATE

本批在 integration `2165f57f` 上保留 #609（`5b176a31`）、#611（`fb13711e`）、#613（`8320e736`）原封口提交的来源；共享接线与验收代码冻结于 `4302cc59ed70e2d664bf1359dd71ccc43a271a82`，统一交付见 [PR #615](https://github.com/VeriInfra-Lab/sandproof/pull/615)。生产 Rust、Charon/Aeneas 和工具版本未改。下面记录该精确候选已完成的验收；最终报告提交、当前 CI、正式合入及实际合入适用性由该 PR 和 #333 绑定，不能沿用上一批 1216 项审计背书新增模块。

- [Status 模块](../SandboxModel/Boundaries/SchemaDomains.lean)：22 项真实同步守卫对应和 24 项条件字节声明；两个实际错误版先编译抽取，再由同合同反驳。标准引擎/派生/原对象接线仍依 A6/B4；服务器有界 Writer 另见本页独立接口候选与验收范围，普通 Vec 响应编码只为参考。
- [共享查询模块](../SandboxModel/Foundations/LoopCallbacks.lean)：21 项原 Store::get/contains_key 及四个真实委托的对应；两种实际错误版分别由同一 QueryContract/MembershipContract 反驳。普通值共享借用、lawful 比较和单射投影条件保留；有状态回调和全部调用者业务不由本模块证明，源码邻域不等于完整调用图。
- [SQLite 模块](../SandboxModel/Boundaries/SqliteStatements/Model.lean)：隔离候选条件 UPDATE/修订计算的 38 项正例与 7 项错误版审计；固定五参数执行联合合同包含 raw_bind 失败和 step/reset 原次序，不把编码成功当绑定成功。保持独立完整生成视图，公共根只导入纯行模型；一般候选等价、完整事务/恢复和生产部署未闭合。

[依赖保证](formal/dependencies.md)列明具体 A4/A6/B4、普通值/Drop、失败与下游提供方；[真实使用登记](../verification/use-inventory/responsibilities.json)将本批来源/模块/条件挂到原编译器清单，仍不是完整语义调用图。原生产 Rust/Cargo 来源未变，原准入实际核对 818 项来源、60 个编译变体和 27 项已观察自定义 Drop 责任后采用原始 MIR 捕获，没有重复采集；新增验证根均经原工具实际重建与生成物核对，不能用清单代替证明。

ning 的完整原生入口在 `4302cc59` 退出 0（763.863 秒）：47 份完整成功记录、53 次使用中 52 次复用；包含原 13 阶段 Linux 基础检查，其当前全部跟踪输入对应的功能门槛本轮实际执行。118 项定向准入回归通过（7.131 秒）。未变化的 Rust/组件/原生错误版记录按原完整输入和原始日志摘要复用，不称本轮重新测试；三项新增 case 均已进入正式入口。原生封口摘要为 `dbc528bbe6a2d61cd17123768f77430e9e754a523e310e9e87370228d92e4532`。

a6000 的原完整 `verify-controller.sh` 退出 0（入口 924.178 秒，批次 922.204 秒）：22 项任务中 19 项本轮执行，3 个新增模块复用同一候选刚完成且输入一致的独立组件记录；其独立原入口先分别执行 234.282、182.854 和 157.890 秒。全部保留 Lean 文件、实际重抽取/比较、原正负例、占位检查和依赖审计保留；1328 项预期唯一命名声明与实际审计完全一致，无缺项或额外项，其中包括 SQLite 的 7 项错误版声明，不是 1328 条业务定理。15 份实际物理文件的原服务诊断完整、非陈旧、无错误/警告；三个新增 Audit 分别实际返回 46、21、38 项。原始完整机器回执摘要为 `4fdb2dcfd54b167754dcfdc4f3514ff9007d7701a46bdecc2f34d68705f914cc`。所有新增证明临时/构建/证据实际位于 `/data1/zn`，普通测试与 CI 没有移到证明机。

Status/TLS 的生成辅助声明同名，Status 公共根采用纯值词汇，原 Rust 对应和全部审计在独立环境保留；SQLite 完整生成根也保持独立。这不是删去目标或宣称所有生成根可共同导入。原服务仅在完整准入后转交实际审计消息，总清单不依赖 Lake 是否重建导入文件；缺项及额外项均拒绝。23 份冻结清单保留全部原成员并纳入新的实际输入，不以删清单解决失配。

`4302cc59` 的[当前四项 CI](https://github.com/VeriInfra-Lab/sandproof/actions/runs/39447973596)首次执行全部通过，未重试；#611 旧 SDK CI 失败 39404264085 仍未解释，新候选成功不等于旧故障根因修复。早期接线失败（独立 Lake 视图未继承完整父配置、回归漏导入、根导入位于模块文档后、生成声明冲突）及一次操作取消分别保留；只修正实际接线，不降低准入/错误断言、删去旧目标、手改生成 Lean 或清理 `.lake`。最终报告只补本节及其原冻结摘要，受影响门槛仍须在最终提交复核。

本批完成的是三个代表接口和当前保留证明集合的组合验收，不是整个生产外壳、任意 async Rust 或部署系统的证明。原 opt-in/特权忽略项不计为已执行。T0 全量连接、有界 Writer、其它锁/通知/OS 接口及 T1–T8 剩余公共义务继续开放，T9 排除，#337 历史迁移不作为前置。

## T5/T6 标准 Mutex：原守卫与真实唯一借用

本批从 integration `489f1ed5` 开始，集成来源更新到 `b973b40b`（含 #604 诊断、#610 Redis 重确认与 #612 测试失败诊断）；三份代表生产源码未改变，生产 Rust 和 Charon/Aeneas 均未修改。正式范围见[依赖合同](formal/dependencies.md#a6b2-标准-mutex原守卫中毒与唯一借用)和[十三组来源/责任登记](../verification/std-mutex-environment.json)。[公共模块](../SandboxModel/Boundaries/StdMutex.lean)包含 35 条条件守卫声明、7 条真实 get_mut/借用声明及 4 条连接声明，共 46 个正式审计目标。

原守卫模型区分忙碌与持有原守卫的中毒错误、恢复不回滚/清除、正常 Drop 不析构原 payload、取得时已 unwind 的建议性中毒、遗忘守卫仍锁住及独立的唯一借用条件。原 SerialMutex::get_mut 保留透明 Rust 函数体，由固定原工具生成 Types/Funs 并证明原调用、正常/中毒分支和回写保持；库模型仍依 A6。实际吞错版先编译抽取，再由同一 GetMutContract 的否定定理反驳；专用 into_inner 绑定只准入实际 PoisonError<&mut T> 调用。

ning 隔离试验已通过十一项原源码正确用例、严格 Clippy 和两份新编译的同断言错误版：延后 SerialGuard 解锁、吞掉 get_mut 中毒错误。a6000 已通过独立模块构建和原服务实际 Audit 文件的完整、非陈旧 46 项审计；早期启动命名错误及结果解包错误的原始记录保留，后者经严格核对原服务的嵌套成功结果，不称重新执行。正式可复现入口、最新候选的完整双机验收/CI 与最终合入适用性，以 #333 本批 PR 的精确回执为准；这些局部结果不能替代完整验收。

共享锁实际接线仍依 A6/B2。原 Meter::lock 和 PTY descriptor 的预检分别受非可结束共享借用、动态 I/O 错误类型限制；其测试不是机器证明。业务中毒策略、Snapshot/FD/许可真实所有者、授权、资源归还和无死锁/进展仍由所属业务提供。Condvar 的窄接口见本页独立记录；Tokio Mutex/RwLock、其他通道及 T0 全量使用点仍归 V0；T0/T5/T6/V0 保持开放，T9 排除。

## T5/T6 JoinSet：原任务组收齐

来源为 integration `886f2dd6`，生产 Rust 与固定 Charon/Aeneas 未改。[两个条件模型](../SandboxModel/Boundaries/JoinSet.lean)以 36 条声明为审计目标，覆盖组历史守恒、已完成未取出仍计数、原任务/正常值/完整原错误表示与至多一次交付、取消等待和 abort/Drop 的区别、逐 spawn 检查的容量与正常收齐。合法非空执行、提前 Drop 和只 abort 的同性质反例一并保留；[依赖合同](formal/dependencies.md#a6b2b3-joinset收齐原任务与返回值)和[十五组操作/责任登记](../verification/joinset-environment.json)限定范围。

本批代表连接是中心 `supervise`、受保护 HTTP 连接组与 Guest `Listener::serve`。隔离测试追加到原模块，使用原函数/类型；正确用例覆盖原回调收尾、正常返回值及原 JoinError panic 载荷中的许可、取消等待期间完成、abort 后新 spawn、析构 panic 与独立阻塞工作。两个错误版仅在私有副本去掉中心或 Guest 原收齐循环，须先新编译，再被正确版同一即时原任务断言拒绝。固定原工具对真实 Guest 异步入口的预检与部分 LLBC 单独保留，不交给 Aeneas 当成成功生成物。

局部候选已通过 a6000 原服务检查及 ning 原生试验；正式提交、完整模块/项目检查、实际文件诊断、完整双机入口与 CI 的状态以 #333 本批 PR 精确回执为准。当前不是 async Rust 的机器精化；机械接线仍依赖 A6/B1/B2/B3。中心错误策略、handler 业务、Guest Report、原许可全部别名及真实工作退出仍由 V2/V6/V8/V9 提供。空组或 JoinError 不证明外部进程结束，提前丢弃组也不满足收齐性质；T0/T5/T6/V0 保持开放。

## T5/T6 Notify：PTY 广播与原 Record 通知

本批来源 integration `70d08fd0`，生产 Rust 与 Charon/Aeneas 未改。[模型](../SandboxModel/Boundaries/Notify.lean)以 30 条条件声明为审计目标，分别覆盖单消费者通知合并、分配/消费/取消与数量上界，及先登记后检查的关闭广播、原身份和正常完成后的无漏通知。合法初态、非空成功、迟到拒绝和错误替换反例一起保留；[依赖合同](formal/dependencies.md#a6b2b3-notify关闭广播与单接收者通知)与[七组接线及两项责任登记](../verification/notify-environment.json)约束范围。

隔离 Lean 候选已在 a6000 原服务检查通过；ning 的 9 个确定性原生用例及严格 Clippy 已通过，其中原 `PtyMaster::close/read/write_all` 的广播错误版新编译后，被正确版同一即时 poll 断言拒绝。原 Record 类型/原 reserve 许可的测试只核对通知不直接归还原许可，不执行或证明 `publish/run` 的业务决定。原 Charon 预检在实际 `PtyMaster::read` 报告 coroutine 不支持；失败原文及部分 LLBC 保留，不作为语义反例，也不交给 Aeneas 当成功产物。

正式组合提交、模块构建、实际文件诊断、完整双机验收与 CI 的完成结果，以 #333 对应 PR 的精确回执为准，未完成阶段不由隔离成功代替。当前结论是条件模型及原源码工程证据，未形成 async Rust 机器精化；T0/T5/T6/V0 仍开放。关闭/退休资格、真实终态、字节/部分 I/O、保留期限、旧记录删除与原资源归还归 V8/V6。标准库 Condvar 另见本页独立记录；JoinSet 和其他锁/任务接口不由本 Notify 批次完成。

## T5/T6 Guest 持额消息

从 integration `81885df6` 开展，未修改生产 Rust 或 Charon/Aeneas。有限范围为真实 `CommandOutput` 的 mpsc/oneshot 交接、许可/引用随对象持有，以及 `Producer::frame` 到最后一个 Bytes 所有者正常析构的接线；[依赖合同](formal/dependencies.md#a6b3b4-guest-持额消息)、[逐操作登记](../verification/owned-message-environment.json)和复现入口共同约束范围。

本批以 53 条条件模型定理为审计目标，其中最后一个引用正在析构、尚未归还许可的阶段仍保留计费；原真实服务与完整项目的检查结果按下述精确回执采用，包含原身份、持额、正常析构、FIFO、oneshot 单次接收及接收方丢失后的发送收尾、空 oneshot 关闭时发送方仍活着的返回，以及有界 mpsc 的真实 None 条件。它们证明所声明模型，不是实际 async Rust 机器证明。真实 `CommandSubscription::next` 的原 Charon 预检在该函数报告不支持 Coroutine；失败原文与部分 LLBC 保留，不算业务反例。隔离组件使用原生产类型/函数，在消息检查前先中止并回收私有 helper，不执行命令驱动或会话业务；九个原源码正确用例、一个引用归零但资源尚未归还的受控原语用例、严格 Clippy 和先编译再由同一持额断言拒绝的提前释放版均列入原生验收。正式组合版本、原工具预检、完整项目构建、真实文件诊断、完整双机验收及 CI 的实际结果，以 #333 中本接口 PR 的精确回执为准。局部通过不能代替这些门槛，不据此关闭 T5/T6/V0。

Data-before-End、授权、消费者掉线策略、全部字节副本/引用的预算，以及真实进程/FD/记录归还仍归 V8/V6/V9。尤其不能从保留 `Arc<Shared>` 或收到 Frame 释放通知推出生产记录许可已归还；未知运行方丢失的 `forget` 分支没有因此证明。其他 channel/任务机制继续在 #333 开放，T9 排除。

## 七项边界统一候选的组合验收（#585）

本批在同一候选组合 #556、#563、#569、#574、#576、#578、#584。可执行与证明输入冻结于 `029840e4b1c82450c388ecc16d3655b45c5b9dba`。同步 integration 的 `b305b852` 只补历史关系；同步前后候选文件树逐字一致。各单项的历史冻结证据保留，最终组合以本节和 [PR #585](https://github.com/VeriInfra-Lab/sandproof/pull/585) 的精确回执为准。最终报告提交只改覆盖说明和依赖它的冻结摘要，另核对全部程序、证明、生成物和工具输入未变。

原生产编译器重新采集十组开发/发布构建，覆盖 18 个自有包、60 个编译变体、816 项 Rust/Cargo 来源；27 项已观察自定义 Drop 均有责任登记。原文保留，实际来源准入通过；它是代码定位与失效检查，不是动态调用或取消路径的完整证明。新增嵌套函数地址的 MIR 索引处理只保留完整原文并检查括号配对，25 项索引回归及重新整轮采集通过，没有修改 Charon/Aeneas。完整 foundations AST 重建仅发现一处宏源码 span 的结束列 8→25，生成 Lean 与 translation.json 逐字相同；采用原工具输出并更新对应来源摘要，没有放宽有序 AST 检查。完整组合的一轮验收另在 JSON decode 整图摘要处退出 1：原冻结产物与真实新产物完整比较，仅 valid_identifier 闭包所引标准宏源码 span 的结束列 8→25，两个完整直接调用快照及 encode 全图均相同；只更新 decode 摘要及七份父清单。原脚本再次独立生成、完整绑定/features 和 1699 项清单准入通过，检查器和生成语义未改，原失败记录保留。

指定 Linux 证明环境的原完整 `verify-controller.sh` 退出 0（4943.471 秒）：1035 个预期命名声明的实际依赖审计全部齐全，无缺项或额外项；该数量含辅助和见证声明，不是定理数量。所有保留模块、原工具重抽取、正确/同性质错误版及占位检查通过。原 Linux `check-function.sh` 退出 0（1285.831 秒），13 阶段和 1254 项工具测试通过，无 Linux 排除或工具测试跳过；Rust 工作区、文档测试及诊断配置均通过。同一精确提交的 [CI 完整步骤](https://github.com/VeriInfra-Lab/sandproof/actions/runs/39248077058) 已分别执行原 `check-compat.sh`、`check-bench.sh` 和 `check-distributed-contract.sh` 并成功；SDK/分布式任务与 Harbor 输入任务分开记录，不声称这三项在证明机上再次通过。兼容入口包含真实 SDK/网关与 Guest 组件，但无云端/VM；Harbor 入口只检查框架/回环合同，没有执行评测任务。证明机上额外安排的重复 SDK 入口因 PATH 缺少 uv 退出 127，尚未执行测试；原失败回执保留，没有改为成功，也没有为重复验收扩建 Python 环境。

八份实际 Lean 文件通过原服务完整诊断，文件指纹相符，非陈旧、无错误/警告及未完成依赖；客户端退出时原有 Event loop is closed 收尾提示完整保留，不是 Lean 诊断失败，未改客户端或过滤诊断。旧证明的样式警告在完整构建日志中保留，没有关闭检查。1799 个跟踪输入在每个阶段前后核对，完整 LLBC 仅允许原规则的无序短名排列，原始差异先封存后恢复；生成 Lean 字节未改。完整机器回执 SHA256 为 `4e29979479f5e88cfb18e97eb56df18f0efd8a17a130e69a0ccf8ed37b869377`。最终报告及同树合入复用这些相同程序/证明输入的实际回执，只重跑受影响文档和冻结准入，不重复运行未变证明。PR 的 SDK、Harbor 输入、标题及策略 CI 与上述完整验收分开记录；最终头部全绿后才合入。

首次完整轮次在维护测试中真实失败：夹具使用墙钟，慢盘同步期间越过所建实例的 300 秒寿命，原 get 正确返回 NotFound。仅该测试通过现有 Clock 注入固定时间，保留原 16 轮维护和全部断言，并增加推进时间后必须拒绝过期实例的用例；生产实现、安全检查和超时均未修改。严格 Clippy、格式与八项维护用例通过。原慢盘轮次未完成记录单独保留；最终完整验收的临时数据库使用实测持久 ext4 私有目录，保留原 fsync/事务设置、64 GiB 地址空间限制、CPU0–7、Cargo4作业和串行 Rust 测试，未用内存盘代替持久化或清理缓存。

真实 Rust 同步核心的条件证明、有限轨迹模型和可信机械适配的源码/AST/测试证据仍分别记录。A/B 与业务前提沿用[逐接口依赖登记](formal/dependencies.md)，不是将全部 Tokio、自有回调或外壳认领为已证；隔离候选未因此进入生产调用。未执行的原 opt-in/特权场景不计为通过，没有新增产品 VM、云 SDK 或部署系统验收。V0 的全部使用点、其它通道/通知/锁及 SQL/OS 代表连接和下游业务义务仍以 #333 未完成项为准；T9 继续排除。

## Redis 提交结果：隔离候选与完整验收（#564）

[冻结候选](../verification/experiments/redis-result/candidate.json)更新到生产基线 `f60e81f81629b23a512da52e6511b5f46699535f`，保留 #604 新增诊断；生产 Rust 和原 Charon/Aeneas 未改。隔离候选中的日志仅经已获批的窄输出接口调用，健康判断、返回码、配置与围栏仍由原工具抽取。接口模型保留写失败、抑制、panic 和不返回；不能据正常后态保证异常后围栏、物理日志写入或进展。精确合同见[依赖明细](formal/dependencies.md#a11b4-redis-提交结果接口的具体范围)。

审计入口保留原 131 项，另登记日志效果 13 项、错误助手 2 项和原业务字段保持 1 项，共 147 项；正常健康回写仅额外允许零资源日志端点的逻辑历史变化，原五项非连接字段仍不变。新生成物、完整 AST/Drop、来源回归、实际错误版及两个完整入口的最终适用提交与回执以 #333 和本次 PR 验收记录为准；这里不将局部抽取或旧组合结果当成当前完整通过。具体 codec/schema、Lua、一般旧新等价和部署闭合仍未因此完成。

以下保留旧 `824abe2b` 候选在 `dfaf79a7` 上的历史结论与原始证据；它们不背书 #604 后的新组合。

正式库包含[结果与写围栏](../verification/aeneas/storage/SandproofStorage/Results.lean)、[健康检查回写范围](../verification/aeneas/storage/SandproofStorage/Health.lean)、[泛型调用链的未知围栏](../verification/aeneas/storage/SandproofStorage/Caller.lean)及[原完整载荷和新修订来源](../verification/aeneas/storage/SandproofStorage/Success.lean)，共 40 项核心与辅助声明；[具体见证](../verification/aeneas/storage/SandproofStorage/Examples.lean)另有 91 项见证及辅助声明。中央 Audit 逐项审计全部 131 项，只依赖标准逻辑。成功性质显式要求同一 codec 状态链的往返，正常错误范围由自有 codec 提供，未把这些业务义务移入外部边界。示例 codec 会拒绝超容量和非法字节，并证明自己的条件；完整成功见证经过实际生成的写前健康、修订推进、CAS、写后健康和 Committed 构造，不实例化生产 JsonCodec/schema、物理租约或部署条件。

两个不同私有目录的原工具重建通过，完整核心/公开 LLBC 摘要一致，三个生成文件逐字一致；仅按既有规则规范化无序 short_names 字典，不删除其余 AST。七类公开接线错误和五类源码/析构准入错误被拒绝。全部 27 个不透明调用、52 个由原工具处理的内建/全局/析构声明、363 个 Drop 位置及两版标准库/固定 Redis 源码均有登记。真实 Rust 错误版只将 fence_unknown 的赋值 true 改为 false，编译、原 Charon/Aeneas 抽取及生成 Funs 构建先成功；原 Results 仅更换 import 后，被同一 fence_unknown_exact 与 eval_error_returns_unknown 性质拒绝，三个内核反例/允许输入声明完成依赖审计，未手改生成 Lean 或改弱性质。

完整验收冻结于 `dfaf79a710db0475c3390ac1b888d37b2c37024a`，全部构建、抽取、测试和证明在指定 Linux x86_64 验证环境运行。原 verify-controller.sh 退出 0（1569.987 秒），全部保留模块重抽取、构建、真实正负用例、占位检查及 966 项唯一声明审计完成；原 Linux check-function.sh 退出 0（1717.152 秒），13 个阶段和 1210 项必需工具测试通过，工具测试无跳过、无 Linux 项排除。候选严格 Clippy 与 40 项组件测试通过，其中 26 项使用固定真实 Redis，候选无忽略项；仓库原有 opt-in/特权忽略项不计为已执行。既有 Lean 源的样式 linter 警告保留，未修改旧证明或禁用检查。

本批正式文件的真实 Lean 服务完成 131 项 Redis 审计，最终主 ProofAudit 服务另完成 15 项审计；均完整、非陈旧且无错误/警告。服务准入不筛选行号、声明或严重性，检查实际文件与全部证明源码指纹；18 类无效回执及保留的虚拟 worker 崩溃回执被拒绝。1710 项跟踪输入最终一致；两份原 LLBC 仅允许的无序短名排列先封存、核对完整规范化摘要后恢复原字节。完整机器收据 SHA256 为 `fc9490f8e39bfdbf4eb63e26bec1de18f79eba4ced3c449ff3951c6c56f0f1c6`，原失败记录保留。最后补录仅修改本覆盖章节，不改变已验收代码、定理、工具或冻结输入。

交付依赖固定 #563（头 `824abe2b`，前置 #556），最终目标 integration。验收期间 integration 已推进到 `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；该版本未改 Redis 主体、根锁文件或本批依赖合同，但新增 Host CandidateVerification 传递与中心后台接线。它们须重新核对对应使用点；冻结候选的完整通过不自动背书最新 integration 组合，集成后仍需复验。

正常返回性质仅覆盖登记的普通 payload 值域，排除可观察自定义 Drop、共享可变或资源 payload；异常、panic/unwind、abort、崩溃及终止性不从正常后态定理推出。逻辑连接消失不是物理关闭、回滚或持久化证据，错误允许已发生部分效果，读取/reconnect 不能单独解除未知围栏。完整生产 codec/schema、全部 Lua、读取/reconnect、缓存/outbox/派发和崩溃恢复继续由 V2 提供；原实现与候选的一般等价、物理外部效果及完整 VM/TLS/云端部署未据此证明或验收。不关闭整个 T7/V0，T9 继续排除。

## V0 底座统一与组合验收

#515/#510 的公共任务统一归 V0 #333，按 T0–T8 验收；具体依赖保证见[依赖登记](formal/dependencies.md)。[汇总 PR #551](https://github.com/VeriInfra-Lab/sandproof/pull/551)组合已有 V0、任务/排空、隔离业务试点和旧生成链退役成果，不新增业务证明。下文历史记录继续绑定各自冻结提交；#551 的组合证据以本节和该 PR 的最终验收记录为准。后续接口扩展以本页 T6/T8 节各自冻结组合为准，不能把一个批次的证据用于其它未核对组合。

组合 91fc769be12eacba589afb9a839c3421823f43f7 在指定 Linux 证明环境通过原 check-function.sh（1752 秒、1166 项工具测试、13 阶段）和完整 verify-controller.sh（1229 秒）。143 个保留证明文件及公共入口编译，744 个唯一声明的实际依赖审计通过；两个总入口经原 Lean 服务完整诊断，无错误、非陈旧。原任务正确/错误版组件及 root 安装文件场景通过。1629 个跟踪输入最终一致；原始 LLBC 差异先封存，经原审计器确认仅为允许的无序短名排列后恢复。机器收据 SHA256 为 8ebe95f4a09ca3ce290509a5aa779784a7db47e9de0e48d0392a096a08692174。

SDK 修复的可执行代码冻结于 0a80a9468cf6f3c35a42446e51e9844f3a094020：相对此组合，只增加受限夹具修复、绕开预读的真实 Listener/CommandHttp 同步与异步回归及验收说明；生产 Rust、Cargo.lock、证明/抽取、工具和正式冻结输入均未改变。复用上述完整证明证据，不把测试修复计为新增定理。固定 SDK 原完整兼容入口已在同一类 Linux 环境通过（410 秒）；移除真实 keepalive 拒绝的版本先编译成功，再被原 PID SDK 断言及新增夹具回归拒绝。完整基础入口、普通 CI、最终报告提交和 integration 合入状态在 #551 绑定记录，不能仅据局部通过宣称全部门槛通过。

此前 SDK 提前拒绝断连的失败证据仍保留；#550 中复用的是测试夹具的接收/消费时序修复，没有修改生产处理器或降低错误断言。新增传输回归不经过预读层，且原“头拒绝不读正文”和鉴权顺序检查保留。它们只提供已声明时序的工程证据，不是整个 HTTP/SDK、Tokio 或任意网络交错的形式化证明。

**当前保留证明集合的组合验收已通过；V0 的全部使用点、完整异步 Rust、业务生命周期与部署系统仍未完成证明。** #333 的未勾验收点和 #101 的业务/组合义务继续保留，T9 仍排除。


## T0 真实使用来源与析构复核

[#577](https://github.com/VeriInfra-Lab/sandproof/issues/577) 的隔离来源候选基于固定 #556（398b114d），实际生产来源仍为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；相对该依赖不修改生产 Rust、Cargo.lock 或 Charon/Aeneas。原生采集代码冻结于 `1d051915`，发布/准入实现为 `570c8997`，两者的 Rust 输入和编译配置相同。采集由原生产 Rust 1.97.0 完成，不经过新的解释器，也不产生新的 Lean 程序定理。

[固定入口](../verification/use-inventory/config.json)的五组开发/普通发布构建在 a6000 全部退出 0，完整采集退出 0（611.036 秒）；覆盖 18 个自有包、60 个实际编译单位变体与 798 项 Rust/Cargo 输入。45,129 个函数体记录包含配置重复，不能读作独立函数已证明；完整原文、调用/局部类型/终结语句及实际 Cargo 产物保留在忽略的证据目录。[登记结果](../verification/use-inventory/catalogue.json)与原文回执核对，[责任清单](../verification/use-inventory/responsibilities.json)精确匹配 25 项已观察自定义 Drop。解锁、计数、Unknown、取消、转交子进程与是否归还仍保留原业务责任，不新增整类可信假设。

初版十组编译均成功，索引因发布 MIR 的单行/匿名常量等格式退出 1；该轮原文和失败完整保留，修正后重新执行上述整轮采集。四个保留抽取根另外实际重建，生成 Lean 逐字一致，完整 LLBC 按原允许规则一致；本批只更新已审查的文档、新增来源检查入口及相应摘要，没有更换工具或修改生成语义。发布/来源回归及真实缺项错误对照的结果见 #577。

验收代码冻结于 `ba92936119c08f4c14811672f596a7762d377bcb`。独立 Linux x86_64 检出的原完整 `verify-controller.sh` 退出 0（1800.266 秒），原 `check-function.sh` 退出 0（2005.029 秒，13 阶段、1223 项工具测试无跳过）；768 个预期命名声明全部由实际依赖审计覆盖，无缺项。来源检查及完整入口预检共 26 项定向回归通过，完整原文与发布清单核对通过；三个真实数据缺项错误均被对应检查拒绝，它们不是业务错误版的形式化反例。两个总入口文件经原真实 Lean 服务完整、非陈旧诊断（13.701 秒），主审计的 15 项依赖检查通过。1668 个跟踪输入前后核对一致；两份原始 LLBC 只有原规则允许的无序短名排列差异，先封存、确认完整规范化摘要相同后恢复，没有修改生成 Lean 或清理 `.lake`。完整验收汇总回执 SHA256 为 `63f9f7893736083cc4050b097f29530ae00f011ad0ed8d5588f0d85ac08a6671`。

首轮完整证明因 Lean 创建线程失败退出 134；同一失败命令在原 64 GiB 地址空间上限内，采用 `LEAN_NUM_THREADS=8`、`LEAN_STACK_SIZE_KB=65536` 后通过，上述完整证明沿用该资源设置，不更换工具版本、证明目标或内核。首轮基础检查另发现预检源码副本缺少 Git 上下文；修正测试副本的登记后，原 Cargo 哨兵/陈旧父摘要断言及新增未登记 Rust 拒绝检查均通过。两类失败证据保留，未放宽准入或安全检查。最终报告只补本节、将公共文档的机器别名改为通用环境称谓并同步对应摘要，可执行代码、模型、证明和生成语义与验收代码相同。既有 opt-in/特权忽略项不计为执行，未新增产品 VM、SDK 云端、TLS 或部署验收。

本项只加强已观察源码的清单与可复现定位。动态回调实际实例、结构体字段/合法域、future 取消时的编译器清理、嵌套库析构及外部脚本/配置的完整绑定仍按[依赖登记](formal/dependencies.md#真实使用点的来源清单)和 #333 继续处理。额外 VM 运维入口只作防漏观察，不增加历史工具证明前置；模板范围与 T9 排除不扩大。**T0 全部使用连接、T1–T8 其余义务和整个 V0 均未因此完成。**

## T7 边界：SQLite 提交回执与缓存发布（隔离候选）

[#570](https://github.com/VeriInfra-Lab/sandproof/issues/570) 的 Rust 输入固定为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`；本批验证代码冻结于 `c47c42329560cd226ae1ca48674d206f5d509fc3`，依赖固定 PR #556（398b114d）的最新 B1 接线复核。候选仅在私有副本应用[固定补丁](../verification/experiments/sqlite-commit/candidate.patch)，原 WorldTransaction::commit_with/Drop 实际调用新增同步核心。生产 Rust、锁文件和 Charon/Aeneas 没有因此改动。正式源、生成物和条件分别见[候选清单](../verification/experiments/sqlite-commit/candidate.json)、[环境与完整析构登记](../verification/experiments/sqlite-commit/environment.json)及[接口保证](formal/dependencies.md#a4b2b4-sqlite-提交回执与缓存发布接口隔离候选)。

[Results](../verification/aeneas/sqlite/SandproofSqlite/Results.lean) 的 13 条核心/通用状态性质连接原清空、回调结果分类、事务槽消耗、原候选发布和 rollback 分支；正常返回须取得原 live 事务，正常成功必须由原回调确认。[Examples](../verification/aeneas/sqlite/SandproofSqlite/Examples.lean) 的 5 条允许输入/结果见证分别保留提交未知的写前、写后结局，两者都不能保留旧缓存。18 项声明由原标准依赖命令与真实物理 Audit 文件检查，不增加自定义假设。这些是隔离候选真实核心的条件性质，不是整个生产事务外壳已证。

A4 提供固定 SQLite/rusqlite 的原事务结果；B2/B4 连接原事务句柄、两个原缓存单元及给定值/结果传递，要求不同单元、正常借用、回调及捕获值析构不额外改写缓存。逻辑状态表示两单元的值，不声称机器证明 RefCell 的堆实现。核心后态早于外层 WorldTransaction 候选字段销毁；普通 World 值域及原事务内部结束按登记限制，不提供物理关闭/回滚/释放、终止、panic/unwind、abort 或崩溃后保证，也没有证明原版/候选任意行为等价。

实际保留旧缓存错误版只删除首次清空，先编译并通过原 Charon/Aeneas，再由未改动的 cache_tests.rs:102 拒绝。正式 Results.lean 逐字复制，在四个对应性质处拒绝；三个错误见证/辅助声明经独立内核和标准依赖审计。核心 35 项前端声明/81 个析构位置，包装 49 项声明/117 个析构位置均完整登记；这些包含预声明，不代表每项均在运行。两版标准库各 26 份源及固定 SQLite 源核对通过，实际使用 bundled SQLite 3.53.2（3053002）。完整中间表示只按原规则规范化无序 short_names；生成 Types/Funs/translation.json 逐字再生核对。

指定 Linux x86_64 验证环境的原完整 verify-controller.sh 在 c47c4232 退出 0（1797.588 秒）：全部保留模块、实际重新提取一致性、占位检查及 786 个唯一声明依赖审计通过。专用 verify-sqlite.sh 独立退出 0（182.247 秒），正确候选的原 16 项缓存测试、真实错误版和 18 项物理 Audit 服务检查均通过。原完整 Linux check-function.sh 退出 0（1747.478 秒、13 阶段），1199 项工具测试无跳过，包含工作区与 contention-diagnostics 的 Rust 测试、文档测试和严格 Clippy。最后对 SandboxModel.lean、verification/ProofAudit.lean 和本库 Audit.lean 的真实服务检查退出 0（18.922 秒）；主审计 15 项和本库 18 项均完整、非陈旧、无错误/警告。1683 个跟踪输入按原规则核对一致；两份新 LLBC 的原始输出已保留，仅完整摘要确认无序 short_names 排列等价后恢复原序列化。预置工具链接另行核对。原 opt-in/特权忽略项不计为已执行，产品 VM、TLS、外部云端 SDK 及部署验收没有运行；这不是整个 V0 的验收结论。

完整 commit() 的自有验证器图在原 64 GiB 地址空间限制下仍出现分配失败，没有完整 LLBC；没有证明实际峰值为 64 GiB，也没有把验证器设为可信。具体 SQL/参数/列、完整投影/额度/寿命/outbox、候选正确性、data_version/autocommit、恢复与派发仍归 V5 #338。当前接口不关闭整个代表性 SQL 写入链、T7/V0 或全系统，T9 继续排除。

## T8 边界：TLS 首叶指纹隔离候选

[模型与对应](../SandboxModel/Boundaries/TlsPin.lean)及[候选范围](../verification/tls-pin-environment.json)提供两个实际同步 Rust 决定体的对应，以及显式 A6/A9/A12/B4 条件下的原完整输入组合。源码来自冻结 integration `8a33bc23`，以 `982bc63d` 为证明父基线；候选只通过[隔离补丁](../verification/fixtures/tls-pin-candidate.patch)安装到私有副本，生产 Rust 和 Charon/Aeneas 不改。正常功能、失败和剩余责任以[接口合同](formal/dependencies.md#a6b4-tls-叶证书指纹接口)为准。

验收代码冻结于 `d9d644a1cda45a2e21ce8c37dacecf248c80261a`。固定 Linux 环境的原 `verify-controller.sh` 退出 0（1632.875 秒），原 Linux `check-function.sh` 退出 0（1765.305 秒，13 阶段、1235 项工具测试，无工具跳过）。153 个保留模块及总入口编译，790 个预期唯一声明的实际依赖审计无缺项；最终两个总入口与 TLS 三个文件经原真实 Lean 服务完整、非陈旧诊断（24.417 秒），无警告/错误。1689 个跟踪输入最终一致；原始 LLBC 的无序短名排列差异先封存、按原完整摘要规则确认后恢复，生成 Lean 未改、`.lake` 未清理。完整验收收据 SHA256 为 `12feabf7d58a3a12f190fd7b2927c1b239af50470f6c6eec7e5a090d5227e840`。

本批独立 `verify-tls-pin.sh` 退出 0（191.953 秒），正确和错误候选分别新建 Cargo 目标并由原工具实际重抽取。正确版的 5 项原生检查、16 项真实 TLS 回环、22 条正式定理和依赖审计通过；把授权首叶误换为末证书的版本先编译/抽取成功，再被原 Rust 断言（101）及同一 `RustPinSafe` 的 5 条形式反例拒绝，真实服务与内核分别核对。12 项来源/类型/函数体/接口/审计防漏回归在完整基础入口通过，它们不算程序反例。

此前失败证据保留：反例夹具漏显式导入审计命令，已只补导入；过长临时目录使原 Guest preparation 超过 Unix socket 路径上限，同一二进制只改短私有 TMPDIR 后原全集 70 通过、3 项原忽略；严格再生另发现公共基础 `matches!` 源位置结束列 8→25，生成 Lean 和其它全部字段相同。固定原工具三次输出一致后，采用完整实际再生产物并更新原冻结字段，未扩大规范化规则或删除检查；新提交的三个受影响原门槛和上述完整入口均通过。旧 opt-in/特权忽略场景未计为已运行。最终报告只更新本节、短目录复现说明及既有摘要，可执行代码、生成语义和证明与验收提交逐字一致。

原借用入口的闭包/嵌套借用翻译失败仍保留，不以候选结论背书原函数。两份完整快照、原 pins、实际 SHA-256 和外层转交仍有明确可信符合性条件，原式/候选的一般 Rust 等价证明未交付；增加复制/分配的正常资源条件保留。已核对 integration `2325922a` 的 TLS/Cargo/依赖及 V0 计划输入未变，其 Guest/持久 PID 更新不由本冻结候选自动获得证明，最终组合仍须复验。整个 T8/V0、部署 TLS、任意异步取消及物理退出均未因本批完成；本批未查询、等待或重试 CI。

## 适用版本与边界

### 当前声明边界：固定工具、同步核心与逐符号可信适配

当前验证设计以 integration `0a23b57af60752f61067fc00b0b0b656e98347cb` 定位实现；这不是该提交新增证明验收通过的声明。默认不修改 Charon/Aeneas，用原工具抽取真实同步业务核心；Tokio 与[依赖登记](formal/dependencies.md)中逐符号列出的 B1–B4 薄适配可作为显式可信条件。验证桩只描述登记接口的允许结果和效果，不能替换自己的业务算法。T9 浮点兼容算法本轮排除，Guest 的其他安全/功能目标保留。

| 声明层次 | 可以认领什么 | 不能由此认领什么 |
|---|---|---|
| 已证 | 下文具体输入摘要、工具、生成根和定理审计所列的局部成果 | 当前 integration 全部业务、全系统或新增适配已机器证明 |
| 可信 | 依赖与 B1–B4 登记的互斥、任务/许可移动、取消信号及参数/结果机械交付，按精确版本审查/测试 | 整个 Host/网关/ControllerJournal 正确，或正确的 Completed/Released 结果 |
| 待证 | 授权与完整输入/快照绑定、真实回调算法、动作/提交顺序、未知/部分效果解释、完成/释放判断及组合不变量 | 用适配合同直接消除这些业务义务 |
| 条件进展 | 登记的调度/FIFO适用条件、外部操作最终返回、真实退出最终可观察 | 公平自动启用工作、宿主调用必定结束、消除内部循环等待 |

此边界调整没有新增定理、重抽取或替既有失败补录通过。下面保留各历史批次的实际状态；历史“未证明”记录不因改列可信而升级为已证，旧测试也不背书新符号/源码。Host 当前健康获取先占 status 名额、再在同一个 work 门排队、最后由后台持有两张许可，须独立绑定最新实现；旧单许可/R0 试点不覆盖其新增等待与取消路径。Busy、JoinError、panic、调用方取消都不能统一推断为“回调从未执行”；未知效果和真正退出仍需原对象证据及业务处理。

来宾监听器按原 `buffer_bytes` 限制实际读取，避免 HTTP 缓冲分配余量被当成更大正文帧；此安全 I/O 适配和对应帧/上传回归尚未抽取或证明。它不改变原正文限制，也不是对 HTTP 库、分配器、内核缓冲或整个进程内存的证明。

#311 的输入准入及单次 HTTP 中继响应 EOF 收尾只有工程回归，尚未抽取或证明；宿主健康等待的真实异步接线仍只有工程核对，新增条件任务合同见下方 T6 节。当前单次大输入在原 HTTP 正文预算内按内部写块传送，不再按内部块大小返回 429；下文分别登记整次输入顺序、可能交付前缀与真实正文超限拒绝的义务。后续仍需连接拒绝前无写入、未知失败不重放、实际子进程退出与许可归还，以及健康等待/取消不增加并行工作、不延长原观察有效期；不能将有界等待推出任意宿主操作必定终止。旧成果不为新网关／异步调度背书。

本批 envd 文件组件及模板验证待确认守卫只有工程检查，没有新增对应证明。合并基线中已有的公共基础局部证明仍只适用于其登记输入；来宾依赖改变工作区 `Cargo.lock` 后，不能将旧输入摘要用于当前组合背书，也不能仅改摘要宣称重新验收。模板子系统整体继续排除证明，但原终局回执精确确认后才结算自有引用的连接义务保留。

历史整控制器生成物及其专属实现对应证明已按 owner 决定退役，恢复入口见下节；它们不再属于当前检出版本的已证清单，也不为当前 Rust 服务或二进制背书。当前公共协议、IPv4 判断及网络入口/出口的局部对应范围见下节；新版本地核心、持久化、寿命、宿主执行及网关仍未完成当前版本的全套对应证明。逐批工程测试结果查相关 PR，不在这里复制实验日志。

## 历史整控制器抽取链退役

按 owner 决定，从 #544 的依赖复验基线 `110918e` 删除历史 `SandboxControllerCore` 的生成 Lean、翻译清单、全量 LLBC、重生成脚本，以及只依赖该生成物的 `RustRefinement` 证明层。同步移除 Lake 库、聚合导入和对应旧定理审计项；没有把陈旧产物标记为最新，也没有迁移旧业务证明。

删除前的完整文件可从 [Git 基线 110918e](https://github.com/VeriInfra-Lab/sandproof/tree/110918edd42557a8590065419c47099df9a73423) 恢复；原纯核心成果基线 `0f1901a210e24da50d259e07df75e8c2cb9d181f` 和生成语义源码基线 `8612ef1bf99e1ecbddcbd61e8b5fdeedd69e58fe` 只作历史来源。当前树不维护其逐函数覆盖表，不把旧结论计入当前实现覆盖。

保留 V0、Network、Access、Local、FirstChain、FirstChainLinux 的模型、指定实现对应、生成物、原抽取根与全部准入检查；独立于旧生成物的数学模型也保留，但只作为模型定理。`verify-controller.sh` 继续执行 Rust 回归/静态检查、各保留模块的实际重抽取一致性、全部剩余 Lean 源编译、依赖及占位审计，并显式纳入首链模型门槛。它的通过只表示**当前保留证明集合验收通过**，不表示当前整控制器或部署系统已完成精化。

退役实现冻结于 `18928a60212fde31aaa9bf2db6bc219c49412927`；`8bacc83293a9a060ee5dc95117329f910f15c269` 只修正 README/V0/状态机说明及三份传递输入清单，Rust、Lean 证明体、工具和可执行验收代码均未改变。固定 Linux 环境的原 `check-function.sh` 在前者退出 0，耗时 2213 秒，1118 项工具测试无跳过，工作区 Rust/Clippy/doctest/诊断特性检查通过；修正提交另通过四份完整冻结清单、文档和 61 项受影响准入回归。

首轮 `verify-controller.sh` 在 `18928a6` 的 Rust/Clippy 通过后，因漏同步本次改动的 V0 设计文档摘要退出 1（1114 秒），原日志保留；没有把它归因于外部环境或旧业务。修正提交 `8bacc83` 的完整入口从头重跑并退出 0（1386 秒）：真实重抽取一致性、V0 118、Network 43、Access 85、Local 153、Linux 首链 32、首链模型 53 及保留数学模型 15 项依赖审计通过，共 499 个唯一审计声明。全部 107 个保留证明文件及公共入口完成编译，所有保留模型/抽取目录的占位检查通过。另一干净构建也完成 108 个目标编译；两个变更入口的真实 Lean 服务诊断完整、无错误、非陈旧，独立内核仅有标准逻辑依赖。

87 个保留模块文件相对 #544 逐字节未变；重抽取产生的两份原始 LLBC 仅有原准入允许的无序短名排列变化，已先封存再恢复私有副本，1463 个跟踪输入核对一致。119 个正式 Lean 源的最终指纹与真实服务记录一致；最终机器收据 SHA256 为 `ab8b0557c67b2fbe58f6aa903c5aa5975e3319a936fb3f1b47c2bec8f423ae8c`。原特权/opt-in 忽略项不算已执行，没有产品 KVM、云 SDK、题集或部署验收。最终报告提交只补录本节，不改变冻结实现。

**当前保留证明集合的完整验收通过；当前整控制器和部署系统仍未完成实现精化。** #337 继续负责当前生命周期/工作/维护义务，不能因移除旧生成物阻断就关闭该任务。该退役基线未包含 #545–#548；它们的组合验收另见本页开头，不能由退役单独推出。

## T6 边界：Host 公共任务与双池合同

以下保留 [PR #547](https://github.com/VeriInfra-Lab/sandproof/pull/547) 的原冻结记录；该成果已随 #551 进入 integration，后面的历史局部退出码不代表当前组合仍被旧阻断拦截。原生产基线为 integration `4a5dfdafd4465fef8ec2832f89798f1f16dd09b2`，分支承接 #545。当前合同、源码和库版本以[环境清单](../verification/task-contract-environment.json)、[输入摘要](../verification/task-contract-inputs.sha256)及[B1 登记](formal/dependencies.md#b1-host-任务与原许可池的逐符号可信适配)为准，不修改生产 Rust 或 Charon/Aeneas。

本次复验纳入 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`，验证代码冻结于 `ed3910ec04d5d9bac5f2f9630676e3d7c9a8d746`。原 `lib.rs/status_admission.rs/facade.rs` 机械接线逐字未变，驱动新增的可选模板校验凭据仍在同一 blocking 回调内传递，测试夹具增加对应计数。环境清单补全五处实际调用及边界；模板凭据与缓存正确性没有移入 B1。本轮 29 条条件模型定理、7 项许可组件和 2 项原状态许可测试通过；三个错误副本各自独立编译出新二进制后，被原资源断言拒绝（测试退出 101）。两个完整入口均通过，精确组合证据见下方撤销等待记录；不将这些结果称为回调业务或实际 async Rust 的机器证明。

[Task](../SandboxModel/Boundaries/Task.lean) 提供单次调用的持额／取消／结果来源、任意有限调用集合的双池预算守恒与容量界限。原池绑定、合法执行、逐个析构许可和反例合计 29 条定理纳入模块及总审计。a6000 正式模块编译／29 条实际依赖审计通过，三个正式文件的真实 Lean 服务诊断均 `success=true`、`partial=false`、`stale=false`。这些是接口模型定理，**本模块没有把实际 async Rust 交给 Aeneas 翻译**。

真实 Host 普通任务及健康查询使用点以源码审查和隔离测试核对 B1。冻结代码 `bc1c934a1a7a3794a9b95a39a61244b4d5fdf35c` 的第三份干净检出执行完整模块门槛退出 0（211.313 秒）：29 条内核／依赖审计、7 项新增真实 Rust 组件测试及 2 项原状态许可测试全部通过，包含“队列已获额但尚未再次 poll 即取消”。普通提前释放、双许可提前释放和错误池三个真实副本分别以新 Cargo 目标目录构建出新二进制（构建退出 0），再被同组资源断言拒绝（测试退出 101）；不是编译失败反例。原完整证明入口另一次执行同一模块门槛也通过，两个正负复现收据均已核对。旧共用 Cargo 目录的一轮结果已弃用。

同一冻结代码的原 Linux `check-function.sh` 退出 0，耗时 2103.310 秒，13 阶段全部通过，包含 1136 项工具测试、Rust 工作区／doctest／Clippy 及诊断 feature 检查。原完整 `verify-controller.sh` 退出 1，耗时 1866.146 秒：Rust／Clippy、Task 29、DataLease 12、公共基础 118、网络 43、授权 85、本地 153、Linux 首链 32 条审计实际通过；全部 446 个透明函数翻译耗时 186.136988 秒后，仅在 #337 历史 Types/Funs/清单一致性拒绝，未执行其后的旧证明总构建／审计。六项跟踪生成差异封存后恢复，未导入类型模板另归档；两入口前后 1595 个跟踪输入一致。原 opt-in／特权忽略项不计为已运行，没有产品 KVM、云 SDK、题集或部署验收。

后续 `b3d3e830ad5bdc5cb73cbcb44741a37969aeeb2e` 仅澄清 A6 接管闭包与 B1 跨回调持额的责任、补录复现并同步文档摘要，Rust、Lean、测试及执行脚本与冻结代码逐字相同；a6000 的文档、来源、7 项准入回归、Task 内核／审计及原四个模块门槛再次通过。最终三份服务诊断完整且非过期，230 个正式 Lean 源均为跟踪文件，源码指纹为 `a940f597814612878c80caa13f8c79f59e44972c652bd41d76a0295e9229014d`。本页最终提交只补录结果。最终机器收据 SHA256 为 `db8f9476443530062e6682155ee58f1f83f00c361ea85be540166cf8ad4229aa`；完整日志与收据在忽略的 `artifacts/task-contract-formal-20261017/` 保存。局部合同与组件验收完成，整套证明仍未通过。

本批不声称回调业务、完整异步实现或 Tokio 已获机器证明。调用者仍须提供具体业务核心定理、真实输入／池／部署绑定，以及 B1 所登记机械接线符合合同；有限并发测试不替代该符合性的形式化证明。容量结论是同一家族 work/status 门的任务额度，不是 VM／物理资源总额度。#510 的 R2 业务分支、R3 网关撤销／退出和 V9 组合仍保留；T9 排除，#337 仅记录历史阻断。

## T6 边界：撤销等待

[Revocation](../SandboxModel/Boundaries/Revocation.lean) 是 [B3-until_revoked](formal/dependencies.md#b3-撤销等待的逐符号可信适配) 的独立条件模型，来源为 integration `a2b8a4b6ce5c170c953130b57abc8c2d6c468d0b`，当前精确输入见 [清单](../verification/revocation-contract-inputs.sha256)。24 条定理覆盖原信号/工作标签、选择后无新 poll、正常析构凭证、返回分类、允许竞态、错误分支顺序及 B1 组合；正在 poll 的可见前缀可在 panic 或不返回前保留。中断时不补造清理凭证，也不据此认定没有发生析构。**真实 async Rust 未被 Aeneas 抽取；其接线符合性属于登记的可信适配条件。** 这不证明完整网关授权、撤销后再无任何 I/O、relay 退出、持久提交、物理释放或无条件进展，也不关闭整个 T6/V0。

本批验证代码冻结于 `ed3910ec04d5d9bac5f2f9630676e3d7c9a8d746`，纳入上述最新 integration。a6000 的原 `verify-controller.sh` 退出 0（1220.163 秒），原 `check-function.sh` 退出 0（1682.707 秒、1199 项工具测试、13 阶段）。全部保留证明编译、实际重抽取一致性及 768 个唯一声明的依赖审计通过；总审计根经原真实 Lean 服务完整、非陈旧诊断，无 Lean 错误（6.966 秒）。本轮十项真实 Rust 正例通过；work-first 副本先独立编译成功，再被原 `prior_revocation_or_lost_authority_never_polls_work` 断言拒绝（101）。编译失败不计为语义反例。

1660 个跟踪输入在两个入口前后按原审计规则核对一致；两份原始 LLBC 只有允许的无序短名排列差异，先封存、确认规范化摘要相同后恢复，未编辑生成 Lean 或清理 `.lake`。最终机器收据 SHA256 为 `5c84f9e45d4d6600e31a5607d1d57a73753f8cd51dbd25dab4d7ffe2a17b948e`。最终报告提交只补录本页，程序、证明和冻结输入与上述版本相同。首版 poll 表示的修订、旧失败及恢复记录保留在 #554/PR #556，不以历史局部结果背书本组合。

当前保留集合与声明组件的完整验收通过；原 opt-in/特权忽略项不计为已执行，没有新增实际 async Rust 的抽取证明，也未执行本批产品 KVM、云 SDK 或完整 VM/TLS 部署验收。T6 其它任务机制、具体业务/释放/退出及 T0–T8 未勾验收点继续开放，T9 排除。

## T8 边界：双向字节中继（隔离候选）

归 [#575](https://github.com/VeriInfra-Lab/sandproof/issues/575)/#333，来源为 integration `8a33bc230dbbaefd7bc6855e715c1dd5e63c1173`，证明组合依赖固定 #556 `398b114d516bf9233875823c18a2f9618cf108b7`。代码与证明冻结于 `73789e6840fc926b78dcc13586f53b642596666d`；[候选补丁及复现](../verification/experiments/tunnel-contract/README.md)只在私有副本拆出并实际调用 `decide/finish_upload`，生产 Rust、Charon/Aeneas 与工具版本未改。原 async 入口的 coroutine 不支持及带错误中间表示保留为失败证据，未冒充有效抽取物。

[Contract](../verification/aeneas/tunnel/SandproofTunnel/Contract.lean)/[Results](../verification/aeneas/tunnel/SandproofTunnel/Results.lean) 的 29 项审计声明由 8 条实际生成体值性质、3 条条件步骤连接和 18 条模型/历史/允许执行性质组成，涉及两个真实 Rust 同步函数，**不是 29 个 async Rust 函数证明**。上传先返回仍等待原下行；下行错误优先，否则保留原上传结果。下行先返回允许结束尚未完成的上传；非 biased select 同时就绪时保留两种选择。复用原 StreamHistory，保持原绑定、两个方向的字节前缀和待写预算，给出零字节、上传未读完即可正常返回及满足预算的任意有限源允许通过等见证。允许轨迹存在不保证实际调度最终完成；本层 Ok、raw EOF 和写端接受字节均不等于 HTTP/Connect 成功、客户端读到、子进程退出或 VM 排空。

[A6/B3/B4 登记](formal/dependencies.md#a6b3b4-双向字节中继与结果处理隔离候选)约束原 I/O、四个实际管道/TCP 对象、所选结果和动作接线。实际 poll/await/Pin/Drop 到历史的对应仍是已登记的可信符合性条件，不是完整 async Rust 的机器精化。当前实例限于固定匿名管道 Receiver/Sender、原 TcpStream 的两个 owned halves及无可观察资源/共享状态析构效果的错误载荷；自定义 I/O、带业务资源的错误析构或额外许可/FD 捕获不能直接套用。关闭/Drop 不补造真实退出或释放证据，panic/unwind、abort/崩溃和无条件进展仍在范围外；完整请求绑定、授权、协议、预算和退出/排空由 V6/V8 提供。

正式重放核对 28 个非空前端声明、37 个正常/异常析构位置、完整 AST/目标/配置、原工具及固定库源。原版/候选各 5 项 relay 和 2 项真实管道回归通过，候选严格 Clippy 通过。真实 Rust 错误版只在下行成功后丢弃上传错误，先独立编译及原工具抽取成功，再被原 `upload_write_failure_preserves_the_guests_rejection_response` 断言拒绝；逐字相同的 Contract 在同一 `finish_download_ok` 性质处失败。错误生成体上的三个独立反例/辅助声明通过内核及标准依赖审计，不导入失败证明。有限旧新回归不构成一般行为等价证明。新准入门槛拒绝遗漏/隐藏声明、变更析构、缺失/重复冻结输入及陈旧/不完整/错误指纹服务回执。

本冻结提交在指定隔离 Linux 证明环境的专用 `verify-tunnel.sh`、原完整 `verify-controller.sh` 和原 Linux `check-function.sh` 分别退出 0（146.901 / 2259.132 / 1700.550 秒）；797 个唯一声明审计、1199 项工具测试无跳过及全部 13 个基础阶段通过。1676 个跟踪输入按原规则核对一致；两份旧 LLBC 仅有原规则允许的无序 short_names 排列差异，实际新输出先封存，确认整个 AST 一致后恢复冻结序列化，未删除其它字段或编辑生成 Lean。最终三份物理文件的真实服务诊断完整、非陈旧，无错误/警告：主入口、15 项总审计及新中继 29 项审计均通过（16.954 秒）；共同证明源指纹为 `4835959930aa9bdddf6ac3f3de02d74d11cc421e5ef7fe901988508baca2d2f0`。机器收据 SHA256 为 `a2cf8aed7d401a057184aeeec893c6af285938aa8b144e6563ba963075959884`。

旧模块的 Lean 样式警告保留，未关闭 linter；真实客户端在完整结果后输出的 Python 关闭提示也保留，不冒充 Lean 诊断或忽略失败回执。最终报告提交只补录本页，程序、证明和冻结输入与通过版本相同，另核对报告差异、文档和全部清单。本批未执行产品 VM、云 SDK 或完整 TLS/外部代理部署验收，原 opt-in/特权忽略项不计为已执行。其它 OS/Firecracker/TLS/I/O 使用点和 V8 全业务仍开放，不关闭整个 T8/V0，T9 排除。

## T5/T6 边界：DataLease 只读判定

归 #515/#510，V0 只复用该接口登记和验证接线，不将网关业务加入 #333 完成前置。生产源码定位为 integration `4a5dfdafd4465fef8ec2832f89798f1f16dd09b2`，本批承接 #544 的依赖复验和 #540 测试修复；实际源码、生成物、模型和验收输入以 [DataLease 输入清单](../verification/data-drain-inputs.sha256)为准。原文件 `controller-service/src/application/data_drain.rs` SHA256 为 `12833632ecaeb1d37088119d6b9ea7e03c40f550dc484e5d2b0003f6629b02a5`，未修改生产 Rust、Charon/Aeneas 或证明工具版本。

独立正式库为 [DataDrain](../SandboxModel/Boundaries/DataDrain.lean)，包含自然数／布尔观察策略、实际类型表示和真实生成函数的对应，12 条定理纳入[审计](../SandboxModel/Boundaries/DataDrain/Audit.lean)。实际生产文件通过 `#[path]` 引用，两个转调根只调用原 `routes_dropped/drained`；四个可达自有函数均保留实际结构化体。`routes_dropped` 精确判断本次 Arc 强计数是否为 1；`drained` 返回真当且仅当本次计数为 1 且条件读取的 unknown 为假，unknown／非单持有者拒绝及可表示正常接受均已连接。此结论不是整个共享状态协议或排空／释放证明。

三项标准库读接口的符合性是显式 [A6-data-drain-read](formal/dependencies.md#a6-datalease-只读观察接口) 条件，不是已证明的 Arc／Atomic 实现。观察可以发生在不同时间，模型不承诺原子快照、跨调用稳定或全部写入可见。实际 Acquire 调用、原文件来源、可达函数／类型／外部接口、无错误或析构扩张及两版库源码由独立检查器核对。管理锁、禁止新增 pin、对象绑定、unknown 的建立、持久撤销和物理退出／释放决定仍由使用者提供，不能因本读体证明而整体可信化。

负向复现修改私有副本中唯一真实表达式，删掉 unknown 检查；错误版编译／抽取成功，但原生同一测试以 unknown 断言失败拒绝，Lean 以可表示的 `(count=1, unknown=true)` 输入证明它不满足与正确版相同的 `RustReadSafe`。原文件保持不变，反例经同一服务及独立内核／依赖审计；编译失败或服务失败不算反例。复现命令见[验证链说明](../verification/README.zh-CN.md)。

正式模块入口为 `scripts/verify-data-drain.sh`，已接入完整控制器验收及总依赖审计；仍分别报告模块结果与完整入口结果。新增库和合同文档影响的公共冻结记录只在原抽取／内核复验后同步。整套入口中的历史生成物迁移仍归 #337，本批不迁移旧业务证明，不新增 V4 生命周期或系统完成性覆盖。
## 数据许可登记与重新派发的隔离试点

关联 #510/#515。可审查输入与复现入口见[数据许可实验包](../verification/experiments/data-permit/README.md)，源基线冻结为 integration `0a23b57af60752f61067fc00b0b0b656e98347cb`。包只在独占副本中把实际 `data_access` 路径接入有限同步守卫，不改变生产 Rust、默认抽取根、工具或工具版本；其局部结论不能背书未接入候选的原生产入口。

本批范围为宿主 gate 前提、撤销通道的保留/新建规则，以及重新派发时的撤销、配置和 VM/代次匹配守卫。原锁、watch/订阅、共享对象与机械调用的符合性保留显式信任条件；真实调用绑定由源码审查及有限应用/SQLite 对照支撑，不称一般性循环或异步精化。该旧包本身不包含凭据、完整授权/寿命/权限算法；下一批局部连接见下。存储/恢复、真实进程启动与退出及完整网关仍未因此证明。局部结果与原整套验收的环境阻断分别记录于实验包和关联 PR，不升级本页其他历史覆盖。

## 重新派发的凭据、寿命与 Access 隔离连接

关联 #510/#515，冻结实现仍为 integration `0a23b57af60752f61067fc00b0b0b656e98347cb`；输入和复现入口见[授权重查实验包](../verification/experiments/data-authority/README.md)。它依赖上述数据许可包的原冻结守卫，不修改旧包、生产 Rust、默认抽取根或 Charon/Aeneas。

隔离调用方实际调用的凭据核心覆盖注册项缺失/代数、反向完整绑定、原字节格式与完整摘要相等；寿命核心覆盖原始 SQL 行的 owner 字节及所有数值/Option 字段、正数转换、时钟、精确 Tenant 归属、Running 和截止瞬间拒绝。原 Access 三分支及其真实 Store 查询、事件和完整 World 复制直接抽取，给出精确错误与成功条件。组合使用同一 world/record/actor/代数的明确参数投影，把实际核心结果连接到格式、摘要、行字段、时间和能力的数学条件，没有用“授权有效”布尔值替代业务算法。局部证明、三个错误版同性质拒绝及原始/候选应用对照的准确结果由包内验收入口和关联 PR 记录。

这些机器结论适用于隔离核心及原式 Access；原始/拆分调用路径的一般等价仍未交付，调用连接由独立源码审查与有限真实应用/SQLite 对照支撑。外部原语与获批锁/gate/watch/机械传递按窄合同依赖；自有 SQL/参数/列、身份格式、凭据登记/poison/恢复、缓存与快照、初次完整授权、实际 relay 退出/排空与全局预算仍待证。Closed 测试是显式故障注入，不是合法持久历史可达性结论。生产未接入，不能升级为完整异步系统已证；原整套门禁结果与环境阻断另行报告，T9 继续排除。

## V0/V4 组合基线的分层验收

本次依赖复验源码组合为 `5fc87754ae8f7814d2cc155f4569041f7ce029c7`：integration `4a5dfdafd4465fef8ec2832f89798f1f16dd09b2` 加入 #540 的启动文件测试修复。实际证明输入仍由本次更新的 [V0 清单](../verification/foundations-inputs.sha256)、[共享状态清单](../verification/network-inputs.sha256)、[Linux 首链清单](../verification/first-chain-linux-inputs.sha256)及[首链模型清单](../verification/first-chain-inputs.sha256)绑定；没有修改生产 Rust、独立抽取工程、生成 Lean、模型、定理或工具版本。

相对 `8c03525f5671c70d0a3ff5b10a40bb18b0817deb` 的冻结集成，根锁文件新增 `tz-rs` 与自有 `sandproof-stream-codec`，Guest/数据传输的依赖关系及工作区成员改变。固定 Linux 目标下，相关核心在生产工作区和三个独立抽取工程中可达包的版本、依赖关系及解析 features 均与原基线一致；这些解析结果不能单独代替实际抽取。重新使用原 Charon/Aeneas 后，三组生成 Lean 和翻译清单逐字节相同，完整 LLBC 经各原审计器仅允许的无序表规范化比较一致；原始新 LLBC 保留为实验材料，没有手改生成体。

同一 Linux 证明环境的内核构建及 V0 118、网络 43、授权 85、本地状态 153、Linux 首链 32、首链模型 53 项原定理依赖审计通过，只允许标准逻辑依赖。六份正式审计文件也通过项目原 Lean 服务客户端完整诊断，均无错误、失败依赖或超时，`partial=false`、`stale=false`。完成这些检查后才更新根依赖摘要与 V0 清单的传递引用；没有移除输入、缩小抽取根或改弱验收。标准库/翻译工具的既有合同继续保留。

这次复验只恢复上述既有局部成果对当前依赖组合的版本绑定，不新增业务覆盖，不表示完整生命周期、异步接线、系统组合或整套证明验收完成。该次复验时将历史全量本地生成物迁移归 #337；现已按上节决定退役，#337 保留当前业务实现义务。此处仅保留退役前的冻结组合验收来源，不构成恢复旧生成链的要求。

归 #101/#333/#337/#342。组合源码为 `2c5b6aa3f5dbc9b974779c0aefc11287c515b21f`，保留两线全部冻结成果和当前 integration 的工程代码；`3235e7ee5f0a6428cc5b78c7675769332bfbdc2d` 只修正首链模型对公共清单的传递摘要引用，没有改变 Rust、模型、定理、抽取根或检查脚本。实际输入仍以各模块清单为准，清单更新不是重新验收的替代品。

该组合在独立 Linux 测试环境完成原 `check-function.sh`：1034 项工具测试无跳过，Rust 工作区各检查模式、静态检查、文档和本地链接检查通过。普通 PR 的四项 CI 也通过；不据此宣称 KVM、评测或完整系统证明通过。

证明在另一个独立 Linux 工作区执行，使用私有 Ubuntu 24 用户态、glibc 2.39 和既有固定工具二进制；没有升级宿主系统、重编译 Charon 或改变证明依赖。原 `verify-controller.sh` 实际通过 Rust 回归/静态检查，以及 V0 118、网络 43、授权 85、本地状态 153、Linux 回执 32 项定理审计和对应模块的重新抽取一致性。全部本地控制核心翻译用时 161.933676 秒完成，没有 OOM；随后历史 `Types/Funs/translation.json` 的原一致性检查拒绝，退出 1，未进入全部历史定理的编译与审计。候选生成物和失败诊断保留在私有忽略目录，未用它们覆盖旧成果造绿灯。**局部集合通过，整套验收仍未通过；历史业务迁移继续归 #337。**

首链模型另在 `3235e7ee5f0a6428cc5b78c7675769332bfbdc2d` 执行原 `verify-first-chain.sh`：传递输入摘要、V0 再生检查、模型编译及 53 项依赖审计通过。它仍是模型不变量与条件连接，不能代替中心/节点真实持久微步骤、宿主证据或完成性证明。同一 Lean 服务已在组合版本实测目标读取、隔离尝试及正式诊断；检查的公共插入文件 `stale=false`、`partial=false` 且无错误，不将交互检查推广为未检查模块已证。

## T2 / V0：完整生命周期请求守卫的局部对应

归 #558/#333，开发基线为 #556 的 `398b114d516bf9233875823c18a2f9618cf108b7`，直接 Rust 探针冻结于 `c2c0e707413e612991679488dae28ca15eff3736`。生产 `LifecycleRequest::validate` 未改：身份检查先行，其后核对完整 CreateBinding 和 Renew 非零形状。[模型及合同](../SandboxModel/Boundaries/JsonRequest/Model.lean)、[实际生成体对应](../SandboxModel/Boundaries/JsonRequest/Guard.lean)复用原 V0 类型和身份/创建参数定理。新校验根在原共享生成链中重新抽取，原 176 个不同名称的透明函数体仍全部存在，未新增不透明接口；没有把自有守卫交给 A6。

a6000 的真实 Lean 服务探索、正式模块构建、12 条实际依赖审计通过，正式审计文件诊断 `success=true`、`partial=false`、`stale=false`，仅有标准逻辑依赖。性质覆盖任意表示请求的接受充要条件、原拒绝顺序、非法创建/零值续租的拒绝，并有合法输入域及真实守卫接受见证。另在独立副本删除原身份检查：先 Rust 编译成功、原工具重新抽取成功；原错误断言以 101 拒绝，Lean 内核又证明同一 `GuardContract` 不成立。负向证书 SHA256 为 `2f4643ee0aa459e86159beded8b5f000ae01fbcbd7a8d8e3d29438b1d5cb090c`，没有把编译失败当作反例。

基于 `6d18cfa34199a27c3ede384621250c70d8f308ba` 补入字段连接后，本模块共有 42 条局部审计：原 12 条守卫结果之外，另有九个自有字段回调到完整输入域、完整字段投影的单射、实际编码转换和字节接口条件下的往返，以及先解码再查尾随输入的错误顺序。它们在 a6000 完成模块构建、实际依赖审计和真实 Lean 服务诊断（完整、非陈旧、无失败依赖）。Codec 的字段字节关系/派生合同仍为显式参数，Encoding 是标准派生的字段规约；这部分不是具体 parser、全部异步 Rust 或 serde 内部已获机器证明。

新增原解码器矩阵检查六个编号字段和三个计数器字段，保留字段错误早于后续截断 JSON 的首错，以及固定 Intent unit 分支忽略额外字段/Renew 拒绝额外字段的实际差别。共享抽取 crate 全部 37 项原生测试通过；sandproof-internal 全包 24 项及严格 Clippy 通过。实际 InternalClient 经本地双向 TLS 到原 InternalServer，核对三种 intent 的完整参数到达原回调，七种无效绑定/形状在回调前拒绝；原未知根字段、大小、错误身份和容量检查均保留。该组件回调只记录字段并回显原结果，不执行宿主业务，也不代替真实 VM/部署验收。

本节只报告这个同步守卫的局部结果。八项真实 JSON 回归与严格 Clippy 已通过；完整字节解码、标准派生/原叶子回调、具体 parser 状态/错误字典和编码往返仍在连接，标准派生的有限不透明预检不计为这些目标已证。新根的正式准入/摘要、负向自动入口及原完整证明/基础验收尚未收尾；旧回执不背书当前开发候选，不宣称 #558、T2 或 V0 完成。`GuardContract` 不提供授权、原寿命、持久提交、异步调用等价或宿主效果；普通有限值库合同及实际字节边界条件仍须由各调用者落实。

## T2 / V0：八类字符串编号 JSON 叶子接线

状态：本接口局部交付完成，[PR #548](https://github.com/VeriInfra-Lab/sandproof/pull/548) 进入审查，整套证明仍被 #337 历史生成物一致性阻断，不关闭 T2/V0。代码及全部可执行验收冻结于 `498a82fecc9a26370bc086ac53ba86eee90d3542`；最终覆盖记录提交只改本节报告。依赖 #546/#481，原生产协议候选为 `208fb4e1bdba78be269a14626b3185d275249b1a`，未新增生产 Rust 或工具改动，不声称候选已合入 integration。

九个[直接调用根](../verification/extract-foundations/src/json_ids.rs)覆盖八类编号 Deserialize 和原 String 读取，与原 V0/计数器共享类型。自有解码、校验、TryFrom、错误闭包和 Display 保留原工具生成体；[正式模块](../SandboxModel/Boundaries/JsonIdentifiers.lean)的 18 条定理连接实际调用、完整字符串保持、合法接受、非法拒绝和原错误/失败/发散/效果传播。成功值满足独立 `IdentifierSafe` 的非空、最多 128 字节及 ASCII 允许字符条件。固定 Linux 环境中，原公共 260 条与计数器 12 条审计保留通过；新增及四个原模块门槛均实际执行通过。最终八个正式文件的真实 Lean 服务诊断完整、无错误、非陈旧，独立内核审计仅依赖标准逻辑。

真实 JSON 的 24 项定向回归覆盖八类编号的借用、转义和拥有字符串、边界与首错顺序。冻结提交归档后的完整负向复现退出 0：正确模块通过，错误版删除真实宏中的校验守卫，在独立 Cargo 目录先编译及抽取成功，再被八条同一安全断言以退出 101 拒绝；真实 Lean 服务和内核另验证同一 `IdentifierSafe` 的通用八类反例。源码归档 SHA256 为 `ee31d1a4faae81c74e0dbf2383af361016eb2501baba78e75f3471b5e5f71122`，未改性质、调用根或生产工作区来制造反例。

同一冻结提交的原 Linux `check-function.sh` 退出 0，耗时 2225 秒；1061 项工具测试及 Rust 工作区、文档/格式/Clippy/doctest/诊断 feature 检查通过。完整 `verify-controller.sh` 退出 1，耗时 1747 秒：Rust/Clippy、新编号 18、计数器 12、V0 260、网络 43、授权 85、本地 153、Linux 首链 32 项审计通过；446 个透明函数翻译用时 219.026652 秒，随后仅在 #337 历史 Types/Funs/清单一致性处拒绝，未进入后续旧证明总编译/审计。六项跟踪生成差异和未导入类型模板已封存后恢复；V0/网络 LLBC 的差异仅为原允许规范化的无序短名表，历史核心的规范化结构确有变化。前后 1392 个跟踪输入一致，241 个正式 Lean 源的最终指纹与完整服务记录相同。最终机器收据 SHA256 为 `589d3e8fc0ec012a1d0c520d61bdbf7c5099f5cbfbf1737c50496ff0f29f4c02`。原特权/opt-in 忽略项不计为已执行，没有产品 KVM、云 SDK、题集或部署验收。**本接口局部通过，整套证明未通过**；#337 仅记录，不迁移或跳过。

可信条件和失败范围见 [A6-json-identifier-leaf](formal/dependencies.md#a6-json-字符串编号叶子接口)：四个 Serde 默认方法的有限字典模型、标准 String 释放、有效 UTF-8/正常分配及唯一冻结的格式参数形状；F64 仅是不参与运算的错误载荷。没有将自有校验移入 A6。可观察自定义 Drop、资源许可、取消和 panic/unwind 收尾不在本接口范围。完整 JSON parser、复合 schema/编码往返、实际输入和字典绑定以及授权仍开放；#337 只记录历史阻断，T9 排除。

## T2 / V0：JSON 非零计数器叶子接口

实际采用共享 V0 抽取：60 个生产调用根和一个明确不透明的标准库辅助根均受原准入检查约束。原 260 条公共定理及新增 12 条叶子定理已重新编译、审计，并核对同环境组合导入；没有删除旧根或旧证明。

本批依赖 #481 及其前置真实 Rust 重构，原协议输入为 `208fb4e1bdba78be269a14626b3185d275249b1a`；不表示候选已合入 integration。新根调用原 `PositiveCounter::deserialize`，没有复制校验器或修改生产 Rust、Charon/Aeneas。适用输入/目标/工具和模型见[接口环境](../verification/json-counter-environment.json)及[输入摘要](../verification/json-counter-inputs.sha256)；新入口加入原完整证明入口和总审计，未删除旧门槛。

[公共接口](../SandboxModel/Boundaries/JsonCounter.lean)的 12 条模型/实现/可实现性定理已在固定 Linux 环境通过真实 Lean 服务、独立内核及依赖审计，仅使用标准逻辑。真实泛型 Deserialize 保持原整数 provider 的错误传播与非零检查顺序；非零值保持，零调用原自有错误映射，模型失败/发散/外部效果保留。`CounterSafe` 要求成功值非零；有限 value reader 提供合法接受及零拒绝见证，它不是 JSON parser 实现。

冻结代码 `94dfdcd2a107f7f731bf42ca481483059709f852` 的最终完整负向复现退出 0：真实错误版先编译及原工具抽取成功，再被同一非零成功断言以退出 101 拒绝，并有同一独立 `CounterSafe` 的两个正式反例；真实服务诊断完整、非过期，内核和实际依赖审计通过。正确版 JSON 回归及原 `0 x`/`1 x` 首错回归通过。第三个干净检出的生成 Lean/清单逐字及完整规范化 LLBC 一致；新增门槛和四个原模块入口通过。

同一代码提交的原 Linux `check-function.sh` 退出 0，1053 项工具测试及工作区 Rust/文档/格式/Clippy/doctest/诊断 feature 检查通过。完整 `verify-controller.sh` 退出 1：Rust/Clippy、新叶子 12、公共 260、网络 43、授权 85、本地 153、Linux 首链 32 项审计均执行通过，全量 446 个透明函数翻译 226.700366 秒后，仅因 #337 的历史 Types/Funs/清单不一致拒绝，未进入后续旧证明总编译/审计。六项跟踪生成差异封存后恢复，未导入的类型模板另封存；前后 1376 个跟踪输入一致，233 个正式 Lean 源的指纹与最终真实服务相同。最终机器收据 SHA256 为 `ec739dcfdd24da275e7b81e8bb80b2a74098024c6081ac1f44a2b5557ef0a0c9`。原特权/opt-in 忽略项仍保留，不计为已执行；没有 KVM、云 SDK 或题集验收。**本接口局部验收完成，整套证明未通过**；#337 仅记录阻断，不迁移。

外部条件见 [A6-json-counter-leaf](formal/dependencies.md#a6-json-非零计数器叶子接口)：实际字典/表示符合性、有限格式化观察和固定工具信任；七处条件清理仅登记，未获得 Drop 证明。可观察自定义 Drop、资源持有及 panic/unwind/abort 收尾不在支持范围。完整字节入口、可变 parser/尾随检查、复合 schema、其他编号叶子及真实调用绑定仍开放，不关闭 T2/V0，也不解决 T5/T6 或历史 #337。

## V0 公共基础：指定输入的局部证明

归 #333/#101。表写入批次从 `882427adfae74ee3933244b3e5e3c983996e86a3` 展开（依赖未合入的 #362 及其前置证明批次；不含 #364 的 V4 完整状态/输入迁移）；实际生产源码、抽取根、独立模型、库模型和检查脚本以[输入摘要](../verification/foundations-inputs.sha256)为准，目标/features/cfg、生产/抽取编译器及工具/后端指纹见[环境清单](../verification/foundations-environment.json)。本记录不将起点提交等同于改动后源码；合入后核对指纹。适用目标已冻结为 `x86_64-unknown-linux-gnu`，同版工具在 Linux 重新抽取并通过本模块验收；不将它解释为 KVM 或服务已验证。

| 实际入口/责任 | 已建立的对应或性质 | 仍不覆盖 |
|---|---|---|
| `PositiveCounter::try_from/get/checked_next` | `counter_constructor_refines/valid`、`checked_counter_refines`、耗尽精确错误、数值往返及可实现初值；公共 `makeCounter` 的任意自然数成功/拒绝域闭合，真实机器域含义保持 | 服务分配、高水位/退休、窗口及缓存/事务保持；JSON 和恢复入口仍须连接，非零域不由生成的 u64 别名自动提供 |
| 中心 `RetentionLane::current`（原窗口后继查询，未改实现） | `lane_current_refines`、严格后继及合法非零域、耗尽充要条件、只依赖原高水位、可表示查询初值；覆盖全部 u64 高水位 | 高水位推进/持久恢复、窗口关闭 ACK、代际退休、位置复用许可和旧请求拒绝；查询忽略 phase/身份等字段，不是授权器 |
| `Resources::checked_add/positive/fits` | 四分量完整数学表示、溢出返回 None、正值与容量比较的双向对应 | 各服务的累计计费、减法/单位换算、峰值/物理资源及清理预算连接 |
| `ControlIdentity::validate_node/validate_response` 与实际派生相等 | 全部字段相等、按真实顺序检查版本/原节点/位置/原响应，精确成功与错误对应及成功充要条件；构造/编号范围见下方专项 | JSON 编解码往返、认证、权限、持久原操作绑定及迟到消息处理 |
| `CreateBinding::is_valid`（中心创建实际使用的纯守卫） | `create_binding_valid_refines` 覆盖全部布尔守卫与时间加法溢出；`create_plan_init_realizable` 给出实际表示输入被接受 | 模板授权、完整中心创建决定、节点选择、资源预留和派发；本守卫接受不代表有执行权限 |
| `WorkOutcome<E>::permits_new_attempt/observation` | 泛型结果分类、完整观察保留、干净明确失败才给重试提示、未知/部分失败不直接重试、两个真实抽取根对应 | 宿主证据真实性、重试额度/围栏、回执提交、Ready/Stopped/Cleaned/Released 的真实构造和发布连接 |
| 本地 `Store::get_copy`（真实有限表查询） | `table_get_copy_loop_refines`、`table_get_copy_refines`：全部合法起点的有限循环终止及首次匹配；`table_u64_query_refines` 连接实际 u64 调用根及相等字典；存在/缺失的独立语义，重复键仍返回首条 | 自有键类型需提供 `TableLawfulEq` 的具体对应，不能将其藏为外部假设；查询不授权维护、退休或窗口推进 |
| 本地 `Store::put`（真实有限表写入） | [TableWrites](../SandboxModel/Foundations/TableWrites.lean) 与 [TableWriteContracts](../SandboxModel/Foundations/TableWriteContracts.lean) 的 17 项定理：任意合法起点有限终止、首次替换/缺失追加、完整有序列表对应、目标键更新/其他键不变、键唯一性保持及自然数表示；真实 u64 写入后读取和可实现空表起点 | 泛型键相等与嵌入单射显式提供；原键存在时无需追加余量，缺失时需长度小于 `Usize.max`；物理 OOM 不属纯 Vec 模型；删除见下行，具体业务修改范围/额度和事务保持仍未证明 |
| 本地 `Store::retain`（公共筛选） | [TableRetention](../SandboxModel/Foundations/TableRetention.lean) 的 25 项定理：原记录顺序/内容及行不变量保持、长度不增长、键唯一性保持、纯谓词幂等、目标删除/其他键不变、表示投影、有状态回调的完整顺序语义，以及真实泛型函数和 u64 调用根对应 | 回调字典的正常结果与自有谓词由调用模块证明；纯筛选幂等不推广到有状态回调。重复键首条被删除后可能暴露后条，修改范围定理显式要求相应键所有行保留/删除；不证明业务退休许可、真实内存析构或 panic 后回滚 |
| `Option::is_none_or` 公共接口与真实 u64 调用根 | [OptionPredicates](../SandboxModel/Foundations/OptionPredicates.lean) 的 5 项定理：None 短路真、Some 精确单次原回调、失败传播、显式回调合同下的判断对应、实际 u64 判断对应 | None 的 true 不代表授权；V4 的创建检查、原条件/顺序和具体回调仍须实例化，不把标准库模型视为自有守卫证明 |
| 公共有限复制与缺失前置插入 | [FiniteCopy](../SandboxModel/Foundations/FiniteCopy.lean)与[FiniteInsert](../SandboxModel/Foundations/FiniteInsert.lean)的 19 项定理：按序复制、有限终止和游标不溢出、已有值匹配短路、缺失前置插入、成员/长度/去重/幂等/单射投影，以及比较/克隆失败传播；正常空表起点可实现 | 当前是独立公共算法及条件连接，不是当前 Rust `transition::insert_unique` 已对应。V4 须落实真实抽取循环体/包装连接、自有值比较/克隆和状态不变量；已有值分支不要求容量或克隆，缺失分支要求长度余量；不覆盖物理 OOM、析构效果或外部失败回滚 |
| 公共查询投影 | [TableQueryContracts](../SandboxModel/Foundations/TableQueryContracts.lean)的 5 项定理：首次匹配的注入键/值投影、真实泛型 `get_copy` 和 u64 根的表示对应、命中保持以及键身份被擦除的反例 | 自有键比较/表示和实际调用者清单仍由相应模块提供；不假定键唯一，不授权维护或变更完整 World |
| 字符串身份构造与取值 | [Identifiers](../SandboxModel/Foundations/Identifiers.lean) 与 [IdentityDomain](../SandboxModel/Foundations/IdentityDomain.lean)：八类真实 String/&str 构造和两类取值入口、原值不变、成功/拒绝域、无别名；有效有限 UTF-8 的正常返回 | JSON 字节解析、所有存储恢复调用、物理分配/OOM；`..`、`001` 仍是合法不透明身份，不能用此谓词代替路径检查或授权 |
| 中心已分配编号的纯恢复检查 | [AllocatedNumbers](../SandboxModel/Foundations/AllocatedNumbers.lean)：真实协议解析及中心包装对应；精确前缀、非零、不超过原高水位、规范十进制原文、成功无别名；零高水位拒绝 | 不改变其他本地 ID 的解析规则；完整 CenterState 恢复、前沿真实性、退休/复用/执行许可属于调用者，不能由数值通过推出 |
| 完整身份的构造输入域 | 八类身份通用构造结论与实际 PositiveCounter 构造连接到完整 bindingView，机器协议/位置字段有界；不支持版本仍由原 validate_node 拒绝 | 是值层及构造条件连接，不是实际 Serde 解码对应；可表示的未知版本/零位置允许先解码，之后按原序拒绝；所有恢复调用建立域仍待连接 |
| 公共组合框架 | 有限历史精化、内部空事件、传递性和修改范围并集；[Observations](../SandboxModel/Foundations/Observations.lean)增加观察保持、关键事件不擦除、非空初态、两层传递性与可实现数学见证；指定公共审计扩至 136 项，验收见下段 | 具体关键事件分类/完整观察与真实实现连接尚未闭合；合法正常请求见证、故障步、真实物理证据及无限执行进展不由该框架代替 |

协议部分的生产调整为私有非零计数器改用受检查的私有整数表示，以及四分量资源比较改为同顺序显式守卫；对外类型、JSON、零/耗尽拒绝和错误不变。Rust 内存布局不是冻结线协议的一部分，不能宣称二进制布局完全不变；后续新构造路径仍须保持非零不变量。资源比较与原写法边界对照、计数器 JSON 边界及工作区回归检查已执行。本地抽取适配见下段，不将等价重构的工程测试当成新版本地精化已完成。

实际抽取使用四十九个自由调用根，所有可达自有协议/中心/本地函数保留透明函数体并检查生成顺序/清单/真实定义；不使用“只有清单、没有函数”的空生成结果。脚本、自由调用根与固定根集合三者相等，防止同时删根缩减范围。循环清单允许同一个真实 Rust 函数对应主函数、循环包装及循环体，但要求身份/父函数/文件精确一致、两种循环角色齐全、每项都有真实透明定义；其他重复条目仍拒绝。LLBC 全内容摘要仅规范化 Charon `short_names` 哈希表的键值排列，拒绝重复键，不删除内容或规范化有序 AST；Lean 与翻译清单仍逐字节对照。库模型限[依赖登记](formal/dependencies.md#a6-公共基础库模型使用登记)及其身份字符串合同的二十二个标准库函数及固定默认 u64 格式/同版内置模型；它们与真实库的符合性是显式条件，不是自有适配器免责。旧批次的十五根/八函数验收只适用于下文指定旧提交，不自动背书这次扩展。

表写入最终证明输入为 `4e60dac3dc262fa9a20829df2f77a9d1d6039996`，隔离 Linux 服务器完成 V0/共享模块重新抽取一致性与全部局部依赖审计、36 项抽取正负测试、1016 项完整 Linux 工具测试及文档检查。该提交的完整入口重新执行 Rust 工作区测试/静态检查并通过，全部本地根翻译成功，随后因下述历史生成物不一致退出 1，未进入全部历史定理编译/总审计。完整基础脚本曾在 `5f67affaf22d8a1522d211e304bcd5f1ff03e1ad` 通过，包括诊断特性；两提交之间生产 Rust、Rust 测试和该脚本完全相同，工具/抽取路径修复后的受影响检查已重跑，不将早先 1014 项工具结果替代最终 1016 项。只增加两条表写入 Rust 回归，没有修改生产算法或 SDK 合同。

公共/共享抽取的输出参数现为相对实际 Cargo crate 的 `../aeneas/llbc/...`，避免旧目录绝对路径造成中间表示失配；审计保留全部字段并拒绝错误目标。试验中的项目根相对路径未落到实际目标，及四线程回归发生的 Redis 测试端口争用均未算通过；修正路径并恢复单线程后复验。直接运行工具套件曾漏掉基础脚本的 `umask 022`，权限断言按预期拒绝；按原私有权限条件重跑后最终工具测试通过。没有修改生产权限、Redis 配置或测试断言来消除这些准备问题。局部通过与整套未通过分别记录，后续 V0 自有缺口仍以下表为准。

`bash scripts/verify-foundations.sh` 已验收这个局部集合，并接入完整入口。本地 `planned_resources_closed`、退休账本/依赖/资源/工作检查拆为原顺序短路的单循环函数；保留结果写入仍先于细节清理，只缩短可变借用。初始化的两个最大编号查询改为各自表的显式遍历，空表、零和最大编号的含义不变；可选快照遍历按原顺序展开。窗口查询仍返回首个匹配项的同一引用，补偿/成功枚举检查保持原短路位置。上述改动不改变检查条件或首个拒绝，不升级工具、不删除抽取根、不扩张外部可信条件。

工程证据包括与原资源算法的 800 组对照、冻结原退休检查的多表/多故障对照、最大编号异值及拒绝优先级、快照收集顺序/去重、结果精确保留和窗口引用检查。维护入口已完成隔离翻译，全部正式本地抽取根也已成功生成；这证明抽取可行，不是维护对应定理。

完整 `scripts/verify-controller.sh` 保留全部原检查并包含下节共享状态模块门槛。完整值/运行候选在 Linux 重跑中通过 Rust 工作区测试/静态检查、公共基础和共享状态模块验收，全部正式本地根成功重新生成；随后类型、函数及翻译清单与历史产物不一致，入口以状态 1 退出，未进入其后的全部历史证明编译/总依赖审计。当前完整值表示和三类运行流程已独立迁入共享命名空间；旧完整命名空间仍依赖历史状态及处理函数定理，不因新值表示成立而自动对应。差异包含维护状态、新网络模式及新增输入/结果，不能只更新历史产物和摘要便沿用旧模型定理。该结果不是循环翻译失败，也不是完整证明通过。

默认运行时的首轮全量翻译曾因内存不足终止，尚未到一致性检查。复跑只对固定 Aeneas 二进制中的 OCaml 5.2.1 设置 `OCAMLRUNPARAM=o=20`，缩小堆空间冗余；仍保留 8 GiB 硬限、原工具版本、全部抽取根和检查。全量翻译随后完成，整套入口才实际到达上述不一致失败；不把先前资源终止写成定理反例或语义检查失败。新本地类型、库接口、独立规约和历史对应定理的整体迁移仍由 #337 闭合。当前共享抽取已提供经审查的最小库模型，不使用模板公理；这不等于历史完整命名空间及所有状态机定理已迁移。新全量产物保留在忽略的实验目录，历史目录已恢复为验收前原件，未覆盖仓库历史成果或将其用于当前 Rust 背书；**整套验收未通过**。

V0 与 V4 的完成条件按 [V0 模块范围](formal/00-foundations.md#5-开工与完成条件)分开：整套历史业务迁移留在 #337，不是 #333 的前置条件。完整入口仍执行、失败仍报告；不将独立模块通过写成整套通过。V0 的主要自有剩余项是实际身份 JSON 连接；下表同时列明已交付公共合同的邻接义务，它们不是要求 V0 重做相邻整项业务：

| V0 剩余项 | 与下游的分界 |
|---|---|
| 完整身份 JSON 实现往返、自有 schema/拒绝的翻译连接与公共构造/解析域 | 八类字符串和中心纯编号解析已有局部对应；实际 Serde 连接未闭合。完整业务构造/恢复使用公共域的义务分别归 V1/V2/V3/V4/V5/V7，不因本表纳入 V0 重做 |
| 公共数值、身份角色及窗口比较的调用闭包 | 秒/毫秒、字节/MiB、饱和剩余和现场上限已连接真实共享 helper；本批补实际计数器 `From` 与资源 `values`，不另造未使用比较器。时钟/完整期限、准入、窗口 ACK/关闭、退休推进和复用授权仍由实际 V1/V2/V3/V4/V5 入口建立 |
| 公共查询/修改范围的实际提供方实例 | 通用查询投影、表写入、删除/筛选及投影已建立，14 张表的有限提供方清单已核查；具体自有键/删除谓词、完整 World 业务更新归调用模块，尤其 V4 |
| 事件/流历史、拒绝/未知/证据接口的实际连接 | 单方向有限字节流、驻留/累计预算、合法结束及终态已有公共合同；完整动作/回执守卫已连接实际实现。V8 提供真实流/取消/结束和所有缓冲，V6 提供观察 getter/物理证据，V9 提供真实事件分类、系统组合和条件完成性 |
| 新公共入口的抽取与依赖闭包、最小 Lua/编码桥接的可行性记录 | 已实际测试锁定 Lua 前端的两条原脚本解析；不是 AST 或语义证明。实际 Lua/CAS/耐久协调归 V2，协议路径归 V3；不把未证自有代码列成外部假设 |

完整中心创建仍受字符串 Pattern 等库接口限制，归 V1；参数守卫不能代替它。上述部分包含需要相邻模块提供的实例，不因此要求先完成相邻整项任务或全部历史生命周期。

### 实际值转换闭包与 Lua 前端：指定 Linux 批次

归 #333/#101，PR #462 依赖 #456 的 `449440c5419497a424901777542e5f159b47d84e`。证明与回归输入为 `ce8a88a890316d3b5fe3b8140821086c6d39d625`；`c4627847e66fd309f8179a95a8ab31c3da5af63c` 只修正网络批次对三份新基础生成物的摘要，Rust、Lean、生成物和测试不变，并在该提交重跑原完整证明门槛。本批不修改生产 Rust、公共接口或任何守卫/拒绝结果，只新增实际 `From<PositiveCounter> for u64` 与 `Resources::values` 的两个委托根及六项对应定理。旧 54 根保留，新集合为 56 根；23 个已审查库接口及其模型、固定工具和检查判据不变。生成文件由 a6000 的原脚本实际重新产生，没有手工改写。

[Counter](../SandboxModel/Foundations/Counter.lean) 连接实际 `From` 的原值保持、合法构造往返及任意构造成功后的值与域；不以此前 `get` 对应代替实际转换。精确转换定理不增加非零检查，合法域由原构造入口提供。[Resources](../SandboxModel/Foundations/Resources.lean) 连接实际四元素数组、数学投影及表示下的长度和顺序，完整保留实例、CPU、内存、磁盘，包括零和最大值，不替代 V1 的比率、选点和准入。

两模块在重新构建真实生成依赖后，通过同一 Lean 服务的正式文件完整诊断，均无错误、`stale=false`、`partial=false`；资源长度证明先读取真实目标并隔离试验，再采用成功步骤。六项已登记到原审计入口，公共审计合计 250 项。独立只读复核确认真实透明体、原序、全机器域和零值行为，以及旧根/依赖保留；不是用复核或定理数量替代实际验收。

在 a6000 执行原 `verify-controller.sh`：Rust 工作区/静态检查、V0 56 根/23 库接口的真实再生一致性与 250 项依赖审计通过；网络 43、授权 85、本地 153、Linux 首链 32 项审计及再生检查通过。完整本地核心的 446 个透明函数翻译 162.303253 秒完成，无 OOM；随后历史 Types/Funs/translation 原一致性判据拒绝，退出 1，未进入全部历史定理编译/总审计。新候选摘要与前置批次相同，候选留存、历史原件及中间表示逐文件恢复。**本批列明局部集合通过，整套验收未通过；历史业务迁移仍归 #337。** 原 `verify-first-chain.sh` 另在同一提交通过 53 项模型/条件合同审计，不代表实际持久微步骤、物理证据或最终完成性。

宁独立目录在 `ce8a88a` 执行原完整 `check-function.sh` 退出 0：1042 项工具测试无跳过，Rust 工作区/文档测试/诊断特性、格式、静态和 32 个 Markdown/本地链接/索引检查通过。`c462784` 只修改一份摘要清单，没有 Rust、Lean、测试、生成物或基础入口差异；相关证明闭包已在该提交复验，不把它写成宁重新执行了一遍全量回归。20 项抽取审计正负测试包括新根/透明体及成对删根拒绝。未运行 KVM、云 SDK 或题集。

失败证据保留：首轮宁调用漏传必需的固定 Redis 路径，四项夹具检查拒绝；显式提供同一 7.2.16 二进制后重跑原脚本成功，没有改测试或依赖。首轮完整证明门槛在本批遗漏的三份网络传递摘要处拒绝，修正后重跑全部原入口才到达上述历史业务失配，不把两类失败混为一谈。

最小 Lua 前端可行性已进一步实测：复用 ning 既有 Redis 7.2.16 随带的 Lua 5.1.5 `luac`，仅在 a6000 独立 worker 对真实 READ/CAS 做两个单输入 `-p -l -l`。脚本文件摘要 `65cacadea99e436e6fb6cce1cda5c503f8bb407eaf8f3c0dcd09e7afa12a4cf3` 与本候选相同；两次退出 0，无 stderr、嵌套函数或向后跳转，产生 117/150 条指令，CAS 保留唯一 HSET 前的全部检查及完整四字段参数。未执行脚本、EVAL、启动 Redis 或安装/重编译工具，不把探针加入正式 PATH、模型或可信清单。公开移交见 [#335](https://github.com/VeriInfra-Lab/sandproof/issues/335#issuecomment-6174378105)。

这解除“没有任何可执行前端”的可行性障碍，不生成 AST、`lua_read_refines/lua_cas_refines` 或完整源码语义证明。若后续使用真实 Proto/字节码而非原计划的 AST，须独立审查输入导入、编译器/语义对应和等强的逐语句连接，不能静默改变计划或新增正确性假设。真实参数、原语错误、完整字节 CAS、耐久确认及未知恢复仍由 V2 接通。

JSON 未闭合，#333 保持开放。固定工具的静态引用问题同时涉及类型/值表示一致性及尚未支持的生命周期路径；仅移除检查或全面擦除 static 不是正确修复。真实字符串叶子抽取成功也不证明完整 Serde visitor；该批隔离 Serde 探针的默认格式化模型丢文本，不能据此背书错误全文。下节另连接实际内部错误文本，不等于已连接 Serde 错误。正式工具未改，不把整个自有 schema、codec 或 Lua 移入外部假设。构造/恢复的实际使用按 V1/V2/V3/V4/V5/V7 分工，原允许零的本地前沿解析不能替为非零编号解析。

### JSON 与异步实际调用：隔离可行性批次

归 #333/#101，从 `7b1e1e2c4411033d06910daf17e49f9bd197d682` 的独立任务分支展开；协议生产输入仍为 `a62d50832b89aa7bcac5443aabb893910a8b82fd`。没有修改 Aeneas、Charon、依赖版本、正式生成物或验收判据，没有迁移 #337/V4 的历史业务证明。以下是指定输入的隔离局部结果，**未计入正式 V0 的 260 项审计，不表示完整 JSON、异步精化或 V0 模块完成**。

JSON 实验保留实际 `PositiveCounter::deserialize/try_from`、闭包、`InternalError::fmt`、Serde u64 解码和整数 visitor，进一步纳入真实 `serde_json::Error::custom`。11 项局部定理连接非零值保持、原解码拒绝/外层失败/发散、零分支原 custom 调用、完整字节格式化，以及真实 custom 对原错误文字的处理。手写库合同只提供默认 String-backed 格式化、ToString，以及 `make_error` 在没有 ` at line ` 字节标记时的消息和 Data/0/0 观察；含位置后缀的消息未支持，不能套用此结论。泛型解码与具体 custom 定理分别成立，实际可变 deserializer 字典、结束检查和两者的组合仍未闭合；实际解析行列、完整字段访问和解析器状态不在已证范围。库合同的源码符合性仍是显式外部条件，不是这些库内部实现已证明。

完整身份解码仍在真实派生字段访问的 `PtrMetadata`、静态引用/参数表示处被固定工具拒绝；具体字节入口还保留上一批泛型返回与可变解析器写回形状不一致的缺口。已有部分专门化未通过；新增完整专门化虽由 Charon 生成中间表示，但固定 Aeneas 拒绝非 aeneas preset 输入，因此未采用，也未改写中间表示元数据绕过拒绝。通用不透明 JSON 库入口可以保留自有回调参数，但仅生成这样的签名不等于合同已满足，未将整个身份 codec 作为外部原语。没有用先完成整个 u64 文本解析再检查非零的适配替换原流程：`0 x` 的原数据错误优先于尾随检查，而 `1 x` 保持尾随语法错误；新增回归固定原文字、类别和行列。

异步实验仅把实际 API 中 `.await` 后的两段原错误表达式提为私有 `finish_blocking/finish_kill_wait`，不移动准入、spawn、await、timeout 或 worker 持有的许可。最终生产文件 SHA-256 为 `d56ac103f2dc588a2ffcfeab86b2e0f6793f6684cebfdc1ef6359b007c8b5605`。真实函数重新抽取后，6 项生成语义定理覆盖成功原值、全部业务错误、JoinError/Elapsed 到 OutcomeUnknown 的分类。**这些是返回值层的局部定理**：固定后端的析构擦除尚未获得真实 JoinError/panic payload 的语义连接，不能由此推出所有 Rust 输入必然返回、取消会终止 worker 或许可已释放；真实 async 壳仍被 Charon 的协程限制阻断。公开接口、检查顺序及原拒绝结果不变；未增加可信的自有异步执行器假设。

全部 17 项隔离定理在 a6000 编译并经过原依赖审计，仅依赖 Lean 标准逻辑，不再含前一轮 JSON 试验中的 Formatter 类型公理；生成文件未手改。证明、模型、真实 Lean 服务指纹及原始诊断保存在私有忽略产物中。工程回归与完整验收分别记录，不能把以上局部结果计为整套通过。

ning 的独立候选在上述最终 API 源码及协议测试摘要 `34ae829e7616e0025d98a1d1fcca61dbfea19c429d44de146efbe3fd60e9867c` 上执行原完整 `check-function.sh` 退出 0：1042 项工具测试无跳过，真实固定 Redis、文档、格式、工作区全部目标/文档测试及诊断特性的原检查通过；日志 SHA-256 为 `6a3a1f079c924ff70153d4ddc6a5a1d86db6325b6dd5b8c8bbea3332dfbcc632`。协议编解码 8 项、API 的 6 项生命周期等待回归均包含在该入口中；既有需特权 Rust 忽略项未运行，不算 KVM、题集、云 SDK 或系统证明。

新增首错回归属于原冻结输入，已更新基础清单及网络/首链的传递摘要，未删除输入或改判据。a6000 原完整 `verify-controller.sh` 重新通过 Rust 检查、V0 57 根/260 项、网络 43、授权 85、本地 153 和 Linux 首链 32 项审计及实际再生一致性。全核心 446 个透明函数翻译 164.573124 秒完成，无 OOM；随后原历史 Types/Funs/translation 一致性拒绝，退出 1，未进入所有历史业务定理编译/总审计。日志 SHA-256 为 `54b92db6197a26120c4e67627f1ec23bc5c59f3a119791ccedff729a2bcd695e`。候选保留在私有诊断副本，未覆盖仓库历史产物；**整套验收未通过，历史迁移仍归 #337/V4，不作为 V0 关单条件；V0 自身的完整 JSON 连接仍另有缺口。**

原 `verify-first-chain.sh` 另行通过公共再生检查及 53 项模型/条件连接审计，不等于实际完整生命周期已证。ning 对最后的清单变更复跑 58 项相关抽取正负测试通过，覆盖文档单独复验通过；没有把只改清单后的组合写成重新运行一次完整 Rust 回归。17 项隔离定理的最后服务检查均完整、非过期、无错误，关键根独立依赖核对仅有标准逻辑；实际库模型和未编辑生成物的摘要另行复核一致。

### JSON 解析状态与字段回调：隔离连接试验

归 #333/#101，基于 `ed68165` 的独立候选，协议生产源码仍为上述 `a62d508`，产品 Rust、接口、拒绝顺序、固定工具及正式证明清单均未改。a6000 的最小入口直接调用真实 `serde_json::Deserializer::end`；实际解析器结构、`peek/eat_char`、空白扫描和结束检查均透明抽取，未手写同名替代算法。11 项隔离定理通过同一 Lean 服务、编译及原依赖审计，只依赖标准逻辑，未计入正式 V0 审计。

这些定理证明：在底层 `Read::peek/discard` 给出的**有限读取轨迹**条件下，真实扫描将后状态逐步传递；遇非空白不调用 discard，读取错误保留其后状态和原错误，结束检查使用扫描后的状态与位置。轨迹前提不含待证 scan/end 结论；但真实 `SliceRead` 的 bytes/index 表示、轨迹存在性、任意读取器终止和外层失败/发散仍未实例化。唯一新增函数合同是 `peek_error` 的 TrailingCharacters 分支，保存错误码文本、类别和位置观察，不声称完整 Display 或错误来源链已证，也不把合同未支持分支解释成产品 panic。

完整 JSON 连接仍未完成：具体计数器薄入口按原构造→实际 Deserialize→仅成功后 end 的顺序运行，固定 Charon 的两种部分专门化均成功，但 Aeneas 在第三方 `serde_json` Deserializer 专门化实现的名称生成处拒绝。真实 NodeIdentity 序列访问器缩面则遇到 Deserialize/DeserializeSeed/Deserializer/Visitor/SeqAccess 五个互递归接口；一次退出 0 的初探实际裁掉了目标字段函数，明确不计为有效翻译。静态字段表、字段字节匹配及完整编码仍另有未解限制；没有把整个 codec 或自有字段规则移入外部假设。

字段模型须分别记录错误优先级和游标推进：重复/未知字段先拒绝而不读其值，缺失字段按声明顺序检查；但真实 `deserialize_struct` 在访问器报错后仍可能执行对象/序列收尾，并保留访问器错误优先、补充位置。不得把它建模为报错后解析器完全不再变化。ning 的原协议编解码 8 项回归及新增隔离入口/游标回归通过；这不是完整字段证明、KVM 或系统验收。

本批重跑原 `verify-controller.sh`，Rust 检查、V0 57 根/260 项及相邻网络/授权/本地/Linux 首链的再生和局部审计通过；446 个透明函数翻译 165.986935 秒完成，随后仍在原历史生成物一致性处退出 1，未进入全部历史业务定理编译。运行前只在私有诊断副本中恢复原门槛输入，防止上次已再生产物使检查假通过；仓库历史文件和工具未改。**局部试验通过，完整 JSON 与整套验收均未通过**；#337/V4 历史迁移和 V0 JSON 缺口分别保留。

### 内部错误完整文本与失败：指定 Linux 批次

归 #333/#101，依赖 #462 的 `2ad30c94ff6fae7517e725898710027d162d11ea`；本批证明/回归输入冻结为 `a62d50832b89aa7bcac5443aabb893910a8b82fd`。生产 Rust、公共接口、守卫顺序与拒绝结果未改变。只新增真实 `InternalError::to_string()` 委托根，可达原 `InternalError::fmt` 保持透明；旧 56 根全部保留，新集合为 57 根，23 个非内置外部接口不变。工具、编译器、Lean 和依赖版本未变，生成物由原脚本实际产生，没有手工编辑。

[ByteArray](../SandboxModel/Foundations/ByteArray.lean) 的两项引理证明实际 Lean 字节循环/列表的完整内容与次序，包括任意游标和累积量，不是三个字面量的特例。[ErrorMessages](../SandboxModel/Foundations/ErrorMessages.lean) 的八项定理连接实际三类 Display 及默认 ToString、精确错误文字/类别无混淆、计数器零值拒绝与独立格式化结果，以及外层回调失败/内层格式错误两种分支；最后一项对应原 `expect` 的 panic 类别，不证明 panic 消息或恢复。

标准库写入模型限默认 String-backed、有效有限 UTF-8、可表示缓冲与正常分配域；任意 writer、格式选项、物理 OOM/abort 均不在范围内。实际 writer 与 Formatter 委托在原冻结 string.rs/fmt/mod.rs 内，新增两版 result.rs 摘要连接 `expect` 原错误分支；正负审计检查两版登记遗漏。全部传递摘要同时更新，没有放宽原审核器。

固定后端的字面量默认本机求值证明曾给新实际 fmt/root 带入三个自定义本机构造公理，原审计不接受。局部 `toStr` 包装仍委托原转换，保持同一文本、字节和 U32 长度前提，仅显式提供普通内核范围证明；实际依赖重建后 fmt/root 只余标准逻辑。它不是工具补丁、生成文本改写或审计白名单豁免。

十项定理已导入原公共入口与 Audit，合计 260 项；正式文件在 a6000 同一 Lean 服务完整诊断成功，`stale=false`、`partial=false`，无错误及失败依赖。两个独立只读复核未发现范围/失败语义偏差。

在同一冻结提交执行原 `verify-controller.sh`：Rust 工作区/静态检查、本批 V0 的 57 根/23 库接口真实再生一致性与 260 项依赖审计通过；网络 43、授权 85、本地 153、Linux 首链 32 项及各自再生检查通过。完整本地核心的 446 个透明函数翻译 164.257892 秒完成，无 OOM，随后原历史 Types/Funs/translation 一致性检查拒绝，整套退出 1，未进入全部历史定理编译/总审计。候选摘要与前批一致；候选及原件保留，历史 Lean/清单和中间表示已逐文件恢复。日志摘要为 `c6cb379ed08b6ae796c87c0e93210776699e4cabc42543c5d089c157b319472b`。**本批列明局部集合通过，整套验收未通过；历史业务迁移仍归 #337/V4。**

独立原 `verify-first-chain.sh` 在同一提交退出 0，通过传递摘要、公共再生检查及 53 项模型/条件合同依赖审计，不代替真实持久微步骤、物理证据或最终完成性。

ning 独立目录在同一冻结提交执行原完整 `check-function.sh` 退出 0：1042 项工具测试无跳过、工作区/文档测试/诊断特性、格式/静态及 32 个 Markdown 本地链接/索引检查通过。日志摘要为 `303d6239ec03bc73e54d12dc4de641d39a72e96e42d78206252bf0237f503a77`，20 项抽取正负审计含新根、实际 fmt 透明体和两版 result.rs 漏登记拒绝。没有运行 KVM、云 SDK 或题集。

本批是实际错误文本子链，不是 Serde custom、Expected、具体 JSON Error/行列、解析、复合 schema 或整个 V0 已闭合。JSON 实现连接仍开放；固定工具诊断及剩余连接跟踪 #333，历史业务生成物迁移仍归 #337/V4。没有以缩窄合法输入或变更泛型接口消除翻译障碍。

### 公共数值、回执与流历史：指定 Linux 批次

归 #333/#101，PR #456 堆叠于 #443，证明输入冻结为 `02c786eb107f1c62812373e5dc89181de1d0d364`。生产源码、模型和真实生成物由更新后的输入清单绑定；本批新增 78 项，公共审计合计 244 项。各独立工作区结果已经在这一组合提交重新检查，不以定理数量表示系统覆盖率。

| 入口/合同 | 本批交付 | 保留的边界 |
|---|---|---|
| `numeric::seconds_to_milliseconds`、`bytes_to_mebibytes`、`available_resource` | [Numeric](../SandboxModel/Foundations/Numeric.lean) 的 22 项：真实 helper 精化、秒转换溢出充要条件、向上取整和 U32 拒绝域、除后加一无溢出/有界 cast 保值、剩余两界/耗尽/账本界、零输入见证 | 只委托调用者原表达式，原检查/观察顺序和错误映射保留；不证明完整时钟、期限、准入、物理观察或清理授权 |
| 公共 `ActionIdentity`、`ResourceRef`、`WorkReceipt::validate_for` | [Receipts](../SandboxModel/Foundations/Receipts.lean) 的 30 项：完整字段与输入域、原值构造/相等/单射、版本→全文身份→可选观察资源首拒绝、真实方法精化/成功充要条件、无观察短路、失败传播及合法 attempt 0 | `ResourceGetterExact` 是具体观察提供方的自有义务，不归 A6；检查回执接受不等于工作成功、物理事实、耐久确认或取得重试许可 |
| 规范编号的一般接受完备性 | [NumberCompleteness](../SandboxModel/Foundations/NumberCompleteness.lean) 的 7 项：任意可表示非零编号、正确 UTF-8 前缀及足够高水位下，真实协议/中心入口必接受；连接实际十进制渲染/解析，不靠有限样例 | 不能据此相信高水位来源、完整恢复状态、位置退休或授权；库符合性仍受原 A6 条件限制 |
| 单方向有限字节流 | [StreamHistory](../SandboxModel/Foundations/StreamHistory.lean) 的 19 项：原字节内容/次序、Read/Written/pending 分解、短写、预算、一步/有限历史保持、合法结束和终态、可实现有限完成轨迹 | 这是公共模型；有效结束、I/O 确认、绑定、所有缓冲及机器计数由 V8 连接。存在完成轨迹不是任意公平运行必完成，不证明真实异步中继 |

V0 抽取增加五个薄根，原四十九根不删除。新增 `Option::is_some_and` 的 None/Some/单次 FnOnce/失败合同显式登记在 A6，全部自有动作、回执、比较与闭包仍透明。外部模型集合和原审计判据不放宽为任意调用。

在 a6000 私有工作区执行原 `verify-controller.sh`：Rust 工作区测试/静态检查通过；V0 54 个真实根、23 个已登记库接口重新抽取一致，244 项正式编译与依赖审计通过；网络 43、授权 85、本地 153、Linux 首链 32 项审计及各自再生检查通过。完整本地根翻译 169.592239 秒完成，无 OOM，随后原历史 Types/Funs/translation 一致性检查拒绝，整套退出 1，未进入全部历史证明编译/总审计；新候选摘要与前置批次相同。候选和原历史产物均保留，历史目录已逐字节恢复，没有换产物造通过。**V0 本批列明集合通过，整套验收未通过；历史业务迁移仍归 #337。**

同一冻结提交另执行原 `verify-first-chain.sh`，公共再生、模型编译与 53 项审计通过，只是模型与条件连接。四个新模块在重新构建导入后通过真实 Lean 服务完整诊断，均 `stale=false`、`partial=false`、无错误；固定工具、库、编译器和后端版本未变。

宁私有工作区的原完整 `check-function.sh` 退出 0：1039 项工具测试无跳过，工作区/诊断特性/文档/格式/静态检查通过。627 个 Rust 文件（包括新增文件）逐项 Git blob 与冻结提交完全相同；后续文档、17 项抽取审计正负测试和格式在冻结提交另行通过。新增 JSON 对照 3、数值边界 3、回执顺序 4 条定向回归均通过。上述检查不运行 KVM、云 SDK 或题集，不将固定 Linux 工作区结果推广到未声明平台。

八类字符串与非零计数器的解码只将 Serde 原 primitive→TryFrom→custom 错误流程显式化，没有改复合 schema、泛型接口、字段、Serialize、版本或拒绝序。宁上的旧 derive 对照验证 from_slice/from_str/from_value、对象/数组、未知/重复/缺失、非法字符串/数值及尾部错误，比较成功字节、完整错误文本/类别/行列；对照不等于一般性形式化等价。

**JSON 实际编解码仍未闭合。** 实际叶子解码已能抽取，但完整 schema 仍遇到字段字节模式的 `PtrMetadata`、Expected 动态转换和静态字符串参数的类型一致性限制。真实泛型编码、部分单态化及所有权探针仍未完成实现连接；不把自有 schema 或 `from_slice/to_vec<ControlIdentity>` 设成不透明目标，不改固定工具或生成物造通过。

最小 Lua 前端试验在指定 a6000 私有环境真实失败：没有现成 Lua AST 前端，导入退出 78，未产生 AST；源码审查和文本摘要不充当脚本证明。已固定实际 READ/CAS 的语法子集、原语、原拒绝顺序及后续 Rust revision 检查责任；前端/AST/逐语句语义和实际 Lua/CAS 连接仍由 #333/#335 的相邻批次解决。没有下载工具或扩大可信条件。

14 张有限表的提供方核查：沙箱、操作键、工作尝试已有实际相等连接及全部键/值克隆；VM/快照/网络槽与逻辑工作仍需显式 `TableLawfulEq` 实例，`HostSlotId` 还需实际相等抽取根。借用查询已有 `Access.store_get_refines`，不重复制造公共算法。V4 须连接这些自有字典及真实调用/状态不变量；不能因为底层为 u64 就宣布整表已证明，也不把它们归入 A6。

### 身份构造域与规范编号：指定 Linux 批次

归 #333/#101，PR #443 堆叠于 #435 的 `60a2dd2b617d1b66a28b5033e232720fe482bc35`，组合证明输入冻结为 `f3507f2ebae30e916db0dbe8cfd3012c74dcdf3e`。本批新增 30 项审计定理（字符串 16、规范编号 8、取值/完整身份输入域 6），公共审计共 166 项；定理数量不是全系统覆盖率。实际入口、模型、外部合同和源码摘要以四份输入清单为准，不沿用前批十五根结果背书新根。

中心恢复的纯 `number` 委托协议规范编号 helper；以 `split_at_checked` 后的精确前缀比较替代 `strip_prefix`，仍按原 parse-u64→非零→原高水位→规范原文比较的顺序返回原 Some/None。回归保留原函数作为测试对照，包含 Unicode 切点、前缀错误、零/前导零/正号、边界和溢出。对照样例不是一般性等价证明；新函数实际精化及其成功域/无别名已由指定生成语义定理提供。未改变其他恢复守卫、完整身份字段、Serde 注解或错误枚举。

独立 Linux 证明环境执行原 `verify-controller.sh`：Rust 工作区回归及静态检查通过；V0 49 个真实根/22 个已登记不透明库函数模型重新抽取一致，166 项编译及依赖审计通过；相邻网络 43、授权 85、本地 153、Linux 首链 32 项审计与相应再生检查也通过。完整本地核心翻译 176.416915 秒完成、无 OOM，随后历史 Types/Funs/translation 一致性检查拒绝，退出 1，未进入全部历史定理编译/总审计。新候选保留在忽略目录，原历史生成物已恢复；候选摘要与前置 #435 相同，不用新产物覆盖旧成果造绿灯。**本批列明局部集合通过，整套验收仍未通过。**

同一提交另执行原 `verify-first-chain.sh`，公共再生、模型编译与 53 项审计通过；这是模型及条件连接，不是实际耐久提交、宿主证据或最终完成性闭合。三份新增正式模块经同一真实 Lean 服务检查，均 `stale=false`、`partial=false`，无错误。工具/后端/编译器版本未改变，未清理 `.lake`，不在开发机执行重型证明。

独立 Linux 测试环境执行原完整 `check-function.sh` 退出 0：1036 项工具测试无跳过，文档/格式/静态检查、工作区/文档测试及诊断特性回归通过。六条新增 Rust 测试覆盖八类构造取值、原编号解析和真实 JSON 完整字段/大整数往返、缺失/未知/重复字段、非法字符串/越界输入及原版本→节点→位置拒绝顺序；可表示但不支持的版本与零位置不是解码错误。需 root 的既有忽略项未运行，不算 KVM、SDK 云服务或题集证据。迁移后的仓库 Git 读写与 PR 创建已实测，普通 CI 仍需独立确认执行器授权，不以本地回归代替其状态。

**JSON 实现的形式化往返尚未完成。** 固定 Serde 1.0.228/serde_json 1.0.142 的真实入口预检中，泛型解码在关联方法函数类型转换处失败，编码在参数表示检查处失败；保留实际自有 visitor 的单态试验又遇到库错误闭包抽取、库解析循环提前返回及 Serializer 可变类型限制。只让顶层 `from_slice/to_vec<ControlIdentity>` 成为不透明调用虽可翻译，却省掉全部自有字段处理，已拒绝作为证明结果或正式库合同。试验诊断保留在忽略目录，未导入正式库。工程往返成功不能填补这一实现连接；完整恢复入口建立域、数值减法/单位转换及其余公共接口仍开放。因此 V0 整模块未完成，#333 不关闭；历史业务迁移仍由 #337 跟踪。

### 公共域、查询与可观察接口：指定 Linux 批次

从已合入的 integration `8c03525f5671c70d0a3ff5b10a40bb18b0817deb` 展开，冻结证明输入 `3950f15f02a4e6fa15b9abbdc6e5b5704b55d2a3`。新增 18 项审计定理：计数器域 5、查询投影 5、可观察组合 8；原真实 Rust 构造器对应重新检查，全部生产 Rust、生成语义、抽取根、工具、依赖和原验收脚本不变。本批模型有意修正任意自然数构造域，但不改变机器可表示输入的含义或服务拒绝顺序。

独立 Linux 证明环境完成同一真实 Lean 服务的目标读取/隔离试验及三个正式模块诊断，均非过期、完整且无错误。`verify-foundations.sh` 通过真实冻结输入、15 根/8 库模型、重新抽取一致性、编译和 136 项依赖审计。原 `verify-controller.sh` 实际通过 Rust 回归/静态检查、公共 136/网络 43/授权 85/本地 153/Linux 回执 32 项审计；全部控制核心翻译 163.049420 秒完成，无 OOM，随后历史 `Types/Funs/translation.json` 一致性拒绝、退出 1，未进入全部历史定理编译/总审计。失败和新生成候选保留，正式历史产物恢复，不造绿灯。首链模型原入口另重跑通过，含 V0 再生及 53 项模型审计。

独立测试环境的原 `check-function.sh` 完整通过：1034 项工具测试无跳过，Rust 各检查模式、静态检查及 32 个 Markdown/本地链接/索引通过；既有需 root 的 Rust 忽略项未运行，不算 KVM 证据。冻结批次普通四项 CI 通过。本批不含宿主部署、云 SDK 或题集实跑。

`makeCounter (2^64)` 现在明确拒绝，合法成功域恰为 `0 < n ≤ 2^64 - 1`。这不证明 JSON 拒绝所有非法值；已有完整字段相等也不证明编码往返、自有字符串解析无别名或恢复入口全部合法。结果类别分开只处理断言与未知，不认证物理 `Stopped/Cleaned`。新观察合同只在给定正确分类/观察的条件下保持内容，不允许把自有分类和连接当作外部前提，也不从有限历史推出最终完成性。数学 Unit 见证只证明接口可实现，不是正常产品请求已被接受。

因此本批列明的 V0 集合通过，V0 整模块及整套验收仍未完成；#333 保持开放，历史业务迁移继续归 #337。各自冻结版本的局部结果不冒充与 V4 在途批次的组合结果。

### 公共筛选与可选值批次：独立 Linux 验收

从 `9f2a37d702897156f53503795f6267066a6a6014` 展开，实际证明及全量基础检查候选为 `674daae5047dc98281c8ec3074bfaef0d5c01a47`，新增 30 项定理，公共审计总计 99 项。生产改动只有 `Store::retain` 可见性和回调的 `move` 捕获：原 Vec 筛选算法、检查条件、顺序和拒绝不变；原闭包的可变借用复制使固定 Aeneas 拒绝，最小调整和回归后重新抽取通过。原生命周期及维护行为不由本批迁移或背书。

V0 的可写 Lean、Rust 与下载缓存已独立，实际编译产物 inode 与旧共享缓存不同；工具二进制仅只读复用，版本未改，未清理 `.lake` 或修改 V4 工作区。重型验收用两窗口共同锁错峰。复制后新缓存父目录的组写权限曾被安全助手身份检查拒绝；保留失败日志，只收紧本窗口缓存权限后，同提交完整 Linux 基础检查通过：1018 项工具测试无跳过，Rust 工作区/静态检查/文档/诊断特性通过。Rust 中声明需 root 的专用宿主审计用例未执行，不将它们或 KVM/SDK 云服务/题集算作已验收。

`verify-foundations.sh` 验收 15 个真实根、8 个明确标准库模型、两版库源码摘要、生成一致性及 99 项定理；共享状态模块重新抽取与局部审计也通过。真实 Lean 服务的目标读取、隔离试验和正式文件完整诊断通过，结果不含过期/partial 背书；标准库符合模型仍是显式合同，用户回调/键/业务删留条件必须由调用模块实例化。

完整 `verify-controller.sh` 已在同一候选实际执行：Rust、V0 与共享模块检查通过，全部本地核心翻译完成（132.893 秒），随后历史 Types/Funs/翻译清单不一致，退出 1；**整套验收未通过**，没有进入全部历史定理编译/总审计。新全量产物另存诊断目录，历史产物逐文件恢复；不以替换历史产物和摘要消除 V4/#337 的重新对应义务。最终记录提交只澄清合同/补录证据，Rust、生成语义及证明体保持该候选不变。

有界前置复制和首次匹配循环合同见上表；它们不改变上方 V0 剩余义务，也不把 V4 主链路或全部历史迁移加入 V0 完成条件。

### 有界复制与缺失插入批次：指定提交验收

从 `1bbd3ad7c7d0483841f229023cd88dbc3db462e4` 展开，冻结证明输入为 `debcc227b1641e44e01e4f3b3c1449bc9db76b55`（#407）。新增 19 项公共定理，公共审计合计 118 项。本批只新增独立算法合同、连接接口、审计和文档/摘要；生产 Rust、真实抽取根、标准库模型、工具版本与原验收脚本均未改。V4 仍须将真实生成循环体和包装接入，不能将本批算法定理说成当前业务函数已完成精化。

固定 Linux 服务器的独立 V0 工作区通过同一 Lean 服务完成目标读取、隔离试验和两个正式文件完整诊断（非过期、非部分、无错误）；模块编译与 118 项标准逻辑依赖审计通过。隔离调用试验落实真实 u64 比较/克隆字典、故意失败克隆下的重复值短路、保留旧重复项的缺失插入、正常空表起点、原克隆错误传播及连接接口；试验不是额外的 Rust 实现对应声明。

同一冻结提交的完整 Linux `check-function.sh` 返回 0：1018 项工具测试无跳过，32 个 Markdown 文档检查，以及 Rust 格式/静态/工作区/文档/诊断特性检查通过。既有声明需 root 的 Rust 宿主审计忽略项仍未执行；KVM、云 SDK 和题集不适用，不算作本批通过。

未削弱的完整 `verify-controller.sh` 实际执行：Rust 回归与静态检查、15 个公共根/8 个标准库模型的重新抽取一致性及 118 项公共审计、共享状态模块的重新抽取和局部审计均通过；全部正式本地根成功翻译（138.303 秒），随后历史 Types/Funs/翻译清单失配，退出 1，**整套验收未通过**，未进入后续全部历史定理编译/总审计。新全量产物与上一公共批次候选摘要相同，另存诊断目录，历史产物逐文件恢复；失败仍归 V4/#337 的版本对应，不扩张 V0 完成条件。审查期间 #407 源码保持冻结，本段验收补录独立提交，未改变该批证明输入。

## V4 网络策略与授权：指定实现的局部对应

归 #337/#101，依赖 #349/#355 公共基础与 IPv4 候选；网络授权批次从 `3e7e4ec8bba322df8ef86b6c991cb0e2373c0b94` 展开，本地组件迁移从 `b7cfc9dc45712223f1dbc9da320481e50890fa9b` 展开，完整值与运行入口批次从 `882427adfae74ee3933244b3e5e3c983996e86a3` 展开。实际源码/模型/脚本以[共享状态抽取输入摘要](../verification/network-inputs.sha256)为准，不把起点提交当作改动后源码。固定工具、两版 Rust 编译器/标准库源码及 `x86_64-unknown-linux-gnu` 目标见[环境清单](../verification/network-environment.json)。环境迁移保持 Lean、Charon、Aeneas 和依赖版本不变，重新核对 Linux 二进制与实际配置摘要。前批 Rust 将原私有目标判断移入生产 `NetworkPolicy::destination_allowed`，三个网段名单查询改为同顺序显式遍历；本地位置批次还对 `reduce_slot` 作下节说明的等价抽取适配。完整值批次不修改生产 Rust。

| 实际入口 | 对应和性质 | 前提及未覆盖项 |
|---|---|---|
| `Ipv4Cidr::contains` / `cidr_contains` 抽取根 | 位掩码对应独立的自然数除余网段定义；全部 U32/U8 输入含 `/0`、`/32`、超长前缀与非规范网络号，不假设输入已经规范化 | 同版 Aeneas 标量/位运算模型与翻译工具条件；不证明 SDK 的解析/排序/去重 |
| 自有 `ipv4_list_contains` / `platform_protected_ipv4` | 任意起点的有限循环终止、名单存在匹配、索引递增无溢出；实际 15 项平台常量准确对应独立名单；`protected_ipv4` 真正根连接 | 固定表不包含变化的宿主本地地址；不是实际包过滤或规则安装证明 |
| `NetworkPolicy::destination_allowed` / `destination_allowed` 抽取根 | 三模式、严格解析错误和非单地址拒绝全部对应；IPv4 平台禁止先于允许、允许先于拒绝、未命中使用默认值；IPv4 不偷偷增加协议/端口限制，旧静态模式匹配全部目标字段 | `StringRepresentable` 和 [A6-network-stdlib](formal/dependencies.md#a6-网络地址库模型使用登记)；平台禁止定理只适用于 IPv4 模式，不推广到旧静态名单 |
| `parse_ipv4_bits` 库调用根 | 实际包装对应经审查的字节解析模型；正常解析错误是外层正常返回中的内层错误；段成功满足长度/数字/前导零/数值条件 | 对两版真实标准库的符合性是显式外部条件，源审查与测试不代替标准库证明；单构造错误只覆盖 `Err _` 的观察 |
| 公开输入见证 | 构造有效字符串及字节/可表示证明，实际生成根对公开 IPv4 的默认允许策略返回正常真值，说明输入条件不为空 | 不表示实际公网连通，更不表示当前实例已有网络资格 |
| `allows_network` / `authorized` / `Store::get` | 网络资格恰为有效 Running 权限中的沙箱、代数、虚拟机、网络位置和完整策略与输入/记录一致；入口的租户归属或系统身份对应；真实借用查询连接 V0 首次匹配循环 | 本守卫不读取 phase、terminal、epoch、worker_fences 或当前资源指针；更强撤销/围栏性质须由可达状态与调用者身份绑定证明提供，不假定这些字段已被检查 |
| `handle_ingress` / `ingress` 抽取根 | 查无记录、归属拒绝、网络资格拒绝与成功事件按真实顺序对应独立只读规约；成功后的完整 World 等于输入 | 仅真实入口处理函数；不覆盖 HTTP 鉴权、统一访问派发或租户密钥认证 |
| `handle_egress` / `egress` 抽取根 | 查无记录、网络资格拒绝、目标拒绝与成功事件对应；IPv4 平台禁止连接到真实 `DestinationDenied`；完整 World 不变，因此保持输入中已成立的任意不变量 | 真实出口没有 Actor/租户归属参数，不能声称它自行执行租户鉴权；地址计算仍要求 `StringRepresentable`，后续拒绝不掩盖外层执行失败 |
| 当前 `World::clone` / `clone_world` / `initial_world` 抽取根 | 当前 20 字段、维护状态、窗口、保留记录及新增枚举的具体值复制对应；初态精确对应实际空表/空列表/计数器/运行标志 | 仅初态与只读处理所需的复制闭包，不是维护转换精化；内置 Vec/tuple 复制及分配/drop 边界限 [A6-network-stdlib](formal/dependencies.md#a6-网络地址库模型使用登记) 的具体字典与纯值语义，不含 OOM/物理分配语义 |
| 历史两模式策略的连接 | 历史 Endpoint 往返与单射；拒绝全部/静态名单的决定与当前规约一致 | 不将新增 IPv4、维护字段或生命周期枚举丢掉后冒充完整历史 World 对齐；旧权限形状/可达性定理仍须迁移 |

`SandboxModel/Network/Audit.lean` 创建准入基线的 43 项、`SandboxModel/Access/Audit.lean` 85 项及下节本地组件 153 项定理均在 Linux 编译、逐项依赖审计通过，只允许三个标准逻辑依赖；无占位证明、原生计算背书或自有不透明算法。创建准入证明输入为 `d8e947abc1bfea75424004961f75c647896dc22b`；`bash scripts/verify-network.sh` 通过冻结输入、十四个真实调用/输入值根、自有生成函数/类型/六个循环及平台初始化体、十四个标准库外部接口及两类型、重新抽取一致性、编译和依赖审计；编译器生成并被纯后端消去的 32 类字段/闭包 drop glue 单独核对完整 AST、派发及白名单，三个标量捕获闭包的别名逐字段核对，不能据此略过普通自有函数或自定义析构。该抽取目录保留原名称，但共享当前完整 Rust 类型，防止不同小批各造一份名义不兼容的 World；证明责任仍分属 Network、Access 和 Local，不将完整输入值表示或共享生成闭包当作全输入行为覆盖。

Linux 完整基础检查 `scripts/check-function.sh` 已通过，含 1012 项工具测试（无跳过）、Rust 工作区、静态检查及诊断特性检查；两套抽取审计的 32 项定向测试与文档链接检查也通过。完整值/运行批次的完整回归重跑通过，完整证明入口内再次通过公共基础、共享状态模块及 Rust 检查后于历史生成物一致性处退出；整套验收仍未通过。历史授权批次曾收紧隔离目录权限后通过，未削弱生产检查。以上不包含 KVM、SDK 云服务或完整系统形式化验收。

上述两处理函数的对应比较精确拒绝，或事件与完整当前 World；独立规约只抽象守卫读取的记录字段，同时参数化保留完整状态，不把其余字段丢掉。`egress_preserves_any_invariant` 只保持已成立的不变量，不负责证明创建、发布、撤销建立这些不变量。当前 `handle_access` / `handle_controller` 统一派发、其余当前生命周期与维护转换、全局权限/围栏可达性、SDK 规范化和实际宿主网络仍未连接。未修改虚拟机、通信协议或部署网络规则，不以测试对照代替等价性定理；#337 与完整证明验收继续保留未完成状态。

## V4 本地组件：当前骨架与首批迁移

当前 [Local](../SandboxModel/Local.lean) 将纯模型、实际表示关系、实现对应及性质分开维护，不依赖旧 `RustRefinement` 的完整业务模型。[State/Model](../SandboxModel/Local/State/Model.lean)是不导入 Aeneas 或生成 Rust 类型的独立值模型：整数为自然数，表/向量为有序列表，保留全部当前构造及重复项。[World](../SandboxModel/Local/State/World.lean)逐字段连接 20 字段及嵌套维护状态；值映射的单射性保证不同当前值不会因删字段而变成同一模型值。它不要求任意无界自然数模型都可表示，也不提供字段合法性、唯一性或可达状态不变量。共享基础只含通用数学与固定库模型；旧有限扫描公开名字保留兼容封装，不据此声称旧总证明已经恢复通过。

| 组件 | 当前已证明连接 | 边界与后续义务 |
|---|---|---|
| [公共有限扫描](../SandboxModel/Foundations/FiniteScan.lean)与[克隆](../SandboxModel/Foundations/Cloning.lean) | 从旧成果迁出全部/任一匹配循环的终止与结果定理，去除业务类型依赖；当前真实 IPv4 循环体先证与公共扫描相同，再复用扫描定理，保留任意起点范围；完整 World 的具体复制调用公共克隆引理 | 实际循环体连接不能由通用定理代替；克隆仍限已落实的值保持字典及原纯分配合同 |
| [修改范围](../SandboxModel/Foundations/Focused.lean) | 参数化一块自有状态与其余状态，证明成功只更新自有部分、拒绝对应、其余部分上已成立的不变量保持 | 不建立业务不变量，不自动证明两模块共享字段的竞争或持久原子性 |
| [完整当前状态与输入值](../SandboxModel/Local/State/Properties.lean) | 全部字段及嵌套记录/枚举保真；九组公开输入及每组载荷单射，含维护、IPv4、撤销准备、停止退休；各组真实构造见证，输入值根精确返回，真实初态/克隆连接同一完整状态 | 输入值根只保留类型、不是执行统一入口；构造见证不等于请求已通过准入。九组处理函数及产品子集封闭、业务不变量、窗口/维护行为仍待证 |
| [真实命令与尝试相等](../SandboxModel/Local/State/Commands.lean) | 十二种命令的完整参数、可选恢复来源、三模式策略及当前尝试编号比较对应；完整 EffectId 具体相等字典实例化真实有限表查询 | 只证明比较和首次匹配，不证明派发、去重、回执可信性或重试条件；元素字典没有移入外部假设 |
| [当前完整运行入口](../SandboxModel/Local/Runtime/CurrentRefinement.lean) | `handle_runtime` 三类输入：崩溃、恢复、重复/迟到效果观察的全部成功/拒绝、精确事件和完整自然数 World 对应；所有其他字段保持，崩溃/恢复模型往返，重复/迟到观察精确查询尝试且不改状态 | 迟到观察即使控制器标志为关闭也允许存在检查，与实际实现一致；它不执行或认可宿主动作。不是进程故障、日志重放或 Redis/SQLite 重启恢复；统一控制器分派仍未迁入 |
| [创建准入](../SandboxModel/Local/Creation/GuardRefinement.lean)与[完整创建模型](../SandboxModel/Local/Creation/Properties.lean) | 实际 `handle_create` 七项检查依原序对应；全部拒绝与独立规约双向一致，其他请求错误不可能由创建入口返回；操作键、归属及维护编号前沿的具体比较/查询；模型接受充要条件、关闭授权、沙箱/工作区/账本/计划更新、11 个未改字段与维护其余字段保持、初态准入见证 | `guards_exact` 保留实际成功更新而非假定数学后继；`root_success_after_update` 以真实 `begin_create` 成功为显式未闭合前提。完整成功更新、有限容量内正常返回、初始工作入队、可达不变量与生命周期组合仍未证明；条件连接不能计作整个创建处理函数精化 |
| [当前容器连接](../SandboxModel/Local/Containers.lean)与[创建工作索引](../SandboxModel/Local/Creation/Containers.lean) | 两个实际 `insert_unique` 生成循环及包装连接固定 V0 顺序复制/查重合同；当前 `Store.put` 真实循环连接公共首次替换/缺失追加；已有值不需克隆或追加余量，缺失需可表示容量；相等/克隆失败保持；唯一性、注入视图、其他键查询保持；完整 EffectId 比较和克隆具体实例、初始围栏尝试的非空见证 | 泛型字典由调用者证明，不作外部前提；完整尝试键已实例化。只证明纯容器值语义，不证明物理 OOM/析构/失败回滚；创建的 commit/enqueue、全部字段更新、容量组合与可达不变量仍待连接 |
| [创建编号、计划与操作登记](../SandboxModel/Local/Creation/Updates.lean) | 实际 `record_identity` 两分支与完整 World 对应；`resource_index` 首次匹配、默认空值及原资源顺序；Create 分支的完整初始计划与账本；实际 `store_new_operation` 的两次写入对应独立 `storeOperation`，其他字段保持；连接真实初态的正常登记见证 | 登记不检查创建准入或授予运行权限；编号前沿递增由准入守卫及可达状态另外保证。两张表各自的分支敏感 `TableWriteCapacity` 是显式调用者义务，不包含物理 OOM；其他操作种类、commit/enqueue、整个 `begin_create` 与容量组合仍待证 |
| [资源位置](../SandboxModel/Local/Slots/Properties.lean) | 当前 `reduce_slot` 四类输入的全分支对应，比较完整五字段身份及精确拒绝；绑定接受充要条件、严格新 VM 编号、释放中不可复用、重复释放幂等、合法首次绑定见证；模型的迟到旧绑定拒绝连接到真实抽取根 | 单位置纯转换不检查位置范围、跨位置唯一性或租户权限；`ConfirmRelease` 不自行证明物理资源消失及持有者排空。全局维护调用、持久串行化、宿主证据和复用后执行隔离仍由 V4/V5/V6/V8 连接 |

实际 Rust 适配仅将 Bind 的可选前身份比较改为显式匹配，将派生 VM 顺序比较改为原整数比较；首项身份拒绝、第二项未空闲拒绝、第三项旧编号拒绝及成功绑定保持原序。新增边界对照测试覆盖空记录、全部三相位、缺失/不匹配前身份、零/最大编号与位置；原丢回执/迟到消息及千轮复用回归保留。新根仍直接调用生产函数，不维护另一个待证明控制器实现。位置转换与纯标志切换不启用任何新产品功能。

创建准入基线的本地审计包含 153 项定理，当前完整值/输入、三类运行入口及创建准入完成真实 Lean 诊断、重抽取一致性和逐项依赖审计。创建准入批次固定复用 V0 `9f2a37d702897156f53503795f6267066a6a6014` 与完整状态/输入 `0406010eef1d64edf9bf54a748d5c7644aa85dae`，以 `a48b3a4fd6df83dbcb816d021832ebfd0f9e11d7` 组合，保留 #342 的 `3255255b207d2bcd41b90d92a7445b1ffb67df82`。未修改生产 Rust；完整 Linux 基础与整套验收的本批结果另行登记，不沿用前批背书。完整状态和输入的**值层对应**已经建成，当前成功更新仍未完整迁到这套关系上。公共有界扫描/前置克隆缺口交 V0，具体循环体、键和完整状态连接归 V4；当前产品的创建、调度、工作执行、回执、运行发布和销毁继续按入口推进，暂停/快照行为不进入迁移范围。局部审计通过不等于整套验收通过，也没有把内部工作启用义务移为外部公平前提。

容器连接批次从真实 integration `8c03525f5671c70d0a3ff5b10a40bb18b0817deb` 展开，证明输入为 `26ba78f0298b35960a87aaaad0fd9d1499ab4648`，新增 22 项，本地审计合计 175 项。实际 Rust、全部生成语义、V0 公共源码/清单、库模型及工具版本不变；新 V4 模块及其实际公共循环依赖纳入本模块输入清单。独立 Linux 工作区通过同一 Lean 服务的隔离试验、两个正式文件非过期完整诊断，以及正式编译和 175 项标准逻辑依赖审计。原 Linux `check-function.sh` 在同一冻结提交通过：1034 项工具测试无跳过，文档、Rust 格式/静态/工作区/文档/诊断特性检查通过；既有需 root 的 Rust 忽略项未执行。原 `verify-controller.sh` 实际通过 Rust 检查、V0 118/网络 43/授权 85/本地 175/Linux 回执 32 项局部审计及相应重新抽取一致性；全核心翻译 265.628059 秒完成，无 OOM，随后历史 Types/Funs/清单失配退出 1，未进入后续全部历史定理编译/总审计。失败产物完整保留，历史原件按冻结提交恢复；**局部通过，整套未通过**，未削弱任何原关卡。字典、逻辑追加余量和具体状态更新属于调用者自有义务；完整 EffectId 已实例化，但 `begin_create` 成功更新前提仍未闭合。新增模块没有启动虚拟机或提供真实回执，KVM、云 SDK、题集未运行，不扩大暂停/快照范围。本批已自查实际循环、首次匹配、失败传播、容量和模型投影，草稿 PR #436 跟踪；不混用 V0 #435 在途版本的结论。

创建登记批次从私有组合 `c47f47e6e042d03c9d97b484cdbfbec23b930833` 展开，固定依赖公共 #435 的 `60a2dd2b617d1b66a28b5033e232720fe482bc35`、本地 #436 的 `adaf41f4136a61e29c67a840188fdff1f01850ac` 与实际 integration `58c60385de1e87958abc536f2a5665c1230a333b`；最终证明输入为 `1cd3e3a2d350d6f5b8cdb10e18e58cffde7ee20d`。只新增完整状态的独立登记函数、10 项对应/可实现定理及审计/摘要；生产 Rust、生成语义、V0 公共模块、库模型及工具不变。独立证明服务器完成隔离探针、最终正式模块编译、185 项本地依赖审计及同一 Lean 服务的目标读取和完整非过期诊断。原 `verify-controller.sh` 在同一冻结提交通过 Rust 检查、公共 136/网络 43/授权 85/本地 185/Linux 回执 32 项局部审计及相应真实重新抽取一致性；完整核心翻译 274.821293 秒完成，无 OOM，随后历史 Types/Funs/清单失配、退出 1，未进入全部历史定理编译/总审计。新候选三项摘要与前批相同，证据保留，六个历史/中间原件经冻结提交核对后恢复，未削弱关卡或覆盖旧成果造绿灯。原 `verify-first-chain.sh` 另复验通过，保留 #342 的 53 项模型审计及其条件边界。独立 Linux 测试环境的原 `check-function.sh` 在同一冻结提交完整通过：1034 项工具测试无跳过、文档及 Rust 格式/静态/工作区/文档/诊断特性检查通过；既有需 root 的 Rust 忽略项未运行。冻结提交普通四项 CI 通过，KVM、云 SDK 和题集不适用、未执行。末尾空行修正前的结果不替代最终文件诊断和验收。这批只覆盖创建编号与原操作计划/账本登记；`root_success_after_update` 的完整更新前提仍开放，不扩展暂停、恢复、快照或物理执行的证明范围。自查完整字段、调用、首次匹配/重复项、容量及真实初态见证后交草稿 #458，依赖 #435/#436，不自动合并；**局部通过，整套仍未通过**，#337 保持开放。

创建准入证明输入 `d8e947a` 的 Linux `check-function.sh` 已通过，包含 1028 项无跳过工具测试、文档、Rust 格式/静态/工作区/文档与诊断特性检查；既有需 root 的 Rust 忽略项未执行，不计为实机证据。首次完整入口在共享依赖文档旧摘要处失败；`1e7c0a231529d3ab86d5e79f9c01b89c6068516a` 仅同步该文档及引用清单摘要，V0 代码/定理/生成语义与固定 `9f2a` 不变，四份摘要清单通过。新冻结提交经共同重型锁重跑完整入口，再次通过 Rust 检查及 69/43/85/153/32 项局部审计；随后历史完整核心的 Aeneas 翻译因私有进程组内存耗尽被终止，尚未到达历史生成物一致性及整套定理编译/依赖审计。因此**整套验收未通过**，不能沿用前批失败类型或把工具资源终止当作业务反例。失败中间产物保存在忽略证据目录，未覆盖正式历史生成文件。未运行 KVM、SDK 云服务或题集。批次自查已完成，草稿 PR #402 与 #337 记录进度，仍不宣称当前生命周期主链路已经全部精化。

目标仍是分模块建立“Rust 实现 → 抽取的 Lean 语义 → 独立模型 → 安全性质”的验证链；不能每次只运行模型定理便称服务已验证。当前新增义务如下：

| 层次 | 当前实现的待证明连接 |
|---|---|
| 纯控制核心 | 在完整当前值表示、网络入口/出口和三类运行入口之外，补齐九组输入的处理/统一派发与可达性连接；`AbortPrepare`、`RetireActivation`、`RetiredByStop`、维护状态、结果保留与请求窗口的业务转换，连接同一完整状态表示 |
| 持久服务 | 意图、准入、编号/收尾额度和 outbox 原子接受；只在确认提交后派发；不确定提交使缓存失效且恢复同一身份；单写者下缓存与检查点＋连续输入尾/SQL 同一快照对应 |
| 生命周期与授权 | 启动期限和运行寿命的区分、运行起点原子发布、续期/到期竞争、总寿命截顶、归属及令牌撤销、终止抢占；内部恢复和鉴权不能整体假定正确 |
| 宿主执行与回执 | 当前操作/代数/尝试及物理位置 incarnation 绑定；可信停止证据先于逻辑释放；未知结果保留资源占用，迟到回执不恢复旧权限 |
| 精确资源清理 | 启动放行前持久登记目录身份；只消费原设备/inode/创建时间、进程、网络和位置证据；同名替换、缺见证、部分删除及持有者未排空保持未知；已确认 Free 的历史身份不阻止合法位置复用 |
| 有界运行 | 工作流未来转换、有限重试/收尾、数据库与 WAL/temp/检查点、登记和稀疏盘未来增长的合成界；结果到期与闭包退休、单调身份前沿、窗口关闭和旧键拒绝；不以概率碰撞或逻辑释放冒充物理释放 |
| 数据通道 | 单沙箱凭据摘要唯一索引、原代数绑定及跨重启局部拒绝/健康对象隔离；派发与撤销/数据 pin 的次序；有界在途缓冲及并发、持续大输出、上传期限、写半关闭后的迟到拒绝响应、精确停止并回收中继；EOF 不是物理清理证明 |

关系允许证明侧保留抽象历史，而实现删除不再可观察的细节；不能要求压缩前后完整 World 逐字相等，也不能因此遗漏仍可查询的结果或已接受未知工作。有限机器只支持可表示前缀内的安全拒绝与明确前提下的收尾，不承诺无限接单、永久外部故障下完成或任意内存耗尽时可分配。

SQLite 原子事务、Linux/pidfd/文件系统观察、Firecracker/jailer 及密码学原语有明确外部合同；**自有检查次序、见证构造、互斥、恢复、事务连接和授权决定不是外部可信假设**。这些仍需分层对应证明与系统测试。

#262 的接口整理将宿主与数据连接合同及排空/子进程持有者移入 `controller-service/src/application`，网关经 `DataAccessPort` 调用；实际授权、生命周期、恢复和维护编排已迁入 `controller-service/src/application/coordinator`。该基线的本地 HTTP 适配目前仅保留在 `sandbox-api/tests/support/managed.rs` 作为组件夹具，生产 SDK 转换由 `sandbox-api` 的唯一解析/投影与中心 `CenterPort` 连接，不再存在 `sandbox-api/src/managed.rs` 生产模块。应用查询使用数值时间与普通选择条件，SDK 时间/令牌编码不混入控制服务；模板编排单独组织但仍排除证明。此搬迁及可注入可信时钟没有改变纯核心或生成证明，新增应用代码尚无对应证明，不能用旧定理背书新模块。宿主端口已按能力拆分，生产实现位于 `sandbox-node/src/adapter.rs`，物理配置租约位于 `sandbox-node/src/physical_profile.rs`；恢复与执行只共享证据类型，仍须分别证明调用资格、未知观察和回执连接。模块搬迁没有新增 Lean 对应证明。后续对应证明须连接端口调用与原管理锁内的派发重检，以及 `TrackedRelayChild` 的确切回收/未知退出责任；还须证明各检查点读取时钟及同一互斥对象内持久决定/宿主派发/撤销/排空的组合；不能把一次时间快照代替执行后与派发前复核。系统时钟的可信性属于外部前提，应用端口不是可以整体假定正确的授权器。

节点的值判断现集中于 `sandbox-node/src/linux/decisions.rs`（动作字段、资源映射、围栏、槽位坐标、进程身份及命令证据），现场读取在 `identity`/`observation`，执行和补偿在 `execution`/`recovery`，位置退休与精确释放仍由 `host_slots.rs` 负责。纯比较不是授权证书：操作/owner/物理位置核验和原锁内的现场读取顺序仍须连接证明。同步子进程统一使用 `sandbox-runtime::OwnedChild`，异步中继仍独立承担持久身份与排空责任；本次整理未运行抽取或新增 Lean 证明。

## 中心与系统组合：工程实现与待证明连接

`1890690` 之后的重确认写放大候选修改 [Redis CAS 脚本](../sandbox-center-store/src/scripts.rs)：保留原类型、四字段、部署/格式、原修订、完整旧 payload 字节、长度与无 TTL 检查；已存在且新旧 payload 字节完全相同时，仅写入原 Rust 已检查的新修订，初始化及 payload 变化仍写完整四字段。每条接受路径仍恰好一次实际 HSET，原 `appendfsync always`、写后持久健康核对、期限、未知围栏及显式恢复不变。真实 Redis 的 AOF 增长、原记录和强制重启回归属于工程证据，不是 Lua、效果传播或耐久连接的证明。旧冻结候选及旧 Lua 前端试验的摘要、指令和适用结论不覆盖这份新脚本；实际 CAS 与表示/恢复连接仍由 V2 提供。未改纯核心、抽取物、Lean、假设或工具版本。

`a45f490` 之后的创建故障诊断候选修改原[中心等待](../sandbox-center-service/src/application.rs)和[Redis 查询适配](../sandbox-center-store/src/store.rs)，仅增加失败阶段、封闭错误类别及有限数值输出；每进程最多各 64 次写入尝试，输出失败不重试。原返回、期限、写围栏、未知语义和身份检查保持；这些诊断副作用及服务连接尚未证明，旧指定版本定理不覆盖新增代码。纯核心、抽取输入、Lean 和工具版本未修改。

私有构建数据路由在原节点新鲜度检查拒绝时，补充与公开路由相同的固定 `CenterNodeFreshness` 诊断；不输出令牌、路径或请求内容，不改变原 Busy 返回、健康期限或授权条件。适用差异为 `29ee414` 之后的该诊断候选；[私有入口](../sandbox-center-service/src/private_gate.rs)及诊断副作用连接尚未证明，旧局部定理不覆盖它。中心纯核心、已抽取根和 Lean 均未修改。实际 Redis 回归检查重复拒绝不变更权威状态、其他租户仍拒绝、原健康观察恢复后保持同一实例路由；它不证明宿主健康持续可用，也不解释尚无具体诊断的实机 503。

`19d31c0` 之后的原中心观察诊断候选在[状态观察](../sandbox-center/src/observation.rs)保留原传输与策略接受结果，并通过[有限输出](../sandbox-center/src/observation/diagnostics.rs)记录既有时间值和固定类别；每进程最多 64 次写入尝试，失败不重试。它不改变 `healthy_until`、节点权威提交、宿主调用次数或生命周期行为。该诊断副作用、原调度间隔及实际健康持续可用性仍未证明；纯核心、抽取根和 Lean 未修改，旧定理不能背书新增 Rust。对应测试仅核对原拒绝/旧期限过期分类、输出总量和故障输出端，不代替实机原因确认。

`d45f1e0` 之后的[中心后台调度](../sandbox-center/src/runtime.rs)将原健康/生命周期轮次的 I/O 耗时计入扫描间隔；单个原 Background 许可继续覆盖实际阻塞工作，慢轮次只立即进入下一轮，不积压错过的轮次。未放宽健康窗口、原请求期限、准入或节点接受条件。可推进时钟和工作屏障验证短/慢轮次及关闭时的实际任务所有权；调度公平性、长期可用性和真实 I/O 连接仍未证明，不能由旧纯核心定理推得。此修正未改纯核心、抽取根、Lean 或工具版本。

目标与模块顺序见[验证总览](formal-verification-design.md)，状态/故障及义务见其V0–V9详细计划，端口语义见[模块合同](module-contracts.md)。#292 复用原本地算法，新增纯中心决策、Redis 单块协调、宿主持久收件和两端网关；结果确认、逐实例保留位置及代次退休、原 SQLite 高级入口已有工程实现。唯一入口迁移代码已集成（#321），不因此认定#292的完整故障与晋升验收已通过。以下P1–P8按总览对应V0–V9，不是新代码已有证明；#101跟踪交付，实际证明批次须冻结同一组合提交。计划拆分未改变下文历史定理的适用版本。

| 工作包 | 待交付连接 | 当前证明状态 |
|---|---|---|
| P1 共享合同/类型 | 全局/本地身份、版本/窗口、完整参数、有限数值与编码，公开输入穷尽 | 公共数值、完整身份纯检查、创建参数守卫和工作结果分类已有指定输入对应，范围见上节；编码、窗口及所有服务连接仍待证 |
| P2 中心控制 | 唯一固定节点、全局准入/收尾、原意图/终止、查询和退休 | 决策、运行/构建池选择、有限健康否决及确认后窗口关闭已有工程实现；校验、查询及预留已有等价抽取适配，完整创建仍受字符串类型和库模型生成限制阻断，尚无中心对应证明 |
| P3 中心持久协调 | Redis 权威表示、CAS/Lua/耐久确认、缓存/outbox、回执和独立恢复 | 原 CAS 同块保存全局责任及模板/构建引用；只有工程检查，引擎前提和自有连接仍分别待证 |
| P4 内部协议/节点入口 | 双向认证映射、同键全参数去重、终止先到、原映射、确认/窗口 | 原 SQLite 高级入口、同一工作许可、事实提取/续期/确认/有限窗口已有实现；取消、实际执行和退休连接未证明 |
| P5 本地控制/执行 | 当前核心/SQLite 与实际围栏、观察、停止和精确清理 | 下文历史纯核心成果可按版本复用；新增连接未证明 |
| P6 公开入口/两端网关 | 归属/语义、全局查询/路由、本地资格、流完整性、驻留与排空；原销毁等待与退休的交错只读取新鲜耐久匹配终局，期限及未知拒绝不变 | 中心复用原 SDK 解析/投影和唯一流处理，宿主重新检查原本地资格；终局等待的工程回归不等于对应证明，产品迁移和工程验收不增加证明结论 |
| P7 系统组合 | 邻接前提、共同不变量、消息、独立恢复与实际组装精化 | 未交付组合定理 |
| P8 容量/保密/完成性 | 中心预留覆盖节点潜在占用、安全保留/退休、双执行关系及条件推进 | 历史内部义务仍开放；新系统结论未证明 |

[#334](https://github.com/ningZhang-cs/sandproof/issues/334) 的后续试验基线为 [#347](https://github.com/ningZhang-cs/sandproof/pull/347) 的 `74a863e564e6e4252b01933312699d93f2765152`。精确源码 Git blob：[model.rs](../sandbox-center-core/src/model.rs) 为 `8dd95d0be671d2fca695488c6e869d8128f69d4c`，[window.rs](../sandbox-center-core/src/window.rs) 为 `e403d417f5548afe33472a39ab46474f4978df97`，[decision.rs](../sandbox-center-core/src/decision.rs) 为 `68670cb926451457c7c8c6582de29275734c198e`，[validation.rs](../sandbox-center-core/src/validation.rs) 为 `ca4743d9e29286f50f6afe5640802ec6eca8c9f9`。五段校验仍共用原去重集合；借用查询改为原谓词的首个位置再取值，资源预留按原顺序累加且溢出仍返回 None，方法项回调改为调用同方法的闭包。条件、拒绝顺序和字段不变，工程回归不代替 Rust 语义等价定理。

固定 Charon 对真实 `CenterState::create` 调用及全部可达自有逻辑抽取成功，自有不透明函数为零；固定 Aeneas 已越过循环、`find`、`try_fold` 及高阶函数项障碍，目前在 `core::str::pattern` 的类型翻译处失败。实际 `lane` 可生成 Lean，但其标准库 NonZero 相关 trait 生成重复 `markerCopyInst` 字段，未通过 Lean 编译；完整单态化试验也未完成生成，未以手改产物获得通过。另在隔离 Rust 探针中，真实下标查找已完成抽取、未改生成文件的 Lean 编译及对独立 `List.findIdx?` 模型的对应、首个匹配和条件终止证明；首个位置再取值的通用模型引理也通过依赖审计。**探针不是产品的 `lane/create`，通用模型引理不等于真实标准库 position 已被证明；这些局部成果未接入正式证明验收。** 版本、生成清单、完整源码/产物指纹与命令归关联 PR/忽略的 artifacts。未升级工具、未扩大自有可信边界、未覆盖正式 LLBC/Lean；中心创建与标准库连接仍未闭合，不宣称完整证明验收通过。

新增的 `sandbox-protocol/src/internal.rs` 与 `sandbox-internal` 尚未抽取或建立 Lean 对应证明；本批不修改历史纯核心、独立模型及正式 LLBC/Lean 产物。待证连接包括完整身份/协议/计数器编码与拒绝关系、CA/登记摘要到节点角色的映射、固定端点约束、完整正文结束或原连接取消回收后才退还额度、期限与正文预算及任务回收。密码学/TLS 引擎按明确外部原语合同使用，自有登记核验、检查顺序、错误处理和额度连接不能整体视为可信黑箱。只读探测没有耐久结果或执行权限，后续持久映射/高级操作仍需独立连接；组件测试通过不新增系统组合、保密或完成性定理。

本次唯一入口迁移还增加下列连接义务：静态注册与原恢复块一致且不重置代次/高水位；健康/物理空闲的有限否决不能解除责任；GLOBAL 描述到仅改变路径的 LOCAL 描述及原租户映射的准确对应；确认未到不压缩、该实例原责任及引用结算后才退休其固定位置；调用取消不提前退还实际阻塞工作许可；缓存与构建池各自独占、共同峰值仍计入原物理 profile。协议 2 的 `retention_slot` 与该位置的 `request_window` 分开建模，最近完整终局回执、单调拒绝前沿及永久全局/本地编号仍需证明；旧请求/迟到确认不能触及后继实例，长寿命同伴不阻塞另一位置的条件推进，未来每位置回执空间纳入准入预留。这些新增纯转换与 Redis/SQLite 连接只有工程检查，未抽取或证明；历史纯核心结论不覆盖新格式及入口。模板编排/产物服务整体继续排除证明，但核心使用授权、不可变、准确模板及资源描述的前提仍明确。受保护目录账号是可信部署输入，不是可由客户端指定的权限；自有路径/身份/预算检查不因这一输入而自动成立。

需要补建系统状态与历史，不只扩展 World：中心决定/持久缓存、各节点本地核心/持久/真实资源/通道、中心通道、在途消息及独立时钟/故障。模型参数化任意预算内有限静态节点，允许两端滞后、重复/迟到/乱序/丢失及独立崩溃。两节点组件或一台物理机的双服务测试不是一般定理。

`sandbox-center-store` 只有工程测试，尚未抽取或建立对应证明。需连接严格状态编解码、四字段控制块、原字节/修订 CAS、独占写者、持久健康及未知后耐久重确认，再连接未来中心的提交前不派发、额度和原操作身份。`Observed`/`Committed` 类型区分不能代替这组定理；Redis 持久化、可信同机回环/ACL、文件锁和文件系统原语仍需外部合同，自有检查/围栏与连接不移入可信边界。本批不修改历史纯核心或 LLBC/Lean 产物。

新增中心的决策、模型与恢复校验分别位于 `sandbox-center-core`，没有 I/O/VM 依赖，时间是显式输入；序列化、身份字符串/编号解析、整数比值和编码包络仍有自己的对应义务，不宣称能无修改抽取整个服务。异步 HTTPS 复用原 TLS/HTTP 边界，`sandbox-host-service` 在原应用锁前统一限制收件、执行和周期恢复的阻塞任务，并把许可保留到实际任务退出；这一取消/实际执行连接尚未证明。`CenterAuthority` 与脱离借用的 `Dispatch` 需证明成功提交/当前意图/发送顺序及迟到回应隔离；宿主收件需证明原事务、全文/摘要、终止先到、未来 SQL 保留与格式恢复的连接。原子准入映射复用原本地事务；新增义务包括租户映射单射/单调、完整模板及获批形状对应、销毁收件与原核心终止同提交、事实观察与原 Running/Destroyed/ConfirmRelease 的对应，以及确认之前的历史 pin。工程故障测试不等于没有重复 VM 的实现证明，HTTP 收件不授予绕过本地检查的执行许可。修改的 controller-service 格式/容量/恢复胶水没有新增定理；旧纯核心的指定版本结论不背书这些新增组件。所有新模块只有工程检查，实际证明继续归 #101。

查询的调用期间视图与已确认续期顺序、命令句柄/终止范围及 envd 登记丢失、局部故障传播与跨实例推进、零计数键/订阅/池/关联/队列及实际退出计费均为明确义务，精确定义分别见[V7](formal/07-public-api.md)、[V8](formal/08-data-gateway.md)、[V9](formal/09-system-composition.md)。分类为控制器证明、外部条件下证明、工程测试或不承诺不等于完成；尚未闭合的自有连接不能移为外部条件。Start/Connect 无累计输出上限，SDK 客户端 RAM 不属宿主内存结论。

创建/连接等待复用原回复期限，健康否决只重查原会话；迟到观察仅在新鲜持久快照仍为完全相同的原操作时重查，不把旧包当成功。这些连接尚无证明。模板缓存的原身份恢复与库存不可用状态采用工程测试；核心仍须证明缓存失败不授予新执行资格、不能抹除已有实例责任，并连接唯一宿主驱动对原实例的到期/终止/精确收尾。模板排除边界不排除这组局部故障隔离和生命周期连接。

实际 Rust 对应须包含锁/事务/异步/回调及微步骤，固定 Charon/Aeneas 支持子集不覆盖任意 Rust。手写可计算库模型无公理不等于已证明真实库等价；库模型前提、工具/编译器、平台/features/cfg、源码/生成入口及指纹须逐批列明。模型安全、实现精化、系统组合分别验收，有限步安全不直接推出最终完成性。

[#328](https://github.com/ningZhang-cs/sandproof/issues/328) 的查询与模板分发修复不修改已证明纯核心或 LLBC/Lean。当前 `PublicSnapshot` 同时投影原包络的生命周期、隐藏对象及不可变模板绑定，get/list 字段只消费这一视图；后续 V7 对应须连接精确绑定、资源形状与原筛选/排序/分页，而非把模板提供者的另一时刻读取当作同一快照。原 Redis 重确认和未知围栏不变。模板缓存新增按受管修订/摘要的有限轮转提示，只有原可用回执提交产生缓存事实；提示的键清退、有界性及重启不改变责任属于工程检查，模板子系统整体排除边界不变，不新增形式化完成性结论。

## R2 与持续输出的待证明连接

[#290](https://github.com/ningZhang-cs/sandproof/issues/290) 的宿主辅助执行与命令输出策略依[模块合同](module-contracts.md)区分。Start/Connect 不再要求累计输出 `nr ≤ B`；只要求任意有限执行前缀内已写字节不超过已读字节、保持顺序，并分别约束在途队列、单消息、并发与撤销。文件、一元回复和宿主辅助输出仍有各自累计预算。幽灵历史可以增长，不能因此要求实现保存完整历史；执行期限不能替代内存边界。固定 envd、HTTP 库和内核缓冲的可信前提，以及客户端累计输出不在宿主保障内，均须明确。

同步辅助命令新增非阻塞三管道交换、分层期限、累积缓冲前输出检查、启动前额度和固定未知退出回收器。后续局部证明应分别连接额度守恒、错误不产生成功证据、精确进程退出观察、未知保留持久身份及物理资源占用，再连接节点回执与控制器发布。不能把超时视为未执行或已回收，也不能依赖 Drop 的无限等待证明终止；退出仍未知时继续占额，最终完成需要外部恢复/调度前提。这些适配层改动尚未抽取或形式化证明，历史纯核心定理不背书它们；本批未改已证明核心或正式 LLBC/Lean 产物。

#290 R6 的故障投影、恢复样本和后台阶段诊断属于当前应用/宿主实现，只有工程回归，没有新增 Lean 定理。纯核心及生成 Lean 未改变。后续对应证明仍须覆盖原错误分类、局部隔离和共享损坏中止，以及诊断不参与授权、状态转换或资源释放；日志成功不是工作完成证据，不能把本批测试当作当前服务已形式化验证。

## 模板子系统排除边界

按[蓝图](project-blueprint.md)与 [#123](https://github.com/ningZhang-cs/sandproof/issues/123)，整个模板构建与转换子系统采用工程测试，不做形式化证明：第三方构建器、Dockerfile 执行、OCI 获取/解包、镜像制备，以及自有构建编排、查询/别名/登记和发布胶水均排除。

#311 在 `036e2b7` 之后调整[宿主缓存回执](../sandbox-server/src/linux/host_entry/cache.rs)：完整原校验产生的进程内记录与有限条目绑定，重复请求核对原描述、可信路径和实际文件身份；记录不序列化，恢复仍完整重算。`ff18f12` 之后增加同一宿主驱动到原节点摘要缓存的进程内记录交接，并在下载临时文件中保留全零数据的空洞；完整流摘要、逻辑长度、预算和原节点校验仍执行。上述应用/I/O 适配层只有工程回归，没有新增抽取或证明。既有定理适用提交、纯核心和正式 LLBC/Lean 未改变，不能用于背书这段新连接。后续核心组合仍须连接原已校验描述的准确使用、回执拒绝不授予执行资格、不抹除原实例责任及独立精确清理；模板排除边界不免除这些义务。

#311 在 `9a99ffb` 之后调整中心发布元数据的历史本地 owner 比较：已结算且完成原窗口确认退休的记录不再阻止同一全局租户获得新单调本地编号；活跃映射稳定、跨租户历史编号隔离及实例身份唯一性仍检查。该业务包络适配尚未抽取或证明，既有纯核心、LLBC/Lean 及定理适用版本不变；工程回归须分别核对退休前拒绝、退休后接受和跨租户复用拒绝，不能把已有核心定理当作新组合的证明。

但核心**使用**模板绑定、资源形状记账、恢复匹配与实例精确清理仍在后续验证范围，不能借模板排除条款一起排除。部署前提是产物校验后不可变、交接前关闭写者、路径可信且无不合作写者；ctime 与摘要缓存本身不是这一前提的证明。不宣传用户镜像、供应链或整个沙箱平台都已形式化验证。

#311 在 [TemplateBinding](../controller-service/src/templates.rs)增加可选 [guest 身份](../controller-service/src/templates/guest.rs)，扩展原 canonical digest、不可变登记和恢复比较的数据域。既有缺省描述保留原字节；新字段与原路径/owner 投影、SQLite 提交及中心 Redis 权威块的精确对应通过工程回归核对，尚未抽取或证明。后续连接义务包括独立 manifest/config pin、配置与 worker 版本/摘要一致性，以及被声明的产物确已安装在对应 rootfs 中；摘要相同不自行证明安装或源码到机器码对应。未改变纯核心、抽取规则、LLBC/Lean 或已有定理适用提交；历史结论不覆盖扩展后的模板输入及其服务连接。

## 保留数学模型的完成性与剩余实现连接

以下仅是保留数学模型中的定理，不连接已删除的旧生成物，也不是当前 Rust 的完成性证明。

`WorkflowReceiptProgress.lean` 覆盖创建、暂停和恢复正常阶段的当前尝试有效回执；`WorkflowCleanupReceipt.lean` 覆盖销毁与失败补偿，包括停止回执有限扫描不倒退。成功和明确失败均可降低剩余工作量，两类模块已接到实际无限事件轨迹。旧尝试回执不算当前尝试进展，观察到回执不等于它必将到达。

仍未闭合：

1. `ReservationGuardsPersistUnderNoRankDrop`：等待期间预留条件与执行前提持续成立。
2. `WorkflowProtocolClosure.select`：区分可执行的控制器工作与等待外部响应，不能要求所有未完成状态都有立即可执行的控制器动作。
3. `WorkflowServiceProgress.residual`：消除剩余内部进展前提，并组合执行节点动作、当前尝试回执、重试耗尽与状态发布。
4. 所有已接受操作（包括被销毁取代的操作）的终局稳定，以及符合条件的非当前资源最终回收。

这些是内部证明义务，不能换个名称当作环境保证。公平调度不能代替外部响应；外部最终返回可以是失败，不保证操作最终成功。永久外部故障下不承诺无条件完成。

## 长期容量：当前实现与证明限制

显式完整容量配置已经接通逻辑收尾预留、有限物理位置、检查点、分批退休和请求/构建结果保留；见[模块合同](module-contracts.md)。这是实现事实与工程验收范围，不是新增形式化结论。

已退役旧对应定理曾逐步要求 `HandlerMachineHeadroom`、`VecHeadroom` 等容量前提；没有证明新准入、维护、所有后续动作始终保持这些前提，也没有证明 SQLite 页/WAL/temp、辅助库、宿主登记及实际内存/磁盘的组合界。物理观察未知时不得释放占用；新请求不能消耗已接受工作的逻辑收尾额度。编号耗尽须拒绝且不回绕，不能把无界自然数结论直接用于机器整数。

## 结构维护风险

- 完成性证明应按进度量、具体动作、时间组合分层，避免继续向大文件追加特殊情况。
- 请求/调度按真实功能族逐步拆模块；拒绝顺序、完整后继状态、公开输入穷尽覆盖不能变弱。
- 通配公开导出和多个 `insert_unique` 实现应在实际功能修改时梳理；先确认顺序语义，再重新抽取与完整验收，不能仅为整理目录批量搬迁。
- 模型不依赖 Rust 抽取物；对应证明单向依赖模型。扩展仍使用显式输入枚举，不用开放插件绕过穷尽检查。

## 验收与新增服务

### SDK 压缩上传与大标准输入（#311，未证明）

共享 gzip 算法移至[中性编码库](../sandbox-stream-codec/src/lib.rs)，原来宾路径只重导出同一实现，原 Go 对照及全部来宾 codec 回归保留。新增[网关解压连接](../sandbox-data-transport/src/gzip_upload.rs)仍须证明压缩/展开双计数、真实校验 EOF 与成功互斥、块副本背压、取消后真实工作退出及原请求/传输额度保持；不把自有解压调度整体列为外部正确假设。大输入的[原写锁分块](../sandbox-guest-service/src/command.rs)仍须证明同请求全部切片的身份/顺序、无交叉、有限驻留、取消可能交付前缀而不重放，以及原正文准入/期限的连接。配置版本 3 的命名和不可变产物绑定须单独测试。工程验收结果按[验收清单](testing-plan.md)绑定关联 PR 的精确提交，不表示这些义务已证明；已证明 Rust 纯核心、Lean、LLBC、工具版本与证明指纹未改，依赖锁只新增本地编码库，没有第三方版本变更，旧定理与旧实机结果不背书新组合。

### Rust 来宾服务的新增局部核心（#311，抽取试验通过，未证明）

工程验收与本节证明范围独立：正式 Rust daemon、不可变模板安装和同提交 SDK/KVM 证据不使来宾代码获得历史控制器定理。使用 Rust 模板时，自有鉴权/初始化、参数与错误解释、PID/会话登记、输出结束、文件编排、预算及故障恢复仍是 #101 的待证业务义务；只对已登记的内核、运行库和薄适配采用其最小外部合同。现有 Go 模板的端点前提不能整体移用于 Rust 自有实现。工程版本和证据在 #311 记录，本节各条未证明结论继续保持。

[Start 紧凑环境验证](../sandbox-guest-runtime/src/protocol/start_budget.rs)及原 schema 的环境表示类型参数尚未抽取或证明。预检只计数和保存必要上下文；原拥有数据的映射仍严格解码，需要返回不可执行标记时另由原始正文的有序 JSON 校验建立精确无重复条件。待证义务包括完整字符串/字段验证、计数在无重复条件下到合并后数量的下界、校验成功才允许返回拒绝标记、携原错误观察避免重复诊断、规范化副本释放后候选上下文及原始正文仍有效，以及完整协议错误先行到原账户/时限/cwd/变量名拒绝顺序的连接。分块键/转义字节存储、内部索引与 UTF-8 边界、借用归并、几何前缀不漏报、最早重复 token 起点及原 tokenizer 恢复全文到流式首错的对应均未证明。键集合、正文副本和分配器驻留仍独立计费，有限回归不证明全输入或整服务上界。已证明 Rust 纯核心、LLBC/Lean 未改，旧定理不覆盖该解码与服务接线。

[共用 JSON 键表示](../sandbox-guest-runtime/src/file_protocol/request_json/key_arena.rs)在命令有序 JSON 校验中使用借用、短键内联、分块字符串偏移及超单块键独立拥有的存储；完整 UTF-8 的复制/长度不变式、跨表示的精确相等和全序、surrogate 解码对应，以及原诊断字段/位置/全文保持均待证明。布局检查和有限内存采样不构成分配器或整服务内存上界证明。

[环境合成的借用预检](../sandbox-guest-service/src/execution.rs)尚未抽取或证明。待证义务包括覆盖后唯一项与原 BTreeMap 合并结果对应、只对生效项计费、检查溢出、错误优先级及检查通过后才复制的对应；最终字节预算不等于结构分配或整服务内存预算。原已证明 Rust、LLBC/Lean 未改，旧定理不覆盖该优化，独立内存回归也不是分配上界证明。

[Start 参数计数](../sandbox-guest-runtime/src/protocol/argument_count.rs)与正式解码接线尚未抽取或证明。完整 schema 下的计数与原参数数组长度对应、合法消息保持、超额标记不可执行、JSON/尾帧错误优先及原账户/执行期限检查顺序均属新增义务。独立进程的硬内存限制回归不构成通用分配上界；其他字段、运行库及整服务内存仍须另验。原已证明 Rust、LLBC/Lean 未改，旧定理不覆盖该优化。

[服务任务观察](../sandbox-guest-service/src/task_accounting.rs)及显式 spawn 接线尚未抽取或证明。新增义务包括提交先计数、future/闭包及捕获对象析构后扣除、取消/排队/运行时退出中的守卫对应、同锁合计峰值，以及异常观察不可伪装精确计数。任务存活与最后响应字节、未知进程、调度器/库保留及实际内存之间仍需独立对应；本实现不是整体任务上界、内存界或清理证明。原已证明 Rust、LLBC/Lean 未改，旧定理不覆盖观察器及其生产接线。

[请求池饱和响应](../sandbox-guest-service/src/admission_response.rs)及三个 HTTP 调用点增加固定 429、HEAD 抑制正文和原连接预算交付，已有共享样例及固定 SDK 定向测试，尚未抽取或证明。待证义务是饱和观察到无鉴权/正文/业务效果、静态回复到有界传输和原连接期限的对应；原已证明控制核心 Rust、LLBC/Lean 未改，旧定理不覆盖此改动。

[持久 PID 单写容量拒绝](../sandbox-guest-service/src/command/registration.rs)新增独立的许可拒绝事实，并连接到同次 Start 的受控 EOS 响应，尚未抽取或证明。新增义务包括拒绝确实先于 PID 状态访问与用户放行、准备 worker 的独立取消/收尾、排队或运行中的写入不因等待者退出而提前归还名额，以及存储未知/Busy/期限错误不可伪装成容量拒绝；原响应期限和最后字节所有权仍须对应。屏障回归和 SDK 错误解析不是形式化证明，也不能据此认定历史 502 的具体根因。原已证明 Rust 核心、LLBC/Lean 和工具链未变，旧结论不覆盖新增错误分支及响应接线。

[sandproof-guest-core](../sandbox-guest-core/src/lib.rs) 新增命令订阅的身份/驻留/完成判定，以及单文件接收、短写、结束和失败计费。该版本的 Rust 定向测试、固定 SDK 协议样例及固定 Charon/Aeneas 抽取试验通过；未启动服务，未执行生成物的 Lean 编译或对应证明，历史控制核心定理不覆盖它。

可用 `bash scripts/try-guest-extraction.sh` 重现：直接抽取两个类型的 14 个选定方法，包括初始化、输出/写入、结束与失败，产物及前后源码指纹放隔离 artifacts，不修改正式控制器生成物。[清单检查](../tools/check_guest_extraction.py)逐项核对实际方法名、局部性和透明函数体，拒绝空抽取、缺项及不透明自有依赖。当前还有一个标准库 `Option::is_some_and` 不透明依赖；它的外部定义模板未导入正式库，不能当成已成立的公理。试验通过只表示选定 Rust 主体能生成 Lean 表达。

后续仍需证明驻留计数与实际缓冲/释放的连接、原进程/订阅身份、退出与 EOF/排空的先后、部分写入与失败结果，并落实标准库依赖的定义/连接；端口观察来源、公开 PID 歧义、真实分配、认证和协议/系统调用适配均不能整体假定正确。自有代码的实际证明纳入 #101，固定 envd 外部合同在现役模板被正式替换前仍保留。

新 [Linux 进程适配](../sandbox-guest-runtime/src/process.rs)已把原 `CommandStream` 接到真实 wait、管道与消费确认，并实现单次私有 worker 放行及 pidfd 控制。该适配及 [worker](../sandbox-guest-runtime/src/worker.rs)只有组件测试，未抽取或证明；纯核心源码未变，原抽取试验不能背书异步与系统调用。待证连接包括：容量/公开身份先于放行、取消后的原进程责任、私有协议未知结果、真实 PID/创建证据与稳定句柄、stdin 部分效果、管道消费与实际分配/释放。内核、Tokio 与 exec 外部前提须逐项登记；自有状态和协议逻辑不能整体列为可信假设。

[gate 路径退休](../sandbox-guest-runtime/src/process.rs)新增已认证 READY 连接与 Unix socket 路径分离、首次放行前唯一删除尝试、执行 owner 在同步删除后重查原执行期限、原句柄在 unlink 后继续使用、成功删除后解除旧目录清理责任、删除失败仍由原 TempDir 承担最终清理的对应义务。同步文件系统调用的完成性、并发路径变更、崩溃时刻与清理回执尚未抽取或证明；删除不产生进程/请求容量 credit，Unknown 不变。有限 SIGSTOP/取消、真实 ENOTEMPTY 与进程强杀测试不代表故障文件系统、全部启动恢复或 E07。历史已证明纯核心与 Lean 未改。

新增的[命令 JSON/Connect 编解码](../sandbox-guest-runtime/src/protocol.rs)已有固定 SDK 样例、畸形输入、有限的原 Go 响应编解码对照及真实进程组合测试，未抽取或证明。请求存在性/选择器、帧边界、字节保真、预算到实际分配、End 到真实观察及传输完成的连接均为 #101 后续义务；新增 List 借用元数据到有界 JSON、默认标量省略和 optional/oneof 存在性保持也未证明；Serde/base64/HTTP 库的使用不使自有适配逻辑自动成为可信前提。原纯核心和已证明控制器源码未变，旧证明不覆盖该新增模块。

新增的[初始化、鉴权与 MMDS 组件](../sandbox-guest-service/src/lib.rs)只有会话及真实 HTTP/TCP 组件测试，未抽取或证明。#101 待证义务包括：元数据来源与实例绑定、令牌/defaults 原子发布、许可持有与实际派发的顺序、取消/超时的未知边界、重复初始化收敛、正文准入与实际资源释放。SHA-512、常量时间库、HTTP/异步库及内核是逐项外部前提，不覆盖自有头部判定、元数据解析、锁/预算与连接收尾。当前已通过共用来宾 HTTP 装配连接命令/文件接口，跨接口认证状态与实际派发的对应义务见下述装配段落；正式 daemon/config、不可变模板装配及选定 KVM 验收已有实现和测试证据，完整验收状态以测试清单和 #311 为准；这些工程进展不构成证明。原已证明 Rust 与纯核心源码未变，已有定理或抽取记录不能背书此模块。

来宾[账户解析](../sandbox-guest-runtime/src/accounts.rs)、[授权上下文组合](../sandbox-guest-service/src/execution.rs)和 worker 的数值身份切换是新增自有逻辑，尚未抽取或证明。待证义务包括账户记录到数值身份、环境与身份的分离、原许可到真实派发、保存 ID/附加组设置次序、部分切换失败不得 exec、取消后实际阻塞工作与额度的对应。当前只增加组件验收入口，不把 Linux setresuid/setresgid、文件解析或线程池整段当作自有代码已正确的假设；外部内核合同和单线程 worker 前提须单列。纯核心和历史控制器 Rust 未改，旧定理仍不覆盖新代码。

[启动包装](../sandbox-guest-runtime/src/launch_wrapper.rs)、原 worker 及[命令上下文](../sandbox-guest-service/src/execution.rs)新增可信 PATH 与请求环境分离、有效身份查找/相对命中规则、继承 nice 转换、有界脚本/argv 构造和缺失 process 原元数据保留的自有连接义务。READY 前准备、持久 claim 后唯一放行、同一 PID 贯穿 exec、取消查找后的真实 wait 及两次编码预算到实际驻留尚未抽取或证明。288 组 Go 构造对照不执行原 handler；Linux 真实 shell/工具、HTTP 与固定 SDK 证据分别记录，不证明完整 E03、系统调用竞态或整机资源上界。纯核心、历史已证明 Rust、LLBC/Lean 未改，旧结论不覆盖该候选。

[工作目录检查](../sandbox-guest-runtime/src/process/preparation.rs)与[Start 拒绝发送](../sandbox-guest-service/src/command_requests/http/start.rs)的当前义务为父侧原阻塞准备的 ENOENT 到互斥观察、准备所有者到不可替换失败事实、原路径与原错误/期限到协商 EOS 的连接。此观察只在真实 Stat 失败后发布，取消/未知收尾不得复用旧事实；旧 worker 私有握手已不承担当前预检，成功 Stat 与后续 chdir 的竞态仍属明确边界。该检查、跨任务事实和响应所有权未抽取/证明，有限 Go 错误渲染不执行原 Stat/handler。纯核心和已证明 Rust/Lean 未改。

[原生启动分类](../sandbox-guest-runtime/src/worker.rs)和[回复连接](../sandbox-guest-service/src/command_requests/http/start/native.rs)新增完整原放行到原生阶段、正 errno 与内部协议故障区分、新握手拒绝旧 worker、原请求字段与失败事实不可替换及固定错误映射的自有义务。阶段标记、参数移动、已烧号后的真实收尾与 HTTP 最后字节责任均未抽取或证明。固定 Go 符号 errno 渲染及真实 Linux 有限故障分别报告，不证明所有 syscall 失败、完整启动序列或资源耗尽。新增显式管道分配还需证明：stdout→stderr→可选 stdin 的原失败阶段、父/子端及异步注册的所有权转移、未 spawn 分支的部分材料释放、准备观察与同次粗错误一致、原字段到 internal EOS 的连接及最后副本计费。实际 pipe2 失败与受控 Go 渲染分别验证，未证明完整启动故障优先级、Tokio/内核或整机预算。纯核心、历史已证明 Rust/Lean 未改，旧结论不覆盖新版准备与回复适配。

[PTY 准备错误](../sandbox-guest-runtime/src/pty.rs)新增原固定路径 open/解锁/窗口 ioctl 到互斥阶段和正 errno 的对应义务，包括避免 openpt 的 ENOSPC 归一化、失败时 master/slave 精确回收、原 owner 状态/粗错误核对，以及原 PTY 请求字段到有界 EOS 的连接。peer ioctl、复制/异步注册和 worker spawn 不借该事实分类；这些适配改动尚未抽取或证明，工程故障点与 Go 受控格式对照不能背书完整启动优先级或内核终端语义。纯核心、历史已证明 Rust/Lean 未改，旧结论不覆盖该新版运行适配。

[Start NUL 校验连接](../sandbox-guest-runtime/src/worker.rs)新增原请求 NUL 保留到可信 worker、工作目录检查先于 NUL、环境校验先于 argv/cwd 转换、READY 前互斥拒绝事实及原准备所有者实际收尾的对应义务。准备观察经共享锁和状态检查进入有界错误发送器，不从粗 InvalidArgument 推导事实；锁失败/未知清理不生成诊断。原生 errno 和校验失败保持分离，raw 入口的早期拒绝仍在。固定 Go Cmd.Start 的实际校验与有限 HTTP 渲染对照、Linux 候选回归均属工程证据；这些改动尚未抽取或证明，历史纯核心和 Lean 未改。

[Start 路径展开连接](../sandbox-guest-service/src/execution.rs)新增同一账户/初始化快照的原失败事实到 [Start 回复](../sandbox-guest-service/src/command_requests/http/start.rs) 的传递义务，以及三种观察互斥、与原组件错误匹配和旧粗错误接口保持的义务。纯组合、真实账户预检/拒绝顺序、未创建 worker/PID 及最后字节责任尚未抽取或证明；Go 原路径函数对照和 Linux 有限组件回归分别报告，不能代替完整 E03 或系统资源证明。纯核心、历史已证明 Rust/Lean 未改，旧结论不覆盖新增连接。

新增[命令所有权、执行期限与订阅](../sandbox-guest-service/src/command.rs)及 Linux 端口的独立控制句柄/失败排空路径尚未抽取或证明。#101 待证连接包括：初始订阅先于放行、授权到派发、原绝对期限与 pidfd、任务/请求取消的区分、未知结果不重执行、输出与终局顺序、慢订阅拒绝、实际持有块与字节/订阅/记录额度的共同释放、任务丢失后的未决责任和容量保留。只增加真实进程组件证据，未改变已证明控制器 Rust 或纯来宾核心，原抽取和定理不覆盖新异步编排；公开 PID/tag、持久 PID 身份和同会话终局保留已接入，仍是待证义务；终局缓存不跨服务重启持久化。

[准备对象](../sandbox-guest-runtime/src/process/preparation.rs)及[准备管理任务](../sandbox-guest-service/src/command/preparation/owner.rs)进一步接通取消、实际 wait 到原额度归还，以及原身份登记确认到执行所有权移交；只有组件测试，未抽取或证明。同步 spawn 到持有 child、借用握手取消、不可重复放行、期限/确认竞争、未知清理保留、任务丢失隔离和跨任务原额度移交都是新增自有义务，不能整体列为外部前提。Linux 进程句柄/wait、Tokio 取消和文件系统操作仍须逐项外部合同；测试中的正常目录回收不证明故障文件系统清理。原已证明 Rust 与纯核心未变，历史结论不覆盖这批编排。

在 `e2384707754a8e9d1e0a14cfe50c974169635aed` 之后，阻塞准备的确定失败改为通过返回结果保留原命令记录，直到异步父任务处理或丢弃结果；失败通知和记录一起由结果拥有，消费或丢弃时保留先归还记录、后报告 Stopped 的原顺序。原错误、取消和未知责任到结果所有权及最后释放的对应尚未抽取或证明。单线程屏障回归验证该交接窗口，不能推出所有任务或整服务内存上界；纯核心、Lean、抽取和工具版本未改。

[持久 PID 集](../sandbox-guest-runtime/src/pid_claims.rs)和[准备登记连接](../sandbox-guest-service/src/command/registration.rs)增加 VM 绑定/范围到固定图像、单写者锁、单调位图与真实 fsync、损坏/未知时拒绝、不可替换回执到原 gated child、阻塞任务取消和原容量/期限保持的自有义务，尚未抽取或证明。owner 已确认拒绝 PID 复用政策，不等于代码或端到端身份已经验证；目录来源/VM 绑定、首次制备、存储持久性和防有效快照回滚的外部边界仍须明确。新位图不恢复活跃进程，当前纯核心及已证明 Rust 未改，旧定理不覆盖这些文件操作和异步连接。

[内存 PID 登记](../sandbox-guest-service/src/command/registry.rs)增加准备前登记预算、VM/会话与实际持久回执匹配、装表先于 exec、独立交接所有者不因丢回复失去原句柄、真实结束时钟到保留期限、迟到读取拒绝和原条目删除的自有义务。未知记录继续占额，登记/进程/外部订阅的额度分离及元数据编码到实际驻留也未抽取或证明；该组件的每请求鉴权由外层负责。4aadbf6120ea11de4e0a0d99ec46b8e02517ec3a 基线后新增确定性 tag 预约/查询与复用清理：同一表锁下唯一预约、取消释放、未知责任阻止复用、原保留时刻、原子替换旧终局和通知旧任务退出、迟到删除不影响新记录及 tag 元数据额外驻留，均属未抽取/未证明的自有义务；owner 批准行为不构成证明。CommandStatus 新增真实完成时刻，异步命令 owner 已改，原纯控制核心与已证明 Rust 未改；测试不使旧定理自动覆盖新代码。

[异步命令响应体](../sandbox-guest-service/src/command/body.rs)增加原订阅到 Connect 事件/终结顺序、Bytes 最后引用到请求/订阅/输出/记录额度释放、原响应期限与执行期限分离、取消后原进程责任、编码/滞后失败不补成功终局的自有义务，尚未抽取或证明。[keepalive 时钟](../sandbox-guest-service/src/command_clock.rs)与原帧释放时间回执进一步改变上述适配：Start/Data 重置、保活不重置、延迟轮询和周期跳过、同一最后所有者释放边界到异步时钟的对应尚未证明；有限原时间库及假时钟回归不证明完整 HTTP 时序或库/调度器实时性。Linux 执行结果新增 core_dumped，结束文本使用固定 Go 信号名；真实 wait 到字段及 wire 编码的对应同样未证明。异步 driver 与运行端口已改，已证明纯核心未变，旧结论不覆盖这些适配与传输所有权。

在 b98a44c51dd1b0935553f92daa6c94f0fabd5756 基线后新增的 [keepalive 头选择](../sandbox-guest-service/src/command_keepalive.rs)尚未抽取或证明。首字段选择、字节/十进制 i64 解析、90 秒回退、显式 wrapping_mul 与 Go 有符号纳秒的对应，以及正周期/原构造器 panic 分类均是自有待证义务；有限 Go 对照不等于全输入等价证明。cf0e80a12181aac72ea017641e3e8e05bcefdd16 基线后，批准的有界正秒规则经 HTTP 选择、Ready、Start/Connect 分派接到原响应体；新增 parsed-i64 与乘法溢出的区分、效果前拒绝顺序、固定 invalid_argument EOS、原协商格式/期限/请求容量的保持、选定周期不可替换和覆盖组件显式值的自有义务，均未抽取或证明。瞬时时间可表示性、原帧释放到时钟、配置预算及完整运输资源界仍须分别验证。已证明控制器 Rust、纯来宾核心、抽取输入及 Lean 源码未改，原定理不覆盖新增组件。

[命令准备请求所有权](../sandbox-guest-service/src/command/preparation.rs)新增原请求期限到准备/登记接受的最早时刻、守卫到独立准备任务和持久登记票据、取消后真实收尾/阻塞任务退出、接受后移交至实际放行观察的连接义务。原授权/执行期限与后续进程记录责任保持，但准备 owner 和执行 driver 的本候选均未抽取或证明；真实进程有限检查不能证明服务重启、未知系统调用或完整 HTTP Start。已证明纯核心未改。

准备 owner 在 359ced1c8edfa474057f518eb8aaf07654edf76d 之后进一步将同步 spawn 材料制备移入阻塞任务。新增待证义务包括：原记录预留对排队/运行任务数量的约束、原请求/授权在等待者丢失时仍随闭包保留、出队后原期限及撤回检查先于副作用、返回的原 child 到异步握手的唯一移交、返回值未被消费或 panic 时的停止请求与未知责任隔离。屏障及假时钟回归不证明 Tokio/内核取消或实际 RAM 上界；该新版编排未抽取或证明，旧工程回执和历史定理不能背书它。纯核心、既有 LLBC/Lean 及已证明 Rust 均未改变。在 59b13e9fbce0078a9231ab84a472ec80c1193154 之后，cwd Stat 又从 worker 移到原阻塞准备任务、先于 gate/管道/PTY 分配。此顺序、同次 ENOENT 观察到公开错误的连接、阻塞期间责任保持及预检后路径变化到实际 chdir 错误的对应均属未证明适配；不把成功 Stat 当作访问授权或无竞态保证。

[共用 Connect 期限解析](../sandbox-guest-service/src/connect_timeout.rs)从文件请求组件提取，命令执行期限改为首值、空值及有符号整数的固定语义；同一非正 wire 值到 HTTP 立即到期和无执行期限的不同映射、原起点及派发前过期拒绝仍须连接证明。既有有限解析对照和新边界测试不证明完整命令 HTTP 生命周期；已证明纯核心未变。

[Start 连接](../sandbox-guest-service/src/command_requests/start.rs)新增已解码请求/原授权/已选账户不可替换、默认账户阻塞工作到原期限和容量、纯上下文组合到原 worker、首帧/元数据准入先于子进程、持久 claim 与活跃记录先于接受、回复丢失后只管理原进程的自有义务。账户/环境组合和响应预算校验共享重构没有新增证明；有限组件回归不覆盖完整 HTTP 选择/错误、重启及整机压力。在 85d487ec34fcf144999b1064140350a1f8f9d48d 之后，原 tag 到准备前原子预约、本次 AlreadyExists 到有界公开回复、字面选择器到唯一原记录、终局 Connect 与存活控制的区分、每请求鉴权/期限及最后副本所有权均新增未证明连接义务；数字字符串不得转换成 PID。准备、登记和执行端口仍属未证明适配，纯核心及已证明 Rust 未改。

在 592433e3e538869a4a388e29851769569d3bdecb 基线后，Start 连接增加原登记预约的条数/元数据额度拒绝到有界 resource_exhausted EOS 的映射；原事实来源、粗错误匹配、准备前无效果、原期限及最后响应副本占额均属新增未证明连接义务。该适配尚未抽取或证明，纯来宾核心、已证明 Rust 与 Lean 未变；有限组件和 SDK 回归不证明全局容量、任意未知错误或完整 KVM 行为。

[命令 HTTP 选择](../sandbox-guest-service/src/command_requests/http.rs)新增 Basic 预检到方法/媒体类型/压缩/版本/期限的顺序、第一字段与有界诊断、较早期限和协商格式保留、identity 解码前拒绝以及失败/Ready 到原容量的连接义务。命令和文件共用 MIME 选择主体，保留原文件 API；有限 Go 对照不证明完整协议、到期效果边界或公开错误映射。新增选择、共享解析重构及 SDK 路由迁移尚未抽取或证明，纯核心和已证明 Rust/Lean 未改。

[命令头错误回复](../sandbox-guest-service/src/command_requests/http/reply.rs)增加原选择观察到一元/流式固定错误的映射、原方法与 HEAD 的 405 空回复、协商 gzip 终结帧、原始和压缩载荷独立限额，以及编码失败/取消/最后字节引用到原请求容量和期限的连接义务。有限原 Go 字节对照及所有权测试不证明任意错误、完整命令协议或整机内存上界；这部分适配尚未抽取或证明，纯核心和已证明 Rust/Lean 未改。

[显式 Basic 账户回复](../sandbox-guest-service/src/command_requests/http/account.rs)新增有界缺失账户观察来源、原首值/方法不可替换、账户失败先于协议解释、外层 authn 与 handler 格式区分、MIME 规范化及动态头/正文引用到原容量的对应义务。共享 MIME 格式器仅开放 crate 内复用，新的 Connect 规范化及原预检/SDK 测试路由连接均未证明。有限 Go 原字节与本地所有权测试不涵盖 NSS 的任意故障、gRPC、默认账户、正式监听或整机上界；纯核心与已证明 Rust/Lean 未改。

[Start 默认账户回复](../sandbox-guest-service/src/command_requests/http/default_account.rs)新增原查询阶段与有界失败观察、Ready 到 Failure 的原格式保留、默认/显式账户响应区分，以及共享帧/压缩/有限发送器到原请求容量和期限的对应义务。缺失观察和未选格式不能产生公开用户名错误；有限 Go 对照不证明实际 gzip Start、系统账户或完整服务。新连接及共享错误编码重构尚未抽取或证明，纯核心和已证明 Rust/Lean 未改。

[正常命令回复](../sandbox-guest-service/src/command_requests/http/response.rs)新增 Ready 到执行结果/订阅的原格式与请求责任保留、原帧到压缩/分块副本的责任和反压、每帧预算而非累计限额、编码取消至实际退出、原期限及不假造正常 EOS 的对应义务。共用错误帧编码器扩展到数据帧及一元 payload，成功头/实际长度和 SDK 路由迁移也未抽取或证明；固定 Go 有限头/解压 JSON 对照不证明 gzip 位流相同、任意内存峰值或完整协议。纯核心和已证明 Rust/Lean 未改。

[命令 token 回复](../sandbox-guest-service/src/command_requests/http/authorization.rs)新增原鉴权分类/方法不可替换、外层 API 拒绝先于协议解释、HEAD 预算、响应失败及字节副本到原名额/期限的连接义务。复用原 token 有界发送器，未改变 Session 政策；固定 Go 有限 HTTP 对照不证明初始化、完整路由或任意故障。这部分连接和 SDK 测试入口迁移尚未抽取或证明，纯核心和已证明 Rust/Lean 未改。

[命令请求准入/PID/tag 分派](../sandbox-guest-service/src/command_requests.rs)增加原请求额度到 token/Basic/JSON 阻塞工作的所有权连接、取消/超时后实际闭包退出、单次解码和不可替换请求到原注册句柄、部分输入不重放、原响应期限与最后 Bytes 引用释放的自有义务，尚未抽取或证明。真实账户/进程适配沿用现有端口，不把其有限测试提升为 HTTP 路由、跨服务恢复、整机容量或 Go 双端证据；纯核心及已证明 Rust 未改，旧定理不覆盖新编排。

[共用命令 HTTP 派发](../sandbox-guest-service/src/command_http.rs)新增精确路径到普通/保留控制池的连接、拒绝池别名、原鉴权/预检/选择顺序、不可替换 Ready 到同一执行资源、克隆共用内部编号及耗尽不回绕的自有义务。构造增加原持久存储不可变绑定与注册表/VM 的一致性、进程/准备预算及 worker 路径语法校验义务；配置快照不能代替当前文件完整性或实际 claim，原 claim 校验仍保留。唯一 Start 装配、可信 VM/会话及存储来源仍是调用前提，内部编号不代替永久 PID 集；共享账户/运行时也不能由控制请求名额独立推出控制延迟保证。原错误/期限/容量经各发送器保持、取消和回复失败不重放，以及 SDK 夹具迁移均尚未抽取或证明。在 85d487ec34fcf144999b1064140350a1f8f9d48d 之后，公开 HTTP 构造及 daemon 静态准备精确约束 30 秒保留、拒绝发生在 MMDS/PID 工作前的对应亦未证明。纯核心和已证明 Rust/Lean 未改，有限 Linux/SDK 检查不证明完整服务或整机压力上界。

在 `0357f4656fc090b14cc68659cb4b0d3a0df635af` 之后，正式 daemon 的控制阻塞执行器增加了总线程/栈额度守恒、原共享账户准入保持、单阻塞线程兼容、精确路由到任务作用域、Ready 跨任务恢复执行器，以及两个运行时共用退出等待预算的对应义务。新作用域只选择本 crate 的阻塞提交，不自动继承到异步后台任务；每个控制准备与一元回复路径是否完整使用该执行器仍须自有验证。任务计数、账户池及真实命令 HTTP 屏障回归属于工程证据，不证明一般调度活性、NSS/内核调用可中断、整机资源上界或 KVM 清理；纯核心、Lean、抽取与工具版本未改，旧证明不覆盖此实现。

[PID/tag 操作拒绝](../sandbox-guest-service/src/command_requests/http/operation.rs)新增原选择器、实际表查询结果及参数检查顺序到有限错误观察的对应；原终局保留与存活控制的区别、未知不伪装 NotFound、超时不重用旧观察和派发失败不重放也须证明。已观察事实到固定类型/数值/枚举文本、Go JSON 转义、一元/流式错误格式、共用有限发送器和原错误/期限/容量到最后副本均尚未抽取或证明。受控原 Go 格式对照不执行原 getProcess/handleInput/SendSignal，真实 Linux 与 SDK 回归也不能代替完整 E03、任意并发或整机内存证明；纯核心和已证明 Rust/Lean 未改。

新增[通道失败观察](../sandbox-guest-service/src/command_requests/dispatch.rs)连接原句柄不可变 PTY 能力、同次实际拒绝与四种固定公开错误。能力不等于存活、模式正确的 ioctl/输入失败不误分类、原操作只执行一次、旧组件错误保持不变，以及一元格式/期限/预算/最后副本容量的对应尚未抽取或证明。有限 Go 受控格式、真实 Linux 命令和固定 SDK 用例不证明完整原 handler、并发、终端行为或 E03；纯核心和已证明 Rust/Lean 未改。

原独占输入锁下的[stdin 关闭观察](../sandbox-guest-service/src/command.rs)另增加启动时禁用标记到实际无句柄拒绝、缺失端口与同次失败的对应、关闭端口和真实 EPIPE 区分、完成竞争不误归因、旧粗粒度错误保持，以及有限事实跨 HTTP 发送的所有权义务。空写入、重复关闭、取消和资源失败须分别处理；这些新连接尚未抽取或证明，组件/SDK/受控 Go 格式对照不替代完整原服务对照。

[原生输入失败连接](../sandbox-guest-service/src/command.rs)新增同一实际写入的正 errno、原错误/身份/通道到 [HTTP 回复](../sandbox-guest-service/src/command_requests/http/operation.rs) 的自有对应义务。旧粗错误保持、模式事实不覆盖原生事实、部分效果不重试，以及合成挂断/取消/完成不伪造 syscall 失败均未抽取或证明。固定 Go 错误格式、真实 os.Pipe 和 Linux 有限 stdin/PTY 组件分别报告，不证明所有输入故障、调度竞态或完整 E03。历史已证明 Rust、纯核心与 Lean 未改。

[原生信号失败连接](../sandbox-guest-service/src/command.rs)新增原 pidfd syscall 的正 errno 到 [固定错误回复](../sandbox-guest-service/src/command_requests/http/operation.rs) 的对应义务，包含 ESRCH 到 Go 进程已结束错误、原粗结果/期限核对及无再次查找或发送。新增 TERM/KILL 及执行期限先解除输出等待、真实主进程 wait 后关闭剩余输出的适配路径；它绕过普通 EOF 收尾而不伪造纯核心 EOF，旧 CommandStream 定理不覆盖此路径。信号失败仍触发取消、既有错误不被成功覆盖、已排队字节先于 End、登记终局/容量与真实收尾的对应均待证。[PTY 共享描述符](../sandbox-guest-runtime/src/pty.rs)的撤销、等待者唤醒、在途 FD 最后引用及 close/ioctl 竞争同样未抽取或证明。固定 Go 源码顺序核对及有限 pidfd/PTY 回归不代替 PID 复用、完整 Go 信号/输出调度或系统收尾证明。纯核心、历史已证明 Rust 和 Lean 未改，旧证明不能背书新版运行适配。

[原生窗口调整失败连接](../sandbox-guest-service/src/command.rs)新增同一终端 ioctl 正 errno、原粗错误和 [HTTP 回复](../sandbox-guest-service/src/command_requests/http/operation.rs) 的对应义务。实际 EINVAL 不被无 PTY 分类覆盖、挂断写入不推断 resize 失败、uint16 转换及无重试/新查询均未抽取或证明；有限真实挂断场景和受控 Go 格式对照不证明全部 ioctl 故障、调度或系统收尾。纯核心、历史已证明 Rust 与 Lean 未改。

[来宾 HTTP 装配](../sandbox-guest-service/src/guest_http.rs)新增三个处理器共享同一认证状态的对象身份检查、路径到原处理器的唯一分派、原请求/控制池与回复所有权跨装配保持、初始化和轮换共同作用于各接口的对应义务。既有有界监听器到实际命令/文件处理器的连接、错误责任至连接结束、运输取消不推断进程停止也尚未抽取或证明；构造检查不证明调用方账户/VM/PID 配置可信。组件回环与静态检查不证明正式 daemon、任意并发、实时性或整机内存，纯核心与已证明 Rust/Lean 未改。

[命令空消息语义](../sandbox-guest-runtime/src/protocol.rs)修正完整零长度标准流帧到默认 protobuf 请求的连接，不放宽一元空 JSON、原输入预算或截断拒绝。[可选 EOS 校验](../sandbox-guest-runtime/src/protocol/end_stream.rs)另增加首消息优先、完整终结帧/EOF、独立载荷预算、encoding/json 字段/null/重复/折叠语义、Unicode 替换及有界副本、已知形状与 base64 校验而不应用客户端错误的对应义务；不保留任意元数据树的空间界和未知深度遍历也尚未证明。有限原 Go server-stream 对照不覆盖压缩、公开错误全文或完整协议证明；纯核心和已证明 Rust/Lean 未改。

[帧失败观察与回复](../sandbox-guest-runtime/src/protocol/request_body.rs)新增原读取/解压/JSON 顺序到有限错误事实、消息数量与空标准消息/EOS 的区别、第二条标准消息解析前释放第一条字段、低层兼容错误与 HTTP 详细分类分离的对应义务。固定 scalar 到 Connect code/完整文本、原响应格式/期限和最后副本容量的连接尚未抽取或证明；7,568 组固定 Go 完整错误字节对照只覆盖选定帧错误，不覆盖任意 JSON/运输错误、实时性或整机内存。新增共用 HTTP 连接及共享解码重构不沿用旧定理背书，纯核心和已证明 Rust/Lean 未改。

[命令 JSON 诊断与回复](../sandbox-guest-runtime/src/file_protocol/diagnosis/command.rs)新增共享词法器 peek 缓存、字段/列表/map/oneof 顺序、原位置/片段到 process 类型诊断、双解析器分歧不误报、跨命名空间拒绝的自有义务。有限已知递归、未知深度栈、map 键集合、bytes 校验临时空间及错误副本的界，原解码责任/期限到回复失败和最后副本均尚未抽取或证明。固定 Go 完整 HTTP 字节对照和 Linux 入口测试只覆盖选定输入，不证明完整协议、实时性或整机内存；文件共享词法及格式重构也不能沿用旧定理。纯核心和已证明 Rust/Lean 未改。

[EOS 错误诊断](../sandbox-guest-runtime/src/protocol/end_stream/diagnosis.rs)增加原全载荷语法扫描、普通类型错误保留与自定义 error/detail 失败优先级、有限 schema/折叠字段、base64 原始位置及固定 Go 字符引号到公开文本的对应义务。迭代栈、借用未知值、规范化及临时字段/base64 字符串空间，原 framing/解压顺序和未映射分歧到同一请求责任/期限的连接尚未抽取或证明。有限 Go 全文对照与 Linux 入口回归不证明完整 encoding/json、协议、实时性或整机内存；纯核心和已证明 Rust/Lean 未改。

[命令请求 gzip](../sandbox-guest-runtime/src/protocol/request_body.rs)增加原协商到逐帧解压、wire-empty 与展开为空的区分、首消息/尾帧错误顺序、原始与展开预算、EOS 规范化及已解码请求同时驻留的对应义务。命令和文件抽取为同一[有界解压器](../sandbox-guest-service/src/request_gzip.rs)，各自错误分类、原授权/取消/绝对期限、实际 worker 退出及请求容量保留仍须分别证明；可信 expander 接口本身不提供分配上界证明。有限固定 Go 接受性/值对照不证明公开错误全文或整套协议，新增 Selected::decode 与 SDK 迁移尚未抽取或证明。后续[已知 gzip 回复](../sandbox-guest-service/src/command_requests/http/reply.rs)复用文件错误观察，增加实际阶段到固定文本、一元 identity/流式协商 EOS、未知故障不误映射、原错误/期限/容量到发送失败和最后副本的对应义务；有限完整错误字节测试不证明全部 codec 错误或正式端点。纯核心和已证明 Rust/Lean 未改。

[命令请求 JSON](../sandbox-guest-runtime/src/protocol/request_json.rs)增加丢弃未知字段前的 Unicode/容器校验、null 与重复字段/oneof 的区分、环境键唯一性、精确 i32/uint32 归一化、未知枚举到实际拒绝、原始 bytes 长度/字母表到 padding 及尾位解码的自有义务。共用扫描器为命令选择非整数哨兵，文件选择原负整数哨兵；各已知类型拒绝与未知字段丢弃、原字节预算/按需副本上界的对应未抽取或证明。固定 Go Q 只比较有限解码结果/分类，不证明错误全文、鉴权或执行；纯核心与已证明 Rust 未改，旧结论不覆盖新解析逻辑。

来宾 [PTY 内核端口](../sandbox-guest-runtime/src/pty.rs)、worker session/controlling terminal 设置、原身份 resize 和双输入通道连接只有工程测试，尚未抽取或证明。原纯来宾核心的 `OutputMode::Pty` 未改，但旧抽取不能证明新 master/slave FD 所有权、异步就绪与挂断判定、EIO/EOF 到真实 wait 的连接、取消部分效果或数值身份切换顺序。初始/更新尺寸的固定 uint16 转换和实际终端 line discipline 也须在 #101 区分自有判定与外部内核前提；组件及固定 SDK 回调通过不代表正式来宾服务或系统证明通过。

文件[一元编解码](../sandbox-guest-runtime/src/file_protocol.rs)及[授权路径解析](../sandbox-guest-service/src/files.rs)尚未抽取或证明。共享 JSON 解析/有界输出和 Basic/路径辅助函数供命令、文件两端复用，工程回归核对原命令行为；纯来宾核心未修改。字段缺省/精度、编码预算、账号选择、路径展开、授权快照到实际 I/O 的连接均须在 #101 后续证明；属性编码现改为原默认 protobuf JSON 的标量零值/空集合省略，同时保留 optional epoch/空链接和元数据空值，历史合成属性结果不能背书该新编码。原始生成消息和默认 Connect 的有限 HTTP 对照只验证选定属性，零值判定、有界序列化及存在性连接仍未抽取或证明。[文件请求 JSON](../sandbox-guest-runtime/src/file_protocol/request_json.rs)另增加未知字段丢弃前的字符串/深度校验、重复已知字段拒绝、借用数值词法到精确 uint32 的对应、扫描/解码临时空间与原输入预算的连接义务；尚未抽取或证明，有限 Go 请求对照只比较指定解码值与拒绝分类，裸指数词法到五种文件消息的等长兼容转换、按需正文副本、每段数字只扫描一次和原位置保持也是新增未证明义务；[JSON 诊断](../sandbox-guest-runtime/src/file_protocol/diagnosis.rs)及[拒绝编码](../sandbox-guest-service/src/file_rpc_response/rejection/json.rs)另增加原 token/字段顺序到首个失败、UTF-8/UTF-16/行列及原片段、双解码器分歧不误报、Debug 隐私、构建相关前缀及 Go 引号/JSON 转义到完整文本的自有义务；有界工作区、诊断副本、原错误/正文副本的容量及期限连接尚未抽取或证明。固定 SDK 与合成属性通过不证明真实文件操作、DAC、整个服务或系统正确。

新增[真实一元文件端口](../sandbox-guest-runtime/src/filesystem/mod.rs)、账户显示反查及[阻塞文件所有权](../sandbox-guest-service/src/file_io.rs)只有工程回归与固定参考对照，尚未抽取或证明。链接解析与实际 inode/FD、Go 属性对应、遍历/暂存/结果预算、非原子副作用与错误、原授权到派发/完成、取消后容量及结果保留均是新增自有义务，不整体列为外部前提。一元请求编排新增准入到账户/路径/I/O 的同一责任、较早绝对期限、阻塞队列到期检查和返回结果/分类错误的保留义务；账户阻塞取消后原授权仍须有效，已经开始的操作不能因等待超时被解释为未发生。[一元响应](../sandbox-guest-service/src/file_rpc_response.rs)另增加有界 JSON 扩容、原结果到正文/副本/编码失败对象的同一容量、编码 worker 取消收尾和原有效期限的对应义务；与上传共用的 payload 分块源仍未证明。共享纯协议可移植测试及有限 Connect 传输对照不证明完整 protobuf/SDK/服务等价。[一元准入](../sandbox-guest-service/src/file_rpc_requests.rs)新增全头预算、鉴权锁等待、完整 framing 到有界收集、阻塞解码取消/到期、解析结果到文件责任及 Connect 毫秒期限的对应义务；纯 Go 解析投影不证明整个协议实现。[Basic 前置查询](../sandbox-guest-service/src/file_io/rpc/preflight.rs)新增显式/缺省查询顺序、同一账户与设置不可替换、文件槽位提前持有并贯通解码、取消查询保留授权，以及 Basic 临时解码缓冲清零的自有义务；原 Go BasicAuth 有限向量不证明 authn/账户/HTTP 中间件等价。[账户拒绝响应](../sandbox-guest-service/src/file_rpc_response/rejection.rs)另增加实际缺失观察到正确阶段/文本的对应、默认账户不重解析 Basic、与原成功响应共用有界编码及失败/字节副本所有权的义务；Go authn/Connect 原库 HTTP 对照只覆盖指定 JSON 失败，不证明账户来源或整套认证。[一元版本/超时拒绝](../sandbox-guest-service/src/file_rpc_requests/tests/headers.rs)新增第一字段选择、有效期限先收紧、版本错误优先于超时语法错误、Go 引号与 JSON 转义组合、有界诊断到原响应责任的自有义务；指定 HTTP 对照和 Linux 静态检查不是完整协议证明。前序账户/文件及头拒绝连接已有 #408/#409 Linux CI 证据。[共享一元 HTTP 编排](../sandbox-guest-service/src/file_rpc_http.rs)新增精确路径到唯一操作、准备/执行的不可替换交接、协议选择与 Basic 查询的拒绝顺序、共享池及返回失败/字节副本的责任、JSON charset 到实际回复头的对应义务；MIME 类型小写修正和有限原库对照不是完整解析或协议证明。新增编排及 SDK 迁移已有测试清单所列精确提交的 Linux CI 证据；后续候选分别验证，这些实现均尚无证明。系统调用、目录变更、xattr、线程池和序列化的实际连接另须最小外部合同。纯来宾核心和历史已证明 Rust 未改，旧定理不覆盖这些新增实现；组件对照也不证明 POSIX/ACL 隔离或整套部署系统。 Stat/MakeDir/Move 新增[失败观察](../sandbox-guest-runtime/src/filesystem/failure.rs)与[固定错误编码](../sandbox-guest-service/src/file_rpc_response/io.rs)：原 syscall 阶段/errno/路径到公开文本的对应、最多双路径预算、阻塞结果到原槽位和较早期限的保留、已发生部分效果不重放均是新增自有义务，尚未抽取或证明；受控 errno 格式向量与未执行的原 Go 方法对照不证明权限、磁盘满或所有文件竞争。ListDir 新增原组件路径解析/中间预算、循环诊断优先级、根 lstat 及条目消失/失败顺序、实际失败与已解析根的双路径对应、遍历错误丢弃已有结果、带角色上下文到有界错误编码的义务；指定文件替换交错与有限格式向量不证明全部文件系统并发，也不将这些自有逻辑移入外部假设。Remove 新增原失败阶段/errno、首次递归错误、读失败/最终删除优先级、原始路径逐层拼接和嵌套读取名称对应的义务；活动帧诊断预算、超限保留部分效果、固定 inode 重读与响应所有权连接均未抽取或证明。可移植格式测试及受控 Linux 故障点不证明完整 Go 竞争等价，旧定理不能背书新增删除实现。[一元 gzip 成功响应](../sandbox-guest-service/src/file_rpc_response/compression.rs)另新增精确第一字段协商到不可替换 Ready、原 JSON/压缩双预算、编码工作区到实际峰值、原 worker 取消/期限及字节副本到容量释放的义务；错误保持 identity、失败后不重放副作用和最终压缩长度到 HTTP 头的连接均未抽取或证明。有限原库 HTTP 对照不证明完整协议或压缩算法等价。[一元请求 gzip](../sandbox-guest-service/src/file_rpc_requests/compression.rs)新增原始 framing 完成到解压再到 JSON/派发的顺序、双载荷与块/工作区预算、空输入和拼接成员的完成判断、Header/Body 故障事实、实际读取/输出前取消及排队 worker 责任保留的义务；gzip 请求到响应编码的协商和 SDK 构造选项到实际压缩流的连接尚未抽取或证明。有限原库对照和合成客户端探针不证明完整端点等价。[一元 gzip 错误响应](../sandbox-guest-service/src/file_rpc_response/rejection/compression.rs)另增加真实 Header/Body 故障到受控公开诊断、零正文及未知/预算错误不误映射、同一原期限/容量到编码失败与最后副本释放的义务；有限原 Connect HTTP 错误对照和原 SDK 错误解析器探针不证明完整 gzip、文件端点或协议等价，新增映射尚未抽取或证明。[未知请求压缩拒绝](../sandbox-guest-service/src/file_rpc_requests/selection.rs)另有完整头预算到受控诊断副本、第一值与不规范化规则、有效超时先收紧及压缩/版本/超时/账户错误顺序、原字节到 Go 引号/JSON 和固定 501 的自有对应义务；迁移到 decode 后的原槽位/期限连接未抽取或证明，有限 HTTP 向量不证明其他协议或整个 handler 等价。一元 PathExpansion 接线另新增同次账户/默认上下文到准确诊断、Move 双路径先于文件派发、预算失败不误映射、原用户名到有界 Connect JSON 及原槽位/期限的义务；固定 permissions/Connect 的有限 HTTP 对照不证明账户来源、完整协议或整个服务相容，这些新增实现尚未抽取或证明。

新增[流式文件 I/O](../sandbox-guest-runtime/src/filesystem/transfer.rs)和[传输所有权](../sandbox-guest-service/src/file_transfer.rs)已有单文件 Linux 组件及原 SDK 测试路由检查，未抽取或证明。原 `Transfer` 的输入/实际写入计费被接到真实 FD，但纯来宾核心源码未改，原抽取试验不覆盖这个连接。待证义务包括短写量到真实副作用、完整输入/元数据到成功、取消/期限与晚到回复、块释放到背压/容量、原授权到实际阻塞工作结束，以及打开后 FD 与可变路径的区别。xattr、内核读写、异步通道/取消和内存分配须逐项最小外部合同；历史控制器定理不能背书新增适配或整个 HTTP 服务。

[HTTP 方法拒绝](../sandbox-guest-service/src/file_rpc_response/method.rs)新增显式账户前置工作到非 POST 判定、协议/正文拒绝顺序、空回复与 HEAD 头差异、原授权释放及双槽位/期限保留的连接义务，未抽取或证明。固定 Go HTTP 对照只检查有限方法与头组合，不背书正式 HTTP 服务器、其他媒体类型或完整服务；历史纯核心/文件编码证明不覆盖该分支。

[一元令牌拒绝回执](../sandbox-guest-service/src/file_rpc_response/token.rs)新增实际 Session 拒绝到外层数字 401 JSON、HEAD 完整预算/正文抑制、原请求槽位/期限与最后字节副本及失败对象的连接义务。令牌接受谓词、受保护初始化及纯核心不变；重复头/初始化前严格拒绝不能用原 Go 已初始化首字段对照背书。回执和 Linux/SDK 组合尚未抽取或证明，不把固定消息或协议测试当作全鉴权证明。

[正式来宾入口](../sandbox-guest-service/src/daemon.rs)新增配置字节上限到原组件预算、必需/未知/重复字段拒绝、当前有效 UID 与私有输入、独立普通/控制池、同一 VM/服务编号与 Session 到全部路由、一次启动期限、先装配后监听和传输停机边界的自有义务；[启动文件端口](../sandbox-guest-runtime/src/startup_files.rs)另新增原目录 FD 遍历、祖先/最终对象权限与身份、NOFOLLOW/NONBLOCK 和类型拒绝义务。入口及新增 Serde 解码尚未抽取或证明，构造/静态检查不证明机器资源足够、实际模板安装或全部后台工作已停止。Rustix/Linux、文件锁/持久存储、serde_json、MMDS 来源、MIME/TZif 材料、Tokio/Hyper 与可信 root/owner 仍须分别建立最小外部合同。线程栈乘法界不等同总 RSS 上界，worker 路径检查不等同二进制摘要见证。原已证明 Rust、LLBC/Lean 未改，旧定理不覆盖本次配置、文件和异步装配。

[持久服务会话](../sandbox-guest-runtime/src/pid_claims/session.rs)将原 PID 图像升级为 v2，并复用原写入/校验/同步/毒化路径。新增有界非零编号、跨重开单调性、未知或丢失结果不回收、耗尽不回绕、会话与 PID 共享毒化，以及旧格式拒绝而不重建的自有义务；上述新版 runtime 和启动分配连接尚未抽取或证明，旧位图测试或模型不背书新增计数字段。fsync、文件锁、存储不回滚和等待者取消仍为对应外部前提；软件故障点只提供测试证据。未修改原已证明 Rust 或 LLBC/Lean。

[worker 指纹](../sandbox-guest-runtime/src/artifact.rs)及配置 v2 增加期望版本/摘要的受信任来源、同一 FD 的固定缓冲/精确长度与摘要核对、原期限及先核对后读取 MMDS/状态的连接义务。版本常量复用到原 PID 格式头与 worker 握手断言，持久格式/握手字节未变；版本报告只标识编译接口，不证明源码出处或已安装字节。上述新增 runtime/service 代码尚未抽取或证明；SHA-256、文件系统观察、可信安装在服务生命周期内不变及随后按路径 exec 的关系仍是显式外部前提。有限增长/截断/取消测试不证明任意并发文件快照，旧纯核心及历史 LLBC/Lean 不背书该启动校验。

[来宾产物准备](../tools/prepare_guest.py)及[清单消费](../tools/guest_artifacts.py)增加精确 Git 归档/工具配方到实际构建、原输入锁到双二进制版本、同一文件观察/复制/同步到摘要、外部受保护 manifest pin 到后续消费，以及失败/超期/发布不确定的连接义务。实现和有限工具测试不构成源码到机器码对应证明，也不验证编译器、链接器、构建镜像、Python/Git、文件系统、原生库或完整内存/磁盘峰值。新工具尚未在真实 Linux 运行原 release 配方或连接受保护模板安装；构建替身不背书这些未执行项。未修改已证明 Rust、LLBC/Lean，旧定理不覆盖新 Python 适配或其模板组合。


[模板 Rust 安装连接](../tools/template_guest.py)新增独立 profile pin 到安装副本、实际 daemon check 到再次指纹校验、已消费的新 VM 标记到只执行一次 initialize，以及候选状态/原暂存恢复到同一 TemplateBinding.guest 的连接义务。普通文件副本、实际 shell 控制流与故障替身测试不构成真实 daemon/BusyBox/文件系统持久化、镜像 ABI 或 KVM 证明。重命名及 sync 后不得因缺失 PID 文件重新制备是自有控制要求；内核/文件系统、可信新镜像、私有运行盘、MMDS 身份不变及来宾 root/旧盘回滚排除仍为外部前提。未修改已证明 Rust、抽取或 LLBC/Lean；这些 Python/boot 连接尚未证明。 [自助监督接线](../tools/template_build_guest.py)另增加受保护 profile 到原 SQLite 配置摘要、完整源码审计到运行材料子集、同一上传 inode/期限到候选拒绝、原候选恢复到相同 guest 身份的连接义务。SDK 异步取消/连接释放、普通文件读取线程及实际挂载/来宾启动仍属外部实现依赖；本地组件和回环实测不构成证明，也未修改已证明 Rust 或继承其结论。

[正常启动状态装配](../sandbox-guest-service/src/startup.rs)新增可信 VM/模板 ID 的有界校验、域分离与长度分帧摘要、同绑定正常 open、未初始化 Session 与元数据身份钉住、令牌轮换不更换 PID 绑定，以及启动绝对期限/单名额/阻塞结果析构顺序义务；均尚未抽取或证明。元数据来源真实性和 VM ID 不复用、SHA-256、目录来源、原 PID 文件与 fsync、Tokio 排队/取消为显式外部连接前提。有限临时文件/故障点回归不证明服务已部署、有效旧盘回滚防护、启动可中断或整机内存上界。未修改已证明 Rust、LLBC/Lean，也不扩大原定理覆盖。

[有界 HTTP/1 监听](../sandbox-guest-service/src/listener.rs)新增接受连接到任务准入、解析头部前的容量限制、完成回收、原连接期限跨 keep-alive/正文/响应保持、关闭传输与实际工作责任分离、单连接异常隔离及停机排空的自有义务，尚未抽取或证明。Hyper/Tokio 的解析、计时、取消和实际 socket 行为仍须最小外部合同；缓冲参数、回环测试和连接峰值计数不证明整个服务的内存上界或部署正确。纯核心及历史已证明 Rust 未改，旧定理不覆盖此监听组件。

监听配置校验另增加固定 Hyper 两组头解析数组的计数乘法与合计字节可表示性、原字节缓冲上界检查；该自有拒绝逻辑及其与固定解析器存储布局的对应尚未抽取或证明。构造回归不执行极值分配，不证明分配可成功、全连接总量或整机 RSS。原已证明 Rust、LLBC 与 Lean 未改。

#552 对上述未证明监听器增加正文提前放弃观察、响应禁止复用，以及写入结束后的有界原始输入清理。原 `buffer_bytes`/`header_timeout` 和绝对连接期限共同约束收尾，原连接名额持有到清理结束；这些是新增自有实现义务，不能仅靠信任 Hyper/Tokio 宣称成立。确定性连接测试和固定 SDK 经原生产网关处理器、真实 stdio relay、原 CommandHttp 的回归属于工程证据；受控授权来源和命名空间放置、未实跑的完整 VM/TLS/外部代理，以及任意网络故障分别保留为缺口。此次不修改已证明纯核心、锁文件、抽取物、Lean 或 Charon/Aeneas；原基线证明不覆盖新增收尾逻辑。


[顺序批量编排](../sandbox-guest-service/src/file_transfer/batch.rs)和一次账户/defaults 快照解析另增加请求级授权、容量及期限共享、排他 part 借用、路径判重、累计正文/结果预算、失败后不可继续和正常结束证据的自有连接。实现已有 Linux 批量回归，尚未证明；单文件测试或原纯核心结论不覆盖该组合，HTTP/MIME 解析及正式服务仍是后续义务。

[增量 multipart/MIME 解码](../sandbox-guest-runtime/src/multipart/mod.rs)已有本地可移植测试和固定 Go 标准库对照，未抽取或证明。新增义务包括跨块边界与字节前缀、原始/解码计费、UTF-8/续段参数、错误后不可复用以及有效结束到批次成功的连接。明确拒绝 Go 可误作 EOF 成功的 MIME 头截断，不能沿用纯 `Transfer` 或历史控制器定理覆盖解析器。其与 HTTP 传输、账户授权、实际文件副作用及请求总预算的组合见下项，尚未证明。

[multipart 到上传批次的连接](../sandbox-guest-service/src/file_transfer/batch/multipart.rs)已实现，真实文件/固定 SDK 组合已有原提交 Linux 组件证据，后续候选仍须重跑。它增加原授权/容量/期限跨 parser、HTTP frame 和实际 I/O 的所有权链；End 与 HTTP EOF/长度的联合判据、QP 错误前缀、未知字段/尾部预算以及取消后的资源保留均是新的自有义务。该组合未抽取或证明，之前解码器/单文件/批次测试和 `Transfer` 抽取不能代为背书；HTTP 库的先行分配和响应序列化/发送仍须最小外部合同及独立计费。

[gzip 编解码组件](../sandbox-guest-runtime/src/gzip/mod.rs)新增头部预检、DEFLATE/CRC32 外部库连接、逐段结束与真实源 EOF、编码/展开预算及失败前缀义务。已有本地组件和固定 Go 对照，现已连接下述 HTTP/文件批次，未抽取或证明；`flate2`/`zlib-rs`/`crc32fast` 及调用者缓冲/期限/授权所有权仍须外部合同和组合连接。原纯来宾核心及历史已证明 Rust 未改变，旧定理不覆盖本模块或依赖实现。

[HTTP gzip worker](../sandbox-guest-service/src/upload_body/mod.rs)与[文件连接](../sandbox-guest-service/src/file_transfer/batch/gzip.rs)复用原批次，新增压缩头到文件打开的先后关系、压缩长度/展开预算、阻塞解压/实际文件两个任务的原授权守卫、Bytes 最后副本释放到下一读取、取消后的实际退出以及 raw/multipart 不同 metadata 完成点。共享 HTTP 帧校验重构和该所有权连接均未抽取/证明；可移植测试已有本地运行，Linux 授权/文件和合成压缩 POST＋固定 SDK 回读待验证，不能用组件测试或纯 `Transfer` 定理代替组合义务。

[gzip 下载连接](../sandbox-guest-service/src/file_transfer/gzip_download.rs)和[阻塞编码连接](../sandbox-guest-service/src/upload_body/encode.rs)增加同一原授权/容量到实际 open/read/销毁、实际 EOF 而非初始长度、源缓冲与编解码预算、响应 Bytes 最后副本释放及错误不伪装成功的义务。与上传共用的 worker/取消重构及 FD 打开辅助也未抽取或证明。原纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不背书这些连接；可移植测试与 Linux 交叉检查不等于 Linux 组合或系统证明通过。

[内容编码选择](../sandbox-guest-runtime/src/file_encoding/mod.rs)和[HTTP 字段连接](../sandbox-guest-service/src/file_encoding.rs)增加原字段字节/重复值、Go 浮点及排序、预算分配和条件回退/Vary 阶段义务，尚未抽取或证明。固定 Go 函数的有限对照是工程证据，不证明解析器对全部输入等价，也不证明请求鉴权/stat/open 顺序或完整 HTTP 服务。纯来宾核心、历史 Rust 与 LLBC/Lean 未改；所移植 Go 排序/浮点辅助和 Rust 十进制转换仍需对应及最小库合同。

[下载条件/范围](../sandbox-guest-runtime/src/file_download/mod.rs)与[响应计划](../sandbox-guest-service/src/file_download.rs)增加日期/时区转换、条件优先级、区间整数边界、MIME 线长、头变换和阶段执行义务，尚未抽取或证明。固定 Go 日期/ServeContent 的有限原函数对照只验证所列样例；计划中的文件区间不是已完成读取的证据，seek/read 错误、原授权/容量及响应发送的组合另由执行器承担，仍待完整验收。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不背书该计划器或第三方 `time` 库。

[本地时区快照](../sandbox-guest-runtime/src/file_download/local_zone.rs)新增输入/块计数预检、不可变解析结果、历史首类型/同名缩写选择、忽略闰秒修正及 POSIX 延伸查询义务。第三方 `tz-rs 0.7.3` 的 TZif/规则解析与内存分配是新增外部实现边界，未抽取或证明；其更严格材料接受范围、真实时区来源和读取预算须在 daemon 装配时继续复核。固定 Go 原加载/查询/HTTP 日期的有限对照属于测试证据，不是任意时区文件等价或形式化证明。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不背书该快照或新依赖。

[时区文件读取](../sandbox-guest-runtime/src/file_download/local_zone/file.rs)新增打开前配置/路径预算、普通 FD 来源、有限读取/EOF 探测、部分失败不发布与快照独立性义务；Unix open/fstat/read、内存分配、可信路径和模板来源为外部边界。原 `Limits` 校验提取为共享方法，原字节入口拒绝条件保持；适配器与该提取均未抽取/证明。有限文件和故障点测试不能证明并发改写一致性、系统调用可中断、来源真实性或总 RSS；旧定理不背书本实现。

[范围正文执行器](../sandbox-guest-service/src/file_download/stream.rs)、[Linux FD 连接](../sandbox-guest-service/src/file_transfer/identity_download.rs)及[共用阻塞正文任务](../sandbox-guest-service/src/owned_body.rs)新增条件提前结束、准备期/正文期 seek、按计划读取与短读错误、MIME 分段、HEAD 抑制、原期限及最后字节副本到容量释放的义务。共用任务从 gzip 所有权代码抽出，旧路径回归通过仅属工程证据；Linux mtime/同一 FD/原 AuthPermit 组合尚待实跑。这些模块未抽取或证明，不能用纯 `Transfer` 或原历史版本定理覆盖新增执行连接；纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改。

[下载 MIME](../sandbox-guest-runtime/src/file_mime/mod.rs)新增不可变数据库来源/优先级、Unicode 查表、原始 filename 编码、512 字节签名顺序及配置计费义务；[identity 执行](../sandbox-guest-service/src/file_download/stream.rs)新增条件先于嗅探、忽略读取错误但保留前缀、回绕失败与阻塞取消期间原守卫保留的组合义务。有限固定 Go 对照及本地组件通过只属工程证据，Linux 固定路径/原 FD 和模板来源验证尚待实跑。这些模块未抽取或证明；现有 multipart 参数解析仅开放 crate 内复用，算法未改，纯来宾核心、历史已证明 Rust 与 LLBC/Lean 均未改，旧定理不覆盖新增路径。

[协商下载](../sandbox-guest-service/src/file_download/stream/negotiated.rs)与[Linux 延迟打开](../sandbox-guest-runtime/src/filesystem/transfer.rs)新增 stat/选择/open 拒绝顺序、旧 metadata 与后续路径变化、单 worker 身份及期限、Vary 阶段、准备期错误响应和 gzip HEAD 消费/发送时机义务。可移植有限故障对照不代表真实 Linux 或完整端点验收；这些新增及重构代码均未抽取或证明。纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新的文件连接。

[下载账户准入](../sandbox-guest-service/src/file_transfer/download.rs)新增账户查询前预留文件槽位、原授权/期限跨阻塞查询与下载 worker 移交、取消后真实退出才释放、账户耗时不退还和原查询路径到 MIME 选择的对应义务。账户/路径失败仍为显式组件错误；Linux 新连接与 SDK 迁移已交叉检查，执行待补。来宾适配未抽取或证明，纯核心、历史已证明 Rust 与 LLBC/Lean 未改，旧结论不覆盖新增组合。

[下载错误格式化](../sandbox-guest-service/src/file_download/stream/errors.rs)及结构化 Linux errno 新增操作阶段/状态对应、可信解析路径、Go JSON 转义与精确线长、初始范围失败前头部回退，以及错误正文继续持有原授权/容量的义务。有限原 Go 受控错误对照只属工程证据；Linux 真实 errno 与 NUL 适配回归尚待实跑。修改的来宾 runtime/service 均未抽取或证明，纯来宾核心、历史已证明 Rust 和 LLBC/Lean 未改，旧定理不覆盖新增错误响应。

[上传请求连接](../sandbox-guest-service/src/file_transfer/upload.rs)、[可移植头决策](../sandbox-guest-runtime/src/file_upload.rs)及账户池原期限/所有权重构新增编码预检与账户/metadata/类型的拒绝顺序、单次准入、阻塞查询取消后守卫保持和 raw 实际 EOF 的组合义务。共享 metadata 验证已替换真实写入中的相同校验，MIME 大小写辅助仅开放 crate 内复用；这些来宾实现均未抽取或证明。有限固定 Go 头部对照只属工程证据，Linux 文件/账户取消组合待实跑。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不覆盖本候选的新增连接。

[上传成功回执](../sandbox-guest-service/src/upload_response.rs)及原 BatchReply 的响应连接新增完成文件效果与响应交付的分离、原槽位/期限延续、最后字节副本释放、精确编码预算和 Go HTTP 成功头对应义务。下载错误转义/计数抽为共用内部函数，UploadResult 只迁移到可移植类型并保留旧路径重导出；原错误对照须一起重跑。这些来宾实现均未抽取或证明，固定 Go 回环 HTTP 只属有限工程证据；Linux 文件池/SDK 连接待实跑。纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新回执连接。

[上传详细错误](../sandbox-guest-runtime/src/filesystem/transfer/upload.rs)及父目录准备重构新增实际操作阶段/路径/errno/xattr 名的准确记录、粗分类兼容、部分写入计数及失败后不重放的连接义务。原输入/预算拒绝仍独立，固定 processFile 的有限故障状态对照只属工程证据，Linux 真实路径与 FD 错误待实跑；文件 I/O 错误响应现接下述组件，其余拒绝映射仍待补。这些来宾模块未抽取或证明，纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不覆盖本候选。

[上传错误发送](../sandbox-guest-service/src/upload_response/errors.rs)及请求入口新增实际故障经原 worker/批次传递、阶段/errno/属性名与固定 JSON 对应、原声明长度诊断、编码上界与原槽位/期限延续的义务。错误发送失败不回滚文件，实际 I/O 与响应保留分别持有原责任；这些组合均未抽取或证明，原 Go 受控错误和 HTTP 对照只属工程证据，Linux 文件/SDK 连接待实跑。纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖本候选。

[上传头拒绝](../sandbox-guest-service/src/upload_response/rejection.rs)及共享 metadata 详细验证新增首字段字节/键/偏移保留、单条校验优先于总量、先校验再分配、公开错误与原容量/期限连接的对应义务。旧粗分类入口保持，多个无效 metadata 键的 Go map 顺序不固定；有限原方法/错误表达式和 HTTP 对照不是全请求证明。Linux 连接/SDK 错误解析仍待实跑。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，来宾适配仍未抽取或证明，旧定理不覆盖这些诊断及响应。

[gzip 首头错误](../sandbox-guest-service/src/upload_response/gzip_header.rs)新增真实零字节 EOF 与截断/源/预算失败的区分、首头阶段到固定错误包装及原响应责任的连接义务。编码器及正文解码/校验算法未改变；这些来宾适配未抽取或证明，有限 Go 首头/HTTP 对照不证明完整 gzip 上传，Linux 请求/授权和 SDK 解析组合待实跑。纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不覆盖该新增错误分类。

[multipart 路径拒绝](../sandbox-guest-service/src/file_transfer/batch.rs)及错误响应新增完成路径的移动/顺序/排除规则、重复前预算与文件未打开、filename 闸门、两遍迭代编码预算及原响应责任的义务。有限原 resolvePath/CustomPart 与 HTTP 对照只属工程证据；路径展开使用注入观察，Linux 文件副作用/授权/SDK 组合待实跑。来宾适配未抽取或证明，纯来宾核心、历史已证明 Rust 与 LLBC/Lean 未改，旧定理不覆盖新增错误分类和发送。

[multipart 准备错误](../sandbox-guest-runtime/src/multipart/headers.rs)及原上传入口新增三种零正文读取拒绝的阶段分类、旧粗入口兼容、显式预算与不支持文本的独立处理、500 包装和原发送责任延续义务。MIME 参数/正文解析算法未改，固定标准库方法/HTTP 有限对照仅为工程证据，Linux 请求与 SDK 解析待实跑。来宾适配未抽取或证明；纯核心、历史已证明 Rust、LLBC/Lean 未改，旧结论不覆盖新增分类。

[raw gzip 正文错误](../sandbox-guest-service/src/upload_response/gzip_read.rs)新增请求模式/编码共同限定的失败阶段、已交付解压前缀到写入效果、失败前不完成 metadata、已知诊断与资源/源错误分离以及原错误响应责任的对应义务。gzip 算法和纯核心未改；内存复制目标上的固定 Go 原方法/HTTP 对照只属工程证据，Linux 文件/metadata/SDK 组合待实跑。来宾适配未抽取或证明，历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新连接。

[multipart framing 诊断](../sandbox-guest-runtime/src/multipart/framing.rs)和[错误发送](../sandbox-guest-service/src/upload_response/multipart_read.rs)新增读取阶段/换行观察、受行预算约束的错误字节、QP 优先级、两遍 Go Quote/JSON 计长、PartEnd 与当前前缀效果、粗入口兼容及原响应责任的对应义务。固定 Go 表/方法与内存观察对照只属工程证据，Linux 文件/metadata/SDK 组合待补。来宾适配未抽取或证明，纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖本候选。

[MIME 头观察](../sandbox-guest-runtime/src/multipart/header_error.rs)新增完整物理/逻辑行确定时机、原验证顺序/折叠/键规范化、诊断源字节预算、真实源失败后已有头验证与本地长度/预算/期限事实分离、当前 Part 未创建及旧粗入口分类迁移的对应义务。原 Go/内存文件观察/HTTP 对照仅为工程证据，Linux 文件/SDK 与原响应责任连接待实跑。来宾适配未抽取或证明；纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新诊断。

[QP 诊断与前缀](../sandbox-guest-runtime/src/multipart/quoted_printable.rs)新增详细编码错误、固定物理行/后缀预算、源失败与本地检查事实分离、写入先于诊断、旧粗分类迁移及原响应责任的对应义务。新增固定源读窗口、物理行/折叠缩进/末尾 CR 游标、分段读取提交、最后片段终态和满行错误时机的对应义务。原满行缓冲对齐差异已通过保留实际读取事实消除并转为一致回归；相同源读取事实上的原方法对照不是通用精化关系或全端点证明。Linux 文件/metadata/SDK 连接待实跑；来宾适配未抽取或证明，纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖该行为。

[multipart 源失败阶段](../sandbox-guest-runtime/src/multipart/framing.rs)及 gzip 错误连接新增 Open/EOF/失败互斥、来源原因与阶段关联、失败后缓冲字节排出、歧义边界的完成点、近似边界最后字节才携带错误、QP/头/写入优先级和 MIME End 后严格排空的对应义务。原内容编码门控、忽略字段的粘滞错误包装、旧粗入口与原响应责任延续均须保留。固定 Go gzip/MIME/QP 与内存文件观察的有限对照只属工程证据；Linux 文件/metadata/SDK 组合待补。来宾适配未抽取或证明，纯来宾核心、历史已证明 Rust、LLBC/Lean 未改，旧结论不能背书本候选。

[忽略字段的 Close](../sandbox-guest-runtime/src/multipart/discard.rs)新增原始读取预留、事件分片与消费位置的对应、固定 QP 预读/物理行预算、编码错误后只丢弃已预读字节、Close 完成与原始 part/请求成功分离、下一 part 阶段及原授权/期限延续义务。有限对照使用 identity 受控读大小或 gzip 实际记录的读取序列，不是全网络分块等价或通用精化证明。Linux 文件/metadata/SDK 与取消组合待实跑；来宾适配未抽取或证明，纯核心、历史已证明 Rust、LLBC/Lean 未改，旧定理不覆盖新路径。

[文件查询绑定](../sandbox-guest-runtime/src/file_query.rs)及[公开错误响应](../sandbox-guest-service/src/file_query.rs)新增完整原始查询/字段预算、部分 URL 解码与固定参数拒绝顺序、字节到文本边界、整数溢出顺序、Go Quote 两遍线长预算及错误字节保留原守卫/期限的对应义务。签名字段解析不构成授权；准入/鉴权已由 FileHttp 接入 GuestHttp 和正式 daemon，仍须证明该组合保持原身份与责任。固定 Go 原绑定函数、http.Error 对照及 Linux SDK 组合的精确工程证据在验收清单维护，不构成证明；来宾适配未抽取或证明，纯核心、历史已证明 Rust 与 LLBC/Lean 未改。

[文件专用鉴权](../sandbox-guest-service/src/session/file_authorization.rs)新增私有原令牌与摘要/defaults 同次提交、候选失败清零、头/查询预算、令牌优先和签名参数表示、恒定时间比较及锁后时钟读取义务；SHA-256/base64 为外部密码原语，不因此证明原冒号拼接格式的元组唯一性。专用许可不能由调用方转换为通用命令授权，绑定的 query/操作和原绝对期限须经下载/上传入口一直移交到原账户/文件/响应所有权。会话可移植回归及固定 Go 对照只是工程证据，Linux 新组合待实跑。自有会话与鉴权逻辑未抽取或证明，纯核心、历史已证明 Rust 与 LLBC/Lean 未改；旧定理不覆盖本候选。

[文件请求准入](../sandbox-guest-service/src/file_requests.rs)新增共享容量无等待队列、头/查询/鉴权顺序、准入时原期限、取消锁等待及分类拒绝的所有权义务。专用许可携带的原请求名额已连接上传/下载租约，须证明后台阻塞工作、结果和最后响应字节实际释放之前不会提前归还；拒绝编码失败/取消同样沿用原名额和期限。HEAD 早期拒绝新增完整编码预算先于抑制正文、原头部长度及空发送器直到 EOF/丢弃/期限的所有权义务；有限 Go HTTP 对照不构成全输入或传输组合证明。套接字与已解析头/URI/传输缓冲、正式拒绝策略及停止预留不在本组件的已实现预算内。可移植测试只是工程证据，Linux 账户/文件/SDK 连接待实跑；无新增抽取或证明，纯核心、历史已证明 Rust 与 LLBC/Lean 未改。

[共用文件 HTTP 编排](../sandbox-guest-service/src/file_http.rs)新增准入到固定方法派发、可信未消费 Body 的 framing 长度、授权查询到 MIME/路径和上传/下载参数、未映射失败持续占额的组合义务。多范围响应在原 worker 按需生成 boundary，须区分自有 30 字节编码/生成阶段/错误处理与 getrandom/OS 熵的外部前提；取消/超时不能提前释放仍未返回的熵调用。组件及可移植检查不证明完整监听器、最终失败策略或端到端文件行为；Linux 新组合待实跑，无新增抽取/Lean 证明，纯核心和历史已证明 Rust 未改。

[文件 HTTP 构造校验](../sandbox-guest-service/src/file_http.rs)及[一元文件构造校验](../sandbox-guest-service/src/file_rpc_http.rs)新增可信配置到可服务对象的对应义务。原头/编码/响应/gzip/MIME 校验抽为共用函数，须证明无请求/账户/文件效果、无请求相关分配、与原有效配置集合及 read-buffered 容量计算一致；早期配置拒绝不能背书真实资源峰值或系统可用性。这些来宾适配尚未抽取或证明；纯核心、历史已证明 Rust 和 Lean 未改，旧定理不覆盖新增装配连接。

[未知文件账号响应](../sandbox-guest-service/src/upload_response/account.rs)新增缺失账户分类与预算先于诊断分配、拒绝阶段、Go JSON/小响应头/HEAD 以及原容量/期限交付的自有义务。可移植有限 Go 对照不证明完整账户源或 POSIX 语义，Linux 请求/SDK 组合待实跑；这些适配未抽取或证明，历史已证明 Rust、纯核心及 LLBC/Lean 未改。

[路径展开错误](../sandbox-guest-service/src/upload_response/path.rs)新增真实词法失败到原 HTTP 包装、原查询与默认路径区分、诊断分配预算、批次中先前副作用保留与更晚 part 不执行的自有连接义务。共享下载上下文错误发送器的重构有可移植回归；固定 Go 原函数有限对照不证明文件系统解析/竞态或全输入等价，Linux 请求/SDK 组合待实跑。未抽取或证明，历史已证明 Rust、纯核心及 LLBC/Lean 未改。
文件[一元编解码](../sandbox-guest-runtime/src/file_protocol.rs)及[授权路径解析](../sandbox-guest-service/src/files.rs)尚未抽取或证明。共享 JSON 解析/有界输出和 Basic/路径辅助函数供命令、文件两端复用，工程回归核对原命令行为；纯来宾核心未修改。字段缺省/精度、编码预算、账号选择、路径展开、授权快照到实际 I/O 的连接均须在 #101 后续证明，固定 SDK 与合成属性通过不证明真实文件操作、DAC、整个服务或系统正确。

新增[真实一元文件端口](../sandbox-guest-runtime/src/filesystem/mod.rs)、账户显示反查及[阻塞文件所有权](../sandbox-guest-service/src/file_io.rs)只有工程回归与固定参考对照，尚未抽取或证明。链接解析与实际 inode/FD、Go 属性对应、遍历/暂存/结果预算、非原子副作用与错误、原授权到派发/完成、取消后容量及结果保留均是新增自有义务，不整体列为外部前提。系统调用、目录变更、xattr、线程池和序列化的实际连接另须最小外部合同。纯来宾核心和历史已证明 Rust 未改，旧定理不覆盖这些新增实现；组件对照也不证明 POSIX/ACL 隔离或整套部署系统。

### 当前串行实现的模块交接

#262 B8 的代码定位基线为 `beb46514cc1b99d1731616e9d5bd8f4dd143270a`。以下是实际实现的输入、输出和待证明连接，不是新增规约或已完成定理；后续 #101 证明批次必须重新冻结实际源码、抽取产物和工具链。本批保持原管理锁、单控制进程租约、派发顺序和 SQLite，不改产品行为。跨实例有限并行另列 [P2 待办 #284](https://github.com/ningZhang-cs/sandproof/issues/284)，不作为本次重构的前置条件。

| 局部边界与实际入口 | 输入、输出及当前责任 | 后续对应证明需要连接的事实 |
|---|---|---|
| 准入与持久决定：[admission.rs](../controller-service/src/controller/admission.rs) 的 `admit_create_with_options` / `admit_template_create`；[transaction.rs](../controller-service/src/controller/transaction.rs) 的 `WorldTransaction` | 输入原用户、创建参数、模板绑定、可信时间和容量限制；接受时保存请求、资源计划、寿命、预算及工作记录，拒绝/提交未知分别返回错误。`commit_with` 在调用提交前使缓存失效，仅确认提交成功后发布候选状态 | 核对 SQL、检查点＋输入尾、核心状态与 outbox 的表示关系；拒绝不产生外部工作，不确定提交不允许使用候选缓存或创建替代身份。编号高水位与业务接受分别核对，不能要求拒绝时所有持久表逐字不变 |
| 生命周期、寿命与撤销：[lifecycle.rs](../controller-service/src/application/coordinator/lifecycle.rs) 的 `create_locked` / `kill_locked` / `expire_due_report`；[admission.rs](../controller-service/src/controller/admission.rs) 的 `publish_running_at` / `set_lifetime` / `accept_expiry` | 创建接受后才绑定原物理位置、登记原凭据并执行；宿主就绪后采样运行起点。终止意图提交后撤销原数据通道，提交未知也保守撤销但不派发未确认动作；到期时重新读取时钟与寿命 | 原管理锁内持久决定、宿主派发、时间重读、撤销和返回结果的次序；接受之后的错误不能伪装成可安全重新创建的拒绝；终止后旧激活不得恢复 Running 或访问权限 |
| 工作领取与回执：[controller/mod.rs](../controller-service/src/controller/mod.rs) 的 `dispatch_next_for` / `claim_next` / `complete`；[workflow.rs](../controller-service/src/workflow.rs) 的 `drain` | `claim_next` 从原 outbox 选择已提交且核心预检允许的动作，持久领取后调用宿主；传输失败保留未知。`complete` 接收绑定动作身份的宿主回执，核对证据、重复/冲突、当前终止状态并原子应用资源更新与回执 | 派发提交先于副作用；回执不能先于领取；同一回执重放不重复推进，旧激活回执不可复活终止实例。底层 `Journal::dispatch_next` 只是通用记录原语，正式核心连接以 `ControllerJournal` 的资格检查和 `complete` 为准 |
| 恢复与失败隔离：[recovery.rs](../controller-service/src/application/coordinator/recovery.rs) 的 `recover_locked` / `matches_current_reclaim` / `bind_recovery_slots`；[controller/mod.rs](../controller-service/src/controller/mod.rs) 的 `matches_current_*` / `reconcile_observed` | 从原持久快照、未知动作、密封资源计划和精确宿主观察恢复；先核对当前操作/尝试/物理位置，再执行命令专属补偿或只读观察。单实例观察失败保留该实例未知，共享存储不可信则拒绝继续 | 恢复不重新派发未知 PrepareVm；可收敛的初始化/停止补偿只处理原身份，不等于任意重试安全；局部故障隔离不吞掉全局状态损坏。缺失绑定安装资格仍需原位置与前任身份核验，不是新建凭据的许可 |
| 宿主判断、执行与证据：[decisions.rs](../sandbox-node/src/linux/decisions.rs)、[identity.rs](../sandbox-node/src/linux/identity.rs) 的 `validate_action` / `require_fence` / `require_slot`；[execution.rs](../sandbox-node/src/linux/execution.rs) 的 `execute`；[observation.rs](../sandbox-node/src/linux/observation.rs) 的 `observe_action` | 值判断比较动作字段、围栏、位置坐标、进程身份及结果形状；节点另读取当前占用、归属和现场证据。只读观察不启动/停止/清理 VM；执行和命令专属恢复使用原身份 | 纯布尔比较不是执行证书；必须证明现场读取、判断与使用的顺序，及自有见证构造如何满足 `HostEvidence::validates`。不能将整个 NodeHost、观察器或恢复器作为外部正确假设 |
| 终局发布、清理与容量：[publication.rs](../controller-core/src/publication.rs) 的 `handle_publish_destroyed`；[workflow.rs](../controller-service/src/workflow.rs) 的 `finish_accepted_kill_v1` / `cleanup_destroyed_resources`；[host_slots.rs](../sandbox-node/src/host_slots.rs) 的 `release_host_slot`；[recovery.rs](../controller-service/src/application/coordinator/recovery.rs) 的 `release_reclaimed_slots` | 终止计划的可信停止/关闭回执满足核心前置条件后发布 Destroyed，随后继续物理清理。物理位置先持久记录 Releasing 和原退休清单，精确清理并观察确认后才可 Free；数据持有者未排空仍保留占用 | Destroyed、物理清理完成、位置 Free、逻辑退休及结果过期是不同事实。停止证据不能直接证明目录已删；未知不释放位置或预算，旧清理不能影响复用位置。已接受工作的收尾预留和真实字节边界仍需组合证明 |
| 数据权限与在途持有者：[data_access.rs](../controller-service/src/application/coordinator/data_access.rs) 的 `data_permit` / `spawn_data_relay`；[data_credentials.rs](../controller-service/src/application/coordinator/data_credentials.rs)；当前[中心网关](../sandbox-center-service/src/gateway.rs)及共享传输 | 原管理锁内查可信凭据索引、核对归属/寿命/VM/代数并登记撤销、排空及宿主闸门；真正启动中继前再次核对。中心只使用同次已确认全局视图，转发不自行授予宿主权限或拼接命令；B8 历史基线不覆盖新网关 | 许可不是永久授权；旧许可在撤销、到期或身份改变后不能启动连接。自有闸门登记、子进程回收、字节/协议结束和持有者排空须单独连接，不把 EOF 或 HTTP 成功当作物理退出证据 |

局部证明按上述责任建立前置条件、返回关系和后继状态关系，然后组合“创建已受理 → 结果未知 → 接受终止 → 精确停止/清理 → 发布终局”。组合必须包含工作调用资格、持久提交和物理观察之间的关系，不能只重新运行独立模型定理；模板构建/管理仍排除，但核心使用模板绑定与资源形状仍在范围内。有限执行安全对应不能直接推出无限轨迹的最终完成性；后者仍需要内部进度量、调度及外部响应条件。

### 固定工具链的局部抽取评估

以下评估绑定上述代码基线和[固定工具链](../verification/README.zh-CN.md)。真实 Rust 文件未改写，未升级工具；输出保存在仓库外的独立临时目录，没有覆盖正式 LLBC/Lean 产物。采用 `--preset=aeneas --error-on-warnings --sysroot default`，Cargo 使用 `--lib --locked --offline`；Aeneas 使用 `-backend lean -abort-on-error -warnings-as-errors -split-files -emit-json`。入口是 `charon cargo --start-from <实际函数> --dest-file <独立输出> -- --manifest-path <原 crate/Cargo.toml> --lib --locked --offline`，随后转换同一 LLBC，不手写一份替代 Rust 实现。

| 选定真实函数 | 已实测层次与阻碍 | 下一步，不是已完成证明 |
|---|---|---|
| `controller-core` 的 `crate::effect::handle_effect_receipt` 与 `crate::publication::handle_publish_destroyed` | Charon 成功；固定 Aeneas 生成两入口的 Lean 函数及依赖。生成器仍输出标准库外部函数的公理模板，未导入或提交它；未完成外部函数可计算连接、Lean 编译或与独立模型的对应证明 | 在证明批次核对新枚举/状态与标准库连接，重新抽取正式入口，补齐接受、按序拒绝和完整后继状态对应；临时生成成功不背书历史定理或整个服务 |
| `controller-service` 的 `crate::application::coordinator::recovery::can_install_host_slot_binding` | Charon 成功，Aeneas 在 `recovery.rs` 的 `record.terminal && record.phase == Revoking && record.current_vm.is_none()` 分支合并处报 `Could not match the contexts`。额外显式 `--include sandbox_controller_core` 展开自有核心依赖后仍在同处失败 | 后续以同一规则评估等价的局部 Rust 控制流整理，先保留所有边界回归，再重新抽取；不能将自有核心查询保持不透明或升级工具来冒充完整连接。本批只登记阻碍，不改恢复语义 |
| `sandbox-node` 的 `crate::linux::decisions::fence_can_advance` | 最初 macOS 原目标缺少真实 Linux 模块，跨目标又因 SQLite 的 C 编译器缺失阻断。R3 在原服务库分离共享宿主合同与默认 `durable` 实现；节点独立依赖树不再包含 SQLite。使用固定 Charon、原函数、`--targets x86_64-unknown-linux-gnu` 和原始 Linux cfg，已完成 LLBC 抽取及固定 Aeneas 的单函数 Lean 翻译；未复制算法，临时产物不进入正式生成目录 | 该函数的生成 Lean 尚未编译或完成对应定理；节点其他函数、自有见证核验、恢复与服务组合义务仍需逐项抽取验证。正式服务器仍启用同一持久化实现，不能将宿主端口、自有恢复或见证整体当作可信黑箱 |

本批是代码接口交接和抽取可行性评估，**没有新增 Lean 定理，也没有执行完整 `verify-controller.sh` 证明验收**。已有历史证明结论及开放义务不变；抽取成功、环境未就绪、翻译失败、局部证明与完整证明验收必须分别报告。

### 首条跨模块证明批次：待执行义务

R3/R5 的结构整理不新增定理：共享宿主合同只是原声明的唯一归属，`durable` 默认生产实现不变；见证构造仍是本项目代码而非新增假设。令牌 SHA-512 改用既有 Rust 库，摘要格式由回归及真实 SDK/KVM 验收检查，但该密码原语与授权逻辑的合同连接尚未证明。中继别名删除不改变原资源资格或排空算法。核心三处前插去重暂不合并，以免仅为小型复用改变历史抽取函数和证明接口；既有核心源码及证明结论不因此扩展到服务整理。

首批链路为“中心耐久创建 → 原节点就绪 → 丢回执 → 中心重启 → 原身份恢复 → 销毁及精确清理”。先完成 #292 合同、功能/故障/实机验收，再冻结同一真实提交和抽取物；本链尚未实现对应闭合证明。下列本地义务是链路中的必要部分，还须纳入上表 P1–P7 的中心/协议/确认连接；详细批次和闭合判据只在[证明工作流](proof-workflow.md#首条跨模块链路)维护。

- **类型与状态对应：**先核对当前 Rust 与生成 Lean 的差异，包括 IPv4、`AbortPrepare`、`RetireActivation`、`RetiredByStop`。补齐模型与抽取后的对应关系；缩小首批行为范围不等于可以遗漏新增枚举或降低整套检查。
- **持久接受与派发：**证明准入、编号/收尾预留、意图和 outbox 的事务组合；提交前无外部派发，拒绝保持持久状态，提交不确定不生成第二个身份。SQLite 的原子提交是外部原语合同，事务顺序和恢复决定是本项目需要验证的逻辑。
- **执行资格与真实观察：**证明当前操作、尝试、代数、终止围栏和密封资源计划的检查一致；未知不是无副作用失败；终止初始化须引用同一资源的可信停止回执，迟到回执不能恢复权限。Linux/pidfd 的物理观察准确性仍需明确合同和系统测试支持。
- **恢复、授权与收尾：**证明局部观察失败保留该实例的未知状态和预算，不阻止独立实例到期收尾；共享状态不可信则拒绝继续。覆盖重放、凭据恢复资格及网关撤销登记的次序，不把自有恢复循环、互斥锁和授权决定整体假定正确。
- **验收与剩余范围：**按证明工作流读取实时目标、局部试验、重新抽取并运行完整验收；分别报告本链的安全对应、仍需的外部合同，以及内部进展/长期容量义务。首批局部定理不等于全服务、无限运行或完整最终完成性证明。

`scripts/verify-controller.sh` 检查 Rust、重新抽取一致性、所有项目证明文件和未完成证明；`verification/ProofAudit.lean` 审查真实定理传递依赖，只允许 Lean 标准逻辑基础。编译文件数、抽取函数数、局部成功或 CI 绿灯都不是全输入证明覆盖。

`controller-service` 已接入纯控制核心的创建、销毁、状态发布与真实宿主回执；`sandbox-api`、`sandbox-node` 和 `sandbox-server` 新增持久寿命/配额、鉴权、网关、实际磁盘余量检查和实机测试。这些新增服务代码**没有** Rust→Lean 对应证明；现有定理只覆盖原先明确列出的纯核心输入，不自动扩展到 SQLite、时钟、令牌、Linux 网络空间、HTTP 转发或 Firecracker。特别是自有授权、持久化决策、worker 资格和恢复逻辑不能整体放入“适配器正确”假设；需要逐项列出外部原语合同并继续做对应证明。未知回执对账、重启恢复、完整容量保证和长期完成性仍是开放义务。

## V9 首链责任投影模型

归 #342/#101。实现定位基线为 `9eeb815054a0b37d17bff831c23ea34004122892`，本批**不修改、不抽取新增生产 Rust，也不声称实际实现对应已完成**。模型源为 [Record](../SandboxModel/FirstChain/Record.lean)、[System](../SandboxModel/FirstChain/System.lean)、[Scenarios](../SandboxModel/FirstChain/Scenarios.lean)，实际依赖审计为 [Audit](../SandboxModel/FirstChain/Audit.lean)；固定 `lean-toolchain` 的 Lean 4.31.0 与未改变的 `lake-manifest.json`，只依赖标准库。本批不导入未合入的 V0 成果；不重新定义其身份、数值或编码实现。

| 交付 | 已证明结论及边界 |
|---|---|
| 抽象初态、单步及有限可达执行 | `initial_invariant`、`advance_safe`、`step_invariant`、`reachable_invariant`；登记节点列表和实例绑定类型均为参数，不限定一台或两台机器。真实初始化、故障微步骤及有限预算的对应仍待证 |
| 持久责任与释放 | `reachable_responsibility`：可能物理效果保留中心/节点责任且两端未释额；`reachable_release`：中心释放前，节点终止/围栏/停止/清理、零持有者及零引用成立。布尔事实和计数是模型投影，不是实际资源证据或总容量上界 |
| 终止、修改范围及消息 | 中心/节点终止的局部单调、禁止迟到创建/新本地许可/越过围栏的效果、单次抽象认领、其他绑定记录不变；在途事实可重复/延迟/丢失但不伪造。未证明实际全参数拒绝、物理唯一 VM、位置复用或 TLS/两端授权接线 |
| 故障及非空见证 | 已就绪但丢回执后中心崩溃仍可达；写生效而回复未知不清责任；重启不直接恢复派发资格。`original_ready_recoverable` 证明有条件地**存在**原结果恢复路径，不是公平执行最终完成性 |
| 定义审查场景 | 正常启动/清理确认、只收件不得执行、停止不等于清理、连接或引用阻止释放、终止先到、假就绪及提前释额反例；一般安全结论不依赖只枚举这些场景 |

34 项模型定理进入 `bash scripts/verify-first-chain.sh` 的实际编译和依赖审计；仅允许 Lean 标准逻辑依赖，无占位或自定义公理。通过 sandbox-lean 的同一服务客户端隔离试验后编译正式文件；工具试验与本批构建相互区分。现有完整入口会发现 `SandboxModel/FirstChain` 下全部文件，未修改旧完整检查或排除失败根。

本批可移植基础检查通过：固定真实 Redis、Rust 工作区测试/格式/静态检查、文档及 838 项工具测试；7 项既有平台跳过，131 项 Linux 专属工具场景未运行，不计为 Linux/KVM 验收。无生产行为变化，未运行 SDK 实机、KVM 或题集。

完整 `scripts/verify-controller.sh` 的 Rust 检查通过，随后固定 Aeneas 在定位基线的 `controller-core/src/publication.rs` 第 156–182 行报 `Returns inside of nested loops are not supported yet`，退出 2；**整体证明验收未通过**。失败生成的本工作树 LLBC 留作实验产物，正式历史产物未改动；不引入 V0 的在途重构来绕过隔离。该既有抽取/历史对应差距由 V0/#337 的适用批次处理后，再重跑整套验收。

## V9 首链身份连接

归 #342/#101，依赖 V0 已提交快照 `c7d1afd316d825442492c9012baf236def7d5984`（堆叠 #349，尚未进入 integration）。本分支通过合并提交 `d596c06` 组合该快照与首批模型；没有修改另一窗口的分支或工作区。实际来源和完整 V0 冻结条件见[基础输入摘要](../verification/foundations-inputs.sha256)及[环境清单](../verification/foundations-environment.json)，本节模型/连接/审计入口由[首链输入摘要](../verification/first-chain-inputs.sha256)绑定；适用 `aarch64-apple-darwin`、固定 Lean/Charon/Aeneas，不作 Linux/KVM 或部署证明声明。

新增 [Identity](../SandboxModel/FirstChain/Identity.lean) 复用真实抽取身份检查，并接入原首链检查。19 项新增定理与原 34 项一起审计；不是 19 个生命周期处理函数已证明。

| 已建立连接 | 结论与剩余边界 |
|---|---|
| 身份检查成功充要条件 | 实际 `ControlIdentity::validate_response` 与 V0 `checkResponse` 一致，成功保留完整字段；存在可接受身份。编码/解码、构造入口和身份授权未因此闭合 |
| 实例键与操作字段 | 节点/租户/全局沙箱定义实例键，操作/意图/位置/窗口变更不另建实例；仍以完整身份拒绝跨实例、旧意图和旧窗口。原全局→本地映射及预期请求的真实选择待证 |
| 带身份检查的投递合同 | 在登记、原消息、资格及事实转换这些显式前提下，对应原模型步骤，保持不变量/可达性、拒绝不改状态、其他实例记录不变和释放条件。前提提供者仍需真实模块实例化 |
| 迟到就绪回复 | 在模型终止事实成立时，投递合同不能更新就绪；完整中心事实优先级、观察版本、事务和回调对应尚未证明 |

V0 循环重构原样复用；本批公共身份 Lean 定义、抽取脚本、工具版本与业务条件没有改动。跨目录重新生成时，V0 LLBC 仅输出路径元数据改变，所有生成 Lean/翻译清单逐字节一致；保留自然生成产物并在本目录重跑原完整局部一致性检查，不删除或规范化新增字段。库模型符合性及余下工具可信条件仍按依赖登记声明。

本节验收在 Mac 本机独立工作区完成：`verify-foundations.sh` 重新抽取真实入口并通过 44 项 V0 定理审计；`verify-first-chain.sh` 验证输入摘要、重新检查 V0，并通过全部 53 项首链定理编译和依赖审计。sandbox-lean 的同一服务客户端对正式身份连接及审计文件返回完整、非过期、无错误诊断。可移植基础检查通过，包含真实固定 Redis、Rust 工作区检查、文档及 845 项工具测试；其中 7 项平台跳过，另有 131 项 Linux 专属工具场景未运行。未运行 Linux 完整检查、SDK 实机、KVM 或题集。

完整 `verify-controller.sh` 的 Rust 与 V0 检查通过，原嵌套循环障碍已由复用的生产重构解除，固定 Charon/Aeneas 成功重新生成本地控制核心语义；随后因历史提交生成物与当前真实源码生成结果不一致，按原判据退出 1：`generated Rust semantics were stale; regeneration changed checked artifacts`。**整体证明验收仍未通过**，没有进入后续全核心对应证明验收。本工作区的新全核心产物已保留于忽略的实验目录，正式历史产物未覆盖；其迁移及重新对应由 #337 的适用批次负责。该阻碍不同于上节定位旧版本时的循环失败，不能用局部身份结果背书全核心。完整首链的实际处理、提交、宿主/排空、全部分支和最终完成性仍开放，不能由身份子检查推出它们。

首链的真实事件清单与未解提供方统一见 [V9 第一批组合审查](https://github.com/VeriInfra-Lab/sandproof/blob/56b14d642c392316043dfbdbe4d26bacc663fb30/docs/formal/09-system-composition.md#41-342-第一批组合审查与责任投影模型)。FC-ID/COMMIT/EVIDENCE/DRAIN/INIT/EXTEND/IMPLEMENT 均未由本批实例化；有限重试、失败终局、期限、窗口/退休/复用、完整输入拒绝及 Rust/系统精化仍开放。`Safe` 不作为转换守卫，模型性质不是假设“实际实现服从模型”得到的实现结论；类型参数化也不是 V0 交接已完成。本批完成不关闭 #342、V1–V8 或 #101。

## V9 Linux 生命周期回执检查

归 #342/#101，以 `39c3497c24ce0362e1ce6ab05d2d9fc5f0d4b260` 为组合起点，继续使用冻结 V0 `c7d1afd316d825442492c9012baf236def7d5984`。实际新增入口、生产源码、规约、库模型、生成物及验收脚本由 [Linux 输入摘要](../verification/first-chain-linux-inputs.sha256)绑定；目标、编译器、工具二进制和 cfg 见 [Linux 环境](../verification/first-chain-linux-environment.json)。未修改生产 Rust、V0 目录、原 Lake 配置或其输入/环境清单，不将其它窗口在途代码纳入适用范围。

固定 Linux Charon/Aeneas 已编译并实测两个直接生产调用根：`ControlIdentity::validate_response`、`LifecycleResponse::validate_for`。根不复制算法；所有可达自有函数和协议版本常量初始化体都检查 LLBC、翻译清单及真实生成定义。Option 相等使用固定 Rust 标准库的真实函数体，并证明 u64 实例对应可选值相等；字符串和逻辑枚举标签的剩余模型责任按 [A6 登记](formal/dependencies.md#a6-公共基础库模型使用登记)保留，不宣称标准库或翻译器已被证明正确。

| 局部结论 | 范围和限制 |
|---|---|
| Linux 完整身份检查 | 全字段表示与 V0 数学 Binding 相同；真实检查的成功及精确错误对应。四项身份引理是 Linux 实现接入，不作为新增业务性质重复计数 |
| 全生命周期纯回执检查 | `lifecycle_response_refines` 比较完整结果；覆盖三个意图、六类事实的全部组合和按程序顺序的拒绝，身份错误不被后续事实错误覆盖 |
| 就绪的原寿命与范围 | 启动时间严格早于原期限；到期和最大到期精确等于原起点加原寿命，u64 溢出拒绝而不回绕。输入使用自然数表示，没有截断比较 |
| 接受条件与字段边界 | `lifecycle_success_iff` 双向连接身份和 `FactCompatible`；已证接受保留完整身份。此函数忽略 observation_version，版本/事实等级由中心实际处理另行检查，不视为整个系统缺陷 |

[Identity](../SandboxModel/FirstChainLinux/Identity.lean)、[Lifecycle](../SandboxModel/FirstChainLinux/Lifecycle.lean) 的 15 条定理通过正式 [Audit](../SandboxModel/FirstChainLinux/Audit.lean)，只依赖标准逻辑基础；`verify-first-chain-linux.sh` 的源码/工具核对、重新抽取逐字节比较、LLBC 全内容规范摘要、Rust 格式/静态检查和定理审计通过。标准服务客户端在服务器读取正式目标并返回完整、非过期、无错误或失败依赖的诊断。规范摘要只复用 V0 对无序 short_names 表的排序规则，不过滤其它 AST 字段。生成文件未手改，无占位证明、自定义公理或关闭内核检查。

本窗口的证明、抽取、编译和测试在隔离 Linux 工作区执行，本机只编辑、检查 Git/元数据和同步文件。工具修订不变；Linux 工具二进制、cfg 与 Mac 不混记。独立 [Lake 视图](../lake-first-chain.toml) 必须保持原配置全文，只追加新库；检查器拒绝其它差异。两个生成类型集合的全局枚举实例可能重名，故分别编译审计并复用同一数学合同，不同时导入或手改生成类型。

完整 Linux 基础检查已通过，包含真实固定 Redis、Rust 工作区和 983 项工具测试；清单完备性最终自查后，全部 985 项 Linux 工具测试及文档/提交检查重新通过，其中 9 项为本批检查器测试。函数、常量或初始化体遗漏、错误目标/不透明体均被拒绝。首次目录权限、构建资源及测试配置失败作为环境证据保留，没有通过改弱原检查解决；私有执行入口统一安全 umask、固定 Redis 和资源上限。未运行真实 SDK/KVM 或题集，不把局部验收替代整套门槛。

**整体证明验收仍未通过**：Linux `verify-controller.sh` 的 Rust 格式、工作区测试和静态检查通过后，原 V0 输入摘要全部通过；随后在工具指纹关卡退出 1，报 `charon differs from the frozen tool binary/revision`。V0 原清单绑定 Mac 目标及工具二进制，未重抽取或覆盖其产物，也未进入后续全本地核心验收。完整入口保留此关卡，并新增本批 Linux 检查；不改指纹凑通过。V0 迁移需复用该窗口后续冻结的成果；历史本地核心生成物与当前源码的迁移仍归 #337，不以此提前宣称全核心对应通过。

完整 `CenterState::apply_response`、原持久预期操作的选择、重复/旧观察/事实顺序、请求参数与初始化、事务提交、宿主事实及排空仍开放。纯回执检查尚未实例化既有 `IdentityDelivery` 或系统实际处理；FC-ID/COMMIT/EVIDENCE/DRAIN/INIT/EXTEND/IMPLEMENT 和条件最终完成性均未闭合，不关闭 #342/#101。

## V9 Linux 中心回执分类

归 #342/#101。组合起点为 `a45eef28f8259f4ab643a3acb82bf3aff8d48dbd`，接入 V0 #367 的已提交 Linux 快照并继续合并收尾提交 `9f2a37d702897156f53503795f6267066a6a6014`，没有修改另一窗口工作区。实际源码、独立规约、库模型、抽取配置及产物由 [Linux 输入摘要](../verification/first-chain-linux-inputs.sha256)绑定；V0 尚有其计划所列剩余项，不能将 13 个入口及 69 条局部定理说成 V0 全部完成。

生产 `CenterState::apply_response` 实际调用只读的 `Operation::classify_response`：检查回执是否符合原请求 → 完整重复回执 → 旧观察版本 → 不推进的事实等级 → 接受推进。原状态校验、实例/操作选择、状态更新和后继校验均留在原调用者；拆分不改变业务条件或拒绝顺序，也不证明原请求准入合法性。显式错误匹配避免捕获空闭包的析构胶水，不增加自有不透明函数或抽取豁免。新增两项 Rust 回归包含 3024 组旧判定对照及无效重复/有效旧观察重复的顺序边界；它们是测试证据，不是完整中心入口等价证明。

| 实际新增连接 | 已证明内容及边界 |
|---|---|
| 完整结果对应 | `operation_response_refines` 将实际抽取的分类与独立 [ResponseModel](../SandboxModel/FirstChainLinux/ResponseModel.lean) 对应；涵盖全部类型输入及精确 Duplicate/Advance/Stale 结果，不只证明成功后继 |
| 接受充要条件 | 重复要求原请求检查通过和完整回执相等；推进还要求非重复、观察版本严格更新和事实等级严格推进。等级不代替完整身份或事实载荷相等 |
| 拒绝及身份 | 无效重放、非重复旧观察及非推进事实被拒绝；接受推进保持完整原身份。已达最高等级仅禁止同一原操作继续推进，不禁止新续期/销毁操作 |
| 可接受见证 | 存在分类可接受的输入；不说明它通过中心准入、初始化或存在真实虚拟机 |

新增 [ResponseClassification](../SandboxModel/FirstChainLinux/ResponseClassification.lean) 的 17 条定理连同既有 15 条，共 32 条进入真实依赖审计。三个实际生产调用根全部重新抽取；Option 相等和协议事实/回执相等保留实际函数体，字符串、Option 共享借用、逻辑标签及内置模型的外部责任见 [A6](formal/dependencies.md#a6-公共基础库模型使用登记)。固定工具修订不变；复用已登记的 Linux 二进制并重新测量/冻结，不以改指纹替代重抽取。

V0/共享状态及首链抽取的输出路径按实际驱动工作目录冻结；每次先保留旧 LLBC，再要求本次新文件生成成功。完整中间表示摘要不删除输出配置或 AST 字段。V0/共享生成 Lean 和翻译清单不变，重新抽取与 69/43/85/23 条局部审计通过，既有首链 53 条及本批 Linux 32 条审计通过；47 项抽取检查器正负测试通过。

组合候选 `052e94eaad53479fb4bf87d72c435488524be0d5` 的全部相邻局部验收通过；1027 项 Linux 工具测试、文档及提交规范复验通过。Linux `check-function.sh` 已完成真实固定 Redis、Rust 工作区及诊断特性回归；V0 收尾合并前后生产 Rust、Rust 测试和基础入口相同，受影响的工具/抽取/文档检查及完整入口的 Rust 检查在合并后重跑。所有编译、抽取、证明和测试在本窗口独立 Linux 目录运行，本机只编辑及同步；没有执行 SDK/KVM 或题集。

**整体证明验收仍未通过**：该候选的 `verify-controller.sh` 通过 Rust、V0/共享状态和本批 Linux 局部审计，完整本地核心也成功重新生成；随后按原判据退出 1：`generated Rust semantics were stale; regeneration changed checked artifacts`，未进入全部历史定理编译/总审计。失败的新全核心生成物保存在忽略的独立诊断目录，正式历史生成物恢复到原候选，不当作迁移修复。迁移/重新对应仍归 #337。首轮线程创建失败退出 134 的日志也保留；调整私有资源设置后复验越过该关卡，未修改工具版本、内核或检查条件。

完整中心入口另作隔离预检：最终源码在同一固定工具、原生 Linux 单目标模式下，Charon 生成含真实 `CenterState::apply_response` 函数体的 LLBC，`has_errors=false`；固定 Aeneas 在 Rust 标准库 `core/src/str/pattern.rs:99` 的类型转换处报 `Unimplemented`，退出 2。这个结果不同于先前显式目标预检的迭代器错误；不是完整 Rust→Lean 翻译、库模型接通或对应证明成功，不进入正式三根产物。

标准证明服务客户端在服务器读取正式分类定理的实时目标，并返回完整诊断：`success=true`、`partial=false`、`stale=false`，无错误及失败依赖；文件 SHA-256 为 `1d59de88e63db03f694d280b8f1d7229d47505c03ac59b32cc0a9f8a22e38c51`。隔离探索及客户端启动/关闭的传输提示不计作证明成功，正式文件编译与上述依赖审计分别验收。

批次自查已完成，相邻模块的人工独立复核仍待认领。原请求的真实选择、完整中心更新、持久提交、宿主证据、权限撤销/排空与系统投递连接及条件最终完成性仍未闭合；FC-ID/COMMIT/EVIDENCE/DRAIN/INIT/EXTEND/IMPLEMENT 保持未解，不关闭 #342/#101。

### 完整生命周期 JSON 请求的有条件验证（#558）

本批连接原 `server::receive_intent` / `InternalClient::lifecycle` 使用的完整 `LifecycleRequest`，复用共享编号、计数器和身份类型。原 `LifecycleRequest::validate`、`ControlIdentity::validate_node`、`CreateBinding::is_valid` 及自有校验/转换保留固定 Charon/Aeneas 的真实生成体。实际字节入口的完整 Serde 图仍不能由原后端生成 Lean；两个只调用原 `from_slice<LifecycleRequest>` / `to_vec<LifecycleRequest>` 的包装，按[依赖登记 A6/B4](formal/dependencies.md#a6-json-与标准派生的最小合同)保留逐符号可信条件，没有增加独立 Rust 解释器或把自有业务守卫整体设为 opaque。

| 已证明的结论 | 适用条件和限制 |
| --- | --- |
| 原守卫的完整结果、接受条件和首个拒绝 | 检查原版本、完整节点、槽位、创建参数及三个 intent 的形状；不提供授权、寿命准入、持久提交或接收回调业务结论 |
| 全字段表示及逐值相等 | 保留身份、资源、32字节摘要、options、时间和 intent；投影相等推出原请求相等，没有隐藏截断或省略字段 |
| 完整字段读写、往返和有界合法接受 | [具体组合](../SandboxModel/Boundaries/JsonRequest/SliceBinding.lean)使用原叶子/转换与固定标准派生、SliceRead和紧凑渲染条件；原输入域及创建守卫推出存在不超过270336字节的编码和逐值往返，含非空初态见证。往返保持请求值，不要求原始JSON的空白、字段顺序或转义逐字不变 |
| 原错误及容器/文档结束顺序 | 原编号/计数器错误文本、类别和初始位置有对应；容器结束在visitor普通错误后仍可能推进游标，原错误优先，已有位置不被外层改写；最外层document end仅在解码成功后执行。不计算任意输入的parser位置，也不把失败解释成输入状态回滚 |

固定 serde/serde_core/serde_derive 1.0.228、serde_json 1.0.142 的扫描、UTF-8/转义、数值、标准派生、普通Clone及正常分配/有限渲染，实际切片、原错误字典和参数/结果机械绑定，仍由A6/B4提供。源码审查、完整AST准入和原生对照支持这些条件的采用，不是JSON库或async外壳的机器精化。普通String、Vec<u8>及无自定义IO来源的SliceRead/Vec错误属于本域；自定义可观察Drop、FD/许可/锁、panic/unwind、OOM/abort、异步取消及任意有状态序列化器没有据此覆盖。

[调用登记](../verification/json-request-binding.json)绑定五份生产源、十份相关库源、实际编译特性、完整字段/属性/声明顺序、两个完整根AST及完整LLBC摘要。标准结构体按实际派生接受对象或声明顺序的序列；固定Create/Destroy单元分支忽略可成功解析的额外字段，Renew拒绝未知字段，生产策略没有改变。原HTTP输入上限由真实`Limited`提供，options上限仍由原守卫提供，不能把`from_slice`本身当作HTTP限额。旧180个透明生成定义逐字不变，只加入两个真实守卫函数及对应类型；完整中间表示只沿用原无序`short_names`表排序规则，其余有序字段、清理记录和完整源文件文本全部保留。

本模块67条声明进入[依赖审计](../SandboxModel/Boundaries/JsonRequest/Audit.lean)。真实extraction crate的39项回归、严格Clippy和原客户端→mTLS→服务器字段/拒绝检查通过；后者为组件接线证据。永久错误版只移除原身份守卫，先构建和实际重抽取成功，再被原Rust错误顺序断言及同一`GuardContract`的否定证书拒绝，真实Lean服务和内核均核对通过；编译失败不算性质反例。

可执行代码和测试冻结于`a5b43c9b76eec060fbc201737d67c7bcbf451104`。a6000隔离Linux的原完整`verify-controller.sh`退出0（1172.612秒），157份保留项目Lean文件及入口编译、835个唯一声明审计和占位检查通过；原完整`check-function.sh`退出0（1734.82秒，1210项工具测试、13阶段）。1680项跟踪输入前后一致，仅对既有无序短名表采用原规范摘要；本模块与总审计的最终真实服务诊断均完整、非陈旧、无错误/失败依赖，证明源码指纹相同。最终报告只更新本覆盖小节，可执行/证明输入与该验收提交相同，并另核对文档及全部保留冻结清单。

此前完整入口检出的两次失败均保留：一次为字节LLBC嵌入的整份验证文件测试文本过期，按完整字段重新生成后增加提前源码身份检查；另一次为两个旧检查器私有夹具漏复制新公共依赖，19项在原断言前缺文件。修复只补13行夹具依赖和六份摘要，19个测试方法AST、全部检查器及原输入集合未变，最终完整套件中的19项均通过。没有放宽断言、安全检查或AST准入，也未改变生产Rust、Charon/Aeneas、工具版本或清理`.lake`。

原opt-in/特权忽略项未计为已运行；完整VM、部署TLS/网络、授权/存储/宿主效果、真实排空及系统完成性未由本批验收。本项提供代表性完整请求的有条件接入，不关闭T1/T2的其它使用点、T0–T8或整个V0，T9继续排除。
