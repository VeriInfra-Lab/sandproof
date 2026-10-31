# Create 完整同步流程

[文档目录](../../../../docs/README.md) · [V3 范围](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md) · [实际覆盖](../../../../docs/proof-coverage.md)

本包连接唯一实际 `attach_create_using`：原完整 load、全文选择、已有 owner 守卫、缺失时的 owned World 读取、新 owner 守卫、完整安装、deadline、固定 ID 的两个输入、原寿命 UPDATE 和完整 save。正确视图只追加透明根；不复制 load/save 算法，不以整个 codec 或事务作为可信回调。

## 完整合同与输入

`FullCreateFlowContract` 对所有实际编码字典、完整 `CreateAttachment`、namespace 和效果状态陈述实际生成体与同一完整规格的等式。规格复用已经对应的纯选择、owner、安装和 deadline，并调用独立的 `FullLoad`／`FullSave` 规格。所有首错、短路、提前返回和中间效果状态保留。

一般性前提仅为已有的 `FullGraphicContract` 和 `DecodedStringsRepresentable`。save 所需的配置表示条件由同一次成功 load 和完整安装框架推出；不假设安装后整个 Inbox 必定合法或 save 必定成功。

Create 的三个原字段和完整模板保留。PathBuf 的三个 inner 和完整原始字节 Vec 透明生成；路径桥接复用原 Std 值证明，不解释 UTF-8、规范化或实际文件。环境变量字段连接已有获批 Storage 完整键值载体，不新增没有使用的 BTreeMap 操作保证。完整六个类型／字典参数和三个运行时参数按原顺序检查。

正常见证使用显式合成效果，实际值校验器接受安装前、后同一组 Inbox；完整 load、选择、owner、安装和 save 都连接到真实生成的父函数。成功寿命 UPDATE 的任意行数被忽略，包括零；最后的收件 save 仍独立要求更新一行。完整模板值保留不表示真实注册、授权、材料、资源或 admission 已被证明。

## 同合同错误版

- `skip-save` 只把真实父函数最后的 `save_using` 调用改为 `Ok(())`。相同完整输入缺失保存效果，违反原 `FullCreateFlowContract`。
- `eager-world` 把真实 owned World 读取移到已有 owner 守卫之前。相同已有映射输入多出 World 读取，违反同一完整合同的短路／效果顺序。

两版使用相同的完整模型、同一具体输入和同一正常规格，分别实际编译、抽取并到达反例。错误环境禁止导入正确父函数定理；仍适用的独立下层合同直接复用。编译、导入、超时、部分或陈旧结果均不算性质拒绝。

## 复现与交接

`formal.py` 的独立开发入口重新编译并用固定原 Charon/Aeneas 生成完整原枚举、bound Request 和独立 Graphic 视图；重新核对全部实际文件、内核、同服务完整诊断和依赖。只有整个本地导入闭包逐字节未变的文件才可复用完整旧结果，保留旧执行身份。

`native.py` 交独立普通测试环境执行正确、两错、恢复四份隔离源码，保留同六项断言、严格格式与 Clippy、新编译库和测试程序及精确失败集合。原生案例用原 SQLite／Serde／SHA 适配，另检查已有映射、缺失映射、首错、零寿命行、save 失败和 deadline 溢出的实际控制流；它们不代替原 WorldTransaction、提交未知、重启或真实系统测试。

公共 Lake／根导入、原生 case、冻结、总审计、依赖和覆盖由主控单写；本包提供精确候选，不修改共享登记。普通／原生仍在独立测试环境；正式共享入口只有收到相应完整原生封口后才可执行。局部形式检查不能替代公共整套验收。

## 未闭合的连接

标准实例仍需逐符号落实：原 RefCell 共享借用及完整 World clone、三个原位置的同一 live transaction／connection、保留 namespace 与 u64 十进制输入的固定 ID 及原校验／错误、deadline／ID 两参数 SQL，以及实际 JSON／SHA／读写结果。source 委托审查不冒充一般等价或库实现的机器证明。

本包从已解码值及明确效果接口开始，不声称完整 JSON/TLS 链、独立认证上下文、模板／资源事实、外层同事务插入、耐久接受、提交未知、恢复、Resume 或完整 V3 已完成。原 Manifest 和 record FullFlow 保持其原精确适用范围。
