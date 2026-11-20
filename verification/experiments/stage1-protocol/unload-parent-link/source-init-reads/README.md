# 来源初始化的两次配置读取

[完整循环](../source-init-loop/README.md) · [容量父调用](../source-init-capacity/README.md) ·
[完整维护读取](../source-policy-read/README.md) · [证明工作流](../../../../../docs/proof-workflow.md)

接续 #995，生产固定 `b0d791ccc4d4e058b710dbf8f46766652b2853df`。本包交付实际容量读取
的来源及原初始化首错连接；局部证明、公共登记、原生/普通及整套验收分别报告，不能由
一次交互检查或构建代替，也不宣称整个 V3 完成。

## 复用及新连接

实际 capacity::config_using 的完整生成体和完整读取对应已经由 #992 提供。本包直接
复用 capacity_read_refines 和 readFullView，不重复证明或复制读取算法；正确捕获继续
使用 #993 原件。Contract 仅打包原完整值/错误规格，供同合同正反检查。

新增性质连接容量返回的来源：成功必来自这一次完整 read_using 返回的 policy，保留
原 after 状态、完整 capacity 及实际维护校验成功；成功充要关系不丢任何容量字段。
实际 generic 和 standard 容量读取均不返回 Ok(None)。读取错误、失败和非返回保持，
首读错误先于容量、编码及写入。标准根单次成功来源和单次错误分别对应原 wrapper。

两次读取的组合使用同一已有 ReadConfigEffects 类型与原 generic 函数，显式保留
before → middle → after。policyAt/capacityAt 只把原回复投影给原初始化端口，以相同
db 选择同一个 effects 实例；它们不是另一份业务算法。首次读取验证并计算原 count，
第二次在 middle 观察错误时，原父调用保留该错误，不进入后续编码/写入。

合法状态见证提供第一次成功到 middle、第二次实际端口错误及原初始化结果，防止组合
只靠互相矛盾的前提成立。不能通过更换 Deserialize 回调参数使无状态的同一 SQL 查询
在同一输入上同时成功和失败，也不能据此推导两次物理读取来自同一快照。
实际 StandardConfigRead 的物理连接/数据库状态如何对应这些效果状态仍由 V5 提供；
本包不把 generic 组合宣称为完整 SQLite 时序、持久接受或同事务证明。

## 真实错误与同一合同

drop-current-capacity 保留原维护读取和 capacity 存在检查，仅把已取得的成功结果丢弃，
返回 Ok(None)。固定 Charon/Aeneas 已实际编译和抽取该 Rust；类型和函数名全部相同，
只有 controller.capacity.config_using 生成体变化，没有新增 opaque 或改工具。

物化器原样加入完整错误声明，正常合法维护配置下，错误体返回缺失容量，被同一完整
Contract 拒绝；首读错误控制仍保持。正确及错误版共用原完整读取规格，正常完整容量
见证不换成另一份性质。编译、环境、导入和超时失败不算性质拒绝。

正确捕获沿 #993 原身份复用，重新核对源码归档、完整 staging、工具日志、LLBC、全部
生成物及映射。原容量读取声明逐字等于 #995 的实际视图；完整父计划重建。错误捕获的
staging 精确包含原 CODEC、LIB 和唯一变异的 capacity.rs，不能漏掉第三份输入。

## 证明机复现

`row_capture` 是 #993 原正确捕获；`loop_project/loop_audit` 是 #995 正确原件。
`loop_inputs` 私有 JSON 恰含 actual、reference、prior、audit、metadata_inputs 五项原
物化参数。原 Metadata/Row/Capacity 输入闭包继续重建，不把新目录或摘要改记为旧执行。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-reads
parent=verification/experiments/stage1-protocol/unload-parent-link
bash scripts/python-entry.sh "$package/extract.py" --variant drop-current-capacity \
    --output "$reads_evidence/wrong-capture" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
for variant in original drop-current-capacity; do
    actual="$row_capture"
    if test "$variant" != original; then actual="$reads_evidence/wrong-capture"; fi
    project="$reads_evidence/$variant-project"
    bash scripts/python-entry.sh "$package/prepare.py" --actual "$actual" \
        --reference "$row_capture" --prior "$loop_project" --audit "$loop_audit" \
        --loop-inputs "$loop_inputs" --output "$project" \
        --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
    module=$(jq -r .root "$project/plan.json")
    (cd "$project" && flock -n "$SANDPROOF_LOCK_FILE" taskset -c "$four_cpus" lake build "$module")
    bash scripts/python-entry.sh "$parent/diagnose.py" --project "$project" \
        --output "$reads_evidence/$variant-audit" --lock "$SANDPROOF_LOCK_FILE"
done
```

核对 registration-candidate.json 的全部实际文件、唯一审计和依赖。服务结果须完整、
无错误/警告/拒绝、无失败依赖和 stale/partial；原日志与源码摘要重核。各项目使用独立
.lake/build，父源码一致后可复制缓存到独立目录，不能共写、删除 .lake 或更换工具。
本开发入口不签发公共整套成功；共享登记及覆盖只交主控候选。

## ning 原生与普通验收

native.py 只准备原始正确、隔离正确、原始错误、隔离错误四份源码。原生隔离副本恢复
原 lib.rs，另记原适配字节后以固定 rustfmt 规范 CODEC/capacity，捕获原件不改。
四版追加完全相同的四条读取断言，不执行测试。

夹具使用真实 SQLite 内存连接、原维护表 DDL、实际 user_version、v3::maintenance 的
完整合法配置及原 JSON 编解码。只装配本读取所需的配置表，不运行完整 ControllerJournal
opener、生命周期初始化或 VM，以保证本变异到达所声明的容量断言。它不是持久性验收。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-init-reads
for variant in original adapted-original drop-current-capacity adapted-drop-current-capacity; do
    case_root="$reads_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run
done
```

ning 核对实际新编译 libtest 的 manifest/src/fresh、实际程序和输入摘要，再按登记四个
完整名字逐项 `--exact`：

- 完整合法维护配置的全部容量字段，成功回复必须为 Some(原完整 capacity)。
- 配置行已删除时，错误格式版本仍先为 UnsupportedStore；版本恢复后才报真实缺行查询错。
- 原 operation/retention/capacity 缺失或零容量值继续被完整维护校验拒绝，回滚后全文保持。
- 同一真实连接中先读完整 policy，再由夹具在事务内显式删除配置行，第二次容量查询保留
  QueryReturnedNoRows，回滚后全文不变。这个有限组件不是完整初始化并发或快照证明。

正确两版各应 4/4；错误两版各应 3 通过/1 失败，唯一拒绝必须到达
`source capacity read must preserve the complete current validated capacity`。
错误版保留原读取及存在检查，合法配置下返回 Ok(None)，必须在完整值断言失败；
准备、环境、超时、编译或退出 101 本身不算反例。随后另建 original 恢复副本重新编译
并跑四条，应 4/4；全部失败保留。

最终采用候选显式使用原已校验 Redis，运行无参数 bash scripts/check-function.sh。
proof_native_suite、verify-stage1-protocol/verify-controller、公共原生/正式模块、根导入
及覆盖由主控同批统一验收和内容复用，不重复无关全套或新增 KVM。

## 后续提供方

V0/主控继续审查既有库薄绑定及整个生产到隔离候选的一般等价；V5 提供实际查询/JSON/
摘要、连接状态、快照/事务/耐久及整库预算，V7 提供独立认证/配置来源，V6 提供实际资源
事实。普通与原生验收仍交 ning；公共配置、登记、冻结、依赖和覆盖只交主控候选。
