# 原恢复继续退出路径的期限值校验

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../docs/formal/dependencies.md) ·
[工作流](../../../../docs/proof-workflow.md)

固定 integration `9c50da7cc9391518514f14405e6c779e5c5cbe2c`。直接捕获原
`source_work::restore::continued_exit::lifetime_matches`，正确视图不更改生产函数。
输入为已解码的原 NativeCreateSnapshot、完整 CurrentReadyFacts 和 i64 当前毫秒；
不设业务 opaque，不新增库模型，不导入外部占位模板，不修改工具或生成体。

FullLifetimeContract 使用实际生成类型，覆盖任意输入的完整布尔返回值：缺 lifetime
或 started 返回 false，五个时间均须非负；四个保存期限字段须与原有符号整数的
u64 转换相等，保存的 observed_at 不晚于 now，started 不晚于 now 和启动期限，
now 严格小于 deadline 与 max_deadline。原实现没有要求保存观测不早于 started，
没有要求 deadline 小于等于 max_deadline；规格不额外增加这些条件。

Core 对原生成体证明完整合同、接受/拒绝充要关系和总性；复用原 Replay.ok_and
短路数学，不维护第二份协议模型。共同正常值与有限 cast 事实仅作见证，不代替
任意输入定理。正确边界页与两份错误页分开：遗漏 max_end_at 字段比较、将到期
严格小于改为小于等于，均修改实际 Rust 副本并否定同一合同，保留相同正常值。
错误视图不导入正确 Core 或 Boundaries。

这里仅提供期限字段的值关系。其它 native 字段不会因期限相等而获得认证；本包
不证明这些值来自当前同一事务，不证明当前进程存在、死亡、授权或耐久接受。
完整 restore::validate_record、exit::eligible、外层 authority/producer/driver、
JSON/TLS、材料闭合及未知提交/重启仍是独立义务，依原 V0/V1/V5/V6/V7 提供者连接。
本包不更改公共 Resume 允许编号空洞的协议语义。

持原共享重锁、在获准四 CPU 上执行局部开发入口：

```sh
python3 verification/experiments/stage1-protocol/restore-lifetime/formal.py \
  --lock /private/existing-proof-lock --output /private/lifetime-formal
# 原生入口不重复获取父锁；本机开发由外层持锁，ning 由共享套件协调。
python3 verification/experiments/stage1-protocol/restore-lifetime/native.py \
  --output /private/lifetime-native
```

形式入口重编译、重抽三视图并核对字节一致的原生成体；全部实际文件经原内核、
规定同服务完整非 stale/partial 诊断及标准依赖审计。独立工作树和私有视图各有
独立 .lake/build，工具、缓存和失败证据保留。原生四视图使用相同十五项断言，
分别新建 Cargo target，检查原源码、manifest、新编译协议库与服务测试和完整失败集；
格式与严格 Clippy 同样必需。两错后逐字节恢复原源码，最后再执行正确断言。
原生 fixture 是已解码值，不是 SQLite、网络或 VM 的事实见证。

通过状态只按 #635 的精确原结果维护；局部开发不签发 ning 或共享正式验收。
公共登记/Lake、依赖、覆盖仅有候选补丁，交主控单写采用，不能跑旧 V3 根报告
本组件通过。候选基于9c50，新增一个原生组件、一个含三视图的形式任务、五份
含审计模板/二十项声明；固定 Create 十二包不变。同批采用其它在途包时合并增量，
不要重复套用各包的绝对计数。

[公共登记候选](public-registration-candidate.patch) ·
[依赖候选](dependencies-candidate.patch) · [覆盖候选](coverage-candidate.patch)
