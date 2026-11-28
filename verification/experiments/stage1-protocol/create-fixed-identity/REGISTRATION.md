# 固定编号值证明的公共登记候选

本包沿原 V3 分支交付，公共文件由唯一维护者采用；没有新开碎片 PR。本页及补丁不表示
原生、正式共享入口、完整 Create 标准实例或整个 V3 已验收。

`registration-candidate.patch` 基于本包 inventory.json 的源码基线中原 registered.py，
先应用 [Create 完整流程候选](../create-attachment-flow/public-registration-candidate.patch)
后的结果。两者都使用零行上下文，检查／采用用 `git apply --unidiff-zero --check` 和
`git apply --unidiff-zero`。实际目标文件的前置内容摘要见 registration-candidate.json。
不单独跳过原总审计、输入／冻结、原 native 组件和其它真实文件。

候选增加一个 native label 和一个包含正确／两错的完整形式任务，读取每份真实文件及
完整原服务／依赖证据，拒绝缺 case、prepare-only、局部或非反例结果。原共享外层锁
必须沿同一打开对象传递给子进程，不能在外层持锁时让子进程再开同一路径阻塞。

本包六份含审计的模板共 23 个唯一审计名；三视图保留各自的原始声明与精确闭包。
原 Foundations 的字节、字符串、默认十进制和标识符数学直接复用，不重复计数。
默认 str Display 和固定九字节／双参数 format 的有限正常返回合同由公共维护方统一
登记；现有单 u16 格式程序及公共模型不在这里改写。

native.py 给独立普通测试环境运行实际、忽略 namespace、截断编号、恢复实际四视图，
保留同五项断言、新编译 protocol／service 的实际源绑定及严格 Clippy。没有执行过
native 就不能出具完成回执。证明编译同时观察到现有 Create 内部 helper 的可见性告警；
本组件会收紧其内部可见性并核对受影响捕获，不能以关闭警告或变异预期掩盖普通版本。

dependencies-candidate.patch 和 coverage-candidate.patch 分别补精确使用点条件和当前值
对应范围。最终累计数量、根／Lake、输入和冻结由主控按当前候选重新计算并统一验收，
不把源码提交、局部检查或候选补丁解释为共享验收通过。原始证据的实际接收另记。
