# V3：原继续请求的网络策略传递

[父完整绑定](../continue-work-binding/README.md) ·
[工作流](../../../../../docs/proof-workflow.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

固定父#1024作者3dd4c8b6646a6ef73a1efc3052058957495f61c4、真实生产
b0d791ccc4d4e058b710dbf8f46766652b2853df。生产、父包及公共配置/根/登记/冻结/覆盖只读。
本包补父包明确保留的network_policy→DenyAll形式反例，复用原类型、Ports和CompleteContract；
不更换性质，不新增SHA语义，不把两项尚未采用的JSON/SHA使用点写成已采用。

## 实际连接与同一性质

父探针在辅助函数中返回常量策略引用时触发固定Aeneas限制；旧失败保留。
本包只把原策略借用留在实际调用者，再与self交给透明binding_input_with_policy。
其它五个原元组位置移动一次；没有复制算法、Clone、分配、额外守卫或Opaque业务方法。
正常调用仍传原self.network_policy，错误调用仍传原DenyAll。该辅助函数和整个调用者
由同一固定工具抽取，正常/错误辅助函数完整生成体相同，只有真实调用者策略参数不同。

复用父bindingInput、outcome、InputContract和CompleteContract，不另造协议模型。
suppliedInput/replacedInput分别表示实际生成调用者的两处方法调用；questionOfInput仅为
元组到同一Question的表示逆，不表示运行过构造工厂、已授权或已落盘。
实际调用分解定理将该输入接到整个原返回，正常/错误视图分别与各自未拆分入口一般等价。
正确版满足原完整六字段/整个返回合同。非DenyAll原请求使实际错误版违反同一个合同；
保留原DenyAll控制、合法IPv4完整值见证、任意其它字段及任意接口结果。
反例只使用完整输入的策略差异，不由输入不同推出摘要不同，也不用假哈希。
wrong_result解释真实错误输出，不是把规范改成期待错误值。

编码首错及全部成功字节传递沿用父完整结果证明。端口函数等式不单独表示物理调用轨迹、
时间、真实材料、权限或持久接受。仍从已解码值开始；不称完整JSON/TLS或系统闭合。

## 完整条件开发入口

固定工具/目标/Cargo锁沿用父包，生成体原样导入，输出目录须为新目录并使用共享重锁。
以下命令相对本目录；使用工作流固定Python与formal角色，实际私有位置只留本地。

```sh
python3 capture.py --view shared --variant original --output /private/shared-original --tools /private/fixed-tools --lock /private/heavy.lock
python3 capture.py --view shared --variant network-policy-as-deny-all --output /private/shared-wrong --tools /private/fixed-tools --lock /private/heavy.lock
python3 capture.py --view inline --variant network-policy-as-deny-all --output /private/inline-wrong --tools /private/fixed-tools --lock /private/heavy.lock
```

三套实际工具完成后仍由父admit明确拒绝未采用的两个具体JSON/SHA使用点。
旧29准入不放宽，31个候选映射不是正式采用。捕获、编译、导入、超时或工具失败不算性质反例。
captures JSON必须只含shared/original、shared/network-policy-as-deny-all、
inline/network-policy-as-deny-all三套原始目录。检查输入JSON必须且只含captures、reads、
reads_audit；reads/audit必须是父完整220文件/1222审计的原始工程与服务记录。

```sh
SANDPROOF_ACCEPTANCE_ROLE=formal python3 check.py --inputs /private/inputs.json --output /private/checked --lock /private/heavy.lock --lake-packages /private/fixed-packages --lean-mcp /private/fixed-client
```

入口严格回读每套完整4906源码/模式、原工具/日志/IR/全部生成文件，保持117个类型及
191个完整父函数，实际共享视图193函数、未拆分192函数。保留父全部220文件，检查
全部225实际文件/1243唯一审计（21项新增）、十个固定依赖、同服务完整诊断及新目录逐字复现。
清单/命令存在及局部候选通过不能替代此入口。check.py成功也只表示完整条件开发闭包，
interface_adoption_complete/public_acceptance仍为false。

## ning与主控

native.py只补两套新共享借用形态的源，沿用父相同63断言和实际JSON/SHA。
正常视图预期63/63；原网络错误视图预期62通过/1失败，唯一失败必须到达
`continuation binding must include the complete supplied network policy`。
新增错误视图后独立恢复original源码/target，重编译真实业务库与libtest，再全部63通过；
核对同crate的manifest/src/fresh，不只换测试程序。源码准备/格式化不是编译、测试或恢复。
父八视图/五次恢复继续由ning/主控协调内容复用；本包新增两视图/一次恢复，不能重复宣称父已验收。

两项具体encode_continue_work_binding/digest_continue_work_binding的A6/A9/B4采用仍归主控，
正常借用、普通Vec/Global释放及实际编码/哈希完成域沿用父条件。没有新外部库接口或整业务信任。
公共登记、覆盖仅交registration-candidate.json/coverage-candidate.patch；普通完整入口和共享
proof_native_suite/verify-controller/verify-stage1-protocol由ning/主控统一验收，PR保持draft，不自行合并。
同一World/SQL/父事务、当前意图/权限、耐久提交、真实宿主收件/执行及V6/V7事实仍由原提供者承担。
原始证据位置和报告哈希只保留本地，不追加公开issue/PR。
