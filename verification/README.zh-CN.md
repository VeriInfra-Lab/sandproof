# Rust 实现与 Lean 规范的验证链

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

当前阶段首先闭合请求处理路径，随后按同一方法覆盖动作回执、调度、稳定
状态发布和执行节点授权/完成。只有所有入口的双向一致性定理、持久化原子
提交合同和明确的公平性前提都成立，才会把系统称为边界内完整验证。
