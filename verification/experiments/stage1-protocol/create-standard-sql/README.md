# 实际标准 SQL 委托

从实际 StandardCreateAttachment::execute_lifetime_update 到原 lifetime_sql::execute，复用完整 FullSqlValuesContract。FullStandardSqlContract 保留同一连接值、完整 statement、i64 deadline、sandbox 字符串、全部 Result 效果和完整返回上下文。成功行数包含零，不增加“必须恰好一行”的检查。

该上下文是本次库调用的观察载体；字段／引用保持不表示数据库堆不变，也不能把一次调用观察当跨调用一致历史。A4 的固定 plain two-value whole-execute 合同、正常 A6 和 V5 的真实活对象／事务／租约／API 历史保持显式提供者范围。行存在、耐久接受、实际资源及授权不由字段相等推出。

正确、deadline 遗漏、sandbox 遗漏使用同一完整 Model、Fixture 及原 helper 数学。错误版只改变实际 Std 参数，不导入正确 Std 定理；必须实际编译抽取并到达完整合同的否定。编译／导入／抽取失败和超时不是业务反例。

原内部效果方法只移入一个单方法 supertrait；签名、原方法正文、实际泛型 Create 函数正文保留。CreateAttachmentEffectsGrouping 给出旧／新完整字典的双向和全量化域对应，不是另一份协议模型。完整 World／事务字段仍透明，未手改生成正文或固定工具。

formal.py 的三视图包含所有实际文件的内核、规定同服务完整非 stale/partial 诊断、依赖与精确审计。native.py 及普通／格式／Clippy 验收交 ning。公共登记、输入／冻结、累计审计／覆盖／依赖和共享套件由主控单写。

证明机的独立开发入口保留完整三视图；`--prepare-only` 只供抽取与构建预检，不能计作完整检查。输出目录必须是本轮新目录，锁使用本批已有共享重锁：

```sh
python3 verification/experiments/stage1-protocol/create-standard-sql/formal.py \
  --output artifacts/create-standard-sql-development --lock /private/existing-proof-heavy.lock
```

ning 使用 `native.py --output /private/create-standard-sql-native` 运行同源四视图。
全套准入与证据交接以[证明工作流](../../../../docs/proof-workflow.md)和[本组件入口](../README.md)为准；此独立开发入口不产生共享验收封口。

独立开发结果、普通／原生、公共准入和完整 V3 分别验收；本包输入清单本身不表示任何执行通过。适用版本以精确原始结果和覆盖记录为准。真实 JSON／SHA、其余标准委托及对象／同事务连接、提交未知／重启、producer／driver 与 Resume 支持域继续单列。
