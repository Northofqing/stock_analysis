
## 2026-09-21 竞价实测 (周一, 证据更新)
- 本地管线正常: monitor PID 60992 存活, 竞价窗口 (09:15-09:25) dispatcher
  正常触发 (09:22:00 "9:20-9:25 量能扫描"), fail-closed 保重试。
- 但上游数据全程失败, 零推送:
  - P-02: `[竞价] 涨停池批次拒绝: 竞价量能涨停列表获取失败: GrpcBridge
    data gateway failed reason_code=internal provider=None` (09:22:01 /
    09:24:17 两轮)
  - A-02: `候选源不可用: 统一实时行情 Gateway 不可用: GrpcBridge data
    gateway failed reason_code=no_verified_batch` (09:22:09 / 09:24:25)
- 上游状态: VM 50051 通, magic-market-grpc-server 进程在跑 (9/20 深夜
  重启后) — 但竞价时段数据查询返回 grpc internal / no_verified_batch,
  疑似竞价数据源 (涨停池/竞价实时行情 provider) 在 VM 重启后未恢复或
  竞价期数据源本身异常。
- 结论: 非本地接线问题; 上游竞价数据 provider 待查 (竞价价 0.000 缺陷
  的历史变体或 VM 重启后服务状态)。

## 2026-09-21 09:35 连续竞价实测 (补充判定)
- 09:30 连续竞价开始后行情立即恢复: RealtimeMarketQuotes outcome=
  available provider=Tencent 59 次 (09:30:56 起密集), HistoricalDailyBars
  available provider=Baidu, unavailable 仅 4 次 (切换瞬态)。
- 判定: 上游整体数据服务正常 (昨晚 VM 重启+校时有效); 竞价失败
  (09:15-09:25 internal/no_verified_batch) **仅限竞价期数据源** — 竞价
  时段涨停池/竞价行情 provider 在该窗口不可用, 即「竞价价 0.000」缺陷
  的现行变体。非本地接线问题 (dispatcher 正常触发/fail-closed/重试保留)。
- 下一步: 上游查竞价期 provider (magic-sina-rs 竞价通道/涨停池竞价数据源)
  为何仅在 09:15-09:25 窗口失败; 明天竞价窗口复验。
