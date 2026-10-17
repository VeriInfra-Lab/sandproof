# 重新派发凭据、寿命与 Access 授权隔离试点

[验证链](../../README.zh-CN.md) · [网关计划](../../../docs/formal/08-data-gateway.md) · [依赖登记](../../../docs/formal/dependencies.md)

本包把实际重新派发路径中的凭据与寿命值校验拆为同步 Rust 核心，并直接抽取原 Access 实现。使用固定、未修改的 Charon/Aeneas 和原 Lean 服务客户端。生产 Rust、默认抽取根、全局验收脚本及工具版本均不改变；试点只安装到独占源码副本，不默认启用。

冻结实现为 integration `0a23b57af60752f61067fc00b0b0b656e98347cb`。本包依赖同一 PR 的[数据许可试点](../data-permit/README.md)，复用其真实调用副本和七个守卫；旧包完整输入按固定摘要核对，不能静默更新。源码审查和实际接线的限制见[调用对应记录](audits/call-binding.md)。

## 验证哪些真实代码

| 部分 | 真实 Rust 与证明范围 | 尚不由此证明 |
| --- | --- | --- |
| 凭据 | `verify_target` 在隔离副本调用 `registered_digest`、`inverse_binding`、`digest_matches`；`digest` 调用实际字节格式循环。验证缺失/错代数、完整反向二元组、64 字节 ASCII 十六进制格式与完整摘要相等判断 | 凭据 SQL/index、身份编码与持久注册/poison/恢复；SHA-256 内部或摘要相等无条件推出任意原令牌相等 |
| 寿命 | `read_lifetime` 调用原始行校验核心：owner 字节、deadline、正数 disk、可选 started、max/startup 字段与转换；`live_lifetime` 依原顺序调用时钟、owner/phase 和到期核心 | 选中了哪个数据库对象、列/参数绑定、合法寿命最初如何创建/更新、数据库与缓存的共同快照、所有未来时刻均有效 |
| Access | 直接抽取原 `handle_exec/file_read/file_write/handle_access` 及其真实查询、能力判断和 World 复制；验证精确首个错误、成功条件、事件和整个 World 不变 | 完整 `handle_controller`、实际 OS/VM/文件权限、物理中继执行或整个异步网关 |

输入包含实际值与原始行，不用“凭据有效”“寿命有效”“授权正确”这类业务结论替代检查算法。机械接线仍须满足所列同对象、同锁与参数条件。模型/生成体/证明/版本的精确闭包由包内清单登记，结果以该输入的实际复现收据为准。

凭据字节比较把原 `iter().zip` 改为全量下标 XOR/OR 循环；寿命可选字段改为显式分支，正数检查后的转换使用原工具已有的 `cast_unsigned`。这些是隔离真实调用所用的 Rust，不是只供验证、生产路径不调用的替代算法。ASCII 范围处仅有一个带原因的局部 Clippy 风格例外，精确范围由证明覆盖；没有新增外部模型或屏蔽全局检查。完整循环的功能证明不等于编译后常数时间、内存预算或密码侧信道证明。

## 正确与错误版本如何区分

三个独立错误副本各只改变一处真实检查：

- 摘要比较：保留完整扫描后强制判为相等。
- 到期边界：把 `deadline <= now` 改为 `deadline < now`，错误接受恰好到期。
- Access：绕过真实 Exec 的 effective capability 拒绝分支。

错误版必须成功编译和抽取，再被与正确版相同的安全性质拒绝，并有正向形式反例。应用测试仍调用原 `ControllerApplication` 和真实 SQLite，受控 Host 记录实际 `spawn_data_relay` 调用后返回 Busy。次数为零只指这一派发接口；此前仍可能读取 Host token 或查询数据库，不表示所有错误都没有副作用，也不表示启动了产品 relay 进程。

原始/正确隔离副本运行同一组测试，核对完整语义记录。摘要/Access 错误版分别由令牌变更和 Closed 权限的原断言拒绝；到期错误版同时违反精确截止与原耐久到期后的新许可拒绝。正确版的服务与内核检查、坏版同性质拒绝、应用对照和整个仓库门禁分开记录；编译失败、零测试、服务超时不能冒充负例。

Closed 能力场景使用独立、显式启用的测试 feature，只修改已有缓存中一个 record 的 effective，保留 owner、Running、VM、代数、SQLite、时钟和凭据。它检查防御守卫是否实际执行，不宣称该状态可从合法持久历史到达。此 feature 不在 Access 抽取时启用。

## 仍依赖什么

外部 SQLite、Tokio、OS 与密码原语仅提供[登记的接口保证](../../../docs/formal/dependencies.md)。原管理锁、watch/gate 身份、机械字段投影、查询结果转发和实际 Host 参数接线按获批薄适配合同依赖；检查算法及错误/未知的业务含义不整体可信化。

本包机器证明直接适用于隔离副本的同步核心和原式 Access。原始/拆分调用路径的一般等价定理未交付，目前依据逐分支审查与有限应用对照；正式接入生产需要使用同一核心或提供另一个明确对应依据。SQL 文本/参数/列、身份字符串、缓存发布/恢复、撤销决定、真实退出/排空和全局资源预算仍是本项目待证责任。

成功寿命检查只约束采样时刻，不能保证后续 I/O 永不到期；运行时取消、panic、abort、崩溃与 Drop 不保证回滚或真实中继退出。初次 `data_target` 的 supplied-token 比较、配置绑定及其他入口不能借本批重新派发后缀证明直接覆盖。T9 继续排除，关联总任务不因该局部试点关闭。

## 离线复现

在获批的 x86_64 Linux 证明环境执行，预备固定原工具、Lean/MCP runtime、离线 Cargo/Lake 依赖和受保护磁盘临时目录，不运行安装或版本升级命令。所有项目验证在远程证明环境完成，本机仅编辑、读取、提交和传输。

```sh
python3 verification/experiments/data-authority/scripts/verify.py \
  --baseline-archive /prepared/baseline.tar \
  --source-repository /prepared/source-repository \
  --data-permit-package /prepared/data-permit \
  --tool-root /prepared/fixed-aeneas \
  --mcp-runtime /prepared/lean-mcp \
  --lake-packages /prepared/lake-packages \
  --cargo-home /prepared/cargo-home \
  --tmp-directory /prepared/private-disk-tmp \
  --output /prepared/new-experiment-output
```

路径仅为调用方参数示例。基线 archive 必须由上述冻结提交导出；输出目录必须全新且不与输入、源码或缓存重叠。入口先核对本包与旧包、完整基线、原工具/客户端及依赖，再安装相同测试 overlay；原始、候选及三个错误副本各有独立 Cargo target。

原始 LLBC 全字段保留，仅复用冻结项目审计器对无序 `short_names` 表排序并拒绝重复 key；生成 Lean 按字节核对，不删除元数据或手改生成文件。各变体使用私有项目构建目录，只复用固定且 clean 的第三方 packages。服务检查要求对应文件/配置摘要、非陈旧结果和真实诊断；之后独立内核检查并审计实际声明依赖。

`receipt.json` 和逐步日志写入输出目录，失败或输入漂移不能输出整体成功。仓库原 `scripts/check-function.sh`、`scripts/verify-controller.sh` 仍须独立完整调用，局部试点成功不能替代整套验收；其准确结果、环境阻断和最终候选提交记录在关联 PR。
