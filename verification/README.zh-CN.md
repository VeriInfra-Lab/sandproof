# Rust 实现与 Lean 规范的验证链

[文档目录](../docs/README.md) · [证明操作流程](../docs/proof-workflow.md) · [覆盖与剩余义务](../docs/proof-coverage.md)

这里保存的不是另一份手写规范，而是实际 Rust 控制核心经过 Charon 和
Aeneas 抽取得到的 Lean 语义。最终的一致性定理直接比较两边的可执行
处理函数。

固定工具链如下：

- Rust 控制核心：Rust 1.97.0；
- Charon：`b104e24fea7d721b71e6c39fd70f26ff20bc0980`；
- Aeneas：`505b6ca35217e7be5c96c3e2f8045edfbdf47291`；
- Aeneas 使用的 Rust：`nightly-2026-08-18`；
- Lean：4.31.0。

目录含义：

- `aeneas/llbc/`：从实际 Rust 编译器中间表示抽出的输入；
- `aeneas/generated/SandboxControllerCore/`：Aeneas 生成的 Lean 类型和函数；
- `SandboxModel/RustRefinement/`：生成语义与协议规范之间的表示关系和一致性证明。

`FunsExternal.lean` 是少量 Rust 标准库函数的可计算模型。Aeneas 同时会
产生一个以公理占位的模板；重新生成脚本会删除模板，项目只编译这里的
完整定义。

复现命令：

```sh
scripts/bootstrap-aeneas.sh
scripts/verify-controller.sh
```

第二条命令依次检查 Rust 格式、单元测试、静态检查、重新抽取 Rust 语义、
编译全部 Lean 定理，并拒绝 `sorry`、`admit` 或项目自定义 `axiom`。

本页只维护工具链与产物含义，不维护阶段进度；已覆盖入口和剩余义务统一
见证明覆盖记录。已有纯核心定理不自动覆盖新增服务或真实持久化与执行。
