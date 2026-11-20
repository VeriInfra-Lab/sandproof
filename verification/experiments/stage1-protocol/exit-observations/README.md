# 恢复退出的三种观察匹配

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../docs/formal/dependencies.md) ·
[证明工作流](../../../../docs/proof-workflow.md)

固定 integration `9c50da7cc9391518514f14405e6c779e5c5cbe2c`，独立增量。
正确视图只追加两个文件中的三个透明调用根，固定 Charon/Aeneas 捕获原三个 matches 方法，
生产只读。Spec、完整比较、精化/接受/拒绝/总性、正常观察与三个真实 Rust
字段遗漏版共用 FullExitMatchContract；不为错误视图更换规格。

完整 action 比较包含 ActionIdentity 每字段、原 EffectId/Command 所有参数；
work_binding 比较完整32字节值，不证明哈希计算或其来源。直接恢复候选检查正
PID、正 start_time_ticks 和 work.source.host_bootstrap 的 boot_id；没有
要求 PID/ticks 与保存 source 相等。BeforeAdmission 比较 candidate.process
的完整三字段；Continued 比较 work.source.source 的完整三字段。后两者自身
不检查正 PID/ticks，不能把提供方条件加入本匹配规格。

只复用既有 A6 String、Option 字典比较与固定数组/Vec/标量语义；自有比较保持
真实透明生成体。已填充的外部定义复用既有模型正文，避免同时导入两套生成类型
的全局实例。原生成体不手改，不导入生成的含 axiom 模板。既有字节/向量数学
及原 Command 比较策略适配到本独立生成视图；比较与类型非空是支持引理，
不作为独立业务性质计数。默认值只用于类型非空证明，不是已授权候选见证。

持已有重锁，在获准四 CPU 上运行局部形式入口：

```sh
python3 verification/experiments/stage1-protocol/exit-observations/formal.py \
  --lock /private/existing-proof-lock --output /private/exit-observations-formal
python3 verification/experiments/stage1-protocol/exit-observations/native.py \
  --output /private/exit-observations-native
```

形式入口重新捕获四视图，比较字节一致的原生成体，再逐一执行原内核、规定同服务
完整非 stale/partial 诊断与标准依赖审计。每个视图使用独立 .lake/build；公共
正确 Lake 条目在私有副本中实际建立后才选择当前视图，错误路径/重复条目拒绝。
共享执行器可通过 --lock-fd 传递同一已持有描述符；不能重开另一锁等待自身。
原生入口由外层或 ning 共享套件持锁，不重复获取父锁。

原生五视图复用原 SQLite 候选夹具与同一22条断言，含三种正常观察、完整 action
身份/命令尝试、32字节末尾、PID/ticks/boot 变异及直接恢复的正值边界。
三个错误版精确失败集合分别为1/2/2；每版使用全新 Cargo target，检查实际协议库
与服务测试的源、manifest、新编译/链接及完整断言集，执行格式和严格 Clippy。
三错后逐字节恢复源码，恢复版再次执行全部正常断言。受控 HostEvidence 与真实
进程事实分别报告，不把这些夹具当作实际 VM 死亡证明。

本包不验证 current_request、local、native 或内部 authority 的当前性，也不
提供进程已退出、独立认证、材料闭合、同事务、持久接受或派发权限。原
restore::validate_record、exit::eligible、authority/candidates/driver 和
JSON/TLS、未知提交/重启保持独立义务。精确通过范围在 #635/本包PR维护，
局部开发不签发 ning 或共享验收；旧原型输入不改签为最终候选。

[公共登记候选](public-registration-candidate.patch) ·
[依赖候选](dependencies-candidate.patch) · [覆盖候选](coverage-candidate.patch)

公共配置、根级冻结/导入和覆盖由主控单写采用；本包只提供候选。新增1个原生组件、
1个含四视图的形式任务、7份审计模板/57声明，固定 Create 十二包不变。同批采用
其它在途包时由主控计算累计计数，不能重复套用各包绝对值。正式普通/原生由 ning，
共享 verify-stage1-protocol/verify-controller 的接纳和旧冻结阻断另行报告。
