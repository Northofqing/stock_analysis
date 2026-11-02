# Ticket 数据缺口专项计划（2026-11-02）

> 范围：只修 **DataMode 的 Kline / MoneyFlow / OrderBook** 与 **P1.1 的 news / mf**
> 两个缺口。不扩功能、不改 ticket 批次顺序、不把“代码写完”当成“胜率提高”。
>
> 关联：`docs/ticket-lifecycle-plan.md`、`docs/paper-ledger-execution-gap.md`。

## 0. 工程约束

本计划默认只做“最小根因修复”，遵守：

- 防过度防御：几乎不会发生的边界/并发问题，先讲概率和代价，经 operator 确认后再加；
  不为“理论上更稳”私自增加 fallback、重试、探针或抽象层。
- 入口单次校验：provider 数据只在 gateway 边界校验一次，内部用类型流转；
  不在每个消费点重复判空/判新鲜。
- 拒绝提前抽象：单次使用的逻辑就近内联；per-push 能力表只为现有 5 类推送写最小映射，
  不做通用框架。
- 控制复杂度：非必要不用多层 `try/catch`/错误包装；能单函数清晰完成的就单函数完成。

因此本文第 2/3 节的“备选”**默认不做**，只在 operator 明确确认后才进入实现；
首选修法才是本轮范围。

## 1. 结论：5 条根因，不列大清单

| 编号 | 症状 | 最小根因 |
|---|---|---|
| A1 | DataMode 缺 Kline | Kline 用墙钟 24h 判新鲜；最后一次日线是 10-30，周一 10:31 已超 24h |
| A2 | DataMode 缺 OrderBook + T0Advice 为 0 | OrderBook 只由 T0Evidence 严格完整批次标记；T0Evidence 报“服务端内部错误”，同一根因打掉两个功能 |
| A3 | DataMode 列 MoneyFlow 缺失 | `MoneyFlows` 对持仓票返回“无此股”，`BoardFlows` 间歇 server error；MoneyFlow 本是辅助能力，不该进关键门 |
| B1 | `news_candidates=0` | `collect_news_catalysts` 先查 `news_ai_assessment MAX(created_at)`，最新 10-14 早于窗口，直接返回 source stale |
| B2 | `mf_candidates=0` | f62 是东财行业名，`board_member` 是 TDX 概念名，交集仅 1/150；Eastmoney 成分映射因 BoardFlows 错误为空 |

## 2. A 组：DataMode

### A1 Kline

证据：

- `src/monitor/data_mode.rs`：`KLINE_MAX_AGE_SECS = 24 * 60 * 60`，按墙钟秒数判新鲜。
- `src/data_gateway/historical_bars.rs:109`：只有 `HistoricalDailyBars` 批次准入时才
  `mark_capability_success(Capability::Kline)`。
- 最近一次 `HistoricalDailyBars` 是 2026-10-30（`outcome=available provider=Tdx`）；
  2026-11-02 的日线要等 21:30 才落库。周一 10:31 距 10-30 已超过 24h，因此 Kline 被判 missing，
  直到当晚日线同步才恢复。

首选修法：

- 把 Kline 新鲜度改成“最近一个**已完成交易日**”，不要求当日盘中 K 线；
- 判定源用 `stock_daily MAX(date)` 或最近一次 `HistoricalDailyBars` 的 trade_date，
  而不是进程内 `Instant` 的 24h；
- 只改 `data_mode.rs` 的 Kline 新鲜度语义，补周五→周一的单测。

备选：启动时对上一交易日跑一次 `HistoricalDailyBars` 探针，成功后标记 Kline。
代价是每次启动多一次请求，但不动 DataMode 语义。

失败模式：上一交易日日线也缺时，Kline 必须继续 missing；不能用更早的日线冒充。

验收：周一开盘 DataMode 不再仅因 24h 墙钟误判 Kline；只有“上一交易日日线缺失”才 missing。

### A2 OrderBook + T0Advice

证据：

- `src/data_gateway/grpc_source.rs:4601`：OrderBook 只在
  `complete_t0_batch_proves_order_book(...)` 成立后标记；T0 批次不完整/失败，OrderBook 永不标记。
- monitor 日志持续：
  `[做T-持仓][BR-153] 数据批次拒绝 ... gRPC T0Evidence 查询失败: 服务端内部错误`。
- `T0Advice` 最后成功停在 2026-10-22；今天 0 条。做T和 OrderBook 是**同一个 T0Evidence 根因**，
  不是两个独立故障。

首选修法：修 T0Evidence 的桥接/对端“服务端内部错误”，要求 request_id、重试、批次完整性；
修好后 OrderBook 标记与 T0Advice 一起恢复。

备选：把 OrderBook 与 T0Evidence 解耦，用腾讯/新浪五档盘口独立源，按新鲜度标注；
T0Advice 在 T0 修好前继续显式 unavailable，绝不用 stale T0 标记 OrderBook。

失败模式：备选盘口只有五档、深度不足，必须标“浅盘口”，不能写成完整 OrderBook。

验收：T0 批次 `records>0` 且完整才标记 OrderBook；T0Advice 有记录；否则显式 unavailable。

### A3 MoneyFlow

证据：

- 持仓票日志：`缺少主力净流 (MoneyFlows 无此股)`（002131/600396/601598/001216 等）。
- `BoardFlows` 间歇 `服务端内部错误`；板块资金流也不稳定。
- `src/monitor/data_mode.rs` 注释明确 MoneyFlow 是辅助能力；它不应是 Unsafe 的根因，
  但当前 missing 列表会让人误读为“全局不可用”。

首选修法：明确 MoneyFlow 为辅助能力，移出关键 DataMode 路径；缺个股资金流只跳过该信号并
标注 unavailable，不阻断 HoldingPlan。这是 per-push 数据依赖的一部分。

备选：加东财/腾讯个股资金流备源；只有完整批次才使用，拿不到就标 unavailable。

失败模式：没有资金流信号；不能补 0，也不能用板块流冒充个股流。

### A 组总修法：per-push 数据依赖

现在 L5 用全局 DataMode 判所有推送，缺 OrderBook/MoneyFlow 会把 HoldingPlan 也打成
`data_quality`。改为按推送声明必需能力：

- HoldingPlan：Quote + 上一交易日日线；
- T0Advice：Quote + T0Evidence；
- 09:25 竞价：Auction；
- 16:10 明日计划：日线 + 可选 Quote；
- Preopen 新闻：News（不依赖 Quote）。

缺非必需能力只标注，不阻断；必需能力缺失才 fail-closed。这是最先做、收益最大的一步。

## 3. B 组：P1.1 news / mf

### B1 news

证据：

- `src/monitor/ticket_watch_pool.rs:168` 先查
  `SELECT MAX(substr(created_at,1,10)) FROM news_ai_assessment`；
  最新是 2026-10-14，早于窗口起点就直接返回 `news_ai_assessment 最新 ... 早于窗口起点（来源陈旧）`。
- 因此 `news_candidates=0` 不是“今天没有新闻催化”，是**查询在 freshness 门就退出了**。
- 日志里的 `[NewsAI] assessment retained ... existing=false/true`、`deferred_delivery=11`
  是恢复/投递路径，不等于股票级 `target_code` 已产出；global 新闻还走
  `news_ai_global_critical_v1`，不能混表。

首选修法：恢复股票级 `news_ai_assessment` 写入，要求
`impact ∈ positive/major_positive` 且 `target_code` 非空；先查 `[NewsAI]` 的
timestamp 解析（`unix-ms:` 毫秒）、model unavailable、manual_review 三类错误。

备选：用 `news_items`/公告标题做确定性的“公司名/代码 → 股票”映射，产出低证据等级的 catalyst；
不能冒充 AI 评估。

失败模式：news_candidates 仍为 0；显式标 `news unavailable`，不能记 0 当健康。

验收：`MAX(created_at) >= since` 且 `news_candidates>0`；否则 unavailable。

### B2 mf

证据：

- f62 榜 100 行（2026-10-30，东财行业名：食品饮料/医药生物/白酒Ⅲ…）；
- `board_member` 5,624 行、150 个板块名（TDX 概念：融资融券/破净资产/低市净率…）；
- 两者板块名交集 **1/150**；
- `eastmoney_members` 因 `BoardFlows` 服务端错误拿不到，`collect_board_picks` 映射不到成分股，
  返回 0。
- 日期门：10-30 业务日的 f62 需要 10-29，但榜是 10-30，会被判 unavailable；
  11-02 业务日需要 10-30，日期是通的。

首选修法：持久化东财板块→成分映射（BoardFlows/BoardMemberships 同一命名空间），
f62 用这份映射选票；确保 `eastmoney_members` 有数据。

备选：维护一份行业名→股票代码的静态映射（申万/东财），只用于 f62，不复用 TDX `board_member`；
映射不到就标 mf unavailable。

失败模式：映射覆盖率低；必须输出覆盖率，低于阈值报警，不能把 0 候选当“今日无资金流机会”。

验收：`mf_candidates>0` 且输出映射覆盖率；否则 `mf unavailable`。

## 4. 两天验证计划

### 第 1 天

1. 单测 Kline：模拟“上周五最后日线、周一 10:30”，断言 Kline 不再 missing；
   跑 `cargo test --lib data_mode`。
2. 直接探针 T0Evidence / BoardFlows / MoneyFlows：记录 request_id、批次、records、rejections、age。
3. SQL 基线：`news_ai_assessment MAX(created_at)`；f62 vs board_member 交集；
   当前 `mf_candidates`。
4. 实现 per-push 数据依赖最小版本（HoldingPlan/T0Advice/竞价/明日计划各一张能力表）。

### 第 2 天

1. `watch_pool_build --dry-run`：news/mf 候选是否 >0；没有就输出 unavailable 原因。
2. `phase_acceptance --days 5` + `ticket_ops_check`：看 P1.1、批1.3、P2 是否变化。
3. 盘中/盘后 DataMode：Kline 是否只在“上一交易日日线缺失”时 missing；OrderBook 是否由
   T0 批次标记；HoldingPlan 是否不再因 MoneyFlow 被 `data_quality` 拦。
4. 独立证据：原始 provider payload、bridge 日志 request_id、Python 重算候选数；
   不能只用我们自己的摘要。

验收阈值：

- Kline：周一开盘不再因 24h 墙钟误判；只有上一交易日日线缺失才 missing。
- OrderBook/T0：T0 批次 records>0 才标记 OrderBook；T0Advice 有记录；否则显式 unavailable。
- MoneyFlow：HoldingPlan 不因 MoneyFlow 缺失被拦；个股缺流明确标注。
- news：最新日期 >= 窗口起点且 `news_candidates>0`，否则 unavailable。
- mf：`mf_candidates>0` 且输出映射覆盖率；否则 mf unavailable。

## 5. 修好后会恢复 / 仍应 unavailable

会恢复：

- Kline 盘前判断（A1）；
- OrderBook（若 T0 修好，A2）；
- T0Advice（若 T0 修好，A2）；
- news 催化（若股票级评估恢复，B1）；
- mf 候选（若东财映射接通，B2）。

仍可能 unavailable：

- 个股 MoneyFlow（无备源时）；
- 完整深度 OrderBook（用五档备选时）；
- T0Evidence（对端未修时）；
- news AI（provider 未恢复时）。

不允许：用 0、旧 f62、旧 news 或 stale T0 冒充可用。

## 6. 现在不能下的结论

- 不能因为修了 DataMode 就说胜率提高；胜率要看 11-23 样本外，且 D+1 现在只有 74 个样本，
  目标约 220。
- 不能因为 `news_candidates` 从 0 变正就说新闻源健康；要看 `target_code` 和 evidence grade。
- 不能因为 T0Advice 恢复就说做T可执行；它仍是建议，不是 paper 成交。
- T0Evidence / BoardFlows / MoneyFlows 的对端修复没有 ETA；没有原始批次证据前，不能宣布修好。

## 7. 证据索引

- `src/monitor/data_mode.rs`：`KLINE_MAX_AGE_SECS=24h`、MoneyFlow 辅助能力注释、
  per-capability 状态。
- `src/data_gateway/historical_bars.rs:109`：Kline 标记点。
- `src/data_gateway/grpc_source.rs:4601`：OrderBook 由 T0Evidence 严格批次标记。
- `src/monitor/ticket_watch_pool.rs:168`：news freshness 门；
  `collect_money_flow_boards` / `collect_board_picks`：f62 映射。
- `src/bin/monitor/main.rs`：`prepare_t0_messages`（T0Advice 读
  `latest_user_position_snapshot` + T0Evidence，不读 paper）。
- 2026-11-02 快照：10:31 `DataMode=Degraded`（缺 Kline/MoneyFlow/OrderBook，Quote 可用）；
  11:48 `Unsafe`（午休 Quote 缺失，属预期）；`news_ai_assessment` 最新 10-14；
  f62 3 天、board_member 150 名/5,624 行、交集 1。
