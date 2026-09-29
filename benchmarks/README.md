# 固定完整任务集

[文档目录](../docs/README.md) · [项目蓝图](../docs/project-blueprint.md) · [验收清单](../docs/testing-plan.md)

当前入口实现完整题集的源码锁定、资源审计、Harbor/SDK 联合依赖与原任务加载检查，不执行任务代码或启动沙箱。范围为固定 Harbor registry 中 `terminal-bench@2.0` 的全部 **89 题**；不能在失败后删除任务或把未运行计为通过。

[锁文件](terminal-bench-2.0.lock.json)固定 Harbor、E2B SDK、registry 内容摘要、任务仓库提交，以及每题所有输入文件的路径、模式、大小和 SHA-256。文件按 environment、instruction、metadata、oracle、verifier 分别标记；标准解答和私有测试仅记录摘要，不能提前上传到智能体沙箱。原 `docker_image` 标签和 Dockerfile FROM 行只是上游环境参考，不是镜像内容锁，也不表示本服务支持模板构建。

## 获取和核对

需要 Python 3.11+、Git；这里只下载源文件，不安装或执行题目提供的依赖和脚本。源码目录保持独立、干净，不在里面生成报告、缓存或虚拟环境。

```sh
mkdir -p artifacts/terminal-bench
curl --fail --location \
  https://raw.githubusercontent.com/laude-institute/harbor/31668af15560fb50a9d0584816d12726036656cf/registry.json \
  --output artifacts/terminal-bench/registry.json
git init artifacts/terminal-bench/source
git -C artifacts/terminal-bench/source remote add origin https://github.com/laude-institute/terminal-bench-2.git
GIT_LFS_SKIP_SMUDGE=1 git -c core.hooksPath=/dev/null -C artifacts/terminal-bench/source \
  fetch --depth=1 origin 69671fbaac6d67a7ef0dfec016cc38a64ef7a77c
GIT_LFS_SKIP_SMUDGE=1 git -c core.hooksPath=/dev/null -C artifacts/terminal-bench/source \
  checkout --detach FETCH_HEAD
python3 tools/benchmark_tasks.py verify \
  --registry artifacts/terminal-bench/registry.json \
  --source artifacts/terminal-bench/source \
  --lock benchmarks/terminal-bench-2.0.lock.json
```

工具要求 registry 精确字节、89 个唯一任务及固定提交匹配；逐文件对照 Git blob 和 SHA-256，拒绝遗漏、篡改、额外文件、符号链接、submodule 以及未审查的环境字段。普通检查不需要下载题库；小型本地 Git 测试夹具覆盖完整性拒绝路径。`freeze` 使用相同输入重建锁文件，版本更新仍须经过开发流程，不能改用浮动分支。

## Harbor 与 SDK 联合环境

[项目配置](pyproject.toml)和[联合锁](uv.lock)固定 Harbor `31668af1…` 与 E2B SDK `ccaf9fc0…` 的 Git 源码及其运行依赖。它与 SDK 单独的合同环境分开保存，避免修改其他测试的依赖。使用 Python 3.12–3.13、uv 0.11.31：

```sh
bash scripts/check-bench.sh
uv run --locked --project benchmarks python -m benchmarks.check_harbor \
  --registry artifacts/terminal-bench/registry.json \
  --source artifacts/terminal-bench/source \
  --lock benchmarks/terminal-bench-2.0.lock.json \
  --report artifacts/terminal-bench/harbor-inputs.json
```

第一条检查实际安装包的 Git 提交/子目录、Harbor 原 E2B 模块的 SDK 导入，并在联合依赖下运行原 SDK 的本机回环合同测试。CI 的 `harbor-inputs` 执行这一层，不下载或运行题库、不调用云服务和模型。第二条先复核完整固定源码，再使用原 Harbor `Task` 加载全部 89 题，对比其实际解析的 CPU、内存、磁盘和超时字段；不执行初始化、标准解答或评分。Git 提交不符、仅 PyPI 包版本相同、字段解释不一致均拒绝。报告与退出码仍只对应本次检查，不表示本后端继承了上游 E2B 的网络或模板能力。

## 逐题资源审计

[配置样例](capacity.example.json)是既有小型实机测试的资源档位，用于暴露差距，**不是全题集推荐配置**。替换为待验收服务的实际来宾 CPU、内存 MiB、私有盘 MiB 和两个寿命上限；这份输入不能代替读取宿主配置和真实 cgroup、磁盘、进程的验收证据。

```sh
python3 tools/benchmark_tasks.py audit \
  --registry artifacts/terminal-bench/registry.json \
  --source artifacts/terminal-bench/source \
  --lock benchmarks/terminal-bench-2.0.lock.json \
  --capacity benchmarks/capacity.example.json \
  --report artifacts/terminal-bench/resource-audit.json
```

所有题均进入报告，并逐项列出资源差距。退出码 `0` 仅表示源码校验和已配置资源比较通过；`1` 表示存在资源差距，仍生成完整报告；`2` 表示输入不可信或无效，不生成新的成功报告。调用者必须检查本次退出码，不能使用上次遗留的报告。当前工具始终报告 `ready_to_run=false`；它不根据 CPU/内存匹配猜测初始化、网络、标准解答或评分已经通过。

原任务 `memory`/`storage` 的 G/M/K 按固定 Harbor 的二进制单位解释。全部任务要求 10 GiB 盘；CPU/内存分布为 71 题 1 CPU/2 GiB、13 题 1 CPU/4 GiB、3 题 2 CPU/4 GiB、2 题 4 CPU/8 GiB。未修改的固定 Harbor E2B 接入请求 86400 秒 create 寿命，因此审计同时比较单次和总寿命上限，不把任务 agent timeout 冒充该请求。

## 后续执行前提

逐任务审查初始化配方、固定 base 及依赖内容、工作目录/启动进程、网络策略和 Harbor 薄适配仍须实现并验收。Dockerfile 清单不是通用解释器，不能把上游镜像标签伪装成可用模板。缺少能力的任务保留在完整 89 题报告里，先记录阻塞再补齐；标准解答、原评分器、清理及同一智能体的 E2B 对照按验收清单执行。在线 E2B 与模型运行使用另行配置的凭据和预算，不由这个本地工具触发。
