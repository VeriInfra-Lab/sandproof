# 实际标准 ID 委托

本包对应实际 `StandardCreateAttachment::sandbox_id`，从已经解码、满足原字符串表示和输出容量条件的值开始，复用既有 `CreateFixedIdentity` 完整数学。`FullStandardIdentityContract` 保留完整 namespace、全部 u64 位、完整结果及原完整效果对象，不增加 candidate 或 transaction 前提。

正确、遗漏 namespace、截断编号使用同一 Model、正常完整上下文、类型连接及原 helper 数学。两个真实 Rust 错误版只改变 Std 委托参数；不安装或导入正确 Std 对应。正常值、空 namespace、u64 上限和原错误保持有见证，有限见证不替代完整全称对应。

完整 World、WorldTransaction、CommittedWorld、Std 及 rusqlite 事务字段保留实际透明生成体。未使用的上下文字段复用现有 canonical Connection／RefCell 数据载体，每个 Rust 错误类型在同一视图只使用一个载体。本委托不调用 RefCell 或 SQL 操作，没有新增外部函数合同；原固定双参数格式与字符串库条件、正常分配等 A6 范围保持。

该结论不表示 ID 全局唯一、授权、资源存在、耐久接受或整个标准实例／Create／V3 已证明。实际对象和事务历史由具名 V5／A4 提供者承担，JSON／SHA、load／save、提交未知、重启、producer／driver 和 Resume 支持域仍单列。协议 `target > source` 允许空洞。

`formal.py` 必须真实重新编译并抽取三个视图，核对完整生成函数、外部符号、完整载体和原转发根，再检查所有实际文件的内核、规定同服务完整非 stale/partial 诊断、依赖与精确审计。原始失败保留，编译／导入／抽取失败或超时不计性质反例。不得修改固定工具或生成正文。

`native.py` 交 ning：actual／namespace-omitted／number-truncated／restored 四份源共用五项断言，检查实际新编译的协议库与服务测试、源／manifest／链接、精确失败集合、格式和严格 Clippy。实际 WorldTransaction 及指针／值保持是有限 API 见证，不代替一般物理事务证明。

公共登记、输入／冻结、累计审计／覆盖／依赖及共享套件由主控单写，本包只提供候选。采用或版本绑定变化后必须重新核对实际输入和运行结果；输入清单本身不表示执行通过。旧 `083580e5` 本地执行与原始包保留原身份，不改签为新源码执行。普通／原生、共享准入及完整 V3 均未完成。

证明机的独立开发入口保留完整三视图；`--prepare-only` 只供抽取与构建预检，不能计作完整检查。输出使用本轮新目录，锁使用本批已有共享重锁：

```sh
python3 verification/experiments/stage1-protocol/create-standard-identity/formal.py \
  --output artifacts/create-standard-identity-development --lock /private/existing-proof-heavy.lock
```

ning 使用 `native.py --output /private/create-standard-identity-native` 运行同源四视图。
全套准入与证据交接以[证明工作流](../../../../docs/proof-workflow.md)和[本组件入口](../README.md)为准；此独立开发入口不产生共享验收封口。
