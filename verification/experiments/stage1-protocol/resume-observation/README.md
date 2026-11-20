# 完整 Resume 观测值合同

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

固定 Continued-Unload 工程候选 `5e9d41b3252e42df1bb7d6e775a59ec43d745124`；生产只读。
复用 #691 `4f52d972a368596c0a9d4a8fff8913e757dfc778` 的证明依赖，旧证据仍只适用于其原源码。
本次语义迁移须重新抽取和检查；依赖快照或源码摘要更新本身不是验收。
本包对应真实 `ResumeObservation::matches_request`，复用现有完整请求/封套与 ReadyFacts 合同。
`ResumeModel` 包含原请求全文绑定、原/当前封套和全部后继操作/readiness 分支；
`ResumeBindings` 对任意输入连接固定工具的真实生成体，给精确结果、接受/拒绝充要关系和全文/封套保持。
`ResumeValues` 是正反共用的合法值与完整模型拒绝域；`ResumeExamples` 覆盖正常和分支边界。

没有加强实际守卫：StateLost 可作为原请求、Renew 或继续 Unload 的值形状；
继续 Unload 仍须绑定原 Resume 的目标运行代、更新版本、不同完整 key 和原/当前封套。
`continued_state_lost_accepts_iff` 给出这一分支的精确接受条件，合法见证和错误来源代的反例复用同一合同。
该值检查不验证已保存 OriginalContinued，亦不验证 BeforeAdmission/AfterCapture 的来源资格或同一实例确定退出。
这些条件分别由 V4/V5/V6 的实际提供者落实，不能从消息超时、字段相等或枚举标签推出。

负期限仅在 Renew 的 Observed 分支拒绝，Pending/StateLost 不在这里检查其非负性。
不同 current 请求的其他操作只接受更新版本、不同 key 与 NotCurrent；同一请求不接受 NotCurrent。
retention 仅比较完整原/当前值并检查原 slot 形状，不要求其 window 等于 key window。
这些不证明状态声明为真、已持久接受、候选实际死亡、材料完整或被授权。
V1/V2 提供原/当前意图与耐久发送，V4/V5 提供独立 journal 接受与状态/StateLost，V6 提供实际候选/材料，V7 提供认证来源。

四个错误版逐个修改真实 Rust 副本的唯一表达式：session-only 原请求、遗漏当前 retention、在 StateLost 时绕过 continued_request、遗漏续租目标代。
旧“允许 StateLost Unload”变异已成为合法行为，历史反例保留在 #691 的原提交，不在新语义下继续计数。
正确版和全部错误版使用逐字相同的完整模型与见证，必须实际编译/抽取并达到反例；编译、导入失败或超时不算性质拒绝。
复用 `observation-bindings/extract.py --profile resume` 的原八个透明根和固定原四项外部模型；
只安装工具实际输出，不修改生成体或导入 axiom 模板。每个 case 独立 Rust target 和 `.lake/build`，保留缓存与失败证据。

形式入口（先取得共享重锁；私有固定工具依赖缓存）：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/resume-observation/formal.py \
  --packages /private/task/.lake/packages --output /private/resume-formal
```

完整本地导入闭包逐文件经原内核、严格 sandbox-lean 同服务与准确审计，拒绝 stale/partial、遗漏或额外审计。
`--case` 仅诊断，局部结果不构成整个 V3 验收。ning 用原 archive.py 生成同提交完整归档，再运行：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/resume-observation/native.py \
  --source-archive /private/source.tar --target /private/resume-native-target --output /private/resume-native
```

原生入口保留正常和边界断言、格式/Clippy、四版各自原版→错误版→恢复的实编译与同一断言，核对实际两个库/测试来源、fresh=false、二进制与全部源码恢复。
它不签发共享 native seal。主控协调公共根/生成视图、原 verify-stage1-protocol、native/审计登记、冻结与覆盖，并与共享完整入口同批内容复用；旧根通过不覆盖本包。
完整 Create/Unload/续租确认、JSON 字节和独立认证接线、持久收件/确认/退休、故障/竞争与系统组合仍是独立义务。
