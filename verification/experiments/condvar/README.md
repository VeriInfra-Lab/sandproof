# Condvar：等待和重新持锁

[验证工具入口](../../README.zh-CN.md) · [依赖保证](../../../docs/formal/dependencies.md) · [V0](../../../docs/formal/00-foundations.md)

本接口限定同一个原 Condvar、原 Mutex 和一次活跃等待，允许其它线程在等待期间取得、修改和释放这个锁。当前生产代表是 `sandbox-runtime/src/child_process/reaper.rs`。原回收线程只有一个等待消费者的条件由 V6 提供；本接口没有覆盖同一 Condvar 的多个同时等待者或通知公平性。

`Core` 描述原守卫的暂停及重新领取，`Protocol` 限定可执行的等待/唤醒/超时/返回步骤，`Examples` 给出合法非空历史。守卫身份表示本次被传入的 Rust 值，不是其内存地址；重新持锁会获得新的逻辑领取序号，但保留原守卫的 panic 标记。受保护数据表示原字段和句柄，不把外部进程或别名可变堆冻结成快照。

依赖的原标准库、七组生产调用和业务提供方在 [接口登记](../../condvar-environment.json) 中分别列出。A6 提供固定库语义，B2 只连接原锁、原守卫、原通知及完整结果。`reap_ready`、队列谓词、入队资格、进程退出和 Slot 释放策略没有整体变成可信条件。一般性原 Rust 精化也没有由源码登记或原生测试取得。

使用方提供合法原持有者和业务写入关系；如果每个写入保持业务不变量，可以复用等待历史的不变量保持结论。唤醒后必须读取当前数据并再次判断业务条件。通知不保存供未来等待领取的许可；虚假唤醒可能发生；通知或超时观察均不等于已重新取得锁。总返回时间没有硬上界。中毒错误仍持有原守卫，`wait_timeout` 的错误载荷还保留完整超时结果元组。

`returnWithoutReacquiring` 是专门用于反驳的错误模型，它被正确模型同一 `Safe` 性质拒绝，不是提取的 Rust。原生错误版只在 API 夹具中把原锁替换成另一把锁，再由正确版同一原对象断言拒绝；它不是生产 Reaper 变异，也不是 Rust 的形式反例。

在 ning 运行提交后封口的原生入口：

```sh
python3 tools/check_condvar_components.py --output "$NATIVE_EVIDENCE/condvar"
```

该入口在私有源码副本追加 `reaper-tests.rs`，执行四项新用例、原 Reaper 用例、fmt、严格 Clippy 和新编译错误夹具。线程使用通道和原 Mutex 协调；没有随机 sleep。真实 `Slot::retain` 用例支持原对象接线的采用，不证明整个回收循环或无条件进展。

在 a6000 使用原固定证明环境、已验证 ning 回执及 `/data1/zn` 下的临时目录：

```sh
bash scripts/verify-condvar.sh
```

`preflight.py` 对未修改的原 `Reaper::run/reap_ready` 执行 Charon/Aeneas，保存完整 LLBC、日志和源指纹。固定工具的循环借用合并限制单独记录，不算错误版或实现证明。正式批次还要由统一入口执行保留集合、精确审计和原 Lean 服务的真实文件诊断，不能只运行本目录的模型就宣称整套验收通过。
