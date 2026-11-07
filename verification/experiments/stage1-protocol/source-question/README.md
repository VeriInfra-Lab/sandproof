# 来源观察请求的完整字段构造

[协议范围](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包采用真实 `snapshot::question_from_binding`。原 `question` 仍先对完整 anchor 调用原 `serde_json::to_vec` 和 digest，再调用唯一字段构造 helper；摘要计算、错误传播、字段复制与第二项索引的求值顺序保留。其它 snapshot 函数逐字保留。一般新旧等价尚未机器证明。

九份原数学模板及普通 String clone 模型逐字复用，摘要见 `reuse.json`，保留原 Candidate 命名以复用确切声明。`FullQuestionContract` 对任意已解码 anchor、limits、完整给定 binding，完整对应 anchor、node、host_bootstrap、第二项 resource、source 和全部 limits。原列表不足两项仍越界失败，不添加长度前提来隐藏此分支，不从其它 fields 推导 binding。正确/三错使用同一 Spec、Values、Examples 和完整合同；反例分别误取首项 resource、options 摘要、disk 限额作为 material 限额。

形式入口直接捕获真实三参数私有 helper，无业务替代算法或桥接根。固定 Charon/Aeneas 重新产生四份字面生成视图，不手改生成体或导入生成公理模板。普通 String clone 和 Vec 索引采用现有逐符号 A6 合同；没有新库模型。入口检查全部实际 Lean 文件、原内核、规定同服务的完整诊断和依赖审计，错误环境不导入正确 Core/Witnesses。

```sh
python3 verification/experiments/stage1-protocol/source-question/native.py --output artifacts/source-question-native
python3 verification/experiments/stage1-protocol/source-question/formal.py --output artifacts/source-question-formal --lock artifacts/.proof-heavy.lock
```

原生入口为每版保留独立源码和新 target。原版从原 ready/prepare SQLite fixture 导出一份不可变完整 SourceWork；原版与采用版都使用其相同 anchor/limits 经真实 `question`（含原 JSON/SHA），检查完整输出相等。采用版、三错与恢复另运行原八项完整字段/越界断言。有限回归不代替一般 JSON/SHA 对应。此入口是本机开发证据，ning 仍负责普通完整及正式原生验收。

公共 Lake、登记、冻结、依赖、覆盖和根导入由主控协调，两个候选 patch 不直接安装。旧 snapshot.rs 全文件冻结须保留其原适用版本，先核对原语句与依赖再重新验收，不能只换 hash。正常见证仅说明给定值可构造请求；不证明 binding 已计算、来源已授权、已持久接受或实际资源存在。标准 JSON/SHA 一般对应、实际 caller/load/save、同事务及提交未知/重启仍属 V3；V1/V2 意图与耐久发送、V4/V5 本地接受与结果、V6 物理事实、V7 认证/字节来源分别保留。完整 V3 18 要求/49 义务不关闭。
