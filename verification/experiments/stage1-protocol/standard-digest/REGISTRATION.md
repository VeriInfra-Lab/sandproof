# 公共采用候选

本增量堆叠 #752 的精确 ef9aaf82，不重带 #751 的12个 Create 包。当前生产、公共
配置、依赖和覆盖均未修改。source-candidate.patch 仅供审查原 SHA 两次调用的最小
命名；它不包含薄证明根，不自动采用。固定 A9/A6/B4 的源/库条件见组件 README。

public-registration-candidate.patch、dependencies-candidate.patch、coverage-candidate.patch
分别在 #752 对应候选补丁之后应用，由主控单写；三份串联已在隔离副本核对。
不能只更新全局摘要或跑旧根报告新组件通过。原 verify-stage1-protocol.sh 的 full
任务须包含本组件全部正确/错误视图、原 native 输入/日志和实际文件审计。

新增10份含审计模板、45项声明：25项字段/表示/连接/见证/反例，加20项原样复用的
字节数学；另两份证明模板没有审计，不隐藏排除。累计候选448份/1845项，依次接在 #752
的438/1800之后；此计数已按独立候选 Git 索引运行原 audit_templates 核对。固定
create-delivery 仍是12包/42视图，原范围不变。数量不是独立业务性质或验收结论。

formal.py 从完整生产输入重新捕获原标准方法及候选库叶子的实际 MIR，分别核对完整
具体类型、字典和数据流；并重抽取父组件的独立 Read/Load/Graphic 视图及三份 SHA
生成体。原生成 Lean/translation 逐字节比较；完整 IR 值仅沿父组件规则排序唯一键的
short_names 名称表，其余顺序/正文不变。原式的后端失败独立保留，没有采用失败的
字典桩或 trait 排除。源绑定不改称一般旧新内核精化或库符合性证明。

三份正确/错误实际文件都通过原内核、规定同服务完整非 stale/partial 诊断和依赖
审计才可 complete；最终再核对源/模式、全部原 IR/生成体/日志、固定工具与库。
registration.py 的结构准入不代替原 registered.py 的物理文件/原日志读回。
--prepare-only 始终 complete=false，源准备和根构建不当作全文件验收。

ning 使用 native.py 的 original/actual/input-empty/nonempty-tail-zero/restored 五视图，
共用同五条完整摘要断言；四个非空断言须在两错实际编译后全部失败，空输入仍通过。
恢复版先实际写入错误源再恢复原候选字节。保留新编译协议库/service 测试程序的
源/manifest/链接、固定 Cargo 库、完整失败集、格式及严格 Clippy，无 lint 豁免。
该入口不运行 Lean，不代替完整 Linux functional。普通封口交 ning，proof_native_suite
和 verify-controller.sh 由主控同批统一验收及内容复用，本包不重复完整共享套件。

所有源/有限库采用和共享接线仍待主控/V0协调。V5/A4 的活对象/租约/事务/未知提交/
恢复、V7 认证来源、V1/V2 当前意图/耐久发送、V4/V5 本地结果和 V6 实际材料的原责任
保留；其余标准 IO、完整 JSON/TLS、producer/driver、Resume 与完整 V3 继续开放。
