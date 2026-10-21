# 原 Metadata 目录权限判断

返回[证明工作流](../../../docs/proof-workflow.md) · [依赖保证](../../../docs/formal/dependencies.md)

本模块只验证原 `sandbox-internal/src/protected_files.rs::directory(&Metadata, owner)` 如何判断一份已经取得的 Metadata。它不是路径遍历、打开文件或完整受保护读取的证明。来源基线为 integration `139f4db6`；原文件及 Cargo/工具输入与上一批 `799d49e5` 所用输入一致。生产文件保持原样。

采用现有 A3 的三个具体 getter：固定标准库 `MetadataExt for Metadata` 的 `uid/mode`，以及 `Metadata::is_dir`。固定生产和抽取两版源码均沿保存的 `FileAttr/stat` 字段读取；这些 getter 不重新执行 `stat/open`。模型只保留这三个读取结果的投影，同一个输入 Metadata 的调用不能替换成另一份快照。它不要求运行时保存新字段，也不把原函数或外层权限流程设为可信。逐项条件见 [contracts.json](contracts.json)。

`AcceptanceSafe` 对正确版和真实错误版使用同一个性质：正常接受必须对应目录类型、允许的属主，以及无组/其它用户写权限或 root 属主的 sticky 例外。还须证明满足条件会被接受，不能用全部拒绝满足安全性。正常拒绝保持原 `Untrusted`。完整原 IR 重放核对原接收者、参数、分支、位掩码和其中的 unwind 内容；Lean 对应定理覆盖正常结果，不宣称证明了 unwind 清理。

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

完整组件复现入口为：

```sh
python3 verification/experiments/protected-directory/check.py
python3 verification/experiments/protected-directory/proof.py \
  --archive "$FROZEN_SOURCE_ARCHIVE" --tool-root "$FIXED_TOOLS" \
  --std-evidence "$FIXED_STD_SOURCE_COPIES" --output "$NEW_FORMAL_EVIDENCE"
```

原始源码归档由 `git archive 139f4db6990f67c1ed9cd28c4ce28af707d469f9` 得到，摘要见 `environment.json`。标准库证据目录按 `std-sources.json` 的 `版本/相对路径` 保存八份完整源码；它们支持 getter 合同的源码审查，不是标准库的机器证明。运行者先在普通机器和证明机器分别持有现有 native-heavy/formal-heavy 锁，普通和证明步骤不互相持锁等待。

`proof.py` 创建独立 Lake 视图，仅链接固定依赖包缓存，重新抽取正确版和错误版。它使用现有完整 IR 比较器，仅规范化既有 `short_names` 集合顺序；不删除 IR 字段。原生成 `Types/Funs/translation.json` 必须逐字节一致。随后构建本模块，调用原 `lean_session.py` 服务逐一检查全部 16 个实际 Lean 文件，拒绝 partial、stale、超时、错误或警告，并核对 16 条真实审计消息。冻结清单同时绑定原 Rust、模型、生成物、合同、脚本、反例和服务协议。原生证据与形式证据分别保存，不把脚本退出码单独当作全部定理通过。普通机器另运行 `python3 verification/experiments/protected-directory/test_checks.py`，确认源代码、生成物和审计清单变化会被拒绝。

接口结果分别由 `source_ok_iff` / `source_untrusted_iff` 给出接受和拒绝的充要条件；`source_exact` 给出完整正常返回，`source_acceptance_safe` 提供调用者可复用的安全结论。合法私有目录和 root sticky 目录均有原函数接受见证。实际错误版的 `mutant_not_acceptance_safe` 使用同一个 `AcceptanceSafe`，反例为 uid 1001、完整目录模式 `040770`、owner 1001。

两份 stock `Types` 含相同名称的顶层辅助实例，正确版 `Audit` 与错误版 `BadAudit` 因此分别编译和诊断，不能共同导入一个 Lean 环境；它们共享同一份 Metadata/Model。`Elaboration.lean` 只为两个生成 Rust 命名空间提供具名、优先级 1 的 scoped `Decidable` 实例，使原生成的短路 Bool/Prop 形状能够 elaboration；不改生成文件，不为其他命名空间增加全局实例。`ElaborationScope.lean` 实际检查作用域内存在、作用域外不存在。采用的 `Classical.choice` 属于现有审计允许的标准逻辑依赖；原函数对应与错误版反例均经该审计，未增加业务假设或自定义公理。

这是独立组件的验收入口。共享总根、批次缓存、原生证据接入、权威依赖与覆盖文档由统筹接线，并在确定组合提交上运行整套验收。局部通过不能代替该整套结果。旧失败、原始抽取、模板和服务日志留在私有证据；模板中的外部公理不导入证明库。

可依赖的结果只解释这次 Metadata 快照。调用者仍须选对原对象和 owner、正确取得/使用句柄，并证明查验至使用间的控制权、路径替换、全部打开/读取/同步/清理顺序。快照通过不说明未来路径安全、FD 已关闭、物理资源已退出或整个 T8 已完成。panic/unwind、abort、取消和崩溃没有本正常后态；不能据此补造回滚或清理证据。
