# Sandbox Lifecycle Refinement

研究计划：**Firecracker 沙箱在暂停、恢复与销毁交接中的执行权安全与实现精化**。

本项目从 E2B 的真实实现出发，研究一个较小的问题：当生命周期操作并发、超时或遇到控制器崩溃时，如何保证旧操作不能重新激活沙箱，且对外声明的暂停、销毁完成条件确实成立。

下游固定为 **Firecracker v1.16.1 / Linux x86_64 / KVM**；先做单节点、全量内存快照、一个可信执行入口。Docker 只可用于离线制作根文件系统，不是本研究的运行时。

## 阅读顺序

1. [研究计划](RESEARCH_PLAN.zh-CN.md)：研究问题、取舍、贡献候选、阶段验收。
2. [E2B 源码分析](docs/e2b-code-audit.md)：固定版本、调用链、已有机制、需要实验验证的疑点。
3. [状态机设计](docs/state-machine.md)：状态、事件、完成条件、不变量和反例。
4. [Refinement 与 controller 生成路线](docs/refinement-plan.md)：明确哪些实现进入证明、哪些是可信假设。
5. [实验与判定协议](docs/evaluation.md)：避免把异步行为、重连和策略差异误报为漏洞。
6. [可执行小模型](models/README.md)：用于尽早反驳设计，不是完整证明。
7. [固定源码证据](references/code-evidence.md) 与 [版本清单](references/source-manifest.json)。

## 当前状态

- 已完成：第一轮针对生命周期关键路径的源码阅读、范围决策、状态机与证明义务设计。
- 已提供：可重复运行的有限状态探索和故意删除关键条件后的反例。
- 尚未完成：TLA+/Dafny 机械证明、生成的生产 controller、Firecracker 实机实验、E2B 缺陷复现。
- **没有声称发现 E2B 漏洞，也没有声称验证了整个 E2B 或 Firecracker。**

研究日期：2026-09-07。E2B 文档与当前源码并不保证等同于云端部署；源码 SHA 是后续实验的基线。

仓库保持私有，研究结论成熟后再决定公开范围。

## 复现本轮检查

```sh
python3 scripts/fetch_sources.py
python3 scripts/check_references.py
python3 models/lifecycle.py --output reports/model-check.json
```

第一条需要访问 GitHub，下载固定的公开源码；其余检查本地运行。已有源码 checkout 不匹配时脚本会停止，不覆盖本地改动。
