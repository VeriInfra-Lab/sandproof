# Create 寿命 UPDATE 的两项完整值

本包连接唯一真实 `lifecycle_inbox::lifetime_sql::execute`，保留同一个
`&Connection`、完整 SQL、i64 deadline 与完整 sandbox 文本。原实际调用的位置、
identity/deadline/UPDATE/save 顺序不变；成功行数原样返回，包括零。
`FullSqlValuesContract` 从已解码值开始，只描述显式有限库边界下的完整参数与 API
结果，不表示授权、资源存在、真实行效果、同一事务、持久提交或完整 V3。

## 合同与原始实现

原调用为 `connection.execute(statement, params![deadline, sandbox])`。
固定 rusqlite 0.40.2 的 i64 `ToSql` 实际返回 `Owned(Value::Integer)`；新数组中的
原始载体是 `Borrowed(ValueRef::Integer)`，二者原始枚举不相等。本包透明抽取原
`ToSql`、`Value::from(i64)`、`ValueRef::from(&Value)`、共享引用转发与
`ToSqlOutput::to_sql`，证明它们归一化后的完整数值一致。原 str 编码保留全部字节。
真实两元素数组的 `Params::__bind_in` 及固定共享数组迭代连接完整槽位和顺序。
完整 Null/Integer/Real/Text/Blob 及 Borrowed/Owned 载体保留，未缩成只含正常分支。

采用前动态 `&dyn ToSql` 宏视图在固定后端存在未解决的翻译限制；本包不声称已取得
整个旧宏的机器一般等价定理。当前有限采用的依据是透明原始转换的全值证明与下述
逐符号库合同，普通原生对照仅作有限回归。没有保留另一份生产业务实现。

## 依赖边界

- A4：复用现有唯一 SQL `Connection`、`Statement`、`SqlValue` 以及原文
  `bindValues`/`Connection.execute` 定义。`Statement.arguments` 记录逻辑输入，
  不是已成功 raw-bind 的资源；`Connection.run` 仍包含原 raw-bind、后置参数计数及
  step/reset 错误。一次 prepare/run 观察须来自一致的库调用历史，跨调用与事务由
  实际提供者另证。
- 本次仅扩到固定纯 ToSqlOutput 字典和实际两元素共享数组迭代的 whole-execute
  使用点。原五元组的提前计数条件不适用于本数组。真实数组 count=0 时先得到
  index=1；count=1 时先执行第一项绑定，之后才是 `(2,1)` 数量错误；更大的 count
  先处理两项绑定，首个 raw 错误优先。未把这些操作改成提前 `count == 2` 守卫。
- A6：使用已登记字符串字节、有限正常表示域和普通内置数组迭代合同。字符串与
  ByteArray 原数学按原文复用。固定库实现与这些模型的相符仍是显式提供者条件，
  没有添加自定义公理或把整个驱动设为已证。

`materialize.py` 按公共源的精确片段与摘要生成局部连接，原名称和函数体保留。
旧公共 Types 同时登记了另一份业务错误 discriminant，故独立视图只连接公共载体
及当前生成的完整 Params trait，不导入旧业务 Types。共享源保持单写；不会在本包
维护另一套 SQL 算法。生成体、LLBC 与原始外部模板均留证，外部 axiom 模板不导入。

## 正反入口

正常规格与正常完整输入为所有视图共用。任意四输入定理保留 prepare/run 的首错、
实际 Sqlite 错误转换、外层效果及任意成功行数。见证包括完整正常参数、成功零行、
负整数、含 NUL/非 ASCII 的完整文本及两个 API 错误阶段；后者是 helper 的值域
见证，不是完整业务请求已接受。

两个实际 Rust 错误版分别以 `deadline & 0` 丢失整数、以实际空串分支丢失 sandbox。
它们须实际编译、抽取、到达同一 `FullSqlValuesContract` 反例；各自保留零值或空串
正常情形。错误视图不导入正确 helper 对应或正确见证。编译、导入失败与超时不是
性质拒绝。新增自有审计名为 29 个；复用的原字节数学不重新计数。

按项目证明工作流使用固定 Charon/Aeneas、Lean 及规定同服务客户端。以下是独立
开发形式入口，使用单一共享重锁、四核/四线程、原 64GiB 余量和额外写入预算，
每份独立 `.lake/build`；不清理共享缓存：

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/create-lifetime-sql/formal.py \
  --output /private/proof/create-lifetime-sql \
  --lock /private/proof/.proof-heavy.lock
```

`--prepare-only` 仅编译、抽取和构建，不能当作服务验收；默认检查所有实际文件的
内核、完整非 stale/partial 服务诊断及依赖。可用 `--variant` 选择一份视图。
`--lock-fd` 只接收同一锁文件、同一打开对象，供主控外层统一持锁。唯一命名风格
豁免仅作用于独立生成库的真实 `__bind_in` ABI 名称，不关闭内核或依赖检查。

普通/错误版原生入口交 ning；四份源各自新编译协议库和真实 service 测试，执行相同
六项断言、精确失败集合、格式与严格 Clippy，结束再核对所有输入。deadline 错误
版的特定 erasing_op lint 单列，不算业务反例，也不忽略其它错误：

原生测试还以独立内存库的 17 行／触发器模式构造同一个完整参数观察：正确参数更新
17 行，遗漏整数或文本时为零。它使用与形式反例相同的 SQL、deadline 与完整 ID，
用于 ning 检查 API 反例的可实现性；不把测试模式当作部署 schema 或已取得原生回执。

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-lifetime-sql/native.py \
  --output /private/acceptance/create-lifetime-sql
```

公开独立入口不等于已接入共享 `verify-stage1-protocol.sh` 或完整 controller 验收。
公共 Lake/Cargo、根导入、登记、原生套件及依赖/覆盖记录由主控统一采用；见本包的
候选交接说明。原始证据实际接收与源码可读分别记录，普通与共享验收不能由本包
开发形式结果替代。
