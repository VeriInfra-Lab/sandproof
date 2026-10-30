# 暂停成功发布的有限切片

[V4 规格](../../../docs/formal/04-local-core.md) · [证明流程](../../../docs/proof-workflow.md) · [实际覆盖](../../../docs/proof-coverage.md)

本组件检查真实 `controller-core/src/publication.rs::handle_publish_paused`，
不另外实现发布算法。范围是同步纯决定：发布是否允许、返回哪个事件、修改哪些状态。
它不直接执行或验证 Tokio、SQLite、文件持久化或 VM 停机。

## 原生检查

`native.py` 使用原干净提交归档，在三个私有源码副本中包含 `tests.rs`；生产文件不变。
正常输入复用原 `capture::tests` 的真实 reducer 历史。三个副本分别为正确、
仅绕过当前 Stop 成功回执拒绝分支的错误版、从原归档重新建立的恢复版。
唯一修改规则由 `CASES` 和 `mutate` 提供，形式错误版也复用此规则。

同一组断言检查完整发布结果、仅两个原表项发生变化、缺失/未知/旧尝试回执被拒绝，
以及缺少逻辑效果行时仍按原实现查询第 0 次尝试。索引中的 VM/槽均明确非空且已关闭，
避免把资源关闭当作 Stop 回执。改动回执或计数器的输入属于函数接口输入域，
不声称它们全部是系统历史中可达的状态。

每个副本分别进行格式检查、全新编译、原断言和 Clippy；错误版必须编译成功，
且只有规定的 Stop 断言失败。构建失败、零测试、额外失败或日志不匹配都拒绝。
原始输出、测试程序与源码摘要保留在结果目录；这不是形式证明。

## 公共入口

接线和正式验收状态以覆盖记录为准。最终验收须使用同一稳定源码，普通检查只在 ning，
形式抽取和 Lean 检查只在证明机执行。以下有限入口不等于 `all` 或整个 V4：

```sh
bash scripts/python-entry.sh tools/proof_native_suite.py run \
  --scope pause-publication --output /private/acceptance/pause-publication-native

# 证明机消费完整的原 commands/、runtime.json、complete.json 和源码归档。
export SANDPROOF_ACCEPTANCE_ROLE=formal
export SANDPROOF_NATIVE_EVIDENCE=/private/acceptance/pause-publication-native
bash scripts/verify-pause-publication.sh
```

有限范围固定包含完整 Linux `functional` 与 `pause-publication` 两个组件；缺项、
错范围、损坏日志或源码不符不能入场。`native.py` 保留原组件 CLI：
`--archive --archive-sha256 --head --output --lock`。
外层持原 CI/原生协调锁，组件使用本次私有锁，不能二次锁同一个全局锁。

## 正式驱动与最终清单

`replay.py` 使用同一份原源码归档，正确版调用 `regenerate.sh caller` 的真实本地根，
错误版只使用 `native.mutate` 变异，再从同一完整根集合重新运行固定 Charon/Aeneas。
实际生成的 Types、Funs 和 translation JSON 分别与已提交版本逐字节比较；
同时检查发布入口、成功查询、两个索引循环及原捕获方法仍为透明的本地定义。
外部符号集合复用原 `caller_replay.py` 的审核清单；错误版的外部模型只能机械更换
Types 导入和命名空间，不能另写更强的模型。

最终 `inventory.json` 必须逐文件列出全部证明声明、总审计和真实错误版审计，
并另外固定业务合同要求存在的关键定理。它由实际交付的合同与证明一起审查，
不能仅扫描已经写出的声明就认定全部义务完成。缺清单、缺必需定理、漏文件、
声明或审计不符都拒绝。文件数、审计数从该清单及真实导入闭包计算，不预填成功数量。
尚未登记的最终文件不以临时空清单替代，具体准备与通过范围见覆盖记录。

驱动保留正错独立的生成环境，运行原内核构建和同性质反例，再通过项目同一 Lean
服务逐份检查完整本地导入闭包。每份文件都绑定实际内容摘要；服务缺失、依赖失败、
stale、partial、超时、警告、错误或审计目标不符均拒绝。执行结束再次核对原源码、
归档和固定工具，只有全部成功才写组件 `complete=true`。它不是全项目封口。

形式组件保留原生 CLI 的五项参数，另要求 `--tools` 和 `--packages`：

```sh
python3 verification/experiments/pause-publication/replay.py \
  --archive /private/pause-source.tar --archive-sha256 SOURCE_SHA256 \
  --head EXACT_COMMIT --output /private/formal-pause-publication \
  --lock /private/coordination/pause-formal-component.lock \
  --tools /private/fixed-tools --packages /private/fixed-lake-packages
```

该组件直接复用固定工具、原纯 Std 模型、原导入闭包解析器和 `tools/lean_session.py`。
翻译器及已审核模型仍属于原有工具信任条件；框架接通或驱动的合成拒绝测试通过，
不等于真实 Rust 抽取、Lean 定理或最终正式入口已经通过。

共享 CaptureCaller 生成根若改变，原 `capture-caller` 有限入口也须重新验收；
两份有限封口分别交接，不能拼成全项目通过。源码、工具、依赖和实际运行条件都相同，
且原收据及全部证据仍通过原检查时，才允许复用完整 `functional` 组件。

## 必须显式保留的条件

本切片最多证明：给定当前 `World` 的查询结果，真实 Rust 纯处理函数只在相应条件满足时发布。
`Acknowledged(Succeeded ...)` 是逻辑回执，不自动证明物理 VM 已退出；
持久快照字段也不自动证明文件内容、同步落盘或恢复资格真实成立。
V5/V6 负责存储和宿主事实的连接，V9 负责历史中的身份、索引完整性及不变量。
有限切片不证明所有发布前提都能最终实现，也不替代系统进展或全部故障交错。
