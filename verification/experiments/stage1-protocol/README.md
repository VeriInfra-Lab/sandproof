# 第一阶段同步协议：真实 Rust 到实际证明文件

[文档目录](../../../docs/README.md) · [V3](../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../docs/proof-workflow.md) · [适用版本与结果](../../../docs/proof-coverage.md)

本 crate 只透明调用生产 `sandproof_protocol::lifecycle` 方法。固定原 Charon/Aeneas
分别生成 request、manifest、carriers 三个视图；不手改生成算法，不增加翻译器规则。
入口代码和候选冻结存在不等于完成验收，实际通过范围由同一候选的原始记录确认。

## 接口、保证和限制

| 入口/证明 | 可以复用的结论 | 仍需调用者提供 |
|---|---|---|
| `request_validate`、Validation | 已解码请求的全部成功条件及六类首错；Create/Unload/Resume/Renew/Destroy 使用真实分支 | 原 Serde/字节绑定、认证、当前账本、操作授权、资源和生命周期条件 |
| `replay_matches`、ReplayEquality | 任意两个完整请求值相等时且仅此时返回真，包括全部 options/摘要/身份/版本/操作字段 | 不从相等推出请求合法、已授权或持久重放闭合 |
| `accepted_validate`、Accepted | 原独立 context 的 center/deployment、内层请求、retention slot 形状、accepted 版本关系与首拒绝顺序 | context 确实来自同一已认证连接；retention 窗口当前可用、账本事实与 handler 责任 |
| `recorded_matches`、Recorded | 完整原 intent 相等、允许的版本关系、current 与版本相等性的关系 | matches 不推出 validate 成功；terminal/current 的账本真实性、耐久回执与释放资格 |
| Carriers | CaptureId 构造/get、零/最大值拒绝、正常见证；32 字节值完整往返 | 不计算哈希，不推广成所有角色构造器已证 |
| Manifest/LoopWitness/ManifestWitnesses | 首个格式/capture 绑定拒绝与实际生成循环/算术、一个完整正常清单见证 | 不是任意 profile/文件集、物理快照、文件安全或恢复资格的证明 |

唯一请求规格在 `ValidationModel`，全文相等在 `ReplayContract`；没有长期保留第二份
CurrentEnvelope 请求模型。Retention 只检查 slot 为 1..4096，不额外要求 key window
等于 retention window；Renew 的纯形状守卫也不代替截止时间和操作授权政策。

普通值、正常分配及固定翻译器的信任范围沿用原 Aeneas/A6。Request 的四个外部函数
精确为 String.eq、Vec.is_empty、Bool.ne、Option.eq；Option 的 Some/Some 调用原字典并保留失败，
封套实际实例为固定 U64。Manifest 保留原七个字符串/容器接口，Carriers 没有新增外部函数。
模型与固定 Rust 标准库符合仍是显式条件，详见[逐符号依赖](../../../docs/formal/dependencies.md)。
模板中的 `axiom` 仅作为原工具证据保留于验收产物，不导入正式证明。
OOM/abort、资源 Drop/unwind、完整 async shell、TLS/bytes/Serde、handler/账本/资源退出均不因此已证。

## 正确版与真实错误版

`formal.py` 显式登记并检查 Protocol 目录的每份保留证明，漏文件或出现未登记文件即拒绝；
每个物理视图解析完整本地导入闭包，逐文件内核检查，再经原严格服务核对完整、非陈旧的诊断和准确审计。
三个生成库有同名 discriminant 辅助声明，保持独立物理视图，不把实现全部导入同一个公共根。
公共消费者只组合独立 Model；公共根未导入不能成为省略实际文件的理由。

真实错误版由原生产 Rust 的唯一表达式修改生成，不能手写 Lean 错误算法：

- same-incarnation：将 `target <= source` 改成 `<`。保留顺序 pair 反例，并检查同一完整请求守卫合同的否定。
- key-only-replay：只比较 operation key。保留原 pair 反例，并检查同一通用全文重放合同的否定。
- skip-capture-binding：使原 capture 绑定守卫失效。保留实际 loop0 分支、六处私有 helper 引用适配及全部辅助目标。
- session-only-envelope：将完整 intent 比较改为 session 比较。输入形状正常、session 相同但 capture 改变，明确否定正确版同一 `FullRecordContract`。

独立合同和见证逐字相同。编译失败、导入失败、没有运行原断言或未知超时都不能计作反例。
普通测试侧保留原源码、编译产物和日志；每变体真实库与测试均须 `fresh=false`、绑定原 manifest/src，
错误版触发同一断言后还须恢复原源码、重新编译并让同一断言通过。只清自有包变体缓存，保留其它依赖。

## 同一源码与回执

`archive.py` 仅为本组件生成确定性 tar：完整已提交 tracked 文件集、Git blob 内容、文件类型和执行位
均与磁盘核对；拒绝未提交/未跟踪工作、链接、重复或逃逸路径。只固定 uid/gid/owner/mtime 等传输元数据，
不删选源码、配置、Cargo、fixtures 或规则。归档前后再次核对整个输入。B 批草稿不能被工作树打包带入。

`component.py` 在普通测试机运行原 native/negative CLI，再通过原 `native_case` 回执机制登记；
证明机必须先取得当前完整共享 native seal，不能拿裸 `success` 替代。仅此新 case 额外保存一个
明确的原始 `source.tar`，沿原 `receipt.files` 复制和逐字 SHA 校验，缺失、链接或损坏均拒绝；
不传 Cargo target 或二进制，也不扩大成复制任意归档/目录。

原生记录的完整清单、source_sha 和文档结果只属于它自己的历史 tar/提交；证明机另建当前干净候选的
完整 tar，核对相关 Rust、规则、配置、合同及文件类型/执行位仍相同。无关报告或同树新提交可以复用原生组件，
不因 commit/tree/整仓文档改变而重跑所有模块；当前文档仍由强制 functional 入口检查。
这里的复用限顶层 README/普通 docs 等未进入 native 规则的报告；本组件说明位于原 rules 范围，
修改仍按原机制使记录失效，不因此扩大规则排除。
证明侧在自己的工作目录按已核历史 tar 加固定唯一 mutation **重建等字节错误源码**，核对原/错误 SHA，
再将同一修改作用于当前证明副本。这不冒充传来了原 `.rs`，也不向封口回执补文件；
原 `.rs` 和二进制继续保留在普通测试机。冻结 native_rust_inputs 与同性质/同见证核验均保留。

## 验收入口

先在 普通测试机 的干净已提交隔离候选、原固定普通环境和已分配 `CARGO_TARGET_DIR` 中运行完整原生入口：

```sh
bash scripts/python-entry.sh tools/proof_native_suite.py run --output /private/native-evidence
```

`stage1-protocol` 是其中一个原生 case；原两包全部测试、显式七组 integration 发现、Clippy、格式、
文档及四种真实错误版/恢复检查均保留。不要另跑裸七组来代替此入口，不能在证明机跑普通测试。

将原完整封口与所有 JSON/log 证据交到 证明机，按工作流准备同一 Git 候选、固定工具、独立 `.lake/build`：

```sh
export SANDPROOF_NATIVE_EVIDENCE=/private/native-evidence
bash scripts/verify-stage1-protocol.sh
# 整套入口继续保留，组件通过不能替代整套通过：
bash scripts/verify-controller.sh
```

新模块清单与 `frozen.json` 绑定实际源、生成物、模型、检查器、全部审计和导入闭包。
request 四根变化时，三个 profile 的完整原始 IR 和生成字节都必须重新比较；只规范化原检查器允许的
无序 short_names，不删除有序 AST、源码或选项。候选冻结必须再经原入口和全部反例验收，不能刷新摘要后
宣称通过，也不能把旧失败记录改写为成功。

`formal.py` 原 `--source-archive/--native-mutants/--native-source-archive/--case` 与
`replay_formal.py --property replay|validation` 的局部诊断入口继续保留；它们不伪造完整 native seal，
结果属于局部形式检查，不能代替上述完整入口。精确候选、未完成和环境阻断由 PR/原始记录报告。
