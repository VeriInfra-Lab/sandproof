# 协议 4 解码字典与缓冲错误组合

接续 [#707 的编码源接线](../serde-binding/README.md)及
[完整字段回调合同](../envelope-bytes/README.md)。旧 a9c 提交仅为最初候选来源；
统一入口读取本仓库当前实现并重新核对完整输入。
[EnvelopeParser](EnvelopeParser.lean.in)
复用原 `SliceFieldReader`、原自有叶子回调、`finishDocument`、`closeContainer`，没有另建协议值模型或解析算法。
八条定理是**显式 A6/B4 条件下的错误/正常值组合**；不是整个 Serde 或 JSON parser 的机器精化，
也没有签发 `EnvelopeBytes.SliceContract` 的生产实例。已有全文相等、守卫首错、Manifest/carriers
的精确适用证据和原 Rust 错误版保留；持久收件、认证、实际资源与系统结论仍未完成。

## 真实调用路径与对应条件

原 `SliceFieldReader` 的 `D` 已经是任意具体反序列化器类型。缓冲字段用真实
`ContentDeserializer<serde_json::Error>`，以原 SliceRead 缓冲后的同一游标作为 before/after；
它不重新扫描原字节。原始字节身份、有效游标和原 custom 字典仍须由实际库调用提供，
不能从一个随意构造的 reader 推出它发生过。

| 实际固定符号 | 已核对的行为和证明接口 | 尚需的提供者 |
| --- | --- | --- |
| serde_derive `enum_internally::deserialize`；协议 `LifecycleOperation::deserialize` | 先 `TaggedContentVisitor` 缓冲，再 `ContentDeserializer` 进入五个原分支；10 个原 map-key string/bytes visitor 保持 unknown_field 拒绝 | A6 的固定标准派生与缓冲合同，完整生成调用和字典另存 LLBC；不是任意 derive 的证明 |
| serde `TaggedContentVisitor::visit_map/visit_seq` | 按输入读取并保存普通内容；语法错误或重复 tag 可先于自有字段校验发生 | A6 的原扫描/缓冲返回；不能假定先检查已见的零计数器 |
| serde_derive `enum_adjacently::deserialize`；`ExpectedSession` | tag 先到时直接 seed 读取；content 先到时缓冲，再以原 Error 类型重读；声明顺序数组也直接 seed | A6 提供实际选中的 reader；不能统称两种枚举都先缓冲全部对象 |
| serde `ContentDeserializer`、`visit_content_map/seq` | `Error = E`；visitor 失败时短路，不执行 content end；没有 SliceRead、JSON 位置修正 | 两个 `tri!` 的正常返回组合复用 `finishDocument`；B4 绑定原值和同一 Error 字典 |
| serde_json `Deserializer::deserialize_struct/deserialize_any` | 实际 JSON 容器先取得 visitor 返回，仍求值 end_map/end_seq；普通 end 错误输给 visitor 首错；随后 fix_position | `closeContainer`；close 的异常、效果或不终止不能被数据错误抹去 |
| serde_json `Error::custom/make_error/fix_position`、`SliceRead::position` | 两个原自有错误没有位置后缀，custom 产生 data/0/0；仅缺失位置才补原游标，已有位置保持 | 原 Display 已有生成证明；16 份固定库源码支持逐符号 A6 审查；不推广 plainCustom 到任意消息 |
| serde_json `from_trait` | 完整值成功才调用 document end；先前错误不会被 trailing-input 错误替换 | 原 `finishDocument`；不把成功值解码当作业务授权、接受或资源事实 |

原库版本/源码及实际 Cargo feature 图由 `library.py` 重读，登记在 `candidate.json`。
生产与本次 native crate 的 serde/serde_core/derive/json 特性完全一致（包括 alloc、raw_value）；
保留的原字节抽取包装未启用 raw_value。当前普通结构名不选 raw-value 特殊协议，所有实际
字典、关联 Error 和 unwind 继续保留在完整 LLBC。这个特性区别显式保留，未宣称包装代表
所有生产字节入口。库源码冻结与检查器报告只用于来源准入，不能据此称库语义已由 Lean 证明。

正常见证使用同一个完整 payload 返回合同。七项原生回归调用原生产类型，检查：孤立操作的
0/0 叶子错误、请求/Accepted/Recorded 最近外层位置保持、缓冲语法首错、ExpectedSession
两种字段顺序及数组的实际行列、content/container/document 首错、未知/重复/缺失字段、
正常完整值往返及 trailing-input 拒绝。有限输入不能替代一般性定理。

真实错误版只在私有 `request.rs` 删除 LifecycleOperation 的 `deny_unknown_fields`。
原版、错误版和恢复版运行**同一套七项断言**；错误版必须新编译并在原 unknown-field 拒绝
断言失败，其余六项通过，再由固定 Charon 实际抽取，缺失原 visitor 的同一源合同拒绝它。
这是 Rust 原生反例和源准入拒绝，**不是 Lean Rust 形式反例**；原 key-only-replay、
same-incarnation、session-only-envelope 及 Writer 吞错的形式反例另行保留，不迁记为本批结果。

## 执行与交接

持共享重锁，使用独立 `.lake/build` 和原固定工具/包缓存：

```sh
python3 verification/experiments/stage1-protocol/parser-binding/native.py --output /private/parser-native
python3 verification/experiments/stage1-protocol/parser-binding/negative.py --native-evidence /private/parser-native --output /private/parser-wrong
python3 verification/experiments/stage1-protocol/parser-binding/formal.py --output /private/parser-formal
SANDPROOF_PARSER_BINDING_NATIVE=/private/parser-native SANDPROOF_PARSER_BINDING_LLBC=/private/parser-formal/source/artifacts/bytes-recorded_decode.llbc python3 -m unittest discover -s verification/experiments/stage1-protocol/parser-binding -p 'test_*.py'
```

`native.py` 检查完整输入、已登记 archive/unpacked 库源，三版分别构建真实协议库和测试程序，
不能用旧二进制或零测试通过。交接 `result.json` 和三版所有 test/Clippy 日志；不传 target。
`negative.py` 要求三版六条 test/Clippy 记录完整且无重复，并校验其日志/输入，按当前本地源重建完全相同的变异，不读取另一机器的 source/target。
`formal.py` 重新抽取原 Request、JSON/map 视图和四个直接字节根；核对完整 AST 与未变父包，
重新检查全部实际导入文件、内核和审计；缺失、partial/stale 或输入变化均失败。

开发运行可在本机；正式普通/原生回执由 ning 提供，公共完整验收由主控统一运行。
`public-registration-candidate.patch` 在 #705/#706/#707 的对应登记候选后应用，新增入口必须纳入
原 `verify-stage1-protocol.sh` 与 native suite。共享 Lake/根导入继续复用父包候选；模板由私有准备流程物化为原模块路径，进入新增实际文件闭包检查；`.lean.in` 同时进入原规则缓存键，
公共扫描不会提前依赖未采用的父包模板。`audit-registration-candidate.patch` 补齐本批和同堆叠
七个父包模板的全局审计登记；其它历史模板/根级冻结继续由主控协调，不能据本候选声称全套已接纳。
两个文档候选只交主控，不并发写公共依赖和覆盖记录。

后续仍要完成一般完整字段组装与 compact 成功输出的真实 SliceContract 接纳、具体路由字节接线、
标准库条件的主控审查、完整 Manifest、持久收件与重启/重放、结果及退休闭合、V7 认证来源、
V1/V2 耐久发送、V4/V5 本地接受/结果、V6 文件/VM 事实及全链 E2E。ContentDigest 的值往返仍
不证明摘要计算或材料真实完整。
