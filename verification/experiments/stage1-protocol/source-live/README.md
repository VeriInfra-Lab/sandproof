# 卸载来源的存活检查

[文档目录](../../../../docs/README.md) · [协议范围](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包对应固定公开基线 `2be04389e45482cb056c54e716dd097456d6168a` 的
`controller-service/src/controller/lifecycle_inbox/source_work/snapshot.rs::live_native`。
它从完整已解码值开始，使用原 SourceObservationGuard::allows 和启动时间闭包。
生产文件和 V4 实现只读；Create/Replay 的真实事务片由原独占窗口负责。
实际运行结果、失败原件、原生和公共采用状态在 #635 和 PR 维护。

## 完整合同与接受域

FullLiveContract 对任意完整 NativeCreateSnapshot、InternalLocalBinding、
accepted/current version 和 now 给出原 bool 的完整结果等式。正确、三个错误版
使用同一 Spec、Values、Examples 和合同，分别在独立生成/构建环境检查。

接受恰好要求 lifetime/current_vm 存在，accepted 非零且等于 current，owner 一致，
generation/epoch 非零，Running 且非 terminal，now 非负且严格小于 deadline，
start 存在且不晚于 now 和 startup deadline。在这个实际调用点，VM、generation、
epoch 的双侧比较使用同值。正常合法见证、接受/拒绝等价、总性和字段保持分别陈述。

不增加 owner/VM 非零、start 非负、max deadline 或 disk 条件。local 的
sandbox/operation、native 的 current_operation/uncertain_actions 未在本谓词检查；
相应字段的无影响性质明确保留。谓词不自证认证、持久接受、原请求/本地对象来源或
物理资源存在；外层 current、同事务与历史、V4/V5/V6/V7 提供条件继续开放。

## 精确实现与库边界

生成目录中的 Types/Funs 是固定 Charon/Aeneas 的原输出，逐字节核对，不编辑生成体。
生成的外部模板保留在抽取原件中，不导入其 axiom。FunsExternal.lean.in 仅把
Option::is_some_and 委托给依赖登记中的既有 A6 模型，实际业务闭包仍透明。
没有新的业务公理、整体 Std/handler/事务假设或替代算法。

三个真实 Rust 变异只修改原 live_native：忽略 accepted 版本、将实际 deadline
替换为 i64::MAX、遗漏 startup 比较。每个生成体都有具体 false positive 及
否定同一 FullLiveContract 的证明；编译、翻译、导入或超时失败不算性质反例。

## 独立检查

源准入和项目准备不编译 Rust，也不签发原生/形式验收：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/source-live/capture.py --check-only --output artifacts/source-live-inputs
bash scripts/python-entry.sh verification/experiments/stage1-protocol/source-live/native.py --check-only --output artifacts/source-live-native-inputs
bash scripts/python-entry.sh verification/experiments/stage1-protocol/source-live/prepare.py --output artifacts/source-live-projects
```

证明环境使用既有固定工具、包目录、私有 runtime 配置、最多四 CPU 和同一共享重锁：

```sh
SANDPROOF_ACCEPTANCE_ROLE=formal bash scripts/python-entry.sh verification/experiments/stage1-protocol/source-live/formal.py --output artifacts/source-live-proof --lock /private/existing-proof-heavy.lock
```

组件入口实际重抽四视图并与登记生成体比较，各自独立 Lake 构建，通过规定的同服务
客户端检查每份真实 Lean 文件、导入依赖、全部审计及源码/模式/工具/库后置条件。
错误视图不导入正确 Core/Witnesses。保留原64GiB余量下限及四视图16GiB写入预算；
不删除缓存或 .lake。继承锁 FD 必须对应同一锁文件，不争另一全局锁。

ning 运行原版、三错和实际恢复的五个新 target，同一11项断言，核对真实 ELF、
程序/库、断言位置、完整测试清单、严格 Clippy 和来源后置条件：

```sh
SANDPROOF_ACCEPTANCE_ROLE=native bash scripts/python-entry.sh verification/experiments/stage1-protocol/source-live/native.py --output artifacts/source-live-native
```

组件不运行共享完整 functional，也不签署 ning 或公共父范围收据。公共 Lake/根导入、
native case、正式 profile、根冻结和覆盖记录由主控统一采用；登记建议见
[候选登记](registration-candidate.json)。verify-stage1-protocol.sh 未因本包源码存在
就自动覆盖它，不能跑旧根后报告本包通过。整个 V3 的原18要求/49义务及五组剩余
业务连接保持开放。
