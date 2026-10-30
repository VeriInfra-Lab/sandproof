# 普通暂停的登记与 Stop

[证明工作流](../../../../docs/proof-workflow.md) · [生命周期计划](../../../../docs/formal/04-local-core.md)

本切片检查真实 `handle_worker` 到 `worker::pause_stop::record` 的
`CommitSnapshot`、`StopVm` 调用。当前处于开发阶段；夹具和接线已编写不等于
原生、抽取、Lean 或完整入口已通过。正式结果只在证明覆盖和任务 issue 记录。

## 要检查的行为

普通暂停中，Commit 的实际 worker 观察先登记材料，把快照改为不可变的 Durable。
只有交接允许停止原 VM 时才排入 Stop。这里不要求先收到 Commit 的成功回执。
若登记前已撤销，晚到的 Commit 仍可登记已经产生的材料，但不能借此恢复许可或新排 Stop。
登记后发生普通取消则沿用原有有效交接，不能一概当成禁止停止。

Stop 还要核对实际派发、当前尝试、队列、VM 状态、栅栏、当前计划及完整快照身份。
普通 Commit 没有检查快照 owner；后续 Stop 有此检查。终止场景的 Revoking Stop
使用终止栅栏，不能把普通暂停的快照要求强加给它。

原生夹具复用原控制器的 create、running、pause、seal 和 commit 历史，均经真实
`handle_worker` 入口。少数行替换用于检查接口输入域，不声称这些状态都能从合法初态到达。
成功 Stop 的完整 World 对照包含 VM 表和资源索引；不是只检查单个状态字段。

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

形式入口 `scripts/verify-pause-stop.sh` 还需最终冻结清单、真实生成物、实际定理清单及
`replay.py` 全部就绪；开发中缺一即拒绝执行，不生成占位成功。
最终接线将逐字节重抽正确/错误版，检查全部实际文件、同一服务诊断和内核依赖。
共享生成闭包变化时另核对受影响旧模块，分别保留它们原来的范围。

本切片不证明物理 VM 已退出、磁盘真实落盘、完整 worker 一般等价或全生命周期 V9。
这些结果仍需所属实现、边界合同及系统组合提供。
