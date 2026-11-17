# 活跃 registry pin 阻止窗口退休的原生对照

[验收清单](../../../docs/testing-plan.md#同一性质拒绝的错误版本) · [开发规则](../../../CONTRIBUTING.md)

复用原 `active_native_registry_pin_vetoes_close_until_original_certificate_settlement`。
原真实 SQLite 历史已经完成本地清理并保存完整中心确认，随后在同一 journal 登记活跃的
模板验证责任。`close_lifecycle_window` 必须返回 `UnresolvedAction`，保持原窗口及验证绑定；
原证书结算后才允许正常关闭。原完整测试不变，Host 效果由原受控提供方供应。

私有错误版仅在原关闭函数的 `host_slot_release_pinned(local.sandbox)?` 后增加
`&& false`，仍执行同一次查询并传播原错误，但忽略其正结果。其它身份、终态、全文、
确认、来源、native cleanup、save/commit 及故障点全部保留。错误版必须实际新编译，
在原活跃 pin 的同一 `UnresolvedAction` 断言失败；编译失败、早期错误、零测试、
忽略或超时均不算反例。该拒绝点之后的断言未运行，不从失败推定实际物理释放。

三版共同执行原关闭前后崩溃、未知提交的两种耐久结局，以及外来/改全文关闭拒绝三个
控制；正确与恢复版也执行同一个完整活跃 pin 测试，包括责任结算后正常关闭。
完整窗口测试文件单独绑定，全部提交输入逐字节及权限核对，不修改或补弱原断言。

本片是窗口退休的有限原生责任边界。它补充已有纯值窗口校验与其它原生代表，不能
由此声称完整 V3/V5、E8/E15、六类矩阵、真实资源回收、KVM 或新增 Lean 证明完成。
精确候选与实际执行结果在 PR 单独记录，最终组合和实机验收仍交 ning。

## 执行

复用[共享原生运行器](../unknown-host-release/README.md)的严格产物准入。
从干净候选导出完整 Git 归档，按[临时产物规则](../../../docs/ci.md#显式退休的临时产物)
分配自有 0700 空 payload。证据、源码及原共享 Cargo 缓存分开，使用原重锁、
固定 Rust/Cargo 1.97.0、离线依赖与默认测试并行度。三版实际 service libtest 必须
新编译，核对真实 package/manifest/src 以及 test/debug_assertions/overflow_checks
三个 JSON 布尔 true；不能沿用其它源路径的旧程序。

```sh
bash scripts/python-entry.sh verification/experiments/retention-registry-pin/native.py \
  --head EXACT_COMMIT --archive /private/source.tar --archive-sha256 SOURCE_SHA256 \
  --work-root /private/allocated-payload --target /private/original-cargo-target \
  --output /private/new-evidence --lock /private/original-heavy-lock
```

保留完整输入、原编译 JSON、实际程序、退出及断言位置；失败不自动重试或改签。
原执行者完成宿主全进程引用和资源终止审计后，按原状态退休自有 payload，保留原始
证据、源码宽限期与现役缓存。实际程序复核样本按原规则登记有限期限。
本入口不注册为 `proof_native_suite` 的形式准入，不签发 V3 或 ning 正式封口。

检查器自测不运行 Cargo，也不充当上述 Rust 反例：

```sh
bash scripts/python-entry.sh -m unittest tools.tests.test_retention_registry_pin_native
```
