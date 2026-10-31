# 固定 SHA 库与实际六处调用连接

生产只保留一个固定库使用点 `lifecycle_inbox::sha256::digest`：接收原完整
只读 `&[u8]`，调用原 `Sha256::digest(bytes).into()`，返回全部32字节。
`source_work`、配置读取、inbox save、save 重绑定和 Create 的 load/save
六处业务调用者及原 `ContentDigest::from_sha256` 保留透明生成体。

同一 `FullDigestUsesContract actualCalls` 对任意完整切片和接收者值比较全部输入、
完整摘要及整个返回上下文，包含已有完整 World、事务与连接载体。
它不从相等字段推出物理堆不变、同一持久事务、授权、材料存在或耐久接受。
空输入和 abc 的32字节见证复用已检查的纯规格；有限见证不替代全称对应。

有限 A9/A6/B4 条件限定到锁定 `sha2 0.10.9`、`digest 0.10.7` 的具体 Sha256
实例及 `generic-array 0.14.7` 的普通 u8／U32 到 `[u8;32]` 转换。
有效共享切片的长度小于 2^61，正常库完成时对应完整原始字节的 SHA-256；不要求 UTF-8，
不假定摘要单射。模型域外 panic 不描述真实库的失败行为；分配、unwind/abort、取消、
崩溃、资源 Drop 和整个库的机器精化不由这些定理给出。

`binding.py` 检查固定 Charon 的真实薄接口：仅原完整共享切片转发、固定 Digest／From
调用和全部32字节返回，没有业务分支或对象访问。`fixed-interfaces.json` 是逐符号
有限接口记录；该源码／MIR 核对不是库或薄适配的一般 Lean 实现精化。
库符合性保持显式依赖，公共合同及最终登记仍由主控／V0 单写。

`reuse/` 仅携带既有生成数据的原样副本：完整 canonical World 类型及原 GenericArray
32字节表示。它们有原始来源摘要，不是另一份协议／World 算法；当前使用端重新检查
全部实际文件。外部类型继续别名到同一 canonical Connection／RefCell／World 载体。

`formal.py` 从当前唯一生产实现重新编译、抽取正确版、输入截空及末字节清零两错，
不在正确私有视图重新替换业务实现。三视图使用同一完整 Model、Values、库及类型定义，
错误视图不安装／导入正确对应；编译、抽取、导入失败、警告或超时不能算性质反例。
原内核、规定同服务完整非 stale/partial 诊断、全部依赖及严格读回缺一不可。

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/digest-uses/formal.py \
  --output artifacts/digest-uses-development --lock /private/existing-proof-heavy.lock
```

`--prepare-only` 只检查当前抽取与构建，始终不产生完整证明回执。
每次使用新输出目录、独立 `.lake/build`，固定工具、既有共享重锁和缓存保持。

`native.py` 交普通验收方执行，四份 actual／input-empty／tail-zero／restored 源共享
五组断言：空、abc、NUL／非 UTF-8、64字节完整块、65字节块加尾。
它核对实际新编译 protocol／service、源／manifest／链接与精确失败集合，另跑格式和
严格 Clippy。restored 在私有副本实际施加再恢复末字节变异，核对原字节后新编译。
临时 SQLite 和指针／完整 World 观察只是有限 API 见证，不是事务历史或持久收件证明。

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/digest-uses/native.py \
  --output artifacts/digest-uses-native --lock /private/existing-proof-heavy.lock
```

源码准备、局部结果、普通执行和最终验收分别记录；本包不声称以上命令已经全部通过。
共享 `verify-stage1-protocol.sh`、`verify-controller.sh`、`proof_native_suite`、公共
Lake／根导入、冻结／输入／累计审计／覆盖和依赖由主控同批处理，候选见
[登记说明](REGISTRATION.md)。工作流遵循[统一规则](../../../../docs/proof-workflow.md)。

`source_work` 清除材料时的 `Sha256::digest(empty).as_slice()` 是另一个真实使用点，
不在选定六方法内。清除／SQL／提交、各上层预算连接、JSON／TLS、持久收件与恢复、
producer／driver 以及完整 V3 继续单列；请求 Resume 仍允许 target 大于 source 的空洞。
