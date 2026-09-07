# 场景、反例与实验协议

当前阶段已经完成 Lean 场景与 mutation 证明；尚未启动 Firecracker 或 E2B 实机实验。

## 1. 机器检查场景

[`Scenarios.lean`](../SandboxModel/Scenarios.lean) 不是有限搜索器，而是给出可检查的正常路径片段：

| 场景 | 检查内容 |
|---|---|
| create 到 Running | 创建 intent 持久；应用精确 file policy；确认对应 effect；随后才发布 Running |
| pause 到 Paused | manifest 与 sid/source generation/workspace 绑定；durable 后停止 VM 并发布 |
| resume | 从 controller 的 `RestoreSource(snapshot, manifest)` 选择来源，新 generation 重新开始 |
| operation retry/query | 相同输入重试为 stutter；query 返回账本中的状态与结果 |
| controller crash/recover | committed effect 在 controller 重启前后保留 |
| delayed cleanup | generation 1 的 cleanup 晚到时，generation 2 当前 VM 保持不变 |

全局 theorem 对所有合法历史成立；这些场景的作用是证明模型允许正常工作，并展示复杂 guard 如何组合。

## 2. 机器检查 mutant

[`Counterexamples.lean`](../SandboxModel/Counterexamples.lean) 故意绕过单个规则并证明相应性质会失败：

| 删除或破坏的规则 | 产生的坏状态 |
|---|---|
| Running 的 policy receipt/ack guard | 没有应用当前策略也能公开文件 authority |
| file-policy generation 检查 | 旧 generation 策略可写入 worker receipt |
| manifest/workspace 身份检查 | 别的 workspace 的 snapshot 可被选作恢复源 |
| commit-before-dispatch | 没有持久 effect 也能派发 worker 命令 |
| cleanup 的 not-current 检查 | 当前 VM 可被标记 reclaimed |
| worker fence 单调检查 | fence epoch 可倒退或 terminal 可被清除 |

这些是规范敏感性实验，不是 E2B 漏洞结论。

## 3. 实机研究问题

| 问题 | 所需证据 |
|---|---|
| 抽象是否保留真实危险交错？ | E2B/Firecracker 事件映射、场景、mutation、最短失败历史 |
| reducer 是否实现 Step？ | Aeneas simulation、拒绝分支无 effect、operation/outbox 原子性 |
| adapter 是否兑现合同？ | 固定版本审计、failpoint、崩溃恢复、精确资源探针 |
| 强完成语义代价多大？ | create/pause/resume/kill 分段延迟及 journal 写入成本 |

不把“发现商业平台漏洞”设为必达里程碑。零真实缺陷仍可报告，但必须保留独立形式化贡献和清楚的负面结果。

## 4. 固定实验配置

首轮使用单台 x86_64 Linux/KVM、Firecracker v1.16.1、一个固定 kernel/rootfs hash、1 vCPU/512 MiB、独占 workspace 与网络槽。文件策略先固定为一份 read-write policy 引用；不扩展路径 ACL。snapshot 使用全量内存、VM state 和独立磁盘副本，最后原子提交 manifest。

workload 包含 nonce 启动探针、单调计数器、内存与磁盘标记、长驻进程和长连接上的第二条请求。每条历史记录 sid、operation/effect ID、epoch、generation、resource ID、policy/manifest digest、调用与响应、timeout/unknown、journal、dispatch、worker 检查和 ack。

跨组件不依赖 wall clock 构造全序，使用本地单调时钟、因果 token 和 happens-before。无法观察的内部事实保留为未知。

## 5. 关键交错和 failpoint

| 场景 | failpoint | 主要判定 |
|---|---|---|
| create × kill | effect commit、dispatch、activate、Running 发布前后 | terminal 不被迟到 start 覆盖 |
| resume × kill | manifest 选择、restore、activate 前后 | snapshot 不带 authority，旧 effect 不能复活 |
| pause × kill | freeze、seal、manifest commit 前后 | 未 durable snapshot 不得发布 Paused |
| retry × crash | operation 写入、outbox commit、ack 写入前后 | 同一 ID 不产生第二个决定，最终结果稳定 |
| policy × resume | 旧策略命令延迟到新 generation | 旧命令不能覆盖当前 policy receipt |
| cleanup × resume | 旧 VM/slot 清理延迟到新实例发布后 | 只回收精确旧资源 |
| ack × duplicate | 相同及不同 effect 的重复回执 | 结果稳定，另一个 effect 不变 |
| restore × storage fault | 数据写入、seal、manifest commit 间 | 只选择声明故障域内 durable manifest |

先使用确定性 failpoint，再用随机扰动扩展。失败历史经 delta debugging 缩到足以解释根因的最小序列。

## 6. 判定方式

结果分为：

- **Conforms**：至少存在一个满足协议和实时先后约束的抽象历史。
- **Violates**：不存在合法解释，且必要事件可观察、故障处于声明范围。
- **Inconclusive**：timeout、内部不可观察或残余执行无法确认。
- **Contract difference**：E2B profile 与本研究的强完成语义不同。
- **Unsupported configuration**：固定版本或后端不支持该场景。

history checker 应保留所有可能抽象状态。并发操作可以在不违反 real-time order 时重排；无响应请求可能已经生效，不能直接当作失败。E2B profile 必须先冻结，再据其公开承诺判定，不能把我们的强语义自动当作对方合同。

## 7. adapter 验收

| adapter | 最低证据 |
|---|---|
| journal/outbox | 在原子提交前后断电；恢复后 state、operation 和 effects 要么全旧要么全新 |
| policy | 不同 policy digest 和 generation 的命令乱序；只允许当前精确身份生效 |
| snapshot | manifest 前后掉电；restore 后比对内容及承诺保留的 mode/owner/ACL |
| fence | activate/freeze/stop 不可取消窗口插入暂停；旧命令不能越过已确认 fence |
| cleanup | VM/slot ID 复用与迟到回收；当前精确资源保持可用 |
| network | slot inactive 后发起新流量；旧 slot 不得代表新 generation |

这些实验验证 adapter 假设，不会把底层测试结果写成 Lean 已证明的内容。

## 8. 指标与复现

正确性报告按违反的性质和根因去重，包含 scenario/mutant 通过数、failpoint 覆盖、未知结果及原因。成本报告区分人工证明、生成代码和 adapter TCB，并记录 journal 写次数、snapshot 大小、资源残留时间，以及 create/pause/resume/kill 的 p50/p95/p99 分段延迟。

无故障性能点至少 30 次独立试验；尾延迟样本不足时继续采样。最终复现包包含提交 SHA、Lean/Rust/Aeneas/Firecracker/kernel/rootfs hash、配置、原始 histories、failpoint/seed、最小反例、机器规格和一键验证命令。
