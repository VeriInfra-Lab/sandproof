# T7/T8 存储与 OS：有限使用域对账

返回[证明工作流](../../../docs/proof-workflow.md) · [V0 验收点](../../../docs/formal/00-foundations.md#t0t8-公共底座验收点) · [权威依赖合同](../../../docs/formal/dependencies.md) · [覆盖记录](../../../docs/proof-coverage.md)

这份材料审查 integration `e576a8475043c2d21b68231b7f6b1ead09be1952` 的既有使用域，不增设业务规格或批准可信保证。机器可核对目录在 [os-boundary-catalogue.json](../../os-boundary-catalogue.json)。`registered_contract` 仅表示可以用现有 A1–A12/B1–B4 描述该类底层操作，不表示整函数已验证或每个实际调用都已满足条件。

本目录的状态与待办是上述审查基线的历史快照；当前交付状态见 #333 和覆盖记录。`business_obligation` 明确表示仍待业务模块证明，不能读成已证明。后续集成 #617 或 TLS 成果不修改这份历史证据的含义，也不因历史待办文字重新打开已经交付的接口。

## 有限范围与证据

沿用已发布 T0 的 center、host、window-reducer、guest、vm-cli 五根、dev/release、Linux x86-64 共60个编译变体。原捕获提交为 `a9ea9e3d`；本次在同输入的e576副本上实际执行原 `check-published`，818项Rust/Cargo来源、配置、完整MIR/索引、编译回执及日志摘要均通过，未重新编译捕获。目录另外保存确切函数位置、全文/片段摘要和解释。

MIR搜索只帮助找代码；它可能匹配类型和值操作，不能证明不存在其它调用。编译体可跨profile/root重复，数量不能解释为唯一API数、执行次数或定理数。cfg(test)、examples及额外SDK/diagnostics等feature不在这次捕获内。动态trait目标、依赖内部析构、future挂起点取消和外部命令内部仍需具体绑定；模板实现和T9只在证明责任上排除，代码没有从来源中删除，Guest安全没有排除。

query_row/query_map/Row.get、完整codec/Lua、驱动和Guest的全部业务原式没有因这份目录通过就变成“可直接抽取”。业务实例先沿已有窄接口模式做原工具预检；若遇到实际签名缺口，先给具体最小入口再判定归属，不能提前保证所有原式都能复用。

本批没有新Rust算法、抽取根或Lean程序定理。局部清单负例检查防漏、过时和越权标签；它们不是真实Rust语义反例。既有正确/错误实现的证据仍按原模块记录，不重复认领。

## 接口族与归属

每行在JSON中分别给出：原函数及适用版本、原对象/句柄和输入配置、正常结果、错误/部分效果、未知/取消/异常、修改范围、进展条件和下游义务。表中源码入口是可核对的代表定位；并不声称靠这些窗口已证明全体使用点。

| ID | 接口族 | 当前公共状态 | 自有业务提供方 |
|---|---|---|---|
| redis-session | Redis 原连接、超时和诊断 | 已有依赖框架；本行仅来源审查 | V2 |
| redis-result | Redis 提交、重确认和未知围栏 | 已有已集成代表；生产全路径另查 | V2 |
| redis-scripts-codec | Redis READ/CAS 文本、编码与业务顺序 | 现有依赖之上的下游自有决定 | V2 |
| writer-file-lock | 原 open-file-description 的单写者锁 | 已有依赖框架；本行仅来源审查 | V2, V5, V6, V8 |
| sqlite-config | SQLite 原数据库、格式与有效设置 | 已有依赖框架；本行仅来源审查 | V5 |
| sqlite-update | 固定五参数 UPDATE 与计数解释 | 已有已集成代表；生产全路径另查 | V5 |
| sqlite-commit-cache | 原事务提交回执与缓存发布 | 已有已集成代表；生产全路径另查 | V5 |
| sqlite-ledger-recovery | SQLite 完整账本、checkpoint 和恢复 | 现有依赖之上的下游自有决定 | V5 |
| protected-fd | 受保护路径到原 FD 的读取 | 已有依赖框架；本行仅来源审查 | V3, V5, V6, V8 |
| file-staging-sync | 暂存、短写、rename 和目录同步 | 现有依赖之上的下游自有决定 | V5, V6 |
| exact-cleanup | 原目录/对象的删除与退休证据 | 现有依赖之上的下游自有决定 | V6 |
| disk-copy-space | 私有磁盘、稀疏复制与空间观察 | 现有依赖之上的下游自有决定 | V6 |
| helper-child | helper Child 与原 Slot 的停止观察 | #617候选待集成 | V6 |
| pidfd | 首次身份选择、pidfd 与退出观察 | 现有依赖之上的下游自有决定 | V6 |
| firecracker-api | Firecracker/jailer 配置、启动与 API | 现有依赖之上的下游自有决定 | V6 |
| cgroup | 原 cgroup 身份、成员和有效限制 | 现有依赖之上的下游自有决定 | V6 |
| network-rules | netns、tap、nft 与所有出口路径 | 现有依赖之上的下游自有决定 | V6 |
| tls-config | TLS 版本、双向身份与 pin 判定 | 已有已集成代表；生产全路径另查 | V3, V7, V8 |
| tls-original-provenance | 原 TLS 借用连接到候选核心的来源连接 | 公共来源连接缺口 | V0, V3, V8 |
| byte-stream | 短读短写、半关闭和双向中继 | 已有已集成代表；生产全路径另查 | V8 |
| guest-command-pty | Guest 命令、原管道/PTY 与真实退出 | 现有依赖之上的下游自有决定 | V8 |
| guest-files | Guest 文件路径、账号和部分文件效果 | 现有依赖之上的下游自有决定 | V8 |
| clock-deadlines | 时钟值、deadline 与进展条件 | 已有依赖框架；本行仅来源审查 | V2, V5, V6, V7, V8, V9 |
| crypto-bootstrap | 密钥/随机值、摘要和受保护配置 | 已有依赖框架；本行仅来源审查 | V3, V6, V7, V8 |
| template-boundary | 排除模板实现，保留核心使用义务 | 明确排除；消费端义务保留 | V6 |

## 公共底座怎样停止扩张

T7已有Redis结果/未知围栏、SQLite提交与缓存、固定五参数UPDATE代表及原工具接法。调用者继续证明实际脚本/codec、全部SQL、同事务记录、缓存/outbox、耐久恢复和派发；这些归V2/V5，不要求V0先建通用SQL/Lua验证平台。错误不能统称回滚，SQL成功不能统称持久提交。

T8已有流历史和中继代表、TLS首叶/pins候选；#617又提供原Child/Slot的动作代表，但尚未在此基线集成。剩余公共收口条件是统筹完成#617接线/完整验收，以及处理下述唯一新增接线提议。目录需由统筹对应到权威依赖、覆盖和T0使用清单；它本身不勾选整个T7/T8或V0。

**唯一提议：原TLS借用输入的来源连接。** 预检固定原 `verify_pin/peer_pin` 借用根或一个最小输入视图，建立原stream、该次peer_certificates、首叶次序、原pins和SHA字节到已证核心的对应。首叶选择/比较/错误仍透明证明，优先避免复制整条peer链，不把整个verify_peer列成可信。不开发新TLS平台，不重复#617，不改生产代码或工具。

这个提议只使用A6/A9/A12/B4。若实际需要额外可信承诺，必须先列明并等待人类确认；本目录不作批准。现有候选的正常分配、复制/容器开销必须给下游可用条件，并由V3/V8/V9实例化并发和整体内存预算。不能把本地identity的8张证书限制套给任意peer，不能把单TLS载荷界或超时当整个堆内存界，也不能把OOM/abort变成认证拒绝。

V6还负责SafeLaunch、首次原对象选择、实际cgroup/网络/进程、记录/恢复、全部后代和精确清理/释放。V8负责每个方向的字节/短写/半关闭/协议完成、全部buffer与会话/命令。启动接受、就绪、停止、清理、排空、释放分别需要证据，不能由一个命令退出码替换。有限目录不会把这些业务代码改列为可信边界。

## 检查与复现

普通测试在普通测试机执行；源码/MIR来源复核在证明机执行。证明机新增source、tmp、构建和证据放数据盘，原捕获只读，遵守既有native/formal重任务锁；不占锁等待其它机器或本地修改。没有新Rust/Lean实现，因此不重复统筹完整验收；共享最终门槛由统筹接入，CI不替代它。

```bash
python3 -m tools.check_os_boundary_catalogue
python3 -m unittest tools.tests.test_os_boundary_catalogue
python3 tools/check_docs.py

# 证明机：同一固定Python和原T0客户端，复用原来源检查器而不另造MIR解析/复核平台
python3 tools/v0_use_inventory.py check-published --output "$CAPTURE"
```

原检查器逐字比较编译路径。若使用容器，源码和捕获需置于原metadata/receipt记录的虚拟路径；应在本批独立容器中保持解析后的原虚拟路径（只读挂载），不能改原命令/回执、跳过路径检查或共写旧target。所有机器路径与完整私有日志不入仓库。

首次宿主Python3.8缺少zip(strict=True)、启动脚本引号错误，以及未保持原虚拟路径导致的两种严格拒绝均保留为准备失败；固定原Python3.12并在独立命名空间恢复原路径后，原准入才通过。没有重采MIR、更新工具、清理.lake或修改原捕获。

## 统筹接线建议

仅将本目录作为有限源审查材料挂到#333的T7/T8位置；由统筹维护依赖/覆盖文档、use-inventory与总清单。已有代表不重做，#617按自己的封口头集成；TLS提议先做可行性预检。当前所有source-only/候选/业务义务标记必须保留，不把“原源码可定位”改写成“真实系统已证明”。本批不改共享Lake、根/审计、原生/形式套件、全局冻结或CI。
