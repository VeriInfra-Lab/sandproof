# Create deadline 的精确整数转换

[组件入口](../README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../docs/formal/dependencies.md) · [证明工作流](../../../../docs/proof-workflow.md)

唯一真实 `attach_create` 在原 UPDATE 参数位置调用纯 `create_attachment_deadline`。
helper 只承接原 `i64::try_from(...).map_err(|_| UnsupportedInput)` 表达式，不移动
receiver、第二项身份参数、SQL 或 save，不采用较大的效果接口候选。

## 完整值合同和边界

原 Std `TryFrom<u64> for i64` 保留透明生成体：任意输入接受当且仅当不超过
`i64::MAX`，成功保持数学值，溢出精确返回 `PosOverflow`。真实 V3 helper 的
`FullDeadlineContract` 保持同一成功值，并将且仅将该溢出映为 `UnsupportedInput`。
正常零值、最大合法值、首个溢出及 u64 上限都有见证；一般证明不依赖有限枚举。
这些是纯数值见证，不能推出完整 Create 已接受、SQL 成功或耐久保存。

保留原完整 JournalError，以及实际单字段 TryFromIntError 包含的六变体
IntErrorKind。固定全局后端把 TryFromIntError 表示为 Unit，本视图只用局部别名
连接原透明生成的完整错误枚举，不复制枚举、改生成体、工具或全局模型。
标准 `map_err` 保留透明 LLBC，Aeneas 仍选择原固定 Result builtin；与此前
Accepted 校验相同。原 FnOnce 错误闭包保留透明生成体；库模型的源码一致性属于
固定 A6/工具链条件，不声称已证明整个标准库或任意有副作用的 Drop。

Request 的四个既有值根与本操作共享生成视图，完整 ContractError 从原始枚举
核对到已有 Foundations 载体；不会以类型别名认证任何消息。输入从已解码值开始，
不覆盖 JSON/TLS、独立认证来源或完整收件链。

## 正反视图和复现

`unchecked-cast` 将实际 helper 改为 `Ok(value as i64)`，在首个溢出实际接受
错误数值；`wrong-error` 只把原映射改为 UnsupportedStore。两者各自重新编译和
抽取，并反驳与正确版逐字相同的 FullDeadlineContract。它们只导入仍适用的原 Std
证明及共同规格，不导入正确 helper 对应。编译、导入或超时不算性质拒绝。

```sh
# 证明环境：沿用既有固定工具、包缓存、原共享锁和资源门槛。
python3 verification/experiments/stage1-protocol/create-deadline/formal.py \
  --output /private/proof/create-deadline --lock /private/proof/shared-proof.lock
# 独立普通测试环境：实际、两错、恢复四视图同三项断言，保留新编译链接与精确失败集合。
python3 verification/experiments/stage1-protocol/create-deadline/native.py \
  --output /private/acceptance/create-deadline-native
```

两个入口均是明确范围的独立开发入口。形式入口逐项核对完整源码与执行位、固定库源、
真实根、原枚举、未编辑的生成体及全部本地 Lean 导入；使用原实际文件检查器做内核、
完整非 stale/partial 同服务诊断和依赖审计。每视图独立源码、target 和 .lake/build，
不清缓存。普通入口不在证明机执行，不签发共享 native seal。

公共 Lake、任务/原生登记、总根、冻结和覆盖由主控单写；新增目标须实际进入完整检查，
不能只跑旧根宣称采用后通过。原私有候选结果保留旧执行身份；当前入口的实际检查状态
以绑定精确输入的原结果为准。更改调用文件后，旧模块整文件输入会拒绝入场，须由主控
分别核对仍适用证明和当前编译连接，不能直接刷新旧 hash。

完整父函数的惰性 World 读取、完整输入/检查/效果顺序、两输入十进制 ID、两个 SQL
参数和同事务/save、提交未知/重启，以及 Resume/driver/真实字节与资源事实继续另列。
原寿命 UPDATE 不检查成功行数；数值合同不补上恰好一行或持久接受结论。
