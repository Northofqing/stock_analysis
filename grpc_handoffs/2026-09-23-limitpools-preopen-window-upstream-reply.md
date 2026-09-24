# LimitPools 盘前窗口复测结果 — 2026-09-23

本文由 `target\runtime\probe\report-window.ps1` 在窗口捕获结束后自动生成，读的是捕获文件
`C:\DevelopFile\magic-market-data-rs\target\runtime\probe\2026-09-23-window.log` 全文。它是 [2026-09-22-limitpools-preopen-fix-handoff.md](2026-09-22-limitpools-preopen-fix-handoff.md)
里「2026-09-23 09:13 的复测」那一节的结果，也是 2026-09-22 窗口捕获留下的可证伪复跑的答案。

## 一句话状态

**修复在真实窗口成立** —— 路由未停止，2026-09-23T09:25:08.033+08:00 起返回非空批次；没有任何 `provider_route_stopped`。

## 捕获边界与二进制

```text
2026-09-23T09:13:03.412+08:00 ===== capture start minutes=14 interval=20s trading_date=2026-09-23 kind=Upper limit=50 =====
2026-09-23T09:13:03.855+08:00 server pid=49960 started=2026-09-22T23:51:27+08:00 sha256=7EF4CA4A0696E7B93DEF4A91385ED085429847F8E4115BDE4AC33AEA9419F948
2026-09-23T09:13:03.895+08:00 server log size before=6059 file=C:\DevelopFile\magic-market-data-rs\target\runtime\logs\grpc-server.stderr.log
2026-09-23T09:27:26.709+08:00 server log size after=13948
2026-09-23T09:27:26.713+08:00 ===== capture end =====
```

## 服务端记录汇总

| 项 | 值 |
| --- | --- |
| 路由记录总数 | 0 |
| `provider_failure` 记录 | 1 |
| `Eastmoney` 的失败记录 | 12 |
| `HithinkFinance` 的失败记录 | 1 |
| `Tonghuashun` 的失败记录 | 12 |

## 未固定来源调用的读数

| 项 | 值 |
| --- | --- |
| 未固定来源调用 | 42 |
| 其中 exit=0 | 42 |
| 其中 exit=0 且 records>0 | 7 |
| 其中失败 | 0 |
| 首次 exit=0 | 2026-09-23T09:13:04.700+08:00 |
| 首次 records>0 | 2026-09-23T09:25:08.033+08:00 （selected=Eastmoney） |

## 判定依据

读法取自 2026-09-22-limitpools-preopen-fix-handoff.md：

| 观测 | 含义 | 本次是否出现 |
| --- | --- | --- |
| `provider_route_stopped attempt_count=1 attempts=Eastmoney:source_precondition` | 修复未生效或未部署 | 否 |
| `provider_route_exhausted`，attempts 列出全部候选 | 路由已走完，盘前无候选能证明当日池子 —— 正确且不变的失败 | 否 |
| attempts 列出 ≥2 个候选 | 人为的提前停止已消除 | 否 |

## 原始路由记录（逐条，未编辑）

0 条：

```text
```

## 其它服务端记录（逐条，未编辑）

```text
ts=2026-09-23T01:26:31.3189461Z level=ERROR target=grpc_server event=provider_failure stage=provider_response_invalid request_id="win-HithinkFinance-40" operation=limit_pools provider="HithinkFinance" provider_reason="category=decode"
```

## 未固定来源调用的逐条读数

```text
2026-09-23T09:13:04.700+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790125967690
2026-09-23T09:13:25.961+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790125997709
2026-09-23T09:13:46.199+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126017924
2026-09-23T09:14:06.458+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126028899
2026-09-23T09:14:27.988+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126048464
2026-09-23T09:14:48.220+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126059533
2026-09-23T09:15:08.422+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126078511
2026-09-23T09:15:29.902+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126120123
2026-09-23T09:15:50.148+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126138988
2026-09-23T09:16:10.355+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126150213
2026-09-23T09:16:32.186+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126180585
2026-09-23T09:16:52.388+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126211112
2026-09-23T09:17:12.636+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126211112
2026-09-23T09:17:33.979+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126230623
2026-09-23T09:17:54.157+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126272621
2026-09-23T09:18:14.403+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126272621
2026-09-23T09:18:35.826+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126302672
2026-09-23T09:18:56.078+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126332693
2026-09-23T09:19:16.284+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126352021
2026-09-23T09:19:37.765+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126352021
2026-09-23T09:19:57.963+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126393116
2026-09-23T09:20:18.192+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126412226
2026-09-23T09:20:39.618+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126423269
2026-09-23T09:20:59.820+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126442265
2026-09-23T09:21:20.021+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126453423
2026-09-23T09:21:41.495+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126472454
2026-09-23T09:22:01.817+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126513630
2026-09-23T09:22:22.039+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126532663
2026-09-23T09:22:43.921+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126562734
2026-09-23T09:23:04.148+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126573851
2026-09-23T09:23:24.341+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126603956
2026-09-23T09:23:45.888+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126623032
2026-09-23T09:24:06.214+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126633993
2026-09-23T09:24:26.458+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126653074
2026-09-23T09:24:47.889+08:00 exit=0 target=route selected=HithinkFinance complete=true records=0 source_at=unix-ms:1790126683217
2026-09-23T09:25:08.033+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
2026-09-23T09:25:28.162+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
2026-09-23T09:25:49.711+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
2026-09-23T09:26:09.846+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
2026-09-23T09:26:29.979+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
2026-09-23T09:26:51.472+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
2026-09-23T09:27:11.691+08:00 exit=0 target=route selected=Eastmoney complete=true records=4 source_at=2026-09-23
```

## 本交接不宣称

- 不宣称候选能在 09:15 证明**当日**池子 —— 当日池子彼时可能尚不存在。
- 只宣称路由的行为与本次记录一致；记录里没有的东西不作推断。
