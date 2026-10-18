# Redis 调用与结果的隔离候选

[验证入口](../../README.zh-CN.md) · [依赖保证](../../../docs/formal/dependencies.md) · [证明工作流](../../../docs/proof-workflow.md) · [任务 #564](https://github.com/VeriInfra-Lab/sandproof/issues/564)

这里保存真实 Rust 的候选拆分及其来源，不修改当前生产文件。原公开 `commit/reaffirm` 在候选里转调同一份泛型业务函数；原自有 codec 保持原实现，通过显式回调传入，时钟也显式传入。结果解释、身份检查、修订推进、健康检查和写围栏决定仍有真实 Rust 函数体，不能把这些决定当作外部库的保证。

[candidate.json](candidate.json) 固定原基线、候选文件摘要、工具和调用根；[candidate.patch](candidate.patch) 只应用到新私有副本。抽取包装通过路径包含该副本的真实文件，不复制业务算法。

| 调用根 | 覆盖的实际候选代码 |
|---|---|
| `finish` | 已解码输入的完整参数、结果解释、后置健康检查及未知围栏 |
| `commit` | 围栏拒绝、时钟和编码调用顺序、替换检查及结果核心 |
| `reaffirm` | 原载荷克隆、时钟、原身份与新修订、替换检查及结果核心 |

后两项抽取的是公开方法实际调用的泛型业务体。具体 `JsonCodec` 的正确性、公开薄转调与原版本的一般等价，不由抽取成功自动得到，也不因此成为可信库前提。

## 重建与编译

在批准的 Linux x86_64 验证环境中，从包含冻结基线的仓库重建；目标目录必须不存在：

```sh
python3 verification/experiments/redis-result/prepare.py \
  --source-base /path/to/sandproof \
  --destination /path/to/new-private-directory
```

准备程序核对原文件、补丁、模板文件清单及应用后的全部候选摘要。它只准备源码，不运行抽取或证明；额外模板文件也会被拒绝。

已有生成模块使用附加视图编译，不改变原 Lake 配置或依赖版本：

```sh
lake -f lake-storage.toml --no-cache build \
  SandproofStorage.Results SandproofStorage.Key
```

这条命令只检查所列模块，不代替重新抽取一致性、错误版检查或项目完整验收。正式范围及验收状态以任务和覆盖记录为准。

## 如何理解接口与定理

[Results](../../aeneas/storage/SandproofStorage/Results.lean) 直接使用未修改的生成函数：未知结果必须保持写围栏并弃用连接；已围栏的普通提交保持账本和回调状态；成功结果需要完整 CAS 调用及后置检查证据，并保留修订、状态和完整载荷。四个已知拒绝码的定理只解释返回值及本地状态，不从 `Err` 推出 Redis 没有写入或已经回滚。[Key](../../aeneas/storage/SandproofStorage/Key.lean) 复用既有字符串往返证明，保持完整部署字符串。

外部值模型保留正常回复、调用前拒绝、送达未知、panic 和不返回。`LibraryRequest` 记录传给适配的参数，不代表数据已经送达、执行或耐久化。定理中的正常返回不保证调用一定返回，也不为 panic 后的状态提供围栏承诺。业务侧仍须连接异常处理、禁用和恢复义务。

支持域是有限且可表示的字符串/字节、正常分配、固定纯 `String/str` 字典及普通值载荷；不覆盖有可观察自定义 `Drop`、共享可变资源或任意自有比较器的泛型实例。连接、文件和时刻在本接口未观察的内容用身份或值表示，不能据此推导物理关闭、释放或总墙钟期限。

Lua、自有 codec、读取/reconnect、具体调用方和崩溃恢复仍有各自义务。此目录及其局部定理不表示生产存储、整个 V0 或部署系统已经验证完成。
