# V3：完整 CaptureWork 的未来 End 空间预留

[证明工作流](../../../../../docs/proof-workflow.md) ·
[协议范围](../../../../../docs/formal/03-protocol-inbox.md) ·
[逐符号依赖](../../../../../docs/formal/dependencies.md)

本包固定依赖 #1014 作者 `29a97e6441167ccbf0abd8f27655303c63e9925d`，生产输入为
`b0d791ccc4d4e058b710dbf8f46766652b2853df`，不自动追随工程头。实际进度在 #635
和本包 PR 维护。只提供独占同步证明、工具入口和公共登记候选，不修改生产、共享
Lake/根、原生/形式登记、冻结或覆盖记录。最终由主控整理进入 integration。

## 原入口与性质

原 `full/end.rs::reserved_record_bytes` 先编码完整 `CaptureWork`；原 Json 错误先返回。
没有 End 时加 16384，已有 End 时加 0，保留原 usize checked_add；溢出为
MaintenanceCapacity，不回绕。这里只隔离原 `serde_json::to_vec(capture)` 为 typed helper，
同一完整值传给原 serializer，不复制业务算法或信任整个 Serde。

- `SourceEndReservationModel.Contract` 对所有完整 CaptureWork 和逐符号端口陈述
  实际根的结果等于同一规格，覆盖失败和语言级效果；不把接口恒定写成成功。
- `ExactCost` 独立给出自然数语义：Ok 的 total 精确等于原字节数加未来预留且未超
  usize 上限；Err 精确为 MaintenanceCapacity 且数学和超限。
- Proof 的 actual_contract、actual_complete_mathematical_cost 和
  actual_encoding_error_first 连接实际生成体。Order 的 5 条引理、Proof 的 4 条、
  Values 的 7 条、Negative 的 5 条进入完整审计，合计 21 条新增义务。
- Values 给正常已有/缺失 End 和独立接口错误见证，并对任意成功编码字节陈述正常
  结果，不以有限样例代替完整合同。空 Vec/错误身份见证仅属于值接口，不声称真实
  Serde 对完整 CaptureWork 会产生空 JSON 或指定错误。

原生夹具使用原 API/SQLite/JSON 和受控 Host，前后比较完整记录、SQLite 行及 Sources。
这不证明实际进程/Writer/FD、材料或 VM 事实。函数也不验证已有 End 的合法性，不能
从字节计费推出整个 128 KiB 封套足够、授权、已持久保存或可释放 Full。

## 同一合同的真实错误版

`skip-future-end-budget` 仅把原无 End 分支的 END_RECORD_BYTES 改为 0。保留原完整
编码、checked_add 和错误映射；有 End 的正常分支及编码错误控制仍保持。
实际编译器将全 0 条件折叠为 checked_add(bytes, 0)；检查器只接受已观察的这一个
精确生成差异，不使用通用代码或空白规范化。Negative 给任意成功编码的错误返回、
全部端口下的已有 End 控制以及同一 Contract 的实际反例。编译/超时/导入失败均不算
性质拒绝。

完整重抽取有 116 个 canonical 类型和 182 个函数/字典；116 类型和 181 原体逐字复用
父完整视图，只有原容量函数新增。不重造标量/协议类型。外部接口精确为父 29 项加
原 typed CaptureWork 编码；不增加业务 opaque 或自定义公理，不手改生成体或工具。

## 证明侧完整入口

检查器复用原未减项的完整驱动；正确 212 个实际文件/1191 条唯一审计，错误
214/1196，保留父 207/1175 和原 retention26/127。所有实际文件的同服务诊断、依赖
审计、新目录的完整计划/文件逐字重建及十个固定 Lean 依赖均必需；局部通过不代替它们。

```bash
set -euo pipefail
export SANDPROOF_ACCEPTANCE_ROLE=formal
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-reservation
for variant in original skip-future-end-budget; do
    bash scripts/python-entry.sh "$package/capture.py" --variant "$variant" \
        --output "$reservation_capture/$variant" --tools "$fixed_aeneas" --lock "$SANDPROOF_LOCK_FILE"
done
bash scripts/python-entry.sh "$package/check.py" --inputs "$reservation_inputs" \
    --output "$reservation_evidence" --lock "$SANDPROOF_LOCK_FILE" \
    --lake-packages "$fixed_lake_packages" --lean-mcp "$fixed_lean_mcp"
```

输入 JSON 为 correct/wrong/reads/reads_audit/retention/retention_audit 六项；reads 与
审计必须是 #1014 完整正确项目，retention 为原完整 26/127。每版使用独立 .lake/build，
共享重锁与固定工具/预算；保留缓存和失败。正式判定要求完整、非 stale/partial、零
错误/警告/失败依赖，并按当前源码输入回读，不能只读成功字段。

## ning 原生交接

native.py 只准备四套完整输入，不编译/测试。各保留父 60 条，加已有 End 的精确原
完整字节成本和无 End 的未来预留两项，共 62 条同一断言。测试桥只直接调用原函数或
导出原 typed CaptureWork 编码，不替代算法。

```bash
set -euo pipefail
package=verification/experiments/stage1-protocol/unload-parent-link/source-end-reservation
for variant in original adapted-original skip-future-end-budget adapted-skip-future-end-budget; do
    case_root="$reservation_native/$variant"
    bash scripts/python-entry.sh "$package/native.py" --variant "$variant" --output "$case_root"
    flock -n "$SANDPROOF_LOCK_FILE" env CARGO_TARGET_DIR="$case_root/target" cargo test --locked --offline \
        --manifest-path "$case_root/source/controller-service/Cargo.toml" \
        -p sandproof-controller-service --lib --no-run --message-format=json
done
```

各版独立 source/target。核对实际同 crate libtest 的 manifest/src/fresh、源和程序，逐项
--exact 执行 registration-candidate.json 全部 62 个完整名字；不只筛新增前缀。
正确两版 62/62，错误两版 61 通过/1 失败，唯一拒绝必须到达
`missing End must reserve its future16384 bytes` 的原完整断言。退出 101 或其它 panic
不算性质拒绝。两错误后独立 original 重编译并恢复 62/62。采用候选按原固定 Redis
环境执行无参数 check-function.sh；原生和普通未运行不计通过。

首次原生输入回读未计 rustfmt 对全 0 条件的单行收拢，保留失败；后续仅认可这个精确
格式差异及全部文件/模式，不宽泛忽略空白。候选证明的构建/诊断失败留在本地证据，不计为 Rust 性质反例。

普通/原生交 ning；proof_native_suite、verify-stage1-protocol/verify-controller、公共
根/登记/冻结/覆盖由主控同批统一验收和内容复用，不自行并发写入或合并。完整 Full
释放/保存、set_end 的结果与错误后状态、当前 World/SQL/事务/提交未知/耐久、独立认证
和物理/完整 JSON/TLS/系统结论仍开放。set_end 另包的固定后端借用/Box 比较障碍未由
本容量证明解除。证据位置和报告哈希按用户决定只保留本地。
