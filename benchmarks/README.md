# 固定完整任务集

[文档目录](../docs/README.md) · [项目蓝图](../docs/project-blueprint.md) · [验收清单](../docs/testing-plan.md)

预检入口实现完整题集的源码锁定、资源审计、Harbor/SDK 联合依赖与原任务加载检查，不执行任务代码或启动沙箱。独立的本地执行入口见下文，必须连接已配置并由外部监督器审计的真实服务。范围为固定 Harbor registry 中 `terminal-bench@2.0` 的全部 **89 题**；不能在失败后删除任务或把未运行计为通过。

[锁文件](terminal-bench-2.0.lock.json)固定 Harbor、E2B SDK、registry 内容摘要、任务仓库提交，以及每题所有输入文件的路径、模式、大小和 SHA-256。文件按 environment、instruction、metadata、oracle、verifier 分别标记；标准解答和私有测试仅记录摘要，不能提前上传到智能体沙箱。原 `docker_image` 标签和 Dockerfile FROM 行只是上游环境参考，不是镜像内容锁，也不能代替实际构建和可启动产物的证据。

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

2026-09-30 的目标已改为[#123 通用模板链](https://github.com/ningZhang-cs/sandproof/issues/123)。自助构建服务和下述 #13 通用执行入口已实现；实现存在不等于全部 89 题验收通过。下述固定 base 手动诊断是旧路线的局部实验，不是最终产品接口，也不能代替通用构建/转换能力。后续按[验收清单](../docs/testing-plan.md)保留原解答与原评分，全部 89 题不因失败删除；修订解答和跨提交汇总必须单独报告。

现有诊断工具已实现全部任务的显式初始化配方、固定 base 指纹和 Harbor 薄适配；每题是否初始化成功、标准解答与原评分器是否完成，仍以绑定提交和环境的实际报告为准。配方清单不是通用 Dockerfile 解释器，也不表示任意上游镜像标签都可作为模板。

通用模板链的各类实际任务仍须逐批验收。缺少能力的任务保留在完整 89 题报告里，先记录阻塞再补齐；同一智能体的 E2B 对照按验收清单执行。在线 E2B 与模型运行使用另行配置的凭据和预算，不由这个本地工具触发。

## 通用模板标准解答入口

[通用入口](run_generic.py)只替换 Harbor 的模板准备：提交自有构建 API，等待不可变模板，再使用原 `AsyncSandbox`、`OracleAgent` 和 `Verifier`。不使用 `task-recipes.json` 或预制任务镜像替换，不调用 E2B 云模板服务。默认使用原解答；通用入口与实机监督均可显式传 `--oracle-mode repaired`，仅运行下述已注册修复，报告独立标记模式，不计为原解答通过。默认 `--source-kind oci` 与固定 Harbor 优先使用 `docker_image` 一致；显式 `dockerfile` 则打包原 `environment/`，原样交给标准 BuildKit。两条来源在报告中分开记录，不将 Dockerfile 重建声称为上游预构建标签的相同字节。客户端不调用固定 SDK 的 Dockerfile 解析器，因此多阶段不受其限制。

构建诊断按服务返回的原始字节偏移分页保存到 `build-log.json`，总上限 2 MiB、读取期限 30 秒，JSON 展开后的文件也计入逐试验产物预算。`complete` 仅表示已到达终态构建日志末尾，超限记录 `truncated`；读取失败保留已有页和错误类型，不改写原构建终态或重新提交构建。

`filter-js-from-html` 的原评分器会吞掉部分浏览器错误并给出成功奖励。通用链只在评分阶段设置 `PYTEST_ADDOPTS=-s -v` 和 `PYTHONUNBUFFERED=1`，保存逐批浏览器日志，并额外检查批次数、连续批次编号、实际完成的无弹窗检查及被吞掉的超时/浏览器错误；缺日志或检查失败不能报告整题通过，原 reward 与 oracle/verifier 状态仍保留。这不修改原镜像、解答、评分源码、资源或时限，也不安装固定 base 的浏览器包装器。通用链的 `browser_lifecycle_audit=not_instrumented` 明确表示未观察每批浏览器进程回收，最终 VM 资源仍须外部审计；它不是浏览器超时的修复或实机通过证据。

通用入口及实机监督可显式加 `--failure-diagnostics`，在原评分已失败后、销毁原实例前执行[已登记的来宾诊断](failure_diagnostics.py)。`build-cython-ext` 诊断会核对原解答已克隆的 pyknotid 提交和测试文件，直接展开原先被外层断言隐藏的内部 pytest 输出，并记录 Python/依赖版本。额外诊断最多 60 秒，共享逐试验产物预算，不编辑测试或安装依赖；输出为独立的 `failure-diagnostic.txt/json`，不更新原 reward、解答或评分状态，取消和错误仍须清理原实例。它属于评分后的独立复现，不延长原解答/评分期限，未实际运行不能记为已定位或已修复。

`gpt2-codegolf` 的评分后诊断只使用原镜像已有的 Bash、GCC 和基础命令，不依赖系统 Python。它核对原 `gpt2.c` 的固定摘要，记录来宾 overcommit/内存信息，在临时副本中为两次原内存分配加入空指针检查，随后立即退出；分配失败分别返回 77/78，记录 errno，避免诊断再次解引用空指针。副本不运行模型推理，退出时删除临时目录并复核原文件未变；不修改内存策略、资源限制或评分结果。该探针同样需要显式启用 `--failure-diagnostics`，受 60 秒和逐试验产物预算约束。

```sh
uv run --locked --project benchmarks python -m benchmarks.run_generic \
  --registry artifacts/terminal-bench/registry.json \
  --source artifacts/terminal-bench/source \
  --lock benchmarks/terminal-bench-2.0.lock.json \
  --api-url http://127.0.0.1:3000 --build-url http://127.0.0.1:3001 \
  --source-kind dockerfile --task regex-log --task polyglot-c-py \
  --output artifacts/terminal-bench/generic-run
```

两端必须是已配置的本地服务；凭据用 `SANDPROOF_TEST_API_KEY` 提供。CPU、内存、**总盘容量**来自原任务，最小空闲空间另为 64 MiB；原构建/解答/评分时限不改，沙箱寿命仍请求 86400 秒，部署不足须拒绝而非截短。运行池与构建池的资源预算、输出磁盘下载预算必须分别满足请求，不能沿用小镜像探针的 2 GiB 下载上限。原镜像启动默认值来自正式登记结果，不在客户端解释 Dockerfile 的 WORKDIR/USER；平台初始化始终独立于用户 CMD。只接受本轮已约定的公网 IPv4 策略，不在初始化阶段临时扩大权限。

构建盘与目标运行盘是两种资源：构建盘必须同时容纳工具/基础系统、解包暂存和完整目标盘；它的实际空闲空间不足时工作器拒绝，不因目标盘同为 10 GiB 就认为构建盘也只需 10 GiB。监督服务为候选完整字节、暂存及收尾分别记账；早期工作器尚未创建 status.json 时也保留有限失败日志。使用大构建盘不改变原题要求的运行盘大小。

通用模板启动会挂载空的 `/run`，随后重新创建权限为 `1777` 的 `/run/lock`，让原镜像的 `/var/lock → /run/lock` 链接可用。目录创建或权限设置失败时不发布启动就绪标记；不改原链接、应用配置或评分。这是平台启动目录适配，组件回归不能代替 Mailman 等原任务的实机验收。

实机监督可用 `--build-input-max-bytes` 和 `--build-expanded-max-bytes` 显式配置构建下载/输入与展开层的预算，默认分别为 4 GiB 和 8 GiB，仍受构建服务 16 GiB 和 32 GiB 的硬上限约束。原公开 OCI 可能含大于默认下载额度的层；调整时必须同时提供已核对的更大构建盘。配置、构建池上传额度、工作区预留、宿主磁盘预检及 worker 空闲量检查使用同一组数值，并写入 `capacity.json` 和 `acceptance-budgets.json`；原任务运行盘、CPU/内存、评分和时限保持不变。例如原运行盘为 10 GiB，选择 8 GiB 输入与 32 GiB 展开额度时，构建盘须有至少 58 GiB + 64 MiB 实际空闲空间，原 32 GiB 构建盘会在派发前拒绝。提高上限不是构建或原题已通过的证据。

构建意图及幂等键先落盘；构建、创建或清理未知时停止后续任务，不用新身份重跑。只上传锁定的环境文件；solution/tests 由原 Harbor 在解答/评分阶段上传，启动时检查保留目录不存在。报告保留原失败及全部 89 题，`--all` 才选择全集；下载依赖未完全锁定会明确标注。工具退出 0 仅表示本次执行检查通过，`kill_confirmed` 是 SDK 回执，`exact_cleanup=not_audited` 不是宿主回收证据。

通用 KVM 监督入口使用[统一验收配置](template-service-budgets.json)及[配置生成/预检](../tools/template_acceptance.py)，运行池传输和寿命字段直接复用[已有任务配置](local-service-budgets.json)。同时计入构建池和运行池的完整 CPU/VMM 内存额度、额外存活私有盘、发布产物、构建工作区、证据和清理余量；预检使用当前 CPU affinity、可用内存和剩余磁盘。服务/relay 的份额由[显式服务 cgroup 入口](../tools/service_cgroup.py)落实；控制服务与构建监督服务及其子进程共用限额，VM 使用各自独立分组。仅生成配置仍标记为未观察到执行，监督入口在核对实际服务 PID 归属后才记录已施加上限；当前数值仍不是已实测的生产总预算。报告保存实际生成配置及来源摘要，旧提交的实跑仍只绑定旧配置。

在两个原题均通过后，可在同一监督命令加 `--resource-pressure`（必须恰好两个 `--task`、单轮，且两模板各两个实例能同时准入）。[组合探针](../tests/system/template_resource_pressure.py)用正式发布的模板另开四个实例，验证包数据库/系统文件隔离、运行与构建拒绝、占满数据通道时的控制调用与销毁、构建服务 SIGKILL 后原编号回收及替换实例干净。它使用来宾内的 Debian 包工具和 POSIX shell，不预装原题评分依赖；压力结果另存 `resource-pressure.json`，不改原题成绩。创建与构建意图先留存，清理失败不跳过其余实例，未知资源保留现场。当前只有配置/HTTP/SQLite/取消回归通过；新增组合 KVM 场景待独立运行，不能沿用两题旧实测作为通过证据。

原题运行器与固定 base 诊断入口共用逐试验产物预算。可显式传 `--artifact-max-bytes`（默认 134217728，即 128 MiB）、`--artifact-max-file-bytes`（默认 100663296，即 96 MiB）、`--artifact-max-entries`（4096）和 `--artifact-max-depth`（16，最多 64）；数值须为正整数，单文件可调低但不得超过既有 96 MiB 硬上限或总字节。它们是可配置验收默认值，不是已实测的生产容量。命令事件、来宾下载与另存诊断共同计费，重复/失败不退额；原奖励与阶段单独保留，缺证据不能报告完整通过。边界和排除项见[模块合同](../docs/module-contracts.md)，用例见[验收清单](../docs/testing-plan.md)。监督入口按每试验字节上限 × 题数 × 次数计入磁盘预检（默认 89 题两轮仅此项为 22.25 GiB），另计锁定的原环境上下文、构建日志、报告副本、数据库及收尾余量；来宾实际可用块和运行时配额仍须独立检查。

[实机监督入口](../tests/system/harbor_generic_chain.py)额外记录实际 Firecracker 配置、CPU/内存 cgroup、私有盘身份与模板摘要，并在服务退出前核对精确进程、网络空间持有者、jail/cgroup、构建预留和持久终态。它需要独占 root/KVM 主机、固定工具、完整题库、三个已委派的独占 cgroup 父组（运行池、构建池、`--service-cgroup-parent`）和新 UID 范围；`--repetitions 2` 为全新构建/沙箱重复实验。临时开启转发仅在事前授权后使用 `--temporarily-forward`，退出恢复原值。实机使用同一精确提交的 `cargo build --release -p sandproof-server --bin sandproof-server --locked` 产物，报告须记录二进制摘要和构建 profile；调试构建对大磁盘的摘要校验可能耗尽原题期限。选择 `--all` 可运行全部锁定题目；监督入口根据题目资源及重复次数预留模板存储、工作区和 VM 编号，派发前校验宿主资源与可信构建盘空闲量。每题结束后审计原任务身份和两池回收，明确失败可继续，未知则停止；收尾等待还覆盖迟到发布检查 VM 的有界启动/存活期限，期间保持原控制器运行，不把题目执行期限一起延长；任一选中题失败/未运行时返回非零。私有配置保留到操作者完成独立审计，失败时可用原身份恢复，审计后再精确删除本轮目录。实际结果和提交在 PR/#13 记录，不把脚本存在、替身测试或旧路线成绩算作新链路通过。

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

`fix-git` 的原 GitHub 源仓库匿名访问已返回 404。该题改在原初始化预算内下载题目声明的 `alexgshaw/fix-git:20251031` 初始化层，固定 amd64 清单 `sha256:61e431c00c58df652287aadce5457634d9f9330cfdd153ebdf2802df0d540119` 及层/pack 摘要。[来宾辅助程序](guest_git_fixture.py) 只读取原克隆的两个 Git pack 文件，以镜像 reflog 记录的初始提交重建临时只含源码的仓库，不导入镜像工作树、配置、hooks、后置任务提交或 reflog。原 `setup.sh` 摘要校验后完整执行，仅在该进程内把原 clone URL 映射到临时源码；结束即删除临时仓库。原解答、评分、资源及时限不变，来源证据写入 `fix-git-source.json`。这项恢复的实机结果单列，不覆盖源仓库失效时的失败记录。

HTML 过滤题启用 pytest 的 `-s -v` 与非缓冲日志，保留每个浏览器批次的执行、异常与收尾信息。原测试先杀 ChromeDriver 再调用 quit，实际观察到浏览器及 crashpad 进程持续遗留；该题通过 Selenium 官方 `SE_CHROME_PATH` 设置选择[浏览器启动包装器](guest_chromium.py)。包装器不改变 Chromium 参数，使用 pidfd 观察驱动生命周期；驱动退出后回收独立进程组与自己收养的后代，并记录剩余数量。合成进程回归覆盖驱动死亡和脱离原 session 的后代，原浏览器实测另计。用例、断言和时间限制不变；原测试会吞掉部分浏览器异常，必须检查日志中的超时/浏览器失败，不能只用原 reward 声称所有浏览器场景实际通过。环境接口见 [Selenium Manager 官方说明](https://www.selenium.dev/documentation/selenium_manager/)。

该题另外设置来宾 `net.ipv4.tcp_syn_retries=2`，让失联外部资源的 TCP 建连在原页面加载预算内返回失败；读取并保存实际值到 `tcp-connect-policy.json`，不修改宿主网络参数。相同页面的独立 VM 对照覆盖默认值与该设置，并保留真实弹窗阳性及干净页面阴性检查。此配置不改 HTML、浏览器参数、原评分或 20 秒页面时限，也不替换 HTTP 响应；外部子资源仍可能不可达，浏览器测试完成不表示这些远端内容也已被验证。参数含义见 [Linux 内核文档](https://docs.kernel.org/networking/ip-sysctl.html#tcp-syn-retries-integer)。

MTEB 排行榜题约束仍被原 MTEB 1.38.41 支持的 Torch 2.7.1 与 Transformers 4.57.6，保留完整 PyTorch/CUDA 构建。原无上限依赖会选入更新且更大的 CUDA 13 依赖，使随后原 results 完整克隆写满 10 GiB 私有盘；旧兼容依赖是否足以留出空间必须以原解答复测为准。没有预填结果、改变 Git checkout 目标或缩减评分。 初始化另外补齐 Pillow 11.3.0；[上游问题记录](https://github.com/harbor-framework/terminal-bench-2/pull/53)确认旧 MTEB 丢失该间接依赖后重新构建会导入失败。原 MTEB 版本保持 1.38.41。

该题的 pip 安装使用 `--no-compile`，并设置 `PYTHONDONTWRITEBYTECODE=1`，避免可再生成的 Python 字节码缓存挤占原评分器安装 Python 3.13 所需的空间；Python 源码、原生库与完整 CUDA wheel 均保留。初始化记录实际缓存策略及可用空间，不扩大私有盘、不删除结果仓库或评分输入。

## 可选的参考解答修复

显式 `--oracle-mode repaired` 只对[注册修复](oracle_repairs.py)生效。它先核对原 solve.sh 的完整摘要，在独立输出目录复制完整任务，仅对其中 solution 应用有限修改，不修改固定题库；报告记录模式、原文件与新文件摘要及完整 diff。原 Harbor OracleAgent 按副本任务根目录执行原路径校验，评分仍从原任务读取。评分源码和所有资源/时限保持不变。修正版通过必须单独报告，不能计为原标准解答通过，也不能改写原失败记录。选择修正版时，整批选题须全部已注册；混入未注册题或选择包含未注册题的 `--all` 会在派发任何构建/VM 前拒绝，不能先跑部分题再失败。

- `filter-js-from-html`：原评分器先终止 ChromeDriver，再调用 quit，会留下仍存活的 Chromium 进程树。修正版在 solution 副本中携带并安装已有浏览器监督器，仅对本题评分进程设置 `SE_CHROME_PATH`，使浏览器及其后代随原驱动退出而回收；完整原过滤实现、Chromium 程序/参数、评分源码与任务限额保留。报告记录 helper 摘要和 `verifier_repair_env`，除原 reward 外还要求所有浏览器批次执行成功及对应回收记录完整。安装失败、吞掉的浏览器错误、缺失/异常回收证据均不能算通过；默认原模式不启用。此修复不调整 TCP 或其他网络策略，也不代表当前提交已完成原题实机验收。
- `build-cython-ext`：在原 editable 安装中固定 `planarity==0.6`，保留 pyknotid 0.5.3 所需的 `pos/start/end` 节点属性。[上游记录](https://github.com/harbor-framework/terminal-bench-2/issues/75)与本地评分后诊断均定位为 1.0.0 属性改名导致 `KeyError: pos`；不改 NumPy 2.3.0、原 Cython 编译、仓库测试或外层评分。
- `git-multibranch`：原镜像未安装原解答最后一步要求的 `tmux`；修正版仅在工具缺失时刷新包索引并安装，然后执行完整原 Git/SSH/nginx 配置及原 tmux 命令。安装失败立即停止，下载/安装计入原 agent 时限；已存在的 tmux 不重装。原题库、评分、资源和原失败记录保留，修正版成绩单独报告。
- `mcmc-sampling-stan`：在原系统依赖安装中补齐 RcppParallel 源码构建所需的 `cmake`，并设置 `R_REMOTES_UPGRADE=never`，避免后续 RStan 安装自动升级原解答已固定的依赖；缺失依赖仍正常安装，原 R/RStan 版本选择、模型和采样流程保留。
- `make-doom-for-mips`：在安装交叉编译工具前刷新来宾软件包索引，避免镜像旧索引指向已移除包；刷新或安装失败立即停止，不继续用缺失工具构建。
- `build-pmars`：保留同一组 Debian 构建依赖，使用刷新索引后的仓库版本，移除已不可获取的版本固定；保留原 Debian 源码获取、curses 编译与安装，实际版本由运行时依赖清单记录。
- `protein-assembly`：通过 [RCSB 官方 Data API](https://data.rcsb.org/)的 polymer entity 与 chemical component 字典还原修饰残基及有序母体残基；保留单一聚合物筛选，拒绝无法解析的序列。原 FASTA 将部分荧光团编码为 `X`，无法满足 DNA 翻译约束；修复不嵌入目标序列，原 DNA 优化和评分继续执行。
- `pytorch-model-recovery`：在原解答前清理来宾 pip 下载缓存，使用评分器同版 uv 0.9.5 逐个重新安装原版本的大型 Torch/CUDA wheel，并通过公开 hardlink 接口与缓存共享文件。PyPI 解析结果须包含固定版本和 SHA-256；重新安装前后校验代码及数据摘要、完整版本清单与大文件硬链接数，不编辑 uv 缓存内部结构。仅修正版评分器设置 `UV_LINK_MODE=hardlink`，原评分源码、Torch 构建变体及 10 GiB 私有盘预算保持不变；辅助证据写入来宾 `/logs/verifier/torch-wheel-sharing.json`。组件检查不代替完整实机评分。
- `qemu-alpine-ssh`、`qemu-startup`：原 Bullseye 软件源索引引用的 curl 依赖已返回 404。修正版先核对来宾发行版、密钥环、题目输入和软件源布局，再切换至 [Debian 官方快照](https://snapshot.debian.org/)的 2026-08-24 仓库并刷新索引；仅固定快照允许过期元数据，仍由 Debian 原密钥环验证仓库签名和包摘要。原 QEMU/Alpine 镜像、解答与评分流程保留；原软件源另存来宾评分日志目录。
- `build-pov-ray`：从 ICM 公开镜像下载相同三个 2.2 压缩包，并核对与[上游 PR #77](https://github.com/harbor-framework/terminal-bench-2/pull/77)相同的 SHA-256；其余编译与渲染流程不变。原站对直接下载返回 HTTP 403。
- `caffe-cifar-10`：从 BrainChip 公开镜像下载相同 CIFAR-10 binary 数据，内容已核对[作者公布的 MD5](https://www.cs.toronto.edu/~kriz/cifar.html)，执行时再核对固定 SHA-256。下载、解压、原 Caffe 构建及训练均在 VM 原 agent 预算中，不预填数据或训练结果。
- `crack-7z-hash`：先用原 John 自带完整常用密码字典和原规则运行，并要求 John 确认破解成功；解压密码取自实际破解结果，替代原脚本的答案常量。没有任务专用候选表，也没有跳过破解计算。模式语义见 [John 官方说明](https://www.openwall.com/john/doc/MODES.shtml)。
- `install-windows-3.11`：识别并复用原 supervisor 已启动、参数匹配的 VNC WebSocket 进程，避免重复绑定 8080 导致解答退出 1；不把任意端口监听都当成所需服务。原 QEMU 编译、镜像、启动与监控仍执行，原四项评分不变。
- `torch-tensor-parallelism`：原生栈确认 Gloo 销毁时，主线程持有 GIL 等待工作线程，工作线程释放 tensor 又等待 GIL。修正版让模型持有直接 Gloo 后端引用，使用 [Torch 2.7.0 已有的释放 GIL 的析构绑定](https://github.com/pytorch/pytorch/blob/v2.7.0/torch/csrc/distributed/c10d/init.cpp)完成清理；不改集体通信、模型计算、PyTorch 版本或原评分。完整实跑结果按提交记录在独立的 repaired 模式报告中。
- `mteb-leaderboard`：完整克隆 results 仓库时加 `--no-checkout`，避免先展开当前 HEAD 的大工作树占满 10 GiB 私有盘，再执行原脚本指定的 `c0fd3271442f8c0c839084a109fede85c8514dab` 检出。MTEB 1.38.41 的 `results_repo` 仍被当作克隆源，修正版通过其 `MTEB_CACHE` 接口将缓存根指向现有检出的父目录，避免再次复制完整工作树。保留完整历史与对象，不使用浅克隆、稀疏检出或预填结果；原排行榜计算与评分不变。
