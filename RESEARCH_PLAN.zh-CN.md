# 研究计划：Firecracker 沙箱交接中的执行权安全与实现精化

版本：0.1，2026-09-07。状态：可评审的研究设计，尚非已验证系统。

## 1. 建议固定的研究问题

> 在单个逻辑沙箱的暂停、恢复、销毁交接中，面对并发请求、重复/迟到消息和控制器崩溃，能否用一个小型状态机统一描述执行权撤销与状态保存，并从该规范生成 controller 核心，证明它在明确的运行时假设下保持这些语义？

论文暂题：**Execution Authority Across Sandbox Pause–Resume: A Refinement-Based Controller**。

“妙”的地方应当是找到了正确的抽象边界，而不是状态名称特别多。建议的中心抽象是：**持久化的生命周期意图，与节点上实际可执行的权限之间，需要一次有证据的交接。** 快照保存的是计算状态，不应顺便保存“以后仍然有权执行”的决定。

具体而言，暂停涉及三件不同的事：关闭新请求入口、冻结旧执行实例、完成可恢复快照的持久化。恢复涉及另一组不同的事：分配新实例身份、以暂停状态加载快照、校验当前意图后才启动和开放入口。销毁则必须压过所有尚在途中的恢复和发布操作。

这比最初“Created → Running → Paused → Destroyed”的单层枚举更适合研究：它允许我们表达半完成、崩溃和重试，也能解释哪些实现动作只是内部步骤，哪些才改变用户可观察的语义。

## 2. 从 E2B 分析得出的范围选择

代码阅读基线：`e2b-dev/infra@82f6529660ea98087d7a479247c789905b0a3867`，SDK 基线见[证据索引](references/code-evidence.md)。

| 决策 | 首篇工作的选择 | 依据与代价 |
|---|---|---|
| 唯一下游运行时 | 上游 Firecracker v1.16.1，Linux x86_64 + KVM | E2B 使用 microVM；保留内存暂停/恢复的研究含义。不能把 Docker pause/unpause 当作等价替代 |
| 部署 | 一台 Linux 主机，一个 controller 进程与一个受控节点执行器；允许 controller crash/restart | 避免同时解决跨主机共识、失联节点隔离和迁移；两者仍通过异步命令/结果交互 |
| 沙箱内容 | 一个固定 Linux guest、一个固定镜像、1 vCPU/512 MiB 起步 | rootfs/kernel 在实机基线阶段锁定 SHA256，不预先虚构可用镜像哈希 |
| 快照 | 全量内存 + VM state + 独立磁盘副本；本地持久卷、原子 manifest | 避免 E2B 的差分链、UFFD、远端上传和去重一起进入证明。性能上限较低，需明确报告 |
| 核心操作 | create、managed exec admission、pause、resume、kill、query operation | pause 的内部快照必须建模；不暴露独立 snapshot/fork/clone API |
| 暂停语义 | 成功表示快照在声明故障域内持久，旧 VM 已退出 | 比“保存工作已经开始”更强，不能用该差异直接给 E2B 定罪 |
| 销毁语义 | `Accepted` 与 `Completed` 分开；Completed 必须实际静止且旧命令被屏障拒绝 | 不把瞬时不可路由当成物理资源全部清理完毕 |
| 权限 | 一个不可变 owner；对外身份认证作为可信输入；所有 managed exec 经唯一入口 | 证明身份绑定，不开展通用 RBAC、密钥撤销与多租户信息流证明 |
| 网络 | 固定策略，默认禁止 guest 对外业务网络；控制连接独立 | 不把“NetworkDisabled ⇒ 无任何外发包”混进生命周期论文 |
| 超时 | 首先只研究 RPC 超时；租约过期可建模为内部 kill 请求 | 首篇不做任意 TTL 更新、自动暂停和请求触发自动恢复 |

采用 Firecracker 的代价是必须有 Linux/KVM 实验机。当前工作环境为 macOS；本轮没有启动 microVM。若短期没有实验机，先进行有限模型探索和协议实现，不能悄悄把后端换成 Docker 并声称语义相同。

E2B 使用自己的 Firecracker 版本解析/功能门控，其 fork 和 snapshot 格式不保证与这里的上游版本兼容。**本项目 controller 的实验后端**与**E2B 自托管对照系统的原生后端**需要分别锁定，不能把前者直接塞进后者。[源码证据 E01、E15、F01–F04](references/code-evidence.md)

## 3. 研究边界：保留什么，明确放下什么

我们保留的复杂性是：一次逻辑操作由多个异步副作用组成，回调可能晚于另一项操作，controller 可在任意持久化/副作用边界崩溃。这是研究的必要复杂性，不能为了好证明而把所有 API 写成一个原子动作。

我们暂时放下：多节点迁移、HA controller leader election、容器逃逸、KVM/内核证明、恶意宿主、快照密码学、任意网络策略、volume 共享、磁盘层去重、进程级 exactly-once、计费正确性、完整 E2B 功能兼容。硬盘永久丢失不在本地持久卷的保证范围内；控制器掉电与存储故障域须在实验中分别说明。

单节点并不意味着可以只用一个进程内 mutex：controller 的锁不覆盖外部执行器、已发出的异步命令和 controller 崩溃后的工作。**副作用发生处的防旧操作机制**仍是证明对象的一部分。

## 4. 状态机应成为论文起点

详细定义见[状态机设计](docs/state-machine.md)。其三层结构如下：

1. **A：服务契约。** Running、Paused、Revoking、Destroyed 等抽象状态；定义何时接受请求、何时声明完成，以及终止不可逆。
2. **P：异步协议。** 持久意图、操作号、实例号、节点屏障、快照提交、在途命令、未知结果与恢复。
3. **C：controller 实现。** 经验证的纯状态转换核心，加带契约的 journal、executor、Firecracker 和存储适配器。

两条必须分开的性质：

\[
\text{RevokeBarrier}(s) \Rightarrow \text{NoNewManagedAdmission}(s)
\]

\[
\text{DestroyCompleted}(s) \Rightarrow
\text{NoExecutingVM}(s)\land\text{NoFutureActivation}(s)
\]

前者不能自动推出后者；已经运行的 guest 不需要新的 exec 请求也可以继续计算。类似地，快照有本地文件不等于掉电后可恢复；必须有一致且持久化的 bundle/manifest。

模型中 `Unknown` 是 controller 对外部状态的知识状态，不是另一种虚拟机物理状态。RPC 超时不等于操作失败，更不能直接转回 Running 或 Paused。此处若定义错误，后续生成代码会忠实生成错误行为。

## 5. 贡献候选及 novelty 风险

| 候选贡献 | 需要拿出的证据 | 不能采用的表述 |
|---|---|---|
| 沙箱执行权与快照交接的精化模型 | 具体 invariant、反例、抽象映射，说明普通枚举为何不够 | “首次形式化描述 AI sandbox”——尚未做充分相关工作检索 |
| 从规范到可运行 controller 核心的可信链 | P→A 证明、可执行核心契约、生成路径、明确 TCB | “TLA+ 检查通过，因此 Rust/Go controller 已证明正确” |
| 基于历史而非单个返回码的 conformance oracle | 重叠操作、pending/unknown、特性配置、原始 RPC 结果均进入判定 | “模型拒绝、E2B 接受，因此必然漏洞” |

E2B 已有 execution ID、物理 lifecycle ID、reservation、原子 Lua 转换和拒绝后的恢复路径。这些机制是基线，不是我们新发明的东西。[E03–E06、E11、E15–E16](references/code-evidence.md)

[Anvil](https://www.usenix.org/conference/osdi24/presentation/sun-xudong) 已研究 Kubernetes controller 的最终稳定收敛；[IronFleet](https://www.microsoft.com/en-us/research/wp-content/uploads/2015/10/ironfleet.pdf) 已连接状态机精化与实现证明。本项目必须回答更具体的问题：**沙箱“停止以后不会因旧工作重新执行”的契约，如何跨过快照/恢复的副作用边界落到实现？**

如果前两周发现该问题完全退化为现成工作中的通用 fencing/重试案例，或者所有有趣问题都依赖首篇排除的多节点条件，应重新审视选题。不能用“AI agent”标签替代技术增量。

## 6. Controller 生成和 proof 路线

建议首版选择 **Dafny 可执行协议核心 → 官方 Go 后端 → 手写薄适配器**，而非同时自造 DSL、TLA+ 编译器和 Rust 生成器。Go 也方便与 E2B 的 Go 接口/测试工具衔接。[Dafny 官方手册](https://dafny.org/dafny/DafnyRef/DafnyRef.html)

- TLA+ 用于早期探索完整异步协议；它不是 Go 生成源。
- 作为证明权威的 A/P 定义最终放在 Dafny 中，协议核心逐步证明保持 invariant、满足 P→A 模拟关系。
- 由同一份可执行核心产生 Go；编译器、Go runtime 和外部调用边界如实列入 TCB。
- 证明适配器的身份核对、屏障与命令派发纪律；Firecracker/Linux primitive 的底层正确性作为假设。
- 对 TLA+ 与 Dafny 的手工对应做明确的桥接义务；对拍只能发现偏差，不是等价证明。

生成对象仅是决策核心、命令描述和结果处理，不声称自动生成完整云平台。若后续坚持自建 DSL，应独立安排 translation validation 工作，不能作为免费的附加功能。

## 7. 分阶段执行与停止条件

以下按一名熟悉系统与形式化工具的研究者估算 12 周；它是依赖顺序，不是进度保证。

| 阶段 | 时间 | 交付物与硬验收 |
|---|---|---|
| G0：范围与规范评审 | 第 1–2 周 | A/P 状态机、完成条件、E2B profile、相关工作定位；每条 invariant 有非空可达正例和能破坏它的 mutation；先过这关再做平台 |
| G1：完整有限模型 | 第 3–4 周 | TLA+ 模型覆盖两客户竞争、两代实例、重复/重排、journal crash；拒绝全拒绝式“安全模型”；发现反例后记录修订理由 |
| G2：运行时契约 | 第 5–6 周 | 单机 Firecracker v1.16.1 适配器；故障注入覆盖 load/activate/stop/snapshot/fsync；锁定 guest/kernel/hash；确认“屏障在副作用处”可实现 |
| G3：精化与生成 | 第 7–9 周 | Dafny 核心与 P→A 机器检查证明，生成 Go；不能只交枚举类型和 guards；清楚列出未验证 adapter 的假设 |
| G4：E2B 对照与评估 | 第 10–11 周 | 原生 E2B 固定 SHA 的小型自托管实验或局部真实组件 harness；历史归一化、去重、最小反例、误报分析 |
| G5：写作与复现 | 第 12 周 | artifact、证明状态清单、性能/可用性代价与局限；是否存在真实缺陷如实报告 |

第一阶段先解决五个问题：

1. pause 成功到底承诺哪种故障域的可恢复性？
2. kill 的可观察完成点是什么，和正在执行的进程有什么关系？
3. 旧请求在节点屏障前后分别允许做什么？
4. 恢复过程中只有回调防旧消息是否足够？若不够，谁在副作用发生处拒绝它？
5. 每个不可逆步骤之后发生崩溃，恢复动作是否唯一或至少保持安全？

**降范围规则：** 若 G0/G1 不成立，修改协议；若 G2 的执行器只能靠无法验证的“它会正确处理一切”假设，则停止宣称 controller verified，先修补接口；若 G3 太重，缩小至 pause/resume/kill 核心，不扩展第二运行时。E2B 没有复现缺陷不构成换目标挖洞的理由，仍可报告证据支持的 conformance 结果。

## 8. 本轮实际完成与下一步

本轮已阅读并固定 E2B 关键生命周期路径与 Firecracker 快照接口，写出设计和有限探索程序。默认有限模型探索了 **298 个状态、735 条转移**，未发现所列不变量的反例；五种去掉关键条件的变体均产生反例。详见[原始报告](reports/model-check.json)。这只是设计自检，覆盖范围远小于完整 P，更不是 E2B 实验。

下一步最值得做的是 G0 的规范评审：重点检查[状态机第 4–8 节](docs/state-machine.md)，先对“什么叫完成”和“谁有权产生真实副作用”达成严格定义，然后再建设 controller。
