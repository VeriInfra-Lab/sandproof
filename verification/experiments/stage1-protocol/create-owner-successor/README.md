# Create 排除新 sandbox 的 owner 后继候选

[V3 设计](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md)

这是 AE 上的后续独立开发候选。按 #635 owner 评论 `6513509154` 继续闭合 V3 自有调用/映射/事务业务，现有分支无需迁移；原统一验收与汇总 PR 暂停。本目录不修改生产、公共 Lake/根导入/登记/冻结或覆盖记录，不追加 AE+65 旧封口。旧结果保留原提交身份。

## 实际计算与合同

原 `attach_create` 在没有现存 owner 行时，从原 `tx.world()` 的全部 sandbox 记录中排除 `a.local.sandbox`，取其 owner 与 inbox 高水位的最大值，再受检加一；耗尽返回 IdentifierExhausted。原 `owner_for` 的全表证明不包含这项排除。

`candidate.patch` 仅应用到新建私有源码副本：把该 else 分支的重算移入 `create_owner_successor`，用显式有界索引循环保留过滤、最大值和受检后继。原 owner 查找分支保留，World 仍只在原缺失 owner 分支读取。薄根转发完整 Inbox/World 的两个原共享借用和原 sandbox 值。准入检查真实 MIR 参数来源、完整 JournalError、实际透明根和完整生成闭包。原父函数一般等价、完整父调用证明和生产采用尚未完成。

`CreateOwnerSuccessor.FullSuccessorContract` 对任意已解码完整 Inbox、World 与排除 ID，要求实际生成 helper 返回既有 `OwnerMapping.allocation` 对既有 `OwnerMapping.maximum` 的结果，输入为排除该 ID 后的原记录表及原高水位。复用原模型，未引入另一套协议或 owner 定义。`successor_complete` 证明任意表的终止、索引/计数边界、全部过滤与高水位参与和最终错误/正常返回，不假设记录唯一、ID 非零或资源已存在。

七个正例连接实际函数，覆盖空表、排除最大行、排除 ID 不存在、高水位、被排除行达整数上限、其他行达上限及高水位耗尽。四个错误版在同一真实 Rust helper 中反转排除条件、用 owner 字段代替记录 ID、遗漏高水位及使用 wrapping。每版必须实际编译、抽取，再证明同一个 `FullSuccessorContract` 的否定；编译/导入失败、超时或准入拒绝均不算业务反例。正反视图共用逐字节相同的规格/值文件，错误视图仅复用展开/结束连接，不导入正确循环对应或完整合同定理。

## 已有依赖与范围

`reuse/` 是固定未合入交付 `2e35b8c8619e05a1e4bb540105bd3e7f20725615` 中五个文件的原字节：owner 模型、已有算术引理、值见证、有限 Ord 字典投影和错误值别名。清单核对原字节及标准库/固定后端摘要。本目录不重做这些公共性质，也不将旧执行迁记为新执行。

固定后端 Ord 默认方法的有限 U64 字典投影仍为未采用候选，原定义不变。本次零外部函数的生成文件没有 FunsExternal 导入，因此仅在 TypesExternal 提前导入同一 Ord 绑定；后接原错误别名体。没有编辑生成体、修改工具或填充 axiom 模板。该接线及生产采用仍与正式公共依赖分开记录。普通值模型不保证物理分配、OOM/abort、真实 VM 或文件事实。

后续完整 owner/high-water/session/entry 状态修改、原 i64/ID/SQL/save、同事务行连接、commit/未知提交/重启、JSON/TLS 与持久收件组合仍归 V3。字段值关系不证明授权、已持久接受或资源存在；V7、V1/V2、V4/V5、V6 的业务提供条件仍分别保留。已有 Manifest 任意已解码值证明和 `ProtocolRecord.full_flow_contract` 保留原适用范围。

## 检查入口

证明机使用固定工具、四核/四线程、独立 `.lake/build` 和已存在共享重锁：

```sh
python3 verification/experiments/stage1-protocol/create-owner-successor/formal.py \
  --variant all --output artifacts/create-owner-successor-development-01 \
  --lock "$SANDPROOF_PROOF_HEAVY_LOCK"
```

入口重新抽取五个真实视图，并调用原严格检查器覆盖所有安装的真实文件及导入闭包、内核、同服务完整非 stale/partial 诊断和依赖审计。它只修改各私有副本的 Lake 配置，不修改公共配置。该后续包未进入共享入口，局部通过不代表 `verify-stage1-protocol.sh`、普通/原生或整套验收通过。

`native.py --output <新目录>` 交 ning：候选、四错误和恢复候选共六视图共用九条原断言，检查实际协议库/服务测试新编译、格式、Clippy 和精确失败集合。它未在证明机运行。共享登记与原生统一验收由主控后续协调，不因本包局部证明而自动完成。

原始结果保留精确输入、执行位、工具/服务、MIR/未改生成体、失败记录和完整审计；GitHub 源可读与主控原始包读取回执分开记录。完整 V3 仍未完成。
