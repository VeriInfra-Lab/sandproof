# Terminal-Bench 评测

[文档目录](../docs/README.md) · [项目蓝图](../docs/project-blueprint.md) · [验收清单](../docs/testing-plan.md)

## 当前回归：冻结的 Terminal-Bench 2.0 兼容基线

日常晋升使用 `ci` 两题预装变体：使用[制备配方](ci-fixture.Dockerfile)、仓库外不可变干净模板和固定本地评分环境，原解答/测试断言不变；公网关闭、每题全新沙箱。[评分启动器](ci_verifier.py)只替换已审核的公共安装段，不覆盖未知评分脚本。此模式不代表原环境、原启动脚本或本轮通用模板构建通过。部署配置看[CI 运行环境](../docs/ci.md)，正式启用与通过判据看[验收清单](../docs/testing-plan.md)。原四题、十六题和全量入口仍供扩展验收，题集仍由配置选择。

当前采用 2.0 兼容回归，2.1 原题全量为发布前待办。[2.0 题集配置](terminal-bench-2.0-suites.json)使用既有完整源码锁和通用 OCI 模板链，提供四题 `smoke`、十六题 `regression`、`full` 及自选名单；不调用旧固定 base 配方。显式 `oracle_modes` 只启用已有且原脚本摘要匹配的修补，批次遇到不同解答模式时拆分，原评分/资源/时限不变；报告逐题保留 original/repaired。配置与候选固定后不在该验收身份内删换失败题，兼容回归不是原题榜单成绩，也不是形式化证明。

人工诊断遇到外部网络超时可结束该次运行并另选题，不排查代理、不新增网络修复任务；旧检查仍保留实际非成功状态，新名单产生独立身份。不能通过换题把原固定范围、完整 89 题或清理未知改成成功。本次少量实测也不核销全部兼容题的同版本验收。

## 发布前待办：Terminal-Bench 2.1

最终全题集目标为 **Terminal-Bench 2.1 全部 89 题**，来源为[官方题库](https://github.com/harbor-framework/terminal-bench-2-1)与[Harbor Hub 数据集](https://hub.harborframework.com/datasets/terminal-bench/terminal-bench-2-1)。官方使用的数据集名称为 `terminal-bench/terminal-bench-2-1`；正式执行须冻结具体发布摘要与源码提交，不直接跟随 `latest` 或 `main`。[官方发布说明](https://github.com/harbor-framework/terminal-bench-docs/blob/main/content/blog/terminal-bench-2-1.mdx)说明此次修订处理了外部依赖漂移、资源预算和题意/评分不一致等问题；不据修复数量推断本项目旧失败题已全部解决。

范围和批次大小通过参数/配置选择。[题集目录](terminal-bench-2.1-suites.json)提供原八题 `smoke`、扩展十六题 `regression` 和按完整锁生成的 `full`；也支持事前批准的自定义名单。十六题面向编译、Python/异步、文件、Git/HTTP/RPC、数据库、密码工具、OCR 与较大计算资源的工程回归，不是官方小数据集，也不是榜单成绩的统计替代。名单与配置在运行前冻结，失败后不换题；不同范围、版本和提交不拼成 89/89。

[2.1 完整锁](terminal-bench-2.1.lock.json)固定官方发布期间的源码提交 `c5ee500c185224c97cd6caff7866a990a0057f41`、全部 89 题输入和原始资源/时限。[源码清单](terminal-bench-2.1.registry.json)由该 Git 树产生，不是已验证的 Harbor Hub 下载包或发布包摘要；两者不能混称。固定 Harbor/SDK 不升级，原 Harbor 可直接加载 1.1 配置。短任务 ID 仅用于本地目录与报告，原 `[task].name` 保存在锁内；不修改原任务文件。2.0 本地解答/环境修补不能用于 2.1。

```sh
git init artifacts/terminal-bench-21/source
git -C artifacts/terminal-bench-21/source remote add origin https://github.com/harbor-framework/terminal-bench-2-1.git
git -C artifacts/terminal-bench-21/source fetch --depth=1 origin c5ee500c185224c97cd6caff7866a990a0057f41
git -C artifacts/terminal-bench-21/source checkout --detach FETCH_HEAD
uv run --locked --project benchmarks python -m benchmarks.check_harbor \
  --registry benchmarks/terminal-bench-2.1.registry.json \
  --source artifacts/terminal-bench-21/source \
  --lock benchmarks/terminal-bench-2.1.lock.json \
  --report artifacts/terminal-bench-21/inputs.json
```

实机单批入口通过 `--lock` 指定独立版本；自动监督的 `dataset` 选择 2.0 兼容或 2.1 原题，入口与私有配置见[CI 运行环境](../docs/ci.md)。当前默认示例为 2.0 兼容，2.1 保留但不阻塞本次功能交付；转入 2.1 后须逐项复核并移除不再需要的补丁，不自动叠加 2.0 修补。**输入检查、组件通过、KVM 通过与自动 CI 成功/失败实测必须分别记录。** 下文旧固定 base 配方只供诊断，不用旧成绩背书任何新候选或 2.1。


后续迁移由 #13 完成，判据见[验收清单](../docs/testing-plan.md)：

1. 冻结 2.1 的发布摘要、官方源码提交、完整任务 ID 及 environment/instruction/metadata/solution/tests 的逐文件摘要；保留独立的 2.0 锁和报告。
2. 核对固定 Harbor 是否能加载该发布；确需改变 Harbor 获取接口或联合依赖时单独记录版本与回归，不在本批自动升级 SDK 或工具链。
3. 逐题比较 2.0→2.1 的输入、原标准解答、原评分、CPU/内存/磁盘和阶段时限，重新审计模板与宿主预算；不得沿用下文 2.0 的资源分布推断 2.1 可运行。
4. 默认运行 **2.1 自带的标准解答和评分**。现有本地 2.0 修复不自动叠加到 2.1；确需额外修订时重新核对输入并单列模式和差异，不因旧题名相同复用通过结论。
5. 先用少量任务打通全新沙箱→初始化→标准解答→评分→销毁与精确残留检查，再执行固定 2.1 全集。2.0/2.1、原始/本地修订和环境变体结果分开，不将迁移计划、未运行或旧成绩写成 2.1 通过。

## 已有工具：Terminal-Bench 2.0 历史基线

预检入口实现 2.0 完整题集的源码锁定、资源审计、Harbor/SDK 联合依赖与原任务加载检查，不执行任务代码或启动沙箱。独立的本地执行入口见下文，必须连接已配置并由外部监督器审计的真实服务。历史范围为固定 Harbor registry 中 `terminal-bench@2.0` 的全部 **89 题**；不能在失败后删除任务或把未运行计为通过。

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

功能入口为[#123 通用模板链](https://github.com/ningZhang-cs/sandproof/issues/123)。自助构建服务和下述 #13 通用执行入口已实现；实现存在不等于全部 89 题验收通过。下述固定 base 手动诊断是旧路线的局部实验，不是最终产品接口，也不能代替通用构建/转换能力。后续按[验收清单](../docs/testing-plan.md)保留原解答与原评分，全部 89 题不因失败删除；修订解答和跨提交汇总必须单独报告。

现有诊断工具已实现全部任务的显式初始化配方、固定 base 指纹和 Harbor 薄适配；每题是否初始化成功、标准解答与原评分器是否完成，仍以绑定提交和环境的实际报告为准。配方清单不是通用 Dockerfile 解释器，也不表示任意上游镜像标签都可作为模板。

通用模板链的各类实际任务仍须逐批验收。缺少能力的任务保留在完整 89 题报告里，先记录阻塞再补齐；同一智能体的 E2B 对照按验收清单执行。在线 E2B 与模型运行使用另行配置的凭据和预算，不由这个本地工具触发。

## 通用模板标准解答入口

[通用入口](run_generic.py)只替换 Harbor 的模板准备：提交自有构建 API，等待不可变模板，再使用原 `AsyncSandbox`、`OracleAgent` 和 `Verifier`。不使用 `task-recipes.json` 或预制任务镜像替换，不调用 E2B 云模板服务。默认使用原解答；通用入口与实机监督均可显式传 `--oracle-mode repaired`，仅运行下述已注册修复，报告独立标记模式，不计为原解答通过。默认 `--source-kind oci` 与固定 Harbor 优先使用 `docker_image` 一致；显式 `dockerfile` 则打包原 `environment/`，原样交给标准 BuildKit。两条来源在报告中分开记录，不将 Dockerfile 重建声称为上游预构建标签的相同字节。客户端不调用固定 SDK 的 Dockerfile 解析器，因此多阶段不受其限制。

另可显式选择 `--source-kind oci-pip-cache-pruned`，仅适用于[登记环境变体](environment_repairs.py)中的两道 MTEB 题，须匹配任务文件摘要与原镜像引用。它生成只有固定摘要 `FROM` 和一次缓存清理命令的 Dockerfile，通过同一自助构建 API 与隔离 BuildKit 执行；不重装依赖，不替换 Torch/CUDA、模型、解答或评分，也不增加原任务 CPU、内存、总盘和时限。[来宾清理器](guest_pip_cache.py)只移除 `/root/.cache/pip`，拒绝路径链接、异常文件和未知缓存入口，保留模型缓存及已安装文件。构建意图保存原镜像引用、固定基础摘要、helper/Dockerfile/context 摘要和清理路径；单题及整批报告明确 `original_environment=false`。即使选择原解答并评分通过，这也是环境变体结果，不能计为原 OCI 通过。原环境失败记录必须保留；未登记题或混合选题在派发前拒绝。组件测试不代表变体已通过整题实机验收。

构建诊断按服务返回的原始字节偏移分页保存到 `build-log.json`，总上限 2 MiB、读取期限 30 秒，JSON 展开后的文件也计入逐试验产物预算。`complete` 仅表示已到达终态构建日志末尾，超限记录 `truncated`；读取失败保留已有页和错误类型，不改写原构建终态或重新提交构建。

`filter-js-from-html` 的原评分器会吞掉部分浏览器错误并给出成功奖励。通用链只在评分阶段设置 `PYTEST_ADDOPTS=-s -v` 和 `PYTHONUNBUFFERED=1`，保存逐批浏览器日志，并额外检查批次数、连续批次编号、实际完成的无弹窗检查及被吞掉的超时/浏览器错误；缺日志或检查失败不能报告整题通过，原 reward 与 oracle/verifier 状态仍保留。这不修改原镜像、解答、评分源码、资源或时限，也不安装固定 base 的浏览器包装器。通用链的 `browser_lifecycle_audit=not_instrumented` 明确表示未观察每批浏览器进程回收，最终 VM 资源仍须外部审计；它不是浏览器超时的修复或实机通过证据。

通用入口及实机监督可显式加 `--failure-diagnostics`，在原评分已失败后、销毁原实例前执行[已登记的来宾诊断](failure_diagnostics.py)。`build-cython-ext` 诊断会核对原解答已克隆的 pyknotid 提交和测试文件，直接展开原先被外层断言隐藏的内部 pytest 输出，并记录 Python/依赖版本。额外诊断最多 60 秒，共享逐试验产物预算，不编辑测试或安装依赖；输出为独立的 `failure-diagnostic.txt/json`，不更新原 reward、解答或评分状态，取消和错误仍须清理原实例。它属于评分后的独立复现，不延长原解答/评分期限，未实际运行不能记为已定位或已修复。

`gpt2-codegolf` 的评分后诊断仅用于 original 模式，只使用原镜像已有的 Bash、GCC 和基础命令，不依赖系统 Python。它核对原 `gpt2.c` 的固定摘要，记录来宾 overcommit/内存信息，在临时副本中为两次原内存分配加入空指针检查，随后立即退出；分配失败分别返回 77/78，记录 errno，避免诊断再次解引用空指针。副本不运行模型推理，退出时删除临时目录并复核原文件未变；不修改内存策略、资源限制或评分结果。该探针同样需要显式启用 `--failure-diagnostics`，受 60 秒和逐试验产物预算约束。

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

构建意图及幂等键先落盘；构建、创建或清理未知时停止后续任务，不用新身份重跑。原 Dockerfile 来源只上传锁定的环境文件，登记环境变体只上传上述生成的 Dockerfile；solution/tests 由原 Harbor 在解答/评分阶段上传，启动时检查保留目录不存在。报告保留原失败及全部 89 题，`--all` 才选择全集；下载依赖未完全锁定会明确标注。工具退出 0 仅表示本次执行检查通过，`kill_confirmed` 是 SDK 回执，`exact_cleanup=not_audited` 不是宿主回收证据。

停止请求耗时单独记录为 `cleanup_seconds`。SDK 停止异常继续记为 `cleanup_unknown`，并保留异常类型及可用的整数 HTTP 状态码 `cleanup_http_status`；不记录异常正文或凭据，不据此自动重试或把清理未知改成通过。字段缺失表示没有取得有效状态码，不能推断为网络超时；随后精确审计确认销毁也保留首次异常。

通用 KVM 监督入口使用[统一验收配置](template-service-budgets.json)及[配置生成/预检](../tools/template_acceptance.py)，运行池传输和寿命字段直接复用[已有任务配置](local-service-budgets.json)。同时计入构建池和运行池的完整 CPU/VMM 内存额度、额外存活私有盘、发布产物、构建工作区、证据和清理余量；预检使用当前 CPU affinity、可用内存和剩余磁盘。服务/relay 的份额由[显式服务 cgroup 入口](../tools/service_cgroup.py)落实；控制服务与构建监督服务及其子进程共用限额，VM 使用各自独立分组。仅生成配置仍标记为未观察到执行，监督入口在核对实际服务 PID 归属后才记录已施加上限；当前数值仍不是已实测的生产总预算。报告保存实际生成配置及来源摘要，旧提交的实跑仍只绑定旧配置。

分批复跑可在实机监督入口显式加 `--capacity-scope all`：构建、模板、工作区、证据和 VM 编号配额统一按锁定的完整 89 题及重复次数计算，`--task` 仍只选择实际执行的题目。默认 `selected` 保持按选题派生容量。模式与环境来源先对实际选题校验，规划范围不会允许未登记的修复或混合环境变体；原题资源和阶段时限不变。`capacity-basis.json`、逐轮输入及报告记录选题和规划题集，可与 `capacity.json`、`acceptance-budgets.json` 一起逐批比较。完整容量模式下两题压力探针复用该容量，仍核对实际选中的两题资源；容量必须覆盖其四个构建名额。规划 89 题不会把未执行的题计为通过。

同一组资源上限的有限批次可额外指定 `--capacity-jobs N`（仅与 `--capacity-scope all` 一起使用）：每批保留的构建数、模板数、工作区历史、证据和编号按 N 预留，每个名额按完整题集中最大的运行盘及环境上下文计费；CPU、内存、构建时限、单题运行盘及构建暂存仍取完整题集需求。所选题数乘重复次数，加上压力探针另需的两个构建名额，不得超过 N；超额在派发前拒绝，不截断选题。未指定时保留原先完整题集的配额。报告额外记录 `capacity_basis.retained_job_limit`，不能把 N 个保留名额称为同时保留 89 个模板的容量。

有界 physical profile 可显式提供更大的两池私有盘和清理预留，用于覆盖宿主元数据及控制器 DB/WAL/SHM/检查点。通用监督在宿主空闲量预检前将这些额度同时传入两池配置和 `acceptance-budgets.json`，并将相对原题推导下限的增量逐项计入宿主磁盘需求；任一池的总盘、租户盘或清理预留低于该下限均拒绝，不能以增加另一个字段抵消。原题形状、时限不变，未指定有界策略时预算不变。Rust 仍需核对实际元数据边界和完整 physical profile 后才能激活，静态配置通过不代表实机启动通过。

例如各批统一使用 `--capacity-scope all --capacity-jobs 4` 时，每批最多选四题，完整报告仍须覆盖锁定的全部 89 题，含最后不足四题的批次和所有失败。每批在前一批精确资源审计完成后启动独立部署；服务配置和预算相同，部署身份及根目录须独立绑定，不能原地改写完整 physical profile。此选项不实现模板垃圾回收、不删除历史报告，也不绕过有界模式的双倍磁盘/ inode 预检；这些分批结果不能作为单个持续运行部署完成 89 题或长期有界运行的证据。

两个模板均为 Ready 且原题精确回收审计通过后，可在同一监督命令加 `--resource-pressure`（必须恰好两个 `--task`、单轮，且两模板各两个实例能同时准入）。[组合探针](../tests/system/template_resource_pressure.py)用正式发布的模板另开四个实例，验证包数据库/系统文件隔离、运行与构建拒绝、占满数据通道时的控制调用与销毁、构建服务 SIGKILL 后原编号回收及替换实例干净。它使用来宾内的 Debian 包工具和 POSIX shell，不预装原题评分依赖；压力结果另存 `resource-pressure.json`，不改原题成绩。创建与构建意图先留存，清理失败不跳过其余实例，未知资源保留现场。当前只有配置/HTTP/SQLite/取消回归通过；新增组合 KVM 场景待独立运行，不能沿用两题旧实测作为通过证据。

压力探针只使用请求窗口，通过普通客户端先持久保存请求窗口、幂等键和原参数，再提交一次构建；预期容量拒绝也使用独立的有效窗口。观察私有构建 VM 时只读取该构建已经提交的私有窗口；重启回收后核对持久化的原窗口、原实例和结算确认，并继续执行宿主资源审计。旧模式输入明确拒绝，窗口失败和丢失回复均不自动降级或重派。

长题可能运行到构建结果的查询保留期之后。完成独立资源审计后，有界监督沿用运行器已观察到的同一 buildID 的 Ready／Failed 终态，并以 `build_observation=saved_terminal` 标明来源；仍核对已发布模板的摘要，不要求已经退休的构建查询记录继续存在。未观察到终态或身份不匹配时不能借退休认定成功，仍须查询原 buildID，缺失记录保持未知。此处理不延长窗口、保留期或原题时限，也不改变原题成绩。

原题运行器与固定 base 诊断入口共用逐试验产物预算。可显式传 `--artifact-max-bytes`（默认 134217728，即 128 MiB）、`--artifact-max-file-bytes`（默认 100663296，即 96 MiB）、`--artifact-max-entries`（4096）和 `--artifact-max-depth`（16，最多 64）；数值须为正整数，单文件可调低但不得超过既有 96 MiB 硬上限或总字节。它们是可配置验收默认值，不是已实测的生产容量。命令事件、来宾下载与另存诊断共同计费，重复/失败不退额；原奖励与阶段单独保留，缺证据不能报告完整通过。边界和排除项见[模块合同](../docs/module-contracts.md)，用例见[验收清单](../docs/testing-plan.md)。监督入口按每试验字节上限 × 题数 × 次数计入磁盘预检（默认 89 题两轮仅此项为 22.25 GiB），另计锁定的原环境上下文、构建日志、报告副本、数据库及收尾余量；来宾实际可用块和运行时配额仍须独立检查。

[实机监督入口](../tests/system/harbor_generic_chain.py)要求 `--bounded-policy <私有策略文件>`，不接受不完整物理/容量配置或旧运行模式。独立任务入口不再接受 `--build-mode`，只提交窗口请求；服务缺少窗口接口时明确失败，不自动降级。该监督入口额外记录实际 Firecracker 配置、CPU/内存 cgroup、私有盘身份与模板摘要，并在服务退出前核对精确进程、网络空间持有者、jail/cgroup、构建预留和持久终态。它需要独占 root/KVM 主机、固定工具、完整题库、三个已委派的独占 cgroup 父组（运行池、构建池、`--service-cgroup-parent`）和新 UID 范围；`--repetitions 2` 为全新构建/沙箱重复实验。临时开启转发仅在事前授权后使用 `--temporarily-forward`，退出恢复原值。实机使用同一精确提交构建的 `sandproof-center` 与 `sandproof-host`，候选清单绑定两者摘要及构建 profile；调试构建对大磁盘的摘要校验可能耗尽原题期限。准确的构建和参数由[CI 准备入口](../tools/ci_e2e_prepare.py)与脚本帮助维护，不使用旧公开服务二进制。选择 `--all` 可运行全部锁定题目；监督入口根据题目资源及重复次数预留模板存储、工作区和 VM 编号，派发前校验宿主资源与可信构建盘空闲量。每题结束后审计原任务身份和两池回收，明确失败可继续，未知则停止；收尾等待还覆盖迟到发布检查 VM 的有界启动/存活期限，期间保持原控制器运行，不把题目执行期限一起延长；任一选中题失败/未运行时返回非零。私有配置保留到操作者完成独立审计，失败时可用原身份恢复，审计后再精确删除本轮目录。实际结果和提交在 PR/#13 记录，不把脚本存在、替身测试或旧路线成绩算作新链路通过。

另有[预选两题](diagnostic-tasks.json)的手动诊断链路，由[实机脚本](../tests/system/harbor_diagnostic_chain.py)执行：要求锁定的 Harbor/E2B 联合环境、固定提交的原题源码、经核对的 Ubuntu 24.04 10 GiB base、隔离 KVM 宿主与预先批准且在整轮不变的公网 IPv4 策略。每题全新沙箱，先核对 CPU/内存/磁盘和空初态；来宾固定使用公网 DNS `1.1.1.1`，因为现有 base 的宿主本地 DNS 桩不可在 VM 内使用，此处是与原 Docker 环境的已知差异。`polyglot-c-py` 只在来宾内执行原 Dockerfile 的 `apt update && apt install -y gcc`；随后依次上传并运行原 `solution/solve.sh`。原评分器所需的 Python 3.13.9 固定运行时在解答结束后传入并核对哈希，避免原评分器访问其固定下载地址超时；运行时来源和哈希见预选清单，这是另一项明确的环境差异。接着上传并运行未改动的原 `tests/test.sh` 与 `test_outputs.py`，读取原 `reward.txt`。评分后销毁并检查数据库终态、精确进程/网络/磁盘与 base 指纹。失败保留本轮私有目录诊断，不将失败题从预选名单删除；不使用付费模型，不宣称原 Harbor 模板 API 或完整榜单兼容。 这条两题链路的运行时预填与报告独立于下方全题集入口；不能用两题通过代表其余 87 题，也不替代原 Harbor OracleAgent/Verifier 的全题集接入。

完整容量模式还须同时提供 `--run-root <事前指定的新目录>`、`--namespace <16位小写十六进制>`、`--build-namespace <独立身份>`；两个身份的前六位也须不同。私有策略中的数据库、节点目录、jail、构建工作区和注册库必须逐项绑定这些原路径，不会为了测试搬迁或改写 profile。协调文件须在该新目录的直接子路径中；入口在独占创建空目录后，以排他方式持久化私有空文件，已有目录、文件或符号链接均不接管。创建目录前先按有界模式的完整 u64 VM 编号检查 socket 路径长度，不沿用 UID 槽位上限。入口先静态核对配置，两个 Rust 池启动并激活原 profile 后再让 Python 监督服务加入原租约；静态预检不是启动授权。自助模板探针可用 `--builder-rootfs` 指定独立构建基盘，磁盘字段取完整 profile 的原额度，而非沿用固定 24 GiB。首次启用需要空且静止的受管目录；已有部署、部分策略或身份不符直接拒绝；不再提供 `--legacy-unbounded` 或无策略的成功启动入口。

最终服务预算的两项慢流/内存收尾使用[独立监督入口](../tests/system/server_slow_boundaries.py)，与其他系统探针共用同候选中心/私有宿主、固定 Redis/mTLS 和完整服务预算参数；准确必填参数以该入口 `--help` 为准，不保留旧公开服务启动参数。它不运行题目或联网；范围、原时间边界与精确清理规则只以[验收清单](../docs/testing-plan.md)为准，不用旧的缩短空闲配置替代最终配置。


固定镜像的 `benchmarks.run_local` 与 `tests/system/harbor_diagnostic_chain.py` 仅供显式诊断，不作为通用模板产品或 2.1 验收入口。任务修订、逐轮成绩与失败归因查对应 PR/私有报告，不在长期文档维护第二份题目修复台账。

手动诊断脚本也使用完整的物理位置配置，必须通过 `--cgroup-parent` 指定本轮空且已委派的父组。原题的 1 CPU、2048 MiB 来宾内存、10 GiB 磁盘和原时限不变；不能用缺省宿主管理或旧编号映射运行探针。参数以脚本 `--help` 为准。
