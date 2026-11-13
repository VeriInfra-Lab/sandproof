# 当前 V3 JSON 的条件组合

本目录提供当前 V3 的有限条件组合，复用原 AcceptedWire、EnvelopeBytes、EnvelopeBudget 和 EnvelopeWriter。
九条证明正文已由固定原服务 lean_run_code 检查通过（无诊断、非陈旧）；新增正式审计及整批验收另由公共入口执行，不能从局部成功推定通过。
它只组合已有真实生成体和已有库条件，不实现 JSON，也不修改 Charon/Aeneas。

具体库与薄接线范围以[依赖登记](../../../../docs/formal/dependencies.md)的“当前 V3 JSON 的具体采用条件”为准。
源检查单独不能证明 `SliceContract`：本批明确按已有 A6/B4 标准采用固定库符合性条件，
并用原字段回调和守卫定理导出自己的结论。来源捕获、原生反例、内核证明分别记录。

## 调用方提供什么

- `AcceptedCallBinding`：真实空捕获闭包对原完整切片调用固定 Accepted 解码，结果原样返回。
  独立认证上下文仍由调用方选择；这个条件不保证上下文来自正确端点。
- `EnvelopeBytes.SliceContract`：该固定 schema 的原输入／字典／自有字段回调与编解码关系。
  不是一个“解码结果已获授权”的条件。
- `EnvelopeBudget.NormalRenderer`：普通分配域下，该固定紧凑 schema 的完成及原结构长度界。
  本稿由真实解析和业务守卫成功推出 InputDomain/ValidEnvelope，不要求调用方再次假设它们。
- `RecordedWriterCallBinding`、原 `BufferLaws` 和 `StandardCalls`：实际原 Writer、同值固定派生、
  有序原写调用及正常空缓冲。普通 Vec 参考不能单独提供这个 Writer 条件。
- Writer 的 `emitted`、`sameRendering` 和 `success` 保留在结论的准确成功域中。
  它们不表示任意 IO 必成功；所需正常完成域来自已有 NormalRenderer 或实际调用者。

## 可以直接消费什么

`actual_accepts_iff` 表示原同步函数接受恰好需要解析得到同一封套并通过原业务守卫。
解析错误先于守卫，原失败和发散不变成正常成功。`accepted_normal_bounded_roundtrip`
由实际解析／接受结论获得原请求预算内的完整往返。

`recorded_full_value_and_match` 保留回执的完整值及原请求全文匹配。
`recorded_writer_from_actual_input` 接收业务提供的 `sameIntent`，即回执内意图等于原解析意图，
结合实际有界 Writer 的成功返回，得到原完整回执可解码和 `length <= limit`。
这里的 limit 必须由调用方连接真实常量；定理不自行选择更宽的预算。

Inbox 继续消费已有 `InboxLoad.DecodedStringsRepresentable`：固定解码返回的普通 String
提供 `config.host_epoch.boot_id` 的可表示条件，不提供 `ConfigValidation.Valid`。
required_option 按原 Option 字典的机械转发连接；其字段缺失与显式 null 不混同。
load 的规范字节比较、两个配置读取、真实校验和 save／四编码字典的业务判断继续使用原证明，
不在本目录另写一套 Inbox 算法或 schema。

不覆盖实际认证来源、SQL 同事务／恢复、业务回执选择、物理资源、异步 handler、
自定义资源 Drop、panic/unwind 或 OOM/abort；这些条件有各自的提供方。

## 正式接入

将 `ProductionEnvelope.lean.in` 原样安装为 `SandboxModel/Lifecycle/Protocol/ProductionEnvelope.lean`，
沿现有独立视图和实际文件检查接入，九个准确审计目标见 `interface.json`。不新增 Rust 抽取算法：
Accepted 复用原 `accepted_decode` 真实生成体，Writer 复用原 `encode_bounded_using`。
当前具体原闭包及字段的来源准入由 `accepted-json-parser` 提供；它不能单独签发 SliceContract。
共享登记／冻结与最终候选的实际文件、审计及双机验收由统筹完成，不运行旧整套脚本冒充本模块已验收。

## 从仓库准备独立视图

`view.json` 是这一有限视图的实际文件清单，不是新的证明结果。`materialize.py` 的
`install(project)` 只接受新的空目录；也可显式调用：

```sh
python3 verification/experiments/stage1-protocol/production-json/materialize.py \
  --project /private/new-json-project
```

它只安装清单中的文件和来源回执，不抽取、编译、运行测试、安装工具或签发通过。
调用方随后设置原 `.tools`／固定 packages，并沿现有验收入口检查；本命令成功本身
不是验收。当前清单有74份文件，使用正式 ProductionEnvelope 页替代无消费者的旧
`artifacts` 草稿，不额外复制一份旧稿来凑数量。

65项从当前 tracked 路径逐字复制；原 Accepted profile 和独立 Result::map 的四份
生成文件在 `raw/`，均来自已接收的原交接包，没有编辑生成内容。`raw/provenance.json`
保留原捕获、源输入、生成物／LLBC 摘要及原工具流程；其中旧机器路径仅作历史来源，
安装时不读取它们。实际源输入、提供方和原始文件摘要不符时拒绝，不以刷新摘要冒充
重新抽取。

Encoding 通过原 `local_view.install` 在临时私有 scratch 中取得，只安装其原输出；
不复制另一份变换算法，也不把 scratch 的抽取根带入实际视图。Writer 使用当前 tracked
完整 Funs／Types（0a6bb723／c58fbdbf），保留新增错误分支。正式页保持03387795及九条
审计，原正确／错误矩阵和数学不由这个安装器再跑。

Lake 使用现有 `local-view.toml.in`（f38d6b65）。它与先前通过视图的旧配置 d412ef1b
字节不同，但六个实际消费库的 module→source 路径逐项相同，比较记录在清单中。
Carriers／Manifest 是未消费的声明，不因此增加已证明范围；不能从默认 Lake 目标
代替 ProductionEnvelope 实际文件和九项审计。

`formal.py --output /private/new-readback` 是 `protocol-convergence` 使用的有限
证据读取入口。环境变量 `SANDPROOF_PRODUCTION_JSON_ORIGINAL_EVIDENCE` 指向原
`json-current-writer` 证据目录；该目录须包含完整的原结果、十页内核日志、服务请求
和原服务日志。入口检查 `readback.json` 固定的原字节、十页与30项准确审计，并用
原服务的完整 Lean 源指纹核对69份当前 Lean 输入；随后从当前仓库安装74份有限
视图文件，保留六个实际消费库的对应关系。它不重新运行证明或改变原执行身份。

结果显式标明 `REUSE`、原耗时和原 CPU，新增耗时仅是证据核对与安装。缺失、变化、
未完成的原记录或当前输入不符均拒绝，须由统筹决定实际补验；不能自动换成另一份
成功结果或只依据 `complete` 字段放行。原生 Accepted 检查与整批普通/形式准入仍由
公共入口负责，本读取成功本身不等于整套或完整 V3 通过。
