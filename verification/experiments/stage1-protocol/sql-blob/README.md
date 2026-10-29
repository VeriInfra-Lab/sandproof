# rusqlite 原完整字节切片转换

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包以精确 integration `ae8e5a564ab3d81652a23acb2b99e242193fa3a2` 的
StandardInboxSave::update 为使用点：原实现把正文和摘要两个完整 `&[u8]` 交给
`rusqlite::params![bytes, digest]`。只读原 codec，不修改生产、库源、工具或生成体。
独立 crate 固定原 rusqlite 0.40.2 和 bundled 特性；完整依赖锁条目在规范化依赖名称后
必须与生产锁逐项一致，不能由一次重新解析引入不同版本。

## 已检查的转换关系

两个薄根分别转调原 `[u8]::to_sql` 和 `&[u8]::to_sql`。固定工具透明抽取原
ValueRef::from、ToSqlOutput::from、slice ToSql、共享引用 ToSql 及两根，共六函数，
外部函数体为零。保留原 ValueRef、Value 的全部五种变体和完整载荷，以及
ToSqlOutput 的 Borrowed/Owned；没有删除无关变体来通过检查。

SqlBlobValues 对任意有限可表示切片证明：原转换总返回
`Ok(Borrowed(Blob(bytes)))`，保留完整原字节，输出相等当且仅当全部字节相等。
共享引用实现透明转发原字典，不增加验证、编码或哈希。两个独立转换的推论同时
保留两份完整字节，但**没有据此推出原宏顺序或 SQL 参数槽已经连接**。
空值、NUL/无效 UTF-8 正常见证，以及等长但末字节不同和互换值的反例补充一般定理。
这些是完整值反例，没有新增实际 Rust 错误版，也不替代既有错误版验收。

## 载体与工具边界

rusqlite::Error 只复用已有 SandproofSqlite.TypesExternal 的错误身份载体；本视图
从不构造或检查它。原错误枚举含动态 trait 字段，直接透明展开的失败证据保留。
这不是将原自有错误分支设为可信，也不赋予错误文本、引擎状态或提交结果语义。
TypesExternal 另复用已有 SandproofFoundations 的 F64=BitVec64，保留两个 Real
变体的全部惰性载荷。本包没有浮点运算、比较、转换或格式化；当前 SQL 类型使用点
的载体采用仍交主控/V0 确认。不会从 Blob-only 定理声称 Real 编码、T9 或整个库正确。

其余边界沿用固定 Charon/Aeneas、Lean 内核和普通有限 Slice/Result 值表示。
没有新增库操作合同、自定义公理、sorry、业务 opaque 或生成体修改。开发数学检查
通过不等于当前公共入口已采用这些载体或原生验收通过。

## 原宏仍未接通

另一个 original_params 根使用原 params! 宏及原动态 dispatch，在作用域内观察两个
返回值；没有替换成语义可能不同的 tuple Params。实际编译和 Charon 捕获成功，
固定 Aeneas 在原 `rusqlite/src/lib.rs:198` 的 unsized dynamic-trait cast 拒绝翻译。
完整 IR、残留生成物和失败保留；没有填其外部模板、删字段、关闭检查或修改工具。
预检失败始终返回非零，property_rejection=false，不计为性质拒绝。

主控/V0 的最小协调项是该实际两项借用切片的原 cast/dispatch 表示，以及上述惰性
载体在当前使用点的采用；不是整个 Params、Connection.execute 或收件事务正确。
已有固定五元组 UPDATE 合同不覆盖原两项 params! 宏，不能直接迁记。
宏到 SQL 位置、绑定错误/行数/同一事务/提交未知和重启仍开放。正文 JSON 编码、
实际 SHA256、来源与授权、资源真实和完整持久收件结论也没有由字节载体得到证明。

## 复现与公共采用

```sh
python3 verification/experiments/stage1-protocol/sql-blob/development.py --output /private/proof/sql-blob --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/sql-blob/preflight.py --mode params --output /private/proof/sql-params --lock /private/proof/shared-proof.lock
```

使用既有共享重锁并在外层限制四核。每个输出独立源码、Cargo target 和 .lake/build；
development 重新编译/Charon/Aeneas，核对实际原样生成文件和完整导入/审计清单，
通过原 actual_files 检查器逐文件跑内核、同一服务完整非陈旧诊断及依赖审计。
完整原源码/执行位、固定库源和私有衍生输入在结束时重新核对。preflight 只表示
翻译可行性，从不签发 proof_complete；params 模式当前应失败而非绿色接受。

普通/真实错误/恢复原生仍交 ning。公开 formal/native/shared 登记、Lake、冻结和覆盖
由主控统一采用；本包不追加当前 owner 固定窗口或新开迁移 PR，也不跑旧根报告新版
通过。独立开发 complete 不表示 SQL 接线、公共接受或完整 V3 完成。
