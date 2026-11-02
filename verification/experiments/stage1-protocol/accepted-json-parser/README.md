# 当前 Accepted JSON 的真实回调连接

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md) ·
[原完整 WireContract](../json-wire/accepted/AcceptedWireModel.lean.in)

固定 integration 的 9c50da7c，独立于在途 BLOB/SHA/Save 包。生产代码只读，复用
原 decode_accepted_with、完整请求/封套规格及当前已存在的字段来源检查，不另造协议模型。
AuthenticatedCenterContext 是独立参数，不属于 JSON schema，也不能由本包证明其认证来源。

## 复用的来源与新增范围

字段表直接复用 [envelope-schema/inventory.json](../envelope-schema/inventory.json)。
结构体与操作的完整 map/seq 组装、字符串/字节/编号 key、重复/缺失、首错传播、
ExpectedSession 的两种字段顺序、seed、普通 u64，以及全部原叶子，都由既有
[decode-assembly](../decode-assembly/README.md) 检查。本包只增加它对真实生产特性探针的
显式根准入，原 accepted_decode/recorded_decode 入口和全部原检查保持。不能通过重命名
LLBC 声明冒充旧根；原观察器仅补当前 slice 类型格式，用于显示完整回调字典。

`call_binding.py` 复用同一个原观察器，核对探针把完整 slice/context 交给原 helper，
实际无捕获闭包使用原 Accepted Deserialize 字典调用 from_slice，结果原样返回。
探针只在私有副本追加 [原回调表达式](roots.rs.in)，不修改真实 handler 或业务守卫。
`source_admission.py` 组合上述原检查和这个薄调用检查；这里仍是源码准入，不是标准
parser 或异步路由的机器精化，也不能据此签发生产 EnvelopeBytes.SliceContract。

早期独立字段/枚举检查有重复，已从本候选撤出。其已执行成功/失败、原源码副本、
原始捕获及开发测试保留各自身份，不改签为当前复用实现的结果。

## 捕获与真实 Rust 对照

在已分配的至多四个 CPU 上使用固定工具和原共享重锁：

```sh
python3 verification/experiments/stage1-protocol/accepted-json-parser/capture.py \
  --output /private/accepted-parser-source
python3 verification/experiments/stage1-protocol/accepted-json-parser/native.py \
  --output /private/accepted-parser-native
SANDPROOF_CURRENT_ACCEPTED_LLBC=/private/accepted-parser-source/correct/source/artifacts/current-accepted-parser.llbc \
  python3 -m unittest discover -s verification/experiments/stage1-protocol/accepted-json-parser -p 'test_*.py'
```

正确版及 Resources cpu/memory JSON 名对调、Resume source/target JSON 名对调的两个真实
Rust 错误版，都须真实编译并由固定 Charon 捕获；两错须由同一个原字段路由合同拒绝。
这是来源合同拒绝，不是 Lean Rust 性质反例。原完整 parser 的 Aeneas 类型/切片投影失败
保留；本入口不以未运行或失败的 Lean 生成签发证明通过，也不更换工具或手改生成体。
原 decode-assembly 的元数据回归保持；本包只新增实际回调参数、类型、调用次数的
来源准入回归，这些内存证据修改同样不计作编译后的 Rust 反例。

`native.py` 四视图使用同一套十二项断言：保留原七项 parser 回归，新增生产 raw_value
特性下五种操作的独立字节/完整值见证。输入由字面字段名的普通 JSON Value 生成，预期值
直接用真实 Rust 类型构造，避免生产 Serialize 与 Deserialize 一起改错而相互掩盖。
见证保留 Resume 编号空洞、不同 key/retention window、完整32字节值、负时间及 false；
不证明授权、材料真实完整、摘要计算或运行期 lifetime 接受。

每版使用全新 target，检查协议库/测试的新编译、原 manifest、唯一协议库和完整断言集。
两错分别只允许完整 Create/Resume 值断言失败，其余十一项和共同正常 Destroy 见证
必须通过；之后在同一个源副本逐字节恢复，再跑完整恢复版。格式和严格 Clippy 保留。
开发结果与 ning 正式普通/原生回执分开，公共登记和共享验收仍由主控协调；不能运行旧
verify-stage1-protocol.sh 报告本包或完整 V3 通过。

后续语义实例仍须将已登记 A6 的固定解码/派生、每个原叶子回调、完整字段组装、原输入
和游标、容器及 document end 接到同一个实际 codec。compact/Recorded、Writer、持久收件、
认证来源、真实资源与系统结论保持各自义务。现有正常值与来源接线不能替代一般性定理。

集成时不再要求所有无关 V3 文件保持在原基线：`guard_baseline` 仍要求基线祖先关系，
并逐字节冻结原协议源码、同步解码 helper、Cargo 配置／锁及生产中的原解析闭包。
所有当前输入另由完整源码／模式快照和检查后核对绑定；handler 其余分支的采用由其
所属证明负责。这是来源准入，不是新旧 Rust 一般等价证明。修改上述实际 JSON 输入
会拒绝，不能靠重新接受无关文件或刷新旧生成摘要获得通过。
