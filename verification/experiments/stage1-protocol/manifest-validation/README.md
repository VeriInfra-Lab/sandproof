# Manifest 完整值校验证明

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

本切片基于 #684 的 `41c26d161afb1b32afbd2e61fe96b3d2d2c803cb`，复用其文件集合、
长度证明和四个真实 Rust 错误版。capture 补验固定 #687
`bf458df9778c6ae43356fbba60c9bf04db14f7be`，另复用原协议入口的 `skip-capture-binding`。
当前生产 `manifest.rs` 与 #635 原固定提交
`a9c9967889c4cf298e4378c7106948018835fc64` 相同；不自动跟随工程分支。

`ManifestValidationModel` 给出独立的完整接受条件和首错结果；`ManifestValidation`
连接当前协议 4 的真实生成入口。CaptureBinding 比较涵盖完整 operation、来源代和 capture；
VmCompatibility 比较涵盖格式、四个 32 字节摘要及整个有序磁盘列表。
路径与配置循环对任意表示值证明，文件循环复用原 `FileAt`、`CompleteFiles` 和自然数长度规格。
错误顺序以独立规格和当前 Rust 为准，不能将字段相等解释为授权、持久接受或实际文件存在。

输入从已解码值开始。唯一额外的表示域条件是每个 profile 路径满足现有
`IdentifierLibrary.StringRepresentable`，用于原 String→str 借用合同；不预设路径合法、磁盘互异、
profile 匹配或文件完整。字符串、Vec、Option/字典比较与受检算术沿用
[逐符号依赖登记](../../../../docs/formal/dependencies.md)。摘要值相等不表示计算过哈希。
JSON 字节接线、受认证 profile 的来源、材料真实性、持久收件和网络外壳仍是独立义务。

在固定工具和已配置依赖缓存的证明环境，持有共享证明重锁后运行；输出目录必须不存在：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/manifest_validation_formal.py \
  --output artifacts/manifest-validation-formal
bash scripts/python-entry.sh verification/experiments/stage1-protocol/test_manifest_validation_formal.py
```

入口重新编译、抽取正确版和五个原 Rust 错误版，保留完整 LLBC 与生成体；生成物只装入私有视图。
所有视图有独立源码、Cargo target 和 `.lake/build`，顺序复用本工作树的依赖缓存，标准服务
逐次核对依赖指纹。原检查器检查全部实际导入文件及准确审计清单；新增未登记模块直接失败。
`--case` 仅作开发诊断，单例通过不会标记 `all_cases_complete`。

错误版先复用 #684 的真实错误接受证明，再在同一份 `ValidationContract` 下推出矛盾；
七份模型在正反版本逐字相同。编译失败、超时、导入失败均不能作为合同拒绝。
正确版和错误版的源、生成物、实际文件、服务结果与合同摘要保存在输出目录。

capture 补验保留旧 pair 合同与实际错误接受证明，补充同一完整 `ValidationContract` 的反例。
原见证只改变 operation key.sequence，完整 capture 绑定必须拒绝；不把它误称为只改变 CaptureId。
旧 `ManifestWitnesses` 使用 #686 `39577ca1e61f4daa4ba4bbda7db94bf087bdaef0` 的唯一一行协议 4 修正，
正确版同时检查旧合法/pair 证明和已有任意输入完整证明。错误版只按实际生成的 loop0 声明调整
隔离手写 helper 的六处引用，并记录前后摘要；不修改生成体、完整模型、字段见证或工具。

此入口明确记录 `native_checks=not-run`、`full_acceptance=false`。普通和原生验收交独立测试环境，
继续使用 [原 manifest 原生入口](../../stage1-manifest/README.md) 的相同四组断言；capture 沿
[原协议负例入口](../negative_native.py) 保留全部四组原版→变异→恢复断言，同批由主控复用，不能只取 capture 一项签发共享封口。
公共协议登记、冻结、覆盖记录及共享 `proof_native_suite`/`verify-controller.sh` 由主控统一验收；
本地新入口通过不表示旧 `verify-stage1-protocol.sh` 已覆盖本切片。
