# 原中心生命周期的首个受限组件

对应 [V1](../../../docs/formal/01-center-core.md)、[V2](../../../docs/formal/02-center-store.md)
及 #633/#634/#635。本目录复现真实生产 Rust，不提供替代控制器、通用翻译器或默认启用服务。
实际结论和精确提交见本批 PR 与[覆盖记录](../../../docs/proof-coverage.md)。

## 实现范围

当前中心只在原会话行保存完整操作和结果，不维护影子表或旧协议备用写入。正式路由、后台、Unloaded 保存与重启查询已经接线；本目录历史实验不等于这些新函数及整条链已证明。当前输入须重新核对并经原抽取入口生成，不能沿用旧产物背书改动。

可以先用 `version_next`、`replay` 薄调用根验证当前真实版本推进及完整请求重放；完整 accept/terminal/renewal 调用图须独立预检和证明。Resume、再次卸载、完整材料 GC 与系统进展仍未完成。历史工具失败只说明该次候选的结果，不预判当前根成功或失败。

## 普通复现：仅测试机

在确定提交的独立私有 checkout 中，使用固定 Rust 1.97、Python 3.12 和 Redis 7.2.16。
若使用 Git 导出副本，须按原导出文件清单建立本地 Git 索引，供原文档检查器和输入核对使用；
不伪造实际提交身份，不把运行产生的缓存加到源码清单。所有跟踪夹具都保留，即使嵌套目录名为 artifacts。

```sh
export CARGO_NET_OFFLINE=true
export SANDPROOF_REDIS_SERVER=/absolute/fixed/redis-7.2.16/src/redis-server
python3 verification/experiments/stage1-center/native.py \
  --output /private/new-component-result --target /private/stable-cargo-target
```

外层使用原普通重任务锁；不能同时占有本任务的普通和形式重任务锁。
运行原 core 测试、全部 store 单元/真实 Redis 测试、严格 Clippy、格式与文档检查。
Redis 夹具之间使用 `--test-threads=1`，避免原端口预留释放至子进程绑定间的并行争用；
不减少用例、不改变断言，用例内部的显式并发仍执行。原失败日志保留；该设置不是任意外部端口干扰已被形式证明排除。
原夹具在 PONG 后还核对真实 INFO process_id 与原 Child PID，并再次观察该 Child 未退出；
错 PID/未知明确失败并附有界原启动日志。两个真实 Redis 即使同密码且均 PONG，也不能互相冒充原进程。
两个实际错误副本分别只比较 key、跳过期望版本；先检查实际业务库及测试程序都新编译，
再运行正确版同一断言，要求 101 确实来自断言。编译失败不算反例。
原源码/结果/日志摘要、二进制及变体保留；只重置错误副本的自有 Rust 包缓存，不删依赖或 `.lake`。

这不是原全套 `proof_native_suite` seal，不代替 SDK、完整项目基础检查、KVM 或形式验收。

## 固定原工具预检：仅证明机

```sh
python3 verification/experiments/stage1-center/extract.py \
  --profile version --tool-root /private/original-tools --output /private/new-version
# 另可指定 replay 或 accept；每次使用新的结果目录并保留上次原始 IR。
```

三根分别调用生产 `next_lifecycle_version`、原操作记录 `replay` 和完整 `accept_lifecycle`，
不复制业务算法。固定原 Charon/Aeneas、平台及工具摘要，保存全部原 LLBC 和生成文件；
不手改生成 Lean、不添加自有业务 opaque、不关闭借用或内核检查。

当前另有 `--profile terminal`，直接调用真实 `accept_lifecycle_terminal_observation`，覆盖同一生产图的
校验、Cleaned 保存和保留期限决定，不复制实现。原 `accept` 根继续直接委托实际受理函数。
两根供当前候选在证明机做最小预检；结果分别报告 Rust 编译、Charon、Aeneas 和实际 Lean 检查阶段。
没有当前结果前不能沿用历史失败或成功。若前端失败但留下 IR，原脚本另保留 `partial.llbc`，
它不作为完整生成物交给后端。

续期小批使用 `--profile renewal` 和 `--profile renewal-rule`，分别直调中心的完整
`confirm_lifecycle_renewal`（包含实际恢复校验）和原 SQL 适配实际调用的
`can_discard_confirmed_sdk`。后一根只引用原 service 的无 durable 特性纯模块，
不复制删除算法、不将恢复校验设为不透明。Cargo 锁由测试机以固定离线依赖生成，
随后在同一保存源上由证明机运行原工具；新字段/格式改变后的旧生成物不能沿用。
预检源码存在不表示编译、抽取或证明成功；业务引用和回收保持性仍按覆盖记录认领。

首轮 version/replay 均完成原工具生成，但尚未运行 Lean 内核/实际文件服务或证明审计，
不能称已证明。完整 accept 首轮在 Aeneas 循环预处理拒绝两层首匹配循环；拆为原序短路
Session/Profile 查询并分离顺序校验循环后，仍在 `valid_lifecycle_session` 报
`Early returns inside of loops are not supported yet`。两个失败版本、原工具/IR 和诊断均保留。
本批停止扩大完整图适配；完整决定、恢复 validator、预留与修改范围仍是业务实现对应缺口，
不把它们转为可信条件，也不因此阻止未启用的小功能批先接受审查。

接口 `AcceptedLifecycleIntent`/Context 的实际测试和信任条件见
[逐接口依赖登记](../../../docs/formal/dependencies.md#v3-中心身份的薄连接)。
原 #642 局部定理不背书这里的新中心状态或 checker。
