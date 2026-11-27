# 协调器重构后的两个收件配置组件

此组候选适用于 `integration@87bc20b8`、`#1061@de37231d`、`#1081@9be6c5e3` 的隔离组合。先采用 #1081 的协调器登记、事务提供方、默认输入及覆盖说明候选，并按 #1082 处理封套清单。[协调器证明](https://github.com/VeriInfra-Lab/sandproof/pull/1081) · [封套输入](https://github.com/VeriInfra-Lab/sandproof/pull/1082)

[配置输入候选](config-consumer-inputs-candidate.patch)更新两个组件各自对 `controller-service/src/application/coordinator/lifecycle_intent.rs` 的引用，以及默认清单对这两份 inventory 的引用。两个组件各登记 261 项生产输入。相对 #1061，此组合在每份清单中都只改变协调器这一项。历史捕获的记录和证明文件仍可在原提交复查。

| 组件 | 合同及复验范围 |
| --- | --- |
| inbox-config | 对全部有类型的容量字段，`required_bytes` 返回完整预算公式。三个错误版分别少计 options、manifest 或全局预留。[容量公式及边界](../inbox-config/README.md) |
| inbox-config-validation | 在原字符串表示条件下，布尔结果对应完整部署、host、字段范围和预算条件。三个错误版分别放宽 host 版本、漏掉 profile 下界或放宽总预算。[配置接受条件](../inbox-config-validation/README.md) |

在 #1061 原源码上，使用各包的 `formal.py --prepare-only --variant` 重抽每个登记视图。在采用候选的组合源码上，对相同视图运行完整组件形式入口。两个版本各用独立源码与构建目录。对照时以各自 Cargo 的依赖文件确认真实控制器库的编译输入，再比较完整 LLBC、全部生成文件和原有合同。LLBC 比较采用仓库既有规则，仅对键唯一的 `short_names` 表排序。[LLBC 比较方法](../../../../tools/check_foundations_extraction.py)

[默认登记器输入候选](default-registrar-inputs-candidate.patch)只更新默认 Stage1 清单引用的 `registered.py` 和 `test_registered.py`。主控可分别查看、试套这两个补丁，再以同一提交运行原 SHA 和 full 登记。默认 SHA 成功后，full 登记仍会逐包核对其它消费者。独立事务及收敛入口按各自的冻结和验收范围办理。[晋升入口](../../../../tools/ci_promotion_acceptance.py)

主控选定提交后，ning 通过原 `native_task.py run --scope stage1-protocol` 跑完整普通检查和已登记的 V3 原生检查，其中包括 `stage1-inbox-config` 和 `stage1-inbox-config-validation`。证明机消费同一提交的正式回执，运行默认的 `bash scripts/verify-stage1-protocol.sh`。容量公式的形式结果只涉及已解码配置，物理资源的实际预留由资源提供方验收。[双机验收与原始回执要求](../../../../docs/proof-workflow.md)
