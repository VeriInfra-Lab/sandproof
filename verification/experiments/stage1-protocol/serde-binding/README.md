# 协议 4 标准序列化的实际字段接线候选

[组件入口](../README.md) · [完整字段表示](../envelope-schema/README.md) ·
[Writer 对应](../envelope-writer/README.md) ·
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖登记](../../../../docs/formal/dependencies.md)

最初基于 #706 的精确候选；统一入口现读取本仓库已采用的真实生产函数，旧 a9c 提交只作来源记录。
本包落实既有 A6/B4 条件的具体使用点，不新增协议模型、编码算法或可信业务判断。
`candidate.json` 记录具体来源条件，候选 diff 保留原提案来源；当前公共入口由 `registered.py`
调用原源码检查器。完成接线不等于标准实例已经机器证明，原 `StandardCalls`、
`NormalRenderer` 和字节组合仍带显式条件。

## 原始证据与核对内容

`admit.py` 用原固定 Charon 重新抽取两个 `to_vec` 包装及实际 `encode_bounded`
调用根。完整 LLBC、类型、字典、源文本、unwind 与原父包登记一致；没有删除中间字段、
手改生成体或改变工具。输出保存完整原始文件和全部派生调用记录。

`binding.py` 读取实际声明与引用，`check.py` 将它们接回既有完整字段清单：

- 10 个普通结构体的全部字段、声明顺序、原 `self` 字段、同一容器及精确类型字典。
- 五种操作各自的原枚举分支、标记和完整参数；ExpectedSession 的两个分支另核对
  原相邻标记辅助类型、enum name/index、state 和 version，零版本仍可表示。
- 八个透明包装和六个原 Clone/Into 输出路径，包括完整 32 字节摘要、全部标识符及
  计数器。转换业务仍复用原生成体定理，摘要值不代表实际计算过哈希。
- 每个结构体/枚举分支的初始化、字段调用和结束使用同一容器；后续调用只在先前
  Result 的 Continue 分支，Break 返回同一原错误。完整函数摘要还绑定所有未在摘要
  视图中解释的返回、析构和展开语句。

Recorded/Writer 根各含 26 个实际 Serialize 体，Accepted 根为其中 25 个。
这是具体标准派生的源码准入与审查证据，不是一个新的 Rust 解释器，也不声明 Python
检查器证明了编译器或 Serde。原 #703 完整 Serde 翻译障碍仍保留；本包新增库精化
定理为零，原字段单射、原回调、预算、Writer 定理及真实吞错形式反例继续使用原版本。

## 固定库的责任

`library.py` 同时核对生产工作区、Vec 包装、Writer 包装和原生全文用例的实际依赖
特性，以及固定 Serde/serde_json/itoa 和两版 Rust 的具体库源。实际生产/Writer 开启
`raw_value`，普通 Vec 包装没有；当前全部结构体名称均不同于保留名称，不能把两套
编译配置静默视为相同。相邻标记辅助、标准数组/Vec 迭代、紧凑 Formatter、错误包装
和 `write_all` 的逐符号范围在候选中明确列出。

这里的 A6 条件只覆盖这些普通值和标准派生：成功输出保持既有完整字段，正常分配时
满足原结构上界；Writer 的非空块全部写入或返回原非 Interrupted 错误，空块无需调用
write。失败可保留私有输出前缀，不假设回滚或任意两次有状态 Serialize 输出相同。
自定义 Serialize/Drop、任意 IO 错误源、panic/unwind 收尾、OOM/abort、异步取消、
HTTP/TLS 交付及授权/耐久性仍由原模块负责。

## 执行与交接

本地开发可执行；正式普通/native 由 ning 验收。使用独立源码/target，保留失败证据：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/serde-binding/native.py \
  --output artifacts/serde-binding-native
```

该入口新编译原版、真实遗漏 Recorded.terminal 的 Rust 版及恢复版，三者调用同一个
既有 `actual_five_branch_fields_and_roundtrips` 全文断言，并运行 Clippy；实际协议库
和测试二进制均须新编译。错误版必须运行到原断言，零测试、编译失败和超时都失败。
错误版的注解仅在私有源码应用，生产不修改。

证明机持共享重锁运行源准入和真实错误版抽取：

```sh
flock /path/to/shared-heavy.lock .tools/lean-mcp/bin/python \
  verification/experiments/stage1-protocol/serde-binding/admit.py \
  --output artifacts/serde-binding-source
flock /path/to/shared-heavy.lock .tools/lean-mcp/bin/python \
  verification/experiments/stage1-protocol/serde-binding/negative.py \
  --native-evidence /private/serde-binding-native \
  --output artifacts/serde-binding-omission
SANDPROOF_SERDE_BINDING_LLBC=artifacts/serde-binding-source/source/artifacts/bytes-recorded_encode.llbc \
  bash scripts/python-entry.sh -m unittest discover \
  -s verification/experiments/stage1-protocol/serde-binding -p 'test_*.py'
```

跨机仅交接该组件的 `result.json` 与三版 test/Clippy 日志，不复制 Cargo target。
`negative.py` 核对完整当前输入和所有日志，从本机相同源重新应用精确遗漏，再用固定
Charon 抽取并达到同一个字段拒绝。它是实际源码准入反例，不能列作新增 Lean 形式反例。
另有八项实际 LLBC 准入回归，包括同类型字段串线、遗漏、错误字典和首错路径变异。

公共候选接续 #705/#706 的登记；`SANDPROOF_SERDE_BINDING_NATIVE` 指向上述原生记录。
原 `verify-stage1-protocol.sh` 应同时保留全部父证明，并运行本包新的完整输入核对，
不能只跑旧根报告新接线通过。共享 `proof_native_suite`、`verify-controller.sh` 由
主控同批执行和按内容复用，本包不签发共享 seal。

具体 parser 的缓冲枚举字段、容器/文档收尾与错误位置仍未完成，不能声称完整 JSON
链已证。实际路由/B4 采用、上游类型迁移、持久收件、GLOBAL→LOCAL、未知提交/重启、
确认与窗口退休、系统组合和 ning 完整验收继续待办。V7 认证、V1/V2 当前意图与耐久
发送、V4/V5 本地接受/结果及 V6 实际文件/VM 事实的提供责任不变。
