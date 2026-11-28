# 首次卸载的原启动回执

本包对应 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635) 的首次卸载成功路径，从 integration 的 `9b82954c68c85bb9c95831865fed7a278d455cb5` 开始，复用 #768 的存活检查与 #770 的来源行、SQLite 和机械迭代结果。实际适用范围见[覆盖记录](../../../../docs/proof-coverage.md)。

## 性质

`actual_first_work_uses_sqlite_startup` 在 `actual_first_work_uses_bound_startup` 上装入 #770 的 SQLite 操作和实际 `startup_records`。在原 Create 计划及成功读取条件下，扫描的第二槽来自同一事务查询的实际行。它的动作是该计划的完整 `PrepareVm`，通过动作资源、回执和 HostEvidence 检查。父 `current` 成功结果在同一位置保留动作标识和该行原始回执字节的 SHA-256，定理另保留原字节见证。缺槽或中途读行出错仍阻止成功。

- SQL 继续使用原事务、一个 sandbox 文本参数、原排序及两个 BLOB 列。SQLite 的物理读快照、绑定、行、首错语义遵守 A4。
- Serde 接收实际动作和回执的完整字节，保留正常解码及原错误。同一个 `E` 的真实 HostEvidence、ResourceEvidence 和 Serde 字典语义是原 A6/Host 接口条件。
- 扫描顺序和首错规则沿用 #770 的 B4 机械驱动。新 Rust 驱动及父 `current → startup_records::<E>` 对应通过源码、LLBC 和生成调用逐项核对。Lean 检查实际抽取后的处理函数、扫描输出、装配和父定理。
- 调用方仍负责同一 World、原会话与本地绑定、原记录和封存计划、`base_snapshot` 的同源对应及 `FirstRead` 其余观察。结论限首次卸载的来源观察要求。持久登记、失败卸载后的继续或恢复、实际暂停和回收分别验收。

## 复现与适用版本

`proof-inputs` 包含当前切片的原抽取输入视图和 Lean 证明。组合视图只隔离已核对无消费者的同名辅助判别实例。父和收集器视图隔离六项，#770 的 SQLite 类型视图隔离十三项。各生成函数、SQL 字节及构造子保持。已有 #770 驱动、标准摘要与 Foundations 直接从仓库复用。

在原 msra 固定工具环境及正式锁下，先核对仓库根的 `verification/v3-unload-startup-inputs.sha256`。用 `python3 verification/experiments/stage1-protocol/unload-startup-source/materialize.py <新目录>` 生成独立 Lean 工程。工程使用仓库原 `lean-toolchain`、`lake-manifest.json` 和同一 Aeneas。已有对应对象时可在原搜索路径下只检查新增模块，冷环境可在物化目录运行 `lake build`。原件摘要及运行范围见 `evidence.json`。实际工具根、抽取选择与命令参数见 `capture-profile.json`；路径参数由固定工具环境提供。

`native-tests.rs.in` 是已在 ning 使用的定向夹具，单独追加到 `controller-service/src/controller/lifecycle_inbox/source_work/tests.rs` 的隔离副本，使用原 Cargo 锁、原依赖和精确测试名 `controller::lifecycle_inbox::source_work::tests::proof_source_flow_original_request_reaches_same_source_question`。该夹具从原 World/plan 和独立 SQL 查询获得期望的 `PrepareVm` 和原 BLOB，再检查真实输出。#770 仍使用自己的原夹具，两份同名测试分别装入各自隔离副本。仓库原测试文件保持，追加后的完整字节已在 ning 实测。
