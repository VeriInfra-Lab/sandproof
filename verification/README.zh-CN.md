# Rust 实现与 Lean 规范的验证链

[文档目录](../docs/README.md) · [验证设计](../docs/formal-verification-design.md) · [证明操作流程](../docs/proof-workflow.md) · [覆盖与剩余义务](../docs/proof-coverage.md)

这里保存按模块登记的真实 Rust 经固定 Charon/Aeneas 抽取的 Lean 语义、有限库模型及审计入口。当前公共/共享状态/首链的指定实现对应与数学模型定理分开验收；系统服务仍需建立真实提交、I/O、异步及恢复的事件/微步骤连接。旧整控制器生成物与专属对应证明已退役，可从 [删除前基线](https://github.com/VeriInfra-Lab/sandproof/tree/110918e) 恢复，不属于当前已证清单。

获批薄异步接线按[依赖登记 B1–B4](../docs/formal/dependencies.md#逐符号可信适配登记)保留可信条件，业务决定仍需实际实现证明。

固定工具链如下：

- Rust 控制核心：Rust 1.97.0；
- Charon：`b104e24fea7d721b71e6c39fd70f26ff20bc0980`；
- Aeneas：`505b6ca35217e7be5c96c3e2f8045edfbdf47291`；
- Aeneas 使用的 Rust：`nightly-2026-08-18`；
- Lean：4.31.0。

目录含义：

- [数据许可隔离试点](experiments/data-permit/README.md)：冻结真实基线的许可登记与重新派发同步守卫、正负例证明和复现入口；生产未接入，不属于默认全项目验收的已证覆盖；
- [重新派发授权隔离试点](experiments/data-authority/README.md)：实际凭据、寿命原始行和 Access 核心、条件组合与同性质错误版；依赖前一冻结试点，生产未接入；
- [Redis 调用与结果隔离候选](experiments/redis-result/README.md)：真实泛型提交/重申核心、完整参数与围栏结果定理；自有 codec、具体调用连接及正式验收仍有独立义务，不表示生产存储已验证；
- `aeneas/llbc/`：从实际 Rust 编译器中间表示抽出的输入；
- `extract-foundations/`：仅调用生产函数的公共基础抽取根，不复制业务算法；
- `aeneas/foundations/`：公共基础的生成语义和独立命名空间的最小库模型；
- `extract-first-chain/`、`aeneas/first-chain/`：Linux 实际身份、生命周期回执检查及生产中心分类根；不代表完整中心状态更新已抽取或已证明；
- `extract-network/` 与 `aeneas/network/`：共享当前完整 Rust 类型的真实调用根与固定库模型，包含网络判断/准入、World 克隆/初始化、三类运行输入与资源位置转换；输入值根还保留九组公开输入的完整类型，不调用或证明统一派发；保留原目录名，不表示完整控制器行为已覆盖；
- `SandboxModel/Access/`：当前查询/相等/克隆、按序网络准入、完整状态保持及历史两模式策略嵌入；
- `SandboxModel/Foundations/`：公共身份/数值、有限表查询与写入对应，以及可复用的有限扫描、值保持克隆和参数化修改范围引理；
- `SandboxModel/Local/`：当前本地组件的纯模型、表示关系、实现对应和性质；`State/` 维护自然数/有序列表的完整当前值模型、全部输入及无丢字段表示，运行/位置等组件按各自目录连接，不等于所有更新流程已迁移；
- `SandboxModel/FirstChain/`、`SandboxModel/FirstChainLinux/`：条件系统模型，以及真实身份/回执检查的局部对应；不能推广为全链实现闭合。

`FunsExternal.lean` 是少量 Rust 标准库函数的可计算模型。Aeneas 同时会
产生一个以公理占位的模板；重新生成脚本会删除模板，项目只编译这里的
完整定义。定义可计算且无公理，不等于已经证明与实际标准库等价；使用时仍需对应论证或显式外部模型前提，不能漏记这一层。

固定 Charon/Aeneas 主要支持安全、顺序 Rust 子集，不宣称任意 Rust 无条件无损翻译。编译器中间表示经 Charon 形成 LLBC，再由 Aeneas 生成 Lean 语义；抽取成功不自动涵盖 Tokio/async、锁、数据库或系统调用。真实同步业务决定仍抽取并证明，实际事务、锁、回调经逐项登记的接口连接；少量机械接线可依赖 B1–B4 的可信合同，不要求先实现通用异步翻译器。审查与测试支持合同采用，不冒充异步实现精化；不用手写简化业务替代真实实现。

编译器前端、Charon/Aeneas 翻译器、Lean 内核与外部库模型的信任角色分别声明。证明绑定源提交、目标平台、features/cfg、生产/抽取编译器、依赖、工具、生成入口和模型指纹；生成文件不手改，测试/摘要不代替翻译器正确性证明。模型性质、指定实现精化与条件下系统组合分别报告，详见验证设计。

复现命令：

```sh
scripts/bootstrap-aeneas.sh
scripts/verify-controller.sh
```

第二条命令检查 Rust 格式、单元测试、静态检查，分别执行公共基础、共享状态、Linux 首链和首链模型的完整门槛，再编译全部保留 Lean 源并审计依赖；全部保留模型及抽取目录拒绝 `sorry`、`admit` 或项目自定义 `axiom`。通过表示当前保留证明集合一致，不表示统一控制器或部署系统已完成验证。

完整入口包含真实 Redis 用例，须按[开发指南](../CONTRIBUTING.md#3-测试规则)设置经固定版本校验的 `SANDPROOF_REDIS_SERVER`，不能缺失后跳过。内存受限环境可用 `OCAMLRUNPARAM=o=20 scripts/verify-controller.sh` 降低固定 OCaml 运行时的堆空间冗余；这是运行资源调优，不改变工具版本、抽取根或检查内容，也不保证任意源码都能在该内存中完成。完整验收的实际结果仍以覆盖记录和关联 PR 为准。

公共基础可独立执行 `bash scripts/verify-foundations.sh`：检查输入指纹、固定工具与平台、重新抽取一致性、真实入口/全部自有函数体、库模型范围、定理编译和实际依赖。冻结环境见 [foundations-environment.json](foundations-environment.json)，源码/模型/脚本/生成输入摘要见 [foundations-inputs.sha256](foundations-inputs.sha256)。独立模块检查不替代完整验收；完整入口保留全部当前模块检查；旧全量控制器生成链已删除，不再作为这些模块的前置。独立工作区可用 `SANDPROOF_PROOF_TOOL_ROOT` 指向已有固定工具，修订和二进制摘要仍需匹配，不改变工具版本。

公共基础及共享状态的 Charon 输出参数使用抽取 crate 内的固定相对路径 `../aeneas/llbc/...`，因为 Cargo 的编译进程在 crate 目录执行；脚本随后从项目根读取同一输出。每次保留旧产物并要求本次实际生成，规则见[证明工作流](../docs/proof-workflow.md#翻译障碍的识别与处理)。该参数仍在完整 LLBC 摘要中检查，源码位置仍由原 `--remap-path-prefix` 绑定。换工作目录不应只因输出路径而产生差异，错误或绝对输出参数仍拒绝。摘要仅规范化无序 `short_names` 表，不删除路径或其它有序输入字段。

JSON 非零计数器叶子另执行 `bash scripts/verify-json-counter.sh`，检查[真实根](extract-foundations/src/json.rs)、[环境](json-counter-environment.json)、[输入摘要](json-counter-inputs.sha256)、原样再生及 12 条审计。正确版和实际 Rust 零校验缺失版可用 `bash scripts/python-entry.sh tools/check_json_counter_negative.py --output artifacts/json-counter-negative-run` 对照；输出目录必须不存在，失败日志保留。该入口要求已提交的本批源码及原工具/依赖/真实 Lean 服务，错误版必须先编译、抽取成功，再由同一 `CounterSafe` 被形式反例否定。接口保证、原条件析构及字节/parser/schema/调用者剩余义务见[唯一合同](../docs/formal/dependencies.md#a6-json-非零计数器叶子接口)，不能把叶子成功称为完整 JSON 或异步证明。

八类字符串编号叶子执行 `bash scripts/verify-json-identifiers.sh`，冻结[真实调用根](extract-foundations/src/json_ids.rs)、[环境](json-identifiers-environment.json)及[输入](json-identifiers-inputs.sha256)，与原 V0/计数器共用生成类型并审计 18 条新定理。提交本批源码后执行 `bash scripts/python-entry.sh tools/check_json_identifiers_negative.py --output artifacts/json-identifiers-negative-run`：输出目录必须不存在；正确版和移除真实校验的错误版从同一个 Git 提交归档创建，分别使用新 Cargo 构建目录，错误版须先编译/抽取成功，再由同一 Rust 安全断言和同一 `IdentifierSafe` 反例拒绝。库默认方法、标准 String 释放、有限格式化及 parser/schema/调用者剩余条件见[接口合同](../docs/formal/dependencies.md#a6-json-字符串编号叶子接口)。值读取器是非空见证，不是真实 JSON parser；同一安全性定理不证明完整请求绑定或授权。

模块范围按 [V0 计划](../docs/formal/00-foundations.md#范围和模块验收)区分：公共模块的一批检查成功不表示 V0 全部交付；退役旧生成链不消除当前生命周期等业务的实现证明义务，也不使其成为 V0 的关单前置。完整入口按保留集合执行，局部/当前集合验收/全系统完成分别登记。

完整生命周期请求执行 `bash scripts/verify-json-request.sh`。入口冻结[实际调用与字段/字典登记](json-request-binding.json)、原生产协议与服务器/客户端调用、完整公共生成物以及 `verification/json-request-inputs.sha256`；先按原工具重抽取并比对，再检查两个直接库调用的完整 LLBC 和 AST、全部请求定理及其真实依赖。原 `LifecycleRequest::validate`、编号/计数器校验和转换保留生成体；固定 Serde/serde_json 引擎、标准派生及单次参数/结果接线采用明确 A6/B4 条件，不能称整个 async 服务已自动证明。具体输入/错误/解析位置/正常分配和有界合法接受条件见[唯一依赖合同](../docs/formal/dependencies.md#a6-json-与标准派生的最小合同)。

本入口还在新私有副本中运行 `tools/check_json_request_negative.py`，先编译移除原身份守卫的错误 Rust，再实际重抽取；原 Rust 错误顺序断言须失败，Lean 须证明同一 `GuardContract` 的否定。可独立用 `bash scripts/python-entry.sh tools/check_json_request_negative.py --output artifacts/json-request-negative-run` 复现，输出目录必须不存在，必须使用已提交源码和原固定工具/服务；构建失败不算性质反例。完整验收仍执行 `bash scripts/verify-controller.sh` 和 `bash scripts/check-function.sh`；局部接口通过不关闭 V0/T0–T8。

共享当前状态的局部集合可独立执行 `bash scripts/verify-network.sh`：检查真实查询/名单循环、平台常量及初始化体、固定两版 Rust 标准库源码、最小外部接口，以及 Network、Access、Local 三个证明审计。环境见 [network-environment.json](network-environment.json)，输入摘要见 [network-inputs.sha256](network-inputs.sha256)，库条件见[依赖登记](../docs/formal/dependencies.md#a6-网络地址库模型使用登记)。完整入口包含此模块，并继续编译全部保留证明；局部准入/位置证明不等于统一控制器分派、可达状态围栏、物理清理或宿主防火墙已验证。冻结的 V0/共享状态验收目标为 x86_64 Linux；目标、工具二进制或 cfg 不匹配即拒绝，不静默按本机平台重写清单。

DataLease 的只读接口执行 `bash scripts/verify-data-drain.sh`：直接引用原生产文件抽取两个读方法与两个转调入口，核对三项读原语、Acquire 排序、两版标准库、输入摘要和全 LLBC 再生一致性，再编译独立策略／表示关系／对应定理并审计依赖。正式库为 `SandboxModel.Boundaries.DataDrain`；接口条件见[依赖登记](../docs/formal/dependencies.md#a6-datalease-只读观察接口)。这是单次观察下的判定证明，不提供真实退出或释放许可。

Host 公共任务／许可合同执行 `bash scripts/verify-task-contract.sh`：核对冻结来源、固定 Tokio 的 `.crate` 和完整解包源码、定理清单与实际依赖，编译 `SandboxModel.Boundaries.Task.Audit`，随后在私有副本重放正确和三个错误版本。也可单独用 `bash scripts/python-entry.sh tools/check_task_contract_components.py --output artifacts/task-contract-check` 重放组件，输出目录必须不存在；每副本独立 Cargo 目标目录，确认编译成功并取得新二进制后才运行同一资源断言。仅在隔离副本追加测试模块和受控宿主夹具屏障，正式生产文件保持不变。入口要求当前输入与 HEAD 冻结一致，不能用旧提交的副本测试当前代码。

环境和调用登记见 [task-contract-environment.json](task-contract-environment.json)、[task-contract-inputs.sha256](task-contract-inputs.sha256)和[B1 条款](../docs/formal/dependencies.md#b1-host-任务与原许可池的逐符号可信适配)。这些是条件接口模型及适配工程核对，**不是实际 async Rust 的自动抽取证明**。模型不承诺任务终止、物理退出或回调的业务不变量；错误副本被组件断言拒绝与 Lean 模型反例分别报告。所有这些入口按当前批次约定在 a6000 运行。

撤销等待执行 `bash scripts/verify-revocation-contract.sh`：核对 [B3 环境](revocation-contract-environment.json)、输入、实际调用范围、固定 Tokio 源码、24 条条件模型定理及依赖审计，再在独立副本运行正确版和交换分支的错误版。每版须新编译；错误版必须被原同一断言拒绝。单独复现入口为 `bash scripts/python-entry.sh tools/check_revocation_components.py --output artifacts/revocation-check`，输出目录必须不存在。原固定 Charon 不支持该 async 入口，实际接线以[逐符号 B3 条款](../docs/formal/dependencies.md#b3-撤销等待的逐符号可信适配)为条件，不声明自动抽取、回滚或外部任务退出。完整证明入口追加此门槛，保留其余全部模块检查。

缺失 unknown 检查的错误版可在同一固定 Linux 环境用 `bash scripts/python-entry.sh tools/check_data_drain_negative.py --output artifacts/data-drain-negative-check` 复现，输出目录必须不存在。入口在私有副本修改真实表达式、编译／抽取错误版，运行原 Rust 回归，并通过同一真实 Lean 服务和内核证明错误版不满足正确版相同的 `RustReadSafe` 性质；失败日志保留，只有全部步骤通过才生成成功收据。服务使用项目 `.tools/lean-mcp` 和原标准客户端；不修改生产文件或正式生成体。

本页只维护工具链与产物含义，不维护阶段进度；已覆盖入口和剩余义务统一
见证明覆盖记录。已有纯核心定理不自动覆盖新增服务或真实持久化与执行。

Linux 回执批次执行 `bash scripts/verify-first-chain-linux.sh`，冻结环境与输入分别见 [first-chain-linux-environment.json](first-chain-linux-environment.json) 和 [first-chain-linux-inputs.sha256](first-chain-linux-inputs.sha256)。固定工具修订相同不意味着不同平台的工具二进制、cfg 或抽取物相同；二者分开登记，不覆盖 [V0 环境](foundations-environment.json)。[Linux Lake 视图](../lake-first-chain.toml) 必须逐字保留原 Lake 配置，只追加新抽取库；检查器拒绝其它差异，避免修改 V0 冻结配置或维护另一套依赖版本。生成的全局枚举实例名称在两个类型集合中可能重名，故通过同一数学 Binding 合同连接、分别编译审计，不把两套名义类型同时导入一个环境或修改生成文件绕过冲突。标准证明客户端的源码指纹包括所有抽取目录。服务器连接、代理、私有工具路径和运行配置放仓库之外，公共清单不保存这些信息。

新增中心/存储/通信/网关等入口的任务按[V0–V9详细计划](../docs/formal-verification-design.md#2-按真实模块划分证明责任)推进。重新抽取各已登记模块的明确子集，不覆盖统一控制器、整个中心、存储或通信；各模块首次证明批次须增加实际抽取根、窄接口模型/可信适配登记或自有脚本连接、版本清单与定理依赖审计，再纳入完整验收。旧生成文件编译成功不是新模块抽取或证明通过。

## 离线轨迹辅助工具

下列工具用于模型实验和旧数据分析，不是产品系统验收，不把本地 harness 输出当成已确认的 E2B 漏洞。

### 离线运行

```sh
python3 -m tools.trace.selftest
python3 tools/trace/check.py artifacts/example.jsonl
python3 tools/coverage.py
python3 tools/coverage.py --write
```

`example.jsonl` 需由调用者提供；最后一条把生成报告写到忽略的 `artifacts/trace-coverage.md`，不再生成第二份需要手工同步的文档。

规则和性质登记分别在 `tools/trace/rules.py`、`tools/model/properties.py`；数量和对应关系以生成报告为准，不在本页复制。

### JSONL 记录格式

每行一条记录，使用同一轨迹内唯一、递增的 `seq`；`ts` 是单调时钟纳秒值，跨进程时间本身不能证明因果顺序。加载器按 seq 排序，不代替采集端证明时间与因果关系。

```json
{"seq":1,"ts":100,"source":"client","kind":"event","event":"acceptKill","sandbox":"s1","attrs":{"method":"DELETE","http":204,"result":"success"}}
{"seq":2,"ts":200,"source":"observer","kind":"observe","sandbox":"s1","obs":{"running":false,"snapshot_present":false}}
```

| 字段 | 含义 |
|---|---|
| seq / ts / source | 顺序、时间、来源 |
| kind / event | 事件或状态观察，以及抽象事件名 |
| sandbox | 本记录关联的沙箱身份 |
| attrs | 原始动作、返回结果和必要证据；输出前脱敏 |
| attrs.result | success / reject / unknown；超时不意味着动作没执行 |
| obs | 部分观察；没有观察到的字段留空，不能补成 false |

采集器写入抽象事件；`mapper.py` 读取这些名称及观察，不是自动从任意 HTTP 日志推导 Lean 步骤的证明。现有生命周期词汇包括 acceptCreate、acceptPause、acceptResume、acceptKill、rpcTimeout；既有观察包括 running、phase_hint、snapshot_present、snapshot_deleted，更多规则使用其声明的扩展字段。

### 映射与判定边界

- 接受请求、动作真正执行、稳定状态发布是不同事件。kill 的 HTTP 成功不能直接当作 VM 已停止证据；是否表示已接受终止意图也须由固定接口合同支持。
- running store 的记录是具体系统中的代理观察，不自动等于真实执行权限；快照存在不自动等于持久可恢复。
- generation、epoch、精确 VM/slot 或权限策略无法观察时必须保留未知，不能从 API 成功推断。
- `tools/mapping/clauses.json` 与 `tools/mapping/probes.py` 保存旧 harness 的观察投影；修改底层版本、配置或字段后必须重新核对映射，不能直接迁移旧结论。
- Lean 的 `trace_monitor_sound` 证明的是其声明投影与合法模型轨迹之间的关系，不等于 Python 所有规则或现实观察适配器都已验证。

当前 CLI 退出码：0 = Conforms，1 = Violates，2 = Inconclusive，3 = 参数错误。Conforms 只表示启用且可应用的规则没有报错；必要字段缺失会跳过规则，因此**完全没有有效观察也可能得到 Conforms**。必须另核对实际规则覆盖，不能把绿色结果当作系统合规证明。

报告违反时仍需复核固定版本、配置、观察映射和底层合同。契约不同、配置不支持、观察不足与真实违反分别记录；工具输出不能直接升级成漏洞结论。

### 需要外部环境的旧工具

`tools/trace/collect.py`、`observe.py`、`run_scenarios.py`、`scripts/repro_f1.sh` 依赖旧本地 E2B harness：API、Redis、Postgres、带观察端点的 dummy node 和已填充的测试数据；它不是 Firecracker 实机。

连接和本地测试凭据由 `tools/trace/harness.py` 读取的环境变量提供，不能使用生产凭据。此类采集会创建或改变测试沙箱，不属于只读命令，不进入基础 CI。重新使用时先自行准备固定环境、检查资源与费用范围；本页不承诺旧环境现在可直接运行。

固定源码出处与版本见[证据索引](../references/code-evidence.md)及 `references/source-manifest.json`。取得对应 research-sources 后可用 `scripts/check_references.py` 复核版本、哈希和本地链接；没有本地源码时不要把检查未运行说成已通过。

`tools/issues/` 的旧语料处理和 `tools/mapping/` 的校准仍是辅助研究工具，不是另一份产品规格。它们的输出与一次性实验记录放 artifacts 或任务附件，不在 docs 中持续追加报告。
