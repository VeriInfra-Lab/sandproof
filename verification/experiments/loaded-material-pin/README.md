# 恢复后材料使用 pin 的原生对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本包复用真实 source-work 测试
`second_full_capture_keeps_first_source_and_running_backing_bound_to_their_original_operations`。
原测试通过真实 journal、SQLite、World 和派发路径完成恢复，在后继为 Running 时读取
原来源记录，要求 `restore.material_use` 仍为 true；随后才开始下一次 Unload。完整原
Rust 测试文件不变，宿主效果由原受控提供方供应。

私有错误版在 Load 已完成后的激活登记点，把持久 `MayHaveExecuted` 标记错误地视为
材料使用结束，实际保存 `material_use=false`。同一文件的相关工作、记录校验和激活
重放门同步采用该错误寿命政策：共有五处有界替换，先逐处核对唯一原片段及所属函数，
全部满足后才写入。仅翻转 pin 而在更早处得到 `CorruptStore` 不算本反例。

原来源、owner/session/node、代数、摘要、恢复计划、效果来源、激活动作及完整重放
比较保持；真正退休后的拒绝仍沿原 `has_closed_material_use`。原关闭事务与材料退休
实现不变，独立释放/删除门也继续存在。错误版必须通过原读取与校验，实际到达原
`assert!(first_before.restore.as_ref().unwrap().material_use)` 的准确位置；其它 panic、
编译失败、零测试、忽略或超时均不算反例。正确与恢复版执行同一断言并通过。

未知 Prepare 后 Destroy 保留 backing 并最终收尾，以及后继持久接管和精确使用者
释放后才允许材料退休的两个原控制，在三版均须通过。不以全部拒绝满足安全条件。
本包检验实际持久 pin，尚未执行时不预记三版结果；结果绑定精确提交另行记录。

这是第五类错误版的一个 journal 使用寿命代表，错误写入点是 **Load 后的激活登记**。
它没有执行真实 Firecracker Load、文件映射、unlink 或退额，不能从该 pin 反例推出
物理材料已经被删；不代替“Load 返回立即释放”的全部路径、六类矩阵、KVM 或新增
Lean/抽取证明。真实 VM、材料、额度和最终结算仍交 ning 在最终组合上验收。

## 执行

复用[共享原生运行器](../unknown-host-release/README.md)。从干净精确候选导出完整
Git 归档，核对所有成员的对象、权限与输入；按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)
分配自有 0700 空 payload。证据与原共享 Cargo 缓存分别位于 payload 之外。
沿用固定 Rust/Cargo 1.97.0、原 CARGO_HOME 和共享重锁，不改并行、期限或断言。
三版实际 service libtest 均须新编译：原 Cargo 记录的 `test`、`debug_assertions`、
`overflow_checks` 全为 JSON 布尔 true，`fresh=false`，并绑定本副本的 package/manifest/src。

```sh
bash scripts/python-entry.sh verification/experiments/loaded-material-pin/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

保留原编译 JSON、stdout/stderr、实际退出、断言位置、完整输入和二进制摘要。故障样本
若需另存，登记保留理由及退休条件，不无限积累可重建程序。失败不自动重试或改签。
原执行方确认本批精确资源和完整宿主引用后，按实际状态退休；源码宽限期、原始证据
和现役缓存保持。本入口不登记为 `proof_native_suite` 形式准入或 V3 完整封口。

无 Cargo 的检查器回归：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_loaded_material_pin_native
```
