# Business Rules — registered decisions for logic involving dedup / mutex / filter / sort / limit

> Per AGENTS.md §2.10: "Logic involving dedup / mutex / filter / sort / limit MUST be registered in docs/business_rules.md first."
> Each BR has a stable ID (BR-NNN), a one-line intent, and a code pointer.

| BR ID | Status | Intent | Code |
|-------|--------|--------|------|
| BR-052 | ✅ registered | Day-level HTTP cache for sector exclusion boards — same-day reuse avoids 600 HTTP calls per review cycle | `src/decision/exclusion.rs:30-50` (`cached_exclusion_map` + `EXCLUSION_MAP_CACHE`) |
| BR-053 | ✅ registered | Dedup `seen_titles` (announcements / news) within a session via `HashSet<String>`; key prefix "ann:" + first 40 chars | `src/monitor/news_monitor.rs:121-130` |
| BR-054 | ✅ registered | Sector concentration filter: precompute `HashMap<&str, f64> sector_totals` in single pass (was O(N²)) | `src/risk/limits.rs:50-70` (`check_position_limits` valuation loop) |
| BR-055 | ✅ registered | Filter out positions missing prices via explicit "缺价" violation (no silent fallback to cost_price) | `src/risk/limits.rs:74-90` |
| BR-056 | ✅ registered | Cache `chain_hits` / `keyword chain` (config) via `ArcSwap` lock-free load; reload atomic | `src/config.rs:317-340` (`CHAIN_RULES`, `EXCLUSION_BOARDS`, `ANNOUNCE_KEYWORDS`, `MONITOR_CONFIG`) |
| BR-057 | ✅ registered | Cache K-line / financials / money-flow / intraday via `DashMap` sharded locks (was `Mutex<HashMap>` single bottleneck) | `src/data_provider/service.rs:47-55` |
| BR-058 | ✅ registered | AhoCorasick automaton for ACTION_KEYWORDS scan — O(n+m) single pass instead of N × `str::contains()` | `src/decision/decision_decide.rs:368-378` (`ACTION_AC` static) |
| BR-059 | ✅ registered | AhoCorasick single-pass for keyword priority match in `classify_title` (announcement) | `src/data_provider/announcement.rs:170-200` (`KwList` enum + `first_match`) |
| BR-060 | ✅ registered | Monotonic queue for SKDJ rolling window max/min — O(n) instead of O(n×40) | `src/analyzer/analyze.rs:200-235` |
| BR-061 | ✅ registered | Push-index caching in `evaluate_audit` — single `locate_push_idx` at entry, helpers consume `push_idx` for O(1) slice | `src/opportunity/news_outcome.rs:188-260` |
| BR-062 | ✅ registered | Per-iteration RustDHC `analyze_postmarket` dedup — single `sig_opt` reused across `pattern_score` and `breakout_reason` blocks | `src/opportunity/mod.rs:1112-1135` |
| BR-063 | ✅ registered | Tokio `join!` for `compute_account_mode_metrics_blocking` + `latest_account_mode_change` (concurrent DB calls) | `src/bin/monitor/main.rs:479-500` |
| BR-064 | ✅ registered | Sina (hq.sinajs.cn) 接入 fallback priority 1 — GBK 编码 + 公开 HTTP + JSONP 解析, IP 独立于腾讯/东财 | `src/data_provider/sina_provider.rs`, `src/data_provider/stock_code_map.rs` |
| BR-065 | ✅ registered | Baostock (baostock.com) 盘后专用日终数据, 无限调用, WebSocket-like session + 复权 (adjustflag=2) | `src/data_provider/baostock_provider.rs`, `src/data_provider/fallback.rs` |
| BR-066 | ✅ registered | Sina 新闻 API (feed.mix.sina.com.cn) — 实时轮询财经要闻 (90s) + 盘后回溯个股新闻 (15:30), 双写 news_items (详存, 新表, content_hash 标题+摘要 SHA256 去重) | `src/data_provider/sina_news_provider.rs`, `src/data_provider/news_item.rs`, `src/database/mod.rs` |
| BR-067 | ✅ registered | 板块联动归因 (B-002): 标题含「板块名 + 拉升/异动」+ 板块 change_pct > 0 + 主力净占比 > 0 → 生成 `ChainSource::Board` ChainHit, 异动股门槛 5%, 跳过 gate_hits 置信度过滤, 同 board 多新闻去重 | `src/opportunity/chain_mapper.rs::extract_board_rotation_with` |
| BR-068 | ✅ registered | 事件抽取去重 (B-003): simhash 汉明距 ≤ 3 视为同事件, 否则 LCS 公共子串 ≥ 5 字, 双重去重. 跨批次从 `event_seen_simhash` 表加载 (2 天窗口), 批次内 + 跨批次均去重. 5min 周期落库 | `src/opportunity/event_extractor/mod.rs::extract_batch_rules_only_with_seen` + `src/database/concepts.rs::save_event_seen` |
| BR-069 | ✅ registered | 持仓影响零信号抑制 (B-004): `PositionImpact::is_zero_signal()` 检测 reason == "无直接产业链关联" (magic string), 折叠 N 只零信号持仓为单行 summary, 全部零信号则完全抑制推送 (避免 7 行废话/10min = 70 行/小时噪声) | `src/opportunity/impact.rs::is_zero_signal` + `src/opportunity/mod.rs::run_opportunity_scan` |
| BR-049 | ✅ registered | v13 模板分时窗派发 (B-005): 盘前 9:00-9:15 P-01/P-03 在 `monitor_loop` outer loop top (preopen_pushed_date 跨天 flag); 盘后 19:00 后走 `dispatch_post_session_review()` 统一并行派发 A-10/A-01/R-02..R-08，失败逐项显式留痕且不阻塞其他报告。编号 BR-049 避免覆盖 v10 已登记的 BR-020（real_alpha benchmark）。 | `src/bin/monitor/main.rs::monitor_loop`, `src/bin/monitor/push_templates.rs::dispatch_post_session_review` |
| BR-070 | ✅ registered | 量价反向发现: 板块有量价异动 (涨幅≥4% / 量比≥2 / 主力资金加速≥5pp, 阈值 env 可覆盖) 但 `news_match` 无法用新闻文本归因 → 判定"异动无归因", 按严重度 (涨幅+放量超额×5+加速×2) 降序, 限 max_n=5 条. 空新闻文本不臆测有新闻 (红线2.2), 异动板块全部保留 | `src/market_analyzer/sector_monitor.rs::classify_unexplained` + `detect_unexplained_moves` |
| BR-071 | ✅ registered | 盘中虚拟买入落库: 选股推荐按置信度≥50排序取 Top3, D-01 仅 `BuyDip` 动作触发; 每次真实推送/推荐均生成带毫秒时间戳的 `plan_id`, 允许同日同股多次虚拟买入; 价格缺失/非法则跳过, 虚拟腿只写 `paper_trades` 不写真实持仓 | `src/bin/monitor/main.rs::run_stock_screener` + `submit_virtual_buy_from_intraday_pick`, `src/bin/monitor/push_templates.rs::submit_virtual_buy_from_d01` |
| BR-072 | ✅ registered | R-08 明日事件区分展示: 持仓/观察池分【实盘】(get_positions)/【虚拟】(virtual_observation, 按 code 去重, latest 优先) 两类; 宏观公告按 holding_codes 拆"持仓相关"/"非持仓", 各取 TOP 3; 空数据显式提示不臆造 (红线 2.2) | `src/bin/monitor/push_templates.rs::build_event_calendar_macro_summary` + `event_calendar_virtual_holdings` + `render_event_calendar` |
| BR-073 | ✅ registered | 盘后资金净流入 Top10 收盘价虚拟买入 (15:35 发送): fetch_market_main_inflow_top(10) 按 main_net_yi 降序取 Top10 (过滤 ST/北交所/main_net_yi≤0/price≤0), 每只以收盘价 BUY 100 股写 paper_trades; 收盘涨停 (主板≥9.8%/创业科创≥19.8%) 标 NotFilled 不臆造成交; plan_id 带毫秒时间戳允许同日多次; 只写 paper_trades 零写真实持仓; 盘后 15:35 门控发送 (等收盘资金数据稳定) | `src/bin/monitor/push_templates.rs::dispatch_post_close_fund_inflow_buy` + `src/bin/monitor/main.rs::monitor_loop` (post-close 15:35 gate) |
| BR-074 | ✅ registered | CLI operator 认证闸 (默认禁用, opt-in 启用): 默认 `MONITOR_AUTH_REQUIRED` 未设或 != "1" → 跳过认证 (单机 single-user 不打扰). 设 `MONITOR_AUTH_REQUIRED=1` 启用 → monitor / winrate_simulator / live CLI 启动前需 PAM 认证 MONITOR_OPERATOR (或当前 Unix user via whoami), 3 次失败 → exit 1, 无 TTY / PAM 错误也 exit 1. 在 DB init / spawned task / monitor loop 之前. 严格匹配 expected_operator 不接受任意 Unix user. | `src/auth/operator.rs::require_monitor_operator_auth` + `src/bin/monitor/main.rs` / `src/bin/winrate_simulator.rs` / `src/main.rs` 起始 |
| BR-076 | ✅ retired (2026-07-18 safety closure) | 危险的 `PUSH_NORMAL_FORCE` 旁路已删除；环境变量不能改写 AccountMode 或交易授权。Frozen/ReduceOnly 只由真实账户指标、前态和风险阈值计算；通知展示需求不得降级资金安全。回滚不得恢复强制 Normal。 | `src/risk/account_mode.rs::evaluate`, `src/trading/risk_adapter.rs` |
| BR-077 | ✅ registered (v17.1-r2 §3.6 接入) | `STOCK_ANALYSIS_PUSH_V6_ENABLE=1` 走 L6 SinkRouter 投递路径: 设了 → `notify::push_governor_inner` 把 `text` 包成 `PushMessage` (delegatede `l6_sink::build_push_message`) 经 `SinkRouter::route()` 投到注册 sinks. 默认不设 → 仍走原 `push_wechat(text).await` (向后兼容). **L6 当前注册 sinks**: ConsoleSink (stdout 默认关) + MagiclawSink (delegate notify::push_wechat, 含 dry-run + MagicLaw daemon + 飞书 HTTP). env=1 时 MagiclawSink 再调一次 push_wechat — 等价于双重 path 但 sink_name 走 magiclaw. 真换 sink (Feishu Webhook / Wechat Webhook) 等后续 v17.x §3.6 扩. ⚠️ BREAKING 设了 = sink_name 在 L7 analytics 变成 "magiclaw" 而非 "wechat/feishu/dry_run", 旧 dashboards 按 sink 聚合会变化. 回滚: `unset STOCK_ANALYSIS_PUSH_V6_ENABLE` 即回默认路径. 生产环境建议先设 =1 + 看 L7 analytics 一周确认正常再固定. | `src/bin/monitor/l6_sink.rs` (MagiclawSink + ConsoleSink + SinkRouter) + `src/bin/monitor/notify.rs::push_governor_inner` (env opt-in 分支) + `src/bin/monitor/main.rs` (mod l6_sink; + 启动 banner L6 暖身) |
| BR-078 | ✅ registered (v17.4 全天新闻聚合接入，2026-07-17 安全修订) | NewsAggregator 生产注册表只能包含真实可抓取的 NewsFeed；GovCn/MIIT/EarningsCalendar/Consensus/MarketAction/AnalystViews 等未实现或主动触发型适配器不得注册为轮询源，直接调用必须显式返回 unavailable。Jin10/WallStreetCN/CLS/Weibo/Gelonghui/KcbDaily/GovPolicy/公告等真实源抓取失败必须返回错误，禁止降级为空事件。`news_monitor_loop` 每 tick 将去重事件交给 BR-082 NewsFlashGate 和真实推送消费者；空结果只表示所有成功源本轮确实无数据。 | `src/news/aggregator/feed.rs`, `src/news/aggregator/mod.rs`, `src/bin/monitor/news_aggregator_init.rs`, `src/bin/monitor/main.rs` |
| BR-079 | 🟡 spec-only (v17.1-r2 未实施) | 推送 L4 (kind, code) 冷却窗 dedup — Reservation token 原子 reserve/commit/rollback: 时间窗内 (不论 committed) 一律 Deduped; 投递失败 rollback 删占位, 避免"失败占满 24h"; expires_at 用 Option<Instant> (None = 已过期) | spec: `docs/v17.x/v17.1-r2-event-infrastructure.md §5.6`; 计划落点 `src/push_l4/dispatcher.rs` |
| BR-080 | 🟡 spec-only (v17.3 未实施) | 推送 daily_limit 限速 — 全局桶 200/天 + per-kind 二级 (KBuy/KSell 20, KStopLoss 30); fetch_add 单步原子 check+increment (超限回退), 本地时区 day_key 跨天整体 reset (顺带防内存增长); 默认开启 (v15.x 出声), env `PUSH_DISABLE_DAILY_LIMIT=true` 仅调试关闭且 banner 可见 | spec: `docs/v17.x/v17.3-migration-and-persistence.md §5.5`; 计划落点 `src/event/l5_limit.rs` |
| BR-081 | 🟡 spec-only (v17.1-r2 未实施) | DispatcherRegistry 路由早退 — Vec 按注册顺序遍历, accepts() 首个 true 即处理并停止; 启动 validate() 对 2+ dispatcher 覆盖同 event_type 输出 warn (不阻断) | spec: `docs/v17.x/v17.1-r2-event-infrastructure.md §5.4 + §13.4 决策 #12`; 计划落点 `src/event/dispatcher.rs` |
| BR-082 | ✅ registered | v17.4 能力1 新闻推送门 (filter+limit): critical 即时推 = strength≥threshold(默认80) 且 certainty≥60, event_id 当日去重, 每日上限 max_critical_per_day(默认20, 超限 warn 出声); 4 时段聚合 = 09:30/11:30/13:00/15:00 ±90s 各触发 1 次/日, 取当日 buffer 按 strength 降序 Top3; 全部阈值走 MonitorConfig (红线2.9 与 v17.4 §5.1/§6 互引) | `src/bin/monitor/news_aggregator_init.rs` (NewsFlashGate) + `src/bin/monitor/main.rs::news_monitor_loop` |
| BR-083 | ✅ registered | v17.4 能力2 虚拟仓复盘双窗 dedup: 13:00 快照与 evening 全量复盘共用 PushKind::PaperReview (cooldown 86400/票), 快照用 "noon-{code}" 作 dedup code 前缀隔离两窗口; 13:00±90s 当日一次门控 (Mutex<Option<NaiveDate>>) | `src/bin/monitor/main.rs` (noon snapshot cron) + `src/bin/monitor/push_templates.rs::dispatch_paper_review_noon` |
| BR-043 | ✅ registered | v17.3 Replay/History CLI 安全契约：已知 monitor flags 与 event command 可组合且参数顺序不吞命令；`--replay-rate-ms=N` 与双参数形式等价；force replay 仅发布含非空字符串 `payload.text` 且已加 `[REPLAY YYYY-MM-DD]` 的 `push.source`；相邻发布尝试按 `rate_ms` 节流；每次 replay envelope ID 进程内唯一；真实 sink 接受且 attempt/result 哈希链审计持久化后才统计发布成功，任一失败令 CLI 非零退出；history `limit=0` 显式表示不截断并输出全部结果，负数拒绝。 | `src/event/cli.rs`, `src/event/history.rs`, `src/event/replay.rs`, `src/bin/monitor/main.rs` |
| BR-044 | ✅ registered | LaunchGate 灰度回退：当前阶段为 `Gray` 且真实胜率 `< 50%` 时必须回退到 `Shadow`；`50%` 边界不回退。灰度满 30 天且胜率 `>= 55%` 才可升级 `Live`，升级判断优先于回退判断。 | `src/opportunity/launch_gate.rs::LaunchGate::check_transition`, `tests/launch_gate_tests.rs` |

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

## BR-075 — 文档演进路线归档规范（2026-07-11 落地）

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

- 规则文档：`docs/business_rules.md`（本条 BR-075）
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
| BR-045 | ✅ registered | 即时告警归因（旧 v10 注册表别名 BR-019）：只使用事件携带的真实新闻重要度与 `chain_mapper` 规则命中，不调用 LLM；证据缺失必须标注“查无催化”，结果在发送前回写 `AlertDetail.ai_decision`，并且每个已接受告警只允许一次显式可失败的 JSONL/Markdown 审计写入。 | `src/monitor/attribution.rs`, `src/monitor/alert_log.rs`, `src/bin/monitor/main.rs::push` |
| BR-046 | ✅ registered | 虚拟盘四铁律批量查询过滤：`analysis_result.code IN (...)` 中每个代码必须先转义单引号再作为 SQL 字符串字面量加引号，保留 `000001` 等前导零；空持仓直接返回，不生成无界查询。 | `src/trading/paper_engine.rs::check_4_iron_rules` |
| BR-047 | ✅ registered | 推送票级冷却键：`HoldingPlan`/`T0Advice`/`CandidateTriggered`/`ForbiddenOps`/`PaperTrade`/`NewsToIdea` 等 PerTicket 类型必须通过 `push_governor_v3` 传真实股票代码；无票号的两参入口只允许全局模板，误用于票级模板必须显式拒绝，禁止用 `_per_ticket_unbound` 让不同股票共享冷却桶。 | `src/bin/monitor/notify.rs::push_governor`, `src/bin/monitor/main.rs`, `src/bin/monitor/push_templates.rs` |
| BR-048 | ✅ registered | DailyReport 子报告去重必须在 reserve / commit / rollback 全阶段使用同一个 `(kind, code, sub_kind)` 键和同一个冷却窗口；`SectorTier`、`CapitalVerify` 显式覆盖为 1800 秒，`FactorIC` 未覆盖时继承 DailyReport 默认窗口。禁止仅在 reserve 携带 sub_kind、commit 却写入主报告键，或只记录期望窗口而仍使用 24 小时窗口。 | `src/bin/monitor/notify.rs::push_governor_inner_with_sub_kind`, `src/bin/monitor/v14_adapter.rs` |
| BR-050 | ✅ registered | 单元测试数据库单例初始化互斥：同一测试进程只允许一次 migration + `DB_INSTANCE` 发布；并发 `DatabaseManager::init` 必须在测试专用 mutex 内串行化，已初始化后幂等返回。测试不得删除仍被连接池持有的数据库文件，测试数据代码使用 `TEST_CODE` 前缀并仅清理自身行。生产初始化仍保持重复注册显式报错。 | `src/database/mod.rs::DatabaseManager::init`, `src/database/mod.rs::tests` |
| BR-051 | ✅ registered | 测试/生产双向过滤与物理隔离：`STOCK_ENV_MODE=test` 只接受 `TEST_CODE*`，`prod` 只接受真实代码；Rust 单元测试未显式指定环境时默认 test。monitor `--test` 必须在任何数据库/数据源初始化前设置 test 环境与通知 dry-run；未显式给 `DATABASE_PATH` 时只能打开 `data/test/monitor_test.db`，不得读取或写入生产数据库；若显式指向生产默认路径必须记录 BR-051 并以非零状态退出，禁止“拒绝但返回成功”。所有 E2E seed 使用 `TEST_CODE*` 且跟随当前隔离数据库路径。生产常规入口显式设置 prod。 | `src/risk/env_guard.rs`, `src/bin/monitor/main.rs` |
| BR-084 | ✅ registered | 统一订单安全过滤：模拟/纸面订单在写库前必须通过价格、100 股整数手、现金与 100 万单笔上限、涨跌停价、60 秒业务 ID 去重以及 50 万二次确认检查；行情、限价、30 秒内真实账户快照或确认任一缺失即拒绝，禁止成本价/推送价/虚构现金回退。 | `src/trading/order_safety.rs`, `src/trading/mod.rs`, `src/trading/paper_trade.rs` |
| BR-085 | ✅ registered | 模拟建仓集中度与仓位计算：候选代码必须从真实 `stock_concepts` 缓存或静态 chain registry 得到明确产业链；产业链持仓数、当日 T+1 冻结数或真实账户现金任一不可得即拒绝建仓。动态仓位缺少有效波动率时拒绝；非动态模式只能按真实可用现金计算，禁止默认本金、未知产业链按 0、冻结数按 0 或强制最少一手。 | `src/pipeline/position_tracker.rs`, `src/trading/mod.rs` |
| BR-086 | ✅ registered | 订单审计不可变：每次模拟/纸面订单尝试必须记录业务订单 ID、来源、行情时间、决策依据、请求价/执行价、数量、结果与失败原因；成交持仓、审计行及其 SHA-256 链证据必须在同一数据库事务提交。审计表与哈希链禁止 UPDATE/DELETE，启动和追加前必须验证完整链；缺失、部分或不匹配时失败关闭。只允许全空旧链一次性回填；链写失败必须回滚持仓/paper 与审计。回退到非链感知版本前必须冻结订单/paper writer 或部署兼容链 writer，down migration 不得删除证据。应用不得配置小于 5 年的清理策略；审计写入失败必须阻断订单。 | `src/database/order_audit.rs`, `src/trading/paper_trade.rs`, `src/trading/mod.rs`, `src/database/mod.rs` |
| BR-087 | ✅ registered | 交易回报源单例注册：T-14/T-15 管道只允许注册一个真实 `TradeEventSource`，重复注册拒绝；未注册、拉取失败或事件字段无效必须返回显式错误，禁止日志后返回空列表伪装成功。订单/成交分发只消费对应事件类型，缺少必填字段逐条拒绝并留痕。 | `src/bin/monitor/push_templates.rs` |
| BR-089 | ✅ registered | LaunchGate Calmar 口径：Shadow→Gray 的 Calmar 必须来自真实 `ledger` 净值序列，按 245 个 A 股交易日年化收益除以最大回撤计算。序列少于 2 点、日期无序/重复、净值非正或非有限、相邻交易日缺口、总收益不大于 -100%、最大回撤为 0 时指标不可用并显式返回错误；禁止固定 0、预测命中率或纸面占位替代。 | `src/opportunity/launch_gate.rs` |
| BR-091 | ✅ registered | 推送投递审计持久化：`push.delivery.audit` 必须在 dispatcher 返回 `Handled` 前同步追加到按年分片的 hash-chain JSONL，并刷新到文件；每条记录包含完整 envelope、前序 hash 和当前 hash。启动/首次写入必须校验现有链，目录不可写、尾部损坏、序列化或刷盘失败一律返回 `Failed` 且不增加 handled 计数。审计记录不得配置少于 5 年的清理；测试环境写入隔离目录。 | `src/event/dispatcher.rs`, `src/bin/monitor/main.rs` |
| BR-092 | ✅ registered | 日 K 数据进入计算前验证：所有生产 provider 和数据库的 date/OHLCV/amount/pct_chg 必填字段不得由 NULL、解析失败、估算值或当天日期补成 0；OHLC 必须正且有限、high/low 关系有效，成交量/额非负且有限。序列日期必须唯一连续，相邻有效收盘变化绝对值超过 20% 时无人工确认即返回错误。任何坏行、分页失败或不提供必填字段的来源使整批失败，禁止跳行/返回部分页后伪装完整序列。 | `src/database/repository.rs`, `src/data_provider/baostock_provider.rs`, `src/data_provider/eastmoney_provider.rs`, `src/data_provider/gtimg_provider.rs`, `src/data_provider/rustdx_provider.rs`, `src/data_provider/sina_provider.rs`, `src/monitor/data_quality.rs` |
| BR-093 | ✅ registered | R-02 盘面快照缺失语义：三大指数、成交额、涨跌停家数必须来自同一次真实 MarketOverview；请求/任务失败或任一必填值缺失、非有限、越界时整条 R-02 拒绝。炸板率、连板高度、成交额变化和主力净流等未接真实源的字段使用 `Option::None` 渲染“暂无”，0 只表示来源明确返回的真实零，禁止以全零元组代替失败。 | `src/bin/monitor/market_data.rs`, `src/bin/monitor/push_templates.rs` |
| BR-094 | ✅ registered | Agent 工具事实完整性：六个真实数据工具的传输、解析、空结果和计算失败必须返回 `Err`，不得用 `Ok({"error":...})` 伪装成功。ReAct 工具失败时删除同名旧 fact、记录失败审计并只向模型暴露明确的 unavailable observation；validator 只检查已取得的事实，但事实存在时缺少其必填结构必须失败。生产默认列表不得暴露 no-op validator。 | `src/agent/tools.rs`, `src/agent/tools_chip.rs`, `src/agent/tools_money_flow.rs`, `src/agent/tools_news.rs`, `src/agent/tools_research.rs`, `src/agent/tools_sector.rs`, `src/agent/loop_runner.rs`, `src/agent/validation.rs` |
| BR-095 | ✅ registered | 竞价影子预测过滤：`AuctionResult` 的代码、名称、涨跌幅、量比和匹配量占比必须完整且在有效域内，绝对涨跌幅不得超过 20%；`suspected_fake=true` 的虚假申报不得写入 prediction_tracker。预测目标日使用下一 A 股交易日。模块只处理调用方提供的真实快照；未接真实 source 时不得宣称生产扫描已启用。 | `src/opportunity/auction_agent.rs` |
| BR-096 | ✅ registered | 机会评分阈值单一事实源：生产推送门只读根级 `config/strategy.toml::opportunity_push_threshold`，并将同一值显式传入评分和最终门禁。总分值域为 0..=100；数据不足或 winrate 缺失/非正时封顶为 `threshold-1`，禁止默认灰度旁路、未生效的嵌套配置和评分层硬编码阈值。机器合同任一侧缺失或 `threshold > score_max` 必须使 CI 失败。 | `config/strategy.toml`, `config/design_contracts.toml`, `src/opportunity/score.rs`, `src/opportunity/mod.rs`, `tools/compliance/lib/check_design_contradiction.sh` |
| BR-097 | ✅ registered | 实时个股行情完整性与缺失语义：代码、名称、现价、涨跌幅和来源时间是整批必填字段，缺失、非有限、价格非正、绝对涨跌幅超过 20% 或过期时整批返回错误；成交量、量比和主力净流等辅助字段使用 `Option::None` 保留缺失。依赖辅助字段的排序、筛选或监控必须排除该行并记录原因，展示层渲染“暂无”，禁止补 0 制造排名、突破或做 T 信号。持仓净值快照必须取得全部持仓的有效实时价，否则整次快照拒绝，禁止成本价回退。 | `src/market_data.rs`, `src/bin/monitor/market_data.rs`, `src/market_analyzer/limit_up.rs`, `src/bin/monitor/main.rs`, `src/bin/monitor/push_templates.rs` |
| BR-098 | ✅ registered | `pushed_stocks` 决策消费必须以可解析的真实 `push_time`、正数 `push_price` 和结构化 `metric_json` 构造对应 `StrategyInput`，生产评分只采用 8 个已注册 `Strategy::score` 的输出，禁止按 `push_kind` 固定分数。盘中时效按真实当前时间减推送时间计算，超过 1 小时或未来时间拒绝；盘后 Momentum 必须具备真实量比、涨跌幅与价格且实际策略分不少于 8。坏时间、坏 JSON、非有限/缺失必需指标必须显式报错，禁止补 0 继续。 | `src/decision/intraday_monitor.rs`, `src/strategy/v16_4/_helpers.rs`, `src/strategy/v16_4/mod.rs`, `src/bin/monitor/push_templates.rs` |
| BR-099 | ✅ registered | 候选台多源与热度排序必须可证：每个 `CandidateSource` 只由对应真实源文件/数据流贡献，缺源不得用产业链记录复制成选股、优选或放量来源；同代码同源去重。候选价格、涨跌幅和热度使用 `Option` 保留缺失；热度仅在真实涨跌幅与主力净流均存在时按既定公式计算，并且只作为同档同源数候选的次级排序。缺行情的候选不得显示 0 或进入需要实时价的推送/交易。源文件存在但 IO、JSON、代码或名称非法时整源显式失败。P-03 量能证据只在真实量比存在时分档：`<1.0=Weak`、`1.0..<3.0=Mid`、`>=3.0=Strong`；缺失量比则拒绝该条触发，新闻/K 线/盘口未取得独立证据时必须标记 `Missing`。 | `src/opportunity/candidate_panel.rs`, `src/bin/monitor/push_templates.rs`, `src/opportunity/news_ranker.rs` |
| BR-100 | ✅ registered | P-04 虚拟成交回报只能从当地日已持久化的 `paper_trades` 读取，只选择 `Filled/NotFilled/Invalidated` 完成态（`SignalTriggered` 不是成交回报），按自增 ID 升序发送每条真实状态，不得由调用方传入 count 后生成“虚拟仓/NotFilled”占位。DB 失败或任一行的代码、名称、方向、状态、价格、数量、成交价、理由、账户/数据模式非法时整批拒绝。`Filled` 必须同时有正成交价和正确手数；缺失字段渲染为“缺失”，禁止 0/空串回退。候选触发转正必须同时满足人工开关与可审计分层样本门（样本数、Strong/Weak 胜率）；证据源未接入或不完整时必须保持 Shadow，禁止仅靠环境变量越过样本门。 | `src/bin/monitor/push_templates.rs`, `src/opportunity/candidate_state.rs`, `paper_trades` |
| BR-101 | ✅ registered | `chain_daily`/`board_rotation_daily` 的数据库连接或查询失败必须与“无数据”区分，生产推送和决策只能使用严格 `Result` 接口。P-01 盘前新闻热点只能由最新主线簇与最新板块联动快照构建；新闻标题、板块名、候选股真名均来自 `board_rotation_daily` 已持久化证据，不在 09:00 用尚未开盘的行情冒充实时名称源。主线/rotation stocks JSON 非法、代码/概念/新闻/股名为空或主线头股无对应名称证据时整批拒绝。主题与观察股沿用原有最多 3 个主线/每主线头股限制，新闻按已持久化强度顺序取最多 3 条；禁止空 `news_pairs`、代码冒充名称、panic 捕获或空集合回退继续推送。 | `src/database/concepts.rs`, `src/bin/monitor/push_templates.rs`, `src/bin/monitor/main.rs` |
| BR-102 | ✅ registered | A-10 盘后催化复盘的 theme 必须是最新 `chain_daily.concept`，已启动/待启动股名必须由同期 `board_rotation_daily.stocks` 的 code-name 证据解析，沿用头 3 只/随后 3 只的分组限制。持续性只使用真实 `continuation_count`；尚无对应真实量化口径的 score 必须为 `None`，禁止按主线簇数量生成 5.5/7.0/8.5。DB/JSON/名称证据任一失败时整批拒绝，禁止固定“主线题材”或代码当股名。 | `src/bin/monitor/push_templates.rs`, `src/database/concepts.rs` |
| BR-103 | ✅ registered | 纸面绩效与通用成交复盘均按持久化 `(ts,id)` 升序、以 code 分组做数量感知 FIFO；卖出只能使用对应历史买入成本，卖出数量超过可配对买入、重复/缺失 ID 或方向/价格/数量/日期非法时整批失败。绩效比率仅在数学口径可计算时入库/展示：无样本、样本不足、方差为 0 或缺基准必须保持 `None/暂无`；0 只表示真实计算结果恰为 0。 | `src/performance/snapshot.rs`, `src/review/journal.rs`, `src/review/equity.rs`, `src/review/report.rs`, `paper_performance_snapshot` |
| BR-104 | ✅ registered | 虚拟观察快照是研究观察而非已成交交易：同一当地交易日按 code 合并且每个 code 只保留一条最新记录，不得写入 `trades` 冒充 buy fill。写入前必须校验日期、环境隔离、非空名称、正且有限的价格和 100 股整数手。已存在日文件损坏、目录/文件 IO 失败、JSON 序列化或原子替换失败必须显式返回错误，禁止当空快照继续；日文件和 latest 使用同目录临时文件 `fsync + rename`，重试只覆盖同 code 记录。次日复盘的抓取器初始化、单票 K 线请求和后台任务失败使整批失败；尚未形成 T+1 K 线是可展示的真实缺失，不得补 0。 | `src/bin/monitor/main.rs` |
| BR-105 | ✅ registered | 监控标的池、盘后复盘和公告链的数据源失败不得转换为空集合：Scanner 必须用严格 portfolio API 一次性加载真实持仓与自选，保留真实名称并按 code 去重；DB/环境变量解析/名称解析失败返回 `Err`，监控本轮退避重试，禁止 `股票{code}` 占位。R-03 的 fetcher、watchlist 或任一入选 K 线失败使 `source_complete=false` 并拒绝聚合；R-08 公告请求/任务失败时不生成“今日无公告”，持仓缺实时价时不以成本价计算事件。NewsMonitor 初始标的池失败必须等待重试，公告客户端构造或公告拉取失败跳过本轮且出声，rank 后台任务失败不得格式化空榜。 | `src/monitor/scanner.rs`, `src/portfolio/store.rs`, `src/bin/monitor/main.rs` |
| BR-106 | ✅ registered | 东方财富板块榜和成份股响应的 code/name 及请求字段必须逐行完整、有限且在有效域内，协议缺 `data.diff` 或任一坏行使整批失败，禁止缺字段补 0 或跳行。盘中“涨停产业链”不得用板块涨幅推算涨停家数/连板数；无真实个股涨停历史时不生成 R-03。换手率 Top10 必须按真实成份股 `f8` 排序并使用独立 10 分钟计时器，主力流未接入时显示“暂无”，禁止按板块主力流排序后冒充换手率。连板批查使用日 K 最新在前契约，以今日之前的连续涨停判级；初始化、K 线、样本、时效或任务失败使整批拒绝。 | `src/market_analyzer/sector_monitor.rs`, `src/bin/monitor/market_data.rs`, `src/bin/monitor/main.rs`, `src/bin/monitor/push_templates.rs` |
| BR-107 | ✅ registered | 旧 `monitor --buy/--sell CODE:PRICE[:QTY]` 直接写 `trades` 的手工虚拟成交旁路必须拒绝：该输入没有同批 5 秒行情、涨跌停、30 秒账户、AccountMode/DataMode、卖出可用量、二次确认和不可变订单审计，不能被视为 Filled。CLI 检测到这两个参数必须明确非零退出并指向统一 paper decision/order safety 管道；删除可绕过安全门的 `record_virtual_trade` 写接口。研究型 `virtual_observation` 仍只记录观察快照，不是成交。 | `src/bin/monitor/main.rs`, `src/portfolio/mod.rs`, `src/trading/paper_trade.rs` |
| BR-108 | ✅ registered | DataMode 只能由各真实 capability 最近一次成功时间计算：Quote/Kline/MoneyFlow/News 从未成功为 Missing，超过阈值为 stale，OrderBook 未接入保持 Missing；禁止把 `Capability::ALL` 固定标成 30 秒新鲜。共享 banner 在首次真实评估前为 unavailable，调用方必须拒绝对应推送，禁止默认 `AccountMode=Normal/DataMode=Full/仓位=0/盈亏=0`。AccountMode 首次评估也必须按真实 metrics 计算并持久化初始状态；DB 读取失败不得按 Normal。banner 锁失败、模式标签非法或任一健康快照失败必须显式出错并保留/清除不可用状态。 | `src/monitor/data_mode.rs`, `src/bin/monitor/main.rs`, `src/bin/monitor/market_data.rs`, `src/data_provider/fallback.rs`, `src/data_provider/announcement.rs`, `src/broker.rs` |
| BR-109 | ✅ registered | 组合持仓缺失字段不得用数值哨兵或推导值冒充：未落库的 `hard_stop` 必须是 `None`，Checklist/复盘显示“未设”，持仓建议若没有真实硬止损必须拒绝该票，禁止以现价×0.92生成“估算止损”。`find_position`、ST 持仓批查、交易/净值日期解析必须传播 DB/坏行错误，不得转成 `None`、空列表、固定日期或查询起始日。T-16 只能使用通过 5 秒门的真实报价，并用已生效 5%→10%规则确定性重算止损/止盈；报价、持仓、banner 或派生参数失败均不得推送。 | `src/portfolio/mod.rs`, `src/portfolio/store.rs`, `src/risk/stop_loss.rs`, `src/monitor/checklist.rs`, `src/bin/monitor/main.rs`, `src/bin/monitor/push_templates.rs` |
| BR-110 | ✅ registered | 盘后 R 系列报告必须保持来源完整性：R-03 只消费经公共日 K 质量门得到的真实逐股涨停/连板批次，再按产业链聚合排序；不得用 `chain_daily.continuation_count` 同时冒充涨停家数、连板数和龙头板数。R-04 在龙虎榜真实查询源未接入时必须显式 `unavailable`，不得以空列表伪装成功。R-05/R-06 不得从通用交易行推导并不存在的胜率或失败归因；所需闭环样本未接入时明确禁用。R-08 的公告或持仓查询失败必须拒绝整份事件报告，禁止转换成“今日无公告/无持仓”；可选隔夜字段失败只能以明确缺失原因展示。批次必须记录每个子报告结果，不得丢弃返回值后恒报成功。 | `src/market_analyzer/lhb_review.rs`, `src/bin/monitor/main.rs`, `src/bin/monitor/push_templates.rs` |
| BR-111 | ✅ registered | 外部通知投递成功必须同时满足 HTTP 成功和渠道协议明确的成功字段；业务状态字段缺失、类型错误或非成功值均为协议失败，必须传播到调用方和投递审计，禁止缺字段默认成功。 | `src/notification/service.rs`, `src/event/dispatcher.rs`, `src/bin/monitor/notify.rs` |
| BR-112 | ✅ registered | 机会扫描、盘后候选与公告排序只有在新闻、板块、成份股、持仓和排序上下文均来自完整真实批次时才允许进入生产推送或虚拟观察。任一抓取、解析、后台任务、持仓或上下文字段失败必须拒绝整批；禁止把失败变为空集合、把涨跌幅/量比/资金流补 0、使用默认 `MarketContext`，或把未确认投递的公告全部标记为已路由。在严格 `Result` 真值链接通前，生产调度必须显式记录 `disabled=incomplete_source_contract`，不得调用仍含兼容回退的研究实现。 | `src/bin/monitor/main.rs`, `src/opportunity/mod.rs`, `src/opportunity/chain_mapper.rs`, `src/opportunity/news_ranker.rs` |
| BR-113 | ✅ registered | 推送治理与 L7 分析必须使用已验证的真实 banner 和可持久化 SQLite：banner 未发布或锁失败时拒绝治理，持久库打开/写入/读取/反序列化失败必须显式返回错误。生产不得回退内存/no-op store，不得把坏枚举、坏时间、坏 JSON、查询失败补成 Full/Approve/Passed/当前时间/0/空集合。外部 sink 已接受但权威投递或 L7 审计失败时不得向调用方报告 `Pushed`。 | `src/bin/monitor/v14_adapter.rs`, `src/bin/monitor/notify.rs`, `src/push_l7/analytics.rs`, `src/push_l7/sqlite_store.rs` |
| BR-114 | ✅ registered | 产业链分析的概念缓存、逐股在线概念、板块代码、板块成份、龙虎榜和主线持久化是一个来源完整批次。DB/HTTP/工具/分页/JSON/坏行或保存失败必须传播并拒绝报告；不得把失败变成空概念、空板块、空候选或“今日无主线”。概念与板块行的 code/name/数值必须逐行完整；筛选仍沿用未涨停、非 ST/北交所、涨幅 `-3%..=7%`、涨幅降序 Top8，且必须在完整成份批次上执行。 | `src/database/concepts.rs`, `src/pipeline/chain_analysis/mod.rs`, `src/pipeline/chain_analysis/fetchers.rs` |
| BR-115 | ✅ registered | 补充行情与通知协议失败必须端到端传播：财务、资金流、分时等补充源只有在协议解析和字段校验成功后才可缓存或供 Agent/流水线消费；线程 panic、空数据和传输失败必须保留为 `Result::Err`，不得缓存默认对象或按零值继续评分。飞书与 daemon 通知响应体读取、JSON 解析及显式成功字段任一失败均判定投递失败。 | `src/data_provider/financials.rs`, `src/data_provider/service.rs`, `src/data_provider/mod.rs`, `src/agent/tools.rs`, `src/agent/tools_money_flow.rs`, `src/pipeline/extra_context.rs`, `src/bin/monitor/notify.rs` |
| BR-116 | ✅ registered | 周期推送计时器只在真实空批次、明确去重或已确认投递后推进；数据获取、后台任务、治理或 sink 失败时保留到期状态以便下一轮重试并记录失败。持仓健康状态哈希只能在投递确认后提交，检查阶段不得提前覆盖；做 T 扫描使用独立 30 秒计时器，不得复用 5 分钟健康汇总计时器。 | `src/bin/monitor/main.rs` |
| BR-117 | ✅ registered | 新闻题材阶段按证据最小集判定：退潮、日内高潮、分歧和冷阶段只使用调用方已校验的当日涨跌、资金与涨停数，按既定优先级先判定；启动、发酵和三日累计高潮才读取连续板块历史。需要历史的分支遇到历史损坏/不可读必须返回 Unknown 并出声，禁止补 0；不需要历史的分支不得因无关历史文件故障丢弃当日真实结论。 | `src/opportunity/news_ranker.rs`, `src/market_analyzer/sector_history.rs` |
| BR-118 | ✅ registered | 旧版资金流 Markdown 的近 5 日净流入只能解析 `近5日:` 或 `近5日：` 标签之后、首个 `亿` 之前的完整有限浮点文本；标签数字不得进入数值，尾随说明、缺单位、空值、NaN/Inf 或非法字符必须返回缺失并保留中性资金分，禁止抽取任意数字后继续评分。原始 `MoneyFlowSummary` 仍优先于文本兼容路径。 | `src/pipeline/score_breakdown.rs::parse_5d_net_yi`, `src/data_provider/money_flow.rs::format_for_prompt` |
| BR-119 | ✅ registered | 估值历史与卖方一致预期必须在完整本地协议批次上聚合：存在的数值字段必须严格解析为有限值，日期必须合法且按来源约定严格降序；估值日序列不得重复或缺交易日，一致预期不得超出请求窗口。任何坏行、目标价非正/上下限颠倒、空批次或一致预期无 EPS 证据都使整批失败。负 PE/PB 只表示该值不参与估值分位，不得改写为 0；样本不足 30 日时分位保持缺失。 | `src/data_provider/valuation_history.rs`, `src/data_provider/consensus.rs` |
| BR-120 | ✅ registered | 行业对标必须由完整行业映射页和完整成份股批次构建。`data.diff` 缺失、code/name 空或类型错误、重复行业名映射到不同板块、重复成份代码、存在但非法/非有限的 PE/PB/ROE/增速字段，或空成份批次均整批失败；不得跳行、用 `NaN` 哨兵或空数组继续。缺失/空数值字段保持 `None`；正 PE/PB 才参与估值统计，ROE/增速只使用有限真实值；百分位和中位数只在相应证据存在时输出。 | `src/data_provider/industry.rs` |
