# 战略定位：两条轨道与 MBT 的位置

> 历史战略记录。当前以 E2B 的核心用户行为为兼容目标，功能先行、验证后置，具体范围及分工见[项目蓝图](project-blueprint.md)。本文关于审计轨道的研究材料仍保留，不构成继续开展外部系统 bug 工作或改变当前目标的任务。

2026-09。本文固定一次方向决策，回答“verified controller 指什么、E2B 扮演什么、model test 放哪里”。

## 1. 三个对象，一条边界

```
(甲) 我们自己的 controller 决策核心   → 被验证物：证明它正确
(乙) E2B 现成系统                    → 只做需求来源与 bug 试验场，不验证
(丙) Lean 模型                      → 已证明的规范，关系式，不可直接执行
```

**决策 1：verified controller = 我们自己写的纯 reducer**，形态固定为

```text
reduce(state, event) -> Result<(state', effects), Reject>
```

无文件/网络/Firecracker/wall-clock。E2B 的 39 万行 Go 不在验证范围内；它只在轨道二里当被测系统。

## 2. “fully verified” 的诚实定义

系统有三层信任，任何一层缺失都不能宣称完全验证：

| 层 | 内容 | 手段 | 现状 |
|---|---|---|---|
| ① 协议规范 | 任意有限合法历史的安全性质 | Lean 机器证明 | **已完成** |
| ② 实现=规范 | reducer 每个成功分支模拟一个合法 `Step`；Reject 无未持久副作用 | Aeneas 翻译 + simulation 证明 | 未开始 |
| ③ adapter 忠实性 | Linux 按策略施加权限、快照忠实、journal 原子、FC 回执 | 固定版本审计 + 故障注入 + trace 校验 | 未开始 |

**“fully verified controller” 只指 ①+②；③ 永远只能审计/测试。** 论文与汇报必须这样表述，不把 ③ 说成已证明。

## 3. 两条轨道，共用同一规范

| | 轨道一：证明 | 轨道二：边界验证（MBT） |
|---|---|---|
| 目标 | 证明 reducer 模拟规范 | 让模型当 oracle，校验真实系统轨迹 |
| 交付物 | Rust reducer + simulation 定理 | 可执行 trace checker + 映射表 + 判定报告 |
| 依赖 | 需要先写实现 | 不需要写实现，可立即开始 |
| 对 E2B | 无关 | 用来锤实 F-1、继续找候选 |
| 对 adapter | 定义合同 | 用故障注入验证合同 |

两者不叠加也不二选一：证明覆盖“我们的核心”，MBT/审计覆盖“现实边界”。

## 4. MBT 定位：离线 trace 校验

**决策 2/3：先做轨道二，形态取离线 trace 校验。**

流程：

```text
真实系统运行（含故障注入：重复/迟到/超时/崩溃/延迟清理）
   → 采集轨迹：event + 各时点可观察状态（obs 映射的字段）
   → checker：从 obs 重建抽象 World，逐事件判断是否存在合法 Step（evt 映射）
             每个时点检查 Safe / ProtocolSecurity
   → 无法解释的轨迹 = 反例；delta-debug 缩到最小
```

关键工程输入正是 `conformance-method.md` 已定义的两份映射：`obs`（World 字段 ↔ 真实存储）与 `evt`（Event ↔ 真实动作）。静态审计（读代码算 guard 差）和动态 trace 校验（跑代码算 trace 差）是同一套映射的两个用法。

处理不确定性的纪律（沿用 evaluation.md）：无响应请求可能已生效，不能直接当失败；不可观察转移记为 stutter；并发操作在 real-time 约束内可重排。判定五分类：Conforms / Violates / Contract difference / Inconclusive / Unsupported。

## 5. 重要发现：F-1 复现不需要 KVM

F-1（kill × 在途 resume）整条竞态位于 **API 控制面**，不依赖 Firecracker：

- DELETE paused 沙箱：`StartRemoving`（Redis，记录不存在→ErrNotFound）→ `deleteSnapshot`（Postgres）→ 204。**不调用节点。**
- resume：`Reserve`（Redis）→ 取快照（DB）→ 调节点创建（gRPC）→ `Add`（Redis，无锁发布）。竞态点在最后这个 `Add`。

因此可以用**真实 api + 真实 Redis + 真实 Postgres + 桩化节点 gRPC**（返回可延迟的成功）复现，节点只需表现出“restore 进行中”。既有服务器（2 vCPU / 1.8 GB，Ubuntu 20.04）足以跑这套，绕开了 `/dev/kvm` 缺失。这也正是一个合法的 trace 校验装置：被测的是控制面，adapter 用桩，符合“③ 层用合同/测试”的定位。

**已完成**：在 Apple M3 本地用官方 `dummy-orchestrator`（加 8s create failpoint）复现，三轮 race 均 kill 204 / resume 201 / 最终 running，对照保持删除。见 [F-1 复现](e2b-f1-repro.md)。

## 6. 收敛后的下一步

1. 定义 trace 格式与 obs/evt 映射（先覆盖 controller 侧生命周期路径）；
2. 写 checker（可先用 Python，从模型性质导出；后续可换成 Lean 可执行语义）；
3. 用桩节点搭 F-1 复现装置，产出最小轨迹与原始证据；
4. 负例校准：对修复前 commit 跑 checker，必须命中 E25/E27 形状；
5. 再启动轨道一（Rust reducer + Aeneas），并让 trace checker 复用于其 adapter 故障测试。

完成标准沿用研究计划：② 未证明前不称 verified controller；③ 未审计的 adapter 合同不计入结论。
