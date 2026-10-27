# 收件重放与新会话记录候选


当前接线候选采用统一工程提交 `0c8f457a` 的真实生产函数；清单中的 a9c 与旧候选补丁保留为历史来源，不再把该补丁应用到业务实现。验证副本只增加单次调用薄根，错误版只修改目标守卫或字段。当前输入已登记，**重新抽取、原生正反例和 Lean 检查尚待完成**，不能据此声称整套 V3 通过。

实际返回错误的根同时核对完整 `JournalError`，包括新增的 `CaptureMaterialQuotaExceeded`；纯值查询不强迫引入无关错误类型。原完整字段、首次拒绝、短路、负例和实际文件诊断继续保留。
[V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [证明工作流](../../../../docs/proof-workflow.md) · [父组件](../README.md)

固定生产输入为 `a9c9967889c4cf298e4378c7106948018835fc64`，堆叠父项 #711。
本包从已解码、按实际生成类型表示的 Inbox 和 AcceptedLifecycleIntent 开始。
`record` 本身不执行认证或封套校验；两者由父切片的真实认证调用证明连接，不能因记录成功
就推断已经授权、已经持久提交或实际资源存在。

## 本切片合同

- 任意保留请求：完整 key 查找后的完整 intent 重放返回原索引且整个收件箱不变；同 key
  不同 intent 返回 Conflict 且整个收件箱不变。复用 #710 的完整任意长度查找证明。
- 任意 `record_fresh` 输入：退休 session 或完整 key、slot 范围、首个匹配 lane、当前
  retention window、会话容量与 slot 占用、操作形状/选项长度按原顺序决策。所有拒绝
  保持整个收件箱；成功追加完整 Session 和完整 Entry，除 sessions 外所有字段保持。
- 实际 `record` 组合：在不存在保留 key、也不存在相同 session ID 的显式条件下，
  对应同一个 FreshPost 与 freshDecision。该条件只描述容器内容，不提供业务授权。
- Create 和新会话 Destroy 的完整合法见证；retention window 不同但协议封套仍合法的
  拒绝见证。Destroy 写入 terminal tombstone，local/incarnation 等原始空字段保持原语义。

两个真实 Rust 错误版分别删除 fresh 路径的 retention window 守卫、把新 Entry 的
accepted_version 改成 0。正反视图共用完全相同的模型、合法见证和完整合同；错误视图不
导入正确版定理。原生断言比较完整状态，形式反例否定 FullFreshContract 及
FullNewSessionContract，不能只检查返回成功或有限字段。

## 原源码与候选的对应边界

固定工具不能直接翻译当前 iterator/find 的高阶生命周期与 singleton Vec 初始化。
`production-candidate.patch` 是交主控审查的私有候选，生产文件未修改：复用 #710 的
find_entry 拆分；将位置/any 查询改为显式迭代；last_entry 以空判定和末索引表示；单元素
entries 用 with_capacity/push 建立。所有业务判断、拒绝顺序和完整字段保持。
完整候选已包含父切片拆分，主控采用时需与父补丁合并去重。
重构后的选择/短路循环及克隆由一般定理验证，原版与候选用相同原生状态断言对照。
这不是原版与候选的机器检验语义等价证明；采用前必须由主控核对并重新冻结。

驱动先保存原生产 Charon 抽取并核对完整 ContractError enum，再在隔离副本应用候选及
唯一定位的真实变异，重新抽取全部四个协议薄根与实际 record 调用。Types/Funs 原样生成。
保留失败预检，不以编译失败、超时或导入失败充当性质拒绝。

## 依赖及未完成范围

复用 dependencies.md 登记的 String 全文比较/正常返回克隆、Vec、Option、实际比较字典
和固定算术模型；具体自有结构的 Clone 全字段保持由本包证明。ContentDigest 只复制
32 字节值，不证明曾计算哈希或材料完整。纯容器分配模型不涵盖 OOM/abort、地址或物理容量。

已有 session 的 record_existing 追加尚未证明；透明抽取该函数不等于已证。
#687 的任意已解码 Manifest 候选按其精确适用范围保留，本包不重复该证明；材料真实性另由 V6 提供。
该路径真实 Resume 使用 checked_next，而协议请求 validate 允许编号空洞，两者必须分别
陈述，不能加强公共协议规格。SQLite 编解码/写入事务、未知提交与恢复、GLOBAL→LOCAL
映射、JSON/TLS/异步接线以及完整 V3 仍是后续义务。
V7 提供独立认证上下文与配置来源；V1/V2 提供当前意图和耐久发送；V4/V5 提供实际本地
接受/结果与调用次序；V6 提供真实文件/VM 事实。没有把这些条件塞入库模型或整块事务假设。

## 重现与交接

外层须取得共享重锁，输出目录不得预存；每版隔离源码、Cargo target 与 .lake/build，
只共享只读依赖。native.py 检查原版、候选、两个错误版和恢复版；formal.py 对 correct、
skip-retention-window、wrong-recorded-version 分别重抽取并调用原正式检查器检查整个实际
文件闭包及所有审计。完整输入、工具、库和服务内容在前后绑定，禁止 stale/partial。

本机结果仅为开发证据，ning 负责普通/原生正式回执。共享配置、公共登记、覆盖及 V0
依赖采用通过 coordinator-candidate.patch 交主控，不能只跑旧根声称新版通过。
完整 verify-controller.sh 与共享 proof_native_suite 由主控同批复用，不在此重复全套。

开发重现（以下命令外层持有共享重锁）：

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record/native.py --output /private/record-native
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record/formal.py --development-native /private/record-native/result.json --variant correct --output /private/record-correct
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record/formal.py --development-native /private/record-native/result.json --variant skip-retention-window --output /private/record-skip-window
bash scripts/python-entry.sh verification/experiments/stage1-protocol/inbox-record/formal.py --development-native /private/record-native/result.json --variant wrong-recorded-version --output /private/record-wrong-version
```

`--development-native` 只绑定同一完整输入闭包和实际源码，不签发验收 seal。
正式 `--require-native` 要求当前共享 seal、functional 和 stage1-inbox-record 登记回执，
再逐字核对原版、候选/恢复版或对应错误版及唯一变异；两种模式不能混用。
