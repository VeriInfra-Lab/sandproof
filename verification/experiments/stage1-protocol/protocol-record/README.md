# Host24 同步调用方与 Destroy 候选

[组件入口](../README.md) · [范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [任务 #635](https://github.com/VeriInfra-Lab/sandproof/issues/635)

本候选固定基线为 `f3627b7bb9995251abe31e2bd2bf6252daa1f2c8`，按
[#635 的接线审查](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-6489420997)
采用最小同步拆分。`record_lifecycle_intent` 在原 WorldTransaction 内调用唯一的
`record_using`；Replay 仍提前返回，新操作仍按原故障点和提交顺序处理。Destroy
保留释放中拒绝、原 `terminate_local` 及随后查询的当前操作号。

公开的旧三文件提案漏记了 `codec::StandardInboxSave` 及其 `connection` 字段的
两行 `pub(super)` 可见性。原已编译的私有视图包含这两行，本候选将其一并纳入，
复用原标准保存实例的方法体。该修正没有更换 SQL、JSON、SHA 或保存算法；旧提案
及其证据保留原版本，不称三文件提案本身已经编译通过。

17 份调用方模板中16份保持原字节，正常读取fixture只修正两处23→24的手写证明
字面值；`FullFlowContract` 保持原字节。它们复用完整 load、独立认证、全文查找、
record、save 和 state 规格。`roots.rs.in` 只透明调用实际通用入口。
`candidate-inputs.json` 是候选记录，既不是正式冻结，也不是完整验收回执。

相邻 `inbox-load` 的五份模板仅将实际格式常量 23 改为 24；完整合同结构和首错顺序
保持原样。新增格式见证保留 helper 的准确边界：旧版本且收件表、SourceWork 表均
缺席时返回 None；不能改称所有旧版本均拒绝。真实错误版仅将已有收件表分支的
版本守卫留在 23，结论仍为同一 `FullLoadContract` 的否定。

旧 Host23 调用方开发证明保持原适用范围。当前 Host24 调用方及三个真实错误版已在
固定原工具下重新编译/抽取，经原严格真实文件检查器完成开发检查，见输入记录和
[#635](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-6491475311)；
标准实例的方法转发审查也不等于 Serde、SHA、SQL 或事务已机器证明。普通、原生及
错误版断言交 ning；公共 Lake/总根、登记、输入清单、冻结、覆盖及正式封口由验收
窗口统一维护。当前登记尚未更新，运行旧根不能报告本目录目标通过。

[合同与表示关系](contracts.md)列出主要定理、完整状态、效果前后态和具体提供者边界。
`native_common.rs.in` 是交ning的三条真实ControllerJournal断言候选，尚未原生编译/执行；
它补充现有套件，不替换原断言。开发归档与各结果摘要见 `candidate-inputs.json`，
这些记录不充当原生或正式封口。

## 模块入口

`prepare.py` 按原次序追加九份薄根及 renewal 根，只在三个错误视图的真实
`recording.rs` 中替换唯一表达式。`mutations.json` 的失败集合是待 ning 实测的
期望，不是已观察结果。`inventory.json` 来自四份已完整检查的有限文件闭包，
保留正确 200/799、删除 context 192/755、Replay 194/764、旧操作 193/782；
`providers` 给出每个手写输入的原提供文件及摘要，`views` 给出逐文件审计。
该文件是模块内候选清单；公共登记与最终冻结由验收窗口采用，不能直接更新摘要
来背书改动后的业务源码。

在按证明工作流准备好固定 `.tools`、`.lake/packages` 的证明工作区运行：

```sh
python3 verification/experiments/stage1-protocol/protocol-record/formal.py \
  --output artifacts/protocol-record-correct --variant correct --require-native
python3 verification/experiments/stage1-protocol/protocol-record/formal.py \
  --output artifacts/protocol-record-context --variant skip-independent-context --require-native
python3 verification/experiments/stage1-protocol/protocol-record/formal.py \
  --output artifacts/protocol-record-replay --variant replay-falls-through --require-native
python3 verification/experiments/stage1-protocol/protocol-record/formal.py \
  --output artifacts/protocol-record-operation --variant stale-local-operation --require-native
```

四个视图全部重新编译真实 Rust、用原参数抽取并核对透明/外部符号，调用原
`library_binding`，只在独立输出目录注册 Graphic，使用原真实文件、服务和内核
审计检查器。Charon 仍为 Aeneas preset、错误/警告拒绝、原 include/opaque 列表，
原固定虚拟路径、目标和离线锁定 Cargo 输入；Aeneas 仍为 Lean 后端、split-files、
emit-json、警告拒绝。原始 LLBC、translation、未修改生成体、命令/内核/服务日志
及结果均保留在指定输出中。`--prepare-only` 只准备抽取和证明文件；它与
`--require-native` 同用会拒绝，不能成为正式通过。

普通/原生在 ning 使用以下入口；这只执行新增三条断言的正确、三个错误和恢复视图：

```sh
python3 verification/experiments/stage1-protocol/protocol-record/native.py \
  --output artifacts/protocol-record-native
```

每个变体使用独立源码和 target；检查实际 protocol 库、service 测试的
package/manifest/src/profile/fresh 和产物摘要。编译/格式/Clippy 失败不计性质拒绝；
必须到达同一三条完整断言并精确核对退出码、失败集合和通过/失败数量。
所有原组件断言保留在源码中，原 19/12 检查及共享套件继续由总入口运行/按内容复用，
本模块不再执行一遍。验收窗口需将 `stage1-protocol-record` 原生 case 及四个形式
variant 接入公共登记；`--require-native` 会按原回执验证器要求完整 V3 范围、
当前 functional 凭据、本 case 全部原始日志及五个变体。缺登记或凭据必须拒绝。
`--development-native` 仅用于独立开发，不能与 `--require-native` 混用。

Host24 load 的三步补充准备入口为 `load_reproduction.py`：

```sh
# 在原 f3627b7b 隔离 checkout 中，先沿原脚本完成 prepare-02。
python3 /path/to/candidate/verification/experiments/stage1-protocol/protocol-record/load_reproduction.py port \
  --project artifacts/current-host24-load-01/source \
  --preparation artifacts/current-host24-load-01/proof-preparation-02.json \
  --output artifacts/current-host24-load-01/proof-preparation-03.json
# 原正确检查完成后，准备原真实错误版，再运行原 extract-host24-wrong-format-01.py。
python3 /path/to/candidate/verification/experiments/stage1-protocol/protocol-record/load_reproduction.py mutate \
  --repository . --correct artifacts/current-host24-load-01 \
  --project artifacts/host24-wrong-present-format-01/source \
  --output artifacts/host24-wrong-present-format-01/preparation.json
# 原 prepare-host24-wrong-proofs-01.py 完成后，安装已检查的最终反例。
python3 /path/to/candidate/verification/experiments/stage1-protocol/protocol-record/load_reproduction.py install-counter \
  --project artifacts/host24-wrong-present-format-01/source \
  --output artifacts/host24-wrong-present-format-01/actual-files-01/inventory.json
```

这些补充步骤校验原输入、27个版本字面值、唯一 Rust 守卫变异、同一 Host24
模型与 108/424 闭包；不会写入“新检查通过”，也不要求重跑已经读回的数学证据。
原 extract/build/check/prepare-01/02 命令仍在已交原始包中。所有输出须使用新目录，
不能覆盖历史准备、失败或结果。

V7 提供独立认证来源，V1/V2 提供当前意图及耐久发送；V4/V5/V6 的本地接受、未知
提交恢复、材料引用、真实文件、VM 及物理终止事实仍按具名义务交付。manifest现版
准入和真实材料、
JSON/TLS 字节链、持久收件与完整 V3 继续开放。
