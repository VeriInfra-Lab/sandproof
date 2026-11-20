# Create 实际固定编号格式

本包从当前 `controller::identity::sandbox_id` 的原已解码 namespace 和核心 sandbox
编号开始。透明根只按原顺序调用一次真实函数；完整原 ContractError、JournalError、
标识符校验、构造和错误转换由固定工具生成，不修改生成体。

`FullIdentityContract` 对任意完整 namespace 字节表示和任意 u64，要求真实生成体
精确返回 `sb-{namespace}-{u64 十进制}`，或按原完整标识符校验返回
`JournalError::Contract(InvalidIdentifier)`。保留全部字节、128 字节界限及原 ASCII
规则；空 namespace 本身允许。只使用已有 `StringRepresentable` 的正常分配／载体范围。
正常见证和两个字段遗漏错误版使用同一合同、同一具体正常输入。

公共格式类型、惰性两个参数回调、String writer、u64 十进制、字符串和标识符数学直接
复用 Foundations。新增有限适配只解释当前九字节、两个默认 Display 参数的固定程序，
没有给 Unit 填常量。固定 Std 的该使用点正常返回一致性作为具名依赖条件：不声称
Std 格式实现本身已机器证明、任意自定义格式器正确或内存分配总成功。普通 `must_use`
从固定 Std 透明抽取，不另设外部函数假设。原 Unit 模型的不足证据继续保留。

两个真实 Rust 变异分别忽略非空 namespace 和将编号除以十。它们必须实际编译、抽取，
到达具体错误结果并否定同一 `FullIdentityContract`；错误视图不导入正确 ID 对应证明。
导入、编译、服务失败或超时都不算业务反例。

```sh
# 证明环境：现有共享重锁、固定工具、独立输出和 .lake/build。
taskset -c 0-3 python3 verification/experiments/stage1-protocol/create-fixed-identity/formal.py \
  --output /private/proof/current-fixed-identity --lock /private/proof/existing-heavy.lock
# 独立普通测试环境：同一源码及固定普通工具；四视图同五项断言。
python3 verification/experiments/stage1-protocol/create-fixed-identity/native.py \
  --output /private/acceptance/current-fixed-identity
```

`--prepare-only` 只编译、抽取和构建，不返回完整形式通过。完整入口检查每份真实 Lean
文件的内核、规定同服务完整非 stale/partial 诊断及依赖。公共 Lake、根、native、输入、
依赖和覆盖登记由唯一维护者接入；本包独立开发结果不替代普通回执或共享全套验收。

实际 Create 的标准实例调用、同 transaction／connection、SQL 参数、JSON／SHA、提交
未知／重启和恢复编号仍须各自连接。本编号值证明不推出授权、资源存在、耐久接受、
编号全局唯一或完整 V3。当前协议允许 Resume 编号空洞的合同不变。
