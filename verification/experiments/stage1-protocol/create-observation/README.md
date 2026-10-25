# Create 观察值与完整校验合同

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

固定 #691 `4f52d972a368596c0a9d4a8fff8913e757dfc778`，生产源码保持原 a9c 协议 4。
本目录准备实际 `CreateObservation::matches_request` 的完整合同及检查输入；
`CreateObservationEquality` 对实际派生比较证明任意值的完整相等，
`CreateObservationValues` 证明合同的合法见证和字段遗漏拒绝。
这些定理不等于完整 `matches_request` 的实现对应。

`CreateObservationModel` 按原实现保留 Header、Refused、同请求、Unload 继续、Renew 和 NotCurrent 分支，
复用现有 `ReadyFacts.CreateMatches`、`Timing` 与 `ResumeBinding.Continued`。
原始 Create/改参合法见证复用 `ReadyFactValues`；新增续租、卸载、销毁与边界输入使用相同封套合同。
Refused 必须没有历史成功，但其他 Pending 值不能解释为没有资源效果；
Renew/NotCurrent 的部分分支允许复用 key，非 Refused 改变当前请求的分支也不总重新校验旧请求。
不得据值相等推出授权、原意图、耐久拒绝、已应用续租或实际就绪。

`observation-bindings/extract.py --profile create-values` 保留原八个请求/事实/Resume 根，
增加实际 Create 观察值派生比较。`--profile create` 则请求完整 Create 与续租事实入口，
后者仍受固定工具的 `TryFromIntError` 表示差异阻断：原转换返回 `IntErrorKind.NegOverflow`，
固定后端将 `TryFromIntError` 映为 `Unit`。最小 `u64::try_from(i64)` 成败/数值/错误载荷连接交 V0，
不手改生成体、替换转换算法或把整个守卫设为可信。

四个错误版只在隔离生产副本修改一个已登记原表达式：响应只比 key、遗漏当前 retention、
Refused 允许历史成功、负数续租强转回绕。原生入口逐版检查原版→变异→恢复，
编译失败、超时或导入失败均不计为反例。负数见证使用 startup=1、max lifetime=`u64::MAX-1`、
deadline=-2，原请求与当前请求均合法，避免用溢出的非法 Create 充当反例。
全部正反版本使用同一完整合同及见证；目前原生断言不是实际生成体的形式反例。

开发形式入口只验收上述值支持范围，独立抽取、构建并调用原内核与严格同服务检查器，
核对全部实际文件、保留的 Request/ReadyFacts/Resume 证明与审计、完整非 stale/partial 结果。
输出显式保留 `full_guard_refinement=false`、`formal_mutant_checks=false`；
不能用该入口通过关闭 Create 校验义务。每次持有共享重锁，保留失败证据及独立 `.lake/build`：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-observation/formal.py \
  --packages /private/task/.lake/packages --output /private/create-value-formal
```

ning 在精确提交的 `archive.py` 源码归档上运行普通/原生验收：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/create-observation/native.py \
  --source-archive /private/source.tar --target /private/create-native-target --output /private/create-native
```

该入口检查真实业务库及测试二进制的来源、manifest、fresh=false、摘要和完整源码恢复，
另运行本组件格式、Clippy 与准入回归；不签发共享 native seal。
公共登记、`verify-stage1-protocol.sh`、`proof_native_suite`、冻结与覆盖由主控统一验收。
V1/V2 提供原意图与发送，V4/V5 提供耐久接受及已应用结果，V6 提供 VM/文件事实，V7 提供独立认证来源。
完整 Create 实现对应、JSON/schema/TLS、持久收件、重启/乱序/竞争与系统组合均是另列义务。
