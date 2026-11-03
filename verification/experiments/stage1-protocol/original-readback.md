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

## 本批其它有限原件的搬运

建议在操作者控制的`EVIDENCE_ROOT`下分别保存`bundles/`、原prepare输出的
`original-proof-bytes/`、`production-json-original/`、`sqlite-native/readback/`、
`storage-reference/`、`captures/`、最终`native-scope/`与`configs/`。
各目录不互相充当封口；最终23原生label的scope原件不能由SQLite裸7件替代。

本批已保存的B原件档案如下；SHA验证后保留原JSON/命令/路径和source身份，不重新标注RUN。

| 包名 | SHA-256 | 内容 |
|---|---|---|
| b-storage-proof-reference-01.tar.gz | `3f1868eccd93e2903350452d49f14d30e8faf984d1ad816823a90d099e001db4` | SaveBlob6/4和SQLite8/18的44份原记录、日志、实际页/配置及origin-map；不含完整工具或构建缓存 |
| b-sqlite-convergence-native-aff2c174.tar.gz | `fbb64e03efb7ac4550088e51caecff29a4447ee95b0c608d0edd70c9fa101e75` | 原7件native packet、0.294788秒纯读回及原839装载失败；不代表scope全部通过 |
| b-api-captures-02.tar.gz | `7855c9770c129c9f27da536fc02a17c15d6dc66f601c0e358efe9a2a7ca21ac1` | 8fc真实两BLOB及SQLite完整IR/gen，capture_only |
| b-mutants-01.tar.gz | `29b14aed8285c7826669abac901515567223a8b58c76fdc33f81c8a74067ac76` | 8fc七个真实错误版原捕获，不是数学通过 |
| query-and-json-original-evidence-01.tar.gz | `69e61802bdb0b3204def6d3016cad695779646711de4fd078555318f3092d46a` | JSON精确15份原件、查询07原内核/服务/7页9审计及其固定来源，共166文件和150条原路径映射；不是整批封口 |

十包原四档的SHA以fixed-originals.json为唯一登记。JSON档案必须与production-json/readback.json
逐项相符，不拿早期单页草稿替代当前10页30审计。最终公共结果、当前查询证明和整批归档的
统一下载位置与SHA仍待完成后填写，不能据本表宣称已有完整离线复现包。

SQLite的可搬运native_result使用同目录`native-packet/correct`四件和`native-packet/fault`三件，
保留相对布局即可交严格读回。原正确项目、父失败记录和服务结果还有原路径/同安装身份检查；
在原已批准命名空间把搬运原件映射到记录中的固定虚拟前缀，再提供配置定位，不改写原收据。
这是有限运行视图，不依赖历史lzn私有目录，也不是任意路径重写器或任意机器一键重证。
正常分配、第三方库/数据库、薄接线以及V5自有同事务/恢复条件见依赖合同，复现读回不消除它们。

查询原件包的 `index.json` 保留原路径、成员摘要和来源。93份源与当前固定提供方逐字节相同；
原producer另取原提交中的固定字节；两项名称空间变换只重建已登记的原物理页。
在已批准命名空间按有限映射复原后，`SANDPROOF_STANDARD_RECORD_REUSE_EVIDENCE` 指向
`query/physical` 对应的原记录目录。原收据不改路径或revision，不附带工具安装或构建缓存。

`current-readbacks-10.tar.gz` 保存 `ca2e4338` 根的16项纯准入结果：注册、原件准备、
11份原结果、JSON、查询07和SQLite，SHA-256为
`81e78f59c8f7c959421c68228f208497bb5938cfbf5afe948136f8c6c547a220`。
均退出0，没有运行Rust、翻译器、Lean或新服务；该包不是27任务／23项普通验收封口。
SQLite局部正式原件另包 `b-sqlite-formal-resume-08.tar.gz`，SHA-256为
`68f221bc09923d75fd0051016a37b53af280c0bded60f0e1ef09974ad33d0729`，
保留原失败、4项同性质拒绝、3反例及末段服务实际结果。

固定清单另接入原 `lifetime-row` 四个视图和 `06835588` 的标准 BLOB 三个视图。
前者保留原 `9a1dc04a…` 成功记录；后者保留 `6e104100…` 数学执行记录与原
四核分配。标准 BLOB 的既有抽取复用与这次真实数学执行分开记录，后续公共元数据
变化不把任一旧记录改成新的执行。新增原字节包为 `extra-success-originals-01.tar.gz`，
摘要及精确视图仍以 `fixed-originals.json` 为准；其成功不表示另外两个标准摘要包已完成。

同一固定任务可以引用几份独立的原成功结果，例如 Create 流程的正确版、漏保存版
和提前获取 World 版。`originals(result)` 按清单中的视图顺序返回这些未修改对象；
`views` 另保留各自原提交、CPU、服务 revision 和实际文件。不会将它们伪装成一个
曾经运行成功的原父批次，也不会改写另一次未完成的父结果。公共入口仍分别核对
完整视图及其同一性质的拒绝证书，直接读回不新增证明或扩大验收范围。
