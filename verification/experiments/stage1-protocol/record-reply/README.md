# 完整回执返回校验

[V3 入口](../README.md) · [协议范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

实际 [respond](../../../../sandbox-internal/src/server/lifecycle.rs) 在原
`record_lifecycle(intent.clone(), context).await` 之后调用唯一 `record_reply`。
它把原完整匹配判断和返回值分类移入同步 helper；请求解析、完整封套校验、独立上下文、
原 handler 调用、JSON 输出预算以及 HTTP 状态分类保持原接线。

从已解码的完整 `AcceptedLifecycleIntent` 和原 `Result<RecordedLifecycleIntent, ReceiveError>`
开始，`FullReplyContract` 复用既有 `Validation.RecordsOriginal`。
原完整 intent、current version 和 current 关系决定是否返回原完整回执；
不把版本关系加强成恰好等于 accepted version，也不要求 current 总为 true。
`terminal` 不属于原 matches 的认证内容，但成功返回必须保留它和其余全部原字段。
Conflict 保持独立；Capacity、Unavailable 和不匹配回执映射为 Unavailable。

三个 Rust 视图使用同一 Model、Values 和原 Recorded 证明：

| 视图 | 原实现差异 | 同合同证据 |
|---|---|---|
| actual | 当前唯一生产 helper | 任意完整输入/结果的对应；合法完整回执及原错误类别见证 |
| skip-match | 只省略完整 matches 判断 | 正常回执仍成功；同 session 的 changed capture 违反原完整合同 |
| clear-terminal | 原 guard 后把返回 terminal 清零 | 正常 false 回执仍成功；合法匹配的 true 回执违反原完整返回合同 |

错误视图实际编译、抽取并检查反例；不导入正确 Core/Witnesses。编译失败、导入失败、
诊断未完成或超时均不是性质拒绝。全称对应不由有限见证替代。

`capture.py` 仅追加四个既有透明 request/envelope 调用根，实际 helper 直接从
`sandbox-internal` 捕获。生成体不手改；四个外部值接口继续使用原固定 String、Vec、Bool
和 Option/字典模型。原 ReplayEquality/Recorded 仅在私有视图增加 wrapper namespace 的
`open`，其语句和证明正文保持原样；不复制另一份协议模型。

`generated-index.json` 登记全部真实函数、完整文件/审计闭包和原工具输出。
当前实际源码重新抽取后逐字节核对这些生成文件；原生成记录与后续实际执行保持各自身份。
`formal.py` 在捕获前核对实际根 Cargo.lock 的原归档及解包源码，之后调用原全文件内核、
规定同服务和依赖审计器，并核对最后源码、文件模式及固定工具/库输入。

```sh
# 证明环境：沿用既有固定工具、缓存和重任务锁。
env LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536 CARGO_NET_OFFLINE=true \
  taskset -c 0-3 python3 verification/experiments/stage1-protocol/record-reply/formal.py \
  --lock "$SANDPROOF_PROOF_HEAVY_LOCK" --output "$SANDPROOF_REPLY_FORMAL_OUTPUT"
# ning：同一精确源码及原固定普通环境；该命令不代替完整基础检查。
taskset -c 0-3 python3 verification/experiments/stage1-protocol/record-reply/native.py \
  --lock "$SANDPROOF_PROOF_HEAVY_LOCK" --output "$SANDPROOF_REPLY_NATIVE_OUTPUT"
```

原生入口让 actual、两错和 restored 使用同一份七条断言；restored 先实际变异再恢复原字节。
每个视图独立新编译实际协议库和 internal 测试程序，核对源码、manifest、链接产物及完整
失败集合，并保留格式和严格 Clippy。原生检查仅在 ning 执行；入口存在不表示已经运行。

这些定理只覆盖正常有限值模型中的同步返回决策。匹配值不建立授权、已持久接受或实际
资源存在；synthetic context 见证也不是网络认证事实。原 await 的实际 receiver/context/
结果连接、生产者持久责任、实际 JSON/有界 Writer、TLS/认证来源、提交未知、重启和
完整异步取消/析构仍单列。这里没有把整个 handler、Serde 或收件事务设为可信条件。
依赖及公共登记/覆盖候选交主控单写；局部结果、ning 准入、公共接线和完整 V3 分别验收。
