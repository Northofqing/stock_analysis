# 2026-09-22 竞价窗口故障 → 09:30 恢复 (上游侧证据)

## 结论

与 2026-09-20 记录的模式**完全一致**: magic VM 的 gRPC 竞价通道 (LimitPools via
provider=Custom) 在 09:15–09:25 竞价窗口内持续 `internal` (服务端内部错误), 窗口
结束后 **09:30 自愈**。本地无代码缺陷 — 新 reason-code 映射 (commit c430dd8) 生效,
真实原因码可见。

## 时间线 (本地 stderr, 字节偏移 99514392 起)

| 时间 | 事件 |
| --- | --- |
| 09:00–09:11 | Tonghuashun 涨停池盘前正常: accepted=101 (source_at=2026-09-21) |
| 09:15–09:25 | 竞价量能扫描每 ~15s 一轮; gRPC LimitPools 查询持续 `internal` retryable=false (计 ~11 次); A-02 auction repush pushed=false, P-05 candidate board pushed=false; BR-223 保留窗口内重试资格 |
| 09:23:43 | 最后一次 `internal`: provider=Custom → unavailable (竞价量能涨停列表获取失败) |
| 09:25:38 | P-05 候选源不可用 (RealtimeQuotes `no_verified_batch`), 窗口结束 |
| 09:30:21 | **恢复**: Tonghuashun UpperLimitPool available, accepted=13, source_at=2026-09-22 |
| 09:31:46 | Eastmoney limit-pool available, accepted=17, source_at=2026-09-22 |

## 与 9/20 对比

- 9/20: 窗口内失败 (internal/no_verified_batch 混现), 09:30 恢复 → 当日竞价卡零推送
- 9/22: 窗口内失败 (internal 主导), 09:30 恢复 → 当日竞价卡零推送 (A-02/P-05 均 false)

连续两个交易日同模式 = **上游竞价窗口期容量/就绪问题** (推测: VM 在集合竞价阶段
的专有通道未就绪或限流), 非网络抖动。

## 建议 (交上游)

1. 检查 magic VM 竞价窗口 (09:15–09:25) 的 LimitPools 服务就绪时间 — 若服务在
   09:25 后才 warm-up, 提前启动或加窗口期降级数据源。
2. 本地已有备源 (Tonghuashun/Eastmoney 涨停池) 在窗口外可用, 但竞价量能卡语义
   依赖竞价期实时数据, 备源无法替代。
3. 若上游短期无法修: 评估 A-02/P-05 改走 9:20 快照 (盘前备源 + 量比) 的降级路径。
