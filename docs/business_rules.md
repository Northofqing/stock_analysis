# Business Rules — registered decisions for logic involving dedup / mutex / filter / sort / limit

> Per AGENTS.md §2.10: "Logic involving dedup / mutex / filter / sort / limit MUST be registered in docs/business_rules.md first."
> Each BR has a stable ID (BR-NNN), a one-line intent, and a code pointer.

| BR ID | Status | Intent | Code |
|-------|--------|--------|------|
| BR-001 | ✅ registered | Day-level HTTP cache for sector exclusion boards — same-day reuse avoids 600 HTTP calls per review cycle | `src/decision/exclusion.rs:30-50` (`cached_exclusion_map` + `EXCLUSION_MAP_CACHE`) |
| BR-002 | ✅ registered | Dedup `seen_titles` (announcements / news) within a session via `HashSet<String>`; key prefix "ann:" + first 40 chars | `src/monitor/news_monitor.rs:121-130` |
| BR-003 | ✅ registered | Sector concentration filter: precompute `HashMap<&str, f64> sector_totals` in single pass (was O(N²)) | `src/risk/limits.rs:50-70` (`check_position_limits` valuation loop) |
| BR-004 | ✅ registered | Filter out positions missing prices via explicit "缺价" violation (no silent fallback to cost_price) | `src/risk/limits.rs:74-90` |
| BR-005 | ✅ registered | Cache `chain_hits` / `keyword chain` (config) via `ArcSwap` lock-free load; reload atomic | `src/config.rs:317-340` (`CHAIN_RULES`, `EXCLUSION_BOARDS`, `ANNOUNCE_KEYWORDS`, `MONITOR_CONFIG`) |
| BR-006 | ✅ registered | Cache K-line / financials / money-flow / intraday via `DashMap` sharded locks (was `Mutex<HashMap>` single bottleneck) | `src/data_provider/service.rs:47-55` |
| BR-007 | ✅ registered | AhoCorasick automaton for ACTION_KEYWORDS scan — O(n+m) single pass instead of N × `str::contains()` | `src/decision/decision_decide.rs:368-378` (`ACTION_AC` static) |
| BR-008 | ✅ registered | AhoCorasick single-pass for keyword priority match in `classify_title` (announcement) | `src/data_provider/announcement.rs:170-200` (`KwList` enum + `first_match`) |
| BR-009 | ✅ registered | Monotonic queue for SKDJ rolling window max/min — O(n) instead of O(n×40) | `src/analyzer/analyze.rs:200-235` |
| BR-010 | ✅ registered | Push-index caching in `evaluate_audit` — single `locate_push_idx` at entry, helpers consume `push_idx` for O(1) slice | `src/opportunity/news_outcome.rs:188-260` |
| BR-011 | ✅ registered | Per-iteration RustDHC `analyze_postmarket` dedup — single `sig_opt` reused across `pattern_score` and `breakout_reason` blocks | `src/opportunity/mod.rs:1112-1135` |
| BR-012 | ✅ registered | Tokio `join!` for `compute_account_mode_metrics_blocking` + `latest_account_mode_change` (concurrent DB calls) | `src/bin/monitor/main.rs:479-500` |
| BR-014 | ✅ registered | Sina (hq.sinajs.cn) 接入 fallback priority 1 — GBK 编码 + 公开 HTTP + JSONP 解析, IP 独立于腾讯/东财 | `src/data_provider/sina_provider.rs`, `src/data_provider/stock_code_map.rs` |
| BR-015 | ✅ registered | Baostock (baostock.com) 盘后专用日终数据, 无限调用, WebSocket-like session + 复权 (adjustflag=2) | `src/data_provider/baostock_provider.rs`, `src/data_provider/fallback.rs` |
| BR-016 | ✅ registered | Sina 新闻 API (feed.mix.sina.com.cn) — 实时轮询财经要闻 (90s) + 盘后回溯个股新闻 (15:30), 双写 news_items (详存, 新表, content_hash 标题+摘要 SHA256 去重) | `src/data_provider/sina_news_provider.rs`, `src/data_provider/news_item.rs`, `src/database/mod.rs` |
| BR-017 | ✅ registered | 板块联动归因 (B-002): 标题含「板块名 + 拉升/异动」+ 板块 change_pct > 0 + 主力净占比 > 0 → 生成 `ChainSource::Board` ChainHit, 异动股门槛 5%, 跳过 gate_hits 置信度过滤, 同 board 多新闻去重 | `src/opportunity/chain_mapper.rs::extract_board_rotation_with` |
| BR-018 | ✅ registered | 事件抽取去重 (B-003): simhash 汉明距 ≤ 3 视为同事件, 否则 LCS 公共子串 ≥ 5 字, 双重去重. 跨批次从 `event_seen_simhash` 表加载 (2 天窗口), 批次内 + 跨批次均去重. 5min 周期落库 | `src/opportunity/event_extractor/mod.rs::extract_batch_rules_only_with_seen` + `src/database/concepts.rs::save_event_seen` |
| BR-019 | ✅ registered | 持仓影响零信号抑制 (B-004): `PositionImpact::is_zero_signal()` 检测 reason == "无直接产业链关联" (magic string), 折叠 N 只零信号持仓为单行 summary, 全部零信号则完全抑制推送 (避免 7 行废话/10min = 70 行/小时噪声) | `src/opportunity/impact.rs::is_zero_signal` + `src/opportunity/mod.rs::run_opportunity_scan` |
| BR-020 | ✅ registered | v13 模板分时窗派发 (B-005): 盘前 9:00-9:15 P-01/P-03 在 `monitor_loop` outer loop top (preopen_pushed_date 跨天 flag); 盘后 evening_target (从 `OpportunitySchedule::push_evening` 读, 默认 19:00) 走 `dispatch_post_session_review()` 统一推 A-10/A-01/R-02/R-08 真实 dispatcher, R-03/R-04/R-05/R-06 暂时复用占位, 4-hour cap 防 monitor 重启错过窗口 | `src/bin/monitor/main.rs::monitor_loop` (L3834-3870, L5137-5170) + `src/bin/monitor/push_templates.rs::dispatch_post_session_review` |
| BR-021 | ✅ registered | 量价反向发现: 板块有量价异动 (涨幅≥4% / 量比≥2 / 主力资金加速≥5pp, 阈值 env 可覆盖) 但 `news_match` 无法用新闻文本归因 → 判定"异动无归因", 按严重度 (涨幅+放量超额×5+加速×2) 降序, 限 max_n=5 条. 空新闻文本不臆测有新闻 (红线2.2), 异动板块全部保留 | `src/market_analyzer/sector_monitor.rs::classify_unexplained` + `detect_unexplained_moves` |
| BR-025 | ✅ registered | 盘中虚拟买入落库: 选股推荐按置信度≥50排序取 Top3, D-01 仅 `BuyDip` 动作触发; 每次真实推送/推荐均生成带毫秒时间戳的 `plan_id`, 允许同日同股多次虚拟买入; 价格缺失/非法则跳过, 虚拟腿只写 `paper_trades` 不写真实持仓 | `src/bin/monitor/main.rs::run_stock_screener` + `submit_virtual_buy_from_intraday_pick`, `src/bin/monitor/push_templates.rs::submit_virtual_buy_from_d01` |
| BR-026 | ✅ registered | R-08 明日事件区分展示: 持仓/观察池分【实盘】(get_positions)/【虚拟】(virtual_observation, 按 code 去重, latest 优先) 两类; 宏观公告按 holding_codes 拆"持仓相关"/"非持仓", 各取 TOP 3; 空数据显式提示不臆造 (红线 2.2) | `src/bin/monitor/push_templates.rs::build_event_calendar_macro_summary` + `event_calendar_virtual_holdings` + `render_event_calendar` |
| BR-027 | ✅ registered | 盘后资金净流入 Top10 收盘价虚拟买入 (15:35 发送): fetch_market_main_inflow_top(10) 按 main_net_yi 降序取 Top10 (过滤 ST/北交所/main_net_yi≤0/price≤0), 每只以收盘价 BUY 100 股写 paper_trades; 收盘涨停 (主板≥9.8%/创业科创≥19.8%) 标 NotFilled 不臆造成交; plan_id 带毫秒时间戳允许同日多次; 只写 paper_trades 零写真实持仓; 盘后 15:35 门控发送 (等收盘资金数据稳定) | `src/bin/monitor/push_templates.rs::dispatch_post_close_fund_inflow_buy` + `src/bin/monitor/main.rs::monitor_loop` (post-close 15:35 gate) |
| BR-028 | ✅ registered | CLI operator 认证闸 (默认禁用, opt-in 启用): 默认 `MONITOR_AUTH_REQUIRED` 未设或 != "1" → 跳过认证 (单机 single-user 不打扰). 设 `MONITOR_AUTH_REQUIRED=1` 启用 → monitor / winrate_simulator / live CLI 启动前需 PAM 认证 MONITOR_OPERATOR (或当前 Unix user via whoami), 3 次失败 → exit 1, 无 TTY / PAM 错误也 exit 1. 在 DB init / spawned task / monitor loop 之前. 严格匹配 expected_operator 不接受任意 Unix user. | `src/auth/operator.rs::require_monitor_operator_auth` + `src/bin/monitor/main.rs` / `src/bin/winrate_simulator.rs` / `src/main.rs` 起始 |
| BR-029b | ✅ registered (v17.1-hotfix 临时) | `PUSH_NORMAL_FORCE=1` 临时旁路 escape hatch: 设了 → `account_mode::evaluate()` 直接返 `AccountMode::Normal` (绕过 Frozen 保持 + 仓位超限 + 熔断全部判定). 用途: v17.1 治本前让 Frozen 仓位超限时 L5 不再全 Deny (4 铁律: 默认值出声). **不是默认行为, 不设= 不生效**. 治本落地后, Frozen 模式已能放行 (governance.rs Step 2 fall-through) → 此 env var 主要用于 "想试一下健康模式预览" 而非 "保留 Frozen 接收推送". 回滚: `unset PUSH_NORMAL_FORCE` 走标准 evaluate. ⚠️ BREAKING 此 env var 设了 = 强行 Normal 会绕过 BR-021 (Frozen 等下一交易日盘前重置) 强制语义; 生产环境禁用. | `src/risk/account_mode.rs::evaluate` (line 100-110 early-return) |
| BR-029c | ✅ registered (v17.1-r2 §3.6 接入) | `STOCK_ANALYSIS_PUSH_V6_ENABLE=1` 走 L6 SinkRouter 投递路径: 设了 → `notify::push_governor_inner` 把 `text` 包成 `PushMessage` (delegatede `l6_sink::build_push_message`) 经 `SinkRouter::route()` 投到注册 sinks. 默认不设 → 仍走原 `push_wechat(text).await` (向后兼容). **L6 当前注册 sinks**: ConsoleSink (stdout 默认关) + MagiclawSink (delegate notify::push_wechat, 含 dry-run + MagicLaw daemon + 飞书 HTTP). env=1 时 MagiclawSink 再调一次 push_wechat — 等价于双重 path 但 sink_name 走 magiclaw. 真换 sink (Feishu Webhook / Wechat Webhook) 等后续 v17.x §3.6 扩. ⚠️ BREAKING 设了 = sink_name 在 L7 analytics 变成 "magiclaw" 而非 "wechat/feishu/dry_run", 旧 dashboards 按 sink 聚合会变化. 回滚: `unset STOCK_ANALYSIS_PUSH_V6_ENABLE` 即回默认路径. 生产环境建议先设 =1 + 看 L7 analytics 一周确认正常再固定. | `src/bin/monitor/l6_sink.rs` (MagiclawSink + ConsoleSink + SinkRouter) + `src/bin/monitor/notify.rs::push_governor_inner` (env opt-in 分支) + `src/bin/monitor/main.rs` (mod l6_sink; + 启动 banner L6 暖身) |
| BR-029d | ✅ registered (v17.4 全天新闻聚合接入) | NewsAggregator 全局初始化: 启动早期一次性调 `init_news_aggregator()` 注册 15 个 NewsFeed 适配 (Jin10/WallStreetCN/CLS/SinaFlash/WeiboHot/Gelonghui/KcbDaily/GovPolicy = 8 个真 HTTP; GovCn/MIIT/EmAnnouncement/EarningsCalendar/Consensus/MarketAction/AnalystViews = 7 个 unit stub). `news_monitor_loop` 每 tick 调 `tick_news_aggregator(20)`, 返回 dedup 后 `Vec<MarketEvent>`. 本期仅 log event 数 + 按类型分布; 后续 `news_ranker::rank_events(&events)` → 候选 → 推 v14 push 栈. ⚠️ BREAKING 启动时打 8 个真 HTTP Provider (含 HTTP client init) + 7 个 unit stub; 单测要 mock 或隔离 (Unit struct 占位 7 个 stub 不联网, 8 个 Provider::new() 是构造不发起请求, 真正 HTTP 在 .fetch()). 默认行为变更: 之前 news_monitor_loop 只走 NewsMonitor; 现在叠加 NewsAggregator 双轨 — events 同时进 aggregator.tick 和 nm.process_news. 后续 rank 去重避免重复推送. 回滚: 删 `mod news_aggregator_init;` + main.rs 启动 banner 调用 + news_monitor_loop tick 调用; aggregator 单例保持不变. | `src/bin/monitor/news_aggregator_init.rs` (init + tick) + `src/bin/monitor/main.rs` (mod + 启动 banner + news_monitor_loop tick 调用) |
| BR-030 | 🟡 spec-only (v17.1-r2 未实施) | 推送 L4 (kind, code) 冷却窗 dedup — Reservation token 原子 reserve/commit/rollback: 时间窗内 (不论 committed) 一律 Deduped; 投递失败 rollback 删占位, 避免"失败占满 24h"; expires_at 用 Option<Instant> (None = 已过期) | spec: `docs/v17.x/v17.1-r2-event-infrastructure.md §5.6`; 计划落点 `src/push_l4/dispatcher.rs` |
| BR-031 | 🟡 spec-only (v17.3 未实施) | 推送 daily_limit 限速 — 全局桶 200/天 + per-kind 二级 (KBuy/KSell 20, KStopLoss 30); fetch_add 单步原子 check+increment (超限回退), 本地时区 day_key 跨天整体 reset (顺带防内存增长); 默认开启 (v15.x 出声), env `PUSH_DISABLE_DAILY_LIMIT=true` 仅调试关闭且 banner 可见 | spec: `docs/v17.x/v17.3-migration-and-persistence.md §5.5`; 计划落点 `src/event/l5_limit.rs` |
| BR-032 | 🟡 spec-only (v17.1-r2 未实施) | DispatcherRegistry 路由早退 — Vec 按注册顺序遍历, accepts() 首个 true 即处理并停止; 启动 validate() 对 2+ dispatcher 覆盖同 event_type 输出 warn (不阻断) | spec: `docs/v17.x/v17.1-r2-event-infrastructure.md §5.4 + §13.4 决策 #12`; 计划落点 `src/event/dispatcher.rs` |
| BR-033 | ✅ registered | v17.4 能力1 新闻推送门 (filter+limit): critical 即时推 = strength≥threshold(默认80) 且 certainty≥60, event_id 当日去重, 每日上限 max_critical_per_day(默认20, 超限 warn 出声); 4 时段聚合 = 09:30/11:30/13:00/15:00 ±90s 各触发 1 次/日, 取当日 buffer 按 strength 降序 Top3; 全部阈值走 MonitorConfig (红线2.9 与 v17.4 §5.1/§6 互引) | `src/bin/monitor/news_aggregator_init.rs` (NewsFlashGate) + `src/bin/monitor/main.rs::news_monitor_loop` |
| BR-034 | ✅ registered | v17.4 能力2 虚拟仓复盘双窗 dedup: 13:00 快照与 evening 全量复盘共用 PushKind::PaperReview (cooldown 86400/票), 快照用 "noon-{code}" 作 dedup code 前缀隔离两窗口; 13:00±90s 当日一次门控 (Mutex<Option<NaiveDate>>) | `src/bin/monitor/main.rs` (noon snapshot cron) + `src/bin/monitor/push_templates.rs::dispatch_paper_review_noon` |

---

## Old Modules Migration Table (per CLAUDE.md "When new capability is added")

When adding ArcSwap / DashMap / AhoCorasick / SHARED_HTTP_CLIENT patterns, the following old modules were inspected for migration:

| Module | Uses SHARED_HTTP_CLIENT | Uses ArcSwap (config) | Uses DashMap | Uses AhoCorasick | Decision |
|--------|-------------------------|------------------------|--------------|------------------|---------|
| `notification/service.rs` | ✅ migrated | n/a | n/a | n/a | Adopt |
| `data_provider/service.rs` | ✅ migrated | n/a | ✅ migrated | n/a | Adopt |
| `data_provider/north_flow.rs` | ✅ migrated | n/a | n/a | n/a | Adopt |
| `data_provider/mod.rs` (financials) | ✅ migrated | n/a | n/a | n/a | Adopt |
| `data_provider/eastmoney_provider.rs` | partial (still per-call) | n/a | n/a | n/a | **Deferred** — uses cached per-instance client; refactor risky |
| `data_provider/gtimg_provider.rs` | partial | n/a | n/a | n/a | **Deferred** — provider keeps its own connection (data source alignment) |
| `data_provider/money_flow.rs` | partial | n/a | n/a | n/a | **Deferred** — same as gtimg |
| `data_provider/yahoo.rs` | n/a (blocking client) | n/a | n/a | n/a | **Skip** — sync only |
| `data_provider/announcement.rs` | n/a (blocking client) | n/a | n/a | ✅ AhoCorasick | **Partial** — AhoCorasick path applied to `classify_title` |
| `lhb_analyzer.rs` | ✅ migrated | n/a | n/a | n/a | Adopt |
| `monitor/news_ai.rs` | n/a | n/a | n/a | n/a | No shared client needed (linker only) |
| `monitor/news_monitor.rs` | n/a | n/a | n/a | n/a | No shared client needed |
| `monitor/signal_state.rs` | n/a | n/a | n/a | n/a | DB-only |
| `monitor/scanner.rs` | n/a | n/a | n/a | n/a | DB-only |
| `monitor/prediction.rs` | n/a | n/a | n/a | n/a | DB-only |
| `opportunity/auction_agent.rs` | n/a | n/a | n/a | n/a | DB-only |
| `opportunity/chain_mapper.rs` | n/a | ✅ migrated (`Arc<Vec<ChainRuleConfig>>`) | n/a | n/a | Adopt |
| `opportunity/discover.rs` | n/a | n/a | n/a | n/a | DB-only |
| `opportunity/impact.rs` | n/a | n/a | n/a | n/a | DB-only |
| `pipeline/data.rs` | n/a | n/a | n/a | n/a | DB-only |
| `pipeline/position_tracker.rs` | n/a | n/a | n/a | n/a | DB-only |
| `pipeline/score_breakdown.rs` | n/a | ✅ migrated (`&config.factor_feedback`) | n/a | n/a | Adopt |
| `search_service/service.rs` | n/a | n/a | n/a | n/a | DB-only |
| `trend_analyzer.rs` | n/a | n/a | n/a | n/a | Pure compute, no external state |
| `bin/monitor/main.rs` | ✅ migrated (account_mode helpers) | n/a | n/a | n/a | Adopt |
| `bin/winrate_simulator.rs` | n/a | n/a | n/a | n/a | No live IO |
| `app/bootstrap.rs` | n/a | n/a | n/a | n/a | Init-only |
---

## BR-029 — 文档演进路线归档规范（2026-07-11 落地）

> **触发的红线**: AGENTS.md §2.10（业务/规则改动需登记业务规则）
> **范围**: 仅限 `docs/` 目录文档演进路线分类与命名，不涉及代码逻辑

### 规则内容

| 项 | 规范 |
|---|---|
| **演进版本文件夹** | `docs/v9.x/`、`v10/`、`v11/`、`v12/`、`v13/`、`v14.x/`、`v15.x/`、`v16.x/`、`v17.x/`、`v18.x/` 十个；按文档内容的"所属版本时代"归位，与代码版本基线对齐 |
| **pre-v9 前史** | 所有 v2-v8、optimization_report-06-22 之前的"演进前史"文档统一归档到 `docs/_archive/pre-v9-history/`，git 历史可恢复 |
| **命名格式** | `<版本>-<日期 YYYY-MM-DD>-<skill>-<作用>.md`（两段式 skill = 实际产出所用 skill 名，取自 `.agents/skills/`） |
| **skill 推断原则** | spec/设计类 → `brainstorming`；实施类 → `implement`；审计/评审 → `grill-with-docs` 或 `review`；bug 诊断 → `diagnosing-bugs`；实施计划 → `writing-plans`；实施日志 → `executing-plans` |
| **README 必备** | 每个版本文件夹必须含 `README.md`（演进定位 + 上承/下启 + 文件索引 + 同期协作文档路径），`docs/` 根目录含 `README.md` 总索引 |
| **活跃 spec** | 当前活跃的 spec（如 `v13.0-...-brainstorming-push-templates-spec-active.md`、`v14.2-...-brainstorming-push-architecture-active.md`）文件名后缀加 `-active` 标识 |
| **归档 vs 删除** | 一律 `git mv` 或 `mv` 归档，**不删除**；保留 git 恢复可能 |

### 注册表入口

- 规则文档：`docs/business_rules.md`（本条 BR-029）
- 总索引：`docs/README.md`
- 版本索引：各 `docs/v*/README.md`
- 归档索引：`docs/_archive/pre-v9-history/README.md`（待补）

## v18 闭环规则（设计已登记，实施前必须引用）

| Rule ID | 状态 | 规则 | 计划落点 |
|---|---|---|---|
| BR-038 | 🟡 spec-only | 行动数据可用性闸：任何开仓/加仓/调仓的纸面或未来实盘动作，行情、账户及必需特征都必须为 `Available`；`Unavailable`、`Stale`、`Invalid`、`Conflicted` 一律产出可审计拒绝，禁止默认值、降级数值或旁路。减仓/平仓的例外只能由版本化风险策略显式定义。 | `src/data_contract/`、`src/risk/` |
| BR-039 | 🟡 spec-only | 决策批次完整性：同一 strategy/model/config/universe/data-health/portfolio revision 的候选集只能形成一个确定性 `candidate_batch_id`；批次内保留所有候选、排序、入选和拒绝原因，重复提交返回原决定而非重算或新增订单。 | `src/decision/` |
| BR-040 | 🟡 spec-only | 纸面订单幂等：`paper_order_key = decision_id + target_revision + side`；同一 key 的重试只返回既有订单。订单必须先取得风险结果与不可变审计回执，才可进入 `Submitted`。 | `src/paper_ledger/` |
| BR-041 | 🟡 spec-only | 纸面账本对账闸：日内或日终重放事件所得现金、可卖数量、持仓和费用与投影不一致时，账本状态为 `ReconciliationBlocked`，拒绝新纸面订单，直至以纠正事件完成对账；不得直接修改汇总行。 | `src/paper_ledger/` |
| BR-042 | 🟡 spec-only | 模型迭代双账本：Champion 与 Challenger 使用同一时间对齐的输入、各自独立的虚拟资金账本和版本化策略；复盘只能生成 `ModelChangeProposal`，不得直接改生产配置、权重或模型状态。晋升需样本充分性、成本后表现、风险/覆盖证据及人工审批。 | `src/review/`、`src/research/` |
