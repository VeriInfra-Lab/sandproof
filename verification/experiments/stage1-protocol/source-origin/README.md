# 来源操作绑定的完整值校验

[协议范围](../../../../docs/formal/03-protocol-inbox.md) · [依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包采用真实 `snapshot::validate_execution_origin` 的一个小拆分：原位置改为调用返回 bool 的 `source_origin_epoch_matches`，其函数体保留原 epoch/continued identity match。原 Rust 求值/查询/拒绝顺序保留，其它 snapshot 函数逐字不变；不声称新旧实现一般等价已机器证明。固定后端原159函数视图的 `Decidable epoch_matches` 生成类型失败及完整服务诊断保留，新160函数视图由工具重抽取，生成体不手改。

`FullOriginContract` 对任意完整已解码 World、Inbox、Anchor 和调用者给定的 session，对应原全文来源查找、原 session 与 local operation、plan/record、Create/Resume 类别、generation、epoch/continued identity、VM 和 restore_from 形状。保留先查找/索引，再 plan/record，再类别和 epoch，最后全部字段的顺序；查询、回调的错误/效果或不返回按原样传播。表查询通过完整 native key 比较绑定到现有 V0 `LoopCallbacks` 证明，保留第一匹配和完整值，不假设键唯一。

正确和三错共用完全相同的 Spec、Values、WitnessSupport、Examples。三份真实 Rust 错误版只漏原 session 索引、generation 或 VM 比较；分别实际编译/抽取并在同一完整合同上给出反例，错误环境不导入正确 Core/Witnesses。正常接受及可接受输入存在性独立给出。任意输入对应依靠通用定理，具体值只用于正常见证和反例。

只复用原 A6 普通 String/Option/Vec 操作、字典比较和 bool::then_some，以及既有普通错误值载体，没有新增库模型或公理。原通用迭代和字节/数组/Vec 引理正文逐字重绑定，见 `reuse.json`；旧辅助文件依赖独立视图，原请求类型又与当前视图产生全局判别式重名，因此不直接导入其生成类型或修改工具。V0 原表查询合同/证明直接复用，自有调用和完整字段对应仍在本包。

```sh
python3 verification/experiments/stage1-protocol/source-origin/native.py --output artifacts/source-origin-native
python3 verification/experiments/stage1-protocol/source-origin/formal.py --output artifacts/source-origin-formal --lock artifacts/.proof-heavy.lock
```

原生入口为原版/采用版/三错/恢复版各建立独立源码和新 target，同一组12项断言从原 ready/prepare 真实 SQLite 夹具获得数据。只在私有测试副本添加一次转发到真实私有方法的薄根，不提交额外生产 API。正常/省略字段/全文封套变异拒绝、实际链接库/程序、严格 Clippy、格式及恢复均须核对。它是本机开发入口；ning 仍负责普通完整和正式原生验收。公共 Lake/登记/冻结/依赖/覆盖仅提供候选，完整共享结果另列。

本方法不从 anchor.request 自己计算给定 session，不额外检查直接来源的 owner，也允许两个相等 VM 同为 None。继续来源比较完整 identity 的 key/epoch/generation/vm，不能加强为原 plan epoch 必须等于 native epoch。正常形式见证是值域见证，不等于整个 World 的可达性或部署结果；原生正常夹具和物理事实也分别报告。actual getters/history 透明保留在规格中，具体 historical_identity 策略、自有外层 validate_record、JSON/SHA/SQL 和同事务来源仍须连接，不藏成可信 whole-handler。

字段相等不推出授权、持久接受或资源存在。V1/V2 当前意图与耐久发送、V4/V5 本地接受/结果及提交未知/重启、V6 实际资源、V7 认证/字节来源分别保留。其它 snapshot.rs 全文件冻结维持其原执行身份，主控先核对真实未变语句和精确依赖再协调验收，不能只改 hash；完整 V3 的18要求/49义务保持开放。
