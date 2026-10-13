# Rust 实现与 Lean 规范的验证链

[文档目录](../docs/README.md) · [验证设计](../docs/formal-verification-design.md) · [证明操作流程](../docs/proof-workflow.md) · [覆盖与剩余义务](../docs/proof-coverage.md)

这里保存的不是另一份手写规范，而是实际 Rust 控制核心经过 Charon 和
Aeneas 抽取得到的 Lean 语义。已有纯核心的一致性定理比较两边的可执行处理函数；系统服务还需建立真实提交、I/O、异步与恢复的事件/微步骤对应，不能由核心结论自动推出。

固定工具链如下：

- Rust 控制核心：Rust 1.97.0；
- Charon：`b104e24fea7d721b71e6c39fd70f26ff20bc0980`；
- Aeneas：`505b6ca35217e7be5c96c3e2f8045edfbdf47291`；
- Aeneas 使用的 Rust：`nightly-2026-08-18`；
- Lean：4.31.0。

目录含义：

- `aeneas/llbc/`：从实际 Rust 编译器中间表示抽出的输入；
- `aeneas/generated/SandboxControllerCore/`：Aeneas 生成的 Lean 类型和函数；
- `extract-foundations/`：仅调用生产函数的公共基础抽取根，不复制业务算法；
- `aeneas/foundations/`：公共基础的生成语义和独立命名空间的最小库模型；
- `SandboxModel/RustRefinement/`：生成语义与协议规范之间的表示关系和一致性证明。

`FunsExternal.lean` 是少量 Rust 标准库函数的可计算模型。Aeneas 同时会
产生一个以公理占位的模板；重新生成脚本会删除模板，项目只编译这里的
完整定义。定义可计算且无公理，不等于已经证明与实际标准库等价；使用时仍需对应论证或显式外部模型前提，不能漏记这一层。

固定 Charon/Aeneas 主要支持安全、顺序 Rust 子集，不宣称任意 Rust 无条件无损翻译。编译器中间表示经 Charon 形成 LLBC，再由 Aeneas 生成 Lean 语义；抽取成功不自动涵盖 Tokio/async、锁、数据库或系统调用。不能抽取的自有逻辑需可抽取决定/微步骤与实际事务、锁、回调连接，不用手写简化程序替代真实实现。

编译器前端、Charon/Aeneas 翻译器、Lean 内核与外部库模型的信任角色分别声明。证明绑定源提交、目标平台、features/cfg、生产/抽取编译器、依赖、工具、生成入口和模型指纹；生成文件不手改，测试/摘要不代替翻译器正确性证明。模型性质、指定实现精化与条件下系统组合分别报告，详见验证设计。

复现命令：

```sh
scripts/bootstrap-aeneas.sh
scripts/verify-controller.sh
```

第二条命令依次检查 Rust 格式、单元测试、静态检查、重新抽取 Rust 语义、
编译全部 Lean 定理，并拒绝 `sorry`、`admit` 或项目自定义 `axiom`。

公共基础可独立执行 `bash scripts/verify-foundations.sh`：检查输入指纹、固定工具与平台、重新抽取一致性、真实入口/全部自有函数体、库模型范围、定理编译和实际依赖。冻结环境见 [foundations-environment.json](foundations-environment.json)，源码/模型/脚本/生成输入摘要见 [foundations-inputs.sha256](foundations-inputs.sha256)。独立模块检查不替代完整验收；完整入口仍执行原本地控制核心检查，不跳过其失败。独立工作区可用 `SANDPROOF_PROOF_TOOL_ROOT` 指向已有固定工具，修订和二进制摘要仍需匹配，不改变工具版本。

本页只维护工具链与产物含义，不维护阶段进度；已覆盖入口和剩余义务统一
见证明覆盖记录。已有纯核心定理不自动覆盖新增服务或真实持久化与执行。

新增中心/存储/通信/网关等入口的任务按[V0–V9详细计划](../docs/formal-verification-design.md#2-按真实模块划分证明责任)推进。重新抽取已登记历史本地控制核心和公共基础的明确子集，不覆盖整个中心、存储或通信；各模块首次证明批次须增加实际抽取根、外部模型/异步或脚本桥接、版本清单与定理依赖审计，再纳入完整验收。旧生成文件编译成功不是新模块抽取或证明通过。

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
