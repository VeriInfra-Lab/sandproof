# Create owner 守卫的完整值合同

[V3 设计](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md) · [依赖登记](../../../../docs/formal/dependencies.md)

本包连接唯一真实收件实现的两个只读 owner 守卫。正确视图仅追加透明转发根，直接抽取实际生产函数；错误视图才在该实际函数内修改一个指定表达式。原候选数学执行、实际采用后的重新抽取及普通验收分别记录，不改写旧结果身份。

`FullExistingContract` 对任意已解码完整 Inbox、完整外部 owner 和本地 owner，规定按原顺序选取首次全文匹配。该行本地 owner 不等时立即 Conflict，相等时返回 true；没有匹配才返回 false。不加入映射唯一或 ID 非零条件。重复行反例属于这个明确的任意值域，不声称它已通过外层 Inbox 校验。

`FullNewContract` 复用原 `CreateOwnerSuccessor.expected`，对原完整 Inbox、World 和 InternalLocalBinding，按 `local.sandbox` 排除记录后计算原 owner 与高水位最大值的受检后继。分配错误原样传播，只有成功后才比较 `local.owner`。IdentifierExhausted 不能被后面的 owner 不匹配改写成 Conflict。

真实父函数只有在首次 owner 查找返回 false 时读取 World；已有映射的成功或冲突均不调用重算。原选择、完整状态写入、deadline、ID、SQL 和 save 的顺序保留。薄根的完整参数接线和两个守卫的对应不等于整个父函数或新旧实现的一般等价证明；同一事务、提交未知、重启及实际资源仍须另行连接。

四个实际 Rust 错误版分别错误接受本地 owner 不一致、跳过首次冲突、用 owner 数字代替排除的 sandbox，以及把分配错误改为 Conflict。正反视图使用逐字节相同的合同、比较引理和见证；错误视图不安装或导入正确守卫的完整对应定理。每版先实际编译和抽取，再由同一完整合同拒绝；编译、导入、服务初始化失败及超时都不是业务反例。

复用原 OwnerMapping、CreateOwnerSuccessor 的模型、数学证明、值见证和有限 U64 Ord 字典绑定。String 全文比较、Vec 索引和算术仍受原逐符号正常有限值合同约束；自有循环边界、完整字段比较和错误传播由真实生成体对应证明。固定工具、库模型和翻译器的可信责任保留，未新增公理或把整个库设为可信。字段相等不提供认证、授权、持久接受或资源存在的结论。

证明机使用固定工具、独立构建目录、原共享重锁和原容量门槛：

```sh
python3 verification/experiments/stage1-protocol/create-owner-check/formal.py \
  --variant all --output artifacts/create-owner-check-formal \
  --lock "$SANDPROOF_PROOF_HEAVY_LOCK"
```

`--variant actual` 只检查实际正确视图；不能写成四个错误版或正式共享入口也已通过。每个选中视图都重新编译、抽取，并以原严格检查器覆盖全部实际文件、内核、规定同服务完整非 stale/partial 诊断与依赖审计。清单固定完整生成闭包、原始生成字节、物理文件和审计集合，不只跑旧根。

`native.py --output <新目录>` 交 ning，在实际、四错和恢复实际六个独立源码/target 视图运行相同断言。它检查协议库及服务测试确实新编译、源码/manifest/profile、格式、Clippy 和精确失败集合。此原生入口尚未执行；不能将证明所需编译当作普通或原生验收。

旧 [adopted-create](../adopted-create/README.md) 和 [create-install](../create-install/README.md) 继续限定其原冻结版本，不因本包采用而直接更换摘要。公共 Lake、根导入、原生登记、依赖合同、根级输入与覆盖记录由主控单一写入；本包提供具体候选。局部检查不代替 `verify-stage1-protocol.sh`、全项目普通入口、完整持久收件或整个 V3 验收。

后续从原 FullLoad/FullInbox 接受结果落实 owner 数量上限，再连接完整写入容量、高水位、deadline/固定 ID/两个 SQL 参数、save 和同事务事实。原寿命 UPDATE 没有检查成功行数。既有 Manifest 任意已解码值和 FullFlow 证据保留原适用范围；完整字节/JSON/TLS、认证来源和实际文件/VM 事实不由这些值定理推出。公共 Resume 继续允许 target 大于 source 的编号空洞，内部 checked-next 的业务支持域单独闭合。
