# Create 标准实例的完整 World 与连接重绑定

本包连接当前唯一 `StandardCreateAttachment` 的 `world`、
`begin_lifetime_update` 和 `begin_inbox_save`，以及实际
`WorldTransaction::world`、两层原 `Deref`。不修改这些生产函数或 V5。
它是 [#635](https://github.com/VeriInfra-Lab/sandproof/issues/635) 完整 V3 的一个
条件值切片，尚不能代表整个标准实例、持久收件或系统结论。

`StandardCreateModel.lean.in` 的同一 `FullReadAndRebindContract` 覆盖任意完整
效果对象和全部模型分支：先借用实际 candidate，缺失时 panic，正常时返回完整
World；两个重绑定各自读取实际 transaction 槽，缺失时 panic，成功时选择它的
完整 Connection，保留所有其它字段。World 读取没有被加强为检查 transaction；
重绑定也不额外要求 candidate 存在。

真实透明生成体保留 World 的全部 20 字段、完整维护/记录/计划和
`OperationPlan.capture`、WorldTransaction 的四字段、CommittedWorld 的两字段、
标准效果对象的两字段及 rusqlite Transaction 的 `conn/drop_behavior`。
固定库的借用载体是本次只读区间的观察，不是把可变共享堆复制为不可变快照。

既有 `SandboxModel/Access/Cloning.lean` 在独立视图中只换导入和命名空间，并导入
当前完整 `CaptureHandoff` 缺失的 clone 引理；原数学正文直接复用，不重新计作
新开发。共享 Access 文件没有被修改。String/Option clone 与 kernel-bound
字面量绑定复用已有定义；不导入带 axiom 的生成模板或旧业务类型环境。

## 条件与未完范围

- A6：固定两个 Std 版本、x86_64 的这次 RefCell 借用、issued Ref 读取和正常释放
  区间相符；实际计数来自 signed-isize 有效域，正常借用要求非负且小于 isize::MAX。
  普通 World 克隆不回写此单元，不扩任意共享堆、自定义 Drop、重入、unwind、
  OOM/abort 或崩溃。`borrow/try_borrow/BorrowRef::new/drop/Ref::deref` 的两版
  相关正文已逐点核对，但没有机器证明整个 RefCell 实现。
- 原 String/Option/Vec 的正常值克隆条件保持公共登记范围；自有记录和完整 World
  的 clone 始终是真实生成函数，不被设为外部业务回调。
- V5 提供每次调用的实际 candidate 对象、活事务、writer lease 和连贯 API 历史。
  本包没有假设或证明整个 begin/recovery/transaction 正确。
- A4 Connection 完整观察载体沿用唯一公共定义。字段值相等不推出物理对象相同；
  原连接、同一 SQL 事务/行效果、提交未知和重启仍须实际历史接线。

此前双输入 identity、两参数 lifetime SQL 和 generic Create 完整值/首错/顺序
证明保持各自范围。标准配置读取、真实 JSON/输出/SHA、producer/driver、Resume
支持域与持久收件仍另列；原请求 `target > source` 不改成必须加一。实际授权、
资源/文件/VM 存在和耐久接受均不能从本包的值等式推出。

## 正反与复现

正确版保留真实实现。`default-world` 在真实 Rust 方法中仍读取原 World 一次，
随后错误丢弃完整值并返回 `World::default()`。错误源必须实际编译/抽取并到达
同一个完整合同的反例；仅保留空 World 时相同的正常情形。共同规格、完整观察
见证、库和下层数学不随正反版本更换；错误视图不安装或导入正确标准实例定理。
编译、导入、超时或元数据读取失败不算性质拒绝。

证明机使用固定 Charon/Aeneas、独立 `.lake/build` 和原重锁，运行：

```bash
python verification/experiments/stage1-protocol/create-standard-adapter/formal.py \
  --output artifacts/create-standard-adapter-current \
  --lock /path/to/existing/.proof-heavy.lock
```

`--prepare-only` 只捕获和构建，不是完整验收。完整开发结果要求全部实际文件的
原内核、规定同服务完整非 stale/partial 诊断与依赖核对。LLBC 元数据用有界流式
读取：64KiB 缓冲，单项最多 4MiB，不整体载入 AST，也不更改固定工具。

ning 运行原生入口：

```bash
python verification/experiments/stage1-protocol/create-standard-adapter/native.py \
  --output artifacts/create-standard-adapter-native
```

actual/default-world/restored 三视图使用完全相同的三个断言，新编译并核对真实
protocol 库和 service 单元测试源/manifest/链接，检查精确失败集合、格式及严格
Clippy。原真实 journal 配置的非默认完整 World 用于检查值保持；另有读后原值
保持及两次实际连接指针选择的有限 API 见证。这不等于一般物理事务证明。
原生入口源码和开发 Lean 通过与 ning 的正式回执分别报告。

公共登记、根/输入/审计、依赖与覆盖仍由主控单写，见 `REGISTRATION.md`。
`scripts/verify-controller.sh`、共享 proof_native_suite 和完整 V3 验收由主控
同批统一运行或按精确内容复用；本包局部结果不替代这些入口。
