# Event-scoped Selection Shadow Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a shadow-only, event-scoped stock-selection pipeline that preserves real provider publication evidence, maps each event independently, uses Magic TDX as its sole security/market source, and records immutable T0/T0-close/D+1 evidence for honest backtesting.

**Architecture:** NewsAggregator first returns events together with per-feed completeness evidence. The selection module durably ingests eligible events, then independently evaluates each event through a validated chain-config snapshot, exact Magic TDX security identity, one market-data batch, pure quality/feature functions, and a two-phase audit/visibility handshake. The existing post-session scheduler appends settled outcomes; no selection result reaches a sink, TradingBus, paper trade, or order path.

**Tech Stack:** Rust 2021, Tokio `spawn_blocking`, `magic-tdx-rs`, Chrono, Serde/serde_json, SHA-256, Diesel + SQLite, fs2 file locking, existing compliance scripts.

---

## Preconditions and protected worktree

- Design: `docs/superpowers/specs/2026-07-23-event-scoped-selection-pipeline-design.md`
- Business rules: BR-155, BR-156, BR-157.
- Data red lines: 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.9, 2.10.
- Preserve without staging or rewriting the pre-existing T0 edits in:
  `src/bin/monitor/blocking_market_data.rs`,
  `src/bin/monitor/main.rs`,
  `src/bin/monitor/push_templates.rs`,
  `src/data_provider/magic_tdx_provider.rs`,
  `src/data_provider/mod.rs`,
  `src/decision/t0_advisor.rs`,
  and `src/data_provider/magic_tdx_t0.rs`.
- Where this plan requires a small `main.rs` integration hunk, verify the exact diff and stage only that hunk. Never stage the whole pre-existing dirty file.
- Production code has no mock branch. Test doubles implement narrow test-only ports and use `TEST_CODE_` identities.

## Planned file structure

| Path | Responsibility |
| --- | --- |
| `src/signal/market_event.rs` | Explicit provider publication evidence on the canonical event |
| `src/news/aggregator/mod.rs` | Per-feed source-attempt batch and legacy event-only wrapper |
| `src/news/aggregator/feed.rs` | Parse and preserve provider date/timestamp without substituting observed time |
| `src/selection/model.rs` | Selection identities, states, evidence, rejects, features, and outcomes |
| `src/selection/relation.rs` | Validated chain-config snapshot and exact direct-mention relationships |
| `src/selection/quality.rs` | Pure bar/quote continuity, freshness, split, and range validation |
| `src/selection/features.rs` | Pure raw MA/return/volume/5-minute feature calculation |
| `src/selection/magic_tdx.rs` | Sole production market source; create/use/drop Magic TDX inside blocking worker |
| `src/selection/audit.rs` | Strict locked SHA-256 JSONL hash chain and audit receipts |
| `src/selection/pipeline.rs` | Durable inbox, per-event orchestration, retry, and visibility handshake |
| `src/selection/outcome.rs` | T0 close and D+1 settlement from Magic TDX |
| `src/selection/report.rs` | Raw visible-sample backtest aggregation without success-rate claims |
| `src/selection/mod.rs` | Narrow public facade and production port construction |
| `src/database/selection.rs` | Append-only schema, persistence, due-work queries, visibility joins |
| `src/bin/monitor/selection_shadow.rs` | Monitor adapter, kill switch, structured logging |
| `src/bin/selection_backtest.rs` | Read-only operator report CLI |
| `src/bin/selection_live_probe.rs` | Read-only Magic TDX validation probe for Gate D |
| `src/bin/monitor/main.rs` | Two small calls: post-news shadow evaluation and post-session outcome settlement |
| `docs/business_rules.md` | Activate BR-155/156/157 only after all production paths exist |

## Task 1: Preserve real provider publication evidence

**Files:**
- Modify: `src/signal/market_event.rs`
- Modify: `src/news/aggregator/feed.rs`
- Modify: `src/news/aggregator/mod.rs`
- Modify: `src/bin/monitor/news_aggregator_init.rs`
- Modify: `src/news/dispatcher.rs`
- Modify: `src/news/impact.rs`
- Modify: `src/news/stock_mapper.rs`
- Modify: `src/opportunity/event_extractor/core.rs`

- [ ] **Step 1: Add failing provider-evidence tests**

Add tests with fixed `Local` timestamps:

```rust
#[test]
fn date_only_publication_keeps_provider_date_and_observed_occurrence() {
    let observed = local_at(2026, 7, 23, 8, 30, 0);
    let evidence = parse_source_time(Some("2026-07-23"), observed, "TEST_PROVIDER");
    assert_eq!(evidence.occurred_at, observed);
    assert_eq!(
        evidence.provider_publication,
        Some(ProviderPublication {
            published_on: NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
            published_at: None,
        })
    );
    assert!(!evidence.stale);
}

#[test]
fn missing_or_invalid_publication_is_not_inferred_from_observed_at() {
    let observed = local_at(2026, 7, 23, 8, 30, 0);
    for raw in [None, Some(""), Some("not-a-date")] {
        let evidence = parse_source_time(raw, observed, "TEST_PROVIDER");
        assert_eq!(evidence.provider_publication, None);
        assert!(evidence.stale);
    }
}

#[test]
fn legacy_market_event_deserializes_without_publication_evidence() {
    let event: MarketEvent = serde_json::from_value(legacy_market_event_json()).unwrap();
    assert_eq!(event.provider_publication, None);
}
```

- [ ] **Step 2: Run the focused tests and confirm RED**

Run:

```bash
cargo test --lib signal::market_event::tests -- --nocapture
cargo test --lib news::aggregator::feed::tests -- --nocapture
```

Expected: compilation fails because `ProviderPublication`, `parse_source_time`, and `provider_publication` do not exist.

- [ ] **Step 3: Add the explicit event contract**

Implement these types and field:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPublication {
    pub published_on: NaiveDate,
    pub published_at: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceTimeEvidence {
    occurred_at: DateTime<Local>,
    provider_publication: Option<ProviderPublication>,
    stale: bool,
}

pub struct MarketEvent {
    // existing fields remain unchanged
    #[serde(default)]
    pub provider_publication: Option<ProviderPublication>,
}
```

`parse_source_time` must preserve a full provider timestamp when supplied, preserve only `published_on` for date-only input, and return `None + stale=true` for missing, invalid, ambiguous-local-time, or future input. Update every repository `MarketEvent` struct literal to set either validated evidence or `None`; do not derive it from `occurred_at`.

- [ ] **Step 4: Run focused tests and all MarketEvent callers**

Run:

```bash
cargo test --lib signal::market_event -- --nocapture
cargo test --lib news::aggregator -- --nocapture
cargo check --all-targets
```

Expected: PASS; no missing-field compile errors remain.

- [ ] **Step 5: Commit only Task 1 files**

```bash
git add src/signal/market_event.rs src/news/aggregator/feed.rs src/news/aggregator/mod.rs
git add src/bin/monitor/news_aggregator_init.rs src/news/dispatcher.rs src/news/impact.rs
git add src/news/stock_mapper.rs src/opportunity/event_extractor/core.rs
git diff --cached --check
git commit -m "feat(news): preserve provider publication evidence"
```

## Task 2: Make aggregation completeness a typed data contract

**Files:**
- Modify: `src/news/aggregator/mod.rs`
- Modify: `src/bin/monitor/news_aggregator_init.rs`

- [ ] **Step 1: Add failing aggregation-batch tests**

Define a successful test feed and a failing test feed, then assert:

```rust
#[tokio::test]
async fn tick_batch_reports_every_feed_without_treating_failure_as_empty() {
    let aggregator = NewsAggregator::new(vec![
        Arc::new(TestFeed::success("ok", vec![test_event("TEST_CODE_000001")])),
        Arc::new(TestFeed::failure("down", "transport")),
    ]);
    let batch = aggregator.tick_batch(20).await;
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.source_attempts.len(), 2);
    assert!(!batch.sources_complete());
    assert!(matches!(
        &batch.source_attempts[1].status,
        FeedAttemptStatus::Failed { reason_code, .. }
            if reason_code == "feed_fetch_failed"
    ));
}

#[tokio::test]
async fn tick_batch_can_prove_verified_source_empty() {
    let aggregator = NewsAggregator::new(vec![
        Arc::new(TestFeed::success("empty-a", vec![])),
        Arc::new(TestFeed::success("empty-b", vec![])),
    ]);
    let batch = aggregator.tick_batch(20).await;
    assert!(batch.events.is_empty());
    assert!(batch.sources_complete());
}
```

- [ ] **Step 2: Run and confirm RED**

Run:

```bash
cargo test --lib news::aggregator::tests::tick_batch -- --nocapture
```

Expected: compilation fails because the batch types and method do not exist.

- [ ] **Step 3: Implement batch and compatibility wrapper**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedAttemptStatus {
    Succeeded { event_count: usize },
    Failed { reason_code: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedAttempt {
    pub feed_name: String,
    pub source_kind: String,
    pub status: FeedAttemptStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsAggregationBatch {
    pub events: Vec<MarketEvent>,
    pub source_attempts: Vec<FeedAttempt>,
    pub observed_at: DateTime<Local>,
}

impl NewsAggregationBatch {
    pub fn sources_complete(&self) -> bool {
        !self.source_attempts.is_empty()
            && self.source_attempts.iter().all(|attempt| {
                matches!(attempt.status, FeedAttemptStatus::Succeeded { .. })
            })
    }
}
```

Implement `tick_batch`. Keep `tick` as `self.tick_batch(limit).await.events` for existing callers. Failure messages must be bounded and must not include response bodies or URLs.

Change the monitor wrapper to:

```rust
pub async fn tick_news_aggregator_batch(per_feed_limit: usize) -> NewsAggregationBatch;
pub async fn tick_news_aggregator(per_feed_limit: usize) -> Vec<MarketEvent> {
    tick_news_aggregator_batch(per_feed_limit).await.events
}
```

When the global aggregator is absent, return an empty batch with one `Failed` attempt named `global_aggregator`; never return a source-complete empty batch.

- [ ] **Step 4: Run batch and compatibility tests**

Run:

```bash
cargo test --lib news::aggregator -- --nocapture
cargo test --bin monitor news_aggregator_init -- --nocapture
cargo check --bin monitor
```

Expected: PASS; existing event-only callers remain compatible.

- [ ] **Step 5: Commit**

```bash
git add src/news/aggregator/mod.rs src/bin/monitor/news_aggregator_init.rs
git diff --cached --check
git commit -m "feat(news): expose feed-complete aggregation batches"
```

## Task 3: Define selection model and deterministic event/chain identities

**Files:**
- Create: `src/selection/mod.rs`
- Create: `src/selection/model.rs`
- Create: `src/selection/relation.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add failing model and relation tests**

Cover:

```rust
#[test]
fn candidate_identity_changes_when_event_changes() {
    let first = CandidateIdentity::new(
        "event-a", "chain-semiconductor", "TEST_CODE_000001",
        "direct-v1", "feature-v1", test_date(),
    );
    let second = CandidateIdentity::new(
        "event-b", "chain-semiconductor", "TEST_CODE_000001",
        "direct-v1", "feature-v1", test_date(),
    );
    assert_ne!(first, second);
}

#[test]
fn two_events_are_mapped_independently() {
    let snapshot = chain_snapshot(&[
        test_rule("chip", 100, &["芯片"]),
        test_rule("gold", 90, &["黄金"]),
    ]);
    let mapped = map_events(
        &[test_event("芯片扩产 TEST_CODE_000001"),
          test_event("黄金涨价 TEST_CODE_000002")],
        &snapshot,
    );
    assert_eq!(mapped[0].chains, vec!["chip"]);
    assert_eq!(mapped[1].chains, vec!["gold"]);
}

#[test]
fn direct_mention_requires_exact_unique_magic_tdx_identity() {
    let master = test_master(&[
        ("TEST_CODE_000001", "测试甲"),
        ("TEST_CODE_000002", "测试乙"),
    ]);
    assert_eq!(
        direct_mentions("测试甲(TEST_CODE_000001)中标", &master)
            .unwrap()
            .iter()
            .map(|entry| entry.code.as_str())
            .collect::<Vec<_>>(),
        vec!["TEST_CODE_000001"]
    );
    assert!(direct_mentions("测试", &master).unwrap().is_empty());
}
```

Also test duplicate chain IDs, empty keywords, ambiguous duplicate company names, deterministic sorting, multiple chain matches without Top-N truncation, `BoardMembership` research-only, and `AiProposed` rejection.

- [ ] **Step 2: Run and confirm RED**

Run:

```bash
cargo test --lib selection::model -- --nocapture
cargo test --lib selection::relation -- --nocapture
```

Expected: compilation fails because `selection` does not exist.

- [ ] **Step 3: Implement the narrow domain model**

Use strong states:

```rust
pub enum SelectionRunOutcome {
    Completed(SelectionBatch),
    VerifiedEmpty(VerifiedEmptySelection),
    Unavailable(SelectionUnavailable),
}

pub enum RelationEvidence {
    DirectMention(DirectMentionEvidence),
    BoardMembership(BoardMembershipEvidence),
    AiProposed(AiProposedEvidence),
}

impl RelationEvidence {
    pub fn formal_candidate_allowed(&self) -> bool {
        matches!(self, Self::DirectMention(_))
    }
}

pub enum RejectReasonCode {
    MissingProviderPublication,
    StaleEvent,
    FuturePublication,
    IncompleteSourceBatch,
    InvalidChainConfig,
    NoExactSecurityIdentity,
    AmbiguousSecurityIdentity,
    MagicTdxUnavailable,
    MarketDataRejected,
    PersistenceUnavailable,
    AuditUnavailable,
}
```

Implement SHA-256 domain-separated IDs and canonical content hashes. `ChainConfigSnapshot::from_rules` builds a canonical local serializable representation from `ChainRuleConfig`, validates it, and hashes the complete ordered snapshot. Do not use the old mapper’s disk/compiled fallback.

- [ ] **Step 4: Implement exact relation behavior**

For each event:

1. reject missing `provider_publication`, stale, future, or empty provider/title;
2. match only that event’s text against every enabled validated chain rule;
3. preserve all distinct matches sorted by priority descending then chain ID;
4. find only complete exact six-digit codes and exact unique security names in the Magic TDX master snapshot;
5. make only `DirectMention` formal.

Do not call `fetch_flash_titles`, `map_news_to_chains_ai`, or `resolve_stocks`.

- [ ] **Step 5: Run and commit**

Run:

```bash
cargo test --lib selection::model -- --nocapture
cargo test --lib selection::relation -- --nocapture
cargo check --lib
```

Expected: PASS.

```bash
git add src/lib.rs src/selection/mod.rs src/selection/model.rs src/selection/relation.rs
git diff --cached --check
git commit -m "feat(selection): add event-scoped relation model"
```

## Task 4: Implement pure quote/bar quality gates and raw features

**Files:**
- Create: `src/selection/quality.rs`
- Create: `src/selection/features.rs`
- Modify: `src/selection/mod.rs`
- Modify: `src/selection/model.rs`

- [ ] **Step 1: Add failing quality tests**

Use fixed typed bars:

```rust
#[test]
fn rejects_non_positive_price_duplicate_day_and_large_unexplained_jump() {
    assert_eq!(validate_daily(&bars_with_close(0.0)).unwrap_err().code(), "price_non_positive");
    assert_eq!(validate_daily(&bars_with_duplicate_day()).unwrap_err().code(), "duplicate_bar");
    assert_eq!(validate_daily(&bars_with_return(0.205)).unwrap_err().code(), "adjacent_change_gt_20pct");
}

#[test]
fn rejects_stale_intraday_quote_after_five_seconds() {
    let now = fixed_local_time();
    let quote = quote_observed_at(now - chrono::Duration::seconds(6));
    assert_eq!(validate_quote(&quote, now).unwrap_err().code(), "quote_stale");
}
```

Add tests for time gaps, high/low/open/close bounds, nonfinite amount/volume, invalid split continuity, incomplete settled day, and 20.0% boundary behavior.

- [ ] **Step 2: Run and confirm RED**

Run:

```bash
cargo test --lib selection::quality -- --nocapture
```

Expected: compilation fails because the module and validators do not exist.

- [ ] **Step 3: Implement quality gates**

Create normalized types:

```rust
pub struct SelectionQuote {
    pub code: String,
    pub price: f64,
    pub previous_close: f64,
    pub observed_at: DateTime<Local>,
    pub source_at: DateTime<Local>,
    pub volume: f64,
    pub amount: f64,
}

pub struct SelectionBar {
    pub code: String,
    pub started_at: DateTime<Local>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub settled: bool,
}
```

Reject any violation explicitly. Daily continuity uses the real trading calendar, not calendar-day adjacency. A change outside ±20% is a failure requiring manual confirmation; no automatic exception is inferred from symbol prefixes.

- [ ] **Step 4: Add failing feature tests**

```rust
#[test]
fn computes_raw_daily_features_from_twenty_one_settled_bars() {
    let features = compute_daily_features(&linear_bars(21)).unwrap();
    assert_eq!(features.ma5, Some(19.0));
    assert_eq!(features.ma10, Some(16.5));
    assert_eq!(features.ma20, Some(11.5));
    assert_eq!(features.five_day_return, Some(5.0 / 16.0));
}

#[test]
fn missing_volume_denominator_stays_missing_and_blocks_formal_feature_set() {
    let error = compute_daily_features(&bars_with_zero_prior_volume()).unwrap_err();
    assert_eq!(error.code(), "volume_baseline_missing");
}
```

- [ ] **Step 5: Implement raw feature calculation**

Return:

```rust
pub struct RawSelectionFeatures {
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub five_day_return: Option<f64>,
    pub volume_vs_5d: Option<f64>,
    pub volume_vs_20d: Option<f64>,
    pub intraday_volume_pace: Option<f64>,
    pub price_vs_ma5: Option<f64>,
    pub price_vs_ma10: Option<f64>,
    pub price_vs_ma20: Option<f64>,
}
```

Require all formal-candidate features specified by the design. Missing denominators remain `None` and return a structured reject; never substitute zero, one, a score, or a probability.

- [ ] **Step 6: Run and commit**

```bash
cargo test --lib selection::quality -- --nocapture
cargo test --lib selection::features -- --nocapture
cargo check --lib
git add src/selection/mod.rs src/selection/model.rs src/selection/quality.rs src/selection/features.rs
git diff --cached --check
git commit -m "feat(selection): validate and derive raw market features"
```

## Task 5: Add the Magic TDX-only selection adapter

**Files:**
- Create: `src/selection/magic_tdx.rs`
- Modify: `src/selection/mod.rs`
- Modify: `src/selection/model.rs`

- [ ] **Step 1: Add failing normalization and boundary tests**

Test pure conversion from `magic_tdx_rs` protocol records:

```rust
#[test]
fn normalizes_security_master_without_guessing_names() {
    let snapshot = normalize_master(
        test_observed_at(),
        vec![security_info("000001", "平安银行")],
    ).unwrap();
    assert_eq!(snapshot.by_code("000001").unwrap().name, "平安银行");
}

#[test]
fn rejects_quote_when_server_time_cannot_be_proven() {
    let error = normalize_quote(test_quote_with_server_time(""), test_observed_at()).unwrap_err();
    assert_eq!(error.code(), "quote_source_time_missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn blocking_boundary_returns_without_nested_runtime_drop() {
    let thread_id = run_magic_tdx_blocking(|| Ok(std::thread::current().id()))
        .await
        .unwrap();
    assert_ne!(thread_id, std::thread::current().id());
}
```

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib selection::magic_tdx -- --nocapture
```

Expected: compilation fails because the adapter does not exist.

- [ ] **Step 3: Implement one blocking ownership boundary**

Expose:

```rust
pub async fn fetch_selection_market_batch(
    request: SelectionMarketRequest,
) -> Result<SelectionMarketBatch, SelectionSourceError> {
    tokio::task::spawn_blocking(move || fetch_selection_market_batch_blocking(request))
        .await
        .map_err(SelectionSourceError::join)?
}
```

Inside `fetch_selection_market_batch_blocking`:

1. create `TdxSmartClient`;
2. connect once with a bounded timeout;
3. create/use `TdxService`;
4. read Shanghai and Shenzhen security lists;
5. resolve exact requested identities;
6. fetch quotes only when the request is intraday;
7. fetch 21+ settled daily bars and required 5-minute bars in the same worker;
8. normalize and validate all source times;
9. compute a deterministic Magic TDX batch ID;
10. drop service/client before returning from the worker.

No fallback provider, no nested Tokio runtime, no reference to `rustdx`, and no dependency on the uncommitted `magic_tdx_t0.rs`.

- [ ] **Step 4: Make unsupported capabilities explicit**

Beijing security-list coverage and board-membership research remain explicit `CapabilityUnavailable` in this slice. They must not be populated from another provider or an invented endpoint. Formal `DirectMention` evaluation for supported Shanghai/Shenzhen securities remains available.

- [ ] **Step 5: Run and commit**

```bash
cargo test --lib selection::magic_tdx -- --nocapture
cargo check --all-targets
git add src/selection/mod.rs src/selection/model.rs src/selection/magic_tdx.rs
git diff --cached --check
git commit -m "feat(selection): add Magic TDX evidence adapter"
```

## Task 6: Create append-only selection persistence and durable inbox

**Files:**
- Create: `src/database/selection.rs`
- Modify: `src/database/mod.rs`

- [ ] **Step 1: Add failing schema and immutability tests**

Use an in-memory SQLite connection and real test migrations:

```rust
#[test]
fn selection_tables_reject_updates_and_deletes() {
    let mut conn = test_connection();
    DatabaseManager::run_migrations_for_test(&mut conn).unwrap();
    insert_test_inbox(&mut conn);
    assert!(sql_query("UPDATE selection_event_inbox SET content_hash='changed'")
        .execute(&mut conn).is_err());
    assert!(sql_query("DELETE FROM selection_event_inbox")
        .execute(&mut conn).is_err());
}

#[test]
fn same_identity_same_hash_is_idempotent_but_conflicting_hash_fails() {
    let mut repository = test_repository();
    assert_eq!(repository.ingest(test_inbox("hash-a")).unwrap(), Inserted::Yes);
    assert_eq!(repository.ingest(test_inbox("hash-a")).unwrap(), Inserted::No);
    assert_eq!(
        repository.ingest(test_inbox("hash-b")).unwrap_err().code(),
        "identity_content_conflict"
    );
}

#[test]
fn production_query_hides_staged_candidate_without_visibility_receipt() {
    let mut repository = test_repository();
    repository.stage(test_batch()).unwrap();
    assert!(repository.visible_candidates(test_date()).unwrap().is_empty());
}
```

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib database::selection -- --nocapture
```

Expected: compilation fails because the module/schema does not exist.

- [ ] **Step 3: Create the schema**

Create all seven append-only tables:

```text
selection_event_inbox
selection_event_completions
selection_runs
selection_candidates
selection_feature_snapshots
selection_outcomes
selection_visibility_receipts
```

Every identity and content hash is `NOT NULL`; truly absent provider/source values are nullable. Add foreign keys, uniqueness constraints, due-work indexes, and `BEFORE UPDATE/DELETE ... RAISE(ABORT, '... is append-only')` triggers for every table.

- [ ] **Step 4: Implement transactional repository operations**

Implement:

```rust
pub fn ingest_event(&mut self, event: &InboxEvent) -> Result<InsertReceipt, SelectionDbError>;
pub fn pending_events(&mut self, limit: usize) -> Result<Vec<InboxEvent>, SelectionDbError>;
pub fn stage_batch(&mut self, batch: &SelectionBatch) -> Result<StageReceipt, SelectionDbError>;
pub fn publish_visibility(&mut self, receipt: &VisibilityReceipt) -> Result<(), SelectionDbError>;
pub fn append_completion(&mut self, completion: &EventCompletion) -> Result<(), SelectionDbError>;
pub fn due_outcomes(&mut self, as_of: NaiveDate) -> Result<Vec<DueOutcome>, SelectionDbError>;
pub fn append_outcome(&mut self, outcome: &SelectionOutcome) -> Result<(), SelectionDbError>;
pub fn visible_samples(&mut self, filter: &ReportFilter) -> Result<Vec<VisibleSample>, SelectionDbError>;
```

All inserts use bound parameters. `visible_samples` and every production candidate query must inner-join `selection_visibility_receipts`.

- [ ] **Step 5: Run and commit**

```bash
cargo test --lib database::selection -- --nocapture
cargo test --lib database -- --nocapture
git add src/database/mod.rs src/database/selection.rs
git diff --cached --check
git commit -m "feat(selection): add immutable shadow repository"
```

## Task 6A: Add the formal-candidate admission gate

**Files:**
- Create: `src/selection/admission.rs`
- Modify: `src/selection/mod.rs`

- [ ] **Step 1: Add failing hard-exclusion tests**

Prove that a complete, directly related security is still rejected when trend, momentum, overextension, settled-volume confirmation, or intraday same-slot pace fails. Also prove every missing/non-finite required feature is rejected and all applicable reason codes are returned in deterministic order.

```rust
#[test]
fn weak_or_overextended_security_is_rejected_not_staged() {
    let decision = evaluate_admission(
        SelectionEvaluationWindow::PostClose,
        &features_with_downtrend_and_weak_volume(),
    );
    assert!(matches!(decision, AdmissionDecision::Rejected(_)));
    assert!(decision.reason_codes().contains(&"trend_alignment_failed"));
    assert!(decision.reason_codes().contains(&"settled_volume_confirmation_failed"));
}

#[test]
fn intraday_requires_real_same_slot_volume_confirmation() {
    let decision = evaluate_admission(
        SelectionEvaluationWindow::Intraday,
        &features_without_intraday_pace(),
    );
    assert_eq!(decision.reason_codes(), ["intraday_volume_pace_missing"]);
}
```

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib selection::admission -- --nocapture
```

- [ ] **Step 3: Implement `admission-v1`**

Use the exact boundaries in design §6.4 / BR-156. Return `Admitted` or a deterministic structured rejection containing every failed rule. Do not calculate a score, probability, rank, or implicit default.

- [ ] **Step 4: Add the pipeline exclusion contract**

Task 8 must prove rejected securities are absent from staged candidates, visibility receipts, due outcomes, `visible_samples`, and report/backtest inputs while their structured rejection remains in the authoritative audit/completion summary.

- [ ] **Step 5: Run and commit**

```bash
cargo test --lib selection::admission -- --nocapture
cargo test --lib selection -- --nocapture
git add src/selection/mod.rs src/selection/admission.rs
git diff --cached --check
git commit -m "feat(selection): reject weak formal candidates"
```

## Task 7: Add the locked authoritative selection audit

**Files:**
- Create: `src/selection/audit.rs`
- Modify: `src/selection/mod.rs`
- Modify: `src/selection/model.rs`

- [ ] **Step 1: Add failing audit tests**

Cover:

```rust
#[test]
fn prepared_then_committed_returns_chain_hash_receipt() {
    let writer = test_writer();
    let prepared = writer.append(test_record(SelectionAuditPhase::Prepared)).unwrap();
    let committed = writer.append(test_record_with_previous(
        SelectionAuditPhase::Committed,
        prepared.record_hash.clone(),
    )).unwrap();
    assert_eq!(committed.previous_hash, Some(prepared.record_hash));
}

#[test]
fn corrupted_tail_or_unknown_field_blocks_append() {
    let writer = test_writer();
    writer.append(test_record(SelectionAuditPhase::Prepared)).unwrap();
    corrupt_last_line(writer.path());
    assert_eq!(writer.append(test_record(SelectionAuditPhase::Committed))
        .unwrap_err().code(), "audit_chain_invalid");
}
```

Also test missing final newline, hash mismatch, test/production physical paths, and two concurrent writers producing a valid serialized chain.

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib selection::audit -- --nocapture
```

Expected: compilation fails because the audit writer does not exist.

- [ ] **Step 3: Implement strict records and writer**

Use:

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionAuditRecord {
    pub schema_version: u16,
    pub domain: String,
    pub phase: SelectionAuditPhase,
    pub subject_id: String,
    pub content_hash: String,
    pub previous_hash: Option<String>,
    pub recorded_at: DateTime<FixedOffset>,
    pub record_hash: String,
}

pub struct AuditAppendReceipt {
    pub record_hash: String,
    pub previous_hash: Option<String>,
}
```

Hold an fs2 exclusive lock across full-chain read, validation, append, flush, and `sync_data`. Reject partial lines, unknown fields, bad domains, hash mismatches, or missing newline. Use separate production and test paths and do not implement deletion/rewriting.

- [ ] **Step 4: Run and commit**

```bash
cargo test --lib selection::audit -- --nocapture
cargo check --lib
git add src/selection/mod.rs src/selection/model.rs src/selection/audit.rs
git diff --cached --check
git commit -m "feat(selection): add authoritative hash-chain audit"
```

## Task 8: Orchestrate durable ingestion, evaluation, retry, and visibility

**Files:**
- Create: `src/selection/pipeline.rs`
- Modify: `src/selection/mod.rs`
- Modify: `src/selection/model.rs`
- Modify: `src/database/selection.rs`

- [ ] **Step 1: Add failing pipeline tests with narrow ports**

Define test-only implementations of `SelectionRepositoryPort`, `SelectionMarketPort`, and `SelectionAuditPort`. Prove:

```rust
#[tokio::test]
async fn source_failure_never_becomes_verified_empty() {
    let outcome = test_pipeline()
        .evaluate(batch_with_failed_feed_and_no_events())
        .await;
    assert!(matches!(outcome, SelectionRunOutcome::Unavailable(_)));
}

#[tokio::test]
async fn magic_tdx_failure_keeps_event_pending_for_retry() {
    let harness = test_pipeline_with_market_failure("transport");
    let first = harness.pipeline.evaluate(complete_batch_with_direct_event()).await;
    assert!(matches!(first, SelectionRunOutcome::Unavailable(_)));
    assert_eq!(harness.repository.pending_count(), 1);
    assert_eq!(harness.repository.completion_count(), 0);
}

#[tokio::test]
async fn candidate_is_invisible_until_committed_audit_and_receipt() {
    let harness = test_pipeline_with_audit_failure(SelectionAuditPhase::Committed);
    let outcome = harness.pipeline.evaluate(complete_batch_with_direct_event()).await;
    assert!(matches!(outcome, SelectionRunOutcome::Unavailable(_)));
    assert_eq!(harness.repository.staged_count(), 1);
    assert_eq!(harness.repository.visible_count(), 0);
}
```

Also prove event isolation, idempotent retry, permanent stale rejection completion, verified-empty only with complete feeds, single-ticket isolation, hard-rejected securities never enter staged/visible/outcome/report samples, and no sink/trading port in the pipeline type.

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib selection::pipeline -- --nocapture
```

Expected: compilation fails because the pipeline and ports do not exist.

- [ ] **Step 3: Implement the orchestration sequence**

The public production flow must execute:

```text
validate aggregation batch identity
→ append Ingested audit
→ idempotently persist eligible events in selection_event_inbox
→ load pending events
→ load and validate one chain-config snapshot
→ obtain one Magic TDX security-master/market batch in spawn_blocking
→ map every event independently
→ run quality and raw feature functions
→ apply admission-v1 and discard every rejected security from the formal batch
→ append Prepared audit
→ stage run/candidates/features in one SQLite transaction
→ append Committed audit
→ append selection_visibility_receipt
→ append terminal event completions
→ return Completed or VerifiedEmpty
```

Any retryable source, Magic TDX, database, or audit failure leaves the inbox event pending. A permanent event-gate rejection appends a structured completion and audit record. A batch with incomplete feed evidence cannot be `VerifiedEmpty`.

- [ ] **Step 4: Add production facade**

Expose only:

```rust
pub async fn evaluate_market_events(
    batch: SelectionEventBatch,
    context: SelectionContext,
) -> SelectionRunOutcome;
```

`SelectionEventBatch::try_from(NewsAggregationBatch)` preserves every source attempt and observed time. Production facade constructs the real repository, Magic TDX adapter, audit writer, and validated config source; tests construct ports directly.

- [ ] **Step 5: Run and commit**

```bash
cargo test --lib selection::pipeline -- --nocapture
cargo test --lib selection -- --nocapture
cargo check --all-targets
git add src/selection/mod.rs src/selection/model.rs src/selection/pipeline.rs src/database/selection.rs
git diff --cached --check
git commit -m "feat(selection): evaluate durable event-scoped candidates"
```

## Task 9: Wire shadow evaluation after existing news governance

**Files:**
- Create: `src/bin/monitor/selection_shadow.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/news_aggregator_init.rs`

- [ ] **Step 1: Add failing monitor-adapter tests**

Add source-structure and behavior tests:

```rust
#[test]
fn invalid_kill_switch_fails_closed() {
    assert_eq!(parse_selection_shadow_enable(Some("invalid")), Err(KillSwitchError));
}

#[test]
fn shadow_adapter_has_no_push_or_trade_capability() {
    let source = include_str!("selection_shadow.rs");
    for forbidden in ["push_wechat", "SinkRouter", "TradingBus", "paper_trades", "place_order"] {
        assert!(!source.contains(forbidden), "forbidden capability: {forbidden}");
    }
}
```

Extend the existing `main.rs` source test to assert that `push_flash_decisions` textually precedes `selection_shadow::evaluate_news_batch` in the critical-flash block.

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --bin monitor selection_shadow -- --nocapture
```

Expected: compilation fails because the adapter and call do not exist.

- [ ] **Step 3: Implement kill switch and adapter**

`STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE` defaults to enabled when absent, accepts only `1/true` and `0/false`, and fails closed on any other value. The adapter converts the typed aggregation batch, calls the selection facade, and logs only status, counts, reason codes, and identity hashes.

- [ ] **Step 4: Integrate after critical/aggregate governance**

Change the existing news block to:

```rust
let news_batch = news_aggregator_init::tick_news_aggregator_batch(20).await;
let decisions = news_flash_gate.process(
    &news_batch.events,
    chrono::Local::now(),
    threshold,
    max_per_day,
);
push_flash_decisions(decisions).await;
selection_shadow::evaluate_news_batch(news_batch).await;
```

Keep the selection call outside the existing push scope and after push completion. Do not change watchlist, announcement, seen, or push behavior.

- [ ] **Step 5: Remove the obsolete disabled opportunity scheduler**

Delete `last_opp_scan`, its interval calculation, and the `NewsOuterTickPhase::Opportunity` block that only logs `scan disabled=incomplete_source_contract`. Keep `opportunity::run_opportunity_scan` source code temporarily for historical tools, but leave no production caller or disabled parallel schedule in `news_monitor_loop`.

- [ ] **Step 6: Inspect and stage only the new main hunks**

```bash
git diff -- src/bin/monitor/main.rs
git diff --check
```

Stage only the selection integration and obsolete-scheduler removal hunks; verify the cached diff contains none of the pre-existing T0 edits.

- [ ] **Step 7: Run and commit**

```bash
cargo test --bin monitor selection_shadow -- --nocapture
cargo check --bin monitor
git add src/bin/monitor/selection_shadow.rs src/bin/monitor/news_aggregator_init.rs
git diff --cached --check
git commit -m "feat(monitor): run event selection in shadow mode"
```

## Task 10: Append T0-close and D+1 outcomes through the existing scheduler

**Files:**
- Create: `src/selection/outcome.rs`
- Modify: `src/selection/mod.rs`
- Modify: `src/database/selection.rs`
- Modify: `src/bin/monitor/selection_shadow.rs`
- Modify: `src/bin/monitor/main.rs`

- [ ] **Step 1: Add failing pure outcome tests**

```rust
#[test]
fn d1_outcome_uses_immutable_t0_baseline() {
    let outcome = compute_d1_outcome(&test_snapshot(10.0), &settled_bar(
        10.5, 11.5, 9.5, 11.0, 1_200_000.0,
    )).unwrap();
    assert_eq!(outcome.open_return, 0.05);
    assert_eq!(outcome.close_return, 0.10);
    assert_eq!(outcome.mfe, 0.15);
    assert_eq!(outcome.mae, -0.05);
}

#[test]
fn unsettled_or_missing_session_is_expected_wait_not_empty() {
    assert!(matches!(
        compute_due_outcome(&test_snapshot(10.0), None, test_clock()),
        OutcomeAttempt::ExpectedWait(_)
    ));
}
```

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib selection::outcome -- --nocapture
```

Expected: compilation fails because outcome functions do not exist.

- [ ] **Step 3: Implement settlement**

Use the real trading calendar to derive T0 and D+1. Fetch settled unadjusted daily evidence only through the Task 5 Magic TDX blocking adapter. Append unique `T0Close` and `D1Settled` rows; same content is idempotent, conflicting content fails. `ExpectedWait` writes no terminal outcome and remains due; source failure is retryable and audited.

- [ ] **Step 4: Integrate into the existing post-session scheduler**

Add:

```rust
if let Err(error) = selection_shadow::settle_due_outcomes(now).await {
    log::warn!(
        "[selection-shadow][BR-157] outcome settlement unavailable; retry remains eligible: {}",
        error.reason_code()
    );
}
```

Place it after the post-session window check and before account-dependent review tasks. Do not create another timer or scheduler.

- [ ] **Step 5: Add scheduler ownership test**

Extend the existing scheduler source test to assert there is exactly one selection outcome settlement call and that it occurs inside `post_session_review_scheduler`.

- [ ] **Step 6: Inspect, run, and commit**

```bash
cargo test --lib selection::outcome -- --nocapture
cargo test --bin monitor tests_post_session_review_scheduler -- --nocapture
cargo check --bin monitor
git diff -- src/bin/monitor/main.rs
git diff --check
```

Stage only the new scheduler hunk plus Task 10 files, then:

```bash
git diff --cached --check
git commit -m "feat(selection): settle T0 and D1 outcomes"
```

## Task 11: Add raw backtest reporting and a read-only live probe

**Files:**
- Create: `src/selection/report.rs`
- Create: `src/bin/selection_backtest.rs`
- Create: `src/bin/selection_live_probe.rs`
- Modify: `src/selection/mod.rs`

- [ ] **Step 1: Add failing report tests**

```rust
#[test]
fn report_groups_only_visible_samples_and_never_claims_success_rate() {
    let report = build_report(visible_samples(), ReportFilter::default()).unwrap();
    assert_eq!(report.groups[0].sample_count, 2);
    let rendered = render_text(&report);
    assert!(rendered.contains("样本数"));
    assert!(rendered.contains("收益中位数"));
    assert!(rendered.contains("MFE"));
    assert!(rendered.contains("MAE"));
    assert!(!rendered.contains("成功率"));
    assert!(!rendered.contains("胜率"));
}
```

- [ ] **Step 2: Run and confirm RED**

```bash
cargo test --lib selection::report -- --nocapture
```

Expected: compilation fails because the report module does not exist.

- [ ] **Step 3: Implement report and CLI**

Group visible T0/D+1 samples by provider, chain, relation kind, and feature bucket. Output sample count, median/quantile returns, MFE/MAE, and volume change. Missing outcomes are reported separately; no inferred label, success rate, recommendation, or automatic promotion decision.

The CLI supports exact date/provider/chain filters and is read-only:

```text
cargo run --bin selection_backtest -- --from 2026-07-23 --to 2026-08-23
```

- [ ] **Step 4: Implement the live probe**

The probe accepts explicit Shanghai/Shenzhen codes, calls the same production Magic TDX adapter, prints only identity, source times, batch ID, validated bar counts, and feature availability, and writes nothing:

```text
cargo run --bin selection_live_probe -- --code 600396
```

Reject an empty code list, invalid identity, test code in production, and unsupported market.

- [ ] **Step 5: Run and commit**

```bash
cargo test --lib selection::report -- --nocapture
cargo test --bin selection_backtest -- --nocapture
cargo test --bin selection_live_probe -- --nocapture
cargo check --all-targets
git add src/selection/mod.rs src/selection/report.rs src/bin/selection_backtest.rs src/bin/selection_live_probe.rs
git diff --cached --check
git commit -m "feat(selection): report raw shadow outcomes"
```

## Task 12: Activate business rules and complete Gate B/C/D validation

**Files:**
- Modify: `docs/business_rules.md`
- Modify: `docs/superpowers/specs/2026-07-23-event-scoped-selection-pipeline-design.md` only if implementation evidence needs a precise path correction
- Create: `docs/evidence/2026-07-23-event-scoped-selection-validation.md`

- [ ] **Step 1: Audit production references**

Run:

```bash
rg -n "rustdx|fetch_flash_titles|map_news_to_chains_ai|resolve_stocks|push_wechat|TradingBus|paper_trades|place_order" src/selection src/bin/monitor/selection_shadow.rs
rg -n "BR-155|BR-156|BR-157" src/selection src/database/selection.rs src/news/aggregator src/bin/monitor
```

Expected: no forbidden production dependency; every new rule appears in actual production modules.

- [ ] **Step 2: Change BR-155/156/157 from spec-only to active**

Only after all cited production paths exist, replace `📝 spec-only` with the repository’s active/registered status marker. Keep the full approved rule text and actual path list.

- [ ] **Step 3: Run Gate B formatting, lint, and tests**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

Expected: all commands exit 0.

- [ ] **Step 4: Run Gate C compliance**

```bash
bash tools/compliance/check.sh
```

If freshness fails, follow rule 2.4.1 exactly:

```bash
bash tools/one_shot/backfill_daily.sh
bash tools/compliance/check.sh
```

Expected: compliance exits 0. A network/source failure is reported as a blocker; it is not bypassed or replaced with fake data.

- [ ] **Step 5: Measure Gate D coverage**

```bash
cargo llvm-cov --workspace --all-targets --summary-only
cargo llvm-cov --workspace --all-targets --lcov --output-path target/selection-lcov.info
```

Record total coverage and the `src/selection/`, `src/database/selection.rs`, and monitor-adapter coverage in the evidence document. Required: repository unit coverage ≥80%, core selection/data/audit links ≥95%. Add real tests until thresholds pass.

- [ ] **Step 6: Run read-only live validation**

```bash
cargo run --bin selection_live_probe -- --code 600396
cargo run --bin monitor -- --review
cargo run --bin monitor
```

For the long-running monitor, observe at least one complete news tick and selection status, then stop with SIGINT and verify graceful shutdown. Evidence must show:

- provider publication is not inferred when absent;
- source failures are not `VerifiedEmpty`;
- Magic TDX batch/source times are present for evaluated supported securities;
- no selection push/order/paper-trade occurs;
- retryable unavailability remains pending;
- no nested-runtime-drop panic occurs.

The review command may legitimately report `ExpectedWait` outside its data window; it must not panic, fabricate data, or classify unavailable input as no-data.

- [ ] **Step 7: Write validation evidence**

Document exact command, exit code, counts, timestamp, Magic TDX batch IDs with identities redacted where required, coverage, known explicit capability exclusions, and rollback:

```bash
git revert --no-edit stock_analysis/master..feat/event-scoped-selection-shadow-pr
cargo build --release --bin monitor
```

- [ ] **Step 8: Commit Gate evidence**

```bash
git add docs/business_rules.md
git add -f docs/evidence/2026-07-23-event-scoped-selection-validation.md
git diff --cached --check
git commit -m "docs(selection): record release validation evidence"
```

## Task 13: Review and publish a clean PR

**Files:**
- No production-file changes unless review finds a defect; defects return to the applicable earlier task and validation gate.

- [ ] **Step 1: Review the complete feature diff**

```bash
git log --oneline --decorate --max-count=20
git diff --stat stock_analysis/master..HEAD
git diff --check stock_analysis/master..HEAD
```

Verify no pre-existing T0/user work was staged into selection commits.

- [ ] **Step 2: Create a clean PR branch from the remote base**

Because the current local branch contains unrelated local commits, create a temporary worktree from `stock_analysis/master`, cherry-pick only:

1. the selection design commit;
2. this plan commit;
3. Task 1–12 selection commits.

Compare the clean branch’s selection diff to the validated feature diff before pushing. Do not include unrelated T0 or local planning commits.

- [ ] **Step 3: Create the PR with mandatory evidence fields**

The PR body must contain:

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-07-23-event-scoped-selection-pipeline-design.md`
- design: Gate A approved 2026-07-23

### Data-Redlines
- [2.1] Production selection uses real NewsAggregator and Magic TDX only
- [2.2] Missing source/features remain absent or reject explicitly
- [2.3] Price, continuity, split, finite-value, and ±20% gates run before features
- [2.4] Quotes ≤5s; daily evidence ≤1 trading day and settled
- [2.7] Prepared/Committed locked hash-chain plus immutable visibility receipt
- [2.8] Persistence/audit functions perform real I/O
- [2.9] No threshold/config contradiction introduced
- [2.10] BR-155/156/157 registered

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| `MarketEvent` | adopt and harden | explicit provider publication |
| `NewsAggregator` | adopt and harden | typed source completeness |
| `run_opportunity_scan` | reject production caller | loses event identity |
| fallback providers | reject | Magic TDX-only evidence |
| `post_session_review_scheduler` | adopt | single post-close owner |

### Threshold-Proof
- No prediction/selection threshold added; phase 1 records raw evidence without Top-N.

### Business-Rules
- BR-155, BR-156, BR-157

### Rollback
- Before merge: `git revert --no-edit stock_analysis/master..feat/event-scoped-selection-shadow-pr`
- After merge: revert the recorded PR merge SHA, then run `cargo build --release --bin monitor`
```

- [ ] **Step 4: Check every PR checklist item**

Confirm Gate A/B/C/D evidence is linked, every checkbox is checked, CI passes, and blocking objections are zero. Do not merge while any gate is incomplete.
