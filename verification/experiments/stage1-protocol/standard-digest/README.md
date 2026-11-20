# 标准 SHA 字段与原 Load 输入域

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) · [工作流](../../../../docs/proof-workflow.md)

本组件将 #753 的数学与实际调用证明接到本批采用的唯一生产函数
`lifecycle_inbox::sha256::digest`。原 `Sha256::digest(bytes).into()` 两次库调用原样保留，
读取和保存方法委托该函数；正确私有视图不再新增 codec helper 或替换业务正文。
源码采用不等于本批验证通过；当前生成物须由固定工具实际重捕获，不能只换摘要。

原式的方法和薄根已由固定工具透明捕获；原泛型 FixedOutput 辅助字典有两个不同
BlockSize 类型参数，生成体无法通过内核。单独排除 IsLess 的库类型试验也在固定
Aeneas 预处理失败。全部原始成功捕获/失败保留；这些不是性质反例，不改生成体、
不放宽内核、不把原式标为已机器精化。正式候选不使用该排除或失败的字典模型。

唯一生产库叶子的完整 MIR 按既有固定 SHA/GenericArray/AsRef 接口核对；调用方保持独立透明生成体：
同一个完整只读切片、同一具体 Sha256 类型和全部字典、两次原库调用、完整32字节
转换返回，没有分支、对象读取、额外调用或字段投影。该 B4 来源检查不等于库实现或
一般旧新机器等价证明。本批已采用有限 A9/A6/B4 条件，库符合性仍是明确前提，当前验收由统筹另行记录。

`FullDigestContract` 对任意完整正常切片（长度小于2^61）、全部32字节和整个原接收者
陈述候选标准方法；正确和两错必须复用这一合同、库模型及空输入正常见证。
正常值模型不承诺真实库在域外 panic。输入/完整摘要的已检查纯字节数学原样复用；
不是重新证明密码库，也不假定哈希单射、材料真实或已耐久接受。

完整 Context 的双向表示连接 #752 的 NormalContext，复用原 FullLoad 和实际标准
BLOB 读取的条件值结论，消去原正常摘要 agrees 参数。其余 IO、完整解码字符串表示、
Graphic、同一物理对象/事务、两读一致、提交未知/重启及真实 JSON 链继续显式保留。
这里只连接具体字段，不称整个标准 IO、网络外壳或完整 V3 已完成。

`input-empty` 将完整输入改为原输入的空前缀；`nonempty-tail-zero` 对非空输入覆写
摘要的第32字节。两错都保留空输入合法见证，并以同一 abc 完整摘要合同给出反例；
不把编译/抽取/服务失败或超时当性质拒绝。错误环境不安装正确 Core/Bridge/Witnesses。

[来源清单](reuse-provenance.json)固定原样复用的数学/生成数据；[源候选](candidate.json)、
[完整库接口](fixed-interfaces.json)及 generated-index.json 分别固定真实来源和未编辑的
工具输出。组件 inventory 在当前语义/抽取/证明核对后登记，不能以更新 hash 迁记旧结果。
formal.py 重新捕获当前源和独立提供者，再经原检查器检查全部实际文件。输入、模式、
IR/生成目录、原日志、固定工具/库在结束时再次核对；局部结果仍保留其原执行身份。

```sh
taskset -c 0-3 python3 verification/experiments/stage1-protocol/standard-digest/formal.py \
  --output artifacts/standard-digest-formal --lock /path/to/existing-proof-heavy.lock
```

`--prepare-only` 仅捕获和构建，始终 complete=false。继承主控已持有的协调锁时，传
原 --lock-fd，核对同一设备/inode并复制描述，不重新打开等待父锁。每任务至多四核，
独立 .lake/build，保留现役缓存及全部成功/失败。固定工具与同服务客户端不更换。

ning 的原生入口使用当前原文/实际采用/两错/恢复共五视图、同五条完整摘要断言；核对实际
新编译源/manifest/链接、完整失败集合及格式/严格 Clippy，不以 lint 失败冒充反例。

```sh
bash scripts/python-entry.sh verification/experiments/stage1-protocol/standard-digest/native.py \
  --output artifacts/standard-digest-native --lock /path/to/existing-proof-heavy.lock
```

精确执行结果见 #635/本切片 PR；ning 普通/原生、公共共享入口和系统组合分别取得。
[公共登记候选](REGISTRATION.md)只供主控单写审查，本包没有直接改公共配置或覆盖。
#751 的12个 Create 包保持原范围；18项要求/49项义务和完整 V3 不因本字段切片缩减。
