# 源材料保存事实的完整保留守卫

[V3 规格](../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../docs/formal/dependencies.md) ·
[证明工作流](../../../../docs/proof-workflow.md)

固定实际 integration `9c50da7cc9391518514f14405e6c779e5c5cbe2c` 的
`source_work::retirement::preserves_saved_facts`。正确视图不修改生产，直接从
这个原私有函数抽取；没有验证专用替代算法或整个业务 opaque。

同一 `FullPreservationContract` 对任意完整已解码输入给出完整布尔精化、
接受/拒绝 iff 和总性。旧 producer、consumer 若存在，下一份必须保存同一
完整 HostReleaseWitness：position、expected_previous、完整五字段 HostBinding
及 input_sequence。旧 facts 不存在时允许新增，此守卫不验证新增事实真实性。

旧 retirement 不存在时允许新增；旧 Retiring 可保持 Retiring 或转 Settled，
但必须保留完整 plan：全部原始 bytes、work_binding、plan_binding、完整 closure，
以及 reason 的原构造类别与全部字段。只比较 plan_binding 不满足本合同。
旧 Settled 必须保存完整同一状态，包含全部 plan 和 settlement 字段，不能
回退为 Retiring 或改写释放字节数。Retiring→Settled 的这个守卫自身不校验
新 settlement 是否有效；完整退休校验仍需另外执行。

`accepted_path_preserves` 复用标准 Relation.ReflTransGen：每条边若都被这个
真实守卫接受，最初的已存事实在任意有限值历史中保持。存在带原始事实的非空
合格历史见证。V3 仍须连接原 Codec 的完整 work 查找、键与逐行参数；V5 提供同一个物理
来源行及真实事务、失败/未知提交和重启历史。不能把这些场景直接填成成功边。

正确版和删 producer、删 consumer、只比 plan_binding、忘记 Settled 不可变性
四个实际 Rust 错误版使用相同 Spec、Values 和 Examples。错误环境不导入
正确 Core、StateEquality、Trace 或 Witnesses，分别实际计算正常值和反例并
反驳同一完整合同。所有生成体来自固定原 Charon/Aeneas，逐字节核对，不导入
含 axiom 的生成模板。只填既有 A6 普通 Option 两个模型；Vec、固定数组与
标量比较继续使用固定 Aeneas/已登记合同，自有相等均保留真实透明正文。

持已有共享重锁、在获准最多四 CPU 上运行本组件：

```sh
python3 verification/experiments/stage1-protocol/source-preservation/formal.py \
  --lock /private/existing-proof-lock --output /private/source-preservation-formal
python3 verification/experiments/stage1-protocol/source-preservation/native.py \
  --output /private/source-preservation-native
```

形式入口重新编译/捕获五个真实视图，核对原生成体，检查全部实际文件的原内核、
规定同服务完整非 stale/partial 诊断和标准依赖。各视图有独立 .lake/build，
保留全部失败与缓存。共享执行器可用 --lock-fd 转交同一已持有锁，避免自锁。

原生正确、四错、恢复六视图均使用全新 Cargo target、同13项新组件测试及
原 `codec_refuses_erasing_original_release_facts_or_refreshing_fixed_plan`。
测试从原 SQLite Unload/Resume/材料退休/settle 夹具取得真实记录；额外检查
原 Codec 的 Retiring 和 Settled 拒绝位置。严格核对每版实际协议库/服务测试
的源、manifest、新编译/链接、精确失败集合、格式和 Clippy，之后恢复原 Rust。
受控 Host 证据与实际 VM/文件释放事实分开，不把测试 receipt 当物理 attestation。

本包不证明完整 Codec::save、真实 JSON/Serde/SHA、授权、物理资源存在或释放、
原 SQL UPDATE/事务提交、未知提交/恢复、整个系统。比较完整摘要值不证明算过
哈希，也不提供内容真实性或无碰撞结论。正式普通/原生验收仍由 ning，原共享
verify-stage1-protocol/verify-controller 接纳由主控协调；局部开发不签发验收。

公共登记、依赖与覆盖仅提供候选，由主控单写采用：
[公共登记](public-registration-candidate.patch) ·
[依赖](dependencies-candidate.patch) · [覆盖](coverage-candidate.patch)。
原固定 Create 十二包不变；同批其它在途模块的累计数由主控核对。
