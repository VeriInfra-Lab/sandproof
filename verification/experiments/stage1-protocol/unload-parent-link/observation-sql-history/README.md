# 来源观察的 SQLite 写入历史与耐久结果

[原父调用与行循环](../observation-sql-save/README.md) · [原 Drop 与回滚](../observation-drop/README.md) ·
[原提交可见性](../observation-commit/README.md) · [V5 义务](../../../../../docs/formal/05-local-service.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

生产输入固定 integration `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。复用 #971 正反联合抽取的 39 文件视图。
原 SQL 字面量、四个绑定参数、Vec 循环和父调用仍来自原抽取。未改生成声明、生产源或工具。

## 表示关系与条件

`Image` 按 `controller_source_work_records.id` 表示原表。每个有记录的主键保留
`state_json`、`sha256`、`reserve_bytes` 及可空 `work_key`。条件 UPDATE 使用实际四个
绑定值，目标主键及旧键关系决定匹配数，更新 SQL 的三个 SET 列。其他槽位保持原值。
本批的成功轨迹针对保存观察时原键已经登记的行。每步保存实际 `existingSpec` 的返回，
把原 SELECT 的槽与该步事务内行视图对应，再核对实际 UPDATE 调用及其回报。A4 提供
原受保护数据库上该成功语句的行后态和影响行数。Lean 从主键及完整旧键算出恰好一行，
保留 reserve 和键，不把实际返回值固定成成功。SHA 的输出仍绑定已登记的字节计算接口。

`Written` 以一条实际 Begin 的 raw transaction 及其连接为索引。上一行的后态就是下一行的
前态，连续处理任意长成功前缀或整份原编码 Vec。最后结果保持所有原槽的预留和键。
两个不同槽的成功写入在共同后态同时保留两份完整新字节和校验值。A4 的事务结束关系
覆盖确认成功及 `CommitUncertain` 的两个耐久分支。提交未知只允许完整的原图像或完整候选图像。

- `first_row_error_restores_original_image` 保留任意成功前缀及实际首错。失败语句可以留下
  未规定的事务内状态。实际 Drop 回滚返回 Ok 且 A4 确认原事务回滚时，耐久来源表恢复
  原图像，原 memo 保留，原行错误继续返回。回滚失败时沿 #971 的缓存失效结论处理。
- `before_commit_hook_error_restores_original_image` 与 `after_commit_hook_error_keeps_committed_image`
  分别通过原钩子调用顺序，区分整份写入完成后确认回滚和原提交成功后通知失败。
- `committed_image_and_original_result` 在原提交成功时保留完整候选来源表和原更新记录的返回。
  `uncertain_commit_returns_original_error_and_two_images` 保留原 `CommitUncertain` 错误，
  且排除与原图像、完整候选都不同的部分耐久图像。

`CommitCall` 使用 #770 分析过的 V5 成功通过四项校验后的消费和包装条件，绑定本次
`Started` 的 owner、raw transaction 和原错误。外部 A4 只负责真实语句、提交/回滚和耐久
图像，前提是原 schema、WAL、FULL、文件系统和排他租约。V5 仍需实例化初始图像的实际
来源、与 codec 读写的同一历史以及原校验/提交回调。已解码记录到原已登记键的普遍对应、
新占用空槽、整个 World 和其他表的原子性、重开恢复及业务授权各有独立义务。
`CommitCall` 的范围限已经进入原消费提交的分支。校验拒绝与回滚失败均不提供耐久结果。

## 精确复验

`drop_project` 和 `drop_audit` 来自 [#971 的原始复验](../observation-drop/README.md)。
材料化逐文件核验其 39 文件、82 项审计、原服务回执及日志摘要，复用这一完全相同的原抽取。
新增两个逻辑文件，各自进入 Lake 构建、实际文件服务与依赖审计。新视图的 `plan.json`
列出全部文件、原证据与工具配置摘要。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/prepare_observation_sql_history.py" \
    --prior "$drop_project" --audit "$drop_audit" --output "$project" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
(cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build ObservationSqlHistory)
bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
    --output "$audit" --lock "$SANDPROOF_LOCK_FILE"
jq -e '.complete and .physical_files == 41 and (.audited | length) == 101
    and (.warnings | length) == 0 and (.errors | length) == 0 and (.rejected | length) == 0' "$audit/result.json"
```

## ning 原生与公共验收

- #950 的原真实父方法用例 `second_update_failure_rolls_back_the_source_observation_prefix`
  在第二个 UPDATE 的故障点读取同一 SQLite 事务中已更新的第一行，检查确切字节、摘要
  与工作键，再核对原错误、回滚、重开以及同请求重试。
- #959 的原 `source_observation_commit_publishes_exact_bytes_between_hooks_and_replays_after_reopen`
  使用第二连接核对提交钩子前后及重开，保留提前调用后置钩子的原 Rust 错误版。#971 的
  实际 rollback 失败不清 memo 的错误版继续在原完整合同下核对。
- 固定生产已有 `source_work_unknown_commits_keep_actual_original_question_and_result`，
  由原 SQLite 提交注入在两种耐久结局重开，绑定原工作和值。可在确定生产版本直接运行
  `cargo test --locked --offline -p sandproof-controller-service --lib source_work_unknown_commits_keep_actual_original_question_and_result`。
  这个入口使用提交注入，证据是有限真实 SQLite 回归。

按各来源固定输入及原工作流统一取得正式普通、原生与错误版回执。公共入口和
[覆盖候选](public-registration.patch) 由统筹登记。本包局部视图不替代完整 Stage1 或 controller 验收。
