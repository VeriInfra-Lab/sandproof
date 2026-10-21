# BTreeSet 插入接口：当前连接与历史预检

当前交付为固定 Global、`&str` 与 `(&str,u16)` 的正常值连接组件，见
[当前使用范围和复现](model/README.md)以及
[PR #626 的封口结果](https://github.com/VeriInfra-Lab/sandproof/pull/626)。
代码/证明冻结为 `59971b19`：原工具重抽取、正确版22项审计/九份实际文件、
真实错误版同一性质拒绝的两项审计/两份实际文件，以及普通组件均通过。
共享接线、完整组合验收和合入仍由统筹完成，不表示 V0 或 Std 本身已证明。

Lean entries 只表示 new/insert 的集合值视图，不表示 Rust 迭代顺序、树布局或指针身份。
固定类型与 Global 以外不能直接套用；正常分配、借用、Drop 和异常边界以当前说明为准。

## 历史预检原始记录（3590d199）

以下完整保留最初阶段的结论和限制。“尚未 Lean 编译/证明”等用词只描述当时状态，
不能用来否定上面的当前交付；旧模板和失败记录也没有删除或改称已证明。


本实验对应 #333 T3/T4，基线 integration `a12f4e16b491cd58452d2b967f06af1bfc48a847`。
仅新增透明 Std 委托，生产 Rust、Charon/Aeneas、固定工具及共享证明入口均未改。

[实际探针](probe/src/lib.rs)、[实测摘要](preflight-summary.json)、
[精确外部签名](opaque-signatures.json)、[真实来源](source-map.json)。

## 本阶段结论

a6000 实际执行五次 Charon 与五次 Aeneas，退出码全部为 0；
组件 9.402 秒、宿主 9.694 秒。新增源码、target、tmp 和证据物理位于 /data1/zn，
使用原 formal-heavy 重锁；本阶段没有普通测试、Lean 编译或语义证明。

| 根 | 输入与范围 | 结果 |
| --- | --- | --- |
| insert_u64 | 无借用键对照 | 代码生成成功 |
| insert_str | 原外部 BTreeSet<&str> 和原借用字符串 | 代码生成成功，返回 bool 与修改后集合 |
| insert_lane | 原外部 BTreeSet<(&str,u16)>、节点字符串和原 slot | 代码生成成功，使用实际字符串/引用/元组 Ord 与 OrdU16 |
| repeat_str | 同一原集合连续插入同一借用键两次 | 代码生成成功，第二次使用第一次返回的集合 |
| new_then_repeat_str | 真正 Std new、两次 insert、作用域 Drop | 代码生成成功；Drop/unwind 语句在原 LLBC 中保留 |

两个真实键型分别来自 `valid_tenants/valid_nodes` 和 `valid_lanes`。
lane 的 slot 是 u16；没有用 u64 或自建 Vec 替换实际类型。
后三种组合/对照是 API 可行性探针，不是完整生产验证函数。

## 已表达的接口与仍缺的模型

原 insert 返回 bool，没有返回给调用者的引用。
生成的值接口保留可变集合回写：

```text
insert_str : Set Str Global -> Str -> Result (Bool * Set Str Global)
insert_lane : Set (Str * U16) Global -> Str -> U16
              -> Result (Bool * Set (Str * U16) Global)
repeat_str : Set Str Global -> Str
             -> Result ((Bool * Bool) * Set Str Global)
```

这里的 Set 只是阅读缩写，精确全名为
`alloc.collections.btree.set.BTreeSet`。
原生成代码的第二次 insert 使用 seen1，并返回 seen2；
尚未证明重复标志的值必为 false。

生成物要求外部类型与函数实现：一个 BTreeSet 类型、new/insert 两项操作，
以及共享引用、str、二元组的八项比较接口，共 11 个独立声明。
Global 分配器克隆、u64/u16 的 Ord 已使用既有内置字典。
不能把已存在的 storage String/str BTreeMap.get 模型当成 Set.insert 已有实现。

原工具产生的 TypesExternal_Template/FunsExternal_Template 含未实现声明。
它们只保存在忽略的原始证据中，没有改名导入、没有填公理/占位证明，
也没有据此声称 Lean 已检查通过。当前发现的是库模型接入缺口；
后续真实模型接入后的 Lean 类型/trait 检查仍可能暴露新问题。

## 正常与异常边界

单次委托的完整 LLBC 含一个调用及其 unwind/resume 边。
构造探针保留三个条件 Drop 语句：一个正常结束位置、两个 unwind 清理位置；
这是语法位置数量，不是每次运行必执行三次。
生成 Lean 将最终集合丢弃，没有给出这些析构/异常路径的效果证明。

因此，本实验不证明 panic/OOM/abort 后集合未修改，不证明物理内存释放、
任意自定义 key/allocator Drop，也不证明完整 valid_tenants/valid_lanes。
后者的早停、外部集合状态、业务唯一性和配置条件仍需各自连接。

## 建议下一小阶段

先确认并登记既有 A6 中的具体标准库合同，再实现真实定义并检查生成文件。
建议范围只到 Global 与本次两个真实键型；u64 仅作对照：

1. 有效有限集合、合法借用、键值在借用期内稳定，比较采用固定 str 值顺序和元组字典；
   不覆盖任意有副作用的用户 Ord 或 allocator。
2. 正常且可表示、分配成功的域中，new 的值视图为空；
   insert 的新视图是旧视图加原键，bool 当且仅当原先不存在等价键。
3. 其它成员和原键值保持；不额外承诺树形、地址、比较次数/顺序或物理 RAM 上界。
4. panic/abort/OOM 与可观察 Drop 不由正常值合同保证，不假设回滚。

以上是待登记/确认的具体条款，本阶段尚未把它们作为已落实保证使用。
Lean 可用有限集合表示值视图，Rust 仍使用原 BTreeSet，不能替换原算法。
先让两个真实键型的原生成文件完成实际类型检查，再推进合法接受、重复拒绝和错误版。
不在此扩展 contains/len/collect、可变 find、Result 回调、排序或整个验证器。

## 复现和证据

固定 Charon `b104e24fea7d721b71e6c39fd70f26ff20bc0980`、
Aeneas `505b6ca35217e7be5c96c3e2f8045edfbdf47291`、
提取前端 nightly-2026-08-18。两版编译器及相关 Std 源码摘要在实测 JSON。
在 a6000 原固定环境内对每个根执行以下实际参数形状；输出放新私有目录：

```sh
cd verification/experiments/btree-set-binding
cargo generate-lockfile --manifest-path probe/Cargo.toml --offline
# ROOT 为本实验目录，OUTPUT 为 /data1/zn 下新建目录；TOOL_ROOT 为固定工具根。
RUSTFLAGS="--remap-path-prefix=$ROOT=." \
"$TOOL_ROOT/aeneas/charon/bin/charon" cargo --preset=aeneas --error-on-warnings \
  --sysroot default --start-from crate::insert_str --dest-file "$OUTPUT/insert_str.llbc" \
  -- --manifest-path probe/Cargo.toml --lib --locked --offline
"$TOOL_ROOT/aeneas/bin/aeneas" -backend lean -abort-on-error -warnings-as-errors \
  -split-files -emit-json -namespace SandproofBtreeSetProbeRust \
  -subdir SandproofBtreeSetProbe -dest "$OUTPUT/insert_str-lean" "$OUTPUT/insert_str.llbc"
```

实际五根的完整命令、全部 LLBC/生成物（含未使用模板）、Std 源和工具摘要在证据包；
输出路径变化会改变原 LLBC 元数据，应保留原文，不删除字段或回填旧摘要。
包含 62 份逐文件索引记录，SHA256：
`d0ddc44d87f3f3890d1a4dd53d395663931091782866e39f23e39e45d26fac01`。
原始目录 `/data1/zn/sandproof-btree-set-binding.6yeicwz2`。
普通测试、Lean 类型检查、语义定理、全套验收与可合入状态均未由本实验完成。
