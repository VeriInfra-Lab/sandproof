# V1 验收清单

[文档目录](README.md) · [产品范围](project-blueprint.md) · [模块合同](module-contracts.md) · [开发规则](../CONTRIBUTING.md)

本页维护场景与通过判据，不维护逐批进度或机器台账。每次验收在 issue/PR 绑定精确提交、配置、产物摘要、结果和未运行项；原始日志放私有 artifacts 或脱敏 CI 附件。基础、组件、KVM、题集和形式化证明分层报告，不能互相冒充。

## 1. 系统场景

| ID | 场景 | 通过依据 | 主责 |
|---|---|---|---|
| S01 | 原客户端创建、连接、查询、销毁 | 同一实例；真实停止后才返回销毁完成 | C+A+B |
| S02 | 多次命令、后台进程、输出及非零退出 | PID/句柄、输出和退出码正确，后台任务不会被误重复派发 | B+C |
| S03 | 上传下载、修改系统文件、安装依赖 | 私有系统可写、共享基础镜像不变、宿主文件不被越界修改 | B+C |
| S04 | 两用户、两沙箱交叉访问 | 未授权管理、命令、文件和网络请求拒绝，资源不混绑 | A+C+B |
| S05 | connect、set_timeout 与到期 | 延长/缩短语义正确；旧定时任务不会销毁已续期实例 | A+C |
| S06 | create 与 kill、并发重复 kill | 不发布虚假成功；最终无运行残留，无误杀 | A+B |
| S07 | VM 启动后回执丢失、重复派发 | 恢复认出同一资源，不再启动第二台 VM | A+B |
| S08 | controller/worker 在关键窗口重启 | 提交前无外部动作；提交后能重放/对账；未知结果不丢失 | A+B |
| S09 | 旧资源清理迟到、标识被系统复用 | 只处理绑定身份，不影响其他或新资源 | B+A |
| S10 | 禁网、公网/静态规则与资源限制 | 实际探测不能绕过策略，不能访问宿主管理或其他沙箱 | B |
| S11 | 磁盘/配额不足、恶意大输出或文件 | 新请求合理拒绝，收尾仍有空间，宿主与其他沙箱不受污染 | A+B |
| S12 | 客户端超时及命令通道断开 | 状态未知可观察，不自动重复执行有副作用命令 | B+C+A |
| S13 | Dockerfile/OCI 构建、模板创建、执行、评分与清理 | 原环境定义经通用模板链准备，不按题目追加产品行为；全新实例、原解答/评分、真实资源与精确收尾；报告实际适配范围 | B+C |

每个测试至少关联一个场景或明确缺陷。新增测试只补实际需求或缺失证据，不无限扩展矩阵；成功、拒绝、未知、失败与未运行分别报告。

S02/S11/S12 的命令输出验收采用持续流策略：超过旧累计上限仍完整成功且只派发一次；同时保留请求、文件和一元回复的边界测试、断线重连、背压及撤销测试。宿主辅助工具另测 stdin 阻塞、双输出互堵、无输出不退出、stdout/stderr 超量、早退、正常退出与原进程精确回收；未知退出保留额度使用受控观察点测试，不冒充真实不可中断内核故障。实机启动/初始化、网络观察、启动首帧和销毁收尾须绑定同一提交并检查残留；这些工程验收不代表形式化证明。

选定实机入口复用[外层监督器](../tests/system/server_slow_boundaries.py)的 `--command-output-only`：两个全新隔离 VM，固定同步/异步 SDK 各收完整 33 MiB 输出、核对单次派发与模板启动首帧，然后按原 VM/目录/网络/cgroup 身份销毁并检查残留。该选项不运行原慢文件场景；不冒充完整故障矩阵、完整题集或真实不可中断进程故障。

## 2. 模块与故障检查

| 模块 | 基础/组件重点 | 相关场景 |
|---|---|---|
| A：核心与持久服务 | 顺序拒绝、归属、同键改参、旧代数/尝试、重复回执；真实 SQLite 提交前后、不确定 COMMIT/ROLLBACK、缓存差分与恢复 | S01、S04–S09、S11–S12 |
| B：宿主与数据执行 | 固定资源身份、目录替换、持有者排空、未知观察、磁盘/网络/限额、迟到清理、上传与中继收尾 | S02–S04、S07–S12 |
| C：客户端与公共框架 | 固定 SDK 字段/错误/默认值、get/list 筛选排序分页、寿命、命令/文件协议、模板及 Harbor 全链路 | S01–S05、S12–S13 |

并发使用屏障和明确故障点，时间边界使用可推进时钟。基础检查不创建产品 VM，不访问付费模型或 E2B 云服务；命令见开发指南。功能测试不产生证明结论。

串行边界诊断用[应用组件夹具](../controller-service/tests/application_lifecycle/contention.rs)：两位置完整配置、真实 SQLite、受控宿主与时钟。屏障将一次准备操作至少阻塞 25 ms，分别观察另一实例的正常/过期鉴权、续期和到期清理；比较新库与 12 次创建/销毁/维护后的检查点，输入尾上限固定 64，并核对实际裁剪。运行 `cargo test --locked -p sandproof-controller-service --features contention-diagnostics --test application_lifecycle contention:: -- --nocapture`，输出请求、锁等待/持有和受控工作执行耗时；基础检查也覆盖默认关闭及显式开启两种编译。数字不是实机宿主成本或吞吐承诺，不设置平台相关的性能通过阈值。慢操作的等待影响须如实记录，但不能据此取消安全锁；并发优化需另行决定。

### 持久化、身份与有界运行

来宾账户/上下文的[纯判定与阻塞查询测试](../sandbox-guest-service/src/execution/tests.rs)覆盖 Basic 用户选择、home/目录、环境优先级、预算和未知账户；屏障与可推进时钟验证取消/超时不会提前返还仍在运行的查询额度。[真实 worker 组合](../sandbox-guest-runtime/tests/context.rs)从初始化令牌许可解析上下文后执行真实命令，核对完整环境、数值身份、目录和私有 socket 回收；USER/HOME 覆盖不成为 UID/GID。普通检查及 SDK 脚本执行这些场景，但当前账号的成功不能证明 root 到普通账号的切换。

单独的[特权身份组件验收](../tests/guest_credentials.py)要求已复核的精确提交、worker SHA-256，以及新 mount/PID namespace 的 root PID 1 和独立 proc。入口将原二进制核对后复制到本轮 root 独占文件再执行，只创建/清理自己的临时目录与子进程，不修改宿主账号。分别使用管道与 PTY 检查真实/有效/保存/fs UID/GID、附加组、能力、普通账号的组文件访问、root 文件拒绝及不能恢复 root；PTY 另核对三个终端描述符及主进程的独立会话。另以私有用户命名空间制造 setgroups/GID/UID 失败，验证没有用户副作用。它不由普通 CI 自动获取特权执行，结果单独绑定提交和 worker 摘要，不代表 KVM、完整来宾服务或抵抗来宾 root 的隔离证明。

[命令所有权检查](../sandbox-guest-runtime/tests/owner.rs)使用真实 worker、管道和 pidfd，覆盖多线程初始订阅先于输出、二进制/非零退出与 End 顺序、断线/重连不关闭 stdin、阻塞输入与持续输出下的原执行期限、单个慢订阅失败、在途块到实际额度释放、全部客户端句柄丢弃后的继续管理、预过期无副作用、exec 失败和启动结果未知不重执行。SIGSTOP/waitid、实际输出和可推进时钟建立顺序；真实 I/O 段使用正常时钟，避免假时钟抢跑子进程。丢弃未轮询任务验证 `owner_lost` 和未决额度保留；不将它当作真实 envd 服务强制重启/PID 复用验收。普通基础检查和 Linux SDK 脚本执行这些组件，公开注册/错误映射、保留 TTL、整机内存峰值及 KVM 仍单独验收。

[准备与登记交接检查](../sandbox-guest-runtime/tests/preparation.rs)同样纳入上述两条入口：真实 worker 验证登记确认前无用户副作用、同一额度移交、授权轮换等待至派发、确认接受前后取消的不同语义及丢失回复不重执行。受控握手 socket 与可推进时钟覆盖握手取消、坏协议、绝对登记期限；实际 pidfd/wait 核对中止后退出、私有 gate 回收及容量恢复。测试主动提前 wait 自己的原 child，验证管理者缺失回执时保持 `CleanupUnknown` 和占额；关闭管理 runtime 区分确定未 spawn 与可能已有 child 的责任。没有模拟强制 PID 复用、Linux D 状态、故障文件系统、完整服务重启、Go envd 双端对照或 KVM；这些不由组件绿灯核销。

#311 来宾替换先复用 S02/S03/S11/S12：固定同步/异步 SDK 对固定 envd 与 Rust 实现执行同一合同样例，区分主进程退出、管道结束、输出排空、请求超时和 VM 停止；保留 stdin/PTY、非零退出、部分文件写入、断线/重启/旧 PID 及预算边界。真实模板切换还须通过 S01/S04/S05/S09/S13 的相关路径和原身份清理，不用组件样例代替实机。

[PTY 实进程检查](../sandbox-guest-runtime/tests/pty.rs)通过真实终端核对登记前无副作用、主 PID/session/三个标准描述符、窗口尺寸及 SIGWINCH、原字节大块双向传输、错误输入通道、CloseStdin 拒绝和 canonical Ctrl+D。取消输入保留实际前缀，放行诊断丢失保持未知且不重执行；终端 EOF 先于主进程退出时不得 End。管理组件覆盖断线/重连、输入堵塞时独立 resize、原期限及挂断后的收尾，防止满终端输入上的就绪忙转。SIGSTOP/waitid、受控 socket 和可推进时钟建立观察顺序。基础检查和 Linux 兼容脚本均运行该组；固定 SDK 另核对 PTY 字节回调和非零退出，不代替 Go 双端对照、全部公开 PTY 路由、后代/整机压力或 KVM。

[Rust 命令编解码检查](../sandbox-guest-runtime/tests/protocol.rs)消费原共享样例，覆盖字段存在性、非法/多重选择器、二进制字节、畸形/多余帧及每消息预算。[SDK/真实进程组合](../sandbox-guest-runtime/tests/sdk_protocol.rs)由 Linux `check-compat.sh` 运行：固定同步/异步 SDK 经一次性回环测试路由启动当前账号的真实 worker/命令，检查管道二进制/非零、大输出、信号退出、PTY 原字节/非零及 SDK 终端回调，以及 End 帧截断后实际 `CommandHandle.wait()` 报错。测试路由只接受各场景预先确定的一条命令，无公开登记/鉴权；检查实际主进程退出、输出排空和私有 socket 回收。它不是正式 Rust HTTP 服务、固定 Go envd 双端对照、全部命令操作、PTY 一元操作的 SDK 对照、降权或 KVM 证据。

[文件协议检查](../sandbox-guest-runtime/tests/file_protocol.rs)消费同一文件共享样例，核对重复/畸形消息、缺省/null、uint32 深度、int64 尺寸、可选链接字段、时间戳精度/边界和编码预算。[文件上下文测试](../sandbox-guest-service/src/files/tests.rs)覆盖 Basic 与 HTTP username 的分离、一次 Move 查询、默认值/环境不改变身份、词法路径、无效 UID 和展开后预算。Linux 兼容入口还运行[固定同步/异步 SDK 组合](../sandbox-guest-runtime/tests/sdk_files.rs)：原会话授权与路径解析接到文件编解码，核对各一元方法、大尺寸/纳秒/元数据/空列表/可选链接及超额错误。该测试只返回预定合成属性，没有实际 stat、目录修改或文件传输；真实文件权限、链接竞争、磁盘满、部分读写、Go 双端对照和 KVM 仍须独立执行。

[查询绑定检查](../sandbox-guest-runtime/src/file_query/tests.rs)及[错误回执检查](../sandbox-guest-service/src/file_query/tests.rs)覆盖原字节/空值、畸形字段局部丢弃、固定字段顺序、int64 边界与 unsigned 溢出优先级、完整原始输入预算、非 UTF-8 显式拒绝、Go Quote 线长预算及原守卫/期限。可移植定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_query`。固定 SDK 文件测试路由也迁移至共用绑定：原 SDK 读写带加号/百分号/空格的文件名，合成 GET/HEAD/POST 检查重复/非法过期时间先于鉴权和文件效果，HEAD 核对空正文与原长度头，GET/POST plain text 400 经原 SDK 错误解析器核对；Linux 组合待实跑，不能计为完整 E03 或签名验证。

[查询参考适配器](../tests/compat/query-reference/main.go)使用独立锁定的 Go 模块，直接调用 Go 1.26.8 URL.Query 和 oapi runtime v1.6.0 原绑定函数，字段顺序、类型及错误包装来自固定 envd `f455fd58fa3f289e78ce1de690c35968d8d29322` 的 GetFiles/PostFiles 生成代码；main.go 的 HandlerFromMux 使用原默认 ErrorHandler。对照包含种子 0x311、任意百分号字节、非法/Unicode 输入及 2047/2048/2049 字节响应；HTTP 模式比较 GET/HEAD/POST，只调用 http.Error 的真实自有回环服务，不执行完整 envd 或文件操作。已本地显式运行，默认 ignored，普通 CI 不制备 Go 或下载模块。先按 go.mod/go.sum 准备固定依赖，再执行：

```sh
cd tests/compat/query-reference
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -mod=readonly -trimpath -buildvcs=false \
  -o /absolute/path/to/query-reference .
cd ../../..
SANDPROOF_GUEST_QUERY_REFERENCE=/absolute/path/to/query-reference \
  cargo test -p sandproof-guest-service --locked --lib file_query -- --include-ignored --nocapture
```

[文件鉴权检查](../sandbox-guest-service/src/session/file_authorization/tests.rs)和[会话回归](../sandbox-guest-service/tests/session.rs)可在原 macOS/Linux 库运行；会话与初始化 HTTP 组合不依赖 Linux 系统调用，现不再按平台隐藏。覆盖令牌优先/空头回退、非法签名先于过期、相同到期秒、当前快照后读时钟、字段/操作绑定、全部头和保留字节预算、无初始化拒绝、候选轮换失败/取消、原许可阻止轮换和原绝对期限。401 JSON 与最后字节副本保留原守卫另行检查。请求准入组件另检查 HEAD 的完整编码预算、空正文、原长度头、丢弃和到期释放；Linux 公共文件入口核对拒绝前不读正文/查账户。固定 SDK 组合增加合成 HEAD 鉴权拒绝，尚未 Linux 实跑；HEAD 不经 SDK 文件错误解析器断言不存在的响应正文。定向入口为 `cargo test -p sandproof-guest-service --locked --lib session::file_authorization` 与 `cargo test -p sandproof-guest-service --locked --test session`。

[固定 Go 鉴权参考](../tests/compat/guest_authorization_reference.go)保留原 generateSignature/validateSigning 函数体和 SHA-256/base64 方法；仅将令牌存储/日志依赖换为测试替身，清零函数名及 Unix 时钟注入改变，不执行原存储实现或完整 envd。显式组比较签名和允许/拒绝文本，包括读写、缺省/空用户、Unicode/分隔符路径、无/负数/int64 边界到期时间、错误令牌与错误签名；另经原 jsonError 和真实 Go 回环 HTTP 比较 GET/HEAD/POST 四种原签名拒绝；这些有限对照本地已运行，默认 ignored。未初始化匿名回退和重复令牌差异由受保护配置测试单独核对，不能用此对照宣称两种配置完全一致。

```sh
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/auth-reference tests/compat/guest_authorization_reference.go
SANDPROOF_GUEST_AUTH_REFERENCE=/absolute/path/to/auth-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  session::file_authorization -- --include-ignored --nocapture
```

[文件请求准入检查](../sandbox-guest-service/src/file_requests/tests.rs)覆盖共享池无排队、非法预算、头/查询预算及绑定先于鉴权、专用许可持续占额、400/401 错误对象/正文/最后字节副本的所有权、超时与编码失败后的原名额释放。[会话鉴权检查](../sandbox-guest-service/src/session/file_authorization/tests.rs)另用实际写锁和可推进时钟检查取消/到期的鉴权等待释放原容量且不遗留读者。可移植定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_requests` 及上述会话定向入口。它不代表套接字/请求头读取、正式拒绝策略或系统内存峰值已经验收。

新增 Linux [下载许可组合](../sandbox-guest-service/src/file_transfer/download/tests/request/authorization.rs)和[上传许可组合](../sandbox-guest-service/src/file_transfer/batch/tests/authorization.rs)检查错误操作/到期先于账户和文件效果、原 query 到实际路径/用户、鉴权期限不在正文重新起算，以及在途下载字节保留原槽位；入口请求名额另随阻塞账户查询取消、上传成功/拒绝的最后响应副本和下载原期限保留，这些实际执行待补。固定同步/异步 SDK 文件测试路由全部经共用请求准入取得文件专用许可，另用原 download_url/upload_url 生成签名，仅将 URL origin 改到自有回环夹具，经原查询访问 raw/multipart 上传及下载，检查修改路径/用户/操作、过期、错令牌及令牌优先的 401 文本；这是新增 Linux 组合入口，未执行不得计为通过，不代表公开直连模式、中心/宿主签名 URL 转发、E03 或 KVM 验收。

[共用文件 HTTP 编排检查](../sandbox-guest-service/src/file_http/tests.rs)新增真实目录内 framing 已知/未知长度上传、实际下载、多范围动态边界和完整线长、拒绝时不读取正文/不查账户、满池/错误路由无副作用，以及未映射账户错误保留原请求责任；定向 Linux 入口为 `cargo test -p sandproof-guest-service --locked --lib file_http`，实际运行待补。SDK 文件回环夹具已调用共用 FileHttp，保留原目录前置条件与调用集，未映射错误不再由夹具生成通用 JSON。可移植 [boundary 检查](../sandbox-guest-service/src/file_download/boundary.rs)、[计划检查](../sandbox-guest-service/src/file_download/tests.rs)及[流执行检查](../sandbox-guest-service/src/file_download/stream/tests.rs)核对 30 字节编码、熵失败无回退、仅实际多范围请求调用熵源，以及 worker 生成边界后的实际正文/Content-Length；OS 熵 smoke 不证明熵质量。这些不代替真实 socket framing/连接预算、Linux 文件/SDK、完整 E03 或监听器部署验收。

[未知文件账号响应检查](../sandbox-guest-service/src/upload_response/account/tests.rs)覆盖 GET/HEAD/POST 的固定 401、JSON 转义、Go 小响应 Content-Length 边界、HEAD 抑制、精确响应预算、原期限及最后字节副本释放；可移植测试及固定 Go 有限对照已本地运行。对照复用原 upload reference，显式注入 Go `user.UnknownUserError`，只核对原包装/jsonError/实际回环 HTTP 写出，不代表真实账户数据库或完整端点。Linux FileHttp 检查新增空/缺失/特殊字符用户名、无正文读取/文件效果、编码首头先于账户、账户先于后续阶段、不可用源保持未映射分类；实际执行待补。SDK 文件组合新增原同步/异步 read、raw/multipart write、gzip write 的未知账号 AuthenticationException；SDK 会省略空 user，因此显式空 username 另由合成请求经原 SDK 解析器核对。该扩展也待 Linux 实跑，不能记为 E03 通过。

[路径展开错误检查](../sandbox-guest-service/src/upload_response/path/tests.rs)对照固定 permissions 的原 expand/ExpandAndResolve 和 GET/POST 错误包装、真实回环 HTTP jsonError，覆盖特殊字符、默认工作目录触发时原查询为空、HEAD、响应字节上限及长错误头/正文；该有限可移植对照已本地运行，不执行真实账号或失败路径 I/O。Linux [上传路径检查](../sandbox-guest-service/src/file_transfer/batch/tests/upload_path.rs)覆盖 raw 拒绝不轮询 identity 正文、旧粗分类、identity/gzip multipart 前文件保留且后文件不执行、查询覆盖非法文件名；[下载请求检查](../sandbox-guest-service/src/file_transfer/download/tests/request.rs)覆盖账户先于展开、展开先于 stat/协商、默认值与 HEAD。这些仅已交叉检查，实际运行待补。固定 SDK 组合新增原同步/异步 multipart 批次失败后回读已完成文件，Rust 检查后续目标未创建；其 Linux 实跑待补。原 SDK gzip=True 会强制 raw 上传，因此压缩 multipart 只归入组件场景，不能标成 SDK 发出的压缩 multipart。

[一元真实文件检查](../sandbox-guest-runtime/tests/filesystem.rs)使用独占临时目录，覆盖属性/特殊类型、断链与长链接/循环、xattr 与 UTF-8、词法父目录、深度/别名/顺序、覆盖移动及失败后的父目录、递归删除大批次与部分删除预算。普通 CI 禁止创建 SUID/SGID 文件，真实属性夹具使用普通权限；[权限字符串向量](../sandbox-guest-runtime/src/filesystem/metadata.rs)核对固定 Go 的特殊位/类型格式，实际 SUID/SGID 属性读取保留在下面的显式 Go 对照中，不算普通 CI 的内核权限实测。[删除交错检查](../sandbox-guest-runtime/src/filesystem/remove.rs)在真实目录打开后精确替换为链接，核对目标未被遍历；这是指定交错，不能覆盖全部并发文件系统变化。[文件工作池检查](../sandbox-guest-service/src/file_io/tests.rs)以通道屏障和可推进时钟核对取消/超时后授权与额度、结果保留、异常和释放，未轮询请求无副作用。新增[一元请求组合检查](../sandbox-guest-service/src/file_io/rpc/tests.rs)核对满额/到期先于账户查询、取消/到期后的账户与授权保留、Move 部分效果及结果/错误保留原名额；文件工作池另以单线程阻塞池和屏障核对排队到期不执行及运行中到期不提前归还请求所有权。新增候选只有 Linux 交叉静态检查，Linux 实跑待补。普通 Linux 基础检查与兼容入口执行这些组件。

[真实文件 SDK 组合](../sandbox-guest-runtime/tests/sdk_file_io.rs)通过原授权、真实本地账户、路径解析及文件池，让固定同步/异步 SDK 在各自临时树上实际执行五种一元方法，核对链接/元数据、错误、创建与移动、失败后父目录、删除及配额失败后的正常请求。夹具已迁移到同一 execute_rpc 编排；成功响应使用 FileReply::into_response 的有界编码、原期限和需求驱动正文，错误仍由原合成 Connect 测试映射在 Bytes 中保留实际失败所有者。该迁移 Linux 实跑待补，不计为固定 Go 错误对照。仅测试路由开放回环端口、接受预定目录内路径；没有 HTTP 上传/下载、特权账号切换、正式监听器或 KVM。

[一元响应检查](../sandbox-guest-service/src/file_rpc_response/tests.rs)在 macOS/Linux 使用同一原文件 JSON 编码器，覆盖三个响应形状、编码边界/转义膨胀、原期限、背压、最后字节副本、取消编码 worker 及失败对象的容量保留。共享纯命令/文件协议测试也可移植执行，新增 Vec 扩容不越过线长预算的检查。Linux 一元组合另检查真实 mkdir 完成后编码超限、实际池期限先于原请求期限、初始化不被结果保留阻塞，以及到期正文副本继续持有请求/文件两个槽位；此新增 Linux 组合与 SDK 迁移尚未实跑，不用可移植测试代替。

[Connect 传输参考](../tests/compat/connect-reference/main.go)以固定 Go 1.26.8、原 Connect v1.18.1 和 protobuf v1.36.12 在本机真实 HTTP 上返回同一个已编码载荷，比较八组状态/头/正文，包括 2047/2048/2049 字节边界。参考使用 passthrough 测试 codec，隔离原 Connect 传输；不执行 protobuf 编解码、认证、文件操作或完整 envd，不能视为 E03 双端通过。该显式 ignored 对照已本地执行，普通 CI 不自动安装 Go 依赖。

```sh
(cd tests/compat/connect-reference && GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build \
  -mod=readonly -trimpath -buildvcs=false -o /absolute/path/to/connect-reference .)
SANDPROOF_GUEST_CONNECT_REFERENCE=/absolute/path/to/connect-reference \
  cargo test -p sandproof-guest-service --locked --lib file_rpc_response -- --include-ignored --nocapture
cargo test -p sandproof-guest-runtime --locked --test protocol --test file_protocol
```

新增[流式文件端口检查](../sandbox-guest-runtime/tests/file_transfer.rs)覆盖二进制/空文件、父目录/链接、长度和元数据预校验、截断/超额后保留前缀、元数据替换/清空及打开后路径替换、文件增长和缩短。[传输所有权检查](../sandbox-guest-service/src/file_transfer/tests.rs)覆盖原授权阻止初始化、上传结果/下载块及传输引用占额、消费前不预读、取消后不可重用、显式 EOF 与工作者丢失、空闲期限和过期就绪回复。可推进时钟和明确块释放建立顺序；未模拟内核永久阻塞、真实磁盘满或整机峰值。

[原 SDK 文件传输组合](../sandbox-guest-runtime/tests/sdk_file_transfer.rs)与[同步/异步调用集](../tests/compat/guest_file_transfer_sdk.py)经一次性回环路由连接原授权、真实账号/路径解析和传输池，检查原始字节上传、普通/流式下载、大于块预算的内容、元数据替换、目录/缺失错误及先前成功写入不回滚；新增单文件/多文件 multipart、字节/二进制流/文本流、Unicode 完整文件名、批次元数据、重复路径/目录失败后的前缀及后项不处理。Linux `check-compat.sh` 已登记以上入口，已有原提交的 Linux 组件检查；可定向执行 `cargo test -p sandproof-guest-runtime --locked --test file_transfer`、`cargo test -p sandproof-guest-service --locked file_transfer::tests` 和 `cargo test -p sandproof-guest-runtime --features sdk-contract-tests --locked --test sdk_file_transfer -- --nocapture`。后续改动须重跑，交叉类型检查不能算运行通过。测试适配现支持 raw/multipart POST 和 identity GET；multipart 已有原提交 Linux 组件证据，后续候选仍须重跑；不代表正式路由、下述新增 gzip 组合、range/conditional、固定 Go HTTP 双端对照、E03 或 KVM 验收。

新增[批量上传检查](../sandbox-guest-service/src/file_transfer/batch/tests.rs)覆盖单槽位顺序多文件、一次账户查询、元数据/结果保留、相对路径判重、查询空路径覆盖、合法空批次、取消中间文件、总量/数量/结果预算、后续文件失败与整批绝对期限。结果预算拒绝可发生在实际写完之后，测试要求文件保留而整批失败。Linux 基础及兼容入口中的 service 全测试会执行该组，也可定向 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests`；该组已有原提交的 Linux CI 运行验证，后续改动须重跑。此处直接向组件交入已解码文件名和正文块，没有执行 multipart MIME 解析、网络请求或固定 Go HTTP 对照。

[multipart 解码检查](../sandbox-guest-runtime/src/multipart/tests.rs)可在 macOS/Linux 运行，覆盖逐块和全部切点、近似边界二进制、CRLF/LF、前导/尾随内容、空 part、逐字节截断、前一 part 结束后边界行失败、折叠/重复头、完整文件名、RFC 2231 跨续段 UTF-8、quoted-printable 的部分错误/4096 字节临界值，以及全部输入预算、错误后不可复用和大流中固定解析缓冲容量。执行 `cargo test -p sandproof-guest-runtime --locked --lib multipart`；普通 Linux 兼容入口已有 runtime lib 测试，因此会执行可移植组。该解析组件已有本地实跑，不代表下述上传组合或 Linux 全服务验收。

[multipart 上传组件检查](../sandbox-guest-service/src/file_transfer/batch/tests/multipart.rs)接原批次和真实临时文件，覆盖跨帧/跨写块、QP 二进制/前缀错误、未知字段、查询空串覆盖/缺少文件名、空表单、重复路径、后续边界失败、声明长度/实际正文/空帧/尾随内容/trailers 预算及异常顺序。明确通道信号确认已有文件写完且正在等待下一 HTTP 帧，再测试实际 EOF 前的授权/容量保留、结束标记后的传输错误、中途取消和可推进的原期限。服务全测试与 Linux 兼容入口会执行该组，定向入口为 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::multipart`。该组和扩展固定 SDK multipart 已有原提交的 Linux 组件证据，后续候选仍须重跑；完整基础检查不能由组件绿灯替代，没有磁盘满/故障文件系统、正式监听器或 KVM 的通过声明。

[gzip 检查](../sandbox-guest-runtime/src/gzip/tests.rs)在 macOS/Linux 可运行：`cargo test -p sandproof-guest-runtime --locked --lib gzip`。覆盖空/二进制/多段、所有截断点、CRC/长度损坏、逐项预算、精确上限后的真实 EOF/源错误、错误后不可复用、非法 DEFLATE 回溯距离及大流的小缓冲编码/解码。首个头部失败与正文错误分开核对，后者检查实际可见前缀。本地组件及下述固定 Go 对照已运行；Linux 候选、HTTP gzip/文件副作用与 SDK 压缩组合仍待验证。

[Go gzip 适配器](../tests/compat/guest_gzip_reference.go)仅调用固定 Go 1.26.8 `compress/gzip`。显式组比较所有 flag、额外头、名称长度、截断/校验/尾随错误和种子 0x311 变异的初始化结果、字节前缀和终态，并双向检查 Go/Rust 压缩结果；不要求压缩字节相同。默认 ignored，不把普通 CI 绿灯算作执行过 Go 对照：

```sh
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/gzip-reference tests/compat/guest_gzip_reference.go
SANDPROOF_GUEST_GZIP_REFERENCE=/absolute/path/to/gzip-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  gzip::tests::fixed_go_gzip_reference -- --ignored --nocapture
```

[HTTP gzip 所有权检查](../sandbox-guest-service/src/upload_body/tests.rs)可在 macOS/Linux 执行：`cargo test -p sandproof-guest-service --locked --lib upload_body`。原库代码直接接受受控 HTTP Body，测试首个头部之后按需读取、`Bytes` 副本背压、头部/正文等待取消、同步源轮询阻塞期间保留守卫、原绝对期限、真实 HTTP EOF/尾部错误、编码/展开/帧预算与错误穿过 Body 包装。同步通道和明确轮询信号控制交错；可推进时钟只在等待点确认后使用。该组已有本地运行，只验证传输/守卫连接，不替代 Linux AuthPermit/文件实际副作用。

[gzip 文件组合](../sandbox-guest-service/src/file_transfer/batch/tests/gzip.rs)覆盖一批多段 raw/压缩 multipart、无效头部不打开目标、文件内容前缀与旧 metadata、MIME 结束后 CRC 错误、后续 gzip 段失败、真实 EOF 前授权/容量保留，以及取消/期限/展开预算。入口为 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::gzip`，Linux 基础与兼容入口包含该组；本候选实际运行待补。固定 SDK 文件测试路由另加合成 gzip POST、同步/异步原 SDK 回读、首头错误 400 及校验失败后的数据保留。固定 SDK 支持 gzip=True 上传；此组的压缩 POST 由测试 HTTP 客户端构造，原 SDK 压缩上传在下述错误响应驱动另行检查，不能将合成输入记为原 SDK 上传或完整 E03；扩展组合仍待 Linux 实跑。

[上传头检查](../sandbox-guest-service/src/file_upload/tests.rs)及[共享验证](../sandbox-guest-runtime/src/file_upload/tests.rs)覆盖首字段、忽略空 metadata 后缀、无效/超额值与整个 HeaderMap 的独立预算，已有本地运行。定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_upload`。显式 [Go 上传头适配器](../tests/compat/guest_upload_reference.go)调用固定 Go 1.26.8 ParseMediaType，复制固定 runtime 的 metadata 提取/验证原函数；比较媒体类型选择及 metadata 接受值，覆盖不透明字节、Unicode 大小写/空白、无效可选参数与冲突重复参数。不比较 Go map 随机迭代下的首条错误文本，不代表完整 Go HTTP 上传端点；默认 ignored，已有本地显式运行：

```sh
GOTOOLCHAIN=local /absolute/path/to/go1.26.8/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/upload-reference tests/compat/guest_upload_reference.go
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  file_upload::tests::fixed_go_upload_selection_and_metadata_match_original_methods -- --ignored --nocapture
```

[上传故障检查](../sandbox-guest-service/src/file_upload/tests.rs)使用同一 Go 上传参考的 F 操作，复制固定 runtime 的 processFile 原函数，仅替换日志类型与 I/O 调用名；受控返回不同阶段的 EIO/EACCES/ENOSPC/EDQUOT，对照阶段状态，不自行重写预期表。macOS 参考按 Linux errno 身份适配宿主常量；它不执行文件系统操作，也不验证完整错误字符串。显式入口为：

```sh
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  file_upload::tests::upload_fault_status_matches_original_process_file -- --ignored --nocapture
```

[详细上传 FD 检查](../sandbox-guest-runtime/tests/file_transfer.rs)覆盖目标目录、父路径是文件、目标/父链接循环、悬空链接、NUL 路径与遍历预算前置拒绝；[内部写入检查](../sandbox-guest-runtime/src/filesystem/transfer/upload/tests.rs)覆盖真实临时文件短写前缀后注入 ENOSPC、Interrupted 重试、失败后不可重放、零写、只读 FD 的实际 EBADF 和 O_PATH 的 xattr 列表失败。所有真实资源仅位于各用例私有目录，不填满宿主磁盘或改动设备文件。这两组已交叉检查，Linux 执行待补；受控 ENOSPC 不计为真实磁盘满验收。Linux runtime 全测试包含它们，定向入口分别为 `cargo test -p sandproof-guest-runtime --locked --test file_transfer` 和 `cargo test -p sandproof-guest-runtime --locked --lib filesystem::transfer::upload::tests`。

[上传错误回执对照](../sandbox-guest-service/src/upload_response/errors/tests.rs)通过同一 Go 参考的 J 操作，调用原 processFile、EnsureDirs、os.PathError 和 pkg/xattr.Error，再由真实回环 HTTP 执行原 jsonError；只替换实际 I/O 观察，不启动完整 envd。Linux errno 文本固定，核对全部阶段、普通/空间/配额及未知 errno、原请求未知/零/正/最大有符号长度、控制/HTML/Unicode/无效 UTF-8、xattr 单字节与 2047/2048/2049 字节边界；可移植预算、未分类拒绝及原容量/期限检查一起运行，已有本地证据。显式入口为：

```sh
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  upload_response::errors::tests -- --include-ignored --nocapture
```

[Linux 错误响应连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_errors.rs)验证 raw identity/gzip 的真实父路径/目标目录错误穿过 worker、先前 multipart 文件保留且之后文件未打开、Bytes 克隆持有原槽位、实际 I/O 完成后初始化可继续、错误编码预算失败保留文件并归还容量。定向入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_errors`；已交叉检查，实际运行待补。原固定 SDK 驱动另精确检查同步/异步 raw/multipart 目录异常文本，并新增原 SDK gzip=True 的成功写入/回读及目录拒绝，和合成 gzip 场景分别计数；不模拟磁盘满，该 Linux 组合也待实跑。

[上传头拒绝对照](../sandbox-guest-service/src/upload_response/rejection/tests.rs)使用 Go 参考 P 操作：原 encoding.go 选择、metadata 提取/验证，以及 PostFiles/handleRawUpload 的相应拒绝表达式，再经原 jsonError 的真实回环 HTTP 比较 400、头和正文。覆盖首字段/重复值、无效 UTF-8/HTML/Unicode、键长/单字节值、原字节偏移、单条失败先于整体大小、完整总大小及 2048 字节响应边界；不将摘取分支当作完整端点执行。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::rejection -- --include-ignored --nocapture`，预算与响应副本所有权检查同时运行，已有本地证据。

[Linux 上传头拒绝连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_headers.rs)验证四类错误、首值/拒绝顺序、正文零轮询、无文件副作用、账户调用次数、原槽位/期限、初始化与失败收尾；入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_headers`，已交叉检查，实际执行待补。SDK 驱动另用同步/异步测试 HTTP 客户端构造异常头，交给锁定 SDK 原文件错误解析器检查异常类型/文本；普通 SDK 会在客户端校验 metadata 且不发送异常媒体类型，所以此项明确记作合成输入与 SDK 解析组合，不记原 SDK write 发出的请求，Linux 待实跑。

[gzip 首头/响应对照](../sandbox-guest-service/src/upload_response/gzip_header/tests.rs)使用上传 Go 参考 G 操作：固定 getDecompressedBody 调用 Go 1.26.8 gzip.NewReader，成功只报告首头准备完成，失败走 PostFiles 的原包装表达式和 jsonError 真实回环 HTTP。覆盖全部 flag、首头截断、魔数/方法的单字节、名称/注释 512 字节边界、extra 最大长度及 FHCRC，按多种 HTTP/解码分块比较准备结果或完整错误状态/头/JSON；明确不读取 DEFLATE/正文/trailer。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::gzip_header -- --include-ignored --nocapture`，已有本地运行；空输入与长度/源/预算/期限拒绝、首读 Interrupted 重试另有可移植检查。

[Linux gzip 首头请求连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_gzip_header.rs)核对 raw/multipart 首头拒绝先于账户/metadata/文件，原容量/期限/初始化和最后副本释放，以及传输/预算不被伪装成 EOF；入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_gzip_header`，已交叉检查，实际运行待补。SDK 驱动另加合成空/截断/损坏 gzip raw/multipart 请求，经原错误解析器核对异常文本并由 Rust 检查目标未创建；不记作原 SDK 压缩器产生的输入，该 Linux 组合也待实跑。

[multipart 路径拒绝对照](../sandbox-guest-service/src/upload_response/rejection/tests.rs)使用上传 Go 参考 B 操作：复制固定原 resolvePath 与 CustomPart 方法，仅替换命名空间，路径展开注入已解析观察，不调用真实账户/文件系统；缺失文件名经原 FormName 闸门和原 MIME 参数解析。比较完整 400/HTTP 头/JSON，覆盖完成列表中各重复位置、其他路径数量 0–4 与原 >1 列表阈值、完成顺序、Unicode/转义和 2048 字节头边界。沿用上述 upload_response::rejection 显式 Go 入口，已有本地运行，不代表完整原路由或 Linux 路径展开。

[Linux multipart 路径拒绝连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_multipart.rs)覆盖 identity/gzip 的词法重复、先前文件保留/后项不处理、缺失文件名与查询空串覆盖、详细旧入口迁移、原槽位/最后字节副本/期限/初始化、响应预算失败和文件数量拒绝优先级；入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_multipart`，已交叉检查，实际运行待补。固定同步/异步 SDK 驱动增加原 write_files 重复路径的精确异常与已完成顺序检查；另用合成 identity/gzip 无 filename 表单经原 SDK 错误解析器验证，不能记作 SDK write 发出的输入。Linux SDK 组合仍待实跑。

[multipart 准备对照](../sandbox-guest-service/src/upload_response/rejection/tests/setup.rs)使用上传 Go 参考 T 操作，直接调用 Go 1.26.8 Request.MultipartReader/NextPart：受控 reader 记录首次正文读取，进入该 reader 仅报告准备就绪；零读取错误按固定 handleMultipartUpload 原包装和状态经 jsonError 输出。比较各类型/参数、引号/续段/扩展 boundary、ASCII 单字节与 Unicode、重复 Content-Type 首值的就绪或完整 500/头/JSON；不将注入的 reader 错误作为真实服务错误。沿用 upload_response::rejection 显式入口，已有本地运行；独立检查旧 boundary 粗分类、资源预算和非 UTF-8 参数拒绝。

[Linux multipart 准备连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_multipart_setup.rs)核对 identity 正文零轮询、gzip 首头后的拒绝、旧文件不变、首头字段、账户调用、metadata/类型/预算优先级、正文截断不映射为准备失败，以及原槽位/期限/初始化/编码失败/旧入口迁移。定向入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_multipart_setup`，已交叉检查，实际运行待补。SDK 驱动用合成 identity/gzip 三种准备失败经原错误解析器核对 500 和 SandboxException 的完整文本，并由 Rust 夹具检查文件不存在；不记为原 SDK write 产生，Linux 组合待实跑。

[raw gzip 正文对照](../sandbox-guest-service/src/upload_response/gzip_read/tests.rs)使用上传 Go 参考 D 操作：原 getDecompressedBody 创建固定 Go gzip reader，原 processFile 的 ReadFrom 调用由内存目标实际消费 reader，并保留错误前前缀；文件系统调用仍为受控观察，未执行真实写盘/metadata。完整成功只比较 END 和前缀，失败比较前缀及原 500/头/JSON。覆盖首头就绪后的逐字节截断、全部 trailer 字节逐 bit 损坏、后续成员头/正文、空/二进制/多段，使用多种输入/解码分块；未知 DEFLATE 细节不纳入公开诊断。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::gzip_read -- --include-ignored --nocapture`，已有本地运行；转换检查拒绝首头空输入和源/预算/期限等无对应错误。

[Linux raw gzip 错误连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_gzip_read.rs)覆盖三类 500、已写前缀及旧 metadata、multipart 完成文件的独立错误、identity 输入不套用 raw gzip 包装、长度/展开预算保持原分类、旧入口错误不变、原槽位/期限/初始化/字节副本及响应编码失败。入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_gzip_read`，已交叉检查，实际执行待补。固定同步/异步 SDK 驱动的合成压缩请求新增截断和后续头错误，与原校验损坏一起核对原 SDK SandboxException 的精确文本，再由原 SDK 回读前缀；这是测试客户端构造的错误输入，不能记为 SDK 压缩器发送或 Linux 已执行。

[multipart framing 对照](../sandbox-guest-service/src/upload_response/multipart_read/tests.rs)使用上传 Go 参考 N 操作，运行原 handleMultipartUpload、handlePart、resolvePath、processFile；真实 Go MIME reader 读取正文，ReadFrom 的内存目标记录文件前缀，原 handlePart 到达 metadata 回读时标记完成。仅替换日志/文件系统和既有路径观察，不宣称真实写盘/metadata。比较 EOF 阶段、两类坏边界行、CRLF/LF、忽略字段、后继 part 和已完成文件的观察及完整失败 HTTP；Go Q 操作另核对所有 Unicode 标量值和全部字节的 Quote 转义。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::multipart_read -- --include-ignored --nocapture`，已有本地运行；普通组件检查另核对诊断行预算、失败后不可复用、粗入口兼容及转义后精确响应预算。

[Linux framing 连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_framing.rs)覆盖 identity/gzip 中的原始/QP 文件截断、前一个完成文件/旧 metadata、忽略字段 EOF、两类坏边界行后不打开后继文件，以及头/QP/源/预算的独立分类、旧入口兼容、原槽位/期限/初始化/最后副本和响应编码失败。入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_framing`，已交叉检查，实际运行待补。固定同步/异步 SDK 驱动增加合成 identity/gzip MIME 错误，核对原 SDK SandboxException 文本并由 SDK 回读前缀；输入由测试客户端构造，Linux HTTP/文件组合待补，不记为原 SDK write 发送。

[MIME 头诊断对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/headers.rs)沿用上传 Go 参考 N 操作，比较首行缩进/80 字节边界、缺少冒号的原物理行、非法键/值与折叠后的逻辑行、键大小写/空格、控制和非 UTF-8 字节、无终止空行及真实 EOF、较大响应和前一个完成文件；逐项比较文件观察及原 500/头/JSON。入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::multipart_read::tests::headers -- --include-ignored --nocapture`，已有本地运行。参考 V 操作另在原 MIME reader 的底层注入实际 Reader 失败，核对坏头验证与失败包装顺序，不代称真实 HTTP 断网。可移植 runtime 检查另核对未读 EOF 的提前拒绝、逻辑行未完成时等待、真实源失败参与已有头验证且不合成 EOF、失败后不可复用、头字节/字段数预算及合法头截断的独立分类。

[Linux MIME 头连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_mime_headers.rs)核对 identity/gzip 原消息、前一个完成文件与 metadata、当前文件未打开、明确物理行错误后不再轮询输入、真实源失败时已有坏头诊断优先、本地长度/预算/不支持文本/合法头 EOF 的独立分类、旧入口迁移及原槽位/期限/初始化/最后副本/编码失败。入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload_mime_headers`，已交叉检查，实跑待补。固定同步/异步 SDK 驱动新增合成坏 MIME 头，经原错误解析器核对 SandboxException 的精确文本，并由 Rust 夹具核对目标不存在；不记为原 SDK write 生成，Linux 组合待实跑。

[QP 诊断对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/quoted_printable.rs)沿用原上传参考 N/V，比较裸控制字节、短/非法转义、可接受的字面等号、软换行、4096 字节临界值、已完成前一个文件和实际源失败，逐项核对解码前缀、完成标记与原 HTTP。原满行缓冲对齐差异已改为一致回归；新增 L/K 操作在原方法的底层 Reader 限制单次读取大小，独立于 Rust 内部事件分块，核对普通/软换行/带前一物理行的 4096 临界值，以及长 MIME 头/折叠缩进和 NextPart 满行失败。入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib upload_response::multipart_read::tests::quoted_printable -- --include-ignored --nocapture`，已有本地运行。runtime 普通检查覆盖错误优先级、不可复用、旧粗入口和源/块预算分类，响应检查覆盖转义后精确预算。

[Linux QP 连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_quoted_printable.rs)核对 identity/gzip 的实际文件前缀/旧 metadata/前一个完成文件、坏行后零轮询、真实源失败、本地长度/预算与文件写预算优先级、旧入口及原响应槽位/期限/初始化/最后副本/编码失败。已交叉检查，执行待补。固定同步/异步 SDK 驱动增加合成 QP 失败请求、原 SandboxException 精确文本和 SDK 文件前缀回读；输入并非原 SDK write 生成，本地只验证固定 Go/SDK 错误解析，Linux HTTP/文件/SDK 组合待实跑。

[源读缓冲组件检查](../sandbox-guest-runtime/src/multipart/read_buffer.rs)核对分段提交一次实际读取的调用顺序、原输入预算、物理头行末 CR、数据最后片段才携带 EOF、满行不再读取，以及尾随输入处理；原大流用例同时核对两种解析入口的固定缓冲容量。QP 普通测试另区分同次返回字节/EOF、字节/错误和之后才到来的终态。[Linux 读取边界连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_read_boundaries.rs)按明确源读取大小和事件块配置比较满行完成/失败、文件前缀与 metadata，使用 panic-tail 确认 buffer full 不多读。已交叉检查，实际执行待补；它不替代真实 HTTP/gzip 分块与完整双端 SDK 验收。

[multipart gzip 阶段对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/gzip.rs)新增上传 Go 参考 Z 操作，原 getDecompressedBody 的真实 gzip reader 接原 handleMultipartUpload/handlePart/processFile。Rust 使用实际 GzipBody 与 MIME/QP reader；覆盖 MIME 各阶段切点、三类 gzip 失败、前一个完成文件、忽略字段、坏头/QP 诊断优先级、歧义及近似边界尾部，比较内存文件前缀/完成标记及失败 HTTP。原方法遇 MIME End 即结束，故对照不代称 Rust 后续严格排空与 Go 等价。显式入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib gzip_source_stage -- --include-ignored --nocapture`，已有本地运行。普通 runtime 测试另核对 fail_source 的顺序、不可复用及 EOF/失败互斥，响应测试核对精确预算和不接受 Complete 阶段伪造响应。

[Linux multipart gzip 连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_multipart_gzip.rs)检查三个错误阶段、已完成前项/当前文件前缀和 metadata、QP/头/忽略字段优先级、原编码门控、直接批次/旧入口迁移、长度/展开预算和原响应槽位/期限/字节副本。已交叉检查，执行待补。固定同步/异步 SDK 驱动增加合成压缩错误，使用原异常解析器和 SDK 回读核对文本/前缀；Rust 夹具的 QP 文件检查路径按驱动的实际创建目录修正。这些输入不来自原 SDK 压缩器，Linux 组合仍待实跑，不能计作完整 E03。

[忽略 QP 字段对照](../sandbox-guest-service/src/upload_response/multipart_read/tests/ignored.rs)核对 Close 遇编码错误后保留的原始预读位置、后续文件打开/完成及原边界诊断。固定 Go L 操作独立限制源读取，Rust 内部事件分片另设；包含短/非法转义、软换行、裸控制字节、满行及后续内容，并保留前一个完成文件。gzip 组使用原上传参考 Y：Rust 记录实际解压块序列，Go 在原 gzip reader 外以 ReadFull 按序提供同样字节，末尾零项必须读到真实 gzip 错误，不注入诊断；两端必须消费同一来源序列。仅设相同块上限不足以建立该事实。入口 `SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference cargo test -p sandproof-guest-service --locked --lib ignored_qp -- --include-ignored --nocapture`，已有本地运行。

[Discard 普通组件检查](../sandbox-guest-runtime/src/multipart/discard.rs)覆盖有界预读而非全字段排空、错误/顺序/来源/预算、不可复用和 4 MiB 忽略 QP 流中的固定解析容量。[Linux 忽略字段连接](../sandbox-guest-service/src/file_transfer/batch/tests/upload_ignored.rs)核对 identity/gzip 固定源读取及内部块下的后继文件/旧 metadata、先前文件保留、原错误与响应预算、旧入口和等待中的授权/取消/原期限。已交叉检查，执行待补。SDK 驱动另加入对网络分块稳定的合成成功/满行失败，核对原异常解析和后继文件回读/未创建；Linux HTTP/文件/SDK 组合未实跑，不记作原 SDK write 生成或完整 E03。

[上传顺序检查](../sandbox-guest-service/src/file_transfer/batch/tests/upload.rs)连接原授权、实际账户池与临时文件，覆盖编码/首头/用户名/metadata/媒体类型/path 的拒绝优先级，raw/multipart × identity/gzip 四种组合的一次准入/一次账户查询、首 metadata 值和 raw 短正文前缀。条件变量屏障与可推进时钟核对账户查询取消/超时后原授权和上传槽位仍保留，实际查询退出后才允许初始化及容量复用。定向入口 `cargo test -p sandproof-guest-service --locked file_transfer::batch::tests::upload`；Linux service 全测试包含该组，已交叉检查，实际运行待补。固定 SDK 文件夹具所有 POST 已改接同一组件，原 raw/multipart 与合成 gzip 回读组合须在此候选重跑；无正式监听器、签名/完整上传错误 JSON、特权身份切换或 KVM 通过声明。

[上传回执检查](../sandbox-guest-service/src/upload_response/tests.rs)纳入可移植 service lib，覆盖空批次、空 metadata 省略、字段/键顺序、Go 转义、精确响应线长、无效预算、原期限、逐块背压和响应丢弃后最后 Bytes 副本占额，已有本地运行。原上传 Go 参考适配器新增生成类型的成功数组与固定 PostFiles 的写头/正文调用，通过真实本机 HTTP/1.1 服务核对状态、Content-Type、Content-Length、nosniff 缺省和完整正文；含 2047/2048/2049 字节边界、控制/HTML/Unicode 字符及多文件。参考仅发送合成内存响应，不访问实际文件或启动完整 envd；运行后关闭自己的回环服务。显式入口为：

```sh
SANDPROOF_GUEST_UPLOAD_REFERENCE=/absolute/path/to/upload-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  upload_response::tests::fixed_go_upload_success_bytes_and_actual_http_headers_match -- --ignored --nocapture
```

[Linux 回执连接检查](../sandbox-guest-service/src/file_transfer/batch/tests/response.rs)验证已完成文件/metadata 在响应预算失败后保留，实际 BatchReply 与响应字节使用同一原槽位，文件 I/O 完成后初始化仍可推进，空批次响应后容量可复用。SDK 文件夹具已移除成功数组的独立 JSON 拼装；同步/异步合成 POST 检查空 metadata、Unicode/HTML 文件名和大 metadata 响应头/精确正文，再用原 SDK 回读实际文件。这些 Linux 组已交叉检查，实际运行待补；可移植响应对照不能替代它们或完整端点验收。

[gzip 编码所有权检查](../sandbox-guest-service/src/upload_body/encode/tests.rs)纳入上述可移植 service lib 入口，覆盖打开后不预读、空/二进制/大文件块、固定源缓冲、Bytes 克隆背压、打开和读取阻塞期间的取消/原期限、两种总量边界及源错误不能变 HTTP EOF。本地已运行；不模拟 Linux 文件权限或授权实现。

[Linux gzip 下载检查](../sandbox-guest-service/src/file_transfer/gzip_download/tests.rs)和原 [FD 检查](../sandbox-guest-runtime/tests/file_transfer.rs)增加增长/缩短、路径替换、链接、procfs 非 stat 长度读取、真实 AuthPermit/响应 Bytes 持有、容量/期限及原始/压缩预算。定向入口为 `cargo test -p sandproof-guest-service --locked file_transfer::gzip_download::tests`，基础和兼容入口会发现该组；当前候选待 Linux 实跑。固定 SDK 文件路由另以原 `files.read(..., gzip=True)` 检查同步/异步 bytes、stream、Unicode text、空/二进制/大文件与缺失/目录错误，合成 HTTP 客户端检查未自动解码的 gzip 正文、无 Content-Length 及 Vary；该扩展也待 Linux 实跑。路由已接下述 stat→编码选择→open 组件，完整正式入口与 E03 双端验收仍待补。

[内容编码测试](../sandbox-guest-runtime/src/file_encoding/tests.rs)和[HTTP 字段测试](../sandbox-guest-service/src/file_encoding.rs)可在 macOS/Linux 原库运行，覆盖 q/大小写/Unicode 空白/不透明字节、重复参数、identity/wildcard、原始字节/项预算、第一重复头、条件字段非空规则和 Vary 设置阶段。13 项同权重列表回归要求保留原 Go 选择，不能稳定排序后误选 gzip。定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_encoding`；普通 runtime/service 库检查会发现这两组。组件没有 stat/open/实际范围响应或 HTTP 错误顺序的执行证据。

[编码 Go 参考适配](../tests/compat/guest_encoding_reference.go)复制固定 runtime `f455fd58fa3f289e78ce1de690c35968d8d29322` 的原选择函数，仅改变 package/import 和输入输出驱动，固定 Go 1.26.8。显式组比较浮点 bits/错误、Content-Encoding、Accept-Encoding 和条件 identity 选择，包括种子 0x311 的十进制/十六进制/畸形数值与长列表；本地已运行，默认 ignored，普通 CI 不代为执行，也未启动完整 Go envd。原样函数及 Apache/BSD 许可证随源码保留：

```sh
GOTOOLCHAIN=local /absolute/path/to/fixed-go/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/encoding-reference tests/compat/guest_encoding_reference.go
SANDPROOF_GUEST_ENCODING_REFERENCE=/absolute/path/to/encoding-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  file_encoding::tests::fixed_go_encoding_reference -- --ignored --nocapture
```

[下载决策检查](../sandbox-guest-runtime/src/file_download/tests.rs)及[响应计划检查](../sandbox-guest-service/src/file_download/tests.rs)纳入可移植库测试，定向入口为 `cargo test -p sandproof-guest-runtime -p sandproof-guest-service --locked --lib file_download`。覆盖条件优先级、强弱标签、第一重复字段、旧 HTTP 日期、零时间、Range 解析顺序、空文件/后缀零长度、单段 HEAD seek 要求、多段 MIME 字节/长度、错误头保留及组件预算。已有本地运行；输出为读取计划，不等于真实文件读写、原授权保持、错误收尾或网络 HEAD 行为已验证。

[范围正文执行检查](../sandbox-guest-service/src/file_download/stream/tests.rs)纳入可移植 service lib；定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::stream`。覆盖条件拒绝前不 seek/read、完整/单段/多段/零后缀与 HEAD、准备期和正文期的 seek 失败、短读前缀、无伪造结尾、匿名 HTTP 正文错误、原期限以及 Bytes 克隆背压。阻塞 seek/read 用屏障控制，超时用可推进时钟；确认取消后守卫仍保留到实际调用与源析构结束。本地已运行；共享 gzip worker 重构须同时运行原上传/下载所有权回归。

[Linux identity 范围检查](../sandbox-guest-service/src/file_transfer/identity_download/tests.rs)增加真实临时文件、空/二进制、原 FD 替换/增长/缩短、AuthPermit/容量保留及所选文件预算；定向入口为 `cargo test -p sandproof-guest-service --locked file_transfer::identity_download::tests`。固定 SDK 文件夹具的 identity GET/HEAD 已接该组件，合成 HTTP 客户端在原 SDK 写入后检查单/多段、无交集、空文件、304/412、If-Range 回退及 HEAD，原同步/异步 SDK 文件回读继续同路执行。该组还检查原查询扩展名与解析后文件名分离，以及同一 FD 的嗅探/范围读取。测试路由使用受控内建 MIME 表、UTC 和测试边界；identity 自动嗅探，gzip 只查扩展名，合成 HTTP 检查类型/Disposition 和多段各 part 类型，不表示实际模板 MIME/时区/协商已接通；新增 Linux 组尚未实跑，不计为通过或 E03 全端点对照。

[下载顺序检查](../sandbox-guest-service/src/file_download/stream/tests/negotiation.rs)覆盖 stat/目录拒绝先于编码、编码拒绝不 open、两类 Vary 阶段、原四类条件回退、gzip 不嗅探、HEAD 实际消费但抑制正文，以及 stat/open 阻塞取消时原守卫保留。普通组验证准备 seek 的匿名 500 和头清理；显式 Go 组比较嗅探/长度两阶段故障的状态、11 个头和正文，Go httptest 的 HEAD attempted body 按真实 net/http 抑制规则规范化。入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::stream::tests::negotiation`；原 gzip 所有权及计划器回归须同时运行。新增显式组复用下述参考二进制与环境变量，运行该组时另加 `-- --include-ignored --nocapture`；普通 CI 不代为执行 Go。

[下载错误检查](../sandbox-guest-service/src/file_download/stream/tests/errors.rs)覆盖 stat/目录/编码/open 的 JSON 阶段、初始单段 seek 的 416、HEAD 抑制、错误头清理、精确转义字节预算、未知错误不泄漏任意诊断，以及错误 Bytes 克隆保留原守卫。正文开始后的多段 seek 失败保持正文错误，不补成功结尾。普通定向入口为 `cargo test -p sandproof-guest-service --locked --lib file_download::stream::tests::errors`；显式 Go 对照沿用下述参考二进制及 SANDPROOF_GUEST_DOWNLOAD_REFERENCE，并加 `-- --include-ignored --nocapture`。对照直接调用原 ServeContent、PathError、JSON Encoder，复用固定 envd jsonError/Error 和下载分支文本；Linux errno 表复制自 Go 1.26.8 原文件，只有运行于 Linux amd64 时再核对原 syscall.Errno。macOS 表驱动比较已运行，不能当作实际 Linux 文件错误、完整 HTTP 连接或 envd 双端验收。

[Linux 协商下载检查](../sandbox-guest-service/src/file_transfer/download/tests.rs)检查实际文件/目录错误优先级和 gzip 请求下的范围回退；原[文件端口检查](../sandbox-guest-runtime/tests/file_transfer.rs)检查延迟 open 的路径替换、保留旧 stat 信息及删除后 open 失败。SDK 文件夹具已接统一下载，增加同步/异步合成 HTTP 的大小写/q、范围/条件、重复 Accept-Encoding、HEAD 及 stat/406 优先级；这里只统计固定 SDK 原调用和合成请求分别承担的证据；下载错误 JSON 已由同一组件交付，夹具核对 code/message、UTF-8 Content-Type、nosniff、换行及解析路径。真实 ENOTDIR/ELOOP、NUL→EINVAL、延迟 open 后删除和 File seek errno 另有 Linux 定向回归。以上 Linux 新组已交叉检查，实际运行待补。

[下载账户准入检查](../sandbox-guest-service/src/file_transfer/download/tests/request.rs)覆盖同一槽位内的账户→路径→identity/gzip、range/HEAD，初始化默认值与显式用户名、原查询扩展名与解析路径分离、账户/路径失败先于 stat/编码。条件变量和显式通知确认账户调用已进入，再用取消/可推进时钟核对原授权与文件槽位保留、账户期限与请求期限取早、账户耗时不退还、最后正文副本占额及释放后的正常请求。入口 `cargo test -p sandproof-guest-service --locked file_transfer::download::tests::request`；已交叉检查，Linux 执行待补。固定 SDK 文件路由的全部下载已改用该入口，原调用集和请求计数不变；Linux SDK 组合须在此候选重跑，不借可移植编码检查核销该组。

[下载 Go 参考适配](../tests/compat/guest_download_reference.go)直接调用 Go 1.26.8 原 `http.ParseTime` / `http.ServeContent`，使用合成字节、显式本地时区及 httptest writer；默认错误头开关固定为 `httpservecontentkeepheaders=0`。显式组逐项核对日期秒/纳秒，以及状态、选定实体头和完整正文，只把 Go 生成的随机多段边界等长规范化以便比较。覆盖旧日期/时区、条件组合、GET/HEAD/POST、零时间、范围边界和种子 0x311 的畸形输入。本地已运行，默认 ignored；不是完整 envd/SDK 双端、FD 失败、Linux 或 KVM 验收：

```sh
GOTOOLCHAIN=local /absolute/path/to/go1.26.8/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/download-reference tests/compat/guest_download_reference.go
SANDPROOF_GUEST_DOWNLOAD_REFERENCE=/absolute/path/to/download-reference \
  cargo test -p sandproof-guest-service --locked --lib \
  file_download::tests::reference::fixed_go_download_reference -- --ignored --nocapture
```

[下载 MIME 检查](../sandbox-guest-runtime/src/file_mime/tests.rs)和原[流执行检查](../sandbox-guest-service/src/file_download/stream/tests.rs)覆盖 basename/扩展名、filename 编码与精确头预算、内建/系统表优先级、大小写、空表/错误/超额，以及 304/412 和显式 Content-Type 跳过嗅探、512 字节上限、前缀读取错误、回绕与长度顺序、取消阻塞嗅探时守卫保留。本地可移植检查已运行。Linux 固定路径加载及实际 FD MIME 组已交叉检查，尚未实跑；受控源对照不能代替实际来宾 MIME 数据库证据。

[下载 MIME Go 参考适配](../tests/compat/guest_file_mime_reference.go)直接调用固定 Go 1.26.8 的 DetectContentType、FormatMediaType、filepath.Base/Ext；目录查表复制该版本 mime/type.go 和 type_unix.go 的原函数体，仅变更 package/import 并增加驱动，六个路径指向适配器自己创建的临时文件，运行后仅删除自有目录，不读取宿主 MIME 表。显式组含签名截断/变异、种子 0x311 随机输入、原始文件名字节及受控数据库。已本地运行，默认 ignored，普通 CI 不代为执行；不是完整 envd/SDK 双端或 Linux 实测：

```sh
GOTOOLCHAIN=local /absolute/path/to/go1.26.8/bin/go build -trimpath -buildvcs=false \
  -o /absolute/path/to/file-mime-reference tests/compat/guest_file_mime_reference.go
SANDPROOF_GUEST_FILE_MIME_REFERENCE=/absolute/path/to/file-mime-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  file_mime::tests::fixed_go_file_mime_reference -- --ignored --nocapture
```

gzip 新增固定 Cargo 依赖须按[CI 可信预热](ci.md)制备，普通任务仍离线且缺料失败。源码/本地缓存更新不表示黄金盘完成制备；不临时联网、改弱检查或切换现役模板。

[Go MIME 参考适配器](../tests/compat/guest_multipart_reference.go)仅调用固定 Go 1.26.8 标准库，不调用 envd HTTP/文件端点。它无需外部模块，必须使用经过校验的固定工具链；程序会拒绝其他版本。显式对照保存种子 0x311，以相同输入比较四种 Rust 分块下的文件名、正文前缀、part 完成与请求终态；MIME 头 EOF 的严格拒绝单独核对，其余差异使测试失败。普通 CI 不制备 Go，此参考组默认 ignored，不能把普通绿灯记为已运行；本地已显式运行该组。入口为：

```sh
CGO_ENABLED=0 GOTOOLCHAIN=local go build -trimpath -buildvcs=false \
  -o /absolute/path/to/multipart-reference tests/compat/guest_multipart_reference.go
SANDPROOF_GUEST_MULTIPART_REFERENCE=/absolute/path/to/multipart-reference \
  cargo test -p sandproof-guest-runtime --locked --lib \
  multipart::tests::fixed_go_multipart_reference -- --ignored --nocapture
```

[固定 Go 方法对照](../sandbox-guest-runtime/tests/file_reference.rs)为显式 opt-in，不执行时必须报未运行。以固定 runtime `f455fd58fa3f289e78ce1de690c35968d8d29322`、该版要求的 Go 1.26.8 和锁定模块，采用 Go `-overlay` 把仓库[参考适配器](../tests/compat/guest_files_reference.go)映射为 `packages/envd/sandproof_files_reference.go`，使用 `CGO_ENABLED=0 GOTOOLCHAIN=local`、`build -mod=readonly -trimpath -buildvcs=false` 编译该虚拟文件。记录原源码/工具链与适配器二进制摘要，原 Go 源树须保持未改。该程序直接调用原文件服务方法，不启动 envd 主程序、MMDS、cgroup 或监听器。编译后显式运行：

```sh
SANDPROOF_GUEST_GO_FILES_REFERENCE=/absolute/path/to/pinned-reference-adapter \
  cargo test -p sandproof-guest-runtime --locked --test file_reference -- --ignored --nocapture
```

对照在同一路径重建相同文件树，核对属性、错误和最终路径/类型/模式/字节或链接目标；读取时间戳预先固定且精确比较，新建目录返回的动态 mtime 不比较。空 metadata 的省略与空 map 统一，其他字段不宽松忽略。它是原 Go 方法到 Rust 端口的组件对照，不能替代 E03 同 SDK 的完整双端 HTTP 对照，也不核销权限矩阵、磁盘满、流式传输、进程/VM 重启和 KVM 验收。

[初始化会话检查](../sandbox-guest-service/tests/session.rs)消费同一宿主初始化样例，检查默认值/环境替换、元数据身份、令牌原子更新、同令牌重试、旧授权拒绝、重启、恰好预算和取消后恢复。[HTTP 路由检查](../sandbox-guest-service/tests/http.rs)覆盖真实回环请求、慢正文、断流、共享槽位、重复鉴权头及错误脱敏；[MMDS 交换检查](../sandbox-guest-service/src/metadata/tests.rs)以实际 TCP 服务核对两次请求并接到初始化路由，拒绝重定向、畸形令牌/元数据及截断。屏障与可推进时钟控制取消/期限，服务端观察原连接 EOF，验证无遗留连接驱动。基础检查和 Linux SDK 合同脚本均运行这些组件；它们不是 Firecracker MMDS 实测、命令鉴权集成、固定 Go envd 双端对照或 KVM 验收。

首批[命令/文件纯核心测试](../sandbox-guest-core/tests/completion.rs)由 workspace 基础检查发现，覆盖退出/双管道 EOF 的全部顺序、未消费输出、旧内部身份、持续输出的有限驻留、短写/截断和关闭失败；不启动真实进程或操作来宾文件。[固定协议样例](../tests/compat/guest-service-contract.json)由 [SDK schema 检查](../tests/compat/test_guest_service_contract.py)读取，核对命令、PTY、二进制输入、文件属性及编码。它们不是完整 Rust 服务的协议对照，尚未核销 #311 的 E01–E10。

[Linux 进程端口测试](../sandbox-guest-runtime/tests/process.rs)使用真实私有 worker、pidfd、管道和命令，核对放行前无用户副作用、当前主进程 PID、原字节/非零退出、超过单块预算的持续输出、消费前不得 End、stdin 取消与关闭、独立输入句柄的大块双向传输、后代持管道时主进程退出、旧内部身份与 exec 失败。SIGSTOP + waitid 建立已观察的暂停点，验证放行回复取消仍可能执行一次但绝不再次放行，以及取消输入后的实际前缀；不使用随机 sleep 建立顺序。这是普通 Linux 当前账号的组件检查，不是 E2B 对照、账号降权、强制 PID 复用、服务重启、整机资源压力或 KVM 验收。

请求入口（#290 R1）用[管理路由合同](../sandbox-api/tests/http_contract.rs)、[真实应用锁夹具](../sandbox-api/tests/managed_port/admission.rs)、[取消/回应期限测试](../sandbox-api/src/lib.rs)及 [Linux 网关端口夹具](../sandbox-host-service/src/data_gateway/port_tests.rs)验收：受控宿主持锁时请求满额即 429，不新增后端调用；有效/无效数据令牌均受派发预算限制；控制及后台名额保留；HTTP 取消、回应期限耗尽后阻塞工作继续占额，实际结束/异常退出才释放。固定 SDK 的既有 429 映射与重试检查仍须通过，重试不意味着客户端具备幂等键。以上为真实 SQLite/HTTP 加受控宿主的组件证据，不是 KVM 性能或完整拒绝服务防护证明；远程部署还须验收模块合同列出的 TLS 前端约束。

- 接受意图及 outbox 先提交，再执行外部动作；提交前故障不得产生 VM。启动成功而回执未落盘时，重启认出原 VM，不创建替代身份。
- 启动期限不冒充运行寿命；运行起点与 Running 同事务发布。续期、到期和终止竞争不能由旧定时工作复活或误销毁实例。
- 单写者缓存与独立完整重放比较；SQL-only 事务和不确定提交使关系成立或失效，不能发布未确认状态。
- 编号高水位跨重开保持单调，u64/i64 边界不回绕；准入前耗尽无宿主动作，准入后未知保留原操作。数据库损坏、绑定缺失或旧格式不安全时失败关闭。
- 已接受工作的后续转换、重试和清理额度先预留；新请求容量不足被拒绝，不能抢走收尾额度。物理 Free、逻辑退休和结果过期是不同事件。
- 有限位置重用拒绝旧 incarnation；未知对象仍占用。检查点根先提交再删除覆盖前缀；一个长期活跃或未知对象不阻止无关终态退休。
- 窗口关闭/结果删除后旧公开、私有构建键不能创建替代工作；候选交接、私有结算和公开确认分别检查。
- 按完整部署配置核对 DB/WAL/temp、各表、claim/fence/secret、登记、日志、模板缓存和稀疏盘未来增长。条数下降不等于真实字节有界。

组件入口包括[缓存差分](../controller-service/src/controller/cache_tests.rs)、[身份分配](../controller-service/src/controller/allocator_tests.rs)、[API 耗尽](../sandbox-api/tests/managed_port/allocators.rs)、[有界位置与维护](../sandbox-api/tests/managed_port/bounded_slots.rs)及[模板发布](../sandbox-api/tests/managed_port/publication.rs)。实际宿主观察不能仅由这些测试替身代替。

#292 的[原子本地映射与事实组件](../controller-service/src/controller/internal_inbox/tests.rs)纳入 `scripts/check-distributed-contract.sh` 的原收件测试步骤：准入提交前后与两种未知提交结果均重开真实 SQLite，检查实例/租户映射、原配额、固定位置和编号同进退；模板修订/摘要/形状/owner、期限及配额拒绝不产生本地责任；多租户映射单射且重复请求不重分配。销毁收件与原核心终止同事务，准备动作回执丢失时也不得继续旧创建。原工作流的受控证据只用于组件测试：Ready 使用原发布时刻，事实提交前后故障可重建原结果；Destroyed/逻辑回收还不是 Cleaned，必须核对原 ConfirmRelease；该实例的终局确认及保留位置退休完成前，本地维护不能删掉映射依赖。旧收件格式明确拒绝且不修复。这些不是原应用执行组装、KVM 或两题通过的证据，实机仍按 DK 场景验收。

[宿主应用执行组件](../sandbox-host-service/tests/durable_https/execution.rs)在原 ControllerApplication 和真实 SQLite 上验收创建/重复/重开、丢激活回执恢复、销毁先到、原数据许可即时撤销及排空、物理释放未知保留 Stopped、中心断线后的原到期处理；取消由明确宿主屏障控制，实际工作结束前入口许可不归还。[真实 Redis/HTTPS 组合](../sandbox-center-store/tests/redis_ledger.rs)再检查原本地执行事实已持久而中心丢回执/重启，重发原请求得到同一 VM/寿命，只有收到原 Cleaned 才释放中心额度。宿主物理动作在这些组件中受控，不能记为 KVM、SDK 生产入口或 DK 用例通过。

凭据隔离另以[同库索引故障](../controller-service/src/controller/data_credentials/tests.rs)、[管理层恢复与授权](../sandbox-api/tests/managed_port/credential_isolation.rs)及[两台真实虚拟机](../tests/system/server_credential_isolation.py)分层验收：单个宿主凭据缺失、损坏或有效内容改变只拒绝该对象；健康对象仍可执行命令、读写文件。共享索引缺失、损坏或重复绑定须拒绝数据授权，不自动补造登记；原管理清理仍按生命周期合同进行。实机入口保留原凭据 inode，故障后恢复它并逐台核对精确资源，不代表恶意宿主存储已验证。

## 3. Linux/KVM 验收

使用隔离 Linux x86_64 主机，KVM API 12、cgroup v2、固定工具及镜像摘要；通用配置见[示例](../tests/system/host.example.json)，真实地址、SSH 名称和凭据在仓库外提供。使用 root 私有目录、独占 UID/网络范围、空且已委派的 cgroup 父组；不能修改共享祖先或全局网络来让测试通过。确需全局变更须先获批准并恢复原值。

先运行[宿主预检](../tools/check_kvm_host.py)，它只验证环境，不能代替产品测试。系统脚本参数以各自 `--help` 为准，不复制某台机器的绝对路径。新建私有 Git 检出与构建使用 `umask 077`；受保护可执行文件及祖先不能被组或其他用户写入。环境拒绝须修正执行环境，不能降低身份检查。

所有带断言的执行入口必须在 `-O`、`-OO` 或 `PYTHONOPTIMIZE` 下于资源创建前拒绝；[统一守卫](../tests/system/_assertions_required.py)也要绑定源码摘要。C02 的只读安全导出可例外，但不得借导出模式恢复或清理资源。

| 实机入口 | 必查内容 |
|---|---|
| [C02 固定监督](../tests/system/sdk_c02_acceptance.py) | 批准的精确提交、中心/宿主双产物与固定 Redis/TLS 清单、宿主/探针配置；单轮私有身份、实际角色进程回执、失败收尾和原资源审计 |
| [基础 SDK 服务](../tests/system/sdk_server_smoke.py)、[查询列表](../tests/system/sdk_list_queries.py)、[详情](../tests/system/sdk_sandbox_details.py) | create/connect/get/list/kill、字段默认值及归属；真实启停，不以接口成功冒充资源消失 |
| [寿命参数](../tests/system/sdk_lifetime_values.py)、[运行起点](../tests/system/sdk_running_start.py) | 同步/异步默认与显式参数、延长/缩短、总上限、到期及同库重启 |
| [进程崩溃](../tests/system/server_process_crash.py)、[未就绪中止](../tests/system/server_unready_abort.py)、[启动恢复隔离](../tests/system/server_prepare_recovery_isolation.py) | 宿主动作前后退出、回执丢失、恢复同一 VM、单实例故障不阻塞其他实例 |
| [私有系统盘](../tests/system/sdk_private_root.py)、[文件权限](../tests/system/sdk_file_permissions.py) | root/普通用户、两实例隔离、基础镜像不变、失败初始化不污染新实例 |
| [公网规则](../tests/system/sdk_public_network.py)、[网络回执崩溃](../tests/system/sdk_network_receipt_crash.py) | 禁网/公网/静态 IPv4 的真实 TCP/UDP；宿主管理、私有目标及其他沙箱拒绝；丢回执恢复原规则 |
| [网关慢流](../tests/system/gateway_slow_boundaries.py)、[上传中断](../tests/system/transfer_interruption.py) | 命令/文件超时、断线、大输出、迟到拒绝响应、撤销后中继停止并回收 |
| [磁盘预留与耗尽](../tests/system/server_disk_reserve.py)、[物理文件系统组件](../sandbox-server/src/storage.rs)、[固定物理配置](../sandbox-node/src/physical_profile.rs)、[有界轮转](../tests/system/server_bounded_churn.py) | 空间不足仍能收尾、同文件系统要求与跨文件系统拒绝、部署 profile 与真实字节、位置复用及旧动作拒绝；组件通过不代替新入口的实机压力验收 |

网络测试须按当前合同报告 IPv6 与 DNS 边界，不把来宾配置或仅拦 TCP/53 当作完整 DNS 防绕过。静态规则使用合同中的部署配置，不虚构固定 SDK 没有的创建字段。DNS 域名过滤/严格转发控制等未交付能力不能计入通过。

C02 原始目录含凭据、配置及完整日志，只在私有目录保存；`--export-run` 输出有限脱敏摘要。安全导出须确认原执行结束/锁状态，不因丢失结果补造通过。监督器无法确认收尾时保留未知占用和现场；禁止全局杀 Firecracker、宽泛删除 jail/网络/目录。清理后核对原 PID/启动时间、目录身份、namespace、cgroup、磁盘及源镜像摘要。

## 4. 通用模板与 Terminal-Bench

当前使用[冻结的 2.0 兼容回归](../benchmarks/terminal-bench-2.0-suites.json)，题集与批次大小由配置选择，使用通用模板链，不按题目增加产品逻辑。已登记解答修补只在评测层显式启用、核对原脚本摘要并单列结果，原评分/资源/时限不变。2.1 原题全部 89 题为发布前待办，其完整锁和[配置](../benchmarks/terminal-bench-2.1-suites.json)保留；不把旧成绩或修补版结果合并为原题通过。[评测入口](../benchmarks/README.md)维护版本锁与命令。

- 先核对固定 Harbor 与 SDK 联合依赖，再固定全题集 ID、任务文件、环境/原解答/原评分和资源/阶段时限。
- 用户 Dockerfile/公开 OCI → 通用构建转换 → 不可变 Ready 模板 → 全新沙箱 → 原初始化 → 标准解答 → 原评分 → 销毁与精确残留检查。不得为某个题目改产品行为。
- 产品功能以[自助模板链](../tests/system/template_self_service.py)、[公开 OCI](../tests/system/server_public_oci.py)和[通用题目监督](../tests/system/harbor_generic_chain.py)验收；选定真实 Dockerfile 多阶段、ENV/WORKDIR/USER、CMD/ENTRYPOINT、错误产物和发布/清理故障也须覆盖。
- 原题资源与阶段时限不能静默放宽；资源不足、网络外部依赖、原解答/评分问题和系统缺陷分别归因。初始化/标准解答退出 0 不足以通过，还须原评分 reward=1。
- 所有固定任务均进入报告，不删失败或未运行题。版本、原始/本地修订模式、环境变体和跨提交结果分开；修订必须列出改动与原因。
- 正式题目与构建服务在同一宿主配置下核对资源争用、同租户/跨租户容量拒绝及收尾；普通串行实跑不表示满载压力通过。
- 先以少量诊断任务打通完整链路，再跑固定全集；不要求接付费模型。每次重构重跑受影响合同与系统场景，阶段晋升按开发规则执行。

本阶段先验收少量 2.0 兼容题与真实自动检查，全部兼容题和 2.1 原题全量未运行不计完成。人工诊断可按 owner 要求跳过外部网络阻塞并在独立身份中换题，不分析或修复网络；原非成功状态、实际范围和精确清理仍由自动报告保留。固定自动回归不动态换题，不能靠跳过得到正式绿灯。特殊单次晋升豁免只记录于对应 PR。

稳定晋升配置：2.0 `ci` 配置两题 polyglot-c-py、db-wal-recovery；原解答和原测试断言保留，只在独立派生任务中用预装 Python/pytest 替换联网评分启动。使用不可变干净模板、全新沙箱和私有工作盘，禁用公网；不复用解答后的磁盘或旧成绩。结果为 `prepared-ci` 环境及 `ci-preinstalled-v1` 评分启动变体，不是原环境、原评分启动或通用模板构建的验收。鉴权、故障恢复、流式边界仍由相关定向测试检查；构建/镜像/平台安装工具改变时补通用构建实机测试并刷新受影响模板。完整 2.1 原入口 89 题在网络合格主机上作发布前验收，不阻塞每次晋升。

当前检查为 `E2E / terminal-bench-ci`，专用 App 已接通，两级晋升的实际规则见[开发指南](../CONTRIBUTING.md#5-github-配置)。部署/更换时须在同一精确提交上实测两题成功、真实评分失败及随后干净成功，绑定模板/评分依赖清单摘要、候选二进制和基线，独立核对清理；缺包不回源、不跳题、不伪造通过。证据留 #14，不在本页维护另一份运行进度。2.0 smoke 四题、十六题及全量入口保留供显式扩展验收，不混用预装和通用构建报告。

### 配置题集自动验收判据

- 每批大小由批准配置确定，题内串行；使用锁定的 OCI 或原 Dockerfile/context 经通用模板链准备。2.0 的原解答/已登记修补模式固定且分批隔离；2.1 只能使用原解答，不接付费模型，不修改原评分/资源/时限。
- 每题评分 reward=1，且原沙箱、构建、服务、代理的进程/网络/cgroup/jail/磁盘均精确收尾。少题、非零退出、超时、失败或清理未知均不得绿灯；检查名称独立为 `E2E / terminal-bench-2.0-compat` 与 `E2E / terminal-bench-2.1`，手动覆盖另加 `/ diagnostic`。仅配置全部选中题通过才成功；完整锁全部通过才记全题集，兼容版不能称原题全部通过。
- 候选/基线提交、配置、工具及任务逐文件摘要固定；每批报告记录可启动模板摘要。分批汇总结果先持久保存，再回收该批已确认的私有产物。
- 普通 CI 与 KVM 共享准入锁；先暂停新准入，等待全部原任务退出才独占，不能取消已有任务来冒充空闲。只有所有验收资源确认停止后才释放自己的标记。
- 准备与批次使用独立 systemd 单元；管理服务重启接管原身份，不盲目重复提交。启动回执未知且单元无法认出、清理未知时保留原占用，发布失败并要求核对；回写终态丢失只重试同一检查，不重新跑 VM。
- 实测真实成功、失败和随后干净恢复，并验证原单元接管、管理凭据不进入准备/批次/来宾、外来资源不被清理。组件故障夹具不能代替这些宿主验收。
- App 授权、受控安装和两级晋升门槛部署以对应 issue/PR 实测为准；保护要求专用 App 的精确检查名称，手动 `/ diagnostic` 或其他 App 同名检查不能代替。未返回成功不得晋升，不配置空绿色工作流。安装、私有配置与安全边界见[CI 运行环境](ci.md)。

## 5. 中心与宿主验收（组合验收待完成）

本节定义 #292 的必需验收；下方区分已接入的组件检查和仍需实测的完整链路，脚本存在不表示场景已通过。合同依据见[中心与宿主高级操作](module-contracts.md#中心与宿主的高级操作合同)，性质与外部条件见[验证设计](formal-verification-design.md)。首先在同一物理 KVM 主机运行中心、Redis 和一个真实节点，各自独立进程/账号/目录，经版本化双向认证 HTTPS 通信；不使用同进程调用、免鉴权或免持久化替身冒充真实闭环。多台真实宿主另立后续任务，关单范围见蓝图。

普通隔离 CI 已在现有 SDK 合同任务内接入 [check-distributed-contract.sh](../scripts/check-distributed-contract.sh)，使用固定真实 Redis、专用回环数据目录和受控节点/时钟/证书运行下列组件检查；不增加任务检查名称，不访问 KVM 或生产凭据，Redis 不能全由假库替代。脚本包含共享身份、真实 TLS、Redis/SQLite、中心服务与唯一产品入口的定向检查，并明确输出未验收项；固定依赖制备与普通执行环境见[CI 运行环境](ci.md)。这些组件检查不能代替下列 DK 故障组和两题的候选实机验收。

KVM 继续使用可信外部监督器，候选同一提交离线构建 `sandproof-center` 与 `sandproof-host`，实际核对两个 `contract-versions`，绑定二进制/协议/业务信封/宿主格式、固定 Redis、证书配置、模板和完整宿主预算摘要。每轮独立 Redis/SQLite/登记/端口/实例身份；固定受保护测试证书须核对摘要、双方用途与名称，不能关闭证书验证。中心与 Redis 分别为不同非 root 身份，宿主为 root；SDK/Harbor 只指向中心，经真实双向认证宿主启动运行。下列 DK 故障组在两题前执行；只有故障组、两题原定判据、精确收尾与报告均通过才发布 `E2E / terminal-bench-ci` 成功。工具合同源码和自测不代表整条链已安装/验收；新监督链完成真实正负例及取消收尾前，当前已安装门槛保持，不以旧链绿灯替代新架构结果。

两层预算计入原受控准入/独占，不取消他人任务或清空全局资源。故障/缺材料/超时/未知与未运行不能绿灯；结果留 CI 附件和 issue，不每次回写文档。

### 已实现的通信基础检查

独立通信基础的组件入口为 `bash scripts/check-distributed-contract.sh`：同一脚本运行 `cargo test --locked -p sandproof-protocol --test internal_contract` 和 `cargo test --locked -p sandproof-internal --all-targets`，对应测试也随工作区基础检查执行。它们使用独立回环端口和临时测试 CA/证书（仅测试依赖 rcgen 0.14.10），实际交换 TLS/HTTP 字节，不使用生产凭据；核对完整身份/计数器拒绝、双向证书链与登记摘要、证书名/用途、只读路由、两端头/正文边界（含错误响应和分块超额）、握手/完整请求期限、超额即拒及调用方取消后的原连接回收。定向运行不代替开发指南的完整基础检查，实际结果绑定 PR 提交。

本段传输测试只覆盖 DC02 的认证/身份基础与传输预算；高级操作、持久映射和真实 Redis 的组合检查见下一节，不表示 DC02 整体、S1 或任何 DK 用例通过。只读探测也不作为节点健康/可接单证据；双服务实机部署及新 KVM 监督链的正例、负例与取消收尾仍须独立验收。

### 已实现的存储基础检查

同一组件脚本还执行 `sandbox-center-store` 的默认检查和 `redis-tests` 真实引擎用例。运行前把输入锁中的官方 Redis 归档下载至独立临时目录、核对 SHA-256，再编译 `make MALLOC=libc BUILD_TLS=no`；通过 `SANDPROOF_REDIS_SERVER` 提供该构建的绝对可执行路径。版本不符或缺失立即失败，不自动联网安装、使用系统版本、mock 或 skip。此测试 feature 只控制夹具，不提供产品运行模式。测试创建私有目录、独立回环端口和非生产凭据，仅停止/回收本轮子进程及目录。

存储用例覆盖单块 CAS/旧修订冲突、原字节替换拒绝、缺账本不初始化、格式/部署/TTL/损坏/超额拒绝、超过 Lua 整数精度的修订及耗尽、必需持久化配置、实际 noeviction 拒绝、访问凭据/写者身份、引擎卡住时的期限、Redis 崩溃恢复、截断 AOF 拒绝且不修复，以及实际提交后丢失回执时的围栏和耐久重确认。单独的存储测试不宣称无半额度/无重复 VM 的组合结论；实际结果绑定 PR 提交。

同一脚本也运行中心纯决策与原 SQLite 持久收件：多节点确定性负载选择、租户/平台额度、健康/物理空闲/模板否决、未知保留绑定、销毁超越迟到就绪、停止不释额、固定寿命、续期与销毁槽边界；收件包含全文/摘要去重、错身份/窗口/非规范化参数、销毁先到、提交前回滚/后重启以及 SQLite 提交结果未知的两种耐久结果。真实 Redis 与真实 SQLite 既使用明确标注的组件传输替身，也通过真实双向认证 HTTPS 组合，验证提交→异步派发→持久收件、写库后丢回执保留原身份/额度、原请求重放与迟到隔离；HTTPS/SQLite 另验证重开原库及 HTTP 取消不提前返还实际阻塞工作名额。它们不是已部署双服务或 KVM 验收。DC01–DC06/DC08–DC09 的组件覆盖不是整条链路完成；唯一产品入口、本地执行映射、确认/退休与容量仍须在同一候选上通过对应实机矩阵，全部 DK 和监督器受控安装验收不得省略。

### 5.1 组件用例（DC01–DC11）

最小真实创建／销毁的组件证据可通过[独立实机入口](../tests/system/internal_lifecycle_smoke.py)手动验收：普通用户以 `cargo build --release --locked -p sandproof-center-store --example kvm_lifecycle` 构建[组件装配](../sandbox-center-store/examples/kvm_lifecycle.rs)，配合同源码 `sandproof-host relay`；保护精确提交/固定产物后按 `--help` 提供批准配置。它沿用原独占、真实 Redis/mTLS/SQLite/VM 及精确资源审计，不下载镜像、不接付费模型、不改全局网络；仅为组件装配，不是另一生产入口或旧公开服务降级。

正式 SDK、Harbor 与晋升必须使用唯一中心/宿主 `initialize|serve|audit --config` 入口及[共同角色夹具](../tests/system/_center_host_services.py)，不能用组件示例代替。故障明确作用于中心、某个宿主或 Redis，重启保持原受保护配置与账本；公开客户端只访问中心。全局实例与本地 VM 编号通过原已校验宿主收件审计映射，不解析编号后缀。只读中心审计须停止该中心并取得原写者租约；无法观察原请求是否提交时保留未知责任，不将缺记录猜成 Cleaned。成功要求同一 Ready/Cleaned 身份、正确寿命与额度、实际进程账号/目录/cgroup、原 VM PID/时钟/网络/磁盘以及最终残留核对。缺角色、故障组、两题或精确收尾不能通过，安装与结果绑定 issue/PR 精确提交，不仅凭源码关闭 #292。

| 编号 | 注入/输入 | 必需结果 |
|---|---|---|
| DC01 | 两个受控节点、多租户、并发创建及额度边界 | 固定绑定不冲突；平台/租户总额度不超限；本机拒绝不误记成功 |
| DC02 | 重复同操作、同编号异参数、错证书/租户/节点代际/协议 | 原结果复用或明确拒绝；无新增副作用 |
| DC03 | 中心提交前/后中断、提交回复丢失、发送后回执丢失 | 未提交不派发；未知只查/重发原请求；不分配第二个实例 |
| DC04 | 销毁先到、迟到创建/旧回执、过期窗口 | 不复活；旧请求明确拒绝；新实例不受影响 |
| DC05 | Redis/CAS 冲突、无空间、持久化异常、截断/错误类型/格式 | 无半准入/额度丢失；停止新派发；不静默初始化或造成功 |
| DC06 | 心跳过期/节点失联、清理回执未确认 | 不换宿主、不提前释放额度；恢复后按原身份确认 |
| DC07 | 全局 get/list 筛选/排序/分页、模板别名变化；续期确认后从另一入口查询 | 现有固定 SDK 合同保持；租户隔离；原不可变模板不改变；查询对应调用期间合法中心视图，已确认续期不被旧缓存覆盖 |
| DC08 | create/connect/set_timeout 重复/乱序、时钟偏差与期限边界 | 不重置原起点；未确认续期不报成功；到期撤销保持 |
| DC09 | 生命周期未知与退休、有限请求窗口/结果、身份耗尽及已结束键/引用回收 | 未知不回收；保留结果不提前驱逐；旧请求不再执行；驻留条数/字节随声明预算有界，不随历史总请求无限增加 |
| DC10 | 数据鉴权/路由、断线、慢流、超额、终止与流并发；受控端点登记丢失/消费者失效 | 两端资格和预算生效；不重执行或仅凭 PID 接管命令；不将通道结束当 VM 停止；局部失败按范围处理，其他合格消费者在声明条件下推进；无任意地址转发 |
| DC11 | 模板下载中断/损坏/无权限/无空间与重新登记 | 不发布错误模板，不替换 base；仅回收原临时资源 |

测试使用明确故障点、屏障及可推进时钟，不靠随机 sleep。两节点替身使用相同协议和本机端口合同，不另造一套业务规范。存储适配必须使用真实固定 Redis；内存假库不能代替 DC05。

### 5.2 单台真实 KVM 用例（DK01–DK06）

| 编号 | 场景 | 必需结果 |
|---|---|---|
| DK01 | 统一入口创建→get/list→命令→文件→续期→销毁 | 固定 SDK 实际调用及字段符合合同；真实停止；原资源无残留 |
| DK02 | VM 已就绪、节点事实已持久化，丢弃发往中心的回执并重启中心 | 原 VM 仍是同一个；恢复连接后可访问；不重启/不双跑、不重算寿命 |
| DK03 | 已接受启动/停止动作的节点服务强制中断及重启 | 保留原 SQLite 和资源身份，本地对账后向中心确认；未知不虚假成功 |
| DK04 | 受控网络中继阻断中心→节点、销毁竞争及到期 | 中心不换节点/不提前释额；节点本地期限执行；恢复后精确结算 |
| DK05 | Redis 正常重启及中断窗口；保持 VM/节点独立存活 | 新控制写入按合同暂停，恢复原账本；无重复创建/无权限降级 |
| DK06 | 两用户、固定活跃上限的连续创建销毁及历史退休，含两层重启和迟到请求 | 资源/日志/记录符合部署预算；旧请求拒绝；收尾预留有效 |

DK06 的循环次数、采样点、数据规模、期限及预算在验收配置中固定并纳入摘要，不写死为生产上限；不得只跑一次创建销毁就记作长期容量验收。采样包含两层受管驻留键/订阅/池/关联及队列规模，报告内存/条数/字节与预算的对应；库/内核剩余开销单列，不以计数稳定宣称整进程内存已证明。进程、网络、磁盘、数据库/Redis/AOF和日志残留按原身份观察，失败保留有限脱敏证据再精确清理。清理未知保留占用，不宽泛停机/删目录。

两台真机后续任务至少覆盖：同一用户跨节点、两节点全局额度竞争、真实模板传输、节点整机关闭而中心存活、网络分区/恢复、时钟与资源差异。其结果另记，不能由 DK 用例替代。

### 5.3 旧公开入口退出验收

以下是 #292 公开链路切换及关单的必需检查，不表示已经完成。新链路通过对应验收时，同批清除被替代的生产组装和工具引用；在新链路未接通前不提前删除仍在使用的服务。只保留一种生产架构，不提供开发阶段旧库升级或双模式部署。

- 公开 SDK 生命周期、命令/文件及模板用户入口均指向中心；宿主不安装公开用户鉴权表，不接受用户 SDK 管理请求。复用 DC02/DC10、DK01 的身份和端到端场景增加直连宿主拒绝断言，不新建重复故障矩阵。
- 中心或 Redis 不可用时，沿用 DC05/DK05 的失败关闭、固定绑定及本地收尾检查，并确认没有旧公开入口回退；不把节点可独立执行到期/恢复误当降级公开服务。
- 查清并删除原公开 SDK→本地应用组装、宿主全量用户密钥字段、旧模式开关/别名、被替代的配置样例和启动夹具引用。模板构建工具也迁移；保留唯一的本地状态机、SQLite、执行/恢复算法及其组件故障测试，不以清理旧入口为由删掉覆盖。文档/脚本/依赖引用与真实服务产物一致。
- SDK/Harbor 正式验收和两级晋升均运行同候选中心、Redis、真实宿主，经中心进入，报告绑定全部实际产物与协议；不能用旧链路的绿灯代替。普通组件 CI 与可信监督器的分批切换、固定材料制备及安装门槛只在[CI 运行环境](ci.md#中心与宿主拆分的-ci-扩展)维护。

## 6. 证明验收

按[证明工作流](proof-workflow.md)执行固定工具链、重新抽取、全部 Lean 编译与依赖审计。当前已证明输入与缺口只看[覆盖记录](proof-coverage.md)；基础、SDK、KVM 或题集绿灯均不能替代证明。
