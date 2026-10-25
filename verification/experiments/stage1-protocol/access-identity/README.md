# 访问身份的结构校验切片

[组件](../README.md) · [V3](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

固定 #686 `39577ca1e61f4daa4ba4bbda7db94bf087bdaef0`，生产只读；本包独立于后续观察值切片。
薄根直接调用实际 `LifecycleAccessIdentity::validate`，同时保留原四个 Request/Accepted/Recorded 根。
固定原 Charon/Aeneas 实际生成私有 Request 视图，只复用原四项 String/Vec/Bool/Option 外部模型；不手改生成体或导入 axiom 模板。

从已解码值对应协议版本 4、独立 context 的完整 center/node、两种 deployment 关系及 accepted_version 非零。
保留五类首错：UnsupportedVersion、ForeignCenter、ForeignNode、ForeignDeployment、InvalidAcceptedVersion。
`AccessModel` 定义正反共用的接受条件和精确首错，`AccessBindings` 给任意值对应、接受/拒绝充要条件及完整 context 保持；有限合法和拒绝见证不替代一般定理。

原守卫不查 journal 的 session/owner/window/sequence/incarnation 归属，通用字段替换引理明确这点。
外部 incarnation 不是本地 generation；字段校验通过不能推出 Access 授权、当前运行、有效寿命或实例存在。
V7 提供独立认证 context，V1/V2 提供当前意图，V4/V5 提供原 journal 的全文映射、当前阶段和 Access 权限，V6 提供实际运行事实。
完整收件、JSON/TLS、撤销和旧请求竞争仍须分别连接。

四个实际 Rust 错误版分别仅比较 node_id、遗漏 center、允许零版本、倒置 center/node 首错。
它们使用同一完整结果合同和见证；实际编译/抽取/反例与失败证据分别保留，编译失败或超时不算性质拒绝。
局部定理、普通及错误版原生检查、共享完整验收分别报告，精确执行与输入闭包随 PR 记录。

形式入口须先取得共享重锁，使用本任务分配的固定工具依赖缓存；每个 case 有独立 `.lake/build` 和 Rust target：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/access-identity/formal.py \
  --packages /private/task/.lake/packages --output /private/access-formal
```

入口沿原 `formal.check_actual` 检查完整实际导入闭包、逐文件内核和规定同服务客户端的完整非 stale/partial 诊断及精确依赖审计。
ning 使用原 archive.py 从同提交生成的完整源码归档：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/access-identity/native.py \
  --source-archive /private/source.tar --target /private/access-native-target --output /private/access-native
```

原生入口保留正常八项用例、格式/Clippy，以及四个错误版各自 baseline→mutant→restored 的同一断言。
核对两个自有包和测试的 source/manifest/fresh=false、实际二进制及完整源码恢复；不自行签发共享 native seal。
公共登记/生成视图、原 verify-stage1-protocol、共享验收、冻结与覆盖由主控/ning 同批处理；旧根通过不覆盖本包。
