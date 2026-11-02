# 原生快照的原操作完成条件

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包只选择真实 `NativeCreateSnapshot::has_completed_original_execution`。
正确准备视图追加一个透明转发根；错误视图仅使原 `Succeeded` 条件恒真。
生产、V4核心及公共配置/登记/冻结不由这个子包修改，生成体保持工具原输出。
本包是独立候选，不自动加入已冻结的主控验收批次。

`FullCompletionContract` 对任意完整原 snapshot 和 local binding 给出实际函数与
`CompletedOriginal` 的双向关系：owner、Running、非terminal、非零代、VM存在于字段中、
无不确定动作、原操作存在且编号/epoch相等、类别为Create或Resume、Create当且仅当
第一代、状态为Succeeded。只有 `Vec.is_empty` 沿用原Foundations登记；其余原方法与
派生比较透明抽取，没有新可信接口或另一份协议模型。

原函数不检查lifetime内容、binding的sandbox或具体VM身份。一般不变引理和
lifetime=None的正常见证明确保留这些边界。正常Create/第一代与Resume/第二代、
17组字段拒绝使用同一个完整合同；有限值不代替一般性定理。
真实ignore-status版实际返回true的Pending见证否定同一个完整合同，且两版Model/Values
逐字相同。错误版不导入正确实现定理。编译/导入失败或超时不算性质拒绝。

## 执行与边界

持既有共享重锁，复用固定工具和包缓存；每个variant有独立source/target/.lake/build：

```sh
python3 verification/experiments/stage1-protocol/native-readiness/formal.py \
  --variant correct --output /private/native-readiness-correct
python3 verification/experiments/stage1-protocol/native-readiness/formal.py \
  --variant ignore-status --output /private/native-readiness-wrong
```

`--prepare-only`仍真实编译/抽取、核对IR/生成符号与准入回归，并装配完整物理文件闭包；
prepared不表示完成内核/服务检查。`--prepare-only --require-native`拒绝。
默认CLI仅提供开发检查；正式入口须带`--require-native`，消费主控登记的
`stage1-protocol-native-readiness` case和当前functional回执。未登记/缺失/损坏/过期不得
绕过。公开登记候选另交主控，不能仅运行旧根报告本子包已完成公共验收。

普通与原生验收由ning执行：

```sh
python3 verification/experiments/stage1-protocol/native-readiness/native.py \
  --output /private/native-readiness-native
```

保留原readiness测试，在原模块中附加两个相同断言，分别运行original、ignore-status、
restored。每版独立source/target，核对真正新编译的原协议库/业务测试、package/manifest/src、
二进制及精确断言/失败集合，Clippy和格式继续执行。预期正确/恢复两项通过，错误版仅
字段检查失败，scope断言通过；只有实际运行并封口后才能采用这个期望。
原生服务单测使用原workspace Cargo.lock；抽取使用原inbox-record Cargo.lock。
`--require-native`按workspace锁校验两机原归档/实际解包内容，不能把物理路径当内容身份。
不复制Cargo target，不签发共享seal；原全组件/基础检查仍由主控统一验收。

本函数只检查已提供的普通值。`snapshot.rs`从同一个原World/sandbox取得snapshot、
原操作/历史Running结果、期限、来源/凭据及真实探测的连接仍分别待证；不能从布尔结果
推出整体Ready、授权、持久接受、VM存在或执行已真实完成。标准SQL/SHA、未知提交/
重启、真实JSON/TLS及跨模块资源事实仍属原V3/V4/V5/V6/V7义务，完整V3未因此完成。
