# 协议 4 封套的真实有界 Writer 候选

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[工作流](../../../../docs/proof-workflow.md) · [唯一依赖登记](../../../../docs/formal/dependencies.md)

本切片接续 #705 的完整 Accepted/Recorded 字段、字节条件与大小界，生产仍固定
`a9c9967889c4cf298e4378c7106948018835fc64`。隔离候选从当前 `encode_bounded`
移出原初始化、写入和结束判断；先逐字核对历史 Writer 的原写入、结束和响应片段。
生产文件不修改，旧冻结不刷新。一般旧新等价仍是显式 B4 来源条件。

`EnvelopeWriterContract` 使用既有 `ActualWriter`、`WriteContract`、`RustHistory`、
`finishSpec` 和普通 Vec 模型，不另建协议值或 Writer 策略。新增 `SerializeTrace`
只描述固定标准派生有序调用实际 Writer、在首次拒绝后结束的轨迹；未写完的剩余块
仍在轨迹参数中。`StandardCalls` 是该确切调用的 A6/B4 条件，不能套用任意自定义
Serialize，也不因有限回归或接口类型成立就说生产实例已被采用。

已写出的对应性质覆盖：

- 实际初始化把原完整值、原 limit 和同一新 Writer 交给 serializer，再把它的原返回
  Writer/result 交给真实 finish。发散不变成成功；`exceeded` 优先于 serializer 错误。
- 全部成功写块精确连接、原字节界；拒绝保留此前前缀，不回滚。分配拒绝保留标记，
  不被解释为超限。普通 Vec 正常分配模型下，任意有限且总长合限的块有完整实际调用轨迹。
- Accepted/Recorded 复用原完整字段规格与 `SliceContract`，组合成功 Writer 的精确
  字节回读。实际固定标准调用/renderer/parser 实例仍待采用；没有证明整个 Serde。
- 正确版和吞掉 serializer 错误的真实 Rust 版共享 `EncoderContract`。正常非空与
  分配失败见证可实现；错误版在真实写入拒绝后返回空前缀，形式反例否定同一完整合同。

Writer 的外部错误表示仍复用既有原库模型，不把它与 JSON parser 的另一错误表示静默
等同。这里只用完整字节连接两个视图。完整错误字段/来源、线程与资源析构仍遵守原
A6/B4 范围；OOM/abort、任意有状态 Serialize、异步取消及真实 HTTP/TLS 均不在结论内。

## 当前源与工具接线

`roots.rs.in` 的两个普通根直接调用原 `encode_bounded`，携带完整 Accepted/Recorded
和原 limit。`binding.py` 核对实际单次调用 AST、两个参数、标准派生字典、完整类型、
结果和 unwind；完整 LLBC 另外按原规则冻结，不删去不利字段。实际 client/server
位置作为来源输入保留；这里不证明外层路由、handler 或持久收件。

`prepare.py` 只在复制的源码内复用历史已审查的 Buffer/response 拆分，并增加静态
`BoundedSerialize<W,T>` 薄接缝。实际实例仍是原 `serde_json::to_writer(writer,value)`。
初次 FnOnce 翻译漏掉可变借用返回值，带关联 Buffer 界的 trait 又触发固定 Aeneas
参数类型限制；失败源码、LLBC、生成物和日志留在 artifacts，均不计为性质反例。
通用 W 接口通过原固定工具，不修改工具或生成体。

两个生成视图会为同一 `ContractError` 产生重名实例。本视图把该类型接到现有
foundations 的透明完整枚举；原/共享 LLBC 的全部变体、字段、discriminant、泛型及
布局必须一致。它是同一已生成类型的别名，不新增业务不透明类型或公理。生成模板保留
在证据目录且从不导入，外部提供文件由已登记内容逐字安装。

## 复现与验收交接

原工具版本、独立 `.lake/build`、独立 Cargo target 和共享重锁按工作流配置。
主控先协调采用父切片的公共候选，再审查本包 `shared-candidate.patch`；本包检查器
只在私有源码应用这些候选。`public-registration-candidate.patch` 接续父包的登记候选，
为原 `verify-stage1-protocol.sh` 增加正/反版并要求对应 ning 回执，公共文件未直接改写。
覆盖记录也只提供 `coverage-candidate.patch`，不能用局部结果刷新根级旧冻结。

证明环境分别执行，输出目录必须是新的：

```sh
flock /path/to/shared-heavy.lock .tools/lean-mcp/bin/python \
  verification/experiments/stage1-protocol/envelope-writer/formal.py \
  --variant correct --output artifacts/envelope-writer-correct
flock /path/to/shared-heavy.lock .tools/lean-mcp/bin/python \
  verification/experiments/stage1-protocol/envelope-writer/formal.py \
  --variant swallow-error --output artifacts/envelope-writer-wrong
```

入口重新抽取原 map、独立 JSON、Request、四个字节根、预算及当前 Writer；核对完整
原字典/类型/LLBC/生成体，使用原严格 `actual_files` 检查实际闭包与依赖审计，拒绝
stale/partial、漏文件和未登记依赖。原 BoundedWriter 的全部登记审计继续参加。

ning 的原生组件入口：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/envelope-writer/native.py \
  --output artifacts/envelope-writer-native
```

该入口分别新编译未修改生产、正确候选、吞错候选；核对真实自有库与测试二进制，运行
同一完整封套测试及 Clippy。五种操作、两种 Recorded 外层状态逐点注入分配拒绝；
正例检查完整字节和恰好限额，负例必须编译并到达原错误传播断言。构建失败、超时或
导入失败都不算拒绝。锁定 crate 归档和实际解包源码同时核对。`admission_tests.py`
的 AST 错误输入检查另行报告，不冒充真实 Rust 变异实验。

同批 `proof_native_suite` / `verify-controller.sh` 及普通完整验收由 ning 和主控统一
执行、按内容复用。本包不签发共享 native seal。持久收件、GLOBAL→LOCAL、未知提交、
重启、窗口退休和完整系统结论继续待证；字段/字节相等不表示授权、持久接受或资源存在。
V7 认证、V1/V2 意图与耐久发送、V4/V5 本地接受结果、V6 文件/VM 事实责任不变。
