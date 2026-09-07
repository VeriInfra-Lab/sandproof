# 可执行的有限状态机草图

运行（Python 3.9+，无第三方依赖）：

```sh
python3 models/lifecycle.py --output reports/model-check.json
```

默认边界：1 个 sid、2 个执行代、最多 4 个已接受生命周期操作。消息可延迟/重复，worker fence 是单独事件，controller 可以 crash/recover；这是压缩后的协议环境，不是 E2B 源码模拟器。

| 版本 | 默认结果 | 状态数 | 反例长度 |
|---|---|---:|---:|
| 基础模型 | 有限空间内无反例 | 298 | — |
| 无副作用处的 fence | 找到反例 | 71 | 5 |
| 无回调 guard | 找到反例 | 79 | 7 |
| pause 在 durable 前成功 | 找到反例 | 161 | 12 |
| 把 resume timeout 当作回滚 | 找到反例 | 245 | 19 |
| kill 在 quiescence 前完成 | 找到反例 | 23 | 3 |

基础模型检查 735 条转移；所有 7 个 phase 均可达。mutation 的状态数是找到第一个 BFS 反例时已发现的状态数，不是各自完整空间规模。原始报告包含每一步的完整状态。

故意把关键 guard 删掉，是为了检查模型是否能表达危险路径。五个反例都来自本项目自己的变体，**不代表发现五个 E2B 漏洞**。

可用 `--max-revision 6` 扩大操作上界，但执行代仍固定为 2。参数扩大后的无反例仍是有限检查；它不能证明无界实例/操作，也没有检查活性。

本轮也运行了该上界：[bound6 报告](../reports/model-check-bound6.json)记录基础模型 366 个状态、904 条转移，仍无反例；五个 mutation 均被检出。

刻意保留的局限：

- 无完整 API history、两客户显式进程、owner/token、exec 排队/长连接模型。
- 无 executor crash、磁盘掉电/损坏、完整失败补偿与公平性。
- 某些 completion guard 直接读取环境真实状态；正式模型需换成身份绑定且有契约的确认事件。
- `uncertain` 仅演示 timeout 不回滚，没有实现完整知识逻辑。
- Python transition function 不是拟生成的生产 controller；Dafny/Go、TLA+ 文件尚未实现。

模型用途是让状态机尽早暴露反例，不是为计划附上“已经证明正确”的标签。
