# 运行期限与恢复来源值绑定

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

固定 #686 的 `39577ca1e61f4daa4ba4bbda7db94bf087bdaef0`；本包生产只读。
`ReadyFactModel` 定义当前 `CurrentReadyFacts::matches_create`、`matches_resumed`、`matches_resumed_renewal`
三个完整叶子合同，`ReadyFactBindings` 连接真实固定工具生成体，涵盖任意类型表示输入。
Create 保留原 startup deadline、两个 checked_add、启动/观测/到期/总期限关系；Resume 保留来源代、capture、
完整 32 字节 digest、原启动/总期限/deadline/end 及当前时间形状；Resume 后续租另要求目标代对应、非负 i64 期限，
使用原固定整数转换引理证明转换精确，按 extend_only 选择下界或精确结束时间。

这些是已解码值守卫，不是物理观测或授权。来源 `material_bytes` 不在 Resume 叶子的检查范围，
续租叶子也不比较来源的旧 end_at_ms；不能自行加强合同，或据此推出实际材料完整、已恢复、续租单调。
V1/V2 提供原意图、来源事实选择与发送；V4/V5 提供耐久接受、执行/续租政策与结果；V6 提供实际文件/VM 事实；
V7 提供独立认证来源。Digest 值相等不表示计算过哈希。

`ReadyFactValues` 的合法请求/封套与见证由正反版本共用。四个错误版实际修改生产副本的单一原表达式：
遗漏 Create timeout 对应、将来源摘要条件改为恒真、允许恰好到期的观测、去掉负续租期限拒绝。
每版都保持同一完整合同，必须真实编译、抽取并到达反例；失败导入、编译失败或超时不算性质拒绝。
原生产工作树和工具版本不变，不修改生成函数体，不导入外部 axiom 模板。

`extract.py --profile facts` 提取原四个 Request/Accepted/Recorded 根及三个事实叶子。
`resume` profile 另包含真实 ResumeObservation；抽取可行不等于其完整分支已证明。
只复用原四项逐符号外部库模型；全部生成文件保持独立 Request 物理视图，不与别的同名生成根混装。
完整 Resume/Create/Unload 观测外壳、JSON 字节接线、来源选择/原 journal 事务与系统组合仍单列。

开发形式检查（先取得共享重锁；依赖缓存属于本任务，每个 case 独立 `.lake/build`）：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/observation-bindings/formal.py \
  --packages /private/task/.lake/packages --output /private/ready-facts-formal
```

入口重新编译真实 Rust、固定工具抽取，再通过原内核与严格 sandbox-lean 同服务检查完整本地导入闭包、
全部实际文件及准确审计；保留失败证据，拒绝 stale/partial、漏项或额外审计。`--case` 只给局部诊断，不能称完整验收。

ning 使用原 `archive.py` 生成的同提交完整源码归档执行：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/observation-bindings/native.py \
  --source-archive /private/source.tar --target /private/ready-facts-native-target --output /private/ready-facts-native
```

原生入口核对全部正常断言、格式/Clippy及每个错误版的原版/变异/恢复三次真实编译与同一断言。
实际库与测试的来源、manifest、fresh=false、二进制及恢复后的全部源码都须吻合；它不自行签发共享 native seal。
公共透明根/生成视图、原 `verify-stage1-protocol.sh`、共享 native/审计登记、冻结和覆盖记录交主控同批协调；
未登记前不得把旧根的通过记为本包或完整 V3 通过。
