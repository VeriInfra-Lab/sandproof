# 当前生产 SHA 实例的原根与库绑定预检

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包核对精确 integration `ae8e5a564ab3d81652a23acb2b99e242193fa3a2` 的三个生产使用点：
source_work::digest、StandardConfigRead 的 InboxLoadEffects::digest、StandardInboxSave
的 InboxSaveEffects::digest。生产、依赖源码、工具及公共配置均只读，不复制哈希表达式
或提供新算法。独立副本只追加转调原函数/实例的公开薄根，继续使用原已锁 inbox-record
清单、默认 durable 特性和固定工具。

## 原根、接线与未闭合边界

原 source_work 函数把同一完整输入交给固定 Sha256::digest，接原 GenericArray 到
`[u8;32]` 转换，再调用真实 ContentDigest::from_sha256。两个原标准实例返回同一转换
结果；实际生成函数正常返回时带回原 self 值。两个自有实例类型的完整 connection
字段显式捕获，Connection 本身仍为未填的库类型。这里的生成体/字段审查不推出
数据库快照不变、事务成功、认证、材料真实或实际摘要正确。

source-work 视图保留原私有 digest 和既有载体构造共两透明函数；standard-instances
视图保留两个实际标准 digest 方法。两者各有62个未填外部函数、25个未填库常量，
分别有8/9个未填外部类型。它们包括实际 Digest 泛型字典、GenericArray/typenum/CoreWrapper 等绑定；
不是本包接受了62项可信条件。生成器的 axiom 模板只留作原始证据，**不导入、不填体、
不运行这些残留的 Lean 文件**。没有新的模型、定理或 native/formal/shared 回执。

现有 CertificateDigest/Sha256CertificateDigest/DigestCorrect 合同明确适用于旧 TLS
隔离候选，不能自动变成上述当前实例的 A9 证明。需要主控/V0 先确定唯一窄库绑定或
实际表示的采用方式：固定 Sha256 实例在原完整只读字节上的正确摘要及正常返回条件；
原 GenericArray32 转换保持全部32字节和顺序；固定具体类型/字典及 Std AsRef/Into
接线。不能把任意自定义 Digest 实现当作 Sha256，也不假设摘要全局单射。
本包不建议逐个任填62个模板，更不把整个自有 source_work/codec 函数设为可信。
V3 仍负责实际三个函数的参数、值包装和调用对应。

另有 blanket 独立视图：借用切片薄根转调原泛型 Digest::digest，透明捕获原
default→AsRef→update→finalize 控制流及两个原标准 AsRef 转发。GenericArray 直接
保留原 data 关联类型字段，无外部类型或库常量；四函数中三透明，只有原 finalize
尚为未填外部体。这是该控制流的抽取，不是其 Lean 对应或具体 Sha256 状态的算法证明。
具体实例的 default/update/finalize 及原 GenericArray32 转换仍须由唯一库绑定落实。
库的转换源码实际使用 generic-array::transmute/ptr::read，不能用构造值往返替代它。

32字节值往返不证明执行过 SHA256。即使该窄库绑定落实，JSON 正文选择、原文件和
材料、SQL 参数/列与同一事务、提交未知/重启等仍是独立义务。

## 实际根存在性

首次直接选择私有函数时，LLBC 虽有透明原体，Aeneas 最终输出只剩载体构造。不能
把这种“翻译成功”称为包装已抽取。公开薄转发根使原生产函数/方法保留；转发别名
自身可被预处理消去，所以最终准入核对的是**实际生产函数的非外部体**，不是薄别名。
两个按薄别名检查的过严准入失败同样保留，不当作 Rust 或逻辑反例。

驱动实际拒绝原缺根捕获和把目标改成外部体的元数据回归；这些是准入检查，不冒充
实际 Rust 错误版的性质拒绝。原捕获、失败、生成模板和输入记录不改写。

## 复现与交接

```sh
python3 verification/experiments/stage1-protocol/digest-instance/preflight.py --mode source-work --output /private/proof/source-digest --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/digest-instance/preflight.py --mode standard-instances --output /private/proof/standard-digest --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/digest-instance/preflight.py --mode blanket --output /private/proof/digest-blanket --lock /private/proof/shared-proof.lock
```

外层限制四核，使用同一既有共享重锁；每个输出独立源码/Cargo target/.lake/build。
核对清单列出的原源、锁文件、库源、两版标准 AsRef 原源、固定二进制和驱动，记录完整源码字节与执行位；
私有薄根衍生及所有清单输入结束复核。opaque/exclude/跳过借用/类型检查及单态化选项
不得另加。capture_complete/translation_complete 只表示这些阶段及实际根准入；
proof_complete 始终 false。固定抽取前端的原 AtomicCounter fetch_update 弃用警告
保留，不能称普通检查无警告通过；不为消去警告修改无关生产代码。

较宽的泛型库入口曾在固定前端查询 InvalidBufferSize 构造器 fn_sig 时 panic。
原失败保留；缩小根和 include 后才得到上述 blanket 视图，没有更换工具、删除生成字段、
改写库源码或把该编译失败计作性质反例。原 c462e94a 两个公开视图保持原身份。

普通/原生仍归 ning，公共模型/登记/覆盖和正式验收由主控统一采用，不追加当前固定
窗口或新开迁移 PR。完整实际 SHA/JSON/SQL、持久收件和完整 V3 保持开放。
