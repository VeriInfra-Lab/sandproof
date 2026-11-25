# 热分支窗口关闭与额度回收

[证明工作流](../../../docs/proof-workflow.md) · [覆盖记录](../../../docs/proof-coverage.md)

本组件固定候选 `1388de447126e025c9080557261d41baca48bbb4`，对应已审阅的 #1030 与 `integration` 合并树及宿主路由回归。`msra` 上的 Charon 抽取覆盖分支关闭状态、额度占用、生命周期窗口的 Rust 事务路径，以及 `retire_closed_parent` 中的 SQL 调用。Aeneas 生成体和已登记的库、模型与审计保持原输入。

`CloseProof` 检查原分支状态转换与兄弟凭据保持。`CloseSql` 在明示的 SQLite 活动连接、扫描、删除、更新和 A4 提交条件下，证明正常提交看到完整新镜像，正常回滚看到原镜像，未知提交可以看到两种完整镜像之一。三份独立原生构建检查正常、提前提交错误版和恢复后的同一真实 SQLite 断言。错误版须在数据库重开后的原额度断言被拒绝。

两 BLOB 更新的生产侧和消费侧分别引入原生成视图。`BlobInvocation` 携带两边共同核对的原执行身份、SQL、两个有序 BLOB 和受影响行数。同一次物理调用、同一笔活动事务和正确的更新行状态是显式前提。

退休抽取检查器沿 `query_map` 的第三个实参和编译器选择的 `FnMut` 字典定位原回调，核对对同一个回调行的 `Row::get<usize, Vec<u8>>` 调用按顺序读取第 0 到 4 列，并将列号写入 `row_callback_columns`。检查器再按原 `ControlFlow` 枚举闭包的六条普通返回路径。`row_callback_outcomes` 记录每条路径读取了哪些列、返回了哪一列的首个错误，或按什么顺序返回五列成功值。五列只读到首错位置，原错误经同类型转换传播。所有读取成功时，返回原五个值组成的有序 `Stored`。

检查器还从同一个 DELETE 的 `execute` 结果跟踪三条普通路径。`delete_result_outcomes` 记录恰好一行时回到原删除循环，其他成功计数返回 `Err(CorruptStore)`，SQL 错误经原转换后返回。两个错误路径都沿循环退出到真实函数返回。窗口路径检查另行核对退休出错后不会调用 `save` 或 `commit`。

实际游标和 BLOB 值、DELETE 影响的物理行及事务镜像、受保护重开、宿主授权和资源释放仍需各自的实现对应。

## 固定整数 LIMIT 与五列游标

`NormalBranchScan` 沿用原 `ReadView`、绑定返回的原 statement 和 `IssuedMappedCursor`。条件限定于由 Charon 回执生成的固定 SELECT 和整数参数。页内用既有固定 `rowGet index blobColumn` 推出逐次 Next 的五列值、原 SQL 错误及游标 EOF 保持，并将正常扫描映像接到 `CloseSql.Select`。回执同时提供原 `FnMut` 六条路径和原 DELETE 的摘要实参来源。首列操作 BLOB 按原字节排入结果。

用下节原局部入口加 `--with-branch-scan` 重建和检查此页。限定的 SQLite 提供方仍须绑定物理事务和同一次原整数查询。原 Rust `load` 对解码操作键摘要等于第零列的检查、按确切父机筛出所有键、实际闭包到库视图、窗口更新值到两 BLOB 以及受保护重开均有单列义务。

用原入口加 `--with-load-key` 另行抽取透明的原 `branching::load`。源检查器从原五列元组的第零列及第 3 列记录开始，沿原 JSON 解码结果、操作键字段、摘要比较和正常返回追踪。检查器要求成功返回同一解码记录，且经过原键不匹配条件的否分支。这项检查与原 DELETE 的全长摘要实参检查同用一份 LLBC。`OperationKeyBytes` 从原第 0 列的 BLOB 转换推出 `Vec<u8>` 的完整值，用已审计的 `InboxLoadLibrary.digest_ne` 消去完整字节比较。给定选中的字典已归约为原参数顺序的切片比较，守卫返回 `false` 就推出摘要切片等于原列 BLOB。复验入口同时登记这三条 Lean 定理。原 Rust 字典到该切片调用的对应，以及实际 JSON/SHA 到后续 DELETE 摘要计算的对应，各自仍待接入。

`--with-load-key` 从原 V3 固定清单安装 `InboxLoadValueLibrary` 和 `InboxLoadDigestLibrary` 的导入闭包，包含 `ReplayEquality`、配置预算和固定的 Stage1 生成视图。入口按原 InboxLoad 库存核对两页的安装路径、摘要和七条审计，再把这两页送入逐文件 Lean 内核和注册服务检查。结果记录把原库存、V3 物料清单及两页绑定到这次运行。

## 活动 SQLite 原生对照

`sqlite_activity_native.py` 把测试夹具追加到精确候选的私有源码归档。它在原控制器连接上登记固定 SQLite 的语句跟踪及行更新回调，记录一次 `BEGIN IMMEDIATE` 下的实参、游标行数、各自原键对应的 DELETE 行、收件箱两个有序 BLOB 及提交或回滚。提交前使用第二连接核对旧镜像可读、独占写入受阻和产品控制租约拒绝。通过产品打开原文件后，再核对库和租约身份、原 WAL/FULL 设置、整张相关旧或新镜像以及旧请求重放。

夹具分别验证提交前退出、未知但回滚、未知但提交、提交后回复失败和正常确认。还把真实原行的操作键替换成另一份等长 BLOB，固定其余各列和收件箱，要求原退休在尝试 DELETE 或 UPDATE 之前拒绝，重开后两表字节保持。两个独立错误版分别提前提交和删除原操作键比较，均须成功编译，并分别在原提交前读回和原键拒绝断言被拒绝。原生记录仅覆盖上述实物样例。游标值和 SQL 事务结果到现有 Lean 条件的通用实现对应仍需单独完成。

使用下节同一固定工具、协调锁及 `/mnt/data2tb` 私有输出。候选须是已审核且干净的检出，另在独立目标目录编译和运行每个变体。

```sh
taskset -c "$native_cpus" "$proof_python" -B \
  "$candidate_checkout/verification/experiments/hot-branch-close/sqlite_activity_native.py" \
  --candidate "$candidate" --checkout "$candidate_checkout" --output "$sqlite_activity_output"
```

## Linux 复验

使用 `msra` 的数据盘、项目固定的原证明工具和独立输出。按工作流取得原 Stage1 锁及可继承的文件描述符，设置 `SANDPROOF_ACCEPTANCE_ROLE=formal`、`SANDPROOF_LOCK_FILE` 与 `SANDPROOF_LOCK_FD`。两个入口各在四个已分配 CPU 上运行。原生入口需要离线 Cargo 缓存。`--checkout` 指向含固定候选及其父节点的干净检出。

```sh
component=verification/experiments/hot-branch-close
candidate=1388de447126e025c9080557261d41baca48bbb4
taskset -c "$proof_cpus" "$proof_python" -B "$component/replay.py" \
  --approved-linux --with-window --with-blob-bridge --candidate "$candidate" \
  --checkout "$candidate_checkout" --tool-root "$proof_tool_root" \
  --packages "$formal_packages" --output "$proof_output"
taskset -c "$native_cpus" "$proof_python" -B "$component/native_witness.py" \
  --approved-linux --candidate "$candidate" --checkout "$candidate_checkout" \
  --output "$native_output"
```

保留原源码归档、生成物、Rust 和 Lean 逐文件日志、证明服务读回及 `result.json`。本组件的记录与完整登记证明及真实 KVM 验收分别查阅。
