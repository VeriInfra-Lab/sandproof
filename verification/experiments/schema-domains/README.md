# Status schema 家族的验证接入

本目录是 V0/#333 的独立复现材料。范围和证明状态以正式依赖登记、覆盖记录和
[#333](https://github.com/VeriInfra-Lab/sandproof/issues/333) 为准；本文件不建立第二份总计划。

## 证明对象和使用方法

生产函数是 `sandbox-internal/src/status.rs` 的 `StatusRequest::validate` 和
`HostStatus::validate_for`。`regenerate-schema-domains-lean.sh` 用固定原 Charon/Aeneas
直接抽取它们。没有复制验证算法，没有改生产路径或工具。生成库使用独立的
`SandproofSchemaDomainsRust` 命名空间。

`SchemaDomains.Model` 定义独立的完整字段表示和两个函数参数形式的合同；它不导入本线
生成 Funs。`Guard` 证明真实生成函数实现合同，并复用已证明的节点比较和公共表示。
调用者得到的是：

| 接口 | 已证明的同步决定 | 调用者仍需提供 |
| --- | --- | --- |
| 请求 validate | 接受当且仅当部署、节点、代际全部相等；不同则为原 Rejected | 预期节点确实属于原认证端点；不能把相等当授权 |
| 响应 validate_for | 接受当且仅当完整原节点绑定且 slots 在 1..=4096；否则为原 InvalidResponse | 原请求/响应和连接绑定；观察的时间、资源真实性及后续用途 |
| 完整字段表示 | 请求/响应投影相等推出原值相等，包含全部资源和时间字段 | 原库/类型版本和适用普通值域 |

JSON 中 slots 的可表示域是 u16 全域，包含 0 和 65535；这些值可以成功解码，但会被
后续原守卫拒绝。资源的四个 u64 字段可以为零，时间可以为 0 或 u64::MAX。
守卫不检查观察新鲜度、物理资源真实性、配额或业务授权。

## 字节接线的范围

复用现有 `JsonRequest` 的 SliceRead、原错误字典、输入游标、Node 编号/非零计数器叶子、
文档/容器结束和输出字段方法。标准 Serde 派生与 serde_json 引擎作为显式 A6 条件，
具体原 API、参数、类型、派生字典和结果传递按 B4 登记。自有编号、计数器和守卫不能
整体改成可信结果。

StatusRequest、HostStatus、NodeIdentity、Resources 都没有默认/可选字段，均有
`deny_unknown_fields`。对象字段可换序，重复/缺失/未知字段拒绝；固定标准 struct
派生也接受按声明字段顺序的数组，不能把它误写成“只接受对象”。编号按 UTF-8 字节
计数，合法值仅由 ASCII 字母、数字和 `-_.` 组成，非空且最多 128 字节，不做规范化。

`verification/schema-domains-environment.json` 登记完整字段、原调用者、预算及其
提供方，并分类其它内部 schema 的可复用部分和待连接部分。Status 是本批唯一实现家族；
Probe、LifecycleResponse、WindowClose、模板/发布等不能据此自动认领已证。

[有限源码分类](catalogue.json)另逐项记录六份接口文件中的 22 个可序列化主类型、
完整字段/属性和十个实际字节位置；它是绑定源码的复用分类，不是编译器调用图或证明。
特别区分：ContractVersions 只序列化且包含 i64；readiness 的响应实际是 Value；
本地数值 ID 与协议字符串 ID 不同；artifact/readiness 的 node 绑定请求方身份，
Status 的 node 则由原调用约定选择，字段相同不能替代端点角色和授权证明。

验证 crate 的四个小函数直接调用原 `from_slice`/`to_vec`，使用真实生产类型。
其中 `response_encode` 是普通 Vec 的参考渲染入口。原服务器 `json_response` 实际调用
`evidence::encode_bounded`，其私有 Writer 的 checked_add、限额、分配失败、错误和部分
输出另有自有连接义务。比较测试不替代该一般性对应证明，不能把它整体作为 B4 假设。
这项待连接在环境登记和 #333 中保留。

本批的字节定理只覆盖三个真实普通值调用位置：客户端请求 to_vec、服务器请求 from_slice、
客户端响应 from_slice；另有 HostStatus 的参考 to_vec，不能作为服务器 Writer 的证明。
服务器的有界输出是本线明确保留的 V0 公共缺口，未因本批范围转为下游业务已负责。

本批只考虑固定 SliceRead/普通内存值及正常分配下的 Vec 渲染；任意 Serialize 回调、
有状态设备、资源所有权/自定义 Drop、OOM/abort、unwind 和整个异步 shell 不被这些定理覆盖。
有限成功和错误历史不推出公平调度或实际请求最终完成。

## 验收和机器分工

所有普通测试、fmt、Clippy 和原生错误断言在普通验收主机执行。专用证明主机只运行
正式所需编译、MIR/Charon/Aeneas、Lean、形式反例、物理文件诊断和审计；其新增源码、
target、.lake/build、TMPDIR 和证据全部使用任务指定的数据盘私有目录。固定工具/依赖只读复用，
不复制另一套 CI 环境、不清理 .lake。

`tools/check_schema_domains.py` 核对真实守卫根、透明自有函数、完整 LLBC/嵌入源码、
目标、生成物和实际审计清单。只对已登记无序 short_names 表作公共准入器允许的规范化；
不丢失 AST 字段、调用、清理或顺序。不接受编译失败作为语义反例。

两个隔离错误版分别把完整节点比较改为只看 node_id，以及删除 slots 上界。原 Rust
必须先新编译，再由原工具抽取；同一独立 `RequestGuardContract` 或
`ResponseGuardContract` 必须拒绝该实际生成函数。正确和错误版使用同一原生断言。

## 可复现入口与集成门槛

普通验收主机使用已分配的稳定 CARGO_TARGET_DIR、固定 Rust、离线 Cargo 缓存和私有输出：

```sh
bash scripts/verify-schema-domains-native.sh --output /private/status-native
```

它从本次干净提交分别导出正确版与两个真实错误版，检查新编译二进制、相同原生断言、
原状态接口测试、fmt、严格 Clippy 和全部本线准入回归，不运行 Lean 或翻译器。

形式主机的正式模块入口为：

```sh
bash scripts/verify-schema-domains.sh
```

该入口先核对本线冻结闭包，并强制消费原 acceptance_receipts 中已封口的
`schema-domains` 原生 case；case 尚未由统筹注册/封口时应失败，不能以本线裸
complete.json 替换整批收据。随后使用固定原工具、完整新 LLBC 和逐字节生成物比较，
检查三个真实版本及实际 Lean 文件。所有临时目录继承任务的 TMPDIR；在 a6000 上设置到
`/data1/zn` 本线目录。固定包只读复用，各源码版本有独立可写 Lean build，Rust target 保持稳定。

单独复现形式组件（不代替正式入口的原生收据或完整项目验收）：

```sh
bash scripts/python-entry.sh verification/experiments/schema-domains/replay.py \
  --tool-root "$SANDPROOF_PROOF_TOOL_ROOT" --output /private/status-formal-component
```

`Bytes.lean` 复用原叶子、原错误字典、原输入切片与结束顺序，提供合法输入域、
完整字段序列化、正常分配条件下的往返和原 1024/2048 预算。JSON 引擎/标准派生及
具体 API 符合性始终是显式参数；本线源码/完整 AST/字典/库核对支持条件采用，
不把第三方引擎或异步外壳称为已机器证明。叶子产生的原自定义错误先于后续文档结束；
外层 parser 的位置补全仍归固定 A6 引擎条件，不能把叶子的 0/0 位置当作所有最终错误位置。

只有模块重放、原生记录、总入口、完整双机验收和最终 CI 都通过并集成后，才报告本批最终完成。
共享 Lake/总审计/native case/批次/实际文件服务/use-inventory 与权威文档由统筹接线；
本线不另造总入口或进度标准。Status 外的 schema 与服务器自有有界 Writer 仍按前述边界保留。
