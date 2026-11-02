# 标准 BLOB 读取与原 Load 摘要输入域

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) · [统一工作流](../../../../docs/proof-workflow.md)

本组件在 `9c50da7c` 后的本批候选中采用真实读取重构：命名原两个 `Row::get` 表达式，
并将零参数 `[]` 改为 `()`。保留原无捕获闭包、SQL／列参数、首错、字节和整个接收者值。
SHA 同批统一委托唯一 `lifecycle_inbox::sha256::digest`；本包不再在正确私有视图替换读取正文。
原数组只作独立有限 API 对照，不声称一般旧新机器等价。源码采用与当前验证成功分别记录。

`FullColumnsContract` 与 `FullStandardReadContract` 对任意完整值陈述实际 helper、回调、零参数绑定和标准方法的对应。两个真实错误版分别重复第一列、先读第二列，使用同一完整 Model／Values／库解释和正常见证；错误环境不安装或导入正确 Core/Witnesses。编译、抽取、导入失败或超时不是合同反例。

原 Request、协议和 Load 规格继续引用已存在的提供者；四项 Store23→24 文本适配沿用已采用的逐位置规则。`NormalContext` 用有见证的正常行输入表示接入实际读取，成功 BLOB 的 c_int32 上界给出原摘要正常输入域。两个生成视图之间显式传递完整有序字节对、SQL 错误载体和整个 Context；不猜不可表示的业务错误。原 IO 只替换 query_bytes_pair 字段，其他字段、正常摘要 agrees、Graphic 和解码表示条件仍显式保留。

五个有限库接口候选为 Connection::query_row、Statement::ensure_parameter_count、Row::get<usize,Vec<u8>>、usize::idx、固定 Vec FromSql。canonical Connection／Statement／SqlValue／错误和五类 ValueRef 保持完整。query_row 只取首行，不增加唯一行条件；参数个数不等于列数。库符合性、同原对象的行／列／错误观察及正常 BLOB 来源需主控/V0 逐符号协调采用。类型别名、值相等和单次读观察不证明物理同事务、数据库堆不变、16MiB限额、两读一致性、授权、材料存在或耐久接受。

固定工具在私有抽取副本给16个原枚举添加命名元数据，解决独立视图全局辅助实例重名；逆变换必须恢复每份原生产文件，类型和函数正文不变。原 Load 的 ContractError 规范别名复用已采用 FullCreate 的透明原枚举／固定 Foundations 全形状核对。生成体不手改，生成外部公理模板不导入，工具与内核检查不改变。

generated-index.json 固定六次原始捕获及五组实际使用的原 Types/Funs/translation；正式入口重新编译、抽取并逐字节比较，不复制或修补生成体。未导入的外部公理模板不作为证明源发布。完整文件、模式、原 IR/生成目录、命令日志、工具/库在结束时再次核对。主控的三个候选补丁及累计审计要求见 [登记说明](REGISTRATION.md)。

原 LLBC 的 short_names 是按键解释的名称表，固定工具输出顺序不稳定。每次保留原字节/哈希；完整 JSON 值比较仅将该表按唯一键排序，拒绝重复键，其余数组和全部声明/正文保持原顺序。生成 Lean 仍逐字节比较，不将名称表排序差异算作业务反例。

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/standard-blob/formal.py \
  --output artifacts/standard-blob-formal --lock /path/to/existing-proof-heavy.lock
```

`--prepare-only` 只产生当前编译、抽取和构建记录，始终 `complete=false`。正式开发运行核对全部保留真实文件、原内核、规定同服务完整非 stale/partial 诊断及依赖。已持有主控协调锁时传原 `--lock-fd`，核对同一设备/inode后复制原描述，不重新打开等待父锁。每任务至多四个已获准 CPU、独立 `.lake/build`，现役缓存和失败证据保留。

普通方运行同一个 native.py 的数组表示／实际 Unit／两错／原字节恢复五视图，共用九条真实 SQLite API 断言、实际新编译源／manifest／链接、完整失败集合及格式／严格 Clippy。它不运行 Lean，也不代替完整 Linux functional 或收件事务测试。

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/standard-blob/native.py \
  --output artifacts/standard-blob-native --lock /path/to/existing-proof-heavy.lock
```

原7aa候选的局部证据保持原身份，详见 #635；模板数学原样复用不表示新基线已验收。本批已采用当前源码与有限库条件；普通/原生封口、公共登记/根导入/冻结/累计审计及共享套件仍须分别取得当前收据。主控已经交付的12个 Create 包不重复导入。JSON/Writer/TLS、独立认证来源、其余标准 IO、producer/driver、提交未知/重启及 Resume 和整个 V3 继续开放；源准备、局部证明或32字节相等不推出这些结论。
