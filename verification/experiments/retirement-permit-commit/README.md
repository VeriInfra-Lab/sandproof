# 材料删除许可的持久提交原生对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

本片复用原 journal 测试
`old_material_settlement_preserves_live_second_restore_credentials_and_history`。
原历史已有持久后继、精确 consumer release 和另一轮正常恢复；取得旧材料删除许可后，
必须实际读到旧来源的 `material_use=false` 与固定 Retiring 计划，原后继资格保持。
完整原 Rust 测试不变，成功宿主效果仍由原受控提供方供应。

私有错误版只从 `prepare_lifecycle_material_retirement` 删除一次 `codec::save`，
保留原事务、eligibility、完整来源、计划绑定、使用关闭、结构/关系校验、commit 与
前后故障点。错误版在内存中关闭使用和生成计划，却未把它们保存到原 source rows，
随后仍返回真实 `MaterialRetirementPermit`。同一原测试必须取得该许可、读回原 journal，
在 `assert!(!retiring.restore.as_ref().unwrap().material_use)` 被拒绝；更早错误、编译失败、
零测试、忽略或超时不算反例。正确与恢复版执行相同原合同并通过。

原持久后继/精确释放的候选选择、显式 release schema，以及第二次 Full 保留原 backing
三个控制在三版均须通过。不以全部拒绝满足安全条件。完整 material-retirement 测试文件
单独绑定，所有其它提交输入也逐一检查。

这是第四类“检查引用后无保护删除”的 **journal 删除许可持久边界**代表：许可返回时
旧使用和计划仍未关闭/保存。原断言在取得许可后即失败，没有执行物理删除、接受新的
reader 或制造真实并发 check-then-delete；不能据此认领完整第四类、E10、六类矩阵、
KVM 或新增 Lean/抽取。实际程序和三版结果绑定精确候选另行记录，最终组合交 ning。

## 执行

复用[共享原生运行器](../unknown-host-release/README.md)及其严格产物准入。从干净精确
候选导出完整 Git 归档，按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)分配
自有 0700 空 payload。证据、原共享 Cargo 缓存和源码分开；固定 Rust/Cargo 1.97.0、
原 CARGO_HOME/重锁、离线依赖、默认并行和期限保持。三版 service libtest 均须实际
新编译，原 test/debug_assertions/overflow_checks 全为 JSON 布尔 true，fresh=false，
package/manifest/src 与实际程序对应本副本。

```sh
bash scripts/python-entry.sh verification/experiments/retirement-permit-commit/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

保留原始编译 JSON、完整输入、实际程序/退出及原断言位置。失败不自动重试或改签；
原执行者核对完整宿主引用和资源结束后，按原状态退休，保留源码宽限期、原始证据及
现役缓存，可重建程序样本登记有限复核期限。本入口不登记为 `proof_native_suite`
形式准入或 V3 完整封口。

检查器自测不运行 Cargo，也不作为上述 Rust 反例：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_retirement_permit_commit_native
```
