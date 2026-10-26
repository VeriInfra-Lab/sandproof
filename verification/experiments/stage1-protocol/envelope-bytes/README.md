# 协议 4 封套的原字段回调与条件字节组合

本候选接续[完整字段表示](../envelope-schema/README.md)和[七类角色叶子](../schema-leaves/README.md)，
范围由 [V3 计划](../../../../docs/formal/03-protocol-inbox.md)及[逐符号依赖](../../../../docs/formal/dependencies.md)界定。
生产协议固定为 a9c；完整 V3 与 ning 的共享验收仍待完成。

`EnvelopeBytes.lean.in` 枚举实际 Accepted 封套中的全部自有 ID/计数器字段，将各字段的
原生成回调接到原 `SliceFieldReader`；Recorded 路径明确增加 `intent`。一般性定理证明
回调成功给出全部自有字段的值域、原编码转换可被原回调读回、切片身份保持和自有错误
先于 document end。容器修正后的错误位置与原回调的 0/0 位置分开。普通整数、版本、
slot、有符号时间、布尔值和完整字节数组继续由原字段表示保存，不额外要求非零。
五种正常见证与七类角色字段遗漏反例使用同一值域；这些见证不是 Rust 错误版实验。

`SliceContract` 是**显式条件的组合接口**，沿用既有 A6/B4 的字段读取与标准派生分工。
在该条件和成功输出前提下，原回调与 #703 的完整单射定理推出 Accepted/Recorded 的
完整值往返，并复用原守卫首错及完整匹配定理。这里没有生产 parser 的已接纳实例，
也不证明编码必定完成、字节预算或服务器 Writer。四个真实 `from_slice`/`to_vec` 薄包装
的原 LLBC、完整调用体、原类型、派生字典和 unwind 路径只作为接口绑定审查材料；
不得把它们或有限原生回归当作整个 Serde 的证明。#703 的翻译失败证据继续保留。

## 独立抽取与检查

当前 V0 的 `lane_current` 经 `RetentionLane.last_retired` 引入新的协议枚举，和独立
Request 视图生成同名全局判别器。`extract-json.py` 从已核对的原抽取命令选择相同选项，
仅排除本组件不使用的 lane 根，生成独立 JSON 视图。原公共脚本、完整 V0/lane 目标及
其验收范围保留；本组件不能签发全 V0 通过。生成体不手改，三个原 provider 文件逐字节
复用已审查候选。公共 Lake 路径选择以 `shared-candidate.patch` 交主控，只在私有副本应用。

持共享重锁、固定工具和分配的包缓存，在独立 `.lake/build` 中运行：

```sh
python3 verification/experiments/stage1-protocol/envelope-bytes/formal.py --output /private/envelope-bytes
```

驱动核对旧输入和共享候选、重新抽取原 map/JSON/Request 及四个字节包装，保留完整原
LLBC，比较生成物和完整 AST，再经原严格物理文件服务与内核检查全部导入闭包及审计。
`--prepare-only` 只准备，不能报告证明通过。缺失审计、输入变化、部分结果或旧结果失败。
本候选的 `complete=true` 仅表示这一局部条件组合通过。

本地开发测试允许先行；ning 负责正式普通/原生回执，主控负责统一入口与共享验收：

```sh
cargo test --locked --offline --manifest-path verification/experiments/stage1-protocol/envelope-bytes/Cargo.toml
cargo clippy --locked --offline --manifest-path verification/experiments/stage1-protocol/envelope-bytes/Cargo.toml --all-targets -- -D warnings
python3 -m unittest discover -s verification/experiments/stage1-protocol/envelope-bytes -p 'test_*.py'
```

新目标须进入原 `verify-stage1-protocol.sh`、公共模块/原生登记及覆盖候选；不单跑旧入口
报告新版完成。原 key-only、same-incarnation、session-only 等真实错误版及其完整合同
保持不变。标准派生实例、正常完成与容量界、实际 Writer、持久收件及系统组合仍待证；
V1/V2、V4/V5、V6、V7 的业务事实与认证上下文责任保持原分工。
