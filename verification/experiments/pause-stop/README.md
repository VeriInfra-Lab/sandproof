# 普通暂停的登记与 Stop

[证明工作流](../../../../docs/proof-workflow.md) · [生命周期计划](../../../../docs/formal/04-local-core.md)

本切片检查真实 `handle_worker` 到 `worker::pause_stop::record` 的
`CommitSnapshot`、`StopVm` 调用。真实函数的局部定理、具体接受见证、同性质错误证书及
原生组件已有指定版本结果；准确输入、前提及边界见
[覆盖记录](../../../../docs/proof-coverage.md#普通暂停登记与-stop真实函数局部证明与公共验收边界)。
完整公共入口按 [#636 本片验收记录](https://github.com/VeriInfra-Lab/sandproof/issues/636)
的精确候选和原始凭据判断，局部结果不代替该批次验收。
机器对应的目标是实际生产 `record` 及其调用体；外层 `handle_worker` 的唯一转发
通过源码审查和原生连接回归核对，不是整个 worker 的一般等价机器证明。

## 要检查的行为

普通暂停中，Commit 的实际 worker 观察先登记材料，把快照改为不可变的 Durable。
只有交接允许停止原 VM 时才排入 Stop。这里不要求先收到 Commit 的成功回执。
若登记前已撤销，晚到的 Commit 仍可登记已经产生的材料，但不能借此恢复许可或新排 Stop。
登记后发生普通取消则沿用原有有效交接，不能一概当成禁止停止。

Stop 还要核对实际派发、当前尝试、队列、VM 状态、栅栏、当前计划、目标快照身份
及 Durable/immutable 字段。它检查命令与 VM 的 generation 相等，没有直接比较
VM 与 sandbox 记录的 generation。普通 Commit 没有快照 owner 或记录非终止的
直接守卫；后续普通 Stop 有这些检查，不能由 Commit 成功单独推出全部 Stop 前提。
终止场景的 Revoking Stop
使用终止栅栏，不能把普通暂停的快照要求强加给它。

原生夹具复用原控制器的 create、running、pause、seal 和 commit 历史，均经真实
`handle_worker` 入口。少数行替换用于检查接口输入域，不声称这些状态都能从合法初态到达。
成功 Stop 的完整 World 对照包含 VM 表和资源索引；不是只检查单个状态字段。

形式正常接受还保留一个首次创建 Stop 的具体见证：原 effects/logicals 没有该 Stop，
真实 Commit 创建完整效果/逻辑行并放入两个队列。这不表示所有合法输入都能成功。
Commit 到后来 Stop 的 generation 连接另以中间 sandbox/VM 两表保持及后来真实 Stop
成功为条件；实际派发器落实这些保持仍须连接，不由 Tokio 自动保证。

## 正确版与错误版

`native.py` 对同一已提交源码归档分别创建正确、`ignore-handoff`、恢复三份私有副本。
错误版只忽略真实 `may_stop_original` 的布尔结果，保留当前交接查询、None 拒绝、
计划 VM 对比和其它检查。专门反例先使快照持久性、身份、栅栏、队列和尝试全部满足，
再只改变登记、撤销或卸载材料许可。错误版必须真正编译并由同一条性质断言拒绝；
编译、格式或 Clippy 失败不计为抓到反例。形式错误版复用这个变异函数。

正确版和恢复版必须通过全部夹具，错误版仅允许清单中指定的性质断言失败。
原始 stdout、stderr、实际新编译二进制身份及全部源码摘要均保留。

## 验收入口

普通环境使用原离线依赖及原协调锁，取得最终稳定提交后运行：

```sh
bash scripts/python-entry.sh tools/proof_native_suite.py run \
  --scope pause-stop --output /private/acceptance/pause-stop-native
```

该范围固定包含完整 Linux `functional` 和 `pause-stop` 原组件，各一次。
旧 capture、publication、V3 的有限结果不能替代它；默认 `all` 保留旧全部义务。
同一稳定源码上复用 functional 时仍使用原收据和完整原始证据检查，不改标签或键。

形式入口 `scripts/verify-pause-stop.sh` 已接固定工具、原始生成字节比对、同一服务诊断
及审计流程，复用原声明读取和诊断检查器。实际定理清单、正错生成物、冻结范围和完整
审计必须与候选一致；缺一即拒绝，不生成占位成功。无定理的 Conditions 页仍检查文件，
关键定理子集不能代替全部公开声明和审计。
完整入口须逐字节重抽正确/错误版，检查全部实际文件、同一服务诊断和内核依赖。
共享生成闭包变化时另核对受影响旧模块，分别保留它们原来的范围。

本切片不证明物理 VM 已退出、磁盘真实落盘、完整 worker 一般等价或全生命周期 V9。
这些结果仍需所属实现、边界合同及系统组合提供。
