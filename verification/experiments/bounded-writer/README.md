# 真实有界 Writer 的隔离验证

本目录对应 V0 [#333](https://github.com/VeriInfra-Lab/sandproof/issues/333) 的 T2 公共输出接口。
精确交付版本和验收状态见本批 PR；完整项目的状态仍由权威覆盖记录维护。

## 验证哪段实现

生产来源是 `controller-service/src/template_registry/evidence.rs::encode_bounded`
中的私有 `Writer::write/flush` 和最后的结果决定，以及
`sandbox-internal/src/server.rs::json_response` 的成功/失败选择。
生产代码仍使用真实库，本批没有修改它或 Charon/Aeneas。
`flush` 的原体只返回 `Ok(())`，在源码连接中核对；形式根是写入、结束和响应选择，
不据此提供网络刷新或持久化保证。

原工具不能直接翻译这条完整调用：动态错误载荷、递归 Serde trait 图及 HTTP
库类型存在独立限制。本批在冻结的隔离副本中拆出原判断，保留完整旧/新源码片段：

- `write_bounded` 保留原 `checked_add → limit → capacity → reserve → append` 顺序；
- `finish_bounded` 保留原 `exceeded → serializer result → bytes` 顺序；
- Vec 操作和两个固定 IO 错误构造只由机械委托传入；
- HTTP 的 500 空正文与 200 JSON 正文构造由机械委托传入，原结果分支保持透明。

固定原工具抽取这些真正被候选调用的函数。
`candidate-diff.json` 的 `lines` 拼接后给出保留空白的精确源码差异；它是审查材料，
实际私有源码由 `prepare.py` 生成。
生成 Lean 原文与完整 LLBC 保留。候选对应证明不是原生产函数一般等价性的机器证明；
原式到候选的连接由精确源码检查及 ning 对照支持，作为既有 B4 机械接线条件明示。
不能把整个自有限额判断或错误解释设为可信。

## 可以复用的结论与条件

| 接口 | 机器检查的结论 | 调用方或已批准边界提供的条件 |
| --- | --- | --- |
| 单次写入 | 正常成功返回完整本次字节和准确计数，结果长度不超过原限额；拒绝不追加字节 | 固定普通 Vec 的长度、reserve 保留内容、append 和取出内容的精确库行为 |
| 连续写入 | 保留已成功追加的全部字节、原限额和超限标记；错误后继续调用也覆盖 | 每步确实作用于原 Writer；若产生外层失败/外部效果，不假装已正常完成 |
| 结束决定 | 超限标记优先于 serializer 成功或错误；否则原 JSON 错误保留，成功取出原字节 | 原 serializer 的实际最终结果和原 Writer 不被替换 |
| 响应选择 | 编码错误选择 500 空正文；成功选择 200、原完整字节、JSON 类型和关闭连接 | 原 HTTP 构造委托符合既有库及 B4 保证；不承诺对端实际收到正文 |
| 完整 JSON | 可在固定标准派生的正确写入轨迹及错误传播条件下复用精确内容结论 | A6 的固定 Serde/serde_json 引擎、实际类型/参数和调用绑定；不是任意 Serialize 的无条件结论 |

`BufferLaws` 是逐个库原语的条件，不包含“Writer 已经正确”或“结果已经有界”。
这里的正常返回包括 Rust 的 `Err`，不包括未返回的 panic、abort 或外部效果。
历史及 200 正文的界还要求初始字节长度不超过限额；原函数从空 Vec 开始满足这一点，
使用预填充 Writer 的调用者须另外建立这个初始条件。
`WriteContract` 与实现函数分离；正确版和真实错误版使用同一份合同及模型。
普通内存缓冲区见证仅用于证明这些原语条件可以满足，Writer 算法始终来自实际生成体。

## 必须保留的失败行为

1. 大小加法溢出或超限才设置 `exceeded`。分配失败保留原标记，不自行变成超限。
2. 一次失败不会撤销此前已经写入的私有前缀，也不禁止回调继续写小块或空块。
3. Writer 没有保存任意 IO 错误的全局“第一个错误”。单次调用的检查顺序、最终
   serializer 错误及超限优先级分别处理，不能合并成更强的首错保证。
4. 任意自定义 Serialize 可以吞掉分配错误并假装成功，最终返回不完整前缀。
   这是原实现允许的历史；本批保留对应例子，不据此认领任意回调的完整 JSON。
5. 字节限额不是物理 RAM 上界。Vec capacity、分配器、序列化临时数据和整个服务
   的驻留内存没有由本模块给出上界。
6. 普通值/正常返回范围不覆盖资源载荷的可观察 Drop、panic/unwind、OOM/abort、
   async 取消或网络最终交付。JournalError 的其它资源型载荷不能据类型擦除获得保证。

## 复现与机器分工

普通 Rust/Python、fmt、Clippy 和真实错误二进制在 ning；编译/MIR、固定工具抽取、
Lean、原服务诊断和依赖审计在 a6000。调用方持有现有机器协调锁；两个重锁不同时持有。
所有 a6000 的源码、target、TMPDIR、私有 `.lake/build` 和证据在 `/data1/zn`。

`prepare.py` 从冻结生产归档导出新的私有副本，拒绝覆盖已有目录，另加本批抽取 crate。
`--variant no-limit` 只删除真实限额拒绝条件；不能用它改合同、错误工厂或证明模型。
`native.py` 先检查原 API 和正确候选，再实际新编译错误 Rust 并执行同一长度断言；
编译失败不算错误版被语义拒绝。三份自有包的库和测试同时核对 Cargo package_id、
manifest/src 路径及 `fresh=false`，随后直接执行已测量的二进制。各变体前只清这些自有包；
结束保留它们的编译原件，并隔离残留变体缓存，依赖缓存及单一 target 保留。
原生记录不是整套项目验收凭据。

形式复现使用本批登记的固定虚拟路径及只读工具/依赖映射。两版的物理目录互相独立，
映射进入后源码都在 `/tmp/bounded-writer/source`，cwd 相同。因此完整 LLBC 的源码位置、
生成注释和调用选项可以原样重建比较；不得通过删字段、改生成 Lean 或全局 `/tmp`
符号链接消除路径差异。确切映射、环境和命令随本批运行配置登记。

组件验收通过后交由统筹接入原主/独立 Lake、审计、原生 case、形式批次、服务诊断及
使用来源登记。共享完整验收和正常合入由统筹负责；组件通过不表示整个 V0 完成。

在包含本批全部文件的审查源码根目录执行以下组件入口。调度者先取得对应机器的协调锁，
设置已分配的稳定 `CARGO_HOME`、`CARGO_TARGET_DIR`、私有 `TMPDIR`，并填入下列路径变量。
`writer_baseline_archive` 是登记的生产基线归档；两个输出目录均须尚不存在。

```sh
# ning：原 API、正确候选、严格 Clippy、实际错误二进制及库来源检查。
bash scripts/python-entry.sh verification/experiments/bounded-writer/native.py \
  --source-archive "$writer_baseline_archive" \
  --output "$writer_native_evidence"

# a6000 宿主：只编排已有固定运行环境；实际检查在其原 Python/Lean 入口中执行。
python3 verification/experiments/bounded-writer/replay.py \
  --source-archive "$writer_baseline_archive" \
  --output "$writer_formal_evidence" \
  --target "$writer_formal_target" \
  --runtime-seed "$writer_runtime_seed"
```

形式源码根目录和三个形式路径变量都应解析到 `/data1/zn` 下；运行环境种子是现有注册环境，
不是重新安装的环境。逐符号来源、精确机器输入及映射在
`verification/bounded-writer-environment.json`；本批完整输入在
`verification/bounded-writer-inputs.sha256`。完整原生日志、编译原件、隔离缓存及形式日志
留在各机器的私有证据目录，源码提交不包含这些运行日志。

## 从原正式运行环境调用

宿主 `replay.py` 继续要求真实路径在 `/data1/zn`，不接受任意虚拟路径冒充数据盘。
正式批次已经处于原固定运行环境时，使用 `inside.py`；不再从那里启动依赖宿主 seed
物理路径的 `runtime.sh`。统筹保留旧挂载，只追加本批三个私有目录和只读凭据。

宿主先创建三个互不嵌套的真实目录：source 是最终候选源码，work 是独占输出父目录，
target 是本批稳定 Cargo 缓存；全部在 `/data1/zn` 下。宿主执行：

```sh
python3 verification/experiments/bounded-writer/lease.py create \
  --source "$writer_source_physical" --source-runtime "$writer_source_virtual" \
  --work "$writer_work_physical" --work-runtime "$writer_work_virtual" \
  --target "$writer_target_physical" --target-runtime "$writer_target_virtual" \
  --output "$writer_lease_physical"
```

原机凭据 schema 为 `sandproof-bounded-writer-runtime-lease-v1`，仅包含 schema、固定 bwrap
摘要及 directories。迁移到新证明机时，可在宿主添加 `--physical-root /实际私有工作根`
创建 v2 凭据；不要求复刻旧机的 `/data1/zn` 布局，也不需要管理员权限。该根必须是
当前操作者拥有、其他用户不可访问的真实目录，不能是符号链接；source/work/target
必须在该根之下且互不包含。v2 额外记录 `physical_root`，仍逐次核对三个实际目录的
设备与 inode、原 bwrap 摘要和凭据的只读挂载。旧 v1 的 data1 限制保持不变。
新宿主应重新生成目录凭据和执行环境记录，不能复制旧 inode 或旧机器的成功收据。
独立 `replay.py/runtime.sh` 仍是原 a6000 入口；新机使用上述 `inside.py` 配合登记过的
外层 wrapper，固定虚拟抽取路径与原生成物比较不变。

两版凭据的directories 恰好 source/work/target；各项保存 physical_path、
runtime_path、st_dev、st_ino。宿主核对物理 realpath；原运行环境内核对目录的 dev/inode，
拒绝同名替换目录、符号链接或相互嵌套。凭据必须由现有外层 wrapper 只读挂载，chmod
不能代替只读挂载。这是可信操作者的执行记录，不是外部上传者的签名证书。

原运行环境提供 `SANDPROOF_PROOF_BWRAP`，指向现有 bwrap 的只读别名，二进制 SHA256
固定为 `f99bf784b864f142683940b140b6444728b99accd1815768dc0b70298c40da38`。
`inside-runtime.sh` 继承原根和固定工具，设为只读；只将当前私有变体和 target 重新可写
映射到 `/tmp/bounded-writer` 和它的 target-formal，保持原 cwd、配置和生成物原文比较。
每版仍有独立 `.lake/build`，不共写生产或共享 Lean 构建目录。

```sh
# 只做两版原工具抽取与完整 LLBC/生成物比较，不是证明通过。
bash scripts/python-entry.sh verification/experiments/bounded-writer/inside.py extract-only \
  --lease "$SANDPROOF_WRITER_LEASE" \
  --source-archive "$SANDPROOF_WRITER_SOURCE_ARCHIVE" \
  --output "$writer_work_virtual/extract-preflight"

# 完整组件：两版抽取、内核、36+3 实际审计和10+7实际文件诊断，不能跳过证明。
bash scripts/python-entry.sh verification/experiments/bounded-writer/inside.py run \
  --lease "$SANDPROOF_WRITER_LEASE" \
  --source-archive "$SANDPROOF_WRITER_SOURCE_ARCHIVE" \
  --output "$writer_work_virtual/formal-component"
```

归档须位于已登记 source/work 内，两个输出必须全新。完整组件在两版全部准入后，
重新核对实际服务日志摘要、请求/文件版本、完整非陈旧结果和准确审计目标，将日志中
实际出现的 `Checked ...` 消息原样转发给模块 stdout；不按声明清单生成消息。
抽取预检不输出这些证明消息，也不写成完整验收；嵌套 namespace 探针通过同样不能代替
真实抽取或完整组件。外层 wrapper 摘要、目录映射与正式原生凭据由统筹的批次记录绑定。
