# 实验与判定协议

状态：预注册式实验设计。Lean 参数化模型的完整 `Safe` 已通过；尚未启动 Firecracker 或 E2B 实验。

## 1. 三个待回答的问题

| 问题 | 证据 |
|---|---|
| RQ1：抽象是否保留了真正危险的交错？ | Lean 可达性构造、mutation 反例、异步/崩溃边界覆盖；正式模型与源码事件映射 |
| RQ2：生成核心及 adapter 是否守住协议？ | 机械证明状态、实机故障注入、实现历史的 conformance；TCB/未证明部分逐项列出 |
| RQ3：强完成契约的代价是什么，E2B 如何选择？ | admission、冻结、持久化、终止的分段延迟、可恢复性、失败后可用性与真实反例 |

不要把“发现商业平台漏洞”设为必达里程碑。零真实缺陷也可诚实报告，但需有独立的形式化贡献与足够的正/负基线。

## 2. 系统与基线

1. **我们的 controller**：固定 Firecracker v1.16.1，明确 StrictLocal 完成语义。
2. **消融版本**：删 effect fence、删 callback guard、提前 pause success、timeout 回滚、提前 kill completion。它们是我们自己的故意错误实现，不是 E2B 补丁或已知漏洞。
3. **E2B 原生版本**：固定 infra/SDK、构建版本、功能开关与测试配置。若完整自托管成本过大，先抽取真实 API store、transition、orchestrator adapter 的局部 harness；称为组件实验，不称为云端 E2B 验证。

当前 E2B 公开 self-host 路线涉及多服务/云基础设施；不把完整部署当成第一周前提。下游 Firecracker 实验可先在一台 Linux/KVM 主机独立完成。

**性能比较必须匹配语义。** 同一个指标若一边是远端上传已完成，另一边仅是本地缓存完成，不能直接宣称谁更快。可以分别展示 E2B 原生用户可见延迟和我们强契约延迟，并明确两者承诺不同。

## 3. 最小配置与记录

| 维度 | 首轮设置 |
|---|---|
| 并发 | 每个 sid 两个客户端；先一条互相冲突的操作对，再扩展三操作 |
| 运行时 | x86_64、KVM、Firecracker v1.16.1；1 vCPU / 512 MiB 起步 |
| snapshot | 全量内存、state、磁盘；本地持久卷，manifest 原子提交 |
| guest | 一个固定 rootfs/kernel，关闭无关服务；实机前锁定内容 hash |
| 网络 | 固定策略、单一 managed exec 入口；E2B 关闭 auto-resume/auto-pause，验证配置确实生效 |
| workload | nonce 启动探针、单调计数器、内存随机标记和磁盘标记、长驻进程、长连接上的第二条请求 |
| 实验限制 | 只在自有/受控实例与测试数据上故障注入；不需要商业云凭据即可先完成模型与组件阶段 |

每条 history 至少记录：trial/seed、sid、客户端、operation id、q/g/h（内部可用时）、操作/参数 digest、invoke/response、原始返回状态、unknown/pending 标记、节点 effect 事件、配置和版本。

跨节点 wall clock 不直接提供全序。优先用本地单调时钟与因果 token；保持 happens-before 偏序。E2B 黑盒看不到内部执行代时保留“不可观测”，不要用猜测补齐。

## 4. 操作对与注入点

| 场景 | 插入延迟/失败的位置 | 主要性质 |
|---|---|---|
| create × kill | Load 前后、激活后注册前 | terminal 不被迟到 start 发布覆盖 |
| resume × kill | snapshot 已选定、FC load 后、Activate 前后 | 旧副作用屏障与无复活 |
| pause × kill | Freeze 前、LocalSealed 后、manifest 前 | 已终止对象不因快照晚提交而重新获得恢复资格 |
| pause × resume | pause 尚未 Durable/旧进程未退出 | 无双跑、不能从不完整恢复点启动 |
| resume × resume | reservation/新实例分配处 | 重复操作复用结果或冲突，不能同时产生两个 Executing VM |
| exec × pause/kill | 网关已接收但未派发、旧连接下一请求 | 入口屏障后的新 admission；区别旧输出 |
| 每个操作 × crash | journal 前后、effect 前后、完成记录前后 | 不因 Unknown 产生新的错误事实或新实例 |
| pause × 存储故障 | 内存/state/磁盘落盘之间、manifest 提交前后 | 不承诺错误或不持久的恢复点 |
| stale cleanup × new instance | 旧 h 清理暂停，替换实例创建后释放 | 资源回收不能作用于新 h |

先做确定性调度，把关键边界设为 failpoint；再用随机扰动扩展。每条发现经过 delta debugging 去掉无关操作/延迟，得到最短或足够小的因果 trace。

## 5. 从历史判断，不从一个布尔返回值判断

观察结果分五类：

- **Conforms**：存在满足 profile 与实时先后约束的合法抽象历史。
- **Violates**：不存在合法解释，且必要事件可观察、故障在声明范围内。
- **Inconclusive**：RPC 超时、内部不可观测或残余执行未能确认，证据不足。
- **Contract difference**：我们的强契约与 E2B profile 不同。
- **Unsupported configuration**：版本/feature flag/后端不支持该场景。

history checker 保留所有可能的抽象状态：对于观测 o，将当前候选集合推进到能产生 o 的状态（含允许的内部步骤）。集合为空且输入满足 profile 前提时才给 Violates；只保留一条“我们认为应该发生的顺序”会误报并发操作。

并发完成的操作允许重新排序，但必须保留 real-time order：若 A 的响应早于 B 的调用，则不能把 B 排到 A 前面。未收到响应的操作可能已完成，不随意判成失败或删掉。`query` 若不是线性一致读，需单独标注其可用作何种观测。

E2B 的严格 oracle 在 profile 冻结之前是开放研究问题，不能先用我们强语义做大量 fuzz，再倒推不符点是漏洞。

## 6. 安全影响判定与防误报

| 看见的现象 | 还需要确认什么 |
|---|---|
| kill 后收到输出 | 进程是 kill 前派发还是之后新派发？是否仅缓冲/网络延迟？ |
| pause 后服务又可访问 | auto-resume 是否关闭？是否另有客户端 connect/resume？ |
| 旧 SDK handle 仍可用 | E2B 是否承诺恢复后撤销它？不能自行假定 token 按执行代失效 |
| pause 后快照恢复失败 | 是否匹配承诺的故障域？是否版本/CPU 不兼容？持久化是否原本异步？ |
| 两个 FC PID 共存 | 两者是否都 Executing？是否一个 Frozen/退出中？PID 是否复用？ |
| 状态列表显示 running | 是否缓存/最终一致视图？对应哪一个实际实例？ |

安全漏洞报告还需说明具体影响：未授权执行、已撤销对象复活、跨 owner 数据泄露，或其它有证据的安全边界破坏。单纯资源清理慢、策略不同、超时或可用性退化，不自动属于安全漏洞。

## 7. 指标和验收

正确性指标：各 invariant 的反例/通过/未知数、mutation 检出率、误报原因、failpoint 覆盖、正常流程完成率、pending 最终结果。去重以根因和违反性质为单位，不能把同一竞态的 100 个 seed 算成 100 个 bug。

成本指标：代码/证明规模（人工与生成分开）、证明时间、adapter TCB 规模、journal 写次数；pause/resume/kill 分段 p50/p95/p99 延迟、资源残留时间、快照大小、正常及故障后可用性。

每个无故障性能点先做至少 30 次独立试验，报告样本数、分位数和不确定性；预热、缓存冷热、guest 内存脏页比例需一致。尾延迟估计不足时增加样本，不从几十次样本作可靠 p99 结论。故障试验以各个离散边界的覆盖为先，再按种子扩展。

G0/G1 的最低验收已经完成：Lean 中存在正常 create→pause→resume→kill 的构造性可达 trace；17 项 `Safe` 都有初始成立、逐步保持和任意可达状态总定理；七组 workspace authority 要求有独立定理；四个故意删弱的规则各有机器检查反例，覆盖双执行、双路由、过早 Paused 和跨 sandbox 恢复源，并有恢复后旧 generation 文件访问被拒绝的具体证明。这里的 mutation 是规范敏感性验证，不是对 E2B 的漏洞结论。

## 8. 复现实验包

最终应包括：提交 SHA/工具链与镜像 hash、profile、构建脚本、可重放历史、failpoint/seed、结果归一化规则、最小反例、证明文件和验证命令、机器规格与原始指标。

当前 repo 的源码 pin、设计文档、Lean 状态机和完整抽象 safety 证明只是该实验包的第一部分。它不等同于真实系统验证；后续仍需 trace、mutation、refinement、故障注入和原始指标。
