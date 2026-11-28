# 终局确认与关闭值证明

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

本切片固定 #686 的 `39577ca1e61f4daa4ba4bbda7db94bf087bdaef0`，实际 protocol 源与原 a9c 基线相同。
`src/lib.rs` 只透明调用真实方法；`terminal` 抽取视图复用原 Request/Accepted/Recorded 四根与四个逐符号外部模型，
加入 terminal matches、confirmed matches、close shape、closed matches。类型、函数体和原始 LLBC 只由固定 Charon/Aeneas 生成，
在私有目录安装为独立 Request 视图，原工具带 axiom 的模板仅保留为证据，不导入。

`ResultModel` 是四个完整值合同；`ResultEquality` 连接完整 observation/intent 与 status 比较，
`ResultConfirmation` 对任意输入证明四个根的确切布尔结果及接受充要条件。
`ResultValues` 的相同值供正常与全部错误版复用，`ResultExamples` 给合法见证与字段变化拒绝。
Retention 仅沿用 slot 范围条件，不增加 key window 与 retention window 相等的要求。
终局 matches 不检查 status，也不重新 validate 请求或认证 center；即使协议版本不合法，也可与自身匹配。
确认与关闭须 Cleaned；关闭回执须完整 terminal 相等且 retired_through 精确等于请求的 retention window，不能用较新窗口替代。
这些只是已解码值的纯守卫，不证明已授权、已持久确认、实际清理或位置可复用。

`inventory.json` 登记保留证明、精确审计与四个真实 Rust 变异：terminal key-only、confirmation key-only、允许 Pending 关闭、
用较新 frontier 代替相等。正确与错误版保持同一完整合同和见证。编译、抽取或导入失败均不是性质反例。

开发形式入口（先取得共享重锁；`--packages` 指向本任务分配的固定依赖缓存，每个 case 有独立 `.lake/build`）：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/result-confirmation/formal.py \
  --packages /private/task/.lake/packages --output /private/result-formal
```

入口实际重新编译、抽取每个版本，检查完整本地导入闭包中的每个真实文件，使用原内核、严格 sandbox-lean 同服务诊断和准确依赖审计。
`--case` 仅供诊断；结果显式区分全部 case 完成、局部形式通过与完整验收。
原始失败、中断、生成模板、源码、工具与完整输入摘要均留在 artifacts。

ning 使用本组件原 `archive.py` 生成的同提交完整源码归档，再执行：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/result-confirmation/native.py \
  --source-archive /private/source.tar --target /private/result-native-target --output /private/result-native
```

原生入口检查正常六项断言、格式/Clippy；每个错误版执行原版、变异、恢复三次实编译与同一断言，
核对实际两个自有库/测试源、manifest、fresh=false、二进制与全部源码恢复。只清本任务两个包的变体缓存，保留依赖。
此结果不是共享 native seal；主控须将本包、全部实际文件和准确审计纳入公共登记、原 `verify-stage1-protocol.sh` 与同批 `proof_native_suite`。
现有旧根通过不能覆盖这些新根，历史冻结不得只刷新 hash。候选登记、覆盖及精确证据随 PR 交接，公共配置不在本包并发写入。

续租三个根保留为抽取预检：固定工具可翻译其原 `Result::is_err/is_ok`，但 Lean 内置 `core.result.Result` 缺少对应判别标签实例；
它们不属于本切片已证范围。该最小库表示接口交 V0/主控核对原枚举标签；不导入自定义公理或工具模板补洞。
[续租独立切片](../renewal-confirmation/README.md)提供该表示候选及三个守卫的独立检查，不迁记为本终局切片的结论。
JSON/认证来源、原 journal 持久确认、窗口关闭事务和未结资源责任、重启/乱序/竞争与 V1–V7 组合仍是独立义务。
