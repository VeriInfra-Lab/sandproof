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

2026-09-30 的目标已改为[#123 通用模板链](https://github.com/ningZhang-cs/sandproof/issues/123)，由 #13 适配原 Dockerfile/OCI 与资源需求；新链路尚未交付。下述固定 base 手动诊断是旧路线的局部实验，不是最终产品接口，也不能代替通用构建/转换能力。后续按[验收清单](../docs/testing-plan.md)保留原解答与原评分，全部 89 题不因失败删除；修订解答和跨提交汇总必须单独报告。

现有诊断工具已实现全部任务的显式初始化配方、固定 base 指纹和 Harbor 薄适配；每题是否初始化成功、标准解答与原评分器是否完成，仍以绑定提交和环境的实际报告为准。配方清单不是通用 Dockerfile 解释器，也不表示任意上游镜像标签都可作为模板。

通用模板链的环境构建、依赖内容、工作目录/启动进程、网络策略和 Harbor 模板适配仍须实现并验收。缺少能力的任务保留在完整 89 题报告里，先记录阻塞再补齐；同一智能体的 E2B 对照按验收清单执行。在线 E2B 与模型运行使用另行配置的凭据和预算，不由这个本地工具触发。

另有[预选两题](diagnostic-tasks.json)的手动诊断链路，由[实机脚本](../tests/system/harbor_diagnostic_chain.py)执行：要求锁定的 Harbor/E2B 联合环境、固定提交的原题源码、经核对的 Ubuntu 24.04 10 GiB base、隔离 KVM 宿主与预先批准且在整轮不变的公网 IPv4 策略。每题全新沙箱，先核对 CPU/内存/磁盘和空初态；来宾固定使用公网 DNS `1.1.1.1`，因为现有 base 的宿主本地 DNS 桩不可在 VM 内使用，此处是与原 Docker 环境的已知差异。`polyglot-c-py` 只在来宾内执行原 Dockerfile 的 `apt update && apt install -y gcc`；随后依次上传并运行原 `solution/solve.sh`。原评分器所需的 Python 3.13.9 固定运行时在解答结束后传入并核对哈希，避免原评分器访问其固定下载地址超时；运行时来源和哈希见预选清单，这是另一项明确的环境差异。接着上传并运行未改动的原 `tests/test.sh` 与 `test_outputs.py`，读取原 `reward.txt`。评分后销毁并检查数据库终态、精确进程/网络/磁盘与 base 指纹。失败保留本轮私有目录诊断，不将失败题从预选名单删除；不使用付费模型，不宣称原 Harbor 模板 API 或完整榜单兼容。 这条两题链路的运行时预填与报告独立于下方全题集入口；不能用两题通过代表其余 87 题，也不替代原 Harbor OracleAgent/Verifier 的全题集接入。

## 本地标准解答探针

本入口是固定镜像/逐题初始化的诊断工具，不是 [#123](https://github.com/ningZhang-cs/sandproof/issues/123) 的通用模板产品接口；历史实验不能作为该新能力交付。

本地报告写入失败（包括磁盘写满）必须仍尝试销毁原实例；缺少报告不能计为完整通过。可写的结果文件和最终标准输出保留 `report_errors`。主动取消记录 `interrupted`，收尾后继续传播取消信号，不记作仍在运行或成功；销毁未确认则保留 `cleanup_unknown` 和原沙箱身份。进程被直接强制终止或机器故障不能靠本运行器的收尾保证，仍需外部监督器按原资源身份审计，不自动重跑掩盖未知结果。

[本地入口](run_local.py)使用固定 Harbor 的 `OracleAgent` 与 `Verifier`，通过原版异步 SDK 连接显式指定的回环服务。它只创建 `base`，不调用模板 API；[固定配方](task-recipes.json)列出全部 89 题的初始化命令与输入映射，运行时不解释 Dockerfile。配方绑定七种通用基础环境及一个任务专用预制镜像的[固定指纹](base-profiles.lock.json)，其中多阶段文档准备显式删除中间源文件，Windows 3.11 题目在 VM 内启动其原 supervisord。配方存在不代表初始化或评分通过。

```sh
uv run --locked --project benchmarks python -m benchmarks.run_local \
  --registry artifacts/terminal-bench/registry.json \
  --source artifacts/terminal-bench/source \
  --lock benchmarks/terminal-bench-2.0.lock.json \
  --api-url http://127.0.0.1:3000 --task regex-log \
  --output artifacts/terminal-bench/run-regex-log
```

密钥通过 `SANDPROOF_TEST_API_KEY` 提供；程序不读取云端密钥。输出目录必须不存在。默认 `--oracle-mode original`。执行前复核全部 89 题源码，使用该题原始初始化、agent 与 verifier 时限；只在标准解答阶段上传 solution，只在评分阶段上传 tests，保留原退出码和 reward。`oracle=passed` 只表示所选解答脚本退出 0；整题通过还要求原评分器返回 reward=1，不能只凭解答脚本退出码判断解题成功。输出包含可取得的该题日志和仍含全部 89 题的 `full-set.json`；原评分器返回后会尝试记录 dpkg 软件包清单，初始化或解答阶段中断时可能没有该清单。未执行不能算通过。失败和超时返回非零，记录已经开始的阶段与中断命令；失败后限时收集已有日志及来宾内存、磁盘诊断，诊断读取失败不影响随后尝试销毁。销毁未确认会单独标记，必须先核对原资源，不能直接重跑。

服务必须预先配置与当前题目一致的 CPU/内存、10 GiB 独立盘、86400 秒单次及总寿命上限，以及公网 IPv4 出站。服务二进制、配置文件与数据库目录须由 root 持有且仅 root 可访问；安装二进制时使用 `0700`，不能沿用普通构建产物的 `0755`。服务的固定 base 必须切换为该题配方指定的预制镜像；CPU/内存也按原题设置。原 SDK 仍仅请求 `base`，模板构建与任意 Dockerfile/OCI 转换不属于服务能力。Python/Debian 镜像以明确的官方 amd64 清单及所有层摘要固定，加入同一 envd、固定静态 BusyBox 启动配置和独立 guest 网络配置；这些本地实验镜像的隔离与启动证据必须分别取得。DNS 使用公网解析器 1.1.1.1，宿主若有 Docker 的 FORWARD 默认拒绝策略，应由监督器按本轮唯一网卡配置和回收精确转发规则，不能关闭全局防火墙。执行入口不修改宿主网络、不启动服务，不代替独立的 PID/磁盘/cgroup/网络清理审计。

[本地服务预算](local-service-budgets.json)固定本题集的串行实验配置，由监督器合入可信服务配置，不是生产默认值。上传 64 MiB 覆盖固定输入的最大单文件 12,640,606 字节；下载 96 MiB 与命令响应 32 MiB 分别约束日志和含 Connect 编码的输出。请求/写出空闲预算保持实跑使用的 13,000 秒，文件空闲预算为 30 秒；它们独立于题目最长 12,000 秒的执行时限，不代表已验收长时间空闲连接，也不延长任务自身时限。并发普通数据请求仍受每 VM 8 个名额中保留 1 个控制名额的限制，服务全局名额为 16。CPU、来宾内存和镜像按原题设置；每个 VMM 的 cgroup 内存上限为来宾内存加 512 MiB，不能当作整个服务的宿主内存上限。

全局名额与服务/relay 资源观察使用[三 VM 实机探针](../tests/system/server_task_pressure.py)，详见[验收清单](../docs/testing-plan.md)。这项压力实验显式扩大 VM 准入，保持文件/命令/并发/空闲传输预算不变，不能当作串行题集配置或正式部署容量的验收。运行示例（路径须替换为预检过的 root 专用产物；脚本不创建或委派父 cgroup）：

```sh
sudo /path/to/pinned-python tests/system/server_task_pressure.py \
  --server /private/sandproof-server --jailer /private/jailer \
  --firecracker /private/firecracker --kernel /private/vmlinux \
  --rootfs /private/base.ext4 --scratch-parent /private/pressure \
  --cgroup-parent /sys/fs/cgroup/pressure --uid-base 63401 --keep-artifacts
```

完成后从本轮目录导出脱敏 `report.json`；配置与原始日志含测试凭据，保持 root 私有且不提交。成功报告核验六台实例已回收；保留目录仍需独立审计后按精确路径删除。

最终镜像的隔离补验使用[原 SDK 私有盘探针](../tests/system/sdk_private_root.py)的 `--task-profile --expected-cpus N --expected-memory-mib M`：复用双客户端 root/ubuntu 数据断言，检查两个 VM 的包数据库与系统文件隔离，以及失败初始化留下的文件和存活子进程随 VM 销毁后不进入新实例。该场景明确把准入上限改为 2；串行题目实跑不表示已经验收双 VM 满载能力。独立监督器应记录真实 machine-config、10 GiB 私有盘身份、cgroup、宿主可用内存/磁盘与服务进程树，再核对精确资源清理和 base 摘要。失败初始化探针不代替控制器崩溃或全部宿主故障矩阵。

上游初始化和评分脚本包含动态 apt/下载依赖，现阶段记录实际版本，尚未把所有远端依赖内容冻结。这是本地兼容与评分链实验，不是环境完全可复现、完整 Harbor Trial、全题集验收或在线 E2B/模型对照的完成声明。

基础环境与上游可变标签存在明确差异：Python 3.13/Bookworm 选择固定 3.13.9；其余官方标签解析为清单中的精确摘要。镜像中的 init/envd 是本服务需要的适配；金融文档题的 uv 0.8.14 和 reshard 题的 uv 0.8.15 使用显式固定版本安装，前者清理 builder 输入，后者从 Python wheel 获取。默认模式下标准解答和私有评分器保持源文件原样，不以环境适配冒充上游容器内容一致。

来宾初始化补齐精简镜像缺少的 `/dev/fd`，使 Bash 进程替换可以访问 `/proc/self/fd`。Ubuntu 来宾初始化保留基础镜像已有的配置文件，避免非交互 apt 安装因 dpkg 配置确认而中断。个别任务还需要显式基础环境补齐：CompCert 题安装 findutils，MCMC/Stan 题补齐 RcppParallel 当前构建所需的 cmake，Git Webserver 题仅在来宾内解除 ssh 服务屏蔽；pMARS 使用经过原发行版签名验证的 2025-10-01 Debian 历史仓库，POV-Ray 与 RStan/PyStan 题使用同日期 Ubuntu 历史仓库，满足原脚本的版本约束。这些适配不改动标准解答、私有测试或原资源时限，历史仓库也不能代替对其他动态下载内容的完整锁定。

hf-model-inference、pytorch-model-recovery 和两道 MTEB 题的原 pip 安装命令禁用下载缓存，减少 10 GiB 私有盘上的重复存储；保留原 PyTorch 版本及其依赖，不改用其他构建变体。

PyTorch 恢复题的原评分器还会用 uvx 创建独立 Torch 环境。该题先由原 pip 对 `torch==2.7.1` 做 dry-run 解析，记录完整 wheel 版本和摘要，再用与评分器相同的 uv 0.9.5 按这些摘要安装并硬链接缓存；随后仍执行原 pip 安装命令。评分环境通过 `UV_LINK_MODE=hardlink` 复用相同 wheel 的文件，避免两份 CUDA 库挤满私有盘。初始化核对实际版本及大文件链接数，并保存解析和布局记录；不修改原解答或评分器，不直接编辑 uv 缓存。

MTEB 检索题也用此布局共享两个 Python 环境中相同的 wheel；原 pip 继续处理源码分发包，随后核对全部解析版本。该题另为 pip 和 uv 同时约束 Transformers 4.57.6，保留旧 MTEB 导入的 `AutoModelForVision2Seq`；该接口在 [Transformers 5 官方迁移说明](https://github.com/huggingface/transformers/blob/main/MIGRATION_GUIDE_V5.md)中列为已移除。不同 Python ABI 的 wheel 仍分别安装，不修改原评分器要求的 Python 版本。

RStan/PyStan 题以 wheel URL 和 SHA-256 约束 setuptools 80.9.0，保留原 PyStan 3.10.0 仍导入的 `pkg_resources`；[setuptools 官方变更说明](https://setuptools.pypa.io/en/stable/history.html)记录了该模块的移除。原 pip 升级与安装命令继续执行，httpstan、PyStan 和分析脚本均不修改。

MCMC/Stan 题设置 `R_REMOTES_UPGRADE=never`，保留原解答显式安装的 Rcpp、RcppEigen、BH 与 StanHeaders 版本，避免后续安装 RStan 时又被自动升级；缺失依赖仍正常安装。该设置由 [remotes 官方接口](https://remotes.r-lib.org/reference/install_version.html)提供，不修改原模型或采样脚本。

Windows 3.11 题补齐来宾 `/var/tmp`（1777）；其原 QEMU 5.2.0 在未设置 `TMPDIR` 时需要这个目录创建磁盘快照临时文件。保留原 QEMU 构建、Windows 镜像、启动参数和评分器。

path-tracing-reverse 的原评分器会跨 `/app`、`/tmp` 重命名文件。该题初始化在 guest 内保存临时目录内容、停止独立 tmpfs 挂载并恢复到同一私有盘，保留目录内容和 sticky bit；不改动原评分器的文件操作。

两道 QEMU 题的 Bullseye 源固定到与基础镜像构建注释一致的 2026-08-24 官方历史仓库，避免现有索引引用已返回 404 的软件包。历史源保留原 Debian keyring 的签名检查，仅对该固定快照停用过期时间检查；初始精简镜像尚无 CA bundle，apt 通过 HTTP 获取内容并核对签名及包摘要。

Cython 扩展题使用独立 pip 约束将 Planarity 固定为带源码 SHA-256 的 0.6，保留 pyknotid 0.5.3 需要的图属性接口；原标准解答和测试不变。该约束只影响该题，依据为本地异常堆栈及[上游同类问题](https://github.com/harbor-framework/terminal-bench-2/issues/75)。

GPT-2 题的标准解答预留大于来宾物理内存的虚拟地址空间；该题初始化仅在来宾设置 `vm.overcommit_memory=1`，允许延迟按需分配物理页。来宾 4 GiB 内存、宿主 cgroup 上限及原评分时限保持不变。

Mailman 题在来宾补齐基础镜像缺少的 `/var/mail`（root:mail，2775），使 Postfix 能创建本地邮箱。两道 MIPS 题的原 Doom 共享版数据链接失效，改从 [SliTaz 列出的 TuxFamily 镜像](https://www.slitaz.org/en/get/)下载相同发行数据并校验固定 SHA-256；下载失败立即中止初始化，不能把 HTTP 错误正文当作 WAD 文件。

`custom-memory-heap-crash` 使用该题元数据指定的预构建镜像 `alexgshaw/custom-memory-heap-crash:20251031`，amd64 清单固定为 `sha256:1895d7d6240d5fb2ec774d6bfdd1057d170f8d199c15c582e4585d3c63802557`。原 Dockerfile 的两套 GCC 13.2.0 定制库已在其中构建，初始化无需在 600 秒内重新编译编译器。离线转换不执行镜像程序；加入固定 BusyBox/init 和 envd 后仍为 10 GiB 私有盘。执行时核对 task 专属标记、原 program/patches 内容摘要及两套库存在性。该 profile 仅允许原任务与原上游镜像引用使用；不允许其他任务复用，也不包含 solution/tests。原标准解答、评分器、CPU/内存及所有任务时限保持不变，是否实机通过仍以该提交的独立报告为准。

PyTorch 张量并行题保留原 TCPStore 默认后端、PyTorch 2.7.0、Gloo 与评分脚本。`USE_LIBUV=0` 短探针曾结束，但完整原评分仍超时，因此没有保留这项无效设置；原生栈定位到 Gloo 析构时的 GIL 死锁，候选修复单列在下方可选参考解答中。

Windows 3.11 初始化下载仍使用原 archive.org URL，按 8 MiB 分段、最多四条连接下载，核对每个 HTTP Content-Range、总长度及固定 SHA-256 后才发布完整文件。该摘要通过原站公开 SHA-1 核对取得；下载发生在题目初始化预算内，没有在 VM 中预填镜像，也没有延长 600 秒时限。失败不会留下可误用的半成品；下载证据写入 verifier 日志目录。通用下载辅助程序来自本仓库，只允许注册的 helper 标识上传。

HTML 过滤题启用 pytest 的 `-s -v` 与非缓冲日志，保留每个浏览器批次的执行、异常与收尾信息。原测试先杀 ChromeDriver 再调用 quit，实际观察到浏览器及 crashpad 进程持续遗留；该题通过 Selenium 官方 `SE_CHROME_PATH` 设置选择[浏览器启动包装器](guest_chromium.py)。包装器不改变 Chromium 参数，使用 pidfd 观察驱动生命周期；驱动退出后回收独立进程组与自己收养的后代，并记录剩余数量。合成进程回归覆盖驱动死亡和脱离原 session 的后代，原浏览器实测另计。用例、断言和时间限制不变；原测试会吞掉部分浏览器异常，必须检查日志中的超时/浏览器失败，不能只用原 reward 声称所有浏览器场景实际通过。环境接口见 [Selenium Manager 官方说明](https://www.selenium.dev/documentation/selenium_manager/)。

该题另外设置来宾 `net.ipv4.tcp_syn_retries=2`，让失联外部资源的 TCP 建连在原页面加载预算内返回失败；读取并保存实际值到 `tcp-connect-policy.json`，不修改宿主网络参数。相同页面的独立 VM 对照覆盖默认值与该设置，并保留真实弹窗阳性及干净页面阴性检查。此配置不改 HTML、浏览器参数、原评分或 20 秒页面时限，也不替换 HTTP 响应；外部子资源仍可能不可达，浏览器测试完成不表示这些远端内容也已被验证。参数含义见 [Linux 内核文档](https://docs.kernel.org/networking/ip-sysctl.html#tcp-syn-retries-integer)。

MTEB 排行榜题约束仍被原 MTEB 1.38.41 支持的 Torch 2.7.1 与 Transformers 4.57.6，保留完整 PyTorch/CUDA 构建。原无上限依赖会选入更新且更大的 CUDA 13 依赖，使随后原 results 完整克隆写满 10 GiB 私有盘；旧兼容依赖是否足以留出空间必须以原解答复测为准。没有预填结果、改变 Git checkout 目标或缩减评分。 初始化另外补齐 Pillow 11.3.0；[上游问题记录](https://github.com/harbor-framework/terminal-bench-2/pull/53)确认旧 MTEB 丢失该间接依赖后重新构建会导入失败。原 MTEB 版本保持 1.38.41。

该题的 pip 安装使用 `--no-compile`，并设置 `PYTHONDONTWRITEBYTECODE=1`，避免可再生成的 Python 字节码缓存挤占原评分器安装 Python 3.13 所需的空间；Python 源码、原生库与完整 CUDA wheel 均保留。初始化记录实际缓存策略及可用空间，不扩大私有盘、不删除结果仓库或评分输入。

## 可选的参考解答修复

显式 `--oracle-mode repaired` 只对[注册修复](oracle_repairs.py)生效。它先核对原 solve.sh 的完整摘要，在独立输出目录复制完整任务，仅对其中 solution 应用有限修改，不修改固定题库；报告记录模式、原文件与新文件摘要及完整 diff。原 Harbor OracleAgent 按副本任务根目录执行原路径校验，评分仍从原任务读取。评分源码和所有资源/时限保持不变。修正版通过必须单独报告，不能计为原标准解答通过，也不能改写原失败记录。未注册任务直接拒绝执行。

- `protein-assembly`：通过 [RCSB 官方 Data API](https://data.rcsb.org/)的 polymer entity 与 chemical component 字典还原修饰残基及有序母体残基；保留单一聚合物筛选，拒绝无法解析的序列。原 FASTA 将部分荧光团编码为 `X`，无法满足 DNA 翻译约束；修复不嵌入目标序列，原 DNA 优化和评分继续执行。
- `build-pov-ray`：从 ICM 公开镜像下载相同三个 2.2 压缩包，并核对与[上游 PR #77](https://github.com/harbor-framework/terminal-bench-2/pull/77)相同的 SHA-256；其余编译与渲染流程不变。原站对直接下载返回 HTTP 403。
- `caffe-cifar-10`：从 BrainChip 公开镜像下载相同 CIFAR-10 binary 数据，内容已核对[作者公布的 MD5](https://www.cs.toronto.edu/~kriz/cifar.html)，执行时再核对固定 SHA-256。下载、解压、原 Caffe 构建及训练均在 VM 原 agent 预算中，不预填数据或训练结果。
- `crack-7z-hash`：先用原 John 自带完整常用密码字典和原规则运行，并要求 John 确认破解成功；解压密码取自实际破解结果，替代原脚本的答案常量。没有任务专用候选表，也没有跳过破解计算。模式语义见 [John 官方说明](https://www.openwall.com/john/doc/MODES.shtml)。
- `install-windows-3.11`：识别并复用原 supervisor 已启动、参数匹配的 VNC WebSocket 进程，避免重复绑定 8080 导致解答退出 1；不把任意端口监听都当成所需服务。原 QEMU 编译、镜像、启动与监控仍执行，原四项评分不变。
- `torch-tensor-parallelism`：原生栈确认 Gloo 销毁时，主线程持有 GIL 等待工作线程，工作线程释放 tensor 又等待 GIL。修正版让模型持有直接 Gloo 后端引用，使用 [Torch 2.7.0 已有的释放 GIL 的析构绑定](https://github.com/pytorch/pytorch/blob/v2.7.0/torch/csrc/distributed/c10d/init.cpp)完成清理；不改集体通信、模型计算、PyTorch 版本或原评分。完整实跑结果按提交记录在独立的 repaired 模式报告中。
- `mteb-leaderboard`：完整克隆 results 仓库时加 `--no-checkout`，避免先展开当前 HEAD 的大工作树占满 10 GiB 私有盘，再执行原脚本指定的 `c0fd3271442f8c0c839084a109fede85c8514dab` 检出。MTEB 1.38.41 的 `results_repo` 仍被当作克隆源，修正版通过其 `MTEB_CACHE` 接口将缓存根指向现有检出的父目录，避免再次复制完整工作树。保留完整历史与对象，不使用浅克隆、稀疏检出或预填结果；原排行榜计算与评分不变。
