# 共享字段的有限使用域目录

本目录服务于 [V0/T1](../../../docs/formal/00-foundations.md) 的来源和接口对账，
不新增 Rust 算法、Lean 定理或可信保证。实际交付和验收状态见本批 PR；
[依赖](../../../docs/formal/dependencies.md)和[覆盖](../../../docs/proof-coverage.md)
仍由原权威文档维护。本目录不能代替 T0 全量使用点、下游业务证明或整个 T1 的关闭验收。

## 怎样使用

1. 在 [catalogue.json](catalogue.json) 找到真实位置，先读 `source` 原片段及完整文件摘要，
   再读 `input_domain`、`acceptance`、`rejection_order` 和 `conditions`。
2. `proof_refs` 指向已有定理、准确审计声明和原冻结清单；`extraction` 引用原透明生成函数。
   这表示已有成果的来源关系，不表示本批重新证明了它，更不表示整个调用者已经证明。
3. 调用者落实 `providers` 对应的身份选择、授权、持久高水位、时间/资源观察和业务状态条件。
   公共转换缺口保留在 `gaps`，不能仅靠写“下游负责”消除。
4. 原签名未做 Charon/Aeneas 预检的，保持 `unknown-not-preflighted-for-this-location`。
   在 T0 的原 MIR 中找到函数头，也不能把该状态升级成“可以抽取”或“已经证明”。

`domains` 是规则词汇，不是提前假设输入已经合法。比如编号构造器可以接收空串并拒绝，
不能把“输入已满足编号域”写进前提后只证明剩余成功分支。

## 这批选择了什么

有限审查位置在 [selection.json](selection.json)，按 D1–D11 分组。它是本批人工审查的
范围和对应关系；检查器核对它与实际源码/已有证明的关系，不能从文字说明推导正确性。

| 组 | 真实对象 | 要保留的区别 |
| --- | --- | --- |
| D1 | 八类协议不透明 ID 的构造、投影和 Deserialize | 非空、最多128个UTF-8字节、原ASCII集合；`001`与`..`可合法，不等于规范数字或安全路径 |
| D2 | PositiveCounter 构造、get、from、next、Deserialize | 构造排除0，耗尽拒绝不回绕；解码前态仍可能是任意u64 |
| D3 | NodeIdentity、ControlIdentity 和代表守卫/Probe | 完整字段、版本/节点/slot顺序；字段相等不授予权限 |
| D4 | Resources、三个数值helper和两个真实caller | 资源单位、正值检查的调用位置、饱和/溢出/缩窄；helper定理不等于caller证明 |
| D5 | 原规范分配编号parser及中心透明包装 | 精确前缀、非零、高水位上界、规范十进制，拒绝前导零；高水位来源另证 |
| D6 | RetentionLane::current | 原高水位的严格后继与耗尽，不证明退休资格 |
| D7 | 原ActionIdentity、ResourceRef和回执守卫 | attempt0允许，完整身份/资源保留；自有观察getter不能当外部假设 |
| D8 | 本地十类u64 ID及原本地投影 | `new`本身接受0；投影validator可以另行拒绝0，二者不是同一个保证 |
| D9 | CreateBinding、LifecycleRequest及实际守卫 | 完整字段，options是字节数组；创建与Status的资源/时间合法域不同 |
| D10 | StatusRequest、HostStatus及真实守卫 | u16可表示0和4097，响应守卫只接受1..4096；free0/time0不被该守卫拒绝 |
| D11 | ContractVersions、模板文本、Guest字节/文本和预算 | i64保留符号，usize绑定目标平台；原字节不提前冒充UTF-8 String或ASCII ID |

尤其，原 Guest `Query` 的三个可选字段是 `Option<Vec<u8>>`，允许无效UTF-8；
`None`和`Some(empty)`不同。`into_text`按path、username、signature次序转换并返回
首个原参数错误，expiration仍是Option<i64>。这是安全输入的域，不因T9排除而消失；
本批没有证明完整Go查询解析、签名授权或路径操作。

版本也逐接口区分：动作协议的 `PROTOCOL_VERSION` 是1，内部协议的
`INTERNAL_PROTOCOL_VERSION` 是2，虽然二者都是u16，不能共用一个“支持版本”值。
目录逐项绑定原常量、准确类型与实际使用位置；中心/宿主存储版本的u32/i64同样保留。

## 状态和边界

- `existing-implementation-theorem`：原位置有已登记的透明生成函数和已有定理；仍须采用
  定理的准确前提、固定库及真实对象。目录核对不是新的内核检查或重新抽取。
- `conditional-existing-leaf-binding`：已有叶子规则及接口条件可复用；具体输入、原字典、
  游标或标准派生连接不能由叶子规则自动推出。
- `source-reviewed-signature-not-preflighted`：只有原源码审查和可复用理论线索，没有该实际
  签名的预检/证明。本标签不能因为函数已在MIR中出现就变为上两种。

这三种状态都不是“已部署系统正确”。普通值、正常返回、原库模型及获批机械连接的
范围沿用既有依赖登记；不新增任意Drop、async取消、OOM/abort、外部效果回滚或进展保证。
Writer仍是T2；身份/授权、高水位/重启、容量预留、寿命选择、物理观察及系统组合分别由
V1–V9落实。公共字段/转换本身真的缺连接时仍由V0补。

## T0和原MIR怎样关联

目录引用原五个生产根、dev/release配置、60个编译unit和原来源清单；每个位置保存同包的
具体unit索引。它证明的是可核对的源码归属，不表示某个根动态执行了该函数。

[capture-summary.json](capture-summary.json) 来自a6000对已有完整捕获的只读定位。
它核对原receipt/index、有限Rust源码、所选unit的完整MIR及原编译命令摘要，再保留五组
查询的所有原函数头和行号。重复的const/runtime函数头保留，不合并为一个“调用数”。
没有重新捕获、编译、Charon/Aeneas抽取或Lean检查；原`check-published`仍是完整来源准入。

[capture.py](capture.py) 的可复现命令只在原证明机的数据盘执行，读取原捕获且输出新文件：

```sh
python3 verification/experiments/shared-field-bindings/capture.py \
  --capture "$SANDPROOF_USE_CAPTURE_PHYSICAL" \
  --catalogue verification/experiments/shared-field-bindings/catalogue.json \
  --selection verification/experiments/shared-field-bindings/selection.json \
  --output "$shared_fields_new_evidence_file"
```

`lexical_candidates`另外保留三个明确搜索规则在原来源Rust文件中的全部命中；包含测试、
类型同名及未确定cfg的文本。它既不是生产调用计数，也不解析别名、重导出、宏生成或
动态分派。新增匹配或删掉候选会被检查器拒绝，但这些规则不保证检出所有形式的使用。

## 有限公共候选缺口

`gaps`保留最小原签名、已有理论、提供方和下一步。G1本地ID的原new/value连接、G2完整
有符号版本记录、G3原字节到文本实例均尚未预检；先做最小原签名试验再安排证明，
不据目录认定工具不支持或业务实现错误。G4 checked_deadline属于尚未连接的caller：
已有u64秒换算不证明其i64缩窄、加法或首错，纯整数原语能否复用与V5期限策略分开处理。
这四条是有限待核对清单，不能据它们宣称已经发现全项目的全部公共缺口。

## 运行本批检查

普通Python回归和文档检查在ning单进程执行，不启动Rust/Clippy或占用重任务锁。
在含原Git上下文、相同跟踪输入的私有副本中运行：

```sh
bash scripts/python-entry.sh tools/check_shared_field_bindings.py
bash scripts/python-entry.sh verification/experiments/shared-field-bindings/replay.py \
  --output "$shared_fields_new_ordinary_evidence"
```

本批[冻结清单](../../shared-field-bindings-inputs.sha256)包含有限源码、实际定理/审计/生成
引用、原T0配置/目录/清单、目录自身及检查器/负例回归。过时片段、漏字段、错类型/域、
错误函数/定理连接、丢候选、虚假状态升级或遗失公共缺口必须失败。源码变化后先审查
适用性与受影响位置，不以刷新摘要代替审查。

本批没有新增Lean模块，不接Lake、总审计、公共use-inventory或套件；这些共享位置及
权威文档由统筹维护。独立组件通过后仍须正常PR、统筹组合验收及真实调用点收口，
不能据本目录提前关闭T1/T0/V0。
