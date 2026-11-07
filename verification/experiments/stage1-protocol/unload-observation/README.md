# 完整 Unload 结果值合同与投影

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

固定 #690 的 `e7403e13149f4312503ab07a08ace0642b441657`；生产只读。
本包对应真实 `UnloadObservation::matches_request` 及 `UnloadOutcome::unloaded/continued`，复用原请求、封套和 ReadyFacts 时间规格。
薄根直接调用原方法；固定 Charon/Aeneas 的独立 Request 视图还保留原四个请求/封套根与三个 ReadyFacts 根。
工具实际输出在私有目录安装，不修改生成体或导入 axiom 模板，不更换原四项逐符号外部模型。

值合同区分 Pending、Unloaded 和 OriginalContinued：完整原意图、非零 accepted version、原版本关系和 retention slot 形状；
Unloaded 要求正 material_bytes、来源代/capture 相等及原时间形状，OriginalContinued 使用原严格观测时间关系。
两个投影保留对应 payload 的全部字段，并在其他 outcome 返回 None。证明范围从已解码值开始。

matching 不重新 validate 请求协议或 center，不要求 key window 等于 retention window，也不检查 manifest_digest。
正材料字节数字段不表示实际材料已经持久保管，完整 digest 的值投影不表示计算过哈希。
V1/V2 提供原意图与耐久发送，V4/V5 提供独立 journal 结果及保管/继续责任，V6 提供真实文件、退出或同一原 VM 继续事实，V7 提供独立认证来源。
Pending 不证明没有产生效果；OriginalContinued 不是仅凭当前 Running 或缺失回复构造的结论。

四个实际 Rust 错误版分别改为 key-only、遗漏 capture、允许零材料字节、允许恰好到期的继续观测。
正确/错误版保持同一完整合同与见证；编译/抽取/导入失败或超时不算性质拒绝。
局部定理、普通与错误版原生检查、共享完整验收分别报告；精确执行、输入闭包和待验收项随对应 PR 记录，公共覆盖仅由主控协调。

形式入口（先取得共享重锁，使用本任务分配的固定依赖缓存，各 case 独立 `.lake/build`）：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/unload-observation/formal.py \
  --packages /private/task/.lake/packages --output /private/unload-formal
```

入口使用原内核与严格 sandbox-lean 同服务检查完整实际导入闭包、非 stale/partial 诊断与准确依赖审计；保留全部失败证据。
ning 用原 archive.py 生成的同提交完整归档执行：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/unload-observation/native.py \
  --source-archive /private/source.tar --target /private/unload-native-target --output /private/unload-native
```

原生入口保留正常八项断言、格式/Clippy，以及每个错误版的原版→错误版→恢复实编译与同一断言；
核对两个自有包与测试的 source/manifest/fresh=false、二进制和全部源码恢复。该记录不自行签发共享 native seal。
公共透明根、生成视图、原 verify-stage1-protocol、native/审计登记、冻结、覆盖与共享完整入口由主控同批协调，旧根通过不覆盖本包。
Create/续租确认、JSON/认证接线、持久收件、确认/窗口退休、故障竞争及系统组合仍独立登记。
