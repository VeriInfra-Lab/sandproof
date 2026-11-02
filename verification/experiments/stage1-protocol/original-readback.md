# 固定 V3 原证明的证据读回

本入口只接收 `fixed-originals.json` 中十个包、十一份原结果，不能搜索另一份成功记录，
不能替代本批原生准入或全部 V3 验收。它不运行 Rust、翻译器或 Lean。

在操作者控制的证据目录保存清单所列四个原始归档后，先准备一次可迁移的有限目录：

```sh
python3 verification/experiments/stage1-protocol/original_readback.py prepare \
  --bundle /private/original-bundles --output /private/original-proof-bytes
export SANDPROOF_V3_ORIGINAL_EVIDENCE=/private/original-proof-bytes
```

归档摘要固定；准备时只复制选定原结果、原命令和服务日志，以及所需原生成文件。
目录中的 `index.json` 保留原路径与内容文件的对应，不读取原账号的私有路径。
正式批次由登记入口逐个调用 `run --job <原任务名> --output <新的私有目录>`，
并继续执行当前普通准入和最终汇总；直接调用成功不表示这些外围检查已通过。

结果显式记录 `REUSE`，保留原结果路径、摘要、提交、CPU 和原服务 revision。
原物理文件的字节来自当前相同提供方、既有明确变换或已接收的原生成物；所有业务
源码适用性差异依照已审固定清单。未知源变化拒绝，不能只刷新摘要。
原全项目源码指纹由原结果中的输入记录按原 `Path` 排序重建；不要求当前仓库的无关
新增文件与旧全仓相同。原内核命令、日志、完整服务请求、诊断与审计仍逐项检查，
只转发原服务的 `Checked` 消息，不从声明清单合成通过记录。

物理页和日志在新的私有目录中按内容复制，以便现有检查器读取；用于核对这些副本
的命令路径映射与原执行命令分别保留。没有在新目录重新编译，不修改原 JSON，
也不生成冒充旧执行的新检查点。缺少或损坏原证据须补验，不能把读回失败记为证明失败，
或反过来绕过检查器。边界和最终状态见[证明工作流](../../../docs/proof-workflow.md)
与[覆盖记录](../../../docs/proof-coverage.md)。
