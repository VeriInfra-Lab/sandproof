# 卸载观察读取原 Create 选项

本包对应 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635) 的
`admission::read_create_options` 和它调用的 `CreateOptions::encoded`。
实际编码按顺序检查原模板、环境变量和 metadata。合法 UTF-8 中的 ASCII 等号和
NUL 各占一个字节，不会出现在多字节字符内部。实际循环在首个无效键值处停止。
编码使用原完整选项和固定 Serde 派生，再检查 65536 字节上限。

`SourceOptionsEncodedActual` 对应实际字段循环和编码体。
`SourceOptionsEncodedLink` 把它装入实际读函数的编码位置。读函数使用原 connection、
sandbox 键和原 BLOB；在原身份、查询及解码条件成立时，只在完整重编码等于原 BLOB 时
接受该选项。原编码失败保留 InvalidCreateOptions。缺行时的零会话计数及默认值是
`SourceOptionsRead` 的独立分支，仍沿原 SQL 和给定的默认值实例。

[依赖登记](../../../../docs/formal/v3-transaction-source-boundaries.md#a6b4-原-create-选项编码的单句转调)限定
`serialize_create_options` 的原值与返回接线。证明另使用实际 String 字节视图、
原有序 BTreeMap 迭代实例、SQLite 与具体 Serde 解码条件。原模板的
`TemplateBinding::is_valid` 调用结果由其具名项目提供方承担。
原记录身份本身不推出模板已有效。卸载父调用的其余检查和 #637 的持久登记单独交付。

## 复现

`extraction/` 保留固定 Charon/Aeneas 的两个原始生成视图。
[抽取参数与摘要](capture-profile.json) 记录原始捕获、生成体和原 MSRA 结果。
`proof-inputs/` 用带具体签名的外部条件实例化生成视图，`sources.json`
固定仓库内已验收的导入闭包。新候选与原编码候选的 Rust 文件逐字节相同。
实际读函数保留原源码及相同调用签名，其原捕获中的编码位置由本包实际生成体填入。
原读函数的 SQLite、解码和完整比较均保持。

在固定证明环境和原证明锁下，从仓库根运行：

```sh
shasum -a 256 -c verification/v3-unload-stored-options-inputs.sha256
package=verification/experiments/stage1-protocol/unload-stored-options
python3 verification/experiments/stage1-protocol/unload-continued-observation/materialize.py \
  <新私有目录> --package "$package"
# 将固定 Lake 依赖接入新目录的 .lake/packages，再在该目录执行。
lake env bash -eu -c '
  export LEAN_PATH="$PWD/objects${LEAN_PATH:+:$LEAN_PATH}"
  while IFS= read -r module; do
    mkdir -p "objects/$(dirname "$module")"
    lean --root="$PWD" -o "objects/$module.olean" "$module.lean"
  done < build-order.txt
'
```

ning 上分别将 `native-tests.rs.in` 附加到原版和候选的隔离
`controller-service/src/host_contract.rs`，运行
`cargo test -p sandproof-controller-service --lib --locked --offline host_contract::proof_create_options::`。
两版各两条测试覆盖模板、Unicode、完整 JSON、空键、等号、NUL、精确上限和超限。
隔离错误版把环境键的 `b'='` 检查改为 `b'~'`，在同一个
InvalidCreateOptions 断言处失败。生产代码只使用候选实现。
