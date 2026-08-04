# 选股标准 (Selection Standards) 口径 review (from Explore agent)

**整体定性**: 选股口径在 **per-stock numerical gate** (`admission-v1`) 方面**定义良好**、**强审计**（hash-chained, version-tagged, content-hashed），但**散落 4+ 文件中**且对非数值风险（ST / 退市 / 新股 / 北交所 / 板块轮动 / 主力）有明显**完整性缺口**。Look-ahead 纪律好。Live binary 与 backtest 共享常量，但**两个并行的 magic_tdx adapter 存在 → drift 风险**。

---

## 1. 口径定义 (Criteria definition)

### F1. Magic thresholds 分散，无 single source of truth
- src/selection/admission.rs:7-10 — `MAX_PRICE_ABOVE_MA20=0.15`, `MAX_FIVE_DAY_RETURN=0.20`, `MIN_SETTLED_VOLUME_RATIO=1.0`, `MIN_INTRADAY_VOLUME_PACE=1.0`
- src/selection/quality.rs:4-6 — `MAX_QUOTE_AGE_SECONDS=5`, `MAX_ADJACENT_CHANGE=0.20`, `CONTINUITY_RELATIVE_TOLERANCE=0.000001`
- src/selection/features.rs:5 — `MIN_DAILY_BARS=21`
- src/selection/magic_tdx.rs:19-22 — `DAILY_FETCH_COUNT=64`, `FIVE_MINUTE_FETCH_COUNT=288`, `REQUIRED_DAILY_BARS=21`, `MAGIC_TDX_CONNECT_TIMEOUT_SECONDS=3.0`
- src/decision/exclusion.rs:11-22 — `DEFAULT_EXCLUDED_BOARDS` (10 hardcoded board names)
- src/decision/pre_trade_filter.rs:103, 132 — inline `5` (unlock window), `50.0` (pledge threshold)
- Issue: 读 6 文件才能还原口径；无 inline justification (没 `// 20% = A股主板涨停`)
- Impact: mild
- Fix: 单 `selection_thresholds.toml` 或 `thresholds.rs`，启动时载入，stamp 到 audit `context`

### F2. 重复常量 `21` (history depth)
- src/selection/features.rs:5 (`MIN_DAILY_BARS`) 与 src/selection/magic_tdx.rs:21 (`REQUIRED_DAILY_BARS`)
- Issue: 同样语义的双 private const 必须 lockstep；无任何机制 enforce
- Impact: moderate

### F3. `admission-v1` 是**唯一硬 gate**；"scoring model" (`model.rs`) 只存类型
- src/selection/model.rs: 只 identity / master / relation evidence types — **无 scoring weights 或 thresholds**
- Issue: pipeline 调 `evaluate_admission` (binary pass/fail)。**无 candidates 间的定量 ranking**；"model" 名字误导
- Impact: moderate (operators expect scoring; 他们只拿到 boolean gate)
- Fix: rename file 或加 documented scoring layer

### F4. `pre_trade_filter` 阈值 inline，未文档化
- src/decision/pre_trade_filter.rs:103 (`days.abs() <= 5`), :132 (`r > 50.0`)
- 5-day unlock window + 50% pledge line 是 inline literals；从不在 `business_rules.md`
- Fix: 提为 `pub const UNLOCK_WINDOW_DAYS` / `PLEDGE_HIGH_PCT` 并注册到 BR-022

## 2. 数据漂移 (Data drift)

### F5. magic_tdx contract 在 selection 与 data_provider 之间 silent drift
- src/selection/magic_tdx.rs:22, 44 — connect timeout `3.0s`, daily fetch `64`, 5-min fetch `288`
- src/data_provider/magic_tdx_provider.rs:49 — connect timeout `5.0s`
- src/data_provider/magic_tdx_provider.rs:74-83 — daily fetch 用 caller-supplied `days`
- Issue: 两个独立 wrapper 围绕 `magic_tdx_rs`. Selection 严格 3s timeout, production data provider 5s. 4s 网络毛刺 → data_provider 返回 data, selection 失败 → **backtest vs live 看到不同 universe**
- Impact: **severe** (backtest 可复现性破坏)
- Fix: 让 `selection::magic_tdx` 复用 `data_provider::magic_tdx_provider` (或共享一个 timeout constant)

### F6. 数据层无 sanity check — magic_tdx 是否回了 universe 全部
- src/selection/magic_tdx.rs:478-484 — `security_master_empty` 仅在 `identities.is_empty()` 才 reject
- Issue: magic_tdx silent return 50 names 而非 5000，normalize_master 仍接受 → 下游 `direct_mentions` 找不到 → `VerifiedEmpty`。**无 "expected universe size" cross-check** (SH≥~1700, SZ≥~2800)
- Impact: **severe** (silent empty universe 看起来像真实 "no candidates" 信号)
- Fix: assert `master.identities().len() >= MIN_EXPECTED_UNIVERSE` per market

### F7. Stale-quote 只被 5 秒 wall-clock 抓
- src/selection/quality.rs:4, 134-143 — `MAX_QUOTE_AGE_SECONDS=5`
- Issue: 恰好 4.99s 旧 quote pass；**未校 market holidays / server-time skew** between Beijing 与 magic_tdx daemon。Server clock skew 可能把 stale 标 fresh
- Impact: moderate
- Fix: reject `|local_clock - server_clock| > 60s` before 5s check

### F8. 日间 adjacent change 固定 ±20% — 无 per-board calibration
- src/selection/quality.rs:5, 210-218
- Issue: BR-131 / `monitor::data_quality::max_gap_for` 已 encode board-specific limits (主板 10%, 创/科 20%, 北交所 30%, ST 5%)。`selection::quality` 忽略，用 flat 20%
- Impact: moderate — ST 9% jump pass clean, **北交所 25% legal jump 被 reject**
- Fix: 传 `LimitPctResolver` (像 `monitor`) 进 `validate_daily`

## 3. 可解释性 (Explainability)

### F9. 完整 audit chain + reason codes — 强
- src/selection/audit.rs:43-55 (`SelectionAuditRecord`), :96-111 (`SelectionAuditError`)
- src/selection/pipeline.rs:603-630 (admission rejection emit every failure code)
- src/selection/pipeline.rs:923-962 (`append_rejection`)
- 强: hash chain 保证 tamper-evidence

### F10. `ai_degraded` / `stale` 没 echo 进 audit context
- src/selection/audit.rs:28-41 (`SelectionAuditContext`) vs `MarketEvent.stale`, `MarketEvent.ai_degraded` in `src/signal/market_event.rs`
- Issue: audit context 捕获 `provider`, `provider_published_at`, `reason_codes`，**但不收 originating event 的 `ai_degraded` / `stale` 标志**
- Impact: mild

### F11. Audit 文件路径 hard-coded 在 `data/audit/production`
- src/selection/pipeline.rs:1494-1497 — `SelectionAuditWriter::for_environment("data/audit", Production)`
- Issue: 若 binary CWD 改，audit 去不同文件。无 env-var override
- Impact: moderate (audit 跨 CWDs 碎片化)

## 4. 一致性 (Consistency)

### F12. Live & backtest 共享 same versioned criteria — 好
- src/selection/pipeline.rs:36-38 — `RELATION_VERSION="direct-mention-v1"`, `PIPELINE_VERSION="event-selection-v1"`
- src/selection/features.rs:4 — `FEATURE_VERSION="raw-selection-v1"`
- src/selection/admission.rs:6 — `ADMISSION_VERSION="admission-v1"`
- 强: identical constants; `selection_backtest` reads visible runs from DB; `selection_live_probe` 调 same `evaluate_market_events`

### F13. PostClose vs Intraday 对 intraday_volume_pace 处理不同 — documented but asymmetric
- src/selection/admission.rs:70-79, 135-144; tests at :268-292
- Issue: PostClose 容忍 missing intraday_volume_pace, Intraday 要求。Asymmetry 在 features.rs:76-79 (PostClose optional)。**Documented but 不在任何 user-facing doc**

### F14. TZ 一直 (Local) — 但 assume Beijing
- src/selection/magic_tdx.rs:532-541, src/selection/pipeline.rs:30
- Issue: 所有 timestamps are `Local`. **无 DST/Asia-Shanghai assertion**. 用户用 `Asia/Tokyo` 或 TZ unset → silently shifted quote ages → `MAX_QUOTE_AGE_SECONDS=5` 可能对 12-hour-old bar pass
- Impact: **severe for non-CN deploys**
- Fix: pin to `chrono_tz::Asia::Shanghai` or `FixedOffset::east_opt(8*3600)`

### F15. 集合竞价 / 9:30-9:34 不被 intraday volume pace 覆盖
- src/selection/magic_tdx.rs:794-803 — first valid slot 9:35
- Issue: Pre-open 集合竞价 volume + 9:30 open auction prints **被 construct 排除** 出 `cumulative_volume`。Correct for our pace metric, 但**早 09:25 涨停** → `intraday_volume_pace=0` → reject
- Impact: moderate (false negative on gap-up limit-up opens)

## 5. 完整性 (Completeness)

### F16. Universe **silently excludes 北交所** by prefix whitelist
- src/selection/magic_tdx.rs:854-863 (`normalized_equity_code`)
  - SH 只接 `600/601/603/605/688/689`
  - SZ 只接 `000/001/002/003/300/301`
- Issue: `92xxxx` 北交所 codes (per BR-131: 30% limit) **+** `8/4` 老三板 codes **全部 silent dropped 在 master normalization layer**. **不作为 exclusion reason 文档化**
- Impact: **severe** (整个 board segment 从 candidate universe 缺失，但 reviewers 看不到 "北交所" 任何 reject code)
- Fix: 加 北交所 prefix + 注入其 `30%` limit，或 emit `selection_universe_excluded_market` reason

### F17. 无 ST / 退市风险 / 新股 filter
- src/selection/admission.rs (entire file) 和 src/selection/quality.rs — **无 ST / `*ST` / `退` name check, 无 listing-day check, 无 delisting-risk check**
- Issue: ST 股 healthy MA alignment + volume ratio pass `admission-v1` cleanly. 5% daily limit 意味着 "5-day_return=10%" 不可能，但 4%-per-day grinders pass [0,20%] gate
- Impact: **severe** (ST 应在 `DEFAULT_EXCLUDED_BOARDS` 或 per-stock block list)
- Fix: 加 `security_master` field (or `name.contains("ST")`) filter emit `stock_type_excluded`

### F18. 无 low-price / market-cap / liquidity-of-record filter
- admission 忽略 `close` absolute level
- Issue: Sub-¥2 stocks, micro-caps, lightly-traded names pass ratio gates; **absolute price-vs-commission ratio 从未查**
- Impact: moderate
- Fix: 加 `MIN_PRICE_YUAN` threshold (e.g. 2.0)

### F19. 无 主力资金 / 板块轮动 input to admission
- src/selection/admission.rs 只 consume price+volume features
- src/decision/sector_score.rs:54-127 exists 但 **永不被 pipeline.rs 调**
- Issue: Admission 纯 per-stock。概念板块 主力 inflow 或 rotation tier **不 weight** admit decision。强股在弱板块 → admit
- Impact: moderate
- Fix: 加 `sector_tier` field to `RawSelectionFeatures` and gate/down-weight `Watch` / `Excluded` sectors

### F20. Event-type not differentiated
- src/selection/pipeline.rs:1040-1071 (`validate_event_gate`)
- Issue: 政策公告 / 行业新闻 / 个股公告 / 涨停板 / 行业轮动 / 主力异动 **all funnel through same gate**; `MarketEvent.event_type` 捕获但**不 influence admission**
- Impact: mild-moderate

## 6. 时间一致性 (Temporal consistency)

### F21. Look-ahead discipline 良好 — 无 future bars
- src/selection/features.rs:107-128 — baselines use `bars[count-6..count-1]` (excludes latest) for volume, `bars[count-6].close` (5 days ago) for five_day_return
- 强: per-bar pipeline

### F22. Intraday path **silently re-bases price features onto live quote**
- src/selection/pipeline.rs:1119-1138 — when `window=Intraday`, `price_vs_ma5/ma10/ma20` and `five_day_return` are **recomputed from `quote.price`**, while MA5/MA10/MA20 come from yesterday's daily close
- Issue: Stock gap-opens +10% → intraday `five_day_return = +10%` (passes [0,20%]) and `price_vs_ma20 ≈ 10%` (passes [0,15%]) **even though yesterday's daily close told different story**. **Live-only effect** — backtest 用 settled close 看不到
- Impact: **severe** for gap-up candidates (admission flap vs backtest)
- Fix: either 用 **previous daily close** (非 MA20) for `five_day_return`, 或 document explicitly

### F23. `settlement_at` 未显式 typed
- src/selection/outcome.rs:295 — `expected_d1 = next_trading_day(baseline.market_date)`
- D+1 settlement implicit (next trading day's close)。**无 field carrying expected settlement timestamp**; settlement 在 consume-time 计算
- Impact: mild
- Fix: persist `settlement_at: DateTime<Local>` on `T0CloseSnapshot`

### F24. Sessions (上午/下午) handled correctly，但 15:00 cutoff hardcoded
- src/selection/magic_tdx.rs:795-803, 808-828 — morning 9:35-11:30, afternoon 13:05-15:00
- Issue: Hardcoded; 若 exchange 改 session (unlikely but documented in v19.x), needs code change. **无 `exchange_calendar::session()` abstraction**
- Impact: mild

## 7. Critical gap — false positives / false negatives

### F25. False positive: ST stock 5%-per-day grind passes admission
- 组合 F8 + F17
- `adjacent_change_gt_20pct` 不 trigger (limit ±5% so any adjacent change ≤ 5%), **且无 ST filter**, so `evaluate_admission` returns `Admitted`. MA alignment achievable on 5-day grind
- Impact: **severe**
- Fix: Block by `SecurityIdentity.name` 含 `"ST"`, `"*ST"`, `"退"`

### F26. False negative: 北交所 30% move rejected as outlier
- F8 + F16 combined — `adjacent_change_gt_20pct` reject 25% 北交所 move, **AND** 北交所 codes 被 prefix excluded anyway
- Impact: moderate (won't even appear in rejections because they're filtered at master layer)

### F27. False negative: gap-up 涨停 09:25 fails `intraday_volume_pace`
- F15 + admission.rs:136-143
- First completed 5-min slot 09:35; `cumulative_volume ≈ 0` → `intraday_volume_pace ≈ 0`, < 1.0 → `intraday_volume_confirmation_failed`
- Impact: moderate (misses auction-driven breakouts)
- Fix: bypass intraday pace gate for first 5 minutes of session

### F28. False positive: data_provider 返回 data, selection timeouts (5s vs 3s)
- F5
- Backtest using `data_provider` (5s) 会 see candidate that live `selection` (3s) 分类为 `Unavailable` — **同 code path, 不同 outcome**
- Impact: **severe** (backtest P&L ≠ live P&L)

---

## Top-3 priorities

1. **F16 + F25 + F28** (combine): universe silently wrong (北交所 missing), STs slip through, 两个 magic_tdx adapter 时间分歧。**一起 = backtest and live observe different candidate sets** — 先 fix
2. **F8**: switch from flat ±20% → `BR-131` board-aware gap tolerance
3. **F22**: document (or remove) intraday re-basing of price features onto live quote
