# 新闻快讯 stale: 下游侧核实回复 (2026-09-22)

对 `2026-09-22-news-flash-stale-upstream-reply.md` 的核实。**上游的字段语义更正成立,
我方原交接文档的「批次抓取时刻」表述错误, 现更正并补判别数据通道。**

## 1. 我方口径核实结果 (直接读代码)

`src/news/aggregator/feed.rs` (原 97 行, 现已加判别日志):

```rust
let occurred_at = record.published_at.with_timezone(&Local);
let fetched_at   = record.observed_at.with_timezone(&Local);   // ← 上游客证时刻
if fetched_at < occurred_at { bail!("observation precedes publication"); }
let stale = occurred_at.date_naive() != fetched_at.date_naive();
```

- `fetched_at` = **record.observed_at**(上游每条的 observed_instant), 不是本机抓取时刻、
  也不是批次 source_at — 与 9/20 交接修订版 2 的记录一致, 我方 9/22 文档写「抓取日」是
  笔误, 结论推导被带偏, 抱歉。
- stale 判定 = 条目发布日期 ≠ 上游客证日期。749 条 stale ⇒ 这些条目的 published_at 日期
  与其 observed_at 日期不同 (前者旧一日)。**这与「上游时钟日期卡死」无必然关系**, 上游
  的排除 (亚秒偏差 + 窗口内日志日期全 9/22) 是成立的。

## 2. 判别数据通道已落地

按上游要求 (与 9/20 排查指引 2 同项), 在 feed.rs 逐条打印被拒记录三值:

```
[NewsFlashStale][BR-166] provider={jin10|cls|thepaper|eastmoney} item_id={} published_at={} observed_at={}
```

- 提交在 master (2026-09-22 晚些部署上线, 明日开盘的 flash 流即产出判别数据)。
- 判读规则照上游给的: batch source_at=9/22 且条目 published_at=9/22 但 observed_at=9/21
  → 上游客证时刻错位 (上游侧查); 条目 published_at 本身就是 9/21 → 旧条目混入新批次
  (按上游语义「消费者决定新鲜度」, 我方 gate 按设计拒绝, 无缺陷, 但需确认 Jin10 flash
  窗口为何含跨日尾部 — 若上游想对「窗口整体过旧」硬拒再谈 Gate A)。

## 3. 我方 9/22 原文档的更正记录

- 「批次时间戳是今天 + 条目全旧 = 矛盾」: 不成立 — 合同规定 batch source_at =
  最新记录 published_at, 批次可合法包含更旧尾部条目; 两条证据不互斥。
- 「9/2 上游日期卡死同款」: 撤回 — 亚秒时钟偏差 + 窗口内上游日志日期全 9/22, 与 9/2
  形态不同。

## 4. 下一步

- 明日开盘后我方从 [NewsFlashStale] 日志提取样本交上游核对 (同一批次的
  batch source_at 与条目三值)。
- 我方 gate 维持 fail-closed 不变; 若样本证明是上游客证时刻错位, 上游修复后
  stale 自消, 无需下游改动。
