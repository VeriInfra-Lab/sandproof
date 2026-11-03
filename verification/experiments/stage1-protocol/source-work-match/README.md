# 完整 SourceWork 比较

返回 [V3](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)。Refs #635。

固定 `9c50da7cc9391518514f14405e6c779e5c5cbe2c`。本包捕获实际 `SourceWork`
派生 `PartialEq::eq`；隔离副本仅在 lib.rs 加入 `left == right` 的透明抽取根。
生产源码不修改，不随工程分支自动换头。Types/Funs 是原工具输出，禁止手改。

`FullWorkContract` 直接复用现有 `FullReplayContract`，对所有普通已解码值规定
返回 `.ok (decide (left = right))`。正确/错误视图使用同一 Spec、Values、Examples。
`full_derived_contract`、`full_work_contract`、接受/拒绝 iff 和总性覆盖完整 anchor
与 requirement：两份完整 intent、local、native（所有 Command 参数、逻辑效果及
attempt）、continued_from、完整 node/bootstrap/process、全部 options/credential
字节、actions/receipts，以及全部六项观察限制。正常与字段遗漏见证不代替一般证明。

RequestEquality 从已有完整请求比较证明重新在本视图类型上 elaboration；没有
另一份协议模型。字符串、Option、Vec、标量/数组只复用 dependencies.md 的逐符号
A6 合同。凭据 tuple 的实际标准库比较由 `--include core::tuple` 透明抽取，保留
原回调顺序；没有添加可信 Pair 比较或从摘要相等推出内容真实/完整。

两个真实 Rust 副本删除 SourceWork 的派生 PartialEq，分别只比较 work_binding、
只比较 anchor。它们必须实际编译、抽取并到达与全文相等合同矛盾的值；编译失败、
超时、导入失败不算性质拒绝。错误环境不导入完整 work 的 Core/SourceData/Witnesses；
anchor-only 仅复用完整 anchor 的独立比较支持。模板自动生成的 axiom 文件不导入。

形式入口（独立 build、共享重锁、最多四核）：

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/source-work-match/formal.py \
  --output artifacts/source-work-match-formal --lock /path/to/shared-proof-heavy.lock
```

`native.py --output artifacts/source-work-match-native` 使用原 journal 夹具，新增13项
实际断言，包含正常准备值及真实 Codec::save：保留 anchor/key，仅修改观察限制。
正确/恢复须全过，key-only 精确11项失败，anchor-only 精确7项失败；每视图新 target，
格式、严格 Clippy、实际库/程序新编译及逐字节恢复都须核对。Host 输入受控。
原始运行身份、日志与发布树对应在 #635/PR 记录；本机开发不签 ning 正式接纳。

本包从已解码值开始。Values 的摘要只是32字节输入，不声称计算过 anchor 哈希；
正常 SQLite 夹具另使用原代码计算、保存并加载真实工作值。相等不推出授权、
已持久接受、资源存在、JSON/TLS 已证明。实际 `.find` 调用、BTreeSet 的32字节
insert/contains、Codec 投影/逐行键/参数/JSON/SHA/错误传播仍待 V3/V0 连接；
V5/A4 提供物理行、事务/部分效果/未知提交/重启历史，V6 提供材料事实，V7 提供
独立认证。完整 V3 的18项要求/49项义务继续开放。

公共 Lake/登记/依赖/覆盖只提供同目录 candidate.patch，由主控单写采用并运行
真实 verify-stage1-protocol.sh、proof_native_suite 及普通完整验收。局部记录不代替
公共全套，不借旧根宣称本包已接纳。
