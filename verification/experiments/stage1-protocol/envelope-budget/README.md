# 协议 4 封套的结构大小与有界往返候选

接续[原字段与条件字节组合](../envelope-bytes/README.md)，复用同一完整
Accepted/Recorded 表示和原验证器。范围见 [V3 计划](../../../../docs/formal/03-protocol-inbox.md)，
固定库责任见[逐符号依赖](../../../../docs/formal/dependencies.md)。

两个薄根直接读取当前生产 `REQUEST_BODY_BYTES`、`MAX_OPTIONS_BYTES`，由固定
Charon/Aeneas 生成透明定义。Lean 证明其值分别为 278528、65536，覆盖原平台的
32/64 位 `usize`。旧协议的 270336 不是当前生产上限。

`EnvelopeBudget.lean.in` 对原 `EnvelopeSchema` 的全部字段建立结构大小度量：
标识符按 UTF-8 字节数乘六、u8 数组按每项四字节、有界整数与原固定键名逐项计数。
一般性定理覆盖任意完整值和任意选项数组；Accepted 保留 Recorded 外层所需余量。
自有字段值域给出最多八个、每个最多 128 字节的标识符，原创建守卫给出选项上界，
由此推出结构估计不超过 270336，落在当前实际 278528 限额内。这里不改变任何
Resume、retention 或授权条件，也不把摘要的 32 字节大小解释为哈希真实性。

`NormalRenderer` 只列这两个确切 schema 在正常分配域中的紧凑输出完成及长度条件，
使用现有 A6 的分工；数值范围、固定键名和自有预算前提由本模块证明。结合前批
`SliceContract`，得到完整有界往返的存在性和五种正常接受见证，不再要求调用者
先给出成功编码。但协议 4 的具体 A6/B4 实例仍须主控采用，不能报告整个 Serde、
任意自定义序列化器、OOM/abort 或实际 Writer 轨迹已证明。

原生回归调用生产 `encode_bounded`，核对最大合法选项、最长编号、五种操作、
Recorded 外层、实际输出长度阈值及完整值往返；另核对“字节仍可容纳”不能替代
Create 守卫。有限回归及 Lean 的字段遗漏反例分别报告，不称为新增 Rust 错误版
形式反例。原有真实错误版和同一完整合同保留。

在固定工具、独立构建目录和共享重锁下运行：

```sh
python3 verification/experiments/stage1-protocol/envelope-budget/formal.py --output /private/envelope-budget
cargo test --manifest-path verification/experiments/stage1-protocol/envelope-budget/Cargo.toml --locked --offline
```

驱动重新抽取依赖的原 map、JSON、Request、四个字节包装及两个预算根，核对完整
LLBC/类型/字典和生成体，调用原严格物理文件服务与内核检查全部实际导入及审计。
公共 Lake 候选仅在私有副本应用。普通检查、共享 native 登记及完整入口交 ning／
主控同批验收；本组件不能签发全 V0、完整 JSON/TLS、持久收件或整个 V3 完成。
