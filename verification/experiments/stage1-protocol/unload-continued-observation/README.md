# 继续卸载的原回执读取

本包对应 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635) 的
`full::continuation::ready_records`。实际函数保留原 SQL、原 operation_id 参数、
`ORDER BY id` 和同一次 `query_map` 游标。成功匹配时返回完整工作、动作及原始回执字节。
缺匹配时返回 UnresolvedAction，读行、解码或业务首错停止后续取行。重复有效回执按原规则拒绝。

`ContinuedDriverAgreement` 证明固定工具抽取的实际循环与 #770 的逐行驱动逐步等价。
`ContinuedDriverCaller` 将该等式用在实际入口，保留原 SQLite 逐操作合同、历史身份表示、
留存记录读取、动作与回执检查和 HostEvidence/Serde 的具名条件。其余来源业务、事务登记
和外部物理效果分别在所属切片验收。

## 复现

`extraction/` 保留固定 Charon/Aeneas 的原始 Lean 输出与模板。
[抽取参数及指纹](capture-profile.json) 记录原工具的选择项和实际两个透明入口。最终候选已重抽，四份 Lean 只差
continuation.rs 的源码位置注释，因此沿用已审计的生成视图。
`proof-inputs/` 提供同一生成调用、原边界的具体类型和证明。
[生成体对应](driver-capture-link.json) 记录循环改写前后的精确比较。
`sources.json` 引用仓库已有 #770/#771 输入，特别是 #771 已登记的
`SandproofOriginalSourceRows.Types` 组合视图。其它公开证明包保持自己的适用输入。

在固定 msra 工具环境和原证明锁下，从仓库根运行：

```sh
shasum -a 256 -c verification/v3-unload-continued-observation-inputs.sha256
python3 verification/experiments/stage1-protocol/unload-continued-observation/materialize.py <新私有目录>
# 将固定 Lake 依赖接入新目录的 .lake/packages，再在该目录执行：
lake env bash -eu -c '
  export LEAN_PATH="$PWD/objects${LEAN_PATH:+:$LEAN_PATH}"
  while IFS= read -r module; do
    mkdir -p "objects/$(dirname "$module")"
    lean --root="$PWD" -o "objects/$module.olean" "$module.lean"
  done < build-order.txt
'
```

ning 的 `native-tests.rs.in` 是受影响的 SQLite 对照用例。在原版和同一候选的隔离
controller-service 测试文件各装入一次。用例独立确认原始 BLOB 保留、列错误和 JSON 错误
的先后，以及重复有效回执优先于后行列错误。原测试、生成物和内核审计按各自执行身份保留。
在同一夹具上，正确候选通过。隔离错误版只将扫描中的
`Err(error) => break (Err(error), next_state)` 改为 `Err(_) => state = next_state`。
错误版编译成功，在同一个 JSON 首错断言处失败。原错误版只用于验收。
