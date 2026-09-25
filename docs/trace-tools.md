# 轨迹与研究辅助工具

[文档目录](README.md) · [状态机](state-machine.md) · [证明覆盖](proof-coverage.md)

这些工具保留用于模型实验和旧数据分析，不是当前 E2B 兼容 V1 的系统验收入口。旧审计报告、会议记录和 issue 草稿只保存在 Git 历史；不再把本地 harness 的结果描述为已确认的 E2B 漏洞。

## 1. 离线运行

```sh
python3 -m tools.trace.selftest
python3 tools/trace/check.py artifacts/example.jsonl
python3 tools/coverage.py
python3 tools/coverage.py --write
```

`example.jsonl` 需由调用者提供；最后一条把生成报告写到忽略的 `artifacts/trace-coverage.md`，不再生成第二份需要手工同步的文档。

规则和性质登记分别在 `tools/trace/rules.py`、`tools/model/properties.py`；数量和对应关系以生成报告为准，不在本页复制。

## 2. JSONL 记录格式

每行一条记录，使用同一轨迹内唯一、递增的 `seq`；`ts` 是单调时钟纳秒值，跨进程时间本身不能证明因果顺序。加载器按 seq 排序，不代替采集端证明时间与因果关系。

```json
{"seq":1,"ts":100,"source":"client","kind":"event","event":"acceptKill","sandbox":"s1","attrs":{"method":"DELETE","http":204,"result":"success"}}
{"seq":2,"ts":200,"source":"observer","kind":"observe","sandbox":"s1","obs":{"running":false,"snapshot_present":false}}
```

| 字段 | 含义 |
|---|---|
| seq / ts / source | 顺序、时间、来源 |
| kind / event | 事件或状态观察，以及抽象事件名 |
| sandbox | 本记录关联的沙箱身份 |
| attrs | 原始动作、返回结果和必要证据；输出前脱敏 |
| attrs.result | success / reject / unknown；超时不意味着动作没执行 |
| obs | 部分观察；没有观察到的字段留空，不能补成 false |

采集器写入抽象事件；`mapper.py` 读取这些名称及观察，不是自动从任意 HTTP 日志推导 Lean 步骤的证明。现有生命周期词汇包括 acceptCreate、acceptPause、acceptResume、acceptKill、rpcTimeout；既有观察包括 running、phase_hint、snapshot_present、snapshot_deleted，更多规则使用其声明的扩展字段。

## 3. 映射与判定边界

- 接受请求、动作真正执行、稳定状态发布是不同事件。kill 的 HTTP 成功不能直接当作 VM 已停止证据；是否表示已接受终止意图也须由固定接口合同支持。
- running store 的记录是具体系统中的代理观察，不自动等于真实执行权限；快照存在不自动等于持久可恢复。
- generation、epoch、精确 VM/slot 或权限策略无法观察时必须保留未知，不能从 API 成功推断。
- `tools/mapping/clauses.json` 与 `tools/mapping/probes.py` 保存旧 harness 的观察投影；修改底层版本、配置或字段后必须重新核对映射，不能直接迁移旧结论。
- Lean 的 `trace_monitor_sound` 证明的是其声明投影与合法模型轨迹之间的关系，不等于 Python 所有规则或现实观察适配器都已验证。

当前 CLI 退出码：0 = Conforms，1 = Violates，2 = Inconclusive，3 = 参数错误。Conforms 只表示启用且可应用的规则没有报错；必要字段缺失会跳过规则，因此**完全没有有效观察也可能得到 Conforms**。必须另核对实际规则覆盖，不能把绿色结果当作系统合规证明。

报告违反时仍需复核固定版本、配置、观察映射和底层合同。契约不同、配置不支持、观察不足与真实违反分别记录；工具输出不能直接升级成漏洞结论。

## 4. 需要外部环境的旧工具

`tools/trace/collect.py`、`observe.py`、`run_scenarios.py`、`scripts/repro_f1.sh` 依赖旧本地 E2B harness：API、Redis、Postgres、带观察端点的 dummy node 和已填充的测试数据；它不是 Firecracker 实机。

连接和本地测试凭据由 `tools/trace/harness.py` 读取的环境变量提供，不能使用生产凭据。此类采集会创建或改变测试沙箱，不属于只读命令，不进入基础 CI。重新使用时先自行准备固定环境、检查资源与费用范围；本页不承诺旧环境现在可直接运行。

固定源码出处与版本见[证据索引](../references/code-evidence.md)及 `references/source-manifest.json`。取得对应 research-sources 后可用 `scripts/check_references.py` 复核版本、哈希和本地链接；没有本地源码时不要把检查未运行说成已通过。

`tools/issues/` 的旧语料处理和 `tools/mapping/` 的校准仍是辅助研究工具，不是另一份产品规格。它们的输出与一次性实验记录放 artifacts 或任务附件，不在 docs 中持续追加报告。

