# V3 首次卸载的原 World 与第二次 Inbox 读取

本包物化 `RefoundWorldParent` 的 Lean 导入闭包。基线是 #771
`4d5c1db07c8f1b91504c2c7eb9efe5183b0f2421`。保留 #770/#771 原来的模型、
生成体和证明。实际 Rust 对应、抽取与验收见 [证据清单](evidence.json)。

`proof-inputs/` 是原已检查的生成视图、证明和显式调用参数。`extraction/`
保留四个根的原始生成 Lean 及外部模板。七处组合视图变动限于已核对的
判别属性。物化时检查编辑前后摘要并逐字节恢复原件。Native 已通过原
参数化脚本及判别属性隔离。Inbox origin 视图在比对五个实际类型之后使用
固定导入和命名空间，恢复原导入及命名空间后与原生成函数逐字节相同。

`sources.json` 的 223 个模块中有 186 个引用基线源码，37 个来自本包。
`objects.json` 登记可核对的 46 个既有对象及对应物化源码，最终根始终由内核
和原审计重新检查。原对象不入 Git。对象目录由操作方按模块名整理，示例为
`SandproofUnloadStartupJoint/Types.olean`。提供的原对象必须与公开摘要一致，
其余对象由内核从物化源码构建。

在原形式验收锁和固定工具环境中执行：

```sh
python3 materialize.py --repository <正式源码目录> --output <新私有目录>
# 增量物化时另传 --object-root <已登记原对象目录>
# 在新目录的 .lake/packages 接入原固定 Lake 包目录
bash check-materialized.sh <新私有目录>
```

本批在原 msra 锁下复用已通过的父定理和严格依赖审计。ning 通过 186 项收件测试，
其中原 #771 夹具独立从 SQLite 读原始回执并核对真实首次卸载的输出。共享借用需要一条
有作用域的 Clippy 预期。最终 Rust 格式及严格 Clippy 在 ning 通过。msra 对此只重抽
`base_snapshot`，四份生成 Lean 中的区别只限该 Rust 文件的位置注释。原证明视图与
内核结果继续适用。其他原件保留在验收环境。以上记录区分已通过的增量检查与冷构建命令。

SQLite、Serde 和 HostEvidence 继续按原逐操作合同。定理覆盖原 Create 后首次卸载
的正常读取和工作构造。选项、凭据和其它 `FirstRead` 观察仍是条件，实际登记事务由
[#637](https://github.com/VeriInfra-Lab/sandproof/issues/637) 提供。
