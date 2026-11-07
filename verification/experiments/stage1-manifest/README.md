# 当前清单局部证明

[证明覆盖](../../../docs/proof-coverage.md) · [证明工作流](../../../docs/proof-workflow.md)

本包从 #671/#675 选择性迁移 manifest 长度、溢出和文件集合证明，基于 #683 的
`236d40db71fdf06e5ffce3968cabf69eb5244325`。实际范围及未解义务只在覆盖记录维护。
新证明位于 `SandboxModel/Lifecycle/Manifest/`，共用夹具使用独立 `ManifestChecks` 命名空间，
复用当前 `LoopWitness` 与原 Foundations 值/字符串对应，不导入旧 request 证明堆叠。

## 形式入口

在已配置固定工具与 Lake 包的证明机运行，输出目录必须不存在：

```sh
python3 verification/experiments/stage1-manifest/formal.py \
  --output artifacts/manifest-local-formal
```

入口固定正确版及四个真实 Rust 单表达式错误版：wrapping 加法、跳过 drive ID、跳过路径、
跳过文件数量检查。各副本有独立源码、Cargo target 和 `.lake/build`。原
`stage1-protocol/extract.py` 检查工具摘要、重新编译两个自有包并重新生成完整 LLBC/Lean；
不手工改生成体、不移除 IR 字段。错误版能抽取/编译不等于反例成功，必须在同一独立合同下
证明其实际错误接受，再否定合同。

当前生产协议为 4，原保留生成物仍为 3。夹具采用本次生成的协议常量，入口另检查其为 4；
新生成物只安装到私有副本，记录与历史产物的摘要差异。它没有覆盖公共生成物、刷新旧冻结、
更换 Accepted/Recorded 入口或登记完整批次通过。公共登记仍由统一集成协调。

实际本地导入闭包使用原 `proof_batch.closure` 解析，再调用原 `formal.actual_files`：
每个实际文件经内核、完整非陈旧标准服务诊断及准确声明清单审计。`inventory.json` 独立列出
所需审计；任何漏项、额外项或输入变化均拒绝。两模型在全部副本逐字相同。漏文件错误版
实际生成两个循环 helper，反例使用数量不匹配分支调用的 `validate_loop0`，不修改生成体。

结果保留全部输入/生成物/命令和服务摘要，明确 `native_checks=not-run`、
`full_acceptance=false`。它不能代替原 `verify-stage1-protocol.sh`、共享原生封口或
`scripts/verify-controller.sh`；整套入口仍须运行并报告实际结果。

## 原生检查交接

在同一确定提交的普通测试机运行原基础检查及独立 manifest 回归；本脚本不安装测试环境。

```sh
cargo test -p sandproof-protocol --test manifest_checks --locked --offline
git archive HEAD > /private/acceptance/manifest-source.tar
python3 verification/experiments/stage1-manifest/native.py \
  --source-archive /private/acceptance/manifest-source.tar \
  --output /private/acceptance/manifest-negatives \
  --target /private/acceptance/manifest-target
```

原生适配只选择 `mutations.json`，执行原 `negative_native.run` 的完整算法：正确版同断言通过，
实际错误版的业务库和测试程序重新编译、同断言以预期失败拒绝，恢复正确源码后重新编译并通过。
编译失败、零测试、旧二进制和未恢复现场不计为成功。源码归档包含本包脚本、声明和断言，
结果绑定归档摘要；该独立结果不伪造共享 `complete.json`。
