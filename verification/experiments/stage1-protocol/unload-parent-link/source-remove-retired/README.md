# 来源退休的逐记录清槽

[证明工作流](../../../../../docs/proof-workflow.md) ·
[V3 详细计划](../../../../../docs/formal/03-protocol-inbox.md) ·
[依赖合同](../../../../../docs/formal/dependencies.md)

本包固定生产输入 `b0d791ccc4d4e058b710dbf8f46766652b2853df`，
树 `7ca91afd9cd79601d562f3c636066d192fc25fc5`，继续 #635。
复用 #996 的初始化/SQL 视图和 #945 的完整来源记录类型及保留检查；
旧证明保持各自原始输入和证据，不把它们迁记为当前生产事务已证。

`extract.py` 将 `codec::remove_retired` 的原逐记录循环体原样移入
`clear_retired_source_record`，保留原父函数的事务检查、空输入处理、加载和循环。
固定后端仍不能翻译原父循环的嵌套借用，所以本包选择逐记录根。
JSON/SHA 调用逐符号隔离；原 SQL 的六个动态参数隔离为固定六元组。
生产源码保持只读，生成函数体不手改。

## 性质与接口范围

`SourceRetireEquality` 证明完整 `SourceWorkRecord` 的比较等价于值相等，
包括全部六个主字段及其嵌套请求、观察、捕获、恢复和材料字段。
`SourceRetireSpec.rowSpec` 按原先后顺序规定：

1. 复制完整工作键并插入集合，拒绝重复；在当前记录集中按完整值查找。
2. 有捕获时要求已结算；无捕获却有退休状态时拒绝。
3. 执行透明的原 `material_charge`，保留其错误，且材料计费必须为零。
4. 按完整工作键查询原四列，调用原完整校验器，再检查正行号、完整记录、规范字节与保留额度。
5. 用原空值、摘要、行号、旧字节、旧摘要和完整工作键执行条件更新，恰好一行才成功。

`root_contract` 对全部输入和显式库接口成立，保留首错、语言失败、不返回和插入后的集合。
`Contract` 为正确/错误视图使用同一性质。完整值相等不表示已经授权、持久接受、材料真实，
也不证明资源可物理退休；后续错误不表示早先更新已经回滚。

`SourceRetireCases` 复用已有完整保留额度合同，提供任意完整记录在精确逐调用条件
`Ready` 下的正常控制：无捕获、无退休及释放责任，完整当前记录和四列读取，
完整可选记录解码/规范重编码、记录重编码、摘要及六个绑定值全部保持。
原 `checked_retired_record`、`material_charge` 和四列回调均透明执行，不把它们设为假设。
`Ready` 只列库调用点和已给出的值状态；没有构造冒充真实记录编码的 JSON 字节，
也不声称已经证明这些点可由正式 Serde、加载事务或实际资源提供。

接口继续复用既有 SQL 对象和值模型。九个 JSON/SHA 适配仍需逐调用对应，
不把整个 Serde、SQL handler 或父事务设为可信。以下提供者义务仍需单独采用和检查：

- V0：实际 `BTreeSet<Vec<u8>, Global>` 插入、字节向量比较及完整记录的共享扫描；
  不能把已有字符串键合同直接扩大为字节向量键。
- V0/SQL 接线：四列转换、单参数查询与固定六参数绑定的精确接口。
  六元组与原 `params!` 的等效值只在原语句/参数数目及固定 `ToSql` 条件下连接，
  保留参数计数和绑定首错；不混淆 Owned/Borrowed 的原始载体。
- V3：完整可选记录的 JSON 模式，以及退休、释放、恢复绑定元组的实际字段接线。
- V5：原活动事务、完整当前记录及行状态的来源、实际更新/回滚和未知提交。
  物理材料和资源事实继续由原提供方负责。

## 负例及原生输入

`allow-zero-update` 只把原更新行数判定 `!= 1` 改为 `> 1`。
两份 Rust 均已按固定工具编译/抽取；生成差异限于该判定的四个展开分支。
`SourceRetireNegative` 原样导入错误生成根。在同一个 `Ready`、完整记录、四列及六参数
条件下，一行正常控制仍成功，零行却返回成功，由同一完整 `Contract` 拒绝。
这份有条件生成体反例不替代下面真实 SQLite 反例的原生执行。
本目录尚未作为正式 V3 或整套验收封口。

## 证明机复现

先用原固定工具分别取得 `original` 和 `allow-zero-update` 捕获；每次保留原源码归档、
全部五份 staging、完整工具日志、LLBC 和全部生成物。固定角色/预算、四核和共享重锁
由 `extract.py` 检查；不能以失败或没有翻译的捕获代替性质反例。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
package=verification/experiments/stage1-protocol/unload-parent-link/source-remove-retired
for variant in original allow-zero-update; do
    bash scripts/python-entry.sh "$package/extract.py" --variant "$variant" \
        --output "$retire_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$retire_inputs" \
    --output "$retire_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

`retire_inputs` 是私有 JSON，恰含六个原件目录：`correct`、`wrong`、`reads`、
`reads_audit`、`retention`、`retention_audit`。前两项是本包的正确/错误捕获，
后四项分别是 #996 的完整 99 文件/483 审计项目与原审计，以及 #945 的完整
26 文件/127 审计项目与原审计。检查器读取全部原始服务结果和源码，不只相信汇总。

物化器重新构造捕获源码，逐文件核对完整生产归档及五份隔离输入，检查实际 Charon/
Aeneas 成功、完整生成清单、根与九个库适配的映射；对 #945 的 93 个类型和 105 个
函数逐字核对当前声明。声明体原样使用，并保留工具原有的 `reducible` 元数据。
正确项目保留全部 123 份父文件及七份新文件；错误项目再保留实际错误根和反例，
共 130/132 份实际文件、675/679 项唯一审计。不能只构建父根或缩减原审计。

`check.py` 自己持共享重锁，不能在外层重复锁同一文件。两版使用独立 `.lake/build`，
可复制到独立目录的缓存保留；不删除 `.lake`。原同服务客户端检查全部实际文件，
拒绝错误、警告、失败依赖、stale/partial 或缺审计；最后新目录重建整份计划和全部
文件逐字比较。输出始终标为开发检查，不能签发公共原生或整套验收成功。

`native.py` 只准备原始/隔离正确版和对应的两个真实错误版，
逐文件保存完整输入清单，恢复原 `lib.rs`，使用同一份七项 SQLite 断言。
原生编译、实际断言及正确版恢复交给 ning：

```bash
bash scripts/python-entry.sh verification/experiments/stage1-protocol/unload-parent-link/source-remove-retired/native.py \
  --variant original --output /private/source-retire-original
```

另三种 `--variant` 为 `adapted-original`、`allow-zero-update` 和
`adapted-allow-zero-update`。ning 在各自独立源/target 上先实际编译同 crate libtest，
核对 manifest/src/fresh、实际程序与输入摘要，再逐个执行 registration-candidate.json
列出的七个完整名字并使用 `--exact`。测试前缀为
`controller::lifecycle_inbox::source_work::tests::end_pipeline::source_retire_`。
预期正确版各 7/7，错误版各 6/1，失败必须来自
`retired source clearing must reject a zero-row compare-and-set`。
夹具用真实 SQLite 触发器使条件更新返回零行；编译失败、超时或环境失败均不算性质反例。
两份错误版后还须在独立重建的正确源码上取得 7/7 恢复结果。

七项检查覆盖事务门先于空输入、空事务不读缺失 schema、完整目标槽及未改的额度/
其它行、同键改完整字段拒绝、未结算材料保留、真实零行条件更新拒绝，以及重复输入
的首条更新持续到显式回滚。它们调用原父入口，使用真实 SQLite 和原受控 Host 夹具；
不启动 VM，也不把组件结果解释成物理退休或整个父事务的形式化证明。

完整普通入口和公共 `proof_native_suite` / `verify-stage1-protocol` / `verify-controller`
由主控同批协调；这里的开发证明不替代这些封口，也不替代实机验收。
