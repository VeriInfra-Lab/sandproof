# 共享查询与成员判断的复现说明

本目录只交付 `Store::get / contains_key` 这一批。设计和边界的权威位置仍是 [V0](../../../docs/formal/00-foundations.md)、[依赖登记](../../../docs/formal/dependencies.md)和[覆盖记录](../../../docs/proof-coverage.md)；执行状态看 [#333](https://github.com/VeriInfra-Lab/sandproof/issues/333) 及本批 PR。

## 实际验证什么

[验证根](../../extract-loop-callbacks/src/lib.rs)直接调用原 [Store](../../../controller-core/src/store.rs)；没有复制查询算法。固定原 Charon/Aeneas 生成独立的 `SandproofLoopCallbacks` 库。

- `QueryContract`：在原有序行中找第一个相等的键，返回该行原值的表示；找不到返回 None。允许重复键。
- `MembershipContract`：成员判断等于同一次原查询结果是否为 Some。
- [Loop](../../../SandboxModel/Foundations/LoopCallbacks/Loop.lean)证明新生成的共享查询循环体与已证明的 get_copy 循环体一致，再复用已有有限循环定理；没有另建 Store 或独立 Rust 解释器。
- [Refinement](../../../SandboxModel/Foundations/LoopCallbacks/Refinement.lean)连接原函数、四个委托根、空表、首行命中、缺失、实际错误传播，以及键和值的表示。
- [Examples](../../../SandboxModel/Foundations/LoopCallbacks/Examples.lean)提供合法非空、重复键首匹配和缺失见证。
- [Audit](../../../SandboxModel/Foundations/LoopCallbacks/Audit.lean)登记正式库的全部目标；正确/错误实现共用前两项独立合同。

已有 Access/Lookup 对 network 根的 get 连接继续复用。本批新增的是可独立导入的共享查询/contains_key 接法和验收包，不把已有首次查询理论重新算作新成果。

## 使用前要提供什么

| 条件 | 提供方及含义 |
| --- | --- |
| 原有限表和查询键 | 调用者选对原对象，保留原顺序与所有身份字段；不预先假定键唯一 |
| 实际比较实例 | 调用者证明 `TableLawfulEq`；仅有 Rust 的 PartialEq 名称不够。有隐藏状态或外部效果的比较器不属于本批纯值域 |
| 有效共享借用 | 采用原 Aeneas 对安全 Rust 借用的表示；V 不需要 Copy。本批结论是值观察，不是物理地址或可变别名堆快照 |
| 键和值的模型投影 | 使用 represented_query/membership 时，调用者给出完整行表示和 keyView 的注入性；不能丢掉部署/代际等身份后仍声称相等 |
| 固定工具和标准库 | 采用既有原翻译器可信链与 A6 的 Vec、标量、比较和 Option 模型。本批没有新增 B 类可信适配 |
| 业务决定 | 查到一行不表示已授权、新鲜、可释放或持久成功。恢复、授权、提交和释放仍归实际业务模块 |

对本生成库的 Store 可以直接使用这些定理。别的生成根可能有不同的 Lean 类型，须先建立保留原 entries 的连接；不能仅按 Rust 类型名认为 Lean 类型相同。原 Access/Lookup 的桥接可以作参考。

这些同步入口没有 await、后台交接或外部提交；本批不增加异步取消/崩溃保证。准入拒绝新增的未登记 Drop/错误节点。进程中止不能被补成正常返回；表中值的资源生命周期不由查询成功提供。

[source-map.json](source-map.json)只登记本批两个入口，以及 World 查询、checkpoint 去重和迟到回执三个代表邻域。它不是完整调用图，也没有证明这三个调用者的业务。已有 put/get_copy/retain 理论不重写；有状态 FnMut 的捕获状态和调用顺序留到首批交接后的下一批。

## 正确版和错误版如何区分

原生组件在私有导出副本追加同一测试模块，正确版覆盖空/非空、非 Copy 值、重复键首匹配、缺失、整数边界、原共享引用和原行序。错误版必须先新编译，才检查同一断言失败：

1. skip-first：只将原 get 的起点 0 改为 1。
2. invert-presence：只将原 contains_key 的 is_some 改为 is_none。

形式组件重新从确定提交导出完整源码，对三版分别运行原工具。对错误版导入同一 Loop 定理，检查 [SkipFirst](SkipFirst.lean) 和 [InvertPresence](InvertPresence.lean) 对同一 QueryContract/MembershipContract 的否定。编译/抽取失败不算反例；错误输入、源码修改范围、生成库、完整 LLBC 和审计目标都需与冻结记录相符。

原源码的 `src/artifacts` 是生产模块，必须保留。不能使用递归忽略所有 artifacts 的复制方式。可重放驱动采用 git archive 导出完整提交，包含该目录。

## 分机执行

先提交本批全部文件和冻结清单。普通 Rust/Python、格式、Clippy、组件及原生错误断言只在原生测试机；编译/MIR、Charon/Aeneas、Lean、形式反例和审计只在证明机。使用已经配置的固定环境，不安装第二套。

原生组件入口：

```sh
bash scripts/python-entry.sh tools/check_loop_callbacks_contract.py
bash scripts/python-entry.sh -m unittest tools.tests.test_loop_callbacks_contract -v
bash scripts/python-entry.sh tools/check_loop_callbacks_components.py --output "$NATIVE_OUTPUT"
```

原生组件使用已协调的稳定 CARGO_TARGET_DIR；原始测试二进制留在原生测试机。完整批次由统筹将该组件注册为原生 case `loop-callbacks`，通过既有封口/摘要规则交给证明机。

证明组件可单独复现：

```sh
# 在数据盘的本批私有工作目录内设置 TMPDIR、CARGO_TARGET_DIR；
# 原工具和固定包缓存只读复用，.lake/build 由各私有导出独立持有。
bash scripts/python-entry.sh verification/experiments/loop-callbacks/replay.py \
  --tool-root "$SANDPROOF_PROOF_TOOL_ROOT" --output "$FORMAL_OUTPUT"
```

上述命令只交付形式组件结果。最终模块入口为：

```sh
export SANDPROOF_NATIVE_EVIDENCE=/path/to/sealed-native-evidence
bash scripts/verify-loop-callbacks.sh
```

最终入口首先使用原 acceptance_receipts.native_case 检查封口的 loop-callbacks 记录。公共 case 未注册、未运行、源码/规则变化或证据缺失时应拒绝，不能临时跳过。再运行本批形式重放。完整 V0/保留集合及 CI 仍由统筹验收，局部绿灯不能替代。

原服务使用未修改的 tools/lean_session.py。实际文件诊断必须完整、非陈旧、无错误/警告，且审计名称恰好覆盖冻结目标。原工具实际重新生成后比较全部 Lean/JSON 和完整 LLBC；仅复用既有 short_names 无序表的规范化，保留其余字段。

## 统筹需要接入的公共位置

本线不直接修改下列共享文件，具体补丁由统筹在交接提交上实施：

| 位置 | 应接入内容 |
| --- | --- |
| 主 lakefile.toml | 注册 SandproofLoopCallbacks，srcDir 为 verification/aeneas/loop-callbacks；同步独立 Lake 视图，避免重复注册 |
| SandboxModel.lean / 总审计 | 导入本线公共根，并包含 LoopCallbacks.Audit 的全部目标；保留所有原目标 |
| tools/proof_native_suite.py | 在组件列表新增 loop-callbacks，调用 check_loop_callbacks_components.py；CLI require 如需要也增加同名项 |
| tools/proof_batch.py | 新增 verify-loop-callbacks.sh 模块及完整输入闭包；原生部分仍只消费封口记录 |
| tools/proof_services.py | 增加实际 LoopCallbacks/Audit.lean 文件诊断，不移除旧文件 |
| use-inventory 主登记 | 增加本批真实根、库实例、普通值/共享借用及无新增析构的准入来源；不声称完整调用图 |
| 公共冻结清单/依赖键 | 纳入本线新文件、父输入和验收脚本；准确列 RUN/REUSE |
| 权威设计/依赖/覆盖和 #333 | 更新 T3/T4 对应子项、原工具/值借用条件、真实核心与调用者责任、正反例及仍未完成的 FnMut/使用点 |

当前环境的父输入闭包取保守范围，相关公共输入变化可能触发重跑；不为提高缓存命中删去依赖。所有源/依赖/工具/配置/规则和原始证据一致后才复用。
