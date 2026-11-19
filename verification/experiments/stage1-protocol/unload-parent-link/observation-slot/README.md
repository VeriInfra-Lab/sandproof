# 来源观察的首个完整匹配与重放判定

[原观察的保存与提交](../observation-commit/README.md) ·
[来源记录完整相等合同](../first-match/README.md) ·
[证明工作流](../../../../../docs/proof-workflow.md)

固定 integration 提交 `b0d791ccc4d4e058b710dbf8f46766652b2853df`，树
`7ca91afd9cd79601d562f3c636066d192fc25fc5`。从已解码的 `SourceWork` 和
`RawSourceObservation` 开始。本批证明实际生成的向前扫描和所选行的重放判定，并核对
它们在完整 Rust 父调用中的接线。

## 性质与表示

- `lookup_returns_first_complete` 对任意有限原记录向量成立。依据既有
  `FullReplayContract`，检查接收整份原 `SourceWork` 与每条记录内的完整 work。
  实际返回的 `Option Usize` 按自然数索引对应 `List.findIdx?`，覆盖首个完整等值和
  无匹配两种结果。合法见证在一个同绑定但不同完整 work 的记录和两个完整匹配中选第一条
  完整匹配。
- `selected_slot_decision` 对任意所选行中的 `Option RawSourceObservation` 适用。
  依赖全值 `PartialEq` 合同及标准 `ne` 对 `eq` 的取反关系。没有旧观察时为
  `Fresh`，完整相等为 `Replay`，有旧观察但全值不同时为 `Conflict`。
- 错误版只把实际 Rust 扫描内的 `same_work` 改成 `same_binding`。同一份规格和夹具
  下，编译并抽取的错误版命中同绑定异完整 work，形式反例返回索引 0，随后违反
  `firstComplete`。原生夹具通过原记录和真实 SQLite 准备同样的输入，错误版实际在
  完整匹配断言失败。该向量反例刻画扫描本身，数据库能否保存这种向量由各自的保留合同约束。

临时 Rust 只把原 `iter_mut().find` 及相邻的两个重放判断改为固定工具可翻译的索引扫描
与决定。父 IR 复用原可达路径，扫描原事务加载的 `records`，转换该索引并在同一个向量
中取可变行，再把该行旧观察及原 incoming 传给决定。原实际比较字典把整份 work
交给 `SourceWork::eq`。只有 `Fresh` 写入同一行，并将原事务、来源集合和写后克隆
交给 [#959 的提交尾部](../observation-commit/README.md)。正例与父翻译的四个函数定义
和两个类型定义逐字节比较。完整父生成体保留供机械核对，窄 Lean 视图没有导入外部模板。

本批把具体派生相等实现的证明连接、前置认证和读回的完整首错、父方法 Lean 组合、
与 #956 实际行级 SQL 活事务的桥接列为后续。Host 提供观察的物理真实性依赖 V6。
同值、空结果或记录读回均不赋予授权、材料真实性或资源存在结论。V5 的真实事务结果
保持 [#770 与 #959](../observation-commit/README.md) 的约定。

## 形式复验

使用固定 Charon/Aeneas、共享证明锁、独立的 Lean 构建目录和原服务客户端。
三个捕获记录原工具命令、归档、LLBC、生成体与摘要。正反闭包各六份物理 Lean 文件，
其精确清单写入各自 `plan.json`。原生测试仍从固定 integration checkout 装入同一源码改写。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4
package=verification/experiments/stage1-protocol/unload-parent-link
for mode in core binding-only parent; do
    bash scripts/python-entry.sh "$package/observation_slot_capture.py" --mode "$mode" \
        --output "$capture_root/$mode" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
for mode in core binding-only; do
    root=ObservationSlot
    if test "$mode" = binding-only; then root=ObservationSlotBindingOnly; fi
    project="$capture_root/lean-$mode"
    bash scripts/python-entry.sh "$package/prepare_observation_slot.py" --mode "$mode" \
        --captures "$capture_root" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$root")
    bash scripts/python-entry.sh "$package/diagnose.py" --project "$project" \
        --output "$capture_root/audit-$mode" --lock "$SANDPROOF_LOCK_FILE"
    jq -e '.complete and .physical_files == 6' "$capture_root/audit-$mode/result.json"
done
```

## ning 原生复验

对 `core` 与 `binding-only` 各准备一个干净固定 integration checkout、独立
`CARGO_TARGET_DIR`，并在其中执行。第二条正确版用真实保存入口，首次写入依次命中两个
提交钩子。相同观察重放和同绑定异摘要的观察冲突均无提交钩子，实际持久记录保持相同。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$proof_checkout/$package/stage_observation_slot_native.py" \
    --source "$PWD" --mode "$variant"
native=(cargo test --locked --offline -p sandproof-controller-service --lib)
"${native[@]}" --no-run
case "$variant" in
    core) "${native[@]}" source_observation_slot_ ;;
    binding-only)
        code=0
        "${native[@]}" source_observation_slot_uses_the_first_complete_work \
            > "$native_negative_log" 2>&1 || code=$?
        test "$code" = 101
        rg -F 'observation slot must match the complete SourceWork' "$native_negative_log"
        rg -F 'test result: FAILED. 0 passed; 1 failed;' "$native_negative_log"
        ;;
esac
```

## 公共登记候选

统筹可将两个精确生成根、父 IR 接线核对和同提交原生正反入口接入 Stage1 检查。
正确根使用 `ObservationSlot` 的五项依赖审计，错误根使用
`ObservationSlotBindingOnly` 的两项反例审计。接入 [覆盖候选](public-registration.patch)
前须核对正式普通与原生结果、输入归档及公共 Stage1 和同批总入口。当前材料化只用于
本组件的局部验收。
