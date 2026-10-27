# 协议 4 的透明角色叶子候选

本切片接续 #688，不把旧协议 2 的 JsonRequest schema 迁记为协议 4 已证。
七个根直接调用当前真实 `SessionId`、`CenterId`、`IncarnationId`、`CaptureId`、
`RequestWindowId`、`RequestSequence`、`RetentionWindowId` 的泛型 Deserialize。
它们与原 V0 同一次生成，共用原类型、编号、非零计数器、错误字典和证明；
生产源码、公共配置及冻结没有直接修改。

`RoleLeaves.lean.in` 的七条一般性对应证明实际包装恰等于原叶子调用，包含相同
解码器、完整输入及所有返回、失败、发散和外部效果。其余定理复用已有叶子合同：
合法文本/非零数值原样保留，非法编号或零被拒绝；在原 SliceFieldReader 绑定条件下，
原 custom 错误先于文档结束检查。四条见证各对全部对应角色成立，仅说明值读取条件
可实现，不是 JSON 扫描器或具体 from_slice 字典已证。

这些角色是字段的类型，不是独立授权来源。`accepted_version`、ExpectedSession 的
Version、retention slot 等普通整数并不因此变成非零计数器；后续完整 schema 必须保留
它们的原可表示域及守卫顺序。ContentDigest 不属于这七个叶子，其 32 字节值不表示
计算过哈希。真实全文 schema、完整字段覆盖、实际 parser 输入/字典调用位置、对象及
数组形状、未知/重复/缺失字段和字节预算仍待下一切片，不能由包装对应直接推出。

## 共享候选与固定工具

`shared-candidate.patch` 保留为历史来源，当前驱动不再应用。独立视图登记七根及薄 Rust 根、
独立 Result 生成库和指向其真实生成函数的别名。固定后端没有可用 Result::map 入口；
直接在原 V0 生成该标准函数又会被已有 Result::ok 的名字遮蔽。因此 `extract-map.sh`
用同一固定 Charon/Aeneas 独立抽取原标准函数体，再让包装的既有调用名引用它。
没有手写 Result::map 算法、导入外部模板公理、修改工具或删改生成函数体。

历史候选曾因标准 derive 的生成名称配置名称风格豁免。当前独立
`local-view.toml.in` 不包含该豁免，不改生成名称或正文，也不放宽原严格服务的警告、
错误与依赖拒绝。如果固定工具的原产物仍触发风格诊断，必须保留为未通过的阻断。

V0 候选尚未公共接纳。使用已有逐符号字符串、counter、Serde 默认 visitor、错误格式化、
FnOnce 与效果合同；固定 preset 的 Drop 擦除条件继续适用。泛型 D/E 及实际 SliceRead
使用点必须没有未建模的可观察业务析构、资源持有或取消语义，不能据此认证任意资源型
解码器。两个原始 LLBC、配置、生成体和所有实际导入文件均保留供核对。
新增七根各有三处条件清理；标准 map 的错误分支有一处精确回调 Drop，薄 map 根有七处
条件清理。角色调用仅传零捕获包装闭包，不从本证明推出任意资源型 FnOnce 的 Drop 行为。

## 开发检查和 ning 交接

在分配的固定工具及包缓存就绪后，持有共享重锁运行：

```sh
python3 verification/experiments/stage1-protocol/schema-leaves/formal.py --output /private/role-formal
```

驱动复制当前完整输入，安装独立视图及透明根，重新生成七根及 Result 根，使用独立 Rust target
和 `.lake/build`，构建后由原 strict 物理文件服务检查全部生成文件、证明导入闭包和准确
审计。`complete=true` 仅表示本候选局部形式检查，不是公共接纳或完整验收。

ning 可用同入口 `--prepare-only` 只准备精确源码，不运行抽取或 Lean；该模式明确返回
`prepared=true, complete=false`。在输出的 `source` 内，以原分配的 target 运行：

```sh
cargo test --locked --offline --manifest-path verification/extract-foundations/Cargo.toml protocol_leaves::tests::
cargo clippy --locked --offline --manifest-path verification/extract-foundations/Cargo.toml --all-targets -- -D warnings
```

八个原生用例覆盖七角色的正常/边界值、精确序列化、零/非法编号拒绝、尾随字节首错及
标准 map 的成功回调/错误透传。它们是回归测试，不代替一般性定理或实际 Rust 错误版。
本切片没有新增变异验收；此前 request/envelope/Create 等真实错误版仍由各原入口保留。
共享 `proof_native_suite`、`verify-controller.sh` 和原 `verify-stage1-protocol.sh` 的新目标登记
交主控同批接入；旧根通过不能表示本候选通过。ning 的普通/原生与公共完整验收另报。

独立认证由 V7，原意图/耐久发送由 V1/V2，本地接受和结果由 V4/V5，文件/VM 事实由 V6
提供。#688 接线候选的生产采用、完整 JSON/TLS 链及持久收件不在本切片完成结论内。
