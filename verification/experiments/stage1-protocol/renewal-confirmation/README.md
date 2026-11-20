# 续租确认值证明

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

本切片固定 #689 的 `15b98c516c7d7eff25e69d80ce745309cb004c07`，生产源仍与 a9c 基线一致。
复用[终局组件](../result-confirmation/README.md)原透明根与 `extract.py --profile renewal`，
固定 Charon/Aeneas 生成 Request/Accepted/Recorded 四根及三个真实续租守卫。
不复制算法，不修改生成函数体，不导入带 axiom 的工具模板。

历史 `153b2d2b514e1c20e73c5a225c0adb6e950ad7c3` 的 `Retention.confirmation_refines` 已有续租形状结论。
其 renewal.rs/request.rs 与当前逐字相同，但传递依赖的协议常量为 3，且 `is_err` 是外部库模型。
本切片保持原形状语义，复用当前请求/封套规格迁移至协议 4 和实际抽取的 `is_err`；不把形状迁移重复计为新业务性质。
历史 `MayDiscard`、本地回收和它们的原证据不在本包迁移范围内。新增匹配与反例只扩展上述明确的值边界。

`RenewalModel` 定义三个完整合同，正确与错误 Rust 使用相同文件及 `RenewalValues`。
`RenewalLibrary` 复用完整请求校验，`RenewalEquality` 连接完整结果/intent 比较；`RenewalBindings` 对任意输入证明
`is_sdk_renewal`、`matches_request`、`ConfirmedRenewal::matches` 的确切布尔结果与接受充要条件。
有效形状要求 key window=2、接受版本至少为 2、期望版本受检加一与接受版本一致、部署一致、
retention slot 合法、请求自身校验通过，以及 Renew 的非负截止时间关系。
普通更新要求 effective=requested，extend_only 要求 effective≥requested；不增加 retention window=key window。
全文匹配包含 intent 的所有字段，确认匹配还包含有效截止时间。
独立认证上下文、当前意图、实际应用结果和耐久保存分别仍由 V7、V1/V2、V4/V5/V6 提供。

`RenewalResultTag` 是交 V0/主控登记的最小表示候选：复用原 Aeneas discriminant 宏，
给内置 `core.result.Result` 指定原 LLBC 的 Ok=0、Err=1 逻辑标签；这不是 Rust 内存/niche 布局结论。
每次抽取核对原枚举名、两种标签及各一 payload，并绑定固定 Rust std、原宏和内置 Result 源。
`RenewalResultBindings` 证明实际抽取的泛型 `Result::is_err`，不把整个 Result API 或 Serde 设为可信。
仅在隔离抽取视图的手写 FunsExternal 前导入候选，原四个逐符号模型保持原文；生成体不变。
候选经本地形式检查也不表示 V0 已登记或整个业务链已验收。

`inventory.json` 登记精确源、审计和四个真实变异：只比 key、确认忽略有效截止时间、
extend_only 允许缩短、允许首版本。每个错误版必须实际编译/抽取并到达反例；失败编译不能算拒绝。
历史终局证明保留，新增声明不会冒充旧根验证；公共根、冻结、覆盖与原生登记由主控合并。

开发入口先取得共享重锁，使用本任务固定依赖缓存；每个版本独立 `.lake/build` 和抽取 target：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/renewal-confirmation/formal.py \
  --packages /private/task/.lake/packages --output /private/renewal-formal
```

它调用原正式检查器与同服务客户端，检查实际完整导入闭包、内核、完整非 stale/partial 诊断和准确依赖。
`--case` 只供诊断，只有全部 case 通过才报告本地全组完成；不签发完整证明验收。
输入、实际生成文件、表示候选、合同、见证与失败日志均保留并绑定摘要。

ning 对同提交完整归档执行普通/原生验收，原归档工具见终局组件；新增 Rust 测试仍在该组件原 crate：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/renewal-confirmation/native.py \
  --source-archive /private/source.tar --target /private/renewal-native-target --output /private/renewal-native
```

原生驱动核对正常七项断言及四组 baseline→mutant→restored，严格核对真实库/测试工件、fresh=false、
实际断言失败、全部源恢复及格式/Clippy。此结果不是共享 seal；`verify-stage1-protocol.sh`、
`verify-controller.sh` 和共享 proof_native_suite 的同批登记与内容复用由主控统一完成。
JSON 字节/schema、认证连接、耐久确认/退休、持久收件、重启/乱序/竞争与完整系统结论仍另行验收。
