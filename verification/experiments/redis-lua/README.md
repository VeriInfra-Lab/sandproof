# 实际 Redis Lua：原文及正常控制路径

[验证入口](../../README.zh-CN.md) · [V2 计划](../../../docs/formal/02-center-store.md) · [依赖保证](../../../docs/formal/dependencies.md#redis-与-sqlite-的落地合同) · [证明任务 #335](https://github.com/VeriInfra-Lab/sandproof/issues/335)

本批处理生产 `scripts.rs` 的原 READ/CAS 字节，包括 #610 的重确认分支；不改生产 Rust/Lua。完整 CAS 数据效果、codec、耐久、未知提交、恢复和派发仍是 V2 的未解义务。

## 输入与证明分层

`source.py` 只接受两个原始 Rust raw-string 常量及外围注释，拒绝额外声明、重复、动态字符串和超出当前 ASCII 域的输入。`source-map.json` 冻结整个脚本容器及 Rust 调用者摘要。READ 的单 key、部署、格式、限额，以及 CAS 的单 key、部署、格式、旧/新修订、完整新载荷、限额和完整旧载荷，按 `RedisLedger::read/write` 的真实顺序审查；这个来源登记不是 Rust 调用链的机器精化。

生成器只重现原始字符，不翻译 AST 或插入证明。`RawSource.lean` 的字符序列由同一字面量产生；较长序列分成小块，避免字面量展开的栈深度。实际解析在 Lean 中执行，解析成功和原文到 AST 的等式由内核检查。输入导入/引号编码及 Lean 编译器仍属于明确的输入与工具信任链。

`Syntax.lean` 是有限、终止的语法：局部绑定、if/else、return、调用、索引、字段、表数组、not/负号及所需比较/短路运算。复杂度上限明确，范围外语法返回失败；无循环、函数定义、动态赋值、长字符串/长注释或任意 Lua 扩展。词法规则和优先级参考 [Lua 5.1 官方手册](https://www.lua.org/manual/5.1/manual.html)。

`Calls.lean` 检查真实源的调用域：表达式中只有原 key 的 TYPE/HLEN/PTTL/HGET/HSTRLEN、string.len 和 tonumber；语句调用只允许同一 key 的 HSET。拒绝动态命令、其它 key 和受保护全局名遮蔽。固定 Redis 环境与原始 KEYS/ARGV 对象由 A11 及实际接入提供，不能把任意同名函数当作这些原语。

`Flow/Paths` 描述条件取任意分支、保留提前返回的正常控制路径，并证明上界和枚举覆盖。`Scripts` 将当前输入绑定到该模型：READ 没有语句写调用；CAS 每条正常路径最多一次 HSET，返回 0 恰一次，返回非零正常拒绝码则没有写调用。

这些结论是**原文绑定的正常控制路径性质**。当前没有完整表达式/字节值求值、Redis 字段后态及 Lua 实现语义的精化定理，不能据此宣称 `lua_cas_refines`、一般 Lua 解析器正确性或 RedisLedger 已验证。读成功也不能推出规范修订、合法 codec 状态或耐久提交。panic、引擎错误、分配失败、断线、取消和崩溃不由正常路径定理消除。

## 同性质错误版与真实引擎

`source.mutants` 从原 CAS 精确构造两份实际 Lua：重确认分支提前返回而漏写新修订；在守卫前增加写入。Lean 首先证明两者可以解析，再证明正确版同一个路径合同不成立；提前写入版还违反原写入上界。它们不是失败的解析或翻译冒充反例。

`native.py` 为本轮启动新的 Redis 7.2.16，仅使用 0700 私有 Unix socket，无 TCP 监听、无业务数据或既有服务。25 个有限功能用例检查拒绝次序、完整字段/载荷、满 u64 文本、二进制载荷、TTL 和真实 HSET 次数；两份同字节错误版先 SCRIPT LOAD 成功，再由原正确版同一检查拒绝。另一个写后主动报错用例观察写入仍存在。有限引擎回归不是所有输入的定理，也没有运行 AOF 耐久/断电或产品 VM 验收。

## 复现

```sh
bash scripts/python-entry.sh verification/experiments/redis-lua/check.py
bash scripts/python-entry.sh verification/experiments/redis-lua/source.py \
  --output artifacts/new-lua-input-import
bash scripts/python-entry.sh verification/experiments/redis-lua/native.py \
  --redis-server "$SANDPROOF_REDIS_SERVER" --output artifacts/new-lua-native
bash scripts/verify-redis-lua.sh
```

独立生成目录必须不存在。生成后应与正式 RawSource 比较，不能手改生成物或自动接受新生产源。正式模块入口要求本批 ning 原生证据；整体仍执行原 `verify-controller.sh` 及开发指南基础检查。局部构建不作为整套通过，执行与复用分别记录。工具版本、占位/依赖审计及原失败判据保持。
