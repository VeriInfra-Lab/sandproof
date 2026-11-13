# 原解码字段组装候选

[组件入口](../README.md) · [完整字段规格](../envelope-schema/README.md) ·
[原缓冲与错误连接](../parser-binding/README.md) ·
[V3 范围](../../../../docs/formal/03-protocol-inbox.md)

基于 #708 的精确提交，生产固定 a9c9967889c4cf298e4378c7106948018835fc64。
本候选复用既有字段清单，核对原派生 Rust 的十个结构体和五个操作分支的 map/seq
组装：原输入 reader、完整类型字典、字段名/声明位置、临时槽、返回结构字段和
原 Result 错误传播。对象重复字段在读取其值前拒绝；缺失字段保留原 missing_field
调用及其 Error 字典。另核对三种字段标识 visitor 的字符串、完整字节/长度和编号，
ExpectedSession 的原普通 u64/单位 seed、标签先到/内容先到/顺序输入，以及八个透明包装
和六个原标识符/正计数器转换的输入、闭包与原 custom 错误。完整 LLBC 和退出/展开代码
保留，不修改生成体；ContentDigest 的 32 字节值不表示实际计算哈希。
操作标签的原字符串/字节/编号选择还连接到各自已经核对的五个 visitor，不能只用
分支内部字段正确推断外层分派正确。

`binding.py` 只跟踪有限源码位置和赋值来源，检查循环体中的槽读写关系，不执行或
展开任意 Rust 循环，不证明解析器。`check.py` 将这些来源接回原完整字段规格。
其结果是源码准入证据，新增库精化定理为零；不能据此实例化生产 SliceContract。
完整扫描、固定库回调语义、一般性首错组合和析构/展开仍须由适用合同及对应提供。
PlaceMention 可能要求投影有效，不当作无效果语句；源报告逐项保留这类义务。
完整 parser 的固定 Aeneas PtrMetadata 障碍保留，源码检查不解决该翻译障碍。

真实错误版只在私有源副本交换 Recorded.current 和 terminal 的 Deserialize 名称，
Serialize 不变。两者同为 bool，编译成功不代表接线正确。原版、错误版和恢复版
执行原 `actual_five_branch_fields_and_roundtrips` 断言及 Clippy；不为错误版另换性质。
错误版还必须由固定 Charon 实际抽取，再到达同一字段路由拒绝。这个结果属于
原生反例与源码准入拒绝，不是新的 Lean 形式反例。

开发入口持共享重锁，证据保存在新的私有目录；正式原生验收仍归 ning：

```sh
python3 verification/experiments/stage1-protocol/decode-assembly/native.py --output /private/decode-native
python3 verification/experiments/stage1-protocol/decode-assembly/admit.py --output /private/decode-source
python3 verification/experiments/stage1-protocol/decode-assembly/negative.py --native-evidence /private/decode-native --output /private/decode-wrong
SANDPROOF_DECODE_ASSEMBLY_NATIVE=/private/decode-native SANDPROOF_DECODE_ASSEMBLY_LLBC=/private/decode-source/source/artifacts/bytes-recorded_decode.llbc python3 -m unittest discover -s verification/experiments/stage1-protocol/decode-assembly -p 'test_*.py'
```

公共入口只提供 [登记候选](public-registration-candidate.patch)，接续四个父包的对应候选；
新增原生 case、两个真实解码根、错误版抽取和全部源码准入回归进入原
verify-stage1-protocol.sh，保留全部父包 Lean/审计入口。没有新增 Lean 目标，不重复登记
父包已有定理。另提供 [覆盖候选](coverage-candidate.patch) 和 [依赖候选](dependencies-candidate.patch)。
本包未签发共享验收；公共配置、覆盖和依赖仍由主控协调。
认证、持久收件、资源事实、完整 Manifest 和系统结论继续按 V3 原范围登记，不由
字段相等或源检查推出。
