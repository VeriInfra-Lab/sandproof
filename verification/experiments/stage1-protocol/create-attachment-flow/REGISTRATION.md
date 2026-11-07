# 主控公共登记候选

[组件合同与范围](README.md) · [V3](../../../../docs/formal/03-protocol-inbox.md)

这些文件供主控单写采用，当前没有修改公共登记、冻结或覆盖：补丁使用零上下文，采用前核对所列基线并用 `git apply --unidiff-zero --check` 验证；最终公共组合仍由主控审查。

- [登记差异](public-registration-candidate.patch)：新增原生 case、完整三视图 formal job、严格结果形状和同一外层锁的 fd 传递。主控须在实际完整进程链传递该 fd，并核对该处来源；候选不能代替实际入口验收。
- [依赖差异](dependencies-candidate.patch)：现有条件及标准实例的未完成连接。
- [覆盖差异](coverage-candidate.patch)：当前实际 Rust 的完整局部正反结果，保留原执行身份与未完成项。
- [审计与执行清单](registration-candidate.json)：精确 41 个新增名称、重用边界和实际／两错／恢复的原生断言要求。

原完整三视图在实际 Rust／数学模板采用后完成；后续元数据和文档变化只做了继承锁的编译／抽取／构建预检，并核对全部实际 Lean 文件与此前正确视图相同。没有把 prepare-only、原源码执行或源回读冒充新入口的整套验收。全局审计数量必须在合并全部候选后重新计算，原 18 条路径数学及 `loaded_representable` 不重复加数。

普通／原生、主控原始证据接收、公共 `verify-stage1-protocol.sh`／`verify-controller.sh` 及完整 V3 继续单列。原已解码 Manifest 和同步 FullFlow 保留原适用范围；真实字节、标准实例、同事务、提交未知、恢复和系统结论未因本候选完成。
