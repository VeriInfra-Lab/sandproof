# V3：原 set_end 的首错与错误后状态

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包开发中，固定依赖 #1014 作者 `29a97e6441167ccbf0abd8f27655303c63e9925d`，
生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。不自动追随工程头。
本包保留原完整返回值及完整 Record 错误后状态合同；辅助前缀证明不能代替整个 setter。
生产、共享配置、根、登记和覆盖不在本包写入范围。

## 入口、规格与连接

`SourceSetEndSpec` 描述原 set_end：先检查 capture/action/worker，保留完整
observation.matches 及其 JSON 错误，再拒绝不匹配或 completed＋NoRequest。
没有 observation 时，已有 worker/completed 先拒绝，否则绑定完整 Full 并克隆完整 action。
已有 End 比较完整值；新 End 先编码并检查 16384 字节上限，随后才写入 capture.end。
最后调用原 receipt 并要求 Some。新 End 写入后的 receipt 错误保留已写入的内存状态，
普通 Err 不推出记录未变或回滚。

隔离候选把原只读提案前缀移到透明 `propose_end`；同一 Box::new 用显式闭包调用，
同字段可变赋值改为模式匹配且保留 CorruptStore 分支。completed/NoRequest 复用已证
透明守卫。完整 Full、请求匹配、action clone、receipt 和逐调用编码结果复用父规格；
没有复制替代业务算法。新增共享读取的正常借用、分配/析构及生产/staging 的一般连接
仍须明确采用，不能仅由对照测试推出。

`SourceSetEndProposal` 是只读前缀的规格与辅助合同；`SourceSetEndFactor` 证明其后接
原 install/receipt 与原完整 outcome 相等，包括所有错误和完整状态。
`SourceSetEndPrefixProof` 对所有原记录与 observation 证明真实抽取前缀对应；
`SourceSetEndPrefixValues` 复用父解码见证，`SourceSetEndPrefixNegative` 用同一合同
反驳真实错误前缀，并保留 Returned、合法 NoRequest 和 NeverReleased 控制。
`SourceSetEndOrder` 给出原完整状态模型的首错、写前拒绝及写后错误性质；其模型定理
尚不等于完整 setter 的 Rust 对应。

## 固定工具与未闭合范围

正确和错误前缀均由固定 Charon/Aeneas 真实编译抽取。`capture-prefix.py` 和
`validation-prefix.py` 保留完整源码/模式、原工具、原 IR、生成文件与父声明回读，
仅沿用原 29 项接口。视图只插入完整原生成声明，共有部分逐字核对后复用父定义；
不手改生成体或导入含自定义公理的模板。

前缀完整开发入口为 `check-prefix.py`，它调用未修改的原完整检查器。
输入 JSON 固定包含 `correct`、`wrong`、`reads`、`reads_audit`、`retention`、
`retention_audit` 六个本地只读位置：两份本包真实前缀捕获、#1014 正确项目/审计和原
retention 项目/审计。原 207 文件闭包及全部父审计均保留；新增根显式导入状态模型、
拆分等价和见证，错误根另接真实错误版与同合同反例。两视图各自检查全部实际文件、
内核审计、固定依赖及全计划/文件的重新物化，不提供跳过错误版或删减父义务的选项。
生成 Types 仅允许已核对的闭包源位置注释由 420 行移到 417 行；所有类型体和接口
完全相同，实际函数只允许删除那一处三行守卫。不作通用文本归一化。

```sh
SANDPROOF_ACCEPTANCE_ROLE=formal python3 \
  verification/experiments/stage1-protocol/unload-parent-link/source-set-end/check-prefix.py \
  --inputs /private/prefix-inputs.json --output /private/prefix-check \
  --lock /private/proof-heavy.lock --lake-packages /private/fixed-lake-packages \
  --lean-mcp /private/fixed-lean-mcp
```

`registration-candidate.json` 只提交本包前缀范围的公共登记候选，未自行采用到共享根。
完整 setter 的原结果/完整状态合同和准入仍单独保留，不由这个入口消除缺口。

完整 setter 的固定两项工具已成功结束，但原 30 项结构准入仍拒绝新增
`Box.Insts.CoreCmpPartialEqBox.ne`。候选需求见 `dependencies-candidate.patch`：
正常 Box 共享借用下向同一传入 PartialEq 字典的 ne 委托，保留完整值和原结果/失败；
不能擅改为 not eq。该需求交 V0/主控，未据此扩大准入或声明整个 setter 已证。
旧工具失败和候选诊断保留，不算性质拒绝。完整 setter 的对应、错误后状态实际连接、
完整检查器/公共登记及整套验收仍待闭合。

从已解码值开始。外层 save_capture_end 的同一 WorldTransaction、SQL 行/index/身份、
当前 World/撤销资格、未知提交/耐久与物理工作/FD/Writer/材料仍归各提供方。
32 字节值和编码接口见证不证明真实哈希、材料真实性或 JSON/TLS 全链。

## ning 原生交接

`native.py` 准备 original、adapted-original、allow-completed-no-request、
adapted-allow-completed-no-request 四套完整输入，各保留原 4906 文件和同一 66 条断言。
新增正常安装/完整重放、前缀缺失、完整 End 冲突、写前容量、写后回执错误和早拒绝。
桥只通过 cfg(test) 调用原私有函数，或在所属模块为夹具清空 End；没有替代算法。
长 worker 是明确的已解码域输入，不声称外层构造器、授权或父调用接受它。

预期正确两版各 66/66，错误两版各 65 通过/1 失败，唯一断言为
`completed NoRequest must be rejected before installing End`。
完整错误 setter 必须呈现更晚 CorruptStore 和已写 End，不能记成成功回执；
前缀的 WorkerEnded 反例不代替这条后段原生义务。构建失败不算反例。
两错误后各独立重编译 original 并恢复 66/66。当前候选四套输入仅准备/回读，
未编译或执行；旧候选输入不迁记为当前通过。

普通、原生和完整 Linux 入口交 ning；公共 proof_native_suite、整套和覆盖由主控
同批统一验收及严格内容复用。本包局部 Lean 开发通过不能代替这些回执。
证据位置和报告哈希按用户决定保留本地。
