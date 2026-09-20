# 新闻分析零推送根因交接 (2026-09-20 晚核实)

## 现象
- 用户问「新闻的分析怎么还没推送」。核实: 新闻管线整批 fail-closed,
  D-01/N-01/N-02/NewsAI 全部零产出。
- NewsFlashGate 对每个 source event 拒绝:
  `[NewsFlashGate][BR-137] source event rejected before critical and
  aggregate governance: invalid_observation_time`
  (news_aggregator_init.rs:507-510 校验: provenance.fetched_at > now
  || fetched_at < occurred_at)。

## 时间线 (monitor-launchd-20260916.stderr.log 计数)
- 9/14 制品进程 (9/19 21:40 → 9/20 18:29): 170 次拒绝
- 18/52 制品进程 (9/20 18:47 → 21:36): 301 次拒绝
- 22/52 制品进程 (9/20 21:36 起): 48 次拒绝 (首批 21:37:56 + 22:13:16)
- **结论: 预存故障 (至少 24h+), 非 22 Unit 接线引入** (接线在 gate 下游,
  被拒事件不到 dispatch)。

## 根因假设 (待上游核实)
- 上游 magic-market.local VM 的新闻 provenance 时间戳异常 (fetched_at
  未来化或早于 occurred_at) — 与 9/2「上游日期卡 9/1」/ 9/4「上游时钟」
  同族故障。本地时钟正常 (date=2026-09-20 22:25 CST)。
- 对照: 盘后回溯 Sina 个股新闻正常 (fetched_at=本地 15:40, published
  9/14-9/19, 24/51/100 条落库) — 全局新闻源 (CLS/ThePaper/Jin10/
  Eastmoney) 的事件被拒。

## 排查指引
1. 在 VM 上游 (magic-market.local) 检查新闻批次 provenance 时间戳
   (fetched_at vs occurred_at vs 当前 UTC);
2. 本机侧如需观察具体值: 临时在 news_aggregator_init.rs:510 处把
   validation_error 分支日志加 timestamp 值 (当前只打 reason);
3. 修复上游后无需改代码, gate 自动放行。
