# 来源记录实际 Serde 读入口、required_option 与具体库调用准入

[证明工作流](../../../docs/proof-workflow.md) · [完整 nullable 决策](../source-save-load-decision/README.md) · [原读写往返](../source-save-load-roundtrip/README.md)

叠在 #1048，固定生产 `60aad8af23e317f5b1ba913518603d1fda015f4c`。本切片将此前
两个具名 Serde 往返条件拆到原固定库调用与实际字段字典，**不证明整个 Serde/JSON 实现**。

- `original_required_option_forwards_full_value` 对任意原 `D/T/E`、完整两个具体泛型
  字典及原输入证明实际 `required_option` 等于原 `Option<T>::deserialize` 整个返回值。
  不因该字段必须出现而将显式 `null` 拒绝。正常完整 Some 见证和真实
  `drop-required-option-value` 错版共用同一个 `RequiredContract`；错版实际丢失成功值，
  违反同一性质。原失败/空值/其他 modeled effects 均由原完整转发保留。
- `original_decode_forwards_full_input_and_optional_dictionary` 透明证明实际
  `decode_source_payload` 向原 `serde_json::from_slice` 传完整字节、原 Option 字典和原
  `SourceWorkRecord` 内层 Deserialize 字典。原 generic required_option 接的是同一个
  标准 Option 字典接口。实际 `SourceWorkRecord` 使用此前完整真实生成类型，不导入
  Aeneas 外部类型/函数模板的公理。四个实际标准 Serde trait 声明逐字复用原维护策略读端。
- `binding.py` 对四个真实薄调用逐项核对完整 Charon AST、原输入及结果 reborrow/call、
  原具体 `Option<SourceWorkRecord>`/`SourceWorkRecord`/`SourceAnchor` 类型、完整原字典
  与 unwind；Some 编码和裸记录编码使用相同实际记录 Serialize 字典。写端只做 **B4
  原真实调用源准入**，不是写端 Lean 生成体证明。固定工具将完整写端 Serializer 图
  翻译时出现互递归 trait；曾尝试全组、最小薄调用、仅 record 编码仍不能得到可通过
  的实际写端 Lean 体；失败保留，不计为性质拒绝，不换工具或修改生成体。
- `schema.py` 核对实际 SourceAnchor、SourceWork、SourceWorkRecord 完整外层 12+2+6 个
  字段、类型与声明顺序，标准 Serialize/Deserialize、deny_unknown_fields 和四个
  required_option 使用点与原完整生成载体一致，不能代替任意嵌套具体字段 codec 的证明。
  `library.py` 复用原固定库源和实际生产 Cargo 版本/特性，逐符号核对 Option 分派、
  Compact Serializer Some、相同 fresh Vec/Writer、SliceRead/Option visitor、文档 end
  和 deserialize_with 缺字段规则。
- `VecSymbols` / `SliceSymbols` 显式 A6/B4 条件分别连接原实际写入口与读入口；
  `original_some_and_record_have_same_complete_vec` 保持整个 writer reply 与字节结果，
  `original_option_document_reads_written_record` 保持真实非 null 记录的完整值并经原
  文档结束。在这些具名库操作和具体内层 `writtenObject` 状态/字段条件下，
  `actual_typed_decoder_roundtrips_complete_records` 接回 #1047 任意实际编码与原有序
  读回，不把完整内层具体 Serde、真实 SHA、物理事务、业务授权或材料存在声称为已证。

## 证明机复验

三个 Lean 生成视图（原 required、真实错版、原 typed decoder）与第四个源准入视图独立。
第四个仅运行固定 Charon 检查 B4 原完整调用，不伪造 Aeneas 通过。原正式完整闭包重查、
依赖及生成义务复现，新的物理文件和所有 audit 用原注册同一服务检查。

```bash
set -euo pipefail
entry=verification/experiments/source-serde-ports
export SANDPROOF_ACCEPTANCE_ROLE=formal PYTHONDONTWRITEBYTECODE=1
mkdir -m 700 "$serde_captures"
for spec in 'required-only correct required-correct' 'required-only drop-required-option-value required-wrong' \
            'decode-only correct decode' 'call-admission correct call-admission'; do
    read -r profile mode output <<< "$spec"
    bash scripts/python-entry.sh "$entry/serde_capture.py" --profile "$profile" --mode "$mode" \
        --output "$serde_captures/$output" --tools "$fixed_aeneas" --lock "$shared_proof_lock"
done
bash scripts/python-entry.sh "$entry/serde_check.py" --captures "$serde_captures" \
    --parent-output "$decision_formal_output" --current-capture "$canonical_captures/correct" \
    --collector-capture "$collector_captures/correct" --reference "$load_errors_captures/original" \
    --bounds-reference "$record_bounds_original_capture" --cursor-reference "$sql_cursor_original_capture" \
    --collector-reference "$collector_original_capture" --tools "$fixed_aeneas" \
    --lock "$shared_proof_lock" --output "$formal_output"
```

## Ning 原生复验

原版、正确联合版、实际丢失 required Some 值错版均采用原固定生产源；同一真实 serde_json
夹具覆盖 Some/None/原错误、实际全值记录 Some/bare 字节与读回、四个外层必需 Option/普通
可选/未知字段语义。正本实际通过，错版实际编译且执行到相同完整 Some 值断言才是拒绝。
普通和原生实际收据由 Ning，形式通过不代替原生/共享验收。

```bash
entry=verification/experiments/source-serde-ports
SANDPROOF_ACCEPTANCE_ROLE=native PYTHONDONTWRITEBYTECODE=1 bash scripts/python-entry.sh \
    "$entry/serde_native.py" --output "$native_output" --work "$allocated_payload"
```

不修改生产、公共 Lake/Cargo/依赖/覆盖根或公共登记；提供候选差异交主控。真实具体
内层字段/derive conformance (`writtenObject`)、原写端实际机器精化、库正常域、V5
同事务物理列/提交、授权/资源事实及完整 JSON/TLS 系统仍未完成。
