# 标准 Save 摘要及原连接载体的委托

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md) ·
[父摘要切片](../standard-digest/README.md)

本包固定 #753 的 f7164094，只继续未发布增量。生产代码只读；私有候选复用父包的
同一个纯 sha256_bytes 命名接缝，将原 StandardInboxSave::digest 的两库调用转调至它。
StandardConfigRead 的原 Save 委托保留透明生成体，没有另造协议/保存/哈希算法。

两个实际接收者均保留完整原连接载体。可逆值表示将它们接回父包的同一
FullDigestContract；FullSaveDigestContract 要求两个真实路径分别满足该完整合同，
不是只比较某个前缀、某个字段或有限输入。正常输入域仍为完整切片长度小于2^61，
全部32字节及整个接收者后态都进入结论。实际字段、完整值表示及库模型均须随正错版保持。

原方法的实际完整 MIR 单独捕获，接收者精确为 StandardInboxSave，原 self 不参与两次
库调用。绑定检查复用父包的完整接口/数据转移策略，仅加强实际接收者/签名选择；
固定 Sha256 类型、所有字典、完整输入及32字节返回分别核对。这是既有 A9/A6/B4 的
来源准入，不称原 FixedOutput 泛型后端或密码库实现已经机器精化。

输入截空、非空第32字节清零，以及在真实委托处截空分别作为真实 Rust 错误版。
共同空输入合法见证及同一 abc 完整摘要保留；错误环境不安装或导入正确 Core/Witnesses。
编译、抽取、导入、超时或服务失败不是合同反例。已证明的纯字节数学、完整 payload
和任意 Array32 的覆盖观察直接复用，不用正反版本不同的模型或条件掩盖遗漏。

formal.py 重捕获原方法、原纯叶子、泛型 Save/Graphic 及四份实际生成视图，核对完整
IR 值与原生成体，并检查全部保留根、实际文件、同服务诊断、依赖及结束输入。
--prepare-only 始终 complete=false，局部根构建不代替完整入口。ning 的 native.py
另保留 original/actual/三错/restored 六视图、同五条完整双路径摘要断言、新编译/链接、
原字节恢复、格式与严格 Clippy；该准备入口不表示已经验收。
公共配置/根/冻结/累计审计/依赖/覆盖交主控单写，提供候选差异，不重复共享整套。

调用方复用原五条正常摘要域数学及原 save_refines，不为正确/错误版更换性质。新增
完全泛型 save_using 连接，分别将实际 direct/delegated digest 接回同一 Save，保留
全部编码/update 结果和回调状态；合法配置预算和实际长度检查提供正常 SHA 输入域。
原标准编码/SQL字段、完整标准实例、原 JSON/TLS、
物理同事务/lease/连接历史、未知提交/重启、认证、producer/driver、Resume、真实资源
和完整 V3 保持各自原义务；值相等不推出授权、资源存在或耐久接受。V7、V1/V2、
V4/V5、V6 的原提供者不并入纯 SHA 库条件。18项要求/49项义务不因本包缩减。


运行前复用私有交接中的固定环境、共享重锁及四核预算。证明机运行本包 formal.py，
参数为 --output 指向新的私有目录、--lock 指向原共享锁；ning 对 native.py 使用
相同两个参数。只有已持有同一协调锁的父入口可传 --lock-fd。工具与服务沿工作流，
原生结果和普通功能封口分别取得；不在证明机补另一套普通测试环境。
公共采用顺序、完整入口与覆盖候选见 [REGISTRATION.md](REGISTRATION.md)。
