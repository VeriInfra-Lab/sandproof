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
