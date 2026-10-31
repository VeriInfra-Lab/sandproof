# 原 TLS 证书与有限来源连接

返回[证明工作流](../../../docs/proof-workflow.md) · [既有 TLS 值核心](../../../SandboxModel/Boundaries/TlsPin.lean) · [依赖合同](../../../docs/formal/dependencies.md#a6b4-tls-叶证书指纹接口)

范围：在 integration `a12f4e16b491cd58452d2b967f06af1bfc48a847` 的未改生产 Rust 上，登记原证书摘要的窄原语组合，并检查四个原函数与既有已证值核心的有限来源连接。**这不是原 `from_certificate`、`verify_pin` 或 `peer_pin` 的 Lean 机器精化。** 没有新增定理或正式 Lean 模块，生产代码与 Charon/Aeneas 保持原样。

## 证据层次

- [原完整 Charon IR](original.llbc)保留四个入口、原摘要包装及其闭包、派生相等和 DropGlue 的 14 个本地透明体。四个编译器生成的 DropGlue 带有原 unsafe 标记，按准确声明、类型和完整空效果树单独登记；其余自有体新增 unsafe 仍拒绝。原源码、类型、具体 trait 实例、参数、捕获、正常/展开分支及 Drop 内容均在有限冻结中；未知结构不能靠忽略字段通过。
- [有限检查器](../../../tools/check_tls_certificate_binding.py)只接受本次审查的固定树，另核对两处 `verify_peer` 的原字段/完整参数转调。它不是任意 Rust 的解释器、转换器或等价证明。
- [窄原语审查](primitive-audit.json)区分实际摘要动作、字典成员和关联常量；没有把未实现外部模板作为公理导入。
- [对应表](correspondence.json)及[冻结输入](environment.json)连接原调用点、完整输入视图和既有值策略。沿用已有 `allow`、`peerReply`、实际 bytes 核心与条件组合定理，不声称生产调用了 owned 候选。
- [原生对照入口](native.py)只在普通测试机运行，正确原生产体、既有 owned 候选和错误变体使用相同断言。源码/原生依据不提供一般 Rust 等价证明，也不是 Lean 反例。

## 原输入和自有决定

逻辑链视图为 `Option (List Bytes)`：None 与 Some 空链分别保留，原元素顺序和每项完整 DER 字节不变。pin 视图保留原字段的所有 32 字节。它们是证明侧的值，不执行新的 Rust `collect/to_vec`，也不对无复制原实现增加旧候选的分配前提。

| 原函数 | 保留的决定和顺序 |
| --- | --- |
| `verify_pin` | None/空链拒绝；只选首叶；先摘要再 contains，空 pins 也先摘要；完整相等；原 Ok/Authentication |
| `ServerTls::verify_peer` | 原 certificates 与同一 self 的完整 peer_pins；原结果直接返回 |
| `ClientTls::verify_peer` | 相同转调规则，不能串到 ServerTls 或其他对象 |
| `ServerTls::peer_pin` | 先原 verify_peer，`?` 首错返回；成功后从同一原链再选首叶、再次摘要并返回 |

成功路径两次摘要由两处真实调用及先后关系支持；输出相等定理本身不证明物理调用次数、运行成本或现实时间界。摘要不返回、panic、abort 和崩溃不被伪装成 Authentication。当前原 IR 采用既有 `precise_drops=false`、`desugar_drops=false`，保留其实际正常/展开结构，不能外推为完整 Rust 异常析构或物理退出证明。

## 哪些条件仍来自边界

- A12/B4：同一个 stream 的原 session、该次 `peer_certificates` 原有序链；三个异步来源只登记源码邻接；本批既有 `secure_probe` 回环提供代表性 TLS 原生证据，`data_connection` 的完整异步数据路径未单独验收。
- A6：固定 Option/slice/Vec 的具体操作、完整数组相等和结果转交；原首叶/授权选择保持可见。
- A6/A9：原 `CertificateDer::as_ref` 全字节、固定 SHA-256 及原 GenericArray→32byte 转换。原摘要包装只有这几个机械动作，仍未机器证明。
- B4：原 self/链/pins 的逐符号机械绑定。它不自动提供原借用函数到旧值核心的一般等价。

同步 pin 检查内部没有 await；仅要求本次有效共享借用，不冻结后续 await 的连接、权限或撤销状态。握手之前已经可能产生 I/O。角色/节点/请求/当前授权/撤销、全局预算及部署仍归 V3/V8/V9 等模块。

## 复现

保持固定 Rust、Cargo.lock、原 Charon/Aeneas 与既有运行环境；实际工具摘要见冻结记录。普通测试和结构负例只在 ning；重采集只在 a6000 的独立数据盘源码、target、TMPDIR 和证据目录，运行前取得相应既有重任务锁。不要共享可写 target，也不复制 CI 环境。

```bash
# 形式环境：真实重采集，先保存旧 IR，保留完整源/配置/类型/分支；不运行 Aeneas 或 Lean。
python3 verification/experiments/tls-certificate-binding/collect.py \
  --source "$SOURCE" --tool-root "$FIXED_TOOLS" --output "$FORMAL_EVIDENCE"

# 普通测试环境：私有 CARGO_HOME/CARGO_TARGET_DIR、固定离线依赖；目录必须新建。
bash scripts/python-entry.sh verification/experiments/tls-certificate-binding/native.py \
  --output "$NATIVE_EVIDENCE"

# 单独检查有限源/树准入；这条命令不是程序证明。
bash scripts/python-entry.sh tools/check_tls_certificate_binding.py
```

原生入口核对 package_id、manifest、src_path、业务库和测试二进制的新编译标记与摘要；错误版必须成功编译后被对应原断言拒绝。结构负例直接改变 IR/输入，属于检查器回归，不冒充可编译 Rust 或语义反例。组件测试、CI、统筹完整入口分别报告，不能互相替代。

P1/P2 的借用失败和未实例化摘要外部模型保留在前一封口 `e4af428d`，没有因本批连接登记而变为已证明。共享根、总审计、完整入口、权威合同和覆盖记录由统筹接入。

## 验收状态

[实际结果](results.json)绑定 Rust/测试代码提交 `d588611dde7d4dd13be6e75a1ea8f1bde16d695e`：普通组件入口成功，用时 33.568 秒；原生产体与既有候选各 7 项直接用例和 16 项真实 TLS 回环通过，两个真实错误版新编译后被对应原断言拒绝，格式及两版严格 Clippy 通过。业务库/测试的来源与 fresh=false 均核对。

a6000 原完整 IR 重采集和有限准入成功，用时 8.141 秒；14 个本地体、四个决定入口，零 Lean 检查、零新定理。之后 `72d8064f` 仅补原语说明及既有库布局来源，实际 Rust、检查器、夹具、依赖和工具未变。宁轻量核对 18 个库文件与锁定 crate 原字节、10 组当前准入及原始日志/二进制/rlib 完整性通过（1.175 秒）；形式机同样核对来源和当前有限准入通过（0.811 秒），复用原捕获，没有重编译。

这两份补充记录不合成为“新提交已重跑完整 native.py”的回执。后续共享来源准入已由
#629/#631 接入并完成其限定组合验收，准确版本与范围见[覆盖记录](../../../docs/proof-coverage.md#current-status)。
该结果仍不是原借用入口的 Lean 机器精化，也不能用本组件代替新候选的受影响验收。
原始失败、诊断、二进制、rlib 和源码均保留；公开结果只列摘要，私有路径不入库。
