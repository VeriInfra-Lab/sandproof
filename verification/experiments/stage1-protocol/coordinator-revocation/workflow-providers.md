# 四个协议组件的工作流输入

主控采用此补丁的基线是 #1061 的 `7895f00426c6e8c3b412cb4a278f42b50b7633d8`。#1067 更新了手动原生验收、恢复和晋升要求。四个组件都把这份工作流列为验收依据。补丁各更新一份 `inventory.json` 的 `providers["docs/proof-workflow.md"]`。[候选补丁](workflow-providers-candidate.patch) · [工作流变更](https://github.com/VeriInfra-Lab/sandproof/pull/1067)

| 主控准备的源码 | 仅应用本补丁后，四个组件实际读到的结果 |
| --- | --- |
| #1061 的 `7895f004` | 四个 `support.source_guard()` 和四个 `formal.inventory()` 均通过。 |
| `integration@87bc20b8` | 每个组件仍有两处过期引用，分别指向协调器 Rust 和应用回归测试。#1061 已对齐这两处。 |
| #1061 叠入 #1081 的协调器生产版本 | 每个组件均拒绝已改变的 `lifecycle_intent.rs`。该 Rust 引用需随生产版本单独复验。 |
| 按 [#1090](https://github.com/VeriInfra-Lab/sandproof/pull/1090) 和 [#1089](https://github.com/VeriInfra-Lab/sandproof/pull/1089) 组装的公开源码 | 四个组件的两个原读取器仍各自拒绝同一个协调器 Rust 引用。共享 full 登记另外先停在 `digest-uses`。 |

对于含 #1064 生产重构的组合，先采用工作流补丁，再由主控应用[协调器来源补丁](coordinator-source-inputs-candidate.patch)。第二份补丁更新四个组件各自的 `inputs` 中同一个 Rust 路径。这四份清单均由原组件入口直接读取。两份补丁都采用后，在下列命令中可逐项核对。仅采用原协调器的 #1061 使用上表第一行。

在准备好的闭包中提交候选后，主控可在仓库根目录逐个执行原读取器。下面的命令会在第一个失败项退出。#1086 给公共晋升预检提供了相同来源检查的登记候选。[晋升预检](https://github.com/VeriInfra-Lab/sandproof/pull/1086)

```sh
bash scripts/python-entry.sh - <<'PY'
from pathlib import Path
from tools.proof_native_suite import load

area = Path.cwd() / "verification/experiments/stage1-protocol"
for name in ("lifetime-row", "pause-request", "source-origin", "source-question"):
    package = area / name
    for module, check in (("support", "source_guard"), ("formal", "inventory")):
        reader = load(name.replace("-", "_") + "_" + module, package / (module + ".py"))
        getattr(reader, check)()
    print(name, "passed")
PY
```

| 组件 | 正确视图的合同 | 使用同一合同的三个真实 Rust 错误版 |
| --- | --- | --- |
| [lifetime-row](../lifetime-row/README.md) | `LifetimeRow.accepts_iff` | `skip-owner`、`skip-startup`、`lost-start` |
| [pause-request](../pause-request/README.md) | `PauseRequest.accepts_iff` | `origin-binding`、`native-incarnation`、`options-binding` |
| [source-origin](../source-origin/README.md) | `SourceOrigin.full_contract` | `skip-session`、`skip-generation`、`skip-vm` |
| [source-question](../source-question/README.md) | `SourceQuestion.Candidate.accepts_iff` | `first-resource`、`options-binding`、`disk-as-material` |

ning 在主控固定的同一提交上运行 `stage1-protocol` 原生范围，包含 Linux `functional` 及四个 `stage1-protocol-<组件名>` 登记任务。保留原观察器回执和完整封口。msra 的正式形式入口消费该提交的原生封口。这四个组件的结论各自从已解码的输入开始，JSON 字节、持久事务和网关仍需各自的验收。[证明工作流](../../../../docs/proof-workflow.md) · [V3 组件入口](../README.md#验收入口)
