# 原 restore claim 到完整恢复工作的值连接

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) ·
[接口条件](../../../../docs/formal/dependencies.md) · [工作流](../../../../docs/proof-workflow.md)

固定 integration 9c50da7cc9391518514f14405e6c779e5c5cbe2c。对象是当前
controller-service 中 source_work/restore.rs 的原 work，同步接收已解码 SourceWorkRecord。
生产、协议和 V1/V4 实现只读；正确视图不替换原函数或增加业务 opaque。

规格只使用原工具生成的真实类型，保留 restore claim、source requirement、完整 full capture、
manifest digest、seal action、本地 operation 和 generation 的全部值。首拒绝依次为
缺 restore 的 Conflict、缺 capture 的 CorruptStore、material_use=false 的 UnresolvedAction，
随后才读取 completed 和 action；后两项缺失各为 CorruptStore。普通值克隆与 Option/Result
沿已登记 A6，原自有逐字段克隆继续由当前生成体连接；未保证物理分配和异常展开。

after_commit 与 from_claim 是这里两个保留字段的真实构造函数；名称与字段相等不证明
已提交、授权、物理资源或恢复执行。原 source record、当前耐久事务和历史可达性由其
业务提供者连接。本包也不提供公共 Resume 请求的额外编号约束。

已有协议克隆数学由 [reuse.json](reuse.json) 指向原唯一模板，只按独立生成视图明确更换
导入和命名空间，不维护第二份协议模型。自有 capture/source/action 的克隆证明保留每个
完整字段；FullWorkContract 对所有原记录给出完整返回值和首错，不以有限样例替代。

正常值见证与两个真实 Rust 错误版使用相同 FullWorkContract：删材料使用守卫，或只返回
native generation 的低八位。共同正常值保留37，第二错误版以293的完整字段反驳；不改变
公共 Resume 编号规则。普通输入类型非空由原实际类型构造，不以不可能前提取得结论。
这些是已解码值的见证，不签发可达事务或物理资源事实。原生正常对照另复用既有真实
SQLite、capture、Resume 夹具及受控 Host 端口；七项断言核对完整返回值和首错。

在获准 CPU 上持原重锁运行局部开发入口，保留新目录及独立构建：

```sh
python3 verification/experiments/stage1-protocol/restore-work/formal.py \
  --lock /private/existing-proof-lock --output /private/restore-formal
# 普通验收提供方运行完整原生组件；本机开发对照也须由操作者持原重锁。
python3 verification/experiments/stage1-protocol/restore-work/native.py \
  --output /private/restore-native
```

formal.py 三视图重新编译/捕获原函数、固定 Aeneas 翻译，比较全部原生成体，再由原
实际文件检查器执行内核、规定同服务完整诊断和依赖审计。错误视图不导入正确 Core/Early，
保留同一正常见证。native.py 使用全新 target，四视图保留原七项断言，错误版须到达
对应性质失败，并逐字节恢复；格式、严格 Clippy、源/manifest/新编译协议库及测试程序均核对。

这里仍是候选的执行入口，具体通过范围只按 #635 的精确原结果登记。普通/原生和主控
公共/共享验收分别报告；不能只跑旧 V3 根报告本包通过。公共登记、Lake、冻结与覆盖只交
主控候选，不并发写。[登记候选](public-registration-candidate.patch)基于9c50的正式表，
增加一个普通组件及三视图形式任务，固定 Create 的12包不改；若同批采用在途标准包，主控
按 +1 native、+7审计模板/+49声明合并计数，不能重复套用各包绝对计数。
