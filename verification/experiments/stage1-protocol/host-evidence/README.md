# 宿主证据五方法的独立值对应候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包从 integration 精确提交 `ae8e5a564ab3d81652a23acb2b99e242193fa3a2` 开发，
证明对象是透明根所调用的实际 HostEvidence 方法及 recorded_source_identity。
**原生产五方法的完整对应尚未证明。** 原源码捕获成功，但固定 Aeneas 在
RuntimeProcessIdentity::validate_fields 返回处报 no-bottom；单独拆分守卫仍失败，
再提取布尔条件后又遇到 Iterator.all 元数据字段缺失。原失败没有计作性质拒绝。

`source-guard-candidate.patch` 只供 V6/主控审查，未写入生产。它把同一进程字段条件
集中到一个布尔方法，原错误接口仍使用该方法并保留错误；原两个 Bool/Option 消费处
复用此判断。boot 判定保留长度、字节顺序、首拒绝和原字符规则，用按值捕获的 usize
计数闭包接已有 Bytes.all。原负向守卫生成顶层 Prop/Bool 类型错误的记录也保留，
检查对象是重新抽取的正向守卫候选；没有手改生成体、修改工具或过滤字典字段。
候选的文档收尾、来源等价审查、实际采用与受影响验收仍需完成。

## 同一完整值合同

HostEvidenceModel 只导入类型、可判定实例和纯 boot 词汇，不导入正确实现证明。
FullHostValueContract 同时描述 validates、capture_manifest_digest、capture_material_bytes、
recorded_source_identity 和 source_process_identity；正反版本共享逐字相同的规格和值。

完整命令相等覆盖十二种操作、恢复来源、完整网络策略及有序向量、端点地址/协议/端口、
所有编号与操作参数。验证先比较该完整命令，再检查原结果类别与载荷条件；
Prepare/Activate/Stop 只检查非零 PID/start，Freeze 另检查完整进程字段。
不能给 Prepare 擅加 boot 或 PID 上界条件。录入来源先比较独立期望资源，再检查命令、
原结果和完整 PID/start/boot；返回及投影保留同三个字段。

任意有界后缀的计数更新、短路和剩余字节，以及每个 boot 字节索引的完整条件已证。
StringRepresentable 只是原有效字符串表示域，不是假定字段合法、来源可信或已经授权。
十二类正常形状和真实 VM 参数/独立资源拒绝见证补充一般定理，不代替任意值对应。

## 精确边界与依赖

原完整 HostObservation 三字段、十二种 ObservedResult 全载荷、完整 Command、资源和
来源身份均保留。八个外部绑定只复用原 Bool.ne、Option.eq、Bytes.all、Str.len/bytes、
String.eq/clone/deref；Bytes 直接别名到原载体。ASCII digit、RangeInclusive/RangeBounds
及 Option.map 由固定工具透明抽取，没有给它们添加可信模型或导入公理模板。

VectorComparison 模板逐字复用原 Access.Guards 的两个泛型比较证明正文。
`shared-vector-comparison-candidate.patch` 供唯一集成人把它们移入纯模块，保留原名称和
其它证明正文，避免导入两代生成类型；本包没有并发修改公共文件，也没有另造协议模型。

一般边界定理明确：validates 不检查 observation.resource，亦不核验观察中的32字节
摘要；摘要和材料方法只是保留原值。返回来源身份不证明进程当前存活、材料真实、
资源存在、已授权、已持久接受或整个 Ready。实际构造/观察及独立 action 接线交 V6，
认证来源交 V7；原 probes_match、JSON/Serde 字节、SQL/SHA、事务未知/重启仍分别开放。

## 检查与采用状态

inventory 记录原五个透明根、固定 include/目标、原始输入、模板和独立开发证据身份。
正确与错误 Rust 都实际编译和抽取，生成 Types/Funs 原样；错误版分别只禁用完整命令
守卫、独立资源守卫，均到达同一 FullHostValueContract 的反例。错误视图不导入正确
宿主方法的实现对应；未改的 runtime 字段辅助证明在资源错误版自己的生成环境重查。

全部实际导入文件经原检查器逐个内核、同服务非 stale/partial 完整诊断和依赖审计。
完整开发过程、实际命令/配置、所有失败和原始结果保留在 #635 指定的无缓存证据包。
`development.py` 提供独立复现装配：从固定原源复制私有项目，仅在副本应用两个候选
补丁和至多一个真实 Rust 守卫变异，重新编译、抽取，并核对原 IR 的完整业务字段、
变体及标签、透明方法、完整捕获选项和精确生成输出。它复用原 actual_files 检查器，
不复用先前的成功状态。每版使用独立源码、Cargo target 和 .lake/build；只共享固定
工具和包，保留所有失败。原项目内容与文件执行位也纳入输入记录及结束复查。

在已配置的证明环境中，分别执行以下三条命令，使用同一个既有共享重任务锁。
输出目录必须尚不存在。`--prepare-only` 仍实际编译和抽取，但不运行内核/服务检查，
不能把 prepared 当成 complete。

```sh
python3 verification/experiments/stage1-protocol/host-evidence/development.py --variant correct --output /private/proof/host-correct --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/host-evidence/development.py --variant skip-command --output /private/proof/host-skip-command --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/host-evidence/development.py --variant skip-resource --output /private/proof/host-skip-resource --lock /private/proof/shared-proof.lock
```

本入口不签发正式/native 回执；正式/原生登记、根导入、冻结和覆盖由主控协调后续采用，
不能跑旧 verify-stage1-protocol.sh 报告本包已验收。
普通及正确/错误/恢复原生检查仍交 ning，当前封口窗口不追加本包；完整 V3 仍未完成。
