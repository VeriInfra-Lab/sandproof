# 原摘要与数组解码的固定工具预检

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包固定 integration `ae8e5a564ab3d81652a23acb2b99e242193fa3a2`，直接调用原
ContentDigest 的透明派生 Deserialize，以及其原标准32元素数组接口。生产和库源
均不修改，不复制解码算法；薄根只是原方法转发，生成文件原样保留。源、锁文件、
std+raw_value 特性、固定 Serde 1.0.228 / serde_json 1.0.142 及实际工具均须核对。

本包没有新定理、库模型或 native/formal 回执。它把固定工具障碍与已证明结果分开，
不将编译或抽取失败当成性质拒绝，也不把工具生成成功当成解析证明。

## 三个原始视图

| mode | 实际范围与当前固定工具结果 |
|---|---|
| digest | 真实 ContentDigest 派生及其原映射闭包均已透明捕获；Aeneas 在 u8 visitor、范围错误闭包与 trait 实例的混合递归处拒绝 |
| array-constructor | 可生成数组 Deserialize 转发，但没有 visit_seq，ArrayVisitor::new 仍是外部体；没有给它填模型，不能称32元素消费已证 |
| array-callback | 另加泛型 Visitor::visit_seq 原调用，使原数组回调进入捕获；Aeneas 拒绝 Deserialize / DeserializeSeed / Deserializer / Visitor / SeqAccess 的相互递归类型，生成残留不可用于证明 |

第三个视图没有具体 u8 字典，因此与第一个混合递归障碍不同。删除 SeqAccess、Seed
或 visitor 方法字段虽可能得到较小类型，却会失去实际调用闭包；本包不这样处理。
标准数组 visitor 消费元素后还须由实际 deserialize_tuple 检查元组收尾；不能只凭
visitor 成功就推断拒绝多余第33个元素。原32字节值表示也不证明计算过哈希。

## 复现与保留

证明环境使用已注册固定工具，持既有共享重任务锁，每次给尚不存在的独立输出目录：

```sh
python3 verification/experiments/stage1-protocol/serde-digest/preflight.py --mode digest --output /private/proof/digest --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/serde-digest/preflight.py --mode array-constructor --output /private/proof/array-constructor --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/serde-digest/preflight.py --mode array-callback --output /private/proof/array-callback --lock /private/proof/shared-proof.lock
```

工具失败时入口保留原日志、完整 LLBC、生成残留和输入记录，仍返回非零，不将已知
失败改成绿色验收。capture_complete / translation_complete / proof_complete 分别记录；
最后一项始终为 false。没有运行 Lean，不能签发局部物理文件通过。普通/native 仍归
ning，正式入口和公共登记仍由主控协调；本包不追加当前冻结验收窗口或新开迁移 PR。

## 最小后续接口与自有责任

V0/主控需要协调固定库的以下连接及具体采用方式，本包没有新增或预先接受这些假设：

- 原数组 deserialize_tuple 的长度32、原 visitor、同一元素序列和原 Result/首错顺序。
- 原 SeqAccess/DeserializeSeed 的逐元素调用与状态，以及元组结束检查；包括少元素、
  元素错误与额外元素，不假定错误回滚。
- 实际 u8 visitor 的整数范围、原错误回调及 expecting 来源，不能让任意整数截断成字节。
- 原 JSON SliceRead 同一输入与游标；已有条件尾随空白定理不能代替真实 peek/discard
  实例，也不提供完整字段到原字节的关系。

V3 仍负责自己的透明摘要包装、完整协议字段/visitor/错误顺序与原输入的连接；
这些自有义务没有转交成整个 Serde、handler 或事务正确的前提。实际 JSON 全链、
SQL/SHA、提交未知/重启、持久收件及相邻业务事实继续按 V3 计划保持开放。
