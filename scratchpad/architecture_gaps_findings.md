# 架构设计漏洞 (Deep-Modules Audit, from Explore agent)

**整体定性**: `selection/` 和 `monitor/risk.rs` 是 genuine depth。**Push pipeline + data_provider layer 是 wide/shallow, with structural gaps**。

---

## 1. Module-Interface Depth & Gaps

### 1.1 — `push_l2::Template` trait 0 实现；L3 目录不存在
- src/push_l2/template.rs:22 `pub trait Template` — 全 crate 0 `impl Template for`
- `push_l3/` 目录不存在
- `push_templates.rs` (14,655 行, 143 个 `pub fn render_xxx`) 绕过 L2 ，每个 render_xxx 返回 flat `String`, 调用 `crate::notify::push_governor(&text, kind).await`
- Deep: `Render` trait in `push_l3` owns `RenderedText` + structured `Template` impls registered via `inventory`/`static tables`. 14k-line flat module 消失

### 1.2 — `push_l4::Dispatcher` 表面宽大但本质 3-state machine
- src/push_l4/dispatcher.rs:99-285. 公开: `new`, `reserve`, `reserve_with_identity`, `commit`, `commit_with_identity`, `rollback`, `dispatch` (deprecated but still `pub`), `clear_dedup`, `dedup_size`, `stats` + free function `sub_kind_dedup_key`
- 有用 contract: `reserve(event, cooldown, sub_kind) → Reserved | Deduped` + `commit`
- **6+ entry variants 应该 collapse 到 `DedupKey { kind, identity, sub_kind }` builder struct**

### 1.3 — `push_l5::GovernanceEngine` stateless struct around 1 if-chain
- src/push_l5/governance.rs:137-198
- `pub struct GovernanceEngine;` — zero-state struct
- `check()` 是 45-line if-chain (quiet_hour → frozen → data_mode → daily_limit)
- 模块 leak `LAST_FROZEN_WARN_TS: AtomicU64` + `unix_secs_at` (only used to make one warn line quieter)
- Deep: `GovernancePolicy` enum (`Strict | Quiet | Frozen | Degraded`) + 单一 `evaluate(policy, ctx, event) → Outcome` pure function

### 1.4 — `data_provider::mod.rs::DataProvider` trait is dead code
- src/data_provider/mod.rs:279-301
- `DataFetcherManager::new()` (line 313) 构造 `Vec<Box<dyn DataProvider>>` 但**只** call `provider.get_stock_name(code)` — `get_daily_data` 被 bypass 到 `fallback::fetch_kline_with_fallback`
- 5 providers 存在 (`MagicTdxProvider`, `GtimgProvider`, `HttpProvider`, `SinaProvider`, `BaostockProvider`)。**只 2 个** (`GtimgProvider`, `HttpProvider`) 真正 `impl DataProvider` — from `DataFetcherManager::new()` itself, which then **ignores** their `get_daily_data` impl
- **Real kline fallback hard-coded in `fallback.rs`**
- Deep: 单一 `KlineSource` trait `async fn fetch(&self, code: &str, days: usize) → Result<Vec<KlineData>>` + `SourceRegistry` (构造 from `Vec<Arc<dyn KlineSource>>`) own race-merge logic

### 1.5 — `bin/monitor/notify.rs::PushKind` 是 60-variant catalog + 5 parallel match methods
- src/bin/monitor/notify.rs:27-158 (enum) + :160-515 (`impl PushKind`)
- 1 new variant → 5+ match arms (`cooldown_secs`, `level`, `label`, `stable_template_id`, `is_active_spec_target`, `is_legacy_v17_5`, `is_low_priority_v17_6`)
- `DISPATCH_TABLE` (15 audit-marked rows, line 620, per mem `v17x-dispatch-table.md`) 想 externalize 但**"doesn't replace the 5 match blocks"** — 注释 line 596: *"Path D consistent: don't replace existing match methods"*. **Two parallel sources of metadata now coexist**
- Deep: 单 `static DISPATCH_TABLE: &[(PushKind, DispatchMeta)] = ...` 持有 all metadata; methods 变 `fn meta(self) → DispatchMeta { DISPATCH_TABLE[self as usize].1 }`

### 1.6 — `bin/monitor/notify.rs` 3,430 行 + entry surface 过宽
- 混 PushKind 目录 + delivery (CLI + Feishu + Magiclaw daemon) + token mgmt (`now_epoch_secs`, `parse_issue_token_output`) + config resolution (`resolve_magiclaw_bin`) + HTTP probing
- `grep -c "^pub fn" notify.rs` ~30
- Deep: split into `push_kind.rs` (catalog), `magiclaw_transport.rs` (CLI + daemon), `feishu_transport.rs` (HTTP webhook), `auth_token.rs` (token lifecycle)

### 1.7 — `bin/monitor/main.rs` 10,321 行 + 5 helper mods 21,440 行
- main.rs 10,321 + notify.rs 3,430 + push_templates.rs 14,655 + v14_adapter.rs 1,415 + l6_sink.rs 355 + daily_report_router.rs 188 + v17_sources.rs 1,397 = **21,440 across 6 sibling files**
- 所有 `mod xxx;` crate-private to `bin/monitor/` (not `pub mod`)
- `push_templates.rs` 143 `pub fn` 但 main.rs 只 import `crate::notify`, `crate::v14_adapter`, `crate::l6_sink`, `crate::push_templates` (alias `pt`)
- **无 real test surface, 无 documented interface, modules 不能被复用**
- Deep: `monitor_runtime::Runtime` struct owned by `main` takes `Vec<Box<dyn Dispatcher>>` + `Vec<Box<dyn Scheduler>>`; helper modules 变 library crates (`monitor-push`, `monitor-data`, `monitor-scheduler`) with explicit `pub use` surfaces

---

## 2. Seam Placement — `data_provider`

### 2.1 — Real fallback hard-coded 4-way race, 不是 adapter
- src/data_provider/fallback.rs:96-156. `fetch_kline_with_fallback` 直接 reference `MagicTdxProvider`, `SinaProvider`, `GtimgProvider`, `HttpProvider` — **no list/iterator over a trait**
- 加 5th source → edit this function + re-run 6 tests in `fallback.rs`
- `DataProvider` trait (mod.rs:279) was designed for this but isn't used here
- mod.rs:326-330 承认: *"v11 P0-2 commit 2: switched to shared fallback function, internal async, sync entry kept"*
- Deep: `let sources: [Arc<dyn AsyncKlineSource>; 4] = [...]` registered at startup; `FuturesUnordered::from_iter(...)` with same merge logic

### 2.2 — 3 providers bypass trait entirely
- src/data_provider/baostock_provider.rs:73 `pub struct BaostockProvider` 无 `impl DataProvider`
- src/data_provider/yahoo.rs:11 `pub fn fetch_quotes` (free function)
- src/data_provider/magic_tdx_t0.rs (free functions)
- **3 different adapter shapes**: (a) sync `get_daily_data(&self, code, days)` via `DataProvider`, (b) async `fetch_kline_raw(&self, code, days)`, (c) free functions `fetch_quotes(&[String])`
- **不能 share fallback path**
- Deep: 单 `AsyncKlineSource` trait; all 5 providers `impl AsyncKlineSource`. Baostock → post-close specialist in separate `PostCloseKlineSource` trait

---

## 3. Dependency Hygiene — `monitor` ↔ `decision` ↔ `selection`

### 3.1 — `monitor/risk.rs` ↔ `risk/` overlap, no shared abstraction
- src/monitor/risk.rs: `MarketRegime`, `PositionSizer`, `StopLoss`
- src/risk/stop_loss.rs reuse monitor's types via `use crate::monitor::risk::StopLoss` (risk/mod.rs:9-14)
- src/risk/stop_loss.rs:33 `pub fn check_stops(...)` returns `Vec<StopSignal>` from monitor namespace
- src/pipeline/position_tracker.rs:28 calls `use crate::risk::stop_loss::check_stops` then `use crate::monitor::risk::{MarketRegime, PositionSizer, StopLoss}` — **2 `use` statements for one risk domain**
- risk/mod.rs:3-14 文档 split as "monitor 做实时风险告警 / risk 做决策硬约束" but **types cross freely**
- Deep: move all 3 types to third crate `risk-types` both `monitor` and `risk` import; algorithmic distinction (compute vs gate) reflected in functions, not types

### 3.2 — `decision/decision_decide.rs::decide()` 0 production callers
- src/decision/decision_decide.rs:72 `pub fn decide(inputs: DecisionInputs) → FinalDecision`. 864 lines decision logic
- `grep "decision::decide\|decision::decision_decide::decide"` 只 function definition + tests at lines 217/236/252/269/279/289
- **无 `pub use` re-export + no production caller**
- **Deep well-tested algorithm that is dead code in live binary**
- Meanwhile bin/monitor/main.rs:5651 calls `decision::decision_decide::decisions_from_llm` (不同 function), which composes AI summaries — **DecisionInputs → FinalDecision pipeline never runs**
- Deep: Delete `decision_decide.rs` (keep `decisions_from_llm`) OR expose `decide()` as canonical entry that LLM/AI summaries feed into

### 3.3 — `risk::veto_chain` `pipeline::veto_rules` parallel veto systems
- src/risk/veto_chain.rs:83 `pub trait VetoRule` + :104 `pub struct VetoChain`
- src/pipeline/veto_rules.rs (separate file)
- risk/veto_chain.rs:14-17 docs "互补不冲突": VetoChain first, pipeline/veto_rules second
- **2 veto stacks, 2 rule sets, 2 evaluation surfaces, no shared composition point**
- Deep: 单 `policy::Pipeline` takes `Vec<Box<dyn Policy<Ctx = ...>>>` + threads single context

---

## 4. Selection Pipeline Depth

### 4.1 — `SelectionPipeline` 4 port traits 但 production wiring bypasses them
- src/selection/pipeline.rs:234-263 defines `SelectionMarketPort`, `SelectionConfigPort`, `SelectionRepositoryPort`, `SelectionAuditPort` as `pub(crate) trait`
- :1371-1483 `ProductionMarketPort`, `ProductionConfigPort`, `ProductionRepositoryPort`, `ProductionAuditPort` concrete structs
- `evaluate_market_events` (line 1485) hard-codes `Arc::new(ProductionRepositoryPort)` etc. **Traits only substituted in tests** (lines 1529-1683)
- **生产 "seam" is conceptual**: it would be possible to inject a fake, but nothing does
- 4 ports duplicate effort — `SelectionRepositoryPort` has 6 methods (line 245-256) all calling `with_repository` (24-line private helper line 1407)
- Deep: each port is 1-method trait (`trait MarketEvidence { async fn fetch(...) }`); pipeline takes `&dyn MarketEvidence` only where it actually needs variation. Audit + Repository stay concrete

### 4.2 — `append_rejection` called 5 times from `evaluate_inner` with 9-arg invocations
- src/selection/pipeline.rs:923-962 (`append_rejection` 9 params). Calls at lines 315, 416, 520, 548, 575, 604
- 每个 passes same `event`, `event_id`, `event_hash` + per-phase `reason_codes`/`rule_ids`/`retryable` + `chain`/`mention`/`market_batch_id`
- **9-arg signature + 6 callsites 在 600-line `evaluate_inner` = textbook missing domain object**
- Deep: `RejectionContext { event, chain, mention, market_batch_id, recorded_at }` built once per iteration. `pipeline.record_rejection(ctx, reason_codes, rule_ids, retryable)` 1-line

### 4.3 — `selection/outcome.rs::compute_t0_outcome` + `compute_d1_outcome` dead in production
- src/selection/outcome.rs:226 `pub fn compute_t0_outcome` + :282 `pub fn compute_d1_outcome` + :335 `pub fn compute_due_outcome`
- **Only caller: `settle_due_outcomes` (line 390), itself only called from `bin/monitor/selection_shadow.rs:114`**
- `selection_shadow.rs` gated behind `STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE` env (default true), but `compute_due_outcome` 从不被 `selection/pipeline.rs` 调
- Selection pipeline writes `SelectionAuditRecord` + `EventCompletion` + `SelectionBatchInput` (staged candidates), audit chain closes via `settle_due_outcomes` — **但 chain is shadow-mode-only**. **Outcome integration missing**
- Deep: `SelectionPipeline::evaluate` should, after committing visibility, schedule `OutcomeSettlementPort::settle_due(now)` for any committed run. Today loop is "ingest → evaluate → audit → visibility" with no outcome step

### 4.4 — `selection/relation.rs` + `selection/model.rs` deep impl 但 2-function surfaces
- src/selection/relation.rs 559 lines, public surface = `map_events` + `direct_mentions`
- src/selection/model.rs 264 lines, public surface = `SecurityMasterSnapshot::new`, `SecurityIdentity`, `DirectMentionEvidence`, `CandidateIdentity`
- Heavy lifting: ChainConfigSnapshot validation, hash-chained snapshots, identity-token boundary detection
- **pipeline.rs:1115-1244 reinvents features** (`features_for_record`, `t0_market_evidence`, `intraday_volume_evidence`) inline rather than calling into `features.rs`
- **Pipeline does both orchestration AND feature-engineering**; feature module only used for `compute_daily_features` (line 1119)
- Deep: `features::features_for_record(record, window)` should live in `features.rs`, not `pipeline.rs`. Move `t0_market_evidence` + `intraday_volume_evidence` (currently 90 lines, lines 1142-1244) into `features.rs` and export. `pipeline.rs` shrinks ~150 lines, gains real `features::FeatureSet` builder seam

---

## 5. Push Layers (L1, L2, L4, L5, L6, L7) — Real or Vestigial?

### 5.1 — 7-layer architecture 实际是 6 (no L3), L2 trait 无 users
- 目录: `push_l1/`, `push_l2/`, `push_l4/`, `push_l5/`, `push_l6/`, `push_l7/`. **`push_l3/` 不存在**
- docs reference `docs/architecture/v14.2-push-architecture.md` §3.3 (L3 Render) 但 directory absent
- Architecture = L1 (events) → L4 (dedup) → L5 (governance) → L6 (delivery) → L7 (analytics); **L2/L3 no executable counterpart in live binary**
- Deep: Either commit to L2 (write `impl Template for LimitUpTemplate { ... }` for each PushKind) 或 delete `push_l2/template.rs` + trait entirely

### 5.2 — `push_l6::sink.rs::SinkRouter::route` returns single `SinkResult::Ok` only if all sinks succeed
- src/push_l6/sink.rs:148-185
- Line 180: `if errors.is_empty() { SinkResult::Ok } else { SinkResult::Err(...) }`
- 若 `ConsoleSink` always succeeds + `MagiclawSink` fails → overall = `Err`
- **但 only real `Sink` registered is `MagiclawSink`** (l6_sink.rs:143); `ConsoleSink` for debug
- 生产 "all sinks" = 1 sink. **"failure isolation" property (lines 144-145 comment) real per-sink, but `route` then loses that signal**
- Deep: `route` returns `Vec<(sink_name, SinkResult)>` so dispatcher can decide

### 5.3 — `push_l7::SqliteStore` only persistent backend; `InMemoryStore` tests-only
- src/push_l7/sqlite_store.rs 824 lines
- `AnalyticsStore` trait (analytics.rs:85) implemented by both `InMemoryStore` and `SqliteStore`, but bin/monitor/v14_adapter.rs:73 **only constructs `SqliteStore`**
- **InMemoryStore branch of trait seam is dead code in production**
- Deep: Drop `InMemoryStore` and have `SqliteStore::open_in_memory()` for tests, 或 split trait into `RecordSink` (just `record()`) + `AnalyticsQuery`

### 5.4 — Actual L4 dispatcher governance pipeline 在 `v14_adapter.rs`, not in `push_l4/`
- src/bin/monitor/v14_adapter.rs: `v14_gate`, `v14_record_delivery`, `V14Stack` (holds Dispatcher + GovernanceEngine + SqliteStore), `signal_event_for_kind`, `default_profile_for_kind`, `current_governance_ctx`
- **`push_l4`/`push_l5`/`push_l7` library crates are building blocks**; actual push pipeline 在 binary
- **Library crate no running example** (only `bin/v14_e2e.rs` exercises it)
- Deep: `push_runtime::Pipeline` takes `&Dispatcher`, `&GovernanceEngine`, `&dyn AnalyticsStore` + exposes `submit(SignalEvent) → PushOutcome`. `v14_adapter.rs` becomes thin adapter

---

## 6. Notification & Risk Seam

### 6.1 — `account_mode` exists in 3 places
- src/bin/monitor/push_templates.rs:88 `pub enum AccountMode { Normal, ReduceOnly, Frozen }`
- src/risk/account_mode.rs:36 `pub struct PortfolioMetrics`
- src/risk/action_gate.rs:67 `pub enum AccountMode { ... }`
- push_templates.rs:1840, 1891, 2053, 12465, 12490, 12551, 12590, 12648, ... **all contain `use stock_analysis::risk::action_gate::AccountMode as LibAM;` + manual `From`/`Into` adapters**
- local copy exists because `push_templates.rs` predates `risk::action_gate` work + never migrated; comments line 86-87: *"PR1 (risk::account_mode::AccountState) merge later, add From"*
- Deep: **Delete `push_templates::AccountMode`, use `risk::action_gate::AccountMode` directly**

### 6.2 — `risk/veto_chain.rs` panic-catch wrapper 无 real production wiring
- src/risk/veto_chain.rs:115-160. `evaluate_all` uses `std::panic::catch_unwind(AssertUnwindSafe(|| rule.evaluate(ctx)))` to isolate rule panics
- **No production caller wires up a `VetoChain`**
- `veto_rules_live.rs` defines rules (BiasRate, MainFlow, FundamentalDeterioration) but `VetoChain::new(rules)` **never called from bin/monitor or pipeline**
- **80 lines panic-isolation infrastructure = pure scaffolding**

### 6.3 — `bin/monitor/notify.rs::deliver_and_record` 30-line "audit loop" owns 3 side-effects
- src/bin/monitor/notify.rs:1106-1192
- Function: (a) checks `runtime_delivery_audit_health`, (b) calls `push_wechat` 或 `l6_sink::sink_router().route()`, (c) calls `v14_record_delivery`, (d) calls `event::publish_delivery`, (e) calls `settle_dedup_after_delivery`, (f) aggregates audit errors
- **6 audit points in 86 lines**
- Each of (c)/(d)/(e) lives in different module. `deliver_and_record` is **only** place they sequenced
- Error-aggregation logic (`audit_errors.push(...); if !audit_errors.is_empty() { return SinkError }`) duplicates pattern from `v14_record_delivery`
- Deep: `PostDeliveryHook` trait with 3 impls (`AnalyticsHook`, `EventBusHook`, `DedupHook`). `deliver_and_record` becomes `for hook in hooks { hook.after(&event, delivered, channel)?; }`

---

## Summary Table (23 findings)

| # | File:Line | Gap | Deep alternative |
|---|-----------|-----|------------------|
| 1.1 | push_l2/template.rs:22 | Template trait 0 impls; push_l3/ missing | 1 Template impl per PushKind, static-registered |
| 1.2 | push_l4/dispatcher.rs:99 | 9 public methods; only reserve+commit matter | `try_acquire(key, cooldown) → Result<(), Deduped>` |
| 1.3 | push_l5/governance.rs:137 | Zero-state GovernanceEngine over 45-line if-chain | GovernancePolicy enum + pure evaluate() |
| 1.4 | data_provider/mod.rs:279 | DataProvider trait unused for kline fallback | KlineSource trait + SourceRegistry |
| 1.5 | bin/monitor/notify.rs:27 | 60-variant enum + 5 parallel match methods | 1 static DISPATCH_TABLE |
| 1.6 | bin/monitor/notify.rs (3430) | 1 file: catalog + transports + tokens | split by domain |
| 1.7 | main.rs (10321) + 21,440 | Shallow packaging, no public surface | monitor_runtime::Runtime |
| 2.1 | data_provider/fallback.rs:96 | Hard-coded 4-way race | Registry of Arc<dyn AsyncKlineSource> |
| 2.2 | baostock/yahoo/magic_tdx_t0 | 3 adapter shapes | 1 trait, all 5 impls |
| 3.1 | monitor/risk.rs ↔ risk/ | Overlapping types, 2 use per call site | risk-types shared crate |
| 3.2 | decision_decide.rs:72 | `decide()` 0 production callers | `decisions_from_llm` route through `decide()` |
| 3.3 | veto_chain.rs + pipeline/veto_rules | 2 parallel veto stacks | 1 policy::Pipeline with Box<dyn Policy> |
| 4.1 | pipeline.rs:234-263 | 4 port traits, only test substitutes | Single-method ports where variance is real |
| 4.2 | pipeline.rs:923 | append_rejection 9 args × 5 callsites | RejectionContext struct |
| 4.3 | outcome.rs:226,282,335 | Outcome dead in production | Pipeline schedules settle_due after commit |
| 4.4 | pipeline.rs:1115-1244 | Feature engineering inline in pipeline | Move into features.rs |
| 5.1 | push_l2/, push_l3/ | 6-layer, not 7 | Either build L2/L3 or delete trait |
| 5.2 | push_l6/sink.rs:148 | route returns Err if any sink fails | Per-sink result vector |
| 5.3 | push_l7/analytics.rs:114 | InMemoryStore dead code | SqliteStore::open_in_memory() |
| 5.4 | v14_adapter.rs:42 | Real push pipeline in binary, not library | push_runtime::Pipeline in library crate |
| 6.1 | push_templates.rs:88, risk/action_gate.rs:67 | 3 definitions of AccountMode | Delete push_templates copy |
| 6.2 | risk/veto_chain.rs:104 | VetoChain never instantiated | Wire pipeline/veto_rules through VetoChain |
| 6.3 | notify.rs:1106 | deliver_and_record 6 audit hooks by hand | PostDeliveryHook trait loop |

---

**Selection pipeline is deepest, most carefully factored. Push pipeline + data_provider are shallowest, with structural duplication that recent `DISPATCH_TABLE` work (mem `v17x-dispatch-table.md`) is starting to address but has not yet completed.**
