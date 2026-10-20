# 标准 Mutex 原守卫与唯一借用代表接口

[证明流程](../../../docs/proof-workflow.md) · [合同与边界](../../../docs/formal/dependencies.md#a6b2-标准-mutex原守卫中毒与唯一借用) · [覆盖](../../../docs/proof-coverage.md)

本目录提供真实标准 Mutex 代表用法的可复现检查。生产实现与原 Charon/Aeneas 不改；支持范围及十三组原操作/业务提供方由 verification/std-mutex-environment.json 固定。共享守卫是 A6/B2 条件模型，实际透明抽取仅覆盖 contention-diagnostics 下 SerialMutex::get_mut，不把全部异步/共享锁 Rust 或业务回调称为已证。

## 模型和实际函数

Guard 描述同一锁、原数据/句柄、独占守卫及领取序号、中毒、遗忘与真实唯一借用条件。35 条条件声明含合法非空轨迹、诊断失败观察前原解锁的正确/错误模型。借用模块的七条声明证明原生成 get_mut 的分支和回写，Bridge 四条声明连接调用者提供的写关系与守卫模型，共 46 个正式审计目标。中毒恢复不修复数据；正常解锁不析构 Mutex 内的 payload；get_mut 不解开被遗忘守卫留下的底层锁。

## 两台机器与命令

ning 使用现有固定 Rust、Python、离线 Cargo 和真实 Redis 环境，先提交冻结输入，执行独立组件或完整原生入口：

```sh
bash scripts/python-entry.sh tools/check_std_mutex_components.py --output /private/new-mutex-component
bash scripts/python-entry.sh tools/proof_native_suite.py run --output /private/native-evidence
```

组件将测试附加到原模块的私有副本，十一项正确用例、严格 Clippy 与两份新编译错误版均保留原二进制/摘要/日志。late-unlock 必须失败于正确版相同的“诊断失败观察前未解锁”断言；swallow-poison 必须失败于相同的“原中毒可变借用被静默接受”断言。编译失败、缺依赖、超时不能计为反例。

a6000 接收完整 ning 原生成功记录与日志，不接收 Cargo target，按正式流程指定固定工具、已验证来源捕获和缓存后执行：

```sh
bash scripts/verify-std-mutex.sh
bash scripts/verify-controller.sh
```

专用入口实际重抽取原同步函数，比较完整 LLBC、生成 Types/Funs 和清单；仅允许原检查器已审查的无序 short_names 表规范化，保留 raw 产物。共享入口预检保留原 PTY/Meter 后端障碍，不把失败交给 Aeneas 当作成功体。实际错误版仅在隔离源码中改变 get_mut 的中毒分支，重新编译、原工具抽取，再以原服务/内核证明它不满足同一个 GetMutContract。PoisonError<&mut T>::into_inner 只在这一个已核对借用实例采用具体 CPS 模型，不能推广为任意拥有值 API。

完整入口保留所有旧模块、依赖审计、实际物理文件诊断及新模块；同输入复用要核对完整闭包和原始证据，明确 RUN/REUSE，不称复用为本轮重证。重任务按 #333 的主机侧队列运行，不清 Lean 缓存，不改保护或断言。

## 使用者责任

使用者提供原对象/守卫/实际 Rust loan、受保护字段、真实写关系与业务不变量、完整参数、错误与中毒恢复政策。Meter exact=false、PTY close/句柄克隆和最后资源归还不是机械层给出的业务结论。公平、死锁、外部工作退出、进程 abort/崩溃后的清理另证；Condvar、Tokio Mutex/RwLock 和全项目其他使用点不随本批完成。
