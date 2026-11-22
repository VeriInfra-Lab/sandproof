# V3：完整 set_end 生成体与未解释方法的使用证明

[证明工作流](../../../../../docs/proof-workflow.md) ·
[父提案前缀](../source-set-end/README.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包从已解码值开始，连接原完整 `set_end` 生成体、完整返回值和内存 Record 后状态。
父提交精确为 ac1d34156f5137696e5ca3431258308ad124419d（#1016），
生产固定 b0d791ccc4d4e058b710dbf8f46766652b2853df；不自动追随上游新头。
只新增独占证明包；生产、公共配置、总根、登记、冻结和覆盖由主控协调。

## 范围与完整合同

`SourceSetEndFinishProof` 复用父 `SourceSetEndSpec` 的同一完整 Contract，
没有为正反版本更换性质。先经真实提案 helper，保留缺 capture/action/worker、
完整请求匹配、独立编码错误及 completed/NoRequest 首拒绝。旧 End 比较覆盖全部构造和字段；
冲突先于提案编码，相同完整 End 直接进入回执。新 End 保留原 16384 字节上限，
编码错误和超预算在赋值之前，随后只写 capture.end，再调用完整原 receipt。

**赋值后的 receipt 错误保留已经写入的完整 Record。** 不能把 Err 改成回滚、
成功回执或持久提交。父 WorldTransaction、SQL、未知提交/恢复和耐久另有义务。
Representation 仍为父 Full 字符串/配置域；不是授权、FullAllowed、资源存在或已接受假设。
任意端口失败和发散保留，不以有限见证替代全称定理。

## Box 方法与原准入

原固定工具的完整输出列出 31 个外部符号；原严格 30 项准入仍拒绝
`Box.Insts.CoreCmpPartialEqBox.ne`。`capture.py` 直接复用父原捕获和原拒绝规则，
工具结束后仍可能以准入失败退出，不能计作性质反例或公共抽取通过。
`validation.raw` 完整回读这种已拒绝结果，重新到达同一拒绝；保留全部映射和原模板，
不删除额外元数据、改写生成体、导入 axiom 模板、增加可信律或更换工具。

`SourceSetEndBoxUseModel` 将原两个 Box 字典构造点的 ne 槽替换为完全任意的数据参数；
eq 仍是已登记实现。`SourceSetEndBoxUseView` 原样重放两个原枚举 eq 和原 setter，
具体原 enum 字典的默认 ne 则有完整的表示等式。
`SourceSetEndBoxUseProof` 证明 Option.eq 只读取 eq 槽、两份原枚举完整 eq 不变，
最终完整 setter 的结果及状态对任意 ne 选择不变。任意方法可直接失败或发散；
不假定任意 Box.ne 等于 not eq，也不从函数正文没有直接引用推断不依赖。

这个精确使用证书须交主控审查采用，**不会自行令原捕获准入或公共整套通过**。
固定原输出的 117 个完整类型、199/201 个函数逐字核对并复用已检查父定义；
仅原 observation 字典和完整 setter 是新增声明。Box 视图只重新插入使用证明所需的
四个完整原函数声明；两个具体 enum 字典的表示等式也必须接受原内核审计。
正常借用/分配、具体库与生产/staging 的一般连接仍保留。

## 正常与真实错误版

正常见证包括完整 NeverReleased 输出和所有后状态字段，以及相同完整 End 重放。
表示域有合法成员；未解释方法总失败、总发散时仍得到同一合法结果。
这些是值接口的见证，不声称真实 Serde 输出空 Vec 或存在真实物理对象。

错误版只删掉原提案 completed/NoRequest 的三行提前拒绝；保留后段 receipt 校验。
固定 Charon/Aeneas 须实际编译并生成它。117 个 canonical 类型不变，201 个函数中
仅原 proposal 有该精确差异；只允许原闭包及编码 helper 的源码位置注释各移动三行。
`SourceSetEndFinishNegative` 证明完整错误 setter 先写 End、再返回 CorruptStore；
正确 setter 返回更早的 InvalidHostEvidence 且记录不变。由同一个完整 Contract 反驳，
不能把后段 Err、编译失败、导入失败、超时或原准入拒绝当成性质反例。

## 完整开发检查与公共候选

`prepare.py` 从完整原始捕获、父 216 文件/1208 审计和原 retention 26/127 证据重建。
输入 JSON 固定六个私有只读位置：`correct`、`wrong`、`reads`、`reads_audit`、
`retention`、`retention_audit`。两份 raw 捕获、#1016 正确完整项目/审计及原 retention
项目/审计缺失或损坏都拒绝；不提供删减父义务或只验正确版的参数。

`check.py` 调用原未修改的完整开发检查器：正确 223 个实际文件/1236 条唯一审计，
错误 227/1252，各自构建、全部实际文件同服务诊断、依赖审计和完整计划/文件新目录复现。
其中新增局部审计正确 28 条，错误额外 9 条。数量是检查义务，执行结果另看原始回执；
本入口即使完成也只是完整生成体/使用证书的开发检查，不是原捕获、普通/原生或公共验收。

```sh
SANDPROOF_ACCEPTANCE_ROLE=formal python3 \
  verification/experiments/stage1-protocol/unload-parent-link/source-set-end-finish/check.py \
  --inputs /private/finish-inputs.json --output /private/finish-check \
  --lock /private/proof-heavy.lock --lake-packages /private/fixed-lake-packages \
  --lean-mcp /private/fixed-lean-mcp
```

`registration-candidate.json` 和 `dependencies-candidate.patch` 交主控审查。
公共采用必须明确原 31 项元数据、原拒绝与精确证书的关系，不能删掉 Box 映射或
单靠换 hash 放行。此包不并发写公共 Lake、Cargo、总根、登记、冻结和覆盖。

## ning 原生交接

复用 #1016 的原 `native.py` 及同提交 66 条完整断言，四套完整输入各 4906 文件。
正确 original、adapted-original 各须 66/66；错误两版各 65 通过/1 失败，唯一断言须到达
`completed NoRequest must be rejected before installing End`，并核对较晚 CorruptStore 和
已写 End。两错误后各独立重编译 original 并恢复 66/66；不能只换测试二进制复用旧错误库。
本包不新增原生算法或另一份弱断言。原工具编译/MIR 不等于 ning 原生已经执行。

普通完整入口、固定 Redis 及原生运行交 ning；公共 proof_native_suite 由主控同批统一
验收和严格内容复用。此前局部/父开发结果不能迁记为当前 ordinary/native 或公共通过。
从已解码值开始；真实 JSON 字节/认证来源、完整 TLS、持久收件、当前意图和耐久发送、
SQL/同父事务、实际进程/VM/Writer/FD/材料及系统结论仍由原提供方承担。
ContentDigest 的 32 字节值不证明计算过 hash 或材料真实完整。
证据位置和报告哈希按用户决定只保留本地。
