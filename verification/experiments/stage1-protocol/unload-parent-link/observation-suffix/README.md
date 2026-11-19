# 来源观察保存的记录选择与提交

[前置首错](../observation-prefix/README.md) · [完整比较](../observation-comparators/README.md) ·
[记录查找](../observation-slot/README.md) · [提交尾部](../observation-commit/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

生产输入固定于 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树为
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。本地条件证明从已解码的工作和观察值，
以及前置调用已交回的结果和连接开始。

## 对应与性质

`prepare_observation_suffix.py` 复核三份保留的固定 Charon/Aeneas 收据、原 Rust 归档、
归档中的源文件、确定的隔离改写、实际 LLBC、生成文件及原工具执行结果。它核对 #965 的
父 IR 中，只有前置成功才查找记录，且只有取中记录并判为 `Fresh` 才把原事务传给保存。
它将父生成体中从 `branch r` 到方法末尾的语句原字节放入私有 Lean 作用域，并在所绑定的
类型中保留实际的九份生成结构或枚举声明、四份生成适配器及两份生成的提交函数。适配器、
声明和语句来自实际抽取，来源文件的注释和外部模板保留在原收据。新的函数头显式接收此前
交回的结果和连接。局部符号绑定为每个尚需提供的调用给出独立参数。

- `rejected_prefix` 将原错误连同交回的连接返回。`missing_record` 在查找无结果时返回
  `Conflict`。`full_scan` 和 `selected_index` 复用完整工作相等合同，证明找到的下标有效、
  对应原完整工作且早于它的记录都不匹配。
- `stored_full_observation` 按完整旧观察与新值决定 `Fresh`、`Replay` 或 `Conflict`。
  `selected_dispatch` 证明重放返回同一条选中记录的克隆，冲突拒绝。两者均绕开保存尾部。
  首次写入仅替换选中项的观察字段，并把原 Inbox 容量、同一事务及更新的来源集合交给
  实际提交函数。`stays_outside_selected_slot` 保留其它位置的记录。
- 保存失败、提交前检查失败、提交失败各自原样返回。提交后的检查以原提交成功为前提，
  成功交回原克隆，失败保留检查错误。`fresh_commit_returns_updated_record` 在各调用返回
  成功且克隆保持原值时，从整个后续分支推出原连接写回和同一更新记录返回。

完整父方法还有一处生成签名缺口。#965 的具体 Begin 适配器交回业务结果和更新后的
`Connection`，父生成体因此解构 `(r, c)`，而泛型前置函数只声明业务结果。本文从这一
调用之后开始，完整父方法连接回写仍待关闭。#963 的原直接 Begin 生成体保留该返回对，
可用于继续核对完整父方法。#964 提供实际工作及观察全值比较，V5 提供事务、解引用、
`codec::save` 的活事务行连接及 SQLite 提交可见性，V7 提供前置认证上下文来源，V6
提供实际物理来源。全值比较和单项替换不提供授权、资源存在或持久写入的证据。

## 精确复验

`prefix_parent`、`slot_core`、`commit_tail` 分别为前置包的 `parent`、查找包的 `core`
和提交包的 `tail` 输出。它们的生成声明及调用体逐字核对。保留各包的原归档、输出、
实际成功记录和日志。`plan.json` 记录九个物理 Lean 文件的闭包和各生成块摘要。

```bash
set -euo pipefail
export LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_observation_suffix.py" \
    --parent "$prefix_parent" --scan "$slot_core" --tail "$commit_tail" \
    --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build ObservationSuffix)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 9 and (.audited | length) == 17' "$audit/result.json"
```

复验使用现有前置首错、按完整值选行、提交可见性的同规格原生正反入口。分别保留实际写锁
首错、弱绑定匹配和提前执行提交后检查的错误版记录，不将旧反例记为本候选新执行。普通及
原生正式复验由 ning 统一执行。统筹可复核 [覆盖候选](public-registration.patch)，按
九文件闭包登记私有 `ObservationSuffix` 根及审计，同批复用已有正反组件。公共 Stage1
接线和总入口等待统一验收。
