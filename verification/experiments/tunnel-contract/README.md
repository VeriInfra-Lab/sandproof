# 双向字节中继隔离候选

[证明入口](../../README.zh-CN.md) · [V0](../../../docs/formal/00-foundations.md) · [依赖合同](../../../docs/formal/dependencies.md) · [任务 #575](https://github.com/VeriInfra-Lab/sandproof/issues/575)

本目录固定实际中继的隔离输入与接入方法。正式完成状态见任务和证明覆盖；本目录存在、候选编译或单独库构建不代表原完整验收已完成。生产 Rust、Charon/Aeneas 和工具版本不由本目录修改。

## 实际代码与保证

固定生产来源为 integration 8a33bc230dbbaefd7bc6855e715c1dd5e63c1173，证明批次复用 #556 固定头 398b114d。candidate.patch 只修改私有副本的 tunnel.rs 并新增 tunnel/control.rs；原 relay 实际调用同一 decide/finish_upload，不用另一份验证算法替代。

decide 保留结果来源：上传先返回则继续等待原下行；下行先返回则结束本次单请求中继。finish_upload 在下行错误时保留原下行错误，下行成功时保留原上传结果。select 没有 biased；两侧可就绪时不预设上传先选中。下行 EOF 可以结束尚未完成的上传，因而中继 Ok 不是 HTTP/Connect 业务成功、上传完整、子进程退出或 VM 排空证据。

核心值方程使用原 Aeneas Result/Try/FromSame 模型。现有 StreamHistory 提供短写精确前缀、待写分解和有限历史；Contract/Results 将实际决定接到双向选择、等待、返回和中断。成功结论只涉及原写端确认接收的原始下行字节，不代表客户端已读到。允许任意满足预算的有限源完整通过，也允许下行结束时上传尚未读完；允许轨迹的存在不保证实际调度最终完成，不新增命令输出累计字节上限。

实际 Linux 生产调用者是 data_gateway::relay，参数为原匿名管道 Receiver/Sender 及同一 TcpStream 的两个 owned halves。完整对象、调用者、目标/features、源码及纯值/析构条件见 candidate.json；完整 AST、前端预声明、正常/异常析构位置和固定依赖源见 environment.json。前端预声明不代表全部参与运行。

A6 提供原库读写/缓冲/短写/flush/shutdown 允许行为；B3/B4 提供原流、所选结果、动作解释及生命周期的机械对应。这层符合性依赖源码审查、固定摘要与组件测试，不称完整 async Rust 已机器精化。错误载荷限定为无可观察资源/共享状态析构的值；自定义 I/O、错误 Drop 或额外许可/FD 捕获不得自动套用。原句柄自身的关闭/析构、物理退出、协议完整、授权与释放决定仍由相应业务模块提供。

## 原工具手动预检

仅在指定受控 Linux 验证环境运行。PRIVATE 必须是新的私有工作树；SOURCE 提供固定 Git 对象，ASSETS 指向本目录。禁止覆盖开发者生产文件或正式生成目录。

```sh
git clone --no-local --no-checkout "$SOURCE" "$PRIVATE"
git -C "$PRIVATE" checkout --detach 8a33bc230dbbaefd7bc6855e715c1dd5e63c1173
git -C "$PRIVATE" apply --check "$ASSETS/candidate.patch"
git -C "$PRIVATE" apply "$ASSETS/candidate.patch"
cd "$PRIVATE"
cargo fmt --all -- --check
cargo clippy -p sandproof-data-transport --lib --tests --locked --offline -- -D warnings
cargo test -p sandproof-data-transport --locked --offline --lib tunnel::tests -- --test-threads=1
cargo test -p sandproof-host-service --locked --offline --lib data_gateway::relay_stdio::tests -- --test-threads=1
"$TOOLS/aeneas/charon/bin/charon" cargo --preset=aeneas --error-on-warnings \
  --sysroot default --opaque tokio --opaque hyper --opaque axum \
  --include sandproof_data_transport \
  --start-from crate::tunnel::control::decide \
  --start-from crate::tunnel::control::finish_upload \
  --dest-file sandproof_tunnel_policy.llbc -- \
  --manifest-path sandbox-data-transport/Cargo.toml --lib --locked --offline
"$TOOLS/aeneas/bin/aeneas" -backend lean -abort-on-error -warnings-as-errors \
  -split-files -emit-json -namespace SandproofTunnelRust -subdir SandproofTunnel \
  -dest "$GENERATED" "$PRIVATE/sandproof_tunnel_policy.llbc"
```

GENERATED 使用新的私有目录。只接收本次新生成且 has_errors=false 的完整 LLBC；与 environment.json 核对整个中间表示、外部/内建声明和析构，只使用原检查器允许的无序 short_names 规范化。Types/Funs/translation.json 必须逐字对应正式候选，不能手改或复制旧文件伪造再生。原 async 入口会产生 coroutine 不支持诊断；带错误输出是失败证据，不能作为有效抽取物。

在包含本库的证明根执行 `lake -f lake-tunnel.toml --no-cache build SandproofTunnel.Audit`，再以文档指定的真实 Lean 服务检查物理 Audit 文件，要求完整、非陈旧、全部目标审计消息及无错误/警告。单独新库构建不替代原 verify-controller.sh、Linux check-function.sh 或尚未接通的来源/重放门槛。

## 正反版本与责任

错误版只在下行成功后丢弃原上传错误，必须先独立编译/原工具抽取成功，再由正确库同一 Contract.lean 性质和原 upload_write_failure_preserves_the_guests_rejection_response 断言拒绝。Counterexample.lean 只在错误生成库的独立私有工程检查，不导入正确证明根，也不导入失败的 SameContract；它证明原错误确实被丢弃，依赖仍须由原标准命令审计。导入、编译、超时或证明步骤失败不能单独算作行为反例。

有限旧新回归不构成一般等价证明。完整 HTTP/Connect、其它 T8 调用、WriteIdle/TLS、授权、所有队列/额度、RelayExit/DataLease/Unknown 与真实排空仍开放。panic/unwind、abort/崩溃和无条件进展不由正常结果方程担保。T9 继续排除。
