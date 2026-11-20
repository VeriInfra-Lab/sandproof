# Unload 两处 JSON 调用的源码准入候选

[组件入口](../README.md) · [既有 Serde 接线](../serde-binding/README.md) ·
[原选项证明](../unload-stored-options/README.md) ·
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md)

本包固定 integration `b60078c288a86a99219a4cd67bf2797dc36997ab`，为 #635
补 `snapshot::encode_source_options`、`snapshot::encode_source_anchor` 和原
`host_contract::serialize_create_options` 的实际调用证据。生产 Rust、公共登记、
原冻结与 #768/#770/#774/#779/#803 的生成体和定理不变。

**这是源码准入与原生回归候选，新增 Lean 定理为零。** 原固定 Aeneas 对前两个
透明根的预检在导出相互递归的 Serde trait 声明时失败。完整失败保留；本包只用原
Charon 捕获实际调用及两个派生，不把部分生成文件或源码检查充作机器证明，不替换
工具、手改生成体或实现另一套 JSON 算法。

## 核对范围

三个包装各把同一个完整借用值交给一次原 `serde_json::to_vec`，直接返回原结果；
源检查拒绝附加调用、守卫、对象替换、字典替换、结果改写及改变的 unwind 路径。
两处 CreateOptions 调用必须使用同一个实际派生字典，三个调用使用同一原函数。

`source_binding.py` 复用既有 Serde 来源读取器及有序结构体检查器。SourceAnchor 的
12 个实际声明字段、完整类型、原字段引用、原容器以及逐字段首错路径全部核对。
CreateOptions 的四个实际声明字段和原引用/类型/字典均核对；networking/template 的
条件分支及全部原派生体另用完整摘要固定，**不声称检查器解释或机器证明了这些条件**。
固定库源、特性和两版标准库来源沿原 `serde-binding/library.py` 核对。完整原 LLBC
独立保留；摘要视图不替代它，也不删除中间字段。

`profile.json` 保存实际重抽后的完整调用/派生快照及全部生产 Rust/Cargo 输入，
不是对旧摘要换签。输入变化须重新审查和抽取，不提供自动刷新命令。
两个真实 Rust 变体分别把 execution_origin 换成 request、清空 metadata；它们只在
隔离副本应用，使用相同完整值合同。编译/抽取失败、超时或导入失败均不算性质拒绝。
源码检查拒绝和原生断言失败分别记录，不列作 Lean 形式反例。

## 执行

沿已有固定工具和共享重锁，输出必须是各自不存在的私有目录：

```sh
package=verification/experiments/stage1-protocol/unload-json-calls
: "${PROOF_HEAVY_LOCK:?使用既有共享重锁}"
for variant in correct origin-request options-empty; do
  bash scripts/python-entry.sh "$package/admit.py" \
    --output "artifacts/json-calls-$variant" --lock "$PROOF_HEAVY_LOCK" --variant "$variant"
done
SANDPROOF_UNLOAD_JSON_LLBC=artifacts/json-calls-correct/actual.llbc \
  bash scripts/python-entry.sh -m unittest discover -s "$package" -p 'test_*.py'
```

ning 的原生入口；其它机器只能报告开发检查，不能签发正式验收：

```sh
bash scripts/python-entry.sh "$package/native.py" \
  --output artifacts/json-calls-native --lock "$PROOF_HEAVY_LOCK"
```

它在四个独立 source/target 中实际编译原业务 libtest，运行同一组 anchor/options
全文往返、全部顶层字段逐项变动和可选字段有/无用例。原始 anchor 来自现有真实
SQLite/受控 Host 夹具；改变后的普通值只用于编码合同，不代表业务接受或真实资源。
测试桥只附加到隔离副本，生产可见性不变。错误版必须到达相同完整值相等断言，恢复版
重新编译并通过。所有日志、实际二进制及变体源分别绑定，不从旧 target 复用业务程序。

## 尚未完成

本候选支持审查已有 A6/B4 的具体使用点，**不自行采用新可信条件**。
是否将这两个调用及嵌套派生接入原 ParentCalls，须由主控核对依赖登记及实际完整
表示关系；这里没有新增或填入一个假定成功的 Operations 实例。现有同父调用证明
中的 JSON/哈希等条件仍保持，不能因三个调用名字或类型相同便称连接已经证明。

嵌套业务字段的对应、具体 Serde 库精化、完整 JSON 字节链、真实哈希计算、同事务
插入/commit/未知恢复、认证来源、耐久收件、实际 OS/VM/文件及完整 V3 仍未完成。
内容摘要的值往返不表示材料真实完整。共享 `verify-stage1-protocol.sh`、
`proof_native_suite`、`verify-controller.sh` 和覆盖记录由主控同批接入、去重验收；
本包独立入口成功不意味着旧公共入口已经覆盖这些新增文件。
