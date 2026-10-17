# 数据许可登记与重新派发隔离试点

[验证链](../../README.zh-CN.md) · [网关计划](../../../docs/formal/08-data-gateway.md) · [依赖登记](../../../docs/formal/dependencies.md)

本包将冻结版 `data_access.rs` 中的有限守卫拆成真实 Rust `permit_guards.rs`，在隔离副本中由原业务路径调用，再用原 Charon/Aeneas 抽取。生产 Rust、工具与全局验收入口均不修改。本包只证明列明守卫及其组合，不表示完整鉴权、许可对象、任务取消、数据库或 relay 进程已经验证。

下文 B2/B3/B4 使用点采用 owner 已批准的可信薄适配方案；[#537](https://github.com/VeriInfra-Lab/sandproof/pull/537) 仅为文档统一背景，本包合同在下文独立列明，不依赖该 PR 合入。不能把该方案解释为整个 `data_access.rs` 可信。

核心、真实调用 overlay、行为测试、实际 Lean 生成体与证明一同提交；一次性日志、源码副本和编译缓存留在调用方指定的输出目录。冻结源是 integration `0a23b57af60752f61067fc00b0b0b656e98347cb`；旧试点证据不自动适用于本包。

## 范围与证据

七个 helper 保留原分支的完整布尔/可选编号输入：受限宿主必须有 helper gate、原 watch 回调短路读取、撤销信号保留/新建、重新派发前撤销检查、宿主配置检查及 VM/代际完整匹配。真实调用 overlay 保留原权威锁、外部查询与拒绝顺序，以及后续凭据、寿命、Access 和实际 spawn 调用。

必须分别阅读以下结论：

- **核心机器证明**：原工具从实际 helper 生成 Lean；独立策略与生成体的对应，以及同一撤销性质对错误 Rust 的反例。可用性和精确范围由本包冻结输入及实际验收结果确定。
- **调用对应**：原始与隔离候选的源码审查、真实 SQLite 组件和有界行为对照。它们支持调用绑定，不是任意原始/重构执行的一般等价证明。
- **可信机械接线**：B2 的原锁、共享状态/快照；B3 的 watch 身份、订阅与接收者计数、通知/Drop 驱动；B4 的既定参数/结果转发。实际观测是否真实、同一锁/gate/对象是否正确绑定，仍依赖列明合同及审查/测试。
- **仍需业务证明**：凭据/租户鉴权、存储快照与完整请求、寿命、Access、资源总预算、撤销决定与真实退出/排空。核心输入里的 `revoked`/`bounded` 是原调用观察，不能凭核心定理证明该观察来自正确对象。

错误副本只绕过重新派发的撤销守卫。它必须成功编译并在相同业务断言处失败；Lean 负例必须说明同一性质被违反，编译错误或 tactic 搜索失败不算反例。取消等待不表示 relay 退出，零计数不自动表示排空，Unknown 不得解释为成功。T9 继续排除，其他来宾安全业务不因此排除。

本包实际使用的最小可信合同如下，都是组合的显式条件，并非新增 Lean 公理：

| 接口 | 可依赖的保证 | 不由该保证提供 |
| --- | --- | --- |
| 原 management mutex（B2） | 成功取得同一 `Inner` 的排他锁；在该临界区读取同一 journal/许可记录并完成登记，释放/中毒如实可见 | 读到哪个租户/沙箱才正确、快照业务有效、无死锁或最终取得锁 |
| watch 与记录薄接线（B3） | `receiver_count`/`borrow` 是同一 sender 各自读取时的观察；外部 receiver 可在两次读取间 Drop。`subscribe` 属于原 channel；保留/新建按核心返回值执行，创建新 channel 不复活旧撤销 channel | 把不同时间的观察当原子快照、凭据授权、撤销决定正确、进程退出或真实排空 |
| host gate/给定值传递（B4） | 原 sandbox/VM/代际传给原 host API；返回的 gate、错误和值不串线；`bool`/`Option<u64>` 与原字段/读取结果对应，原错误顺序及后续调用不被省略 | 选择目标正确、配置/预算充分、后续凭据/寿命/Access 校验成功、未知效果等于未执行 |
| 原 Aeneas 内置 Bool/Option/U64 模型 | 在固定工具和有效可表示值前提下提供布尔、枚举和全宽整数语义；本批没有新增外部函数模型 | 任意 Rust、库、编译器或物理资源实现已被证明 |

核心只读取输入并返回值，不修改 journal、watch、许可或进程。真正副作用的对象和修改范围由原调用绑定审查及对照测试核对；不假设所有错误无副作用。panic 不保证回滚，取消不保证收尾，abort/崩溃不保证执行 Drop；这些路径不由本批有限守卫证明。核心定理不提供全局内存/许可预算或业务最终进展。

## 离线复现

在获批的 x86_64 Linux 证明环境执行。项目本轮所有测试、抽取和证明在远程证明机完成，本机只编辑与读取。预备固定工具和离线 Cargo/Lake 依赖，不运行安装/升级命令。

1. 在含该 Git 对象的仓库准备 `git archive 0a23b57af60752f61067fc00b0b0b656e98347cb` 的 tar；压缩可选。输入清单逐文件核对，压缩时间戳不参与语义摘要。
2. 准备原 Aeneas 工具根、固定 Lean/MCP runtime、原 Lake packages 缓存、离线 Cargo home。环境与输入摘要见本包清单，任何不匹配直接失败。
3. 使用原项目中未修改且摘要匹配的 `tools/lean_session.py` 与 `scripts/lean-mcp-server.sh`。总入口仅将本次指定源的这两份文件复制到私有 proof 工程，连接现有 runtime；不携带旧客户端副本，不修改工具。

```sh
python3 verification/experiments/data-permit/scripts/verify.py \
  --baseline-archive /prepared/baseline.tar \
  --source-repository /prepared/source-repository \
  --tool-root /prepared/fixed-aeneas \
  --mcp-runtime /prepared/lean-mcp \
  --lake-packages /prepared/lake-packages \
  --cargo-home /prepared/cargo-home \
  --tmp-directory /prepared/private-disk-tmp \
  --output /prepared/new-experiment-output
```

这些是调用方路径示例，不是仓库内的机器配置。输出必须不存在，且不得包含源仓库或包；脚本拒绝覆盖。临时目录须已存在、当前账号私有且位于支持目录 birth identity 的磁盘上。涉及真实 Redis fixture 时另设 `SANDPROOF_REDIS_SERVER` 为固定版本二进制；缺少它不得跳过。

总入口核对输入 → 导出原始/候选/错误副本 → 安装严格 overlay → 原始与候选测试/语义记录对照 → 编译并拒绝错误副本 → 原工具重新抽取及生成 Lean 对照 → 同一真实 Lean 服务 → 内核构建与实际依赖审计。失败、超时、零测试或输入漂移不会输出整体通过。原始完整验收 `scripts/check-function.sh` 和 `scripts/verify-controller.sh` 仍独立执行，局部成功不能代替它们。

原始、候选、错误副本分别使用独立 Cargo target 重新编译；不通过共享 target 的旧产物确认新源码。LLBC 与生成 Lean 均原样比对，不删除字段或手改生成文件。

`receipt.json` 与分步日志写在输出目录，记录本批实际输入摘要与退出状态；结果只适用于该冻结闭包。源码/overlay/core/模型/证明变化后重新生成和验证，不只刷新摘要沿用旧证明。公共仓库不保存机器地址、访问方式、完整日志或构建缓存。
