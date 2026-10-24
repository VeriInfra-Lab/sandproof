# V7 生命周期请求、鉴权与操作查询

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明覆盖](../proof-coverage.md)

任务：[#639](https://github.com/VeriInfra-Lab/sandproof/issues/639)。本页定义第一阶段的实现与证明义务；实际版本及已验证范围以[覆盖记录](../proof-coverage.md)为准。本页的性质是验收义务，已有结果按覆盖记录复用，不自动成为新生命周期的已证结论。

## 1. 输入输出与范围

对应 [sandbox-api](../../sandbox-api/src/lib.rs) 与 [sandbox-center-service](../../sandbox-center-service/src/lib.rs) 的正式中心入口。本阶段重点是 create/unload/resume/destroy、请求窗口和授权结果查询。完整 get/list 投影、分页与所有兼容端点后置；原功能/检查不删除。

公开路径/schema 和键规则以[模块合同](../module-contracts.md#身份请求和结果)为准。客户端第一次发操作前已有键，owner 从鉴权派生，规范化输入含种类、完整参数、会话和期望版本；不能事后生成内部 ID 冒充客户端可重试身份。

当前正式路由和中心端口已支持请求窗口、create、unload、destroy、query；Resume 路由和业务提交尚无成功实现。可以先证明真实 `secret_equal`、认证 owner 来源及操作键构造；HeaderMap、JSON 与响应接线必须逐项连接已登记合同，不能把整个 handler 信任。字节相等定理不包含常数时间或侧信道保证。当前查询偶发 500 由 #676 跟踪，不因正常查询样例成功而消失。

## 2. 待证义务

- 字节→解析→规范化→鉴权→实际 V1 调用的参数对应；限额/首错、字段缺失/非法/冲突和无权限明确区分。自有解码逻辑不整体可信。
- 有效旧键全文相同先返回原结果；不能因当前 phase 改变拒绝合法重放，也不能在状态检查前把未授权历史泄露给别人。
- 仅不存在的新键检查窗口有效性、序号范围、版本/阶段和资源；耗尽/过期拒绝而不换键。结果退休仍能通过持久窗口前沿拒绝迟到旧请求。
- 接受、进行中、成功、可恢复失败、StateLost 和未知有对应编码；HTTP 超时不声明无效果。客户端可用原键查询，后台继续推进。
- 返回 Unloaded/Running/Destroyed 的每个字段来自已保存的匹配事实；结果记录不是未来访问令牌。总寿命与原创建起点绑定，不因重放或恢复重置。
- 正常支持域能通过，不以“一律拒绝 unload/resume”满足安全；旧无键 SDK create 不被描述为具有新语义。

## 3. 连接与验收

V0 提供实际 JSON/字节/Writer，V1 提供决定，V2 提供接受与结果持久性，V8 提供新的运行代访问。抽取真实 normalize/auth 决定、参数构造及响应分类，并连接原 handler/Writer；新增 wrapper 不得漏掉生产前后检查。

验收同键并发/冲突/迟到、结果未知、跨用户查询、过期窗口、寿命边界和非法完整请求。原 SDK 兼容检查与新明确操作接口分别报告；必要 SDK 扩展/适配是产品接口工作，不能用验证专用入口代替。负责 LC-I1 与 LC-P1/2/8 的公开部分。
