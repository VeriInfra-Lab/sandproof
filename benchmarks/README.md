# 固定完整任务集

[文档目录](../docs/README.md) · [项目蓝图](../docs/project-blueprint.md) · [验收清单](../docs/testing-plan.md)

预检入口实现完整题集的源码锁定、资源审计、Harbor/SDK 联合依赖与原任务加载检查，不执行任务代码或启动沙箱。独立的本地执行入口见下文，必须连接已配置并由外部监督器审计的真实服务。范围为固定 Harbor registry 中 `terminal-bench@2.0` 的全部 **89 题**；不能在失败后删除任务或把未运行计为通过。

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

## 执行与验收边界

全部任务的显式初始化配方、固定 base 指纹和 Harbor 薄适配已实现；每题是否初始化成功、标准解答与原评分器是否完成，仍以绑定提交和环境的实际报告为准。配方清单不是通用 Dockerfile 解释器，也不表示任意上游镜像标签都可作为模板。失败、超时和缺少能力的任务均保留在完整 89 题报告中；同一智能体的 E2B 对照按验收清单另行执行。在线 E2B 与模型运行需要单独配置的凭据和预算，不由这个本地工具触发。

## 本地标准解答探针

[本地入口](run_local.py)使用固定 Harbor 的 `OracleAgent` 与 `Verifier`，通过原版异步 SDK 连接显式指定的回环服务。它只创建 `base`，不调用模板 API；[固定配方](task-recipes.json)列出全部 89 题的初始化命令与输入映射，运行时不解释 Dockerfile。配方绑定七种[基础环境指纹](base-profiles.lock.json)，其中多阶段文档准备显式删除中间源文件，Windows 3.11 题目在 VM 内启动其原 supervisord。配方存在不代表初始化或评分通过。

```sh
uv run --locked --project benchmarks python -m benchmarks.run_local \
  --registry artifacts/terminal-bench/registry.json \
  --source artifacts/terminal-bench/source \
  --lock benchmarks/terminal-bench-2.0.lock.json \
  --api-url http://127.0.0.1:3000 --task regex-log \
  --output artifacts/terminal-bench/run-regex-log
```

密钥通过 `SANDPROOF_TEST_API_KEY` 提供；程序不读取云端密钥。输出目录必须不存在。执行前复核全部 89 题源码，使用该题原始初始化、agent 与 verifier 时限；只在标准解答阶段上传 solution，只在评分阶段上传 tests，保留原退出码和 reward。输出包含该题日志、实际 dpkg 软件包清单以及仍含全部 89 题的 `full-set.json`，未执行不能算通过。失败和超时返回非零，记录已经开始的阶段与中断命令；失败后限时收集已有日志及来宾内存、磁盘诊断，诊断读取失败不影响随后尝试销毁。销毁未确认会单独标记，必须先核对原资源，不能直接重跑。

服务必须预先配置与当前题目一致的 CPU/内存、10 GiB 独立盘、86400 秒单次及总寿命上限，以及公网 IPv4 出站。服务的固定 base 必须切换为该题配方指定的预制镜像；CPU/内存也按原题设置。原 SDK 仍仅请求 `base`，模板构建与任意 Dockerfile/OCI 转换不属于服务能力。Python/Debian 镜像以明确的官方 amd64 清单及所有层摘要固定，加入同一 envd、固定静态 BusyBox 启动配置和独立 guest 网络配置；这些本地实验镜像的隔离与启动证据必须分别取得。DNS 使用公网解析器 1.1.1.1，宿主若有 Docker 的 FORWARD 默认拒绝策略，应由监督器按本轮唯一网卡配置和回收精确转发规则，不能关闭全局防火墙。执行入口不修改宿主网络、不启动服务，不代替独立的 PID/磁盘/cgroup/网络清理审计。

上游初始化和评分脚本包含动态 apt/下载依赖，现阶段记录实际版本，尚未把所有远端依赖内容冻结。这是本地兼容与评分链实验，不是环境完全可复现、完整 Harbor Trial、全题集验收或在线 E2B/模型对照的完成声明。

基础环境与上游可变标签存在明确差异：Python 3.13/Bookworm 选择固定 3.13.9；其余官方标签解析为清单中的精确摘要。镜像中的 init/envd 是本服务需要的适配；金融文档题的 uv 0.8.14 和 reshard 题的 uv 0.8.15 使用显式固定版本安装，前者清理 builder 输入，后者从 Python wheel 获取。原任务标准解答和私有评分器保持源文件原样，不以环境适配冒充上游容器内容一致。

来宾初始化补齐精简镜像缺少的 `/dev/fd`，使 Bash 进程替换可以访问 `/proc/self/fd`。Ubuntu 来宾初始化保留基础镜像已有的配置文件，避免非交互 apt 安装因 dpkg 配置确认而中断。个别任务还需要显式基础环境补齐：CompCert 题安装 findutils，Git Webserver 题仅在来宾内解除 ssh 服务屏蔽；pMARS 和 POV-Ray 题分别使用经过原发行版签名验证的 2025-10-01 Debian/Ubuntu 历史仓库满足原脚本的版本约束。这些适配不改动标准解答、私有测试或原资源时限，历史仓库也不能代替对其他动态下载内容的完整锁定。

hf-model-inference 的原 pip 安装命令禁用下载缓存，减少 10 GiB 私有盘上的重复存储；保留原 PyTorch 版本及其依赖，不改用其他构建变体。
