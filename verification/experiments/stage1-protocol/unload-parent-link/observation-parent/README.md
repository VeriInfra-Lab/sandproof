# 保存来源观察的父方法与连接回写

[前置首错](../observation-prefix/README.md) · [记录查找](../observation-slot/README.md) ·
[保存后段](../observation-suffix/README.md) · [逐行 SQL](../row-sql-composition/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定生产 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。复用 #963 隔离源码中的完整
`ControllerJournal::save_source_observation` 生成体。该源码只将原记录查找及保存尾部
提出为已有辅助函数，前置代码直接调用 `WorldTransaction::begin`。实际生成体保留 Begin
返回的业务结果和连接，避开 #965 泛型前置视图中回写签名不一致的问题。生产源保持固定。

## 表示与结论

检查器校验原归档、调用闭包、真实 Charon/Aeneas 收据和父 IR。它比较 #963 与 #965 的实际
记录、Inbox、Sources、journal、保存及匹配字典声明，保留完整父生成声明及原 `CurrentSource`
结构的字节。名称作用域用分立参数承接库及其它业务接口，不导入含公理的外部模板。

- `invalid_observation_first` 从工作所需观察与守卫拒绝推出 `InvalidHostEvidence`，直接
  返回原 journal，结论不要求 Begin 或任何下游接口成功。
- `begin_error`、`inbox_error`、`inbox_absent`、`authentication_error`、`sources_error`、
  `sources_absent`、`current_error` 和 `different_current_conflicts` 逐步只要求已到达位置
  的回报，并交回 Begin 返回的原连接。空 Inbox 或 Sources 对应 `UnsupportedStore`。
  已重建的当前工作使用与记录扫描相同的完整工作相等判断，其不一致先于扫描。
- `exact_current_continues` 将完整一致的路径等同于 #967 的实际后段。它传入 Begin 的
  事务、原 Inbox、从该事务读出的 Sources 及 Begin 的连接回写。由此接上第一条完整
  匹配、`Fresh`/`Replay`/`Conflict` 及保存和提交首错。
- `fresh_commit_uses_started_transaction` 把同一 Begin 事务用于读 Inbox、读 Sources、
  读取当前 World、保存更新后的来源集合及提交。保存用的连接来自原事务的同一解引用，
  父方法交回 Begin 的回写连接，并返回只更新选中观察字段的同一记录。

定理从已解码值开始。#964 提供完整工作和观察比较，原观察守卫的值证明见 #965。
V0 的 Option 借用/复制和字符串借用、V5 的事务开始、解引用、World 读取、两处存储读取、
`codec::save`、提交，以及 V7 认证上下文的来源均以各自的回报条件接入。原 `authenticated`
和 `snapshot::current` 仍需各自提供业务语义，V6 提供物理来源事实。本切片核对同一事务值
和连接的实际传递。把该值的 `codec::save` 接到真实 SQLite 的 #956 逐行读写、持久可见性
及失败回滚，仍是后续义务。

## 精确复验

`slot_parent` 和 `slot_core` 为 #963 的原父及独立查找捕获，`prefix_parent` 为 #965 的
原父捕获，`commit_tail` 为 #959 的原提交尾部。各自使用原归档及固定工具生成。
`plan.json` 绑定 11 个物理 Lean 文件、父 IR 和两份完整生成声明。

```bash
set -euo pipefail
export LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_observation_parent.py" \
    --original "$slot_parent" --prefix "$prefix_parent" --scan "$slot_core" --tail "$commit_tail" \
    --output "$project" --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build ObservationParent)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 11 and (.audited | length) == 28' "$audit/result.json"
```

ning 的正式普通及原生复验复用 #965、#963、#959 的同一源码和正反入口。三个原错误版本分别
前移 Begin、放宽为弱匹配、提前执行提交后检查。复用仍绑定原候选并保留原负例收据。统筹可将
`ObservationParent` 的完整父声明闭包和审计补到既有 Stage1 入口，覆盖表使用
[候选补丁](public-registration.patch)。公共登记及完整验收由统筹执行。
