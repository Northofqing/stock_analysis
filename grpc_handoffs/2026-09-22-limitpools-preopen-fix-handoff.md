# LimitPools 盘前路由修复交接 — 2026-09-22

本文交接盘前涨停池（LimitPools）09:15–09:25 窗口失败的**根因、修复、部署与复测安排**。它是
[2026-09-22-review-upstream-failures-upstream-reply.md](2026-09-22-review-upstream-failures-upstream-reply.md)
中 LimitPools 一节的后续状态；该回复里 R-08/R-09、HistoricalBars 与健康记录三项的结论不变，本文不重复。

## 一句话状态

修复已编译、已部署、已用等价代码路径实测通过；**2026-09-23 09:13 的盘前窗口探测已排定，结果尚未产生**。

## 根因

`execute_limit_pool_route` 对任何「不可重试且非 scope 拒绝」的失败直接终止路由。Eastmoney 是
LimitPools 的**第一个**登记候选，其涨停池适配器要求源端 `data.qdate` 等于请求日。盘前源端 `qdate`
仍是上一交易日，于是第一个候选抛出 `FailedPrecondition`，路由在**候选 1** 停下 ——
Tonghuashun 与 HithinkFinance 根本没有被调用。开盘后 `qdate` 变为请求日，守卫不再触发，路由自愈。

这是**候选自身的**事实，不是关于请求的事实；BR-059 早已规定关于候选的事实应推进到下一个候选。

2026-09-22 窗口的服务端记录共 35 条，全部为
`stage=provider_route_stopped attempt_count=1 attempts=Eastmoney:source_precondition`。

## 修复内容

- `crates/magic-market-service/src/lib.rs`：路由新增第三个推进条件 —— 候选返回
  `ServiceError::FailedPrecondition` 时不再终止，记为有界的 `rejected`/`source_precondition`
  尝试后**推进**。日期守卫本身不改，Eastmoney 仍无法证明非当日的池子。
- `crates/magic-market-grpc-server/src/app.rs`：`ServiceError::Unavailable` 分支此前不写任何记录，
  现补写有界 `service_failure` 记录 —— 即回复中承诺的「健康记录」那一项。

提交：`4d41462`（修复）、`53236d6`/`2ff20e7`/`b8b331a`（窗口捕获证据）、`3b65fb5`（部署后实测）。

## 部署状态

服务已于 2026-09-22 23:51:27 用**带修复**的二进制重启：

```
server pid=49960 started=2026-09-22T23:51:27+08:00
sha256=7EF4CA4A0696E7B93DEF4A91385ED085429847F8E4115BDE4AC33AEA9419F948
```

此前线上二进制编于 2026-09-21 22:21，**不含**修复 —— 这是 2026-09-22 窗口仍会失败的原因，
也是本次必须先重新部署的原因。

## 已完成的实测（等价代码路径）

盘前窗口不常驻，但守卫是**日期分歧**而不是时钟，因此用过去交易日 `2026-09-21` 可按需复现同一路径。
2026-09-22 23:53 实测，一次抓取（未固定来源 1 次 + 各候选固定 1 次）：

```
exit=0  target=route          selected=Tonghuashun complete=true records=50 source_at=2026-09-21
exit=73 target=Eastmoney      code=FailedPrecondition  message=Eastmoney protocol error: limit-pool source qdate 2026-09-22 does not match requested date 2026-09-21
exit=0  target=Tonghuashun    selected=Tonghuashun complete=true records=50 source_at=2026-09-21
exit=0  target=HithinkFinance selected=HithinkFinance complete=true records=50
```

这一次抓取同时含两半：

- 守卫**照样触发** —— Eastmoney 单独调用仍 `FailedPrecondition`，并写出
  `service_failure stage=source_precondition_failed` 记录；
- **未固定来源的路由不再停在它上面** —— 返回 OK 并由能证明该日期的候选作答。

该次调用**没有**写出任何 `provider_route_failure`；修复前这类调用每次写一条。

## 2026-09-23 09:13 的复测

- 任务：Windows 计划任务 `magic-market-limitpools-preopen-probe`，2026-09-23 09:13 触发，
  运行至 09:27（两端各留 2 分钟包住 09:15–09:25）。
- 内容：每 20 秒一次未固定来源调用；每第三轮对每个候选各固定调用一次；并抄录服务端 stderr 增量。
- 输出：服务本机 `target\runtime\probe\2026-09-23-window.log`。
- 读法：看服务端记录的 `stage=` 与 `attempts=`。

| 观测 | 含义 |
| --- | --- |
| `attempts=Eastmoney:source_precondition,Tonghuashun:source_precondition`（或直接成功） | 人为的提前停止已消除，修复在真实窗口成立 |
| 仍是 `provider_route_stopped attempt_count=1 attempts=Eastmoney:source_precondition` | 修复未生效或未部署 |
| `provider_route_exhausted`，attempts 列出全部候选 | 路由已走完，但盘前没有候选能证明当日池子 —— 这是**正确且不变**的失败，原因在候选就绪度，不在本缺陷 |

注意：该任务注册为「仅在登录时运行」。若 09:13 前注销登录，它不会执行，且当日无第二次机会。

## 对下游可见的变化

客户端可见字符串有一处变化，**且仅在请求仍然失败时**：`provider_route_stopped` →
`provider_route_exhausted`（后者更准确，因为路由确实走完了所有候选）。`retryable` 标志、gRPC code、
reason code 词表、请求/记录 schema 与 `magic-error-detail-bin` trailer 形状**均不变**。

## 本交接不宣称

- **不宣称 09:15–09:25 窗口已恢复。** 上述实测用过去日期替代的是同一段代码路径，但不是同一个时钟。
- 不宣称 Tonghuashun 或 HithinkFinance 能在 09:15 证明**当日**池子 —— 当日池子彼时可能尚不存在。
  若两候选都无法证明，路由会 `exhausted`，这是正确且不变的失败。
- 只宣称：路由不再被第一个候选的源端前置条件挡住。