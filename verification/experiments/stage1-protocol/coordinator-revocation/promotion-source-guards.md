# 晋升预检中的四个原组件来源检查

在 #1061 的 `7895f004` 上，默认 Stage1 冻结清单与 `validate_registration(profile='full')` 均通过。随后分别调用 `lifetime-row`、`pause-request`、`source-origin` 和 `source-question` 的 `support.source_guard()`，四者均拒绝 `docs/proof-workflow.md` 的旧引用。此前的 #1061 提交 `de37231d` 通过四个读取器。#1067 改动了工作流文档中的晋升和手动验收要求。[工作流变更](https://github.com/VeriInfra-Lab/sandproof/pull/1067) · [实际读取器复核命令](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-7231982197)

四个组件的原生与形式入口均执行自己的来源检查，覆盖 `inputs`、`providers`、`templates` 和 `drivers`。晋升预检的公共登记步骤已检查这四包的 `inputs`。文档位于各包的 `providers`。候选补丁在预检完成原登记步骤后调用候选检出目录中的四个原读取器。拒绝原因写入现有的 `stage1-registration.log`，该次预检随即退出。

主控可在仓库根目录检查[晋升预检候选补丁](promotion-source-guards-candidate.patch)。该补丁修改晋升工具及其针对性测试。在 `integration@87bc20b8` 和 `#1061@7895f004`，这两个目标文件的内容相同。

```sh
git apply --check verification/experiments/stage1-protocol/coordinator-revocation/promotion-source-guards-candidate.patch
```

## 验证范围

| 实际输入 | 现行预检 | 应用候选后的预检 |
| --- | --- | --- |
| #1061 的 `7895f004` | 通过 SHA 与 full 登记 | 拒绝首个旧工作流引用 |
| 在隔离副本将四个工作流引用指向当前文档 | 通过原有 SHA 与 full 登记 | 四个原读取器均通过 |
| 分四次仅放回一包的旧引用 | 四次均通过 | 四次均由对应原读取器拒绝 |

候选预检也能在旧提交 `de37231d` 上执行四个原读取器并通过。原晋升工具的回归测试和新增测试在 msra 作开发检查。新测试分别让四个组件读取失败，能在旧实现上复现漏检。

表中的四处引用调整仅用于构造预检对照。这个交付覆盖进入普通验收前的来源检查。各包当前版本的抽取、Lean、原生和完整晋升仍需各自的回执。主控固定可准入的提交后，ning 按该提交运行普通和原生验收。[证明工作流](../../../../docs/proof-workflow.md)
