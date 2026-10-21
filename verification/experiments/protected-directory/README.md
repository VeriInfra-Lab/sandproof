# 原 Metadata 目录权限判断

返回[证明工作流](../../../docs/proof-workflow.md) · [依赖保证](../../../docs/formal/dependencies.md)

本模块只验证原 `sandbox-internal/src/protected_files.rs::directory(&Metadata, owner)` 如何判断一份已经取得的 Metadata。它不是路径遍历、打开文件或完整受保护读取的证明。来源基线为 integration `139f4db6`；原文件及 Cargo/工具输入与上一批 `799d49e5` 所用输入一致。生产文件保持原样。

采用现有 A3 的三个具体 getter：固定标准库 `MetadataExt for Metadata` 的 `uid/mode`，以及 `Metadata::is_dir`。固定生产和抽取两版源码均沿保存的 `FileAttr/stat` 字段读取；这些 getter 不重新执行 `stat/open`。模型只保留这三个读取结果的投影，同一个输入 Metadata 的调用不能替换成另一份快照。它不要求运行时保存新字段，也不把原函数或外层权限流程设为可信。逐项条件见 [contracts.json](contracts.json)。

`AcceptanceSafe` 对正确版和真实错误版使用同一个性质：正常接受必须对应目录类型、允许的属主，以及无组/其它用户写权限或 root 属主的 sticky 例外。还须证明满足条件会被接受，不能用全部拒绝满足安全性。正常拒绝保持原 `Untrusted`；读取顺序、原参数、所有比较/位掩码和unwind内容由完整原IR与实际对应共同核对。

错误副本只把原 `0o022` 写权限掩码改为 `0o000`；它必须成功编译、原工具抽取，并由上述同一性质的具体反例拒绝。原生断言和 Lean 反例分开报告。三个既有 rustix 前端失败入口不在本批重试。

普通机器使用固定 Rust、离线依赖、私有 Cargo target 与 native-heavy 锁运行：

```sh
python3 verification/experiments/protected-directory/native.py \
  --archive "$FROZEN_SOURCE_ARCHIVE" --output "$NEW_NATIVE_EVIDENCE"
```

证明机器使用固定原工具、formal-heavy 锁、私有源码/target/build，所有新材料放数据盘。抽取入口为：

```sh
python3 verification/experiments/protected-directory/extract.py \
  --archive "$FROZEN_SOURCE_ARCHIVE" --tool-root "$FIXED_TOOLS" \
  --output "$NEW_EXTRACTION_EVIDENCE"
```

本模块的完整局部 Lean、审计和实际文件服务入口随组件交付，不代替统筹的整套验收。任何未运行项保持未完成，不由生成文件或默认根构建认领。

可依赖的结果只解释这次 Metadata 快照。调用者仍须选对原对象和 owner、正确取得/使用句柄，并证明查验至使用间的控制权、路径替换、全部打开/读取/同步/清理顺序。快照通过不说明未来路径安全、FD 已关闭、物理资源已退出或整个 T8 已完成。panic/unwind、abort、取消和崩溃没有本正常后态；不能据此补造回滚或清理证据。
