# 2026-09-22 新闻快讯全量 stale 拒绝 (上游侧证据)

## 现象

今日 (9/22) 新闻快讯链零产出: NewsFlashGate 拒绝 **749 条** flash, 全部
reason=stale; N-01 快讯 / N-02 聚合零推送 (对照: 9/21 及更早有正常快讯流)。

## 本地判定规则 (无本地缺陷)

- `src/news/aggregator/feed.rs:97`:
  `let stale = occurred_at.date_naive() != fetched_at.date_naive();`
  → **条目发布日期 ≠ 抓取日** 即 stale。
- `news_aggregator_init.rs:501-506`: gate 层另有
  `publication_date_not_current` (occurred_at 日期 ≠ 本地今日) 兜底。

749/749 全部 stale ⇒ 今日收到的 flash 条目 **occurred_at 全部落在非 9/22 的
日期** (即上游 feed 的条目日期停在旧日, 或上游时钟日期未翻到 9/22)。

## 佐证: 批次级时间戳是新的, 条目级日期是旧的

- Jin10 flash 批次: 09:30-11:55 每 ~2 分钟一批, `accepted=15`,
  `source_at=2026-09-22 11:18:18 / 11:21:35 / 11:53:01` (批次抓取时刻 = 今天,
  新鲜 ✓)
- 但批次内条目全部 stale → **条目自身的 occurred_at ≠ 9/22**

即: 抓取正常、网络正常、本地 gate 正常; **上游 feed 内容日期未翻日**。
与 2026-09-02 事故同款 (magic 上游时钟日期卡 9/1, 时间在走日期不翻 → 全量
stale 整批拒; 当时重启上游 VM 后恢复)。

## 影响面

- N-01 NewsFlashCritical / N-02 NewsFlashAggregated: 零推送 (9/22)
- D-01 AI 新闻分析 (公告侧): 55 条正常推送 (不受影响, 走 announcement 链)
- 竞价/持仓/卖出/复盘等其余推送链: 不受影响

## 建议 (交上游)

1. 检查 magic VM 系统日期是否翻到 2026-09-22 (`date`), 以及 NTP/w32tm
   是否仍同步 (rev 4 曾修 +0.70s 时间偏移, 日期翻日需确认)。
2. 检查 flash 条目 occurred_at 的生成逻辑 (是否用了缓存/静态日期)。
3. 本地无需改动: gate 规则是既定的 fail-closed 语义 (旧闻不进聚合缓冲)。
