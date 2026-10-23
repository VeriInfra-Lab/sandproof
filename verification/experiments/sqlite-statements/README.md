# SQLite 代表语句的隔离连接

本目录属于 [V0 #333](https://github.com/VeriInfra-Lab/sandproof/issues/333) 的 T7 批次。
它只准备隔离候选和复现材料，不修改或启用生产调用，不改 Charon/Aeneas。
执行及交付规则见[证明工作流](../../../docs/proof-workflow.md)。正式范围和已证明状态由公共覆盖记录及该 issue 维护；本目录存在不表示模块或完整入口通过。

## 固定范围

原入口是 `controller-service/src/journal.rs::write_accepted_with_ids` 的
`sandboxes` 条件 UPDATE。候选只把原修订号计算和原 UPDATE/五参数/行数判断移动到
同模块的 `statement_core`，由原候选调用者实际调用。SQL 原文字节不变。
`candidate.json` 固定基线、原文件、候选文件、补丁和完整源码归档摘要。

参数依次为新修订号、完整状态字节、sandbox、owner 和 expected revision。
保留 `checked_add`、`as_sql_integer`、`changed != 1` 和原错误传播；不把 SQL、
参数选择或结果解释交给可信业务回调。SELECT/列解码仅用于核对读回形状，不能代替 UPDATE 连接。

原 `params!` 候选经过真实预检：Charon 可生成 UPDATE 的 LLBC，但原 Aeneas
不支持宏产生的动态 trait unsized cast。因此当前候选只把该宏换成固定 rusqlite
提供的五元组 `Params`。两者在这条五参数语句上依次绑定 1–5；元组先检查参数
个数，原宏路径在迭代后检查，不能据此声称任意 SQL/自定义 ToSql 都等价。

## 前提与不推出的结论

| 提供方 | 本批使用的条件 | 本批仍须保留的责任 |
|---|---|---|
| A4 固定 rusqlite/SQLite | 原事务/数据库、声明 schema/配置/故障域中的参数绑定和执行 | 自有 SQL、五位置/类型、整数边界、行数及错误解释 |
| 已获批 B4 | 登记的原对象与普通值机械传递 | 不包含授权、选语句、修订号、提交/恢复或释放决定 |
| 调用者/部署 | 原数据库保护、相同事务、实际 schema、有效列域及正常语句修改范围所需的触发器限制 | 非 STRICT SQLite 的声明类型本身不能证明所有行已在模型域内 |
| V5 / #338 | 完整事务、投影、outbox、寿命、确认/派发与恢复 | 不能把本批局部成功或影响行数当耐久提交 |

底层错误可能已有部分效果；回滚成功、回滚未知、提交确认和提交未知分别解释。
外部 VM/文件/网络效果不随 SQLite 回滚。普通字节/整数的绑定模型不覆盖任意
有状态 ToSql/回调/资源析构，也不保证 OOM、进程崩溃或永久外部故障下完成。
A/B 条件是显式依赖；源码审查/原生测试不冒充其机器证明。

## 物化

从有该精确提交的仓库创建新私有副本：

```sh
python3 verification/experiments/sqlite-statements/prepare.py \
  --source-base /path/to/repository \
  --destination /path/to/private/candidate
```

远端不复制 Git 历史。可使用 `candidate.json` 已冻结摘要的完整 Git 归档：

```sh
python3 verification/experiments/sqlite-statements/prepare.py \
  --source-archive /path/to/frozen-source.tar \
  --destination /path/to/private/candidate
```

已有目标（含符号链接）、原文件/补丁/归档不符、扩大候选修改范围均拒绝。
物化回执只证明该准备步骤，不是编译、测试或证明成功。

## ning：真实 SQLite 组件

使用已有固定 Rust/Cargo 环境，在宿主监督进程持有原 `native-heavy.lock` 时运行：

```sh
python3 verification/experiments/sqlite-statements/native.py \
  --source-archive /path/to/frozen-source.tar \
  --output /path/to/private/new-native-evidence
```

显式设置既有 `CARGO_HOME`、协调的 `CARGO_TARGET_DIR` 和 PATH；不另建 CI 环境。
入口物化三个私有源副本，正确版运行原八项实际数据库用例和严格 Clippy；错误版
仅把实际调用核心的 `changed != 1` 改成 `changed > 1`，新编译后必须触发原来的
“零行 UPDATE 必须修订冲突”断言。编译失败、没有新二进制或不同断言不能封口。
另一个实际 SQL 错误版只移除 WHERE 中的 owner 条件，保留原五个参数和行数检查，必须触发原 owner 拒绝断言；这项是数据库回归，不冒充该 SQL 错误版的形式反驳。源码、测试二进制和原始日志保留。有限原版/候选对照不证明一般行为等价。

物化及来源准入检查同样只在 ning：

```sh
python3 -m unittest tools.tests.test_sqlite_statements_materialization -v
```

## a6000：原工具抽取

源码、TMPDIR、稳定 formal target、生成物和全部证据的实际宿主落点都使用
`/data1/zn/` 下本线目录；固定工具/Lean 包缓存只读复用。
获得统筹安排的时隙并由宿主监督持 `formal-heavy.lock` 后运行：

```sh
python3 verification/experiments/sqlite-statements/translate.py \
  --source /path/to/private/candidate \
  --output /path/to/private/new-extraction-evidence \
  --tool-root /path/to/fixed-original-tools
```

一次原 Charon 编译同时覆盖两个实际核心。保存完整 LLBC、原 Aeneas Lean/库存、
源码/工具摘要及全部诊断；失败返回非零，不用旧产物补齐。
`--variant accept-zero-changes` 只接受精确登记的错误源码，供后续同性质反驳，
不是把抽取成功当成反例成立。

抽取不包含最终库模型、Rust 对应、SQL 连接、形式错误版反驳、实际文件诊断与
依赖审计。它们全部完成后才可交本线 draft PR；共享 Lake/入口/总审计、权威
文档和完整双机验收由统筹加入同一个 PR。本批正式集成前不启动 T8。

## 可复用的证明接口

`SandboxModel/Boundaries/SqliteStatements/` 分为六部分：

- `Model`：只描述该表的有限 UPDATE 片段，参数保留整数、完整文本与 blob；证明条件完整、匹配行写入原值、其他行不变。它不实现通用 SQL 解析器。
- `Integer`：针对原工具抽取的真实 `next_revision` / `as_sql_integer`，证明 checked_add、SQL 整数上限及原错误类别。
- `Bindings`：完整五元组的顺序和值，以及原连接上的执行调用。
- `Contract`：不依赖所选程序的 `NormalReplyContract`，规定原 SQL、参数和正常/错误返回的对应。
- `Refinement`：实际候选抽取函数满足该合同，保留原转换和底层错误。
- `Bridge`：把原 SQL 的字节等式、实际五参数、A4 通用片段语义及真实 Rust 的行数判断连接起来；包含非空成功和错误可有部分效果的例子。

使用者提供原事务/数据库的一次实际调用观察及 A4 条件，才能得到该调用的行效果。
单次观察不能当作可复制数据库；多次读写必须由 V5 维护同一事务的连续历史。
`Ok(1)` 说明语句命中一行，不说明事务提交，更不说明 outbox 已派发。

对应关系的可信部分逐符号登记在
`verification/sqlite-statements-environment.json`：每项都写输入域、原对象、首个失败、
正常返回、修改范围、展开/崩溃限制和调用者责任。固定引擎语义是 A4，原值传递是 B4，
自有 SQL、参数选择、错误和行数解释保留在抽取证明中。调用者仍须提供：

1. 原事务、正确且受保护的数据库和固定库/部署配置。
2. 实际 schema、所有行的非 NULL/有效整数/UTF-8/blob 域，以及 BINARY 文本比较和未登记触发器不存在。
3. 表的逻辑行视图及同一事务事件历史。列表顺序是逻辑枚举，不是任意 SELECT 的返回排序承诺。
4. 完整事务、分配的 outbox IDs、序列化、后续投影、提交、恢复及授权的业务证明。

`Journal::sandbox` 已只读核对：原 SELECT 按 `sandbox_id=?1` 读取
`owner_id, revision, state_blob`，按列 0/1/2 解码为 `String/i64/Vec<u8>`，先通过
`TenantId::try_from` 验证 owner，再用 `u64::try_from` 转换 revision；负数是
`CorruptStore`。这不是本批 SELECT 的对应证明；读取与转换的生产连接仍须在 V5 中完成。

原真实控制器的 `controller::storage::apply` 把原 `WorldTransaction`、完整 record/ledger
字节、actions 和预留 IDs 传给 `write_accepted_with_ids`。本批只分离其中的修订号与
条件 UPDATE；前后的检查、outbox 循环和调用关系留在完整候选源码中。

## 同一性质拒绝实际错误版

`negative.py` 只把隔离 Rust 中的 `changed != 1` 改为 `changed > 1`，仍先运行
原编译器/Charon/Aeneas。`negative/Funs.lean` 是其未修改原输出；独立负例视图载入
它以及与正确版同字节的 `Contract.lean`。`negative/Counterexample.lean` 使用合法
非空 ASCII 标识符、expected=1/next=2、原连接和返回零行的可实现正常观察，证明
该真实错误版不满足同一个 `NormalReplyContract`。负例不能导入正确版 Refinement。

源头的 owner 条件删错另由实际数据库回归检测，未宣称该 SQL 错误版已机器证明。
完整原 LLBC、普通/展开路径 Drop、字段和 span 均保留并检查。借用核心不拥有外层
Transaction/Connection；本批正常返回证明不提供库析构、panic/unwind/abort 或崩溃后状态。
原工具自动生成的接口模板以未修改 `.txt` 保存于 `raw-templates/`，仅为生成证据，
从不作为 Lean 输入；实际模型文件没有自定义公理或占位证明。

## 模块复现及共享接线

普通准入回归在 ning：

```sh
python3 -m unittest tools.tests.test_sqlite_statements_materialization \
  tools.tests.test_sqlite_statements_admission -v
bash -n scripts/regenerate-sqlite-statements.sh scripts/verify-sqlite-statements.sh
```

a6000 获得协调时隙并在宿主持锁后，使用原工具、现有 Cargo target 和只读 Lean 缓存：

```sh
export SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE=/path/to/frozen-source.tar
export SANDPROOF_PROOF_TOOL_ROOT=/path/to/fixed-original-tools
bash scripts/verify-sqlite-statements.sh
```

入口检查冻结输入，重新物化并抽取正/误两个实际 Rust 核心，对比完整 AST/原生成库存，
再用两个小型独立 Lean 视图运行原服务、实际文件诊断和全部审计。所有可写文件位于
当前源码目录的 `artifacts/` 下，a6000 当前源码目录也必须在 `/data1/zn/`。
该入口只负责本形式模块，不运行普通测试或 CI，不替代 matching ning 回执和完整集成门槛。
`regenerate-sqlite-statements.sh` 只生成新证据目录，不自动更新正式文件或冻结记录。

独立视图是必要的：目前原 Aeneas 多个生成库含重名的全局 discriminant 实例；直接
共同导入会发生实例声明冲突。这里完整保留生成物，通过已存在的独立模块验收方式
组合证据，不关闭检查、不替换旧根、不改变原工具。纯 `Model` 可独立导入；真实
Rust 连接定理及所有原有模块仍须分别进入完整入口。共享 Lake/总入口、native case、
总审计、用法登记和权威覆盖/依赖文档由统筹接入同一个 draft PR；正式集成前不启动 T8。

固定库源码复核的特别限制：`raw_bind_parameter` 也可能失败，即使 i64/str/blob
的 ToSql 转换本身成功。因此本模型按整个 `Connection.execute` 提供 A4 联合摘要：
`Statement.arguments` 仅为完整输入的逻辑编码，`Connection.run` 包含真实 raw_bind、
step/reset 的原 API 返回结果。固定库在 DONE 后检查 reset，ROW 与其他 step 错误
分别使用原错误优先级；Statement::drop 会忽略 finalize_ 的 Result，不能宣称清理
错误都会返回，或 API 成功就证明清理无错。编码 helper 的 Ok 不等于
真实绑定已经成功。真实 Rust 核心只观察整个 execute 的最终结果，直接运行内部
`__bind_in` 的使用点由完整源准入拒绝；这组模型不能用于裸 Statement 或任意
有状态 ToSql。调用者仍从实际库得到整个 API 的成功/错误观察，不能从编码引理
推断已执行过任何原生绑定或数据库写入。
