# Create 观察值与完整校验合同

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

值支持固定 #691 `4f52d972a368596c0a9d4a8fff8913e757dfc778`；完整守卫候选堆叠于
#699 `8dca00fcb8717771de31df307097fd762ce78805`。生产源码保持原 a9c 协议 4。
本目录准备实际 `CreateObservation::matches_request` 的完整合同及检查输入；
`CreateObservationEquality` 对实际派生比较证明任意值的完整相等，
`CreateObservationValues` 证明合同的合法见证和字段遗漏拒绝。
这些定理不等于完整 `matches_request` 的实现对应。

`CreateObservationModel` 按原实现保留 Header、Refused、同请求、Unload 继续、Renew 和 NotCurrent 分支，
复用现有 `ReadyFacts.CreateMatches`、`Timing` 与 `ResumeBinding.Continued`。
原始 Create/改参合法见证复用 `ReadyFactValues`；新增续租、卸载、销毁与边界输入使用相同封套合同。
Refused 必须没有历史成功，但其他 Pending 值不能解释为没有资源效果；
Renew/NotCurrent 的部分分支允许复用 key，非 Refused 改变当前请求的分支也不总重新校验旧请求。
不得据值相等推出授权、原意图、耐久拒绝、已应用续租或实际就绪。

`observation-bindings/extract.py --profile create-values` 保留原八个请求/事实/Resume 根，
增加实际 Create 观察值派生比较。`--profile create` 则请求完整 Create 与续租事实入口，
默认环境下后者受固定工具的 `TryFromIntError` 表示差异阻断：原转换返回 `IntErrorKind.NegOverflow`，
固定后端将 `TryFromIntError` 映为 `Unit`。最小 `u64::try_from(i64)` 成败/数值/错误载荷连接交 V0，
不手改生成体、替换转换算法或把整个守卫设为可信。

`conversion-candidate/` 提供该接口的**隔离审查候选**，尚未获 V0 采用。
它在私有生成命名空间把 `TryFromIntError` 解释为实际生成的 `IntErrorKind`，
保留固定 Rust/LLBC 的唯一字段；全局后端的 `Unit` 模型保持原样。
原始生成函数不改写，手写的私有 `FunsExternal` 增加候选类型与 #696 原 Result tag 的导入。
这属于新增类型表示候选，不能仅因原四个外部函数未变就称库模型完全未变。
单字段表示的双向往返、错误种类保留、实际转换的任意输入成败/精确值/负溢出载荷
及 `is_err` 擦除关系在候选环境检查；擦除不保留错误种类另有反例。
表示关系依据精确 Std 源码和原 LLBC 审查，不是 Charon/Aeneas 正确性的机器证明，
也没有 ABI/内存布局、`Display` 或错误消费链结论。单独运行转换候选不包含完整 Create 守卫证明。

候选不进入默认导入或公共登记；文件保留为 `.lean.in`，仅由下列入口安装到新建私有目录。
入口重新抽取完整 `create` profile（同时保留 Create 比较根），检查原 Request/ReadyFacts/Resume/Create
值证明及新增文件的完整实际导入闭包和审计。提供 #696 精确提交
`3e2a7dc2efc00c8efbe226486ee704605b73bf82` 中的 `RenewalResultTag.lean` 原文件，
入口按内容核对并复用它，不另造 Result 模型。固定 Rust 的 `rust-src` 也须存在并符合审查摘要。
持有共享重锁运行；正式普通/原生验收仍交 ning：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-observation/conversion_candidate.py \
  --packages /private/task/.lake/packages --result-tag-source /private/696/RenewalResultTag.lean \
  --output /private/create-conversion-candidate
```

该结果始终标记 `candidate_only=true`、`v0_adopted=false`、`full_guard_refinement=false`。
V0 需审查名称解析后的字段表示及 Result tag 依赖；下述守卫候选可在同一隔离解释中继续检查，
其正式采用仍等待 V0。公共依赖合同、根/原生登记、冻结与覆盖记录由主控协调。现有值证明入口不变。

四个错误版只在隔离生产副本修改一个已登记原表达式：响应只比 key、遗漏当前 retention、
Refused 允许历史成功、负数续租强转回绕。原生入口逐版检查原版→变异→恢复，
编译失败、超时或导入失败均不计为反例。负数见证使用 startup=1、max lifetime=`u64::MAX-1`、
deadline=-2，原请求与当前请求均合法，避免用溢出的非法 Create 充当反例。
全部正反版本使用同一完整合同及见证；原生断言与下面的实际生成体形式反例分别检查。

开发形式入口只验收上述值支持范围，独立抽取、构建并调用原内核与严格同服务检查器，
核对全部实际文件、保留的 Request/ReadyFacts/Resume 证明与审计、完整非 stale/partial 结果。
输出显式保留 `full_guard_refinement=false`、`formal_mutant_checks=false`；
不能用该入口通过关闭 Create 校验义务。每次持有共享重锁，保留失败证据及独立 `.lake/build`：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-observation/formal.py \
  --packages /private/task/.lake/packages --output /private/create-value-formal
```


`guard-candidate/` 对实际生成的 `create_matches_request` 证明任意输入的 `FullContract`，
并连接原 `CreateObservationValues` 的正常、拒绝及未加强边界见证。
`CreateObservationBindings` 复用既有请求验证、完整相等、ReadyFacts、Continued 与版本比较引理，
证明续租事实叶子、Refused/非 Refused 分支、完整接受与拒绝 iff，以及全文请求保持。
正反环境逐字节复用原 `CreateObservationModel` 和 `CreateObservationValues`，不改原合同。
四个错误版分别证明实际错误函数返回 true，并由原拒绝见证推出违反同一 `FullContract`；
错误版环境不导入正确守卫的对应定理。

完整 `create` profile 另保留一个仅调用实际 `u64::try_from(i64)` 的透明根，
让负数强转错误版仍生成同一标准转换及错误类型；其业务守卫保持实际变异后的强转，
没有用该根替换错误算法。由 `conversion_root_exact` 核对该根，实际生成体不手改。
正确版和四个错误版各有独立源码、Rust target、原 LLBC 和 `.lake/build`，
全部实际文件通过原 `formal.check_actual` 内核检查与同服务完整诊断/审计。
缺文件、缺审计、旧合同或见证变化、编译/抽取/导入失败均不算性质拒绝。

持有共享重锁，提供 #696 的两个原文件，运行新的完整守卫候选入口：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-observation/guard_candidate.py \
  --packages /private/task/.lake/packages \
  --result-tag-source /private/696/RenewalResultTag.lean \
  --result-bindings-source /private/696/RenewalResultBindings.lean \
  --output /private/create-guard-candidate
```

只有正确版实际检查完成才置 `full_guard_refinement=true`，四个错误版全通过才置
`formal_mutant_checks=true`，全部五版完成才置 `all_cases_complete=true`。
同时始终保留 `candidate_only=true`、`v0_adopted=false`、`full_v3_proof=false`、`full_acceptance=false`。
这是候选解释下已解码值的守卫对应；不证明认证上下文来源、持久拒绝/接受、应用过续租或 VM 就绪。
`--case` 仅供隔离开发，单版成功不能报告五版检查通过。

ning 在精确提交的 `archive.py` 源码归档上运行普通/原生验收：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-observation/native.py \
  --source-archive /private/source.tar --target /private/create-native-target --output /private/create-native
```

该入口检查真实业务库及测试二进制的来源、manifest、fresh=false、摘要和完整源码恢复，
另运行本组件格式、Clippy 与值/转换/守卫准入回归；不签发共享 native seal。
公共登记、`verify-stage1-protocol.sh`、`proof_native_suite`、冻结与覆盖由主控统一验收。
V1/V2 提供原意图与发送，V4/V5 提供耐久接受及已应用结果，V6 提供 VM/文件事实，V7 提供独立认证来源。
完整守卫候选的 V0 采用、JSON/schema/TLS、持久收件、重启/乱序/竞争与系统组合均是另列义务。
