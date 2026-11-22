# V3：原继续请求的完整摘要绑定

[工作流](../../../../../docs/proof-workflow.md) ·
[父六字段接口](../source-end-accessor/README.md) ·
[逐符号合同](../../../../../docs/formal/dependencies.md)

固定生产 b0d791ccc4d4e058b710dbf8f46766652b2853df、父 #1018
2bb7a92f5dc61704d906dc81c3425219901fa4ee，不自动追随工程分支。
生产只读；公共配置、根、原生登记、冻结和覆盖只交候选，由主控采用。
本包是有条件开发切片，两项新具体 JSON/SHA 使用点尚未采用，正式准入仍拒绝。
清单、命令或 draft PR 存在不表示检查已通过。

## 精确性质与表示

真实 `OriginalContinueWorkRequest::binding_digest` 的输入为已解码完整值。
`bindingInput` 按原顺序保留 identity、完整 full、完整 source、完整 end_receipt、
完整 network_policy、file_policy。它对 Question 单射；这不是 JSON 或 SHA 单射。
`outcome` 保留一次编码的完整 Result：外层失败/发散和原 JSON 错误先返回；
仅编码成功时将全部返回 Vec 的原 slice 交给 SHA 接口，再构造32字节摘要值。
没有域前缀、长度守卫、授权、落盘或真实资源存在的附加结论。

`Contract` 是任意完整请求和允许接口结果下的整个原返回性质。
`CompleteContract` 同时要求完整六字段传递和原 `Contract`，正反版本共用它。
实际未拆分入口保留；仅在隔离源码将该次 JSON/SHA 调用提出成具体薄接口。
第二生成视图将原元组表达式移动一次到透明 `binding_input`，不复制算法、不克隆借用。
两个视图都重新编译/抽取；对任意请求和接口参数证明拆分前后相等。
正常版通过同一完整合同；错误版只把原 file_policy 换成零值，实际编译、抽取，
由完整输入部分反驳同一合同，保留零值控制和任意其它完整字段。
这不声称错误版的所有摘要都不同，也不用假哈希或 SHA 注入性构造反例。

原 network_policy 替换为 DenyAll 的真实错误版继续保留独立未拆分抽取与原生断言。
它的常量引用在透明元组返回探针中触发固定 Aeneas 限制，不能算形式性质拒绝；
本包没有据此声称该错误版的完整形式反例已完成。编译/翻译/导入失败和超时均不算反例。

## 条件和提供者

父完整213文件/1199审计、retention26/127及所有原业务生成体继续保留。
新四视图须保留117个完整类型、191个父函数；未拆分视图192函数、元组视图193函数。
各正反对仅一个完整业务函数不同，全部外部接口相同。生成体原样插入，不导入带 axiom 的模板。

旧29接口不放宽。新增的 `encode_continue_work_binding` 和
`digest_continue_work_binding` 仅是候选具体使用点：

- A6 JSON：固定派生、实际六元组 schema/属性、原借用对象、完整值、原返回及失败域；
- A9/A6/B4 SHA：同一次编码返回的完整原 slice、有效长度小于2^61及正常完成域；
- 生产到 staging：父普通 Vec/Global 正常借用和不可观察释放条件，原对象及调用接线。

这些条件以显式参数保留，复用原 digest 提供者，不另造 SHA 语义。
`capture.py::admit` 始终明确拒绝这两项未采用使用点；`check.py` 只检查条件开发闭包，
即使完成也输出 `interface_adoption_complete=false`、`public_acceptance=false`。
具体库符合性和采用归主控/V0，真实原生接线归 ning；不把整个 Serde、服务或事务设为可信。
实际派发、当前意图/权限、同一 World/SQL/父事务、提交未知/耐久、V6物理事实、V7认证
仍归原提供者。没有完整 JSON/TLS、持久收件或系统闭合结论。

## 可重现入口

固定工具、目标和 Cargo 锁沿用父包。证明机按工作流取得共享重锁；
以下输出目录必须为新目录。两类驱动都在工具成功后因未采用接口而明确拒绝，
原始完整源码、IR、生成文件和日志仍保留；预检拒绝不等于性质拒绝。

```sh
python3 capture.py --variant original --output /private/inline-original --tools /private/fixed-tools --lock /private/heavy.lock
python3 capture.py --variant file-policy-as-zero --output /private/inline-file-policy-as-zero --tools /private/fixed-tools --lock /private/heavy.lock
python3 capture.py --variant network-policy-as-deny-all --output /private/inline-network-policy-as-deny-all --tools /private/fixed-tools --lock /private/heavy.lock
python3 tuple.py --variant original --output /private/tuple-original --tools /private/fixed-tools --lock /private/heavy.lock
python3 tuple.py --variant file-policy-as-zero --output /private/tuple-file-policy-as-zero --tools /private/fixed-tools --lock /private/heavy.lock
```

本例命令相对本目录，实际由固定 Python 入口/证明机角色调用。
captures JSON 必须含 `inline/original`、`inline/file-policy-as-zero`、`tuple/original`、
`tuple/file-policy-as-zero` 四个原始目录。检查输入 JSON 必须且只含 captures、reads、
reads_audit、retention、retention_audit 五项。实际位置仅本地保存。

```sh
SANDPROOF_ACCEPTANCE_ROLE=formal python3 check.py --inputs /private/inputs.json --output /private/checked --lock /private/heavy.lock --lake-packages /private/fixed-packages --lean-mcp /private/fixed-lean-client
```

完整条件开发入口必须严格回读全部4906源码/模式、固定工具、原日志、IR、全部生成文件及父证据；
检查所有220个实际文件和1222项唯一审计（23项新增），同服务诊断须完整且无stale/partial，
核对十个固定依赖，再在新目录逐字复现整个计划、输入和全部文件。局部候选通过不代替此入口。

## ning 原生和公共验收

`native.py --variant <name> --output /private/new-input` 只准备源，不编译、运行或恢复。
八视图见 native-candidate.json：原生产、两个真实错误版、对应三个薄适配视图，
另加正常/文件策略错误的透明元组视图。每套保留同一61项父断言及两项新增断言，共63项。
三个正常视图须63/63；五个错误视图须62通过/1失败且只到达对应完整字段断言。
每个错误视图后独立恢复 original 源码和 target，重编译业务库与 libtest，再全部63项通过。
不能仅换测试二进制；核对同 crate 的 manifest/src/fresh 和完整源码。
运行命令、受影响检查、真实回执和失败恢复由 ning 按工作流同批执行。

registration-candidate.json 和 coverage-candidate.patch 只交公共登记/覆盖候选。
共享 proof_native_suite、verify-controller、verify-stage1-protocol 仍由主控统一采用并验收；
不把本包条件开发成功升级为正式准入，不重复整套共享步骤。
原始证据位置和报告哈希只保留本地，不追加到公开 issue 或 PR。
