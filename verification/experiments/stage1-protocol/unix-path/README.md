# Unix PathBuf 的完整原始字节表示

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

本包从 integration 精确提交 `ae8e5a564ab3d81652a23acb2b99e242193fa3a2` 开发，
消去 ReadyCandidate 载体提案中的 PathBuf 外部类型模型。固定 Linux x86_64 视图直接
透明抽取原 PathBuf→OsString→Unix Buf→Vec<u8> 类型层级，保留三个 inner 字段；
库源和生产 Rust 均不修改。独立路径 crate 只是原标准方法的转发，六个生成函数全部
透明，外部函数和外部类型为零，未添加路径操作假设或手改生成体。

## 原字节与完整值

原 PathBuf::into_os_string、From<OsString> 及 Unix OsStringExt::from_vec/into_vec
给出任意有限可表示字节的双向往返和全值表示单射。这里的值相等指生成结构的完整
原始字节表示，不是 Path::eq 或文件系统对象相同性。正常见证保留空值、NUL、
无效 UTF-8 和所有原字节；遗漏首字节的一对值具有相同后缀，但完整原值及实际输出
不同。没有以这些见证代替任意值证明，也不推导路径可打开、安全、已授权或资源存在。

两版固定 Rust 标准库的四份原文均核对内容。机器证明对应固定抽取前端的实际生成体，
继续沿用原工具/普通 Vec 值语义的边界；源审查不冒充生产编译器或整个标准库的证明。
没有 Windows、IO、规范化、容量/分配行为或析构/崩溃结论。

## 接回 ReadyCandidate

正确和既有 credential 遗漏版从当前真实 snapshot.rs 重新编译抽取，追加相同类型
include。新生成体保留原八个候选字段及完整嵌套值，PathBuf 不再是外部类型。
ReadyPathConnection 以完整字节建立两份实际生成类型之间的双向表示，连接原标准
构造和投影。can_probe 的原完整规格、一般证明、正常值与错误反例正文直接复用
已发布的 `5a571e5a720dd5488423d8bf4b490ebc078bd369`，不另写协议模型。
错误视图只在私有 Rust 副本禁用原 credential 守卫，仍反驳同一 FullProbeContract；
不导入正确探测对应。两版各自重查未改的路径库证明。

**BTreeMap 的当前视图采用仍未落实。** ReadyTypesExternal.candidate 只保留原
Storage 键值条目载体提案，属于主控/V0 的待审条件；本包只消去 PathBuf 提案。
can_probe 也不提供整体 Ready、凭据有效、当前来源、已持久接受或实际 VM/文件事实。

## 独立复现与采用

公开 development.py 复制完整当前源码、记录执行位，核对两版库源和固定二进制，
重新编译/抽取并逐字核对生成输出，再用原 actual_files 检查器执行实际导入闭包的
内核、完整非陈旧同服务诊断和精确审计。每个输出有独立源码、target 和 .lake/build，
使用同一既有共享重任务锁。原始失败保留，不减少检查或修改内核选项。

路径模式不需要其它证明分支。两个 Ready 模式另外从本地同仓库 Git 对象读取上述
固定提交的数学模板，逐项核对 inventory 中的内容摘要；不写入或切换该旧分支。
若干净克隆缺少该对象，先按原 Git 权限取得精确提交，再执行证明入口。

```sh
git fetch origin 5a571e5a720dd5488423d8bf4b490ebc078bd369
python3 verification/experiments/stage1-protocol/unix-path/development.py --variant path --output /private/proof/unix-path --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/unix-path/development.py --variant correct --output /private/proof/ready-path --lock /private/proof/shared-proof.lock
python3 verification/experiments/stage1-protocol/unix-path/development.py --variant ignore-credential --output /private/proof/ready-path-wrong --lock /private/proof/shared-proof.lock
```

该入口签发独立开发结果，不签发 native/formal seal。公共 Lake、模块/原生登记、
根导入、冻结及覆盖候选由主控统一采用，不能跑旧入口报告本包已验收。
普通/正确/错误/恢复原生仍交 ning，不追加当前固定验收窗口或新开迁移 PR。
实际 JSON/Serde 字节、SQL/SHA、事务未知/重启及完整 V3 保留原义务。
