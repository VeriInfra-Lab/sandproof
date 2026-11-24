# 保存顺序与完整旧责任检查的组合

[证明工作流](../../../docs/proof-workflow.md) · [来源保存顺序](../source-save-order/unload-parent-link/README.md)

本包固定 Rust 输入为 integration `60aad8af23e317f5b1ba913518603d1fda015f4c`，
复用 #1032 的 `0b00572ed94fe72ae8cb94b9919f081c9ab1e892` 保存顺序和 #945 的
`5f84dbf199c19ffb4505ca657d1c9fb11905e65d` 完整比较、首个匹配、已存事实提供者。
这些是精确 Git 对象，不随远端分支头变化。全新克隆先取得两个 PR 的公开 head ref，
并保留上述提交；检查器读取其原文件，不复制一份新的协议规格。

`complete_save_retention` 对任意两份完整记录向量复用原 `SourceFirstMatch.retained`、
`SourceWorkMatch.full_derived_contract` 和 `SourcePreservation.Expected`。每条旧记录先查其
完整 binding；选中首条完整 `SourceWork` 相等的记录，并在同一对记录上核对旧事实。
读取、头部、容量和编码前缀接受后，完整保留谓词为假时返回 `Conflict`；为真时原样返回
原写入的整个结果，包括错误与翻译器失败。不要求向量唯一，也不以摘要相等替代全文相等。

两份实际抽取共用同一 `expected` 合同。正常见证保留原工作；错误 Rust 只把实际内层
`same_work` 换为 `same_binding`，使配置长度已变的记录进入原写入并违反同一合同。
原保存顺序的十二条定理在当前同类型视图中重新检查，较早的拒绝不要求键查询返回成功。
#1032 的跨视图错误载体传输留在其原项目；本包直接使用同一个生成的完整 `JournalError`。

`bridge.rs.in` 是隔离副本中的薄接线：read、limit、encode、write 原样委托；retains
直接接到原完整记录检查。原父调用传入原 `JournalSourceSave` 和 `JournalSavedKeys`。
IR 核对完整参数与原 `BTreeSet::contains`，父视图包含全部共有生成定义。生成体不修改。
该机械分段与生产源码的对应仍以精确源码审查及 Ning 原生对照为条件，不称生产重构的
一般等价证明，也不修改生产文件。

形式定理从已解码值及具名 read／limit／encode 正常结果开始。键接口按原完整 32 字节值
返回，复用 V0 的逐符号比较合同；实际 SQL、JSON／SHA、配置读取、键集合构造、事务、
提交未知和物理资源事实仍由原提供者落实。父捕获的四个不透明 I/O 入口只用于接线审查，
不导入为可信业务模型。正常返回不等于已授权或已耐久接受。

## 证明机开发复验

使用固定 Charon/Aeneas、已注册 Lean 服务、共享重锁和独立 `.lake/build`。不安装另一套
普通测试环境。输出目录须全新，捕获和工作树所在文件系统均须至少有 80 GiB 空间。

```bash
entry=verification/experiments/source-save-retention
export SANDPROOF_ACCEPTANCE_ROLE=formal
mkdir -m 700 "$capture_root"
for mode in correct binding-only parent; do
    bash scripts/python-entry.sh "$entry/capture.py" --mode "$mode" \
        --output "$capture_root/$mode" --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/check.py" --captures "$capture_root" \
    --output "$formal_output" --tools "$fixed_aeneas" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp" --lock "$shared_proof_lock"
```

`check.py` 核对完整 Rust 源快照、实际生成体和父调用的共有定义，在两个独立项目检查
全部实际 Lean 文件、完整服务诊断、全部依赖审计和固定库源码，并重现最终输入闭包。
这是有条件开发检查；普通回执与公共采用缺失时不会报告完整正式验收。

## Ning 原生与公共验收

从本 PR 的干净精确提交运行以下入口。三份源码和 Cargo target 相互独立，实际测试程序
必须本次重新编译同 crate 的业务源码。原版和组合版执行完整原 source-work 测试组及两个
原 SQLite 断言；错误版必须编译成功，并命中完整工作检查的原断言。源码、完整日志和
二进制摘要留在 Ning 的独立私有证据目录。源码和目标放到既有回收工具分配的空 payload；
不得将原始证据放进该目录。可执行文件是可重建临时产物，不作为长期原始证据保留。

```bash
SANDPROOF_ACCEPTANCE_ROLE=native bash scripts/python-entry.sh \
    verification/experiments/source-save-retention/native.py --output "$native_output" \
    --work "$allocated_payload"
```

开始前按 [CI 临时产物规则](../../../docs/ci.md#显式退休的临时产物)由原执行账号运行
`allocate --kind acceptance --candidate <本PR精确提交>`，将返回的空 payload 传给 `--work`。
结果原样记录工作目录身份，不据测试退出码伪造资源收尾。成功、失败或取消后，先独立
保留原报告、精确输入、日志和二进制摘要；原执行者确认所有自有进程/挂载/资源结束，
再按原结果提供真实收尾收据并调用既有 `finish`。未知或在途时保留原位，不自动删除。

主控同批协调完整普通入口、`proof_native_suite`、`scripts/verify-controller.sh`、公共根、
Stage1 登记及覆盖记录，不单独重复共享整套验收。采用候选见 `registration-candidate.json`。
本包不替代 #1032 已保留的真实调序错误版或其它下游来源保存、回执和继续协议证明。
