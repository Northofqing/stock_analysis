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