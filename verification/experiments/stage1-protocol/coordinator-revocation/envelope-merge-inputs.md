# 合并 #1061 时的封套输入

[候选补丁](envelope-merge-inputs-candidate.patch)适用于 `integration@87bc20b8` 与 `#1061@de37231d` 的组合。主控先保留两边对 `frozen.json` 的修改，并在默认 Stage1 根清单中登记组合后的文件。相对 #1061，这份组合清单中只有 `inputs["tools/ci_retention.py"]` 发生变化，对应原生构建回收工具。协议 4 的 request、manifest 和 carriers 生成记录与 #1061 逐字段相等。[主控 PR](https://github.com/VeriInfra-Lab/sandproof/pull/1061)

补丁同步四个组件对组合清单的直接引用。bytes 还引用 schema 的输入清单，budget 引用 schema 与 bytes，writer 引用前三份。默认根清单登记四个最终文件。组件的 Rust、证明、错误版、生成预期和审计项均取原版本。

在隔离组合的仓库根目录执行以下命令，`patch_file` 指向本候选补丁。

```sh
git apply --check "$patch_file"
git apply "$patch_file"
shasum -a 256 --check --quiet verification/stage1-protocol-inputs.sha256
bash scripts/python-entry.sh -c 'from pathlib import Path; from tools.proof_native_suite import load; r = Path.cwd(); load("envelope_merge", r / "verification/experiments/stage1-protocol/registered.py").validate_registration(r, profile="full")'
```

上述两项准入沿用晋升入口的实际检查。ning 在主控确定的集成提交签发普通和原生证据，证明机在同一候选运行正式 Stage1 入口。加入应用协调器证明栈后，还须复核协调器 Rust 和公共登记器的其他消费者。[晋升准入](../../../../tools/ci_promotion_acceptance.py) · [协调器交接](https://github.com/VeriInfra-Lab/sandproof/issues/635#issuecomment-7228984552)
