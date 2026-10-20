# BTreeSet 正常值连接

本批只连接固定标准库的 `new`、`insert`，以及这两个操作实际需要的比较字典。
Rust 继续调用真实 `std::collections::BTreeSet`；Lean 中的列表仅表示集合有哪些值，
不能当作真实树的内部布局或迭代顺序。

预检来源为 integration `a12f4e16`，原探针与原始证据见
[上一阶段](../README.md)。本批源码在
[隔离 crate](../../../extract-btree-set-binding/src/lib.rs)，
[普通用例](../../../extract-btree-set-binding/tests/insert_contract.rs)。

## 使用范围

| 项目 | 具体范围 |
| --- | --- |
| 分配器 | 固定 Global，不覆盖自定义分配器或可观察的分配器 Clone/Drop |
| 实际键 | `&str`、`(&str,u16)`；u64 只作对照 |
| 输入 | 有效有限集合、合法并在借用期稳定的 UTF-8 键；真实 Rust 表示和分配必须可用 |
| 正常返回 | 新集合包含原成员与原键；标志当且仅当此前没有等价键 |
| 重复项 | 返回 false；保持原集合的值视图，不用新键替换原视图 |
| 其它成员 | 值和成员关系保持；不观察物理地址或共享别名的可变堆 |
| 比较 | 固定字符串字节顺序、引用转发和先节点后 u16 的元组比较 |
| 异常 | 不保证 panic/OOM/abort 回滚、物理释放、取消收尾或无条件终止 |
| 析构 | 只适用于普通内存释放可忽略的这些键与 Global，不推广到持许可、锁、FD 或自定义 Drop |
| 未覆盖 | contains/len/collect/迭代/排序，以及完整 valid_tenants、valid_nodes、valid_lanes |

标准库模型与固定 Rust Std 的一致性继续属于既有 A6 条件。
这里没有证明标准库树算法或编译器本身正确，也没有增加新的 Rust 翻译规则。
正常值合同不能直接保证分配成功或真实程序一定返回。

## 模块分工

- `Contracts.lean`：比较与相等的条件、插入后应满足的成员/标志性质；正确版和错误版引用同一个 InsertContract。
- `Theory.lean`：字节比较、固定字符串/引用/元组/U16 字典，以及成员保持、重复幂等和普通值视图的推导。
- `Refinement.lean`：引用未经编辑的原生成 Rust 委托，连接字符串和节点/slot 两种真实键型；证明连续两次插入使用同一个回写后集合。
- `Examples.lean`：已有元素时接受新值、原重复项保持及不同 slot 不混同的非空见证。
- 本模块自己的审计与错误版证明文件，须在获得具体文件写入批准后固化；不修改公共根、总审计或共享 Lake。

下游仍须证明自己的 ID 选择、哪些集合应唯一、配置是否合法、循环何时早停，
以及外部捕获状态和业务拒绝次序。三个实际 validator 的类型来源登记不是完整函数证明。

## 可复现入口

普通检查在原生测试机，形式化在固定 Linux 证明环境。
每项使用独立源码、target、.lake/build、TMPDIR 和证据目录；重任务使用既有协调锁，
等待原生机时不占证明机重锁。证明机新增数据放数据盘的本任务私有目录。

```sh
# 在已提交的本批源码上，普通原生组件：
bash scripts/python-entry.sh tools/check_btree_set_binding_components.py \
  --output "$NATIVE_OUTPUT"

# 下列正式形式入口需要完整 environment.json/inputs.sha256 和两个专属证明文件；
# 这些前提未齐时应拒绝，不能改成跳过。
bash scripts/python-entry.sh verification/experiments/btree-set-binding/model/replay.py \
  --tool-root "$FIXED_TOOLS" --output "$FORMAL_OUTPUT"
```

独立形式入口逐次重做真实 Rust 编译与 Charon/Aeneas，保留全部 LLBC 字段，
仅沿用既有 short_names 无序表规范化。实际生成 Lean/JSON 逐字节核对，不手改生成物；
未实现的原工具模板移到忽略证据目录，不导入。随后运行 Lean 内核与原服务的实际文件诊断，
逐个核对准确审计目标，并原样转发已经验证的审计消息。

错误版只在隔离副本中把原 `insert_str` 的 `seen.insert(value)` 改成
`!seen.insert(value)`。它须先正常编译和抽取，再被正确版相同 InsertContract
拒绝；原生侧仍使用正确版相同的首次插入断言。编译错误、超时和服务异常不是性质拒绝。

完整 V0 的共享登记、原生 case、公共根/总审计、完整入口、CI 和正式合入由统筹完成。
单独组件成功不能称为整套验收或 V0 完成。

## 当前证据适用性

- 换行前源码的形式组件已完成：22 个准确正例/辅助审计、九份实际文件，以及实际翻转标志错误版的两个审计；它们不代表 22 个业务函数。
- 首轮原生六用例通过，随后固定格式检查发现函数签名应折为一行，入口退出；旧日志保留。
- 原生测试机固定格式工具的输出已形成 `9cea1728`；六项正确用例、真实错误版相同断言、fmt 和严格 Clippy 全部通过。两个版本的实际库和测试程序都重新编译，并核对原 manifest/src/target。
- 换行改变源码摘要和位置；新摘要尚须重新抽取对齐，不能直接沿用换行前形式结论。
- 两份专属审计/错误版文件的直接用户批准仍待答，尚未写入。完整冻结、最终形式重放、PR、CI 和共享集成尚未完成。
