# 2026-09-22 19:00 复盘批次 R-08/R-09 上游侧证据

## 现象

19:00 复盘批次 13 任务: delivered=5 (R-03/R-04/R-11/R-13/A-10),
R-07 expected_wait (发布门), **R-08/R-09 failed** — 全部因上游 gRPC
`internal` / `no_verified_batch`:

- R-08 (EventCalendar 事件日历):
  `GrpcBridge data gateway failed reason_code=internal retryable=true`
  (EventCalendar 查询 服务端内部错误)
- R-09 (ProviderTopN 复盘排行):
  `reason_code=source_transport_failed ... provider=Some(Eastmoney)
  retryable=true: gRPC ProviderTopNRankings 查询失败: 服务端内部错误`

## 当日上游故障全景 (同一天, 四起独立失败)

| 时间窗 | 能力 | reason_code | 恢复 |
| --- | --- | --- | --- |
| 09:15-09:25 | LimitPools (竞价) | internal | 09:30 自愈 |
| 全天 | HistoricalBars 日线 | no_verified_batch (部分码 Baidu 成功, 逐码 flaky) | 19:09 部分恢复 |
| 19:00 | EventCalendar | internal | 未恢复 |
| 19:00 | ProviderTopNRankings (Eastmoney) | internal | 未恢复 |

另: 新闻快讯条目 stale 案 (749 条) 已单独成文
(2026-09-22-news-flash-stale-upstream-reply.md / downstream-reply.md)。

## 本地侧处置

- R-08/R-09 复盘任务保持 retryable 语义 (失败出声, 任务层已记录);
- 归因日线已加 adaptive 回退 (OutcomeDailyBars op, 服务端多提供方链);
- 归因补跑工具已加逐码容错 (双路失败跳过) — 但今日补跑因 BR-255
  epoch 链完整性 (daily chain invalid at row 5, 部分码缺价导致) 未能
  落库, 归因卡今日仍缺失, 明早 15:05 主路径重试。

## 交上游排查

1. EventCalendar 与 ProviderTopNRankings 的服务端错误 (`internal`) —
   是否与同日 LimitPools/HistoricalBars 故障同源 (服务端某共享组件)?
2. HistoricalBars 逐码 flaky (同请求不同码成功/失败交替) — 疑似服务端
   对部分标的的 verified batch 校验状态不一致。
3. 建议服务端按能力 (operation) 维度的健康度自检 + 失败请求的
   request_id 侧日志抽查。
