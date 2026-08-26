# BR-174 Event Selection Evidence Closure Design

Status: Gate A draft — independent review required before implementation
Date: 2026-07-28
Scope: event-scoped shadow selection only
Rules: AGENTS 2.1 / 2.2 / 2.3 / 2.4 / 2.7 / 2.8 / 2.10; BR-137 / BR-155 / BR-156 / BR-157 / BR-159 / BR-164 / BR-171 / BR-174 / BR-176 / BR-177 / BR-178 / BR-180
Supersedes: the BR-155 board-membership `research_only` restriction and, only for schema-v2
`HardRejected` research samples, the BR-156 admitted-only outcome restriction in
`2026-07-23-event-scoped-selection-pipeline-design.md`; all visibility, push, trading and other
safety gates remain active

Design-hash status: the cross-process outcome-claim amendment passed independent Gate A review.
Gate B binds the exact bytes of this document through the frozen `DESIGN_SHA256` in
`src/data_gateway/outcome_daily_bars.rs`. Any later design-byte change requires a deliberate
hash rotation in the same implementation change; mixed old/new bindings are rejected.

## 1. Problem

The current formal selection path proves only exact company/code mentions, writes only admitted
candidates, and settles only T0 and D1. It therefore cannot answer four required questions:

1. Which securities are related to an event through a real provider board membership?
2. Why was each source-backed security admitted or rejected?
3. Did the hard rejection improve quality relative to the admitted cohort?
4. What happened at D3 and D5, not only at D1?

The legacy opportunity path cannot fill these gaps. It owns old business interfaces, may default
missing money-flow/K-line dimensions, and lacks the immutable evidence and visibility protocol of
the formal selection path. BR-174 extends the formal path and does not revive that legacy path.

## 2. Current-state evidence

These commands were run from the downstream repository on 2026-07-28. Reduced exact output is
pasted so the design does not rely on an uncitable prose summary.

```bash
git -C ../magic-market-data-rs rev-parse HEAD
# b2b68df78156df1d67824e5c44c0cb01b752f55a

rg -n 'magic-tdx-rs = .*rev = "b2b68' Cargo.toml
# 51:magic-tdx-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "b2b68df78156df1d67824e5c44c0cb01b752f55a", version = "=0.2.0" }

rg -n '^pub enum OutcomePhase|^    T0Close,|^    D1Settled,' src/database/selection.rs
# 132:pub enum OutcomePhase {
# 133:    T0Close,
# 134:    D1Settled,
```

```bash
rg -n "selection_rejections|append_rejection|SelectionAuditPhase::Rejected" \
  src/database/selection.rs src/selection
# src/selection/pipeline.rs:315:                self.append_rejection(
# src/selection/pipeline.rs:416:                    self.append_rejection(
# src/selection/pipeline.rs:520:                        self.append_rejection(
# src/selection/pipeline.rs:548:                            self.append_rejection(
# src/selection/pipeline.rs:576:                            self.append_rejection(
# src/selection/pipeline.rs:604:                            self.append_rejection(
# src/selection/pipeline.rs:923:    fn append_rejection(
# src/selection/pipeline.rs:938:                SelectionAuditPhase::Rejected,
# src/selection/pipeline.rs:1768:            .contains(&SelectionAuditPhase::Rejected));
# src/selection/pipeline.rs:2114:            .contains(&SelectionAuditPhase::Rejected));
# src/selection/audit.rs:372:    if record.phase == SelectionAuditPhase::Rejected
# src/selection/audit.rs:523:            reason_codes: if phase == SelectionAuditPhase::Rejected {
# src/selection/audit.rs:529:            retryable: (phase == SelectionAuditPhase::Rejected).then_some(false),
# src/selection/audit.rs:661:            .append(record(SelectionAuditPhase::Rejected, "TEST_CODE_candidate"))
# src/selection/audit.rs:742:        let complete = record(SelectionAuditPhase::Rejected, "TEST_CODE_rejected");
```

```bash
rg -n 'admitted_global_news: Vec::new' src/selection/pipeline.rs
# 1946:            admitted_global_news: Vec::new(),
# 1968:                admitted_global_news: Vec::new(),
# 1980:                admitted_global_news: Vec::new(),
# 1994:                admitted_global_news: Vec::new(),
# 2007:                admitted_global_news: Vec::new(),
# 2307:            admitted_global_news: Vec::new(),
```

```bash
rg -n "BoardConstituentProvider|board_constituents: true|MAX_DISCOVERY_LIMIT" \
  ../magic-market-data-rs/crates/magic-market-core/src/discovery.rs \
  ../magic-market-data-rs/crates/magic-tdx-rs/src/board_provider.rs
# ../magic-market-data-rs/crates/magic-market-core/src/discovery.rs:7:const MAX_DISCOVERY_LIMIT: u32 = 10_000;
# ../magic-market-data-rs/crates/magic-market-core/src/discovery.rs:222:    if limit.get() > MAX_DISCOVERY_LIMIT {
# ../magic-market-data-rs/crates/magic-market-core/src/discovery.rs:224:            "{family} limit must be at most {MAX_DISCOVERY_LIMIT}"
# ../magic-market-data-rs/crates/magic-market-core/src/discovery.rs:256:pub trait BoardConstituentProvider {
# ../magic-market-data-rs/crates/magic-tdx-rs/src/board_provider.rs:5:    AssetClass, BoardCategory, BoardConstituentProvider, BoardConstituentRequest, BoardDefinition,
# ../magic-market-data-rs/crates/magic-tdx-rs/src/board_provider.rs:74:            board_constituents: true,
# ../magic-market-data-rs/crates/magic-tdx-rs/src/board_provider.rs:113:impl BoardConstituentProvider for TdxBoardProvider {

sed -n '342,356p' ../magic-market-data-rs/crates/magic-tdx-rs/src/board_provider.rs
# fn finish<T>(&self, records: Vec<T>) -> Result<DataBatch<T>, TdxError> {
#     if records.is_empty() {
#         return Err(TdxError::InvalidData(
#             "TDX normalized board operation returned no records".into(),
#         ));

rg -n "evaluate_news_batch|settle_due_outcomes" src/bin/monitor/main.rs
# 4154:        match selection_shadow::settle_due_outcomes(now).await {
# 6301:            selection_shadow::evaluate_news_batch(news_batch).await;
```

```bash
rg -n "enum FeedAttemptStatus|VerifiedEmpty|sources_complete|seen_simhash" \
  src/news/aggregator/mod.rs src/news/aggregator/feed.rs
# src/news/aggregator/mod.rs:140:pub enum FeedAttemptStatus {
# src/news/aggregator/mod.rs:167:    pub fn sources_complete(&self) -> bool {
# src/news/aggregator/mod.rs:179:    seen_simhash: std::sync::Mutex<HashSet<u64>>,
# src/news/aggregator/mod.rs:186:            seen_simhash: std::sync::Mutex::new(HashSet::new()),
# src/news/aggregator/mod.rs:232:        let mut seen = match self.seen_simhash.lock() {
# src/news/aggregator/feed.rs:68:        GatewayBatch::VerifiedEmpty(_) => Ok(NewsFeedOutput::default()),

sed -n '232,247p' src/news/aggregator/mod.rs
#         let mut seen = match self.seen_simhash.lock() {
#             Ok(g) => g,
#             Err(p) => p.into_inner(),
#         };
#         all_events.retain(|e| {
#             let h = e.simhash;
#             if seen.contains(&h) {
#                 false
#             } else {
#                 seen.insert(h);
#                 true
#             }
#         });

rg -n 'PRAGMA synchronous = NORMAL|PRAGMA foreign_keys = ON' \
  src/database/mod.rs src/database/selection.rs
# src/database/mod.rs:109:        ("synchronous=NORMAL", "PRAGMA synchronous = NORMAL"),
# src/database/selection.rs:1557:        conn.batch_execute("PRAGMA foreign_keys = ON;")

rg -n 'board_change_pct: h.board_change_pct.unwrap_or\\(0.0\\)|fund_flow_pct.unwrap_or\\(0.0\\)|K线数据抓取失败，趋势结构按0分处理|vol_vs_20d_avg.unwrap_or\\(0.0\\)' \
  src/opportunity/mod.rs
# 1160:                board_change_pct: h.board_change_pct.unwrap_or(0.0),
# 1161:                board_main_net_pct: h.fund_flow_pct.unwrap_or(0.0),
# 1581:            .unwrap_or((0.0, "K线数据抓取失败，趋势结构按0分处理".to_string(), None));
# 1585:                let vol_ratio = sig.vol_vs_20d_avg.unwrap_or(0.0);

sed -n '35,52p' src/selection/pipeline.rs
# const RELATION_VERSION: &str = "direct-mention-v1";
# const PIPELINE_VERSION: &str = "event-selection-v1";
# const DEFAULT_PENDING_LIMIT: usize = 200;
#
# #[derive(Debug, Clone)]
# pub struct SelectionEventBatch {
#     events: Vec<MarketEvent>,
#     source_attempts: Vec<FeedAttempt>,
#     observed_at: DateTime<Local>,
#     batch_id: String,
#     content_hash: String,
# }
```

The pinned upstream proves a strict constituent batch with a 10,000 request bound, but has no
`total/truncated` field, no historical membership as-of, and maps empty to `InvalidData`. The
selection conversion currently ignores the source-bound batch and retains only lossy events. The
notification aggregator also advances an in-process simhash set before the durable selection inbox,
and its `Succeeded { event_count }` attempt erases `VerifiedEmpty` evidence. The monitor entry points
remain the owners; BR-174 deepens their dependencies without renaming them. Production pooled
connections currently select `synchronous=NORMAL` and do not globally enable/read back foreign-key
enforcement; the selection unit-test connection setting is not production proof. Gate B must first
make `foreign_keys=ON` and `synchronous=FULL` verified invariants on every pooled connection before
any v2 migration or receipt is considered durable.

## 3. Vocabulary and non-claims

### 3.1 ProviderBoardConstituent

`ProviderBoardConstituent` means only:

1. a governed event matched a versioned chain rule;
2. the rule's exact provider board name resolved uniquely to a canonical provider board code; and
3. the provider batch proved that the security is a constituent of that board.

It does **not** mean beneficiary, causal exposure, revenue exposure, expected winner, or investment
recommendation. User-visible text must use “事件关联板块成分股”.

### 3.2 Sample decisions

- `RelationAttempt`: an append-only acquisition attempt for either `DirectMention` or
  `ProviderBoardConstituent`; it may resolve or reject without occupying the logical relation
  identity.
- `EvaluationAttempt`: an append-only per-canonical-security market/feature attempt; evidence
  failure is queryable here and may later retry without changing a terminal sample.
- `Admitted`: complete relation and market evidence passed every BR-156 hard gate.
- `HardRejected`: complete relation and T0-comparable market evidence failed one or more hard gates.

Only `Admitted` and `HardRejected` are terminal samples. A relationship or raw security identity
that cannot be canonicalised remains a relation attempt. Once canonical identity exists, later
market/freshness/source failures remain evaluation attempts. Neither failure class fabricates a
sample or enters a return denominator.

For schema-v2, `Admitted` becomes shadow-visible only through its receipted
`selection_samples` row; it is never projected into the v1 `selection_candidates` table.
This is the BR-157 visibility invariant expressed on the v2 physical schema rather than a reuse of
the v1 foreign-key graph. BR-174 does not register or call any sink, recommendation or order port.
`HardRejected` is invisible research-only. Any future push or trading consumer requires a separate
Gate A.

## 4. Architecture and ownership

```text
AdmittedGlobalNewsBatch(GlobalNewsRecord + BatchEvidence)
  -> exact provider/batch/item source-fact identity
  -> V2IngressPrepared audit
  -> atomic source-batch/fact/attempt + BR-137/BR-155 ingress gate stage
  -> V2IngressCommitted audit + ingress receipt
  -> optional deterministic MarketEvent notification projection
  -> consume receipted IngressAdmitted facts only
  -> versioned chain exact-keyword mapping
  -> append DirectMention / ProviderBoardConstituent relation attempt
  -> canonical security identity
  -> explicit provider-board binding + BoardDataGateway strict constituents batch
  -> MagicTdxSelectionGateway market evidence
  -> BR-156 hard admission
  -> compute final decision in memory
  -> Prepared audit
  -> atomic SQLite stage of attempts + terminal samples/reasons
  -> Committed audit + commit receipt
       -> admitted sample becomes shadow-visible through the v2 receipt join
       -> hard rejection research cohort
  -> T0 / D1 / D3 / D5 settlement
  -> prospective admitted vs hard-rejected cohort report
```

Provider construction, blocking client lifecycle, wire types, capability checks and evidence
normalisation stay in `src/data_gateway/**`. `src/selection/**` consumes downstream fact types and
must not construct `TdxHqClient` or `TdxBoardProvider`.

The current aggregator API is split; `tick_news_aggregator_batch` is deleted after its callers
migrate because it mutates simhash before returning. The replacement ownership is:

```text
selection_shadow::recover_all_v2_runs
  -> drain config-activation/ingress/generation/outcome-claim/outcome envelope-only queue
  -> drain config-activation/ingress/generation/outcome-claim/outcome manifested-unreceipted queue
  -> recover lock-free active outcome claims in claim_enveloped_at/claim_id order
  -> require both queues empty; recovery error aborts the whole tick before provider work
  -> selection_shadow::require_current_config_activation
  -> news_aggregator_init::fetch_raw_global_news_batch
  -> UnifiedGlobalNewsFeed::fetch_raw (typed records/feed attempts only; no MarketEvent/simhash)
  -> selection_shadow::ingest_source_batch(&RawNewsAggregationBatch)
  -> require ingress receipt
  -> news_aggregator_init::project_notifications_after_ingress(receipted_batch)
       (deterministic MarketEvent projection + simhash mutation)
  -> NewsFlashGate / NewsAI
  -> selection_shadow::evaluate_receipted_pending
```

If source ingress cannot obtain a receipt, the tick emits a visible error and does not advance
notification simhash or call notification/NewsAI/selection evaluation for that batch. The next raw
provider acquisition remains retryable. This ordering prevents a successful notification side
effect from erasing an unpersisted selection fact. Type signatures prevent passing an
unreceipted `RawNewsAggregationBatch` to `project_notifications_after_ingress`.

`recover_all_v2_runs` is the first operation of every normal/review/test/canary selection tick and
the post-close settlement tick; no financial/news provider future is constructed or polled before
it clears all claimable recovery work and returns an empty-queues receipt. An exact provider replay
for a previously receipted lock-free active outcome claim is recovery work and may run inside this
step under §7.13.1; no **new** acquisition intent may start first. A lock-busy claim is proven
live-owned, is excluded from this process's recoverable set without a lease, and does not block
other subjects. The queue query spans all five run kinds and is not scoped to the next logical
subject. This makes BR-176 a global recovery barrier rather than a generation-only helper.

### 4.1 Source-bound news identity

`SelectionEventBatch::try_from` must consume `admitted_global_news` as the authoritative input;
retaining only `MarketEvent` is invalid under BR-166. `UnifiedGlobalNewsFeed` centralises one stable
projection identity function:

```text
event_id = sha256("BR166_GLOBAL_NEWS_EVENT_V1\0" + provider_source + "\0" + item_id)
```

Logical fact identity and acquisition identity are separate:

```rust
#[derive(Serialize)]
struct SourceFactKeyPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.source_fact_key.v1"
    provider_source: &'a str,
    item_id: &'a str,
}

#[derive(Serialize)]
struct SourceFactContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.source_content.v1"
    provider_source: &'a str,
    item_id: &'a str,
    title: &'a str,
    summary: Option<&'a str>,
    content: Option<&'a str>,
    publisher: &'a str,
    canonical_url: &'a str,
    published_at_rfc3339_nanos_utc: &'a str,
    instruments_sorted: &'a [String],
    topics_sorted: &'a [String],
    language: &'a str,
    record_source: &'a str,
    record_source_at: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct AcquiredGlobalNewsRecordPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.acquired_global_news_record.v1"
    source_fact_key: &'a str,
    provider_content_hash: &'a str,
    record: &'a SourceFactContentPreimage<'a>,
    record_provider: &'a str,
    record_source: &'a str,
    record_source_at: Option<&'a str>,
    record_observed_at: &'a str,
    record_batch_id: &'a str,
    record_batch_content_hash: &'a str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct SourceFactConflictPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.source_fact_conflict.v1"
    source_fact_key: &'a str,
    authoritative_provider_content_hash: &'a str,
    attempted_provider_content_hash: &'a str,
}

#[derive(Serialize)]
struct SourceFactAttemptPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.source_attempt.v1"
    ingress_run_id: &'a str,
    source_fact_key: &'a str,
    source_batch_attempt_id: &'a str,
    provider_ordinal: u32,
    source_batch_id: &'a str,
    record_batch_id: &'a str,
    observed_at: &'a str,
    batch_evidence_hash: &'a str,
}

#[derive(Serialize)]
struct FeedAttemptKeyPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.feed_attempt_key.v1"
    ingress_run_id: &'a str,
    feed_identity: &'a str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct FeedBatchEvidencePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.feed_batch_evidence.v1"
    feed_identity: &'a str,
    provider: &'a str,
    source: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
    batch_quality: &'a str, // exactly "complete"
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct FeedAvailableEvidencePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.feed_available_evidence.v1"
    feed_identity: &'a str,
    provider: Option<&'a str>,
    source: Option<&'a str>,
    source_at: Option<&'a str>,
    observed_at: Option<&'a str>,
    batch_id: Option<&'a str>,
    batch_content_hash: Option<&'a str>,
}

#[derive(Serialize)]
struct FeedSourceRecordHashPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.feed_source_record.v1"
    provider_ordinal: u32,
    source_fact_key: &'a str,
    provider_content_hash: &'a str,
}

#[derive(Serialize)]
struct FeedSourceContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.feed_source_content.v1"
    feed_identity: &'a str,
    evidence_hash: &'a str,
    record_hashes_in_provider_order: &'a [String],
}

#[derive(Serialize)]
struct FeedAttemptContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.feed_attempt_content.v2"
    feed_identity: &'a str,
    request_hash: &'a str,
    request_evidence_hash: &'a str,
    status_kind: &'a str, // "available" | "verified_empty" | "unavailable"
    record_count: Option<u32>,
    evidence_hash: Option<&'a str>,
    source_content_hash: Option<&'a str>,
    available_evidence_hash: Option<&'a str>,
    failed_stage: Option<&'a str>,
    reason_code: Option<&'a str>,
    retryable: Option<bool>,
    detail_hash: Option<&'a str>,
    error_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
struct RegisteredFeedConfigurationPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.registered_feed_config.v1"
    gateway_provider: &'a str,
    provider_id: &'a str,
    source_contract: &'a str,
    capability_name: &'a str,
    max_limit: u32,
    upstream_revision: &'a str,
}

#[derive(Serialize)]
struct RegisteredFeedIdentityPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.registered_feed_identity.v1"
    feed_name: &'a str,
    gateway_provider: &'a str,
    configuration_hash: &'a str,
}

#[derive(Serialize)]
struct RegisteredFeedEntryPreimage<'a> {
    ordinal: u32,
    feed_identity: &'a str,
    gateway_provider: &'a str,
    capability_name: &'a str,
    configuration_hash: &'a str,
}

#[derive(Serialize)]
struct RegisteredFeedSnapshotPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.registered_feed_snapshot.v1"
    feeds_sorted: &'a [RegisteredFeedEntryPreimage<'a>],
}

#[derive(Serialize)]
struct SourceBatchContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.source_batch_content.v1"
    registered_feed_snapshot_hash: &'a str,
    feed_attempt_hashes_in_registered_feed_order: &'a [String],
    source_record_hashes_in_feed_then_provider_order: &'a [String],
    event_projection_ids_in_feed_then_provider_order: &'a [String],
    aggregator_observed_at_rfc3339_nanos_utc: &'a str,
}

#[derive(Serialize)]
struct IngressGateInputPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.ingress_gate_input.v1"
    source_fact_key: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    provider_published_at_rfc3339_nanos_utc: &'a str,
    record_observed_at: &'a str,
    batch_observed_at: &'a str,
    batch_content_hash: &'a str,
    evaluated_at_rfc3339_nanos_utc: &'a str,
    freshness_max_age_secs: u64,
    future_tolerance_secs: u64,
    gate_version: &'a str,
}

#[derive(Serialize)]
struct IngressGateReceiptPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.ingress_gate_receipt.v1"
    ingress_run_id: &'a str,
    source_fact_key: &'a str,
    ingress_gate_input_hash: &'a str,
    decision: &'a str, // "admitted" | "rejected"
    reason_code: Option<&'a str>,
    retryable: Option<bool>,
    evaluated_at_rfc3339_nanos_utc: &'a str,
}
```

Each identity is `hex(sha256(serde_json::to_vec(preimage)))`. These are dedicated structs with fixed
declared field order and no maps; JSON is compact UTF-8. Option `None` serializes as JSON `null` and
is distinct from `Some("")`. Provider strings are retained byte-for-byte after Gateway admission;
`provider_content_hash = sha256_json(SourceFactContentPreimage)`. Every later
`source_fact_content_hash` field in stage inputs, attempts and samples means exactly this provider
logical-content hash, never the full `SelectionSourceFactRowContentPreimage` row hash. The
generation-receipt trigger joins the referenced source fact and requires equality.
For every source-fact attempt:

- `acquired_record_json = canonical_json(AcquiredGlobalNewsRecordPreimage)` and
  `acquired_record_hash = sha256_json(AcquiredGlobalNewsRecordPreimage)`;
- `batch_evidence_json = canonical_json(FeedBatchEvidencePreimage)` and
  `batch_evidence_hash = sha256_json(FeedBatchEvidencePreimage)`;
- `attempt_result=accepted | replay` requires SQL `conflict_hash=NULL`;
- `attempt_result=conflict` requires
  `conflict_hash = sha256_json(SourceFactConflictPreimage)`, whose authoritative hash equals the
  immutable source-fact row and whose attempted hash equals the nested acquired record.

The nested `record` must serialize to the same bytes used for `provider_content_hash`; duplicated
record/evidence columns in the attempt row must equal the corresponding preimage fields. The stage
and ingress-receipt validators parse both JSON columns with `deny_unknown_fields`, reserialize them,
and require byte-for-byte JSON equality and all four hashes above. There is no legacy/free-form
attempt payload variant. `IngressGateInputPreimage.batch_content_hash` is exactly that attempt's
derived per-feed `record_batch_content_hash`/`FeedSourceContentPreimage` hash from §7.1, and
`batch_observed_at` is the matching `FeedBatchEvidencePreimage.observed_at`; neither field may use
the later aggregate `source_batch_content_hash`.
`ingress_gate_input_json = canonical_json(IngressGateInputPreimage)` and
`ingress_gate_input_hash = sha256_json(IngressGateInputPreimage)`;
`ingress_gate_receipt_json = canonical_json(IngressGateReceiptPreimage)` and
`ingress_gate_receipt_hash = sha256_json(IngressGateReceiptPreimage)`. The source-fact row stores
both JSON/hash pairs plus duplicated decision/reason/retryable fields. Stage/receipt/read-model
validation strictly parses both JSON values, recomputes provider publication, record/batch
observation, derived per-feed batch hash, evaluation time, freshness/future thresholds, gate
version and all decision fields, and requires exact equality. The latter is decision evidence, not
the later run commit receipt. No trim/case/Unicode normalization occurs here. `published_at` uses UTC
`SecondsFormat::Nanos, use_z=true`. Instruments/topics are copied, sorted by UTF-8 byte order and
rejected on duplicates before hashing; the raw record still preserves provider order. All identity
constructors reject embedded NUL/control characters in provider identity fields. Golden
preimage/hex vectors are checked into
`tests/fixtures/selection/br174_hash_vectors_v1.json`; tests deserialize the expected JSON as bytes
and compare both the exact compact UTF-8 preimage and SHA-256 hex, so a serializer, absence encoding
or field-order change fails.

Provider-owned content fields include identity, title, optional summary/body/instruments/topics,
publisher, official URL and provider publication time, but exclude the later acquisition batch ID
and observation time. Every attempt is appended with the complete
`GlobalNewsRecord + BatchEvidence` before evaluation. The first source-schema-valid logical content is
authoritative even when its immutable ingress-gate result is rejected;
a later observation cannot reclassify it. The same content in a later batch appends a new source-fact attempt and reuses the
logical fact; changed provider-owned content for the same key is an explicit conflict and does not
mutate the fact. The stored notification `event_id` is a separate domain hash defined above, although
both keys use the same stable provider-source/item components.

The first source-fact attempt runs BR-137/BR-155 publication/future/staleness/identity checks exactly
once against the provider publication time and that acquisition's provider observation time, not a
later scheduler wall clock. It persists the gate version, inputs, result and receipt. Only an
`IngressAdmitted` fact may create relation attempts. Initial stale/future/invalid facts remain
immutable ingress rejections. Dependency handling always reuses the admitted ingress receipt and
must not reclassify the fact as stale merely because time passed. The prospective generation window
in §5.2 may close before a later retry; that creates a distinct non-retryable
`prospective_window_closed` generation attempt/manifest and never rewrites the ingress decision.

If a `MarketEvent` projection exists, its identity, provider/source, URL, publication and observed
time must be re-computable from and agree with that source fact. A projection without an exact source
fact fails closed.

The ingress run is committed before any relation, board or market request begins. Relation
generation can read only `IngressAdmitted` facts whose `first_ingress_run_id` inner-joins an
`ingress_run` commit receipt. Notification simhash is not a selection fact gate. A source fact whose notification projection is
`DedupSuppressed` still enters the durable v2 inbox and is independently evaluated; only exact
logical fact/content replay is de-duplicated there. Therefore a crash after the in-process
notification simhash set advances cannot lose the selection fact: its ingress receipt already
exists and pending generation reads it after restart.
No fuzzy/cross-provider notification similarity may suppress a security-level fact under BR-155.

The source-batch identity hashes, in stable provider/batch/item order:

1. all admitted source records and their batch evidence;
2. their re-computable event projection IDs;
3. typed feed attempts and their evidence; and
4. aggregator observed time.

Notification disposition is intentionally excluded: source ingress commits before notification
projection, and the later simhash result cannot be UPDATE-filled into an append-only source
attempt. Existing notification audit owns `Delivered`/`DedupSuppressed`; selection stores only the
deterministic projection ID and never depends on that disposition.

`FeedAttemptStatus::Succeeded { event_count }` is replaced by source-bound variants:

```text
Available { record_count, evidence, content_hash }
VerifiedEmpty { evidence, content_hash }
Unavailable { reason_code, retryable, available_evidence, detail_hash }
```

Every registered feed produces exactly one typed terminal attempt. `Available` and `VerifiedEmpty`
retain the exact immutable `BatchEvidence`; a `VerifiedEmpty` projection may not become a default
empty `NewsFeedOutput`. Facts from one available provider may still be evaluated when another
provider is unavailable, but a global `VerifiedEmpty`, `sources_complete=true`, or verified
no-relation conclusion is permitted only when every registered feed has a complete, evidenced
`Available` or `VerifiedEmpty` terminal state. Missing attempt/evidence is `Unavailable`, never an
empty source. This changes the internal feed-attempt enum and serialization; no compatibility
projection may preserve the evidence-erasing `Succeeded` shape.

For `Unavailable`, `available_evidence` is optional structured evidence acquired before failure,
not a complete `BatchEvidence`. If present it is hashed as
`available_evidence_hash = sha256_json(FeedAvailableEvidencePreimage)`; at least one optional
evidence field must be non-NULL. It may never set complete `evidence_hash`,
`source_content_hash`, `record_count`, or child source-fact attempts.

## 5. Explicit provider board binding

The existing `ChainRuleConfig.board_keyword` has historical fuzzy-search semantics and is not
authoritative provider identity. It remains legacy/research metadata and must not create formal
relations.

Formal expansion adds one optional, all-or-none object:

```toml
[rules.provider_board]
provider = "tdx"
code = "tdx:concept:CPO"
name = "CPO"
kind = "concept"
binding_audit_hash = "<64-lowercase-hex-sha256>"
```

For each matched rule:

1. missing the object means `board_binding_not_configured`;
2. a partial binding is a configuration error and disables the configuration snapshot;
3. provider is exactly `tdx`, kind is exactly `industry | concept`;
4. code must equal `tdx:{kind}:{name}` after trim-only normalisation;
5. the binding must have passed the release-time live binding audit against the current pinned
   Magic TDX provider;
6. runtime calls `board_constituents` using the explicit canonical code and fixed limit 10,000;
7. an empty batch retains upstream `InvalidData` and non-retryable classification;
8. a returned length of 10,000 is `board_constituents_may_be_truncated` and fails closed;
9. a shorter strict batch must have one board code/name/kind and valid canonical A-share identities
   on every record.

The fixed maximum request and equality rejection are necessary because the current upstream
`DataBatch` has no `total`/`truncated` field. This is an explicitly documented pinned-revision
contract, not a claim that the board directory is globally complete. A future upstream
`total/truncated` contract may replace this rule only through a new Gate A review.

There is no substring search, synonym table, directory “best match”, LLM match, static security
list or fallback to the old opportunity chain. There is no Top-N truncation.

Logical relation identity and acquisition-attempt identity are separate:

```text
relation_key = sha256_json(RelationKeyPreimage)
attempt_id   = sha256_json(RelationAttemptPreimage)
```

`typed_binding_state_hash` encodes `NotConfigured`, `InvalidConfig` or the full verified binding
object; it never uses NULL/empty provider code as identity.
`relation_source_identity_hash` is variant-specific:

- `DirectMention`: source-fact identity plus exact-code/exact-name kind, canonical normalized value,
  and the byte span in the immutable source title/body field;
- `ProviderBoardConstituent`: the complete verified binding object hash for the board acquisition
  relation. Each returned canonical member remains explicit in the resolved attempt payload and is
  merged into its own `(event, chain, code)` sample evidence set.

`stage_run_id` is a persisted generation-run identity created before acquisition. Replaying the same
crashed run and content is idempotent; a later run records a new attempt even when the transport
error fingerprint is identical. Local `attempted_at` is retained as audit chronology only and never
masquerades as provider `observed_at` or `source_at`. A retryable failure does not occupy the
terminal relation identity; a later successful provider batch appends a new attempt. The first
successfully committed resolved attempt is authoritative. A different successful content hash for
an already committed relation conflicts and does not overwrite it.

A missing/invalid binding or constituent/canonicalisation failure has no canonical security
identity and therefore remains relation-attempt audit only; it must not create a fake
`selection_samples.stock_code`.

A read-only live binding audit tool validates human-proposed exact triples against the provider
directory and emits only a derived artifact to an explicit output path; it never selects a triple
or rewrites checked-in configuration automatically. It requests each Industry/Concept directory with
limit 10,000 as two independent provider calls and rejects either `len == 10_000` as potentially
truncated. It must preserve the two provider-owned batches separately; it may not manufacture a
local aggregate batch ID, content hash, source time or observation time. Its immutable JSON artifact
contains:

- schema version and artifact content hash;
- exact upstream revision `b2b68df78156df1d67824e5c44c0cb01b752f55a`;
- exactly two real directory-batch evidence objects, one for `concept` and one for `industry`,
  sorted by category UTF-8 bytes, each retaining its own provider/source/source_at/observed/batch
  ID, every provider-ordered directory record, recomputable batch content hash and record count;
- requested limit;
- exact board code/name/kind;
- each binding's own release-time directory member count;
- audit command version and recorded time.

The checked-in path is fixed:

```text
config/selection/provider_board_bindings.v1.json
```

The sole operator input is a second checked-in, strict canonical file at:

```text
config/selection/provider_board_binding_proposal.v1.json
```

It contains no provider evidence or derived hash/count:

```rust
#[derive(Serialize)]
struct ProposalBindingPreimage<'a> {
    chain_id: &'a str,
    provider: &'a str,
    kind: &'a str,
    code: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct BoardBindingProposalInputPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_binding_proposal.v1"
    schema_version: &'a str, // exactly "selection-provider-board-binding-proposal-v1"
    validity_policy_version: &'a str,
    valid_from_rfc3339_nanos_utc: &'a str,
    expires_at_rfc3339_nanos_utc: &'a str,
    reviewed_by: &'a str,
    reviewed_at_rfc3339_nanos_utc: &'a str,
    bindings_sorted: &'a [ProposalBindingPreimage<'a>],
}
```

`proposal_input_content_hash = sha256_json(BoardBindingProposalInputPreimage)`. The checked-in bytes
must equal its compact fixed-order canonical JSON plus one LF. Bindings are unique and sorted by
`(chain_id, provider, kind, code, name)` UTF-8 bytes; each provider is exactly `tdx`, each kind is
exactly `concept | industry`, and each code equals `tdx:{kind}:{name}`. `bindings=[]` is allowed.
`reviewed_by` is non-empty and `reviewed_at` is not later than live audit `recorded_at`.

`VALIDITY_POLICY_VERSION` is exactly `selection-board-binding-validity-v1`. All proposal timestamps
and artifact-local `recorded_at` timestamps are parsed as exact RFC3339 nanosecond UTC strings;
provider `observed_at` uses the separately frozen parsing rule below. The fixed policy is:

```text
reviewed_at <= recorded_at <= valid_from <= recorded_at + 24 hours
valid_from < expires_at <= valid_from + 30 * 24 hours
```

No environment variable or CLI option may change this policy/version. A different validity policy
requires a new Gate A version. The proposal file is the human-reviewed selection/release-window
input committed through PR; it is not provider evidence. The audit tool always reads this fixed
path, performs the two live directory calls, exact-matches every proposed triple, and derives all
record/batch/binding/artifact hashes and counts. The output artifact must contain exactly one derived
binding for every proposal binding and no extra binding.

The audit tool reuses `BoardDataGateway::production_tdx()` and its fixed production
resolver/connect/retry/timeout policy. The capture CLI accepts exactly one option,
`--output <explicit-path>`; host, IP, port, source, resolver, timeout, retry, proposal path and
provider overrides are forbidden. The constants are:

```text
BOARD_CONNECTION_POLICY_VERSION = "selection-board-tdx-production-v1"
BOARD_AUDIT_CAPTURE_MAX_AGE_SECS = 300
BOARD_AUDIT_ROOT_POLICY_VERSION = "selection-board-audit-root-v1"
BOARD_AUDIT_ROOT_RELATIVE_PATH = "data/audit/production"
BOARD_DIRECTORY_PROVIDER = "tdx"
BOARD_DIRECTORY_SOURCE = "tdx-block-files"
SELECTION_RELEASE_DATABASE_RELATIVE_PATH = "data/stock_analysis.db"
```

The connection-policy hash is frozen as:

```rust
#[derive(Serialize)]
struct BoardConnectionPolicyPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_connection_policy.v1"
    version: &'a str,
    provider: &'a str, // exactly "tdx"
    gateway_constructor: &'a str, // exactly "BoardDataGateway::production_tdx"
    resolver_policy: &'a str, // exactly "magic_tdx_production_resolver_v1"
    endpoint_override: &'a str, // exactly "forbidden"
}

#[derive(Serialize)]
struct BoardAuditRootBindingPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_audit_root.v1"
    version: &'a str,
    repository_relative_path: &'a str,
}

#[derive(Serialize)]
struct ProductionEvidencePathBindingPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.production_evidence_path.v1"
    kind: &'a str,
    source: &'a str, // exactly "fixed_cargo_manifest_dir"
    canonical_absolute_path: &'a str,
}
```

`connection_policy_hash = sha256_json(BoardConnectionPolicyPreimage)`. The implementation bytes
are additionally covered by `executable_revision`; the version/hash is not a provider endpoint
attestation. `provider_endpoint_evidence` is JSON `null` for the current TDX contract because the
upstream batch does not provide provider-authenticated endpoint evidence. A local socket target,
configured server or tool self-report may not populate it. It may become non-NULL only after a new
upstream strong evidence contract and Gate A revision.
`audit_root_binding_hash = sha256_json(BoardAuditRootBindingPreimage)`; the capture and Gate D
verification resolve that fixed repository-relative root and accept no CLI/environment override.
All production evidence paths are anchored to the compile-time repository root from
`env!("CARGO_MANIFEST_DIR")`, never the process CWD. `ProductionEvidenceRoots::open_pinned()` first
performs a lexical join from that root and rejects `..`. It then opens the repository root and each
path component relative to the already pinned parent directory handle using no-follow semantics
(`openat`/platform equivalent with `O_NOFOLLOW`, plus `O_DIRECTORY` for directories); it never
validates a path and later reopens that path by name. The returned non-`Clone`
`PinnedProductionEvidenceSet` owns every directory/file handle through the complete capture or
release verification. Regular files/audit records are read only from those handles.
`open_pinned()` pins the production database parent and main-file object but deliberately does not
start a SQLite read transaction. It constructs a descriptor-anchored VFS capability that can later
open the main database and every then-present `-wal`/`-shm` sidecar relative to the still-pinned
parent with no-follow semantics.

Read-only Gate D modes call `snapshot_sqlite_after_writes()` immediately after pinning. The live
canary calls it only after its ingress/generation commits and only while the order-audit freeze
lock is held. That method records the main/sidecar identities and presence set, starts one SQLite
read transaction, creates and fsyncs a private immutable online-backup snapshot, hashes the exact
backup bytes, and queries only that pinned backup. It also hashes a canonical logical-high-water
manifest containing the ordered config-activation, ingress, generation, manifest and receipt rows
used by the canary. Sidecar creation before this post-write snapshot is allowed; creation,
replacement, disappearance or identity mismatch from snapshot start through backup completion
fails. Calling `Connection::open(path)` after validation is forbidden and an unsupported
pinned-VFS/backup platform fails closed.

Before and after every read/snapshot, the implementation `fstat`s all pinned handles and requires
their `(device,inode,file-type)` identities to equal the initially bound values. At completion it
also re-traverses from the still-pinned root handle with no-follow opens and requires each current
name to resolve to the same identity; rename, unlink/recreate, symlink insertion or component
replacement therefore fails even though reads themselves remain pinned to the original object.
For fixed regular inputs (proposal, artifact, activation and executable/config inputs), initial
`size/change-time/content-sha256` and canonical bytes are recorded; after all dependent validation,
the same pinned handles are rewound and reread and must reproduce the exact size, metadata and
content hash. Selection/board audit files are read only while their existing OS audit lock is held:
the verifier freezes a byte high-water mark and prefix SHA-256, validates exactly that prefix, then
requires the same prefix hash/high-water before releasing the lock. Appends beyond high-water,
truncation or in-place overwrite during the locked verification fail. The pinned SQLite backup
binds the logical main/WAL/SHM snapshot rather than assuming main-file inode identity is sufficient.
Canonical absolute paths are derived from the pinned handles and must equal the fixed lexical
targets. A missing target, identity/canonicalization mismatch, environment override or symlink
fails closed. Capture uses this handle set for the fixed board audit root; every Gate D release mode
additionally pins the fixed production database, proposal, artifact, selection audit root and board
audit root. Caller-selected paths are accepted only by mutually exclusive diagnostic modes that
always emit `release_eligible=false` and may not emit any `*_verified=true` field. Gate B tests
replace a directory or file concurrently between initial traversal, read and final identity check;
the release path must either continue reading the pinned original and then reject the name change,
or fail earlier, never accept the replacement. Further negatives overwrite a fixed file in place,
truncate/append an audit file, and independently swap the main DB, WAL and SHM objects.

The strict file has `schema_version=selection-provider-board-bindings-v1`, `upstream_revision`,
`valid_from`, explicit human-reviewed `expires_at`, and bindings sorted uniquely by
`(chain_id, provider, kind, code, name)`. Unknown/duplicate JSON fields, duplicate binding keys,
unsorted rows or invalid/future evidence chronology fail validation. `artifact_content_hash` uses:

This corrected shape replaces the earlier Gate-A-only `v1` draft before any production activation
or Gate D release. The earlier singular-directory-batch preimage was never published, activated or
accepted as release evidence. Therefore the checked-in path, schema-version string and hash-domain
names remain `v1`, while every artifact encoded with the superseded singular fields is invalid and
must be regenerated from live provider evidence. This is not an in-place reinterpretation of a
released hash domain.

```rust
#[derive(Serialize)]
struct ArtifactBindingPreimage<'a> {
    chain_id: &'a str,
    provider: &'a str,
    kind: &'a str,
    code: &'a str,
    name: &'a str,
    binding_audit_hash: &'a str,
    directory_record_hash: &'a str,
    release_directory_member_count: u32,
}

#[derive(Serialize)]
struct DirectoryRecordSourceEvidencePreimage<'a> {
    provider: &'a str,
    source: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
}

#[derive(Serialize)]
struct DirectoryBoardRecordPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_directory_record.v1"
    provider_ordinal: u32,
    code: &'a str,
    name: &'a str,
    kind: &'a str,
    member_count: u32,
    evidence: &'a DirectoryRecordSourceEvidencePreimage<'a>,
}

#[derive(Serialize)]
struct DirectoryBatchContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_directory_batch.v1"
    category: &'a str,
    provider: &'a str,
    source: &'a str,
    source_at: Option<&'a str>,
    observed_at: &'a str,
    batch_id: &'a str,
    records_in_provider_order: &'a [DirectoryBoardRecordPreimage<'a>],
}

#[derive(Serialize)]
struct DirectoryBatchEvidencePreimage<'a> {
    content: &'a DirectoryBatchContentPreimage<'a>,
    batch_content_hash: &'a str,
    record_count: u32,
}

#[derive(Serialize)]
struct BoardAuditSubjectPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_audit_subject.v1"
    proposal_input_content_hash: &'a str,
    audit_command_version: &'a str,
    connection_policy_hash: &'a str,
}

#[derive(Serialize)]
struct BoardAuditPreparedContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_audit_prepared.v1"
    audit_subject_id: &'a str,
    audit_run_id: &'a str,
    proposal_input_content_hash: &'a str,
    audit_command_version: &'a str,
    connection_policy_version: &'a str,
    connection_policy_hash: &'a str,
    provider_endpoint_evidence: Option<&'a str>,
    audit_root_policy_version: &'a str,
    audit_root_binding_hash: &'a str,
    requested_categories_sorted: &'a [String],
    requested_limit: u32,
    prepared_at_rfc3339_nanos_utc: &'a str,
}

#[derive(Serialize)]
struct AttestedDirectoryBatchPreimage<'a> {
    category: &'a str,
    batch_content_hash: &'a str,
    record_count: u32,
    observed_at: &'a str,
}

#[derive(Serialize)]
struct BoardAuditAttestationContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_audit_attestation.v1"
    audit_subject_id: &'a str,
    audit_run_id: &'a str,
    proposal_input_content_hash: &'a str,
    upstream_revision: &'a str,
    audit_command_version: &'a str,
    connection_policy_version: &'a str,
    connection_policy_hash: &'a str,
    provider_endpoint_evidence: Option<&'a str>,
    audit_root_policy_version: &'a str,
    audit_root_binding_hash: &'a str,
    requested_limit: u32,
    directory_batches_by_category: &'a [AttestedDirectoryBatchPreimage<'a>],
    recorded_at_rfc3339_nanos_utc: &'a str,
}

#[derive(Serialize)]
struct BoardAuditCommittedContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_audit_committed.v1"
    audit_subject_id: &'a str,
    audit_run_id: &'a str,
    prepared_record_hash: &'a str,
    attestation_content_hash: &'a str,
    committed_at_rfc3339_nanos_utc: &'a str,
}

#[derive(Serialize)]
struct BoardAuditAttestationReceiptPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_audit_receipt.v1"
    audit_subject_id: &'a str,
    audit_run_id: &'a str,
    prepared_record_hash: &'a str,
    committed_record_hash: &'a str,
    attestation_content_hash: &'a str,
    audit_root_policy_version: &'a str,
    audit_root_binding_hash: &'a str,
}

#[derive(Serialize)]
struct ArtifactHashPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_artifact.v1"
    schema_version: &'a str,
    upstream_revision: &'a str,
    proposal_input: &'a BoardBindingProposalInputPreimage<'a>,
    proposal_input_content_hash: &'a str,
    connection_policy_version: &'a str,
    connection_policy_hash: &'a str,
    provider_endpoint_evidence: Option<&'a str>,
    valid_from_rfc3339_nanos_utc: &'a str,
    expires_at_rfc3339_nanos_utc: &'a str,
    directory_batches_by_category: &'a [DirectoryBatchEvidencePreimage<'a>],
    requested_limit: u32,
    audit_command_version: &'a str,
    recorded_at_rfc3339_nanos_utc: &'a str,
    audit_attestation_receipt: &'a BoardAuditAttestationReceiptPreimage<'a>,
    audit_attestation_receipt_hash: &'a str,
    bindings_sorted: &'a [ArtifactBindingPreimage<'a>],
}
```

`artifact_content_hash = hex(sha256(b"SELECTION_PROVIDER_BOARD_BINDINGS_V1\0" ||
canonical_json(ArtifactHashPreimage)))`. The struct excludes `artifact_content_hash`, uses no maps,
and contains bindings sorted uniquely by `(chain_id, provider, kind, code, name)`.
The nested `proposal_input` must recompute to `proposal_input_content_hash`; outer
`valid_from_rfc3339_nanos_utc`/`expires_at_rfc3339_nanos_utc` must equal its release window, and
outer bindings must map one-to-one to its bindings before adding only the derived audit
hash/record-hash/member-count fields.
`connection_policy_version` must equal `BOARD_CONNECTION_POLICY_VERSION`, its preimage must
recompute to `connection_policy_hash`, and current `provider_endpoint_evidence` must be JSON
`null`. The artifact fields are sufficient to reconstruct
`BoardAuditAttestationContentPreimage`; it must recompute to
`audit_attestation_receipt.attestation_content_hash`. The nested receipt must recompute under
`audit_attestation_receipt_hash =
sha256_json(BoardAuditAttestationReceiptPreimage)`. Its subject/run IDs, both audit record hashes
and attestation hash are non-empty, canonical and immutable; their existence in the original audit
chain is a separate Gate D verification below. The receipt's root policy/version binding equals
the fixed `BoardAuditRootBindingPreimage` constants and recomputes to
`audit_root_binding_hash`; a detached, flattened or partially copied receipt is invalid.
The artifact outer `recorded_at_rfc3339_nanos_utc` is byte-for-byte equal to the reconstructed
attestation `recorded_at` and Committed content `committed_at`; the outer Committed audit record
time verified at Gate D must normalize through `canonical_nanos_utc` to that same string.
`directory_batches_by_category` has exactly two entries with categories `concept` and `industry`,
sorted uniquely by `content.category` UTF-8 bytes. Each entry embeds the complete, provider-ordered
record preimages of one actual upstream directory request; no detached hash/count reference is
permitted. `requested_limit` is exactly `10_000`; `9_999`, `10_001` or any other value fails before
artifact acceptance. The record ordinals are contiguous `0..record_count` in provider order and

```text
directory_record_hash = sha256_json(DirectoryBoardRecordPreimage)
batch_content_hash = sha256_json(DirectoryBatchContentPreimage)
0 < record_count == content.records_in_provider_order.len() < 10_000
```

The common §6.1 fixed-order compact JSON and lowercase SHA-256 rules apply to both hashes. Every
directory record is validated even when `bindings=[]`: `name` is non-empty and trim-stable
(`name == name.trim()`), `kind` equals its enclosing category, `member_count > 0`, and
`code == "tdx:{category}:{name}"` byte-for-byte after that trim-stability check. Every record
evidence `source/source_at/observed_at/batch_id` equals the enclosing batch fields byte-for-byte;
record code/name/kind triples are unique within a category. A malformed unused record invalidates
the whole artifact. The two category batches must have distinct non-empty `batch_id` values even
when all other provider evidence is equal. Each batch independently satisfies provider, source,
chronology and content validation.
Regardless of whether `bindings` is empty, both enclosing directory batches must have
`provider == BOARD_DIRECTORY_PROVIDER`, `source == BOARD_DIRECTORY_SOURCE` and `source_at == None`
byte-for-byte. Every embedded record evidence must repeat that same fixed source identity and
absence, including the record-level `provider` now retained inside and hashed by
`DirectoryRecordSourceEvidencePreimage`; it may not be discarded during normalization.
Attestation reconstruction and Gate D repeat these checks independently; a self-consistent
hash using another provider/source, one correct category plus one incorrect category, a non-NULL
`source_at`, a wrong record-level provider, or an unselected bad record invalidates the whole
artifact. Golden/negative vectors change only one record's provider while leaving batch evidence
correct and must fail.

`BOARD_AUDIT_CAPTURE_MAX_AGE_SECS` applies independently to both batches:

```text
observed_at <= recorded_at
recorded_at - observed_at <= 300 seconds
```

`recorded_at` is exact RFC3339 nanosecond UTC. An observed evidence time is preserved byte-for-byte
but must parse completely as either exact RFC3339 nanosecond UTC or exact
`unix-ms:<unsigned-decimal>`; the decimal has no sign, whitespace or non-canonical leading zero,
must fit `u64`, convert without overflow to a valid UTC instant, and consume the entire string.
Prefix/trailing parsing, parse failure and future evidence all reject. `source_at` remains absent
for current TDX directory evidence and is never replaced by either observed or recorded time.

A `(provider, batch_id)` seen with different content is an immutable evidence collision. The audit
run fails and may start a new provider fetch, but it must not rewrite the captured record, hash,
count or chronology to salvage the proposal. A retry must produce a new complete capture and remains
subject to the distinct-batch-ID rule. The strict loader round-trips the bytes and rejects any
non-canonical whitespace/key/order representation. This defines one machine-verifiable preimage
without relying on incidental map ordering, an opaque detached digest or a fabricated aggregate
batch.

The audit capture is itself an attested two-phase run:

1. `audit_subject_id = sha256_json(BoardAuditSubjectPreimage)` and `audit_run_id` is a canonical
   UUIDv7 allocated before any append;
2. under the existing BR-174 production audit-root OS lock, the tool validates the whole chain and
   appends a `Prepared` hash-chain record whose content hash is
   `sha256_json(BoardAuditPreparedContentPreimage)`, then `sync_data`;
3. only after Prepared durability does it perform the two provider calls and construct the complete
   directory preimages;
4. `audit_attestation_content_hash =
   sha256_json(BoardAuditAttestationContentPreimage)`, whose two batch entries are category-sorted
   and exactly match the artifact batches;
5. under the same root/lock protocol it revalidates the chain, appends a `Committed` hash-chain
   record whose content hash is `sha256_json(BoardAuditCommittedContentPreimage)`, and `sync_data`;
6. it constructs `BoardAuditAttestationReceiptPreimage` from the exact subject/run ID, Prepared
   record hash, Committed record hash, attestation content hash and fixed audit-root binding,
   computes `audit_attestation_receipt_hash`, and only then may it emit the proposed artifact with
   that complete nested receipt.

The Prepared and Committed *record hashes* are the existing tamper-evident audit-chain record hashes,
including phase/content hash/previous-record hash under §8; they are not merely the content hashes
above. A failed/cancelled run may leave Prepared without Committed and emits no artifact. Reusing
the same run ID with different content, an attestation/batch mismatch, or an audit append/fsync
failure fails closed.
For both phase records, existing `SelectionAuditRecord.subject_id` is exactly `audit_run_id`;
`audit_subject_id` remains the deterministic proposal/command/connection-policy identity inside
both content preimages. The validator recomputes that deterministic subject and rejects a record
whose run subject or nested logical subject differs.
The audit-chain clock is part of the attestation, not advisory metadata. The outer Prepared
`SelectionAuditRecord.recorded_at` is first parsed and normalized by
`canonical_nanos_utc(outer_recorded_at)` and that canonical string must equal
`prepared_at_rfc3339_nanos_utc`; the outer Committed time is normalized the same way and must equal
`committed_at_rfc3339_nanos_utc`. This comparison deliberately preserves the existing outer
`DateTime<FixedOffset>`/Chrono `AutoSi` serialization and historical record hashes; only the
normalized instant is compared to the canonical inner nanosecond-UTC string. After the two provider
calls, one canonical clock value is
captured and copied byte-for-byte to both
`BoardAuditAttestationContentPreimage.recorded_at_rfc3339_nanos_utc` and
`BoardAuditCommittedContentPreimage.committed_at_rfc3339_nanos_utc`; therefore the attestation
recorded time and inner committed time use the same bytes, while the outer Committed record must
normalize to that same instant.
Validation enforces, for both directory batches:

```text
previous audit-tail recorded_at <= prepared_at <= directory observed_at
directory observed_at <= attestation recorded_at == committed_at
```

The Committed record must follow the matching Prepared record in the validated chain. If unrelated
records were appended while the network calls ran, the re-acquired-lock tail time must also be
`<= committed_at`; the tool never backdates either phase to make an old observation look fresh.
Gate D reconstructs and verifies the entire chronology in addition to content and record hashes.
Negative vectors cover a one-nanosecond inner/outer mismatch, an observation before Prepared, an
old observation paired with a new validity window, a backfilled attestation time, Committed before
attestation/current tail, and Prepared before the previous tail.

`binding_audit_hash` is not the artifact hash. Each entry contains its own exact per-binding hash:

```rust
#[derive(Serialize)]
struct BindingAuditPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_binding.v1"
    upstream_revision: &'a str,
    chain_id: &'a str,
    provider: &'a str,
    kind: &'a str,
    code: &'a str,
    name: &'a str,
    directory_category: &'a str,
    directory_source: &'a str,
    directory_source_at: Option<&'a str>,
    directory_observed_at: &'a str,
    directory_batch_id: &'a str,
    directory_batch_content_hash: &'a str,
    directory_record_hash: &'a str,
    release_directory_member_count: u32,
    proposal_input_content_hash: &'a str,
    proposal_reviewed_by: &'a str,
    proposal_reviewed_at_rfc3339_nanos_utc: &'a str,
    validity_policy_version: &'a str,
    audit_command_version: &'a str,
    connection_policy_version: &'a str,
    connection_policy_hash: &'a str,
    provider_endpoint_evidence: Option<&'a str>,
    audit_attestation_receipt_hash: &'a str,
    recorded_at_rfc3339_nanos_utc: &'a str,
    valid_from_rfc3339_nanos_utc: &'a str,
    expires_at_rfc3339_nanos_utc: &'a str,
}
```

It is `hex(sha256(serde_json::to_vec(preimage)))` under the common compact/fixed-order/null rules
in §6.1. The artifact hash covers the sorted entries including these per-binding hashes. Runtime
stores and verifies both `artifact_content_hash` and the selected `binding_audit_hash`; substituting
one for the other is a configuration error. Each binding's `directory_category` must equal its
`kind`, its `provider` must equal the corresponding directory batch provider, and its directory
evidence fields must equal the corresponding entry in
`directory_batches_by_category`. The loader must find exactly one record in that category whose
`(code, name, kind)` equals the binding triple, recompute that record's
`directory_record_hash`, and require both the binding hash and
`release_directory_member_count` to equal the exact record hash and `member_count`.
The matched record's `member_count` must be greater than zero.
Not-found, duplicate, cross-category, record-hash mismatch, member-count mismatch, a missing
category batch or a singular locally aggregated batch fails validation.

`AUDIT_COMMAND_VERSION` is exactly `selection-board-binding-audit-v1`. The artifact outer
`audit_command_version` must equal this constant; it is not a CLI argument. The CLI may choose only
the explicit output path. It may not select bindings, change the proposal-input path/policy,
configure timeout/retry/resolver/host/IP/port/source, or inject category records, provider/source
evidence, batch IDs, hashes, counts, recorded time, validity, binding audit hashes or artifact
content hashes. Binding selection/release window comes only from the fixed checked-in proposal;
connection behaviour comes only from the fixed production Gateway; evidence values come from the
two live provider calls, the audited clock and deterministic hashing.

Every `BindingAuditPreimage` copies outer `audit_command_version`,
`connection_policy_version`, `connection_policy_hash`,
`provider_endpoint_evidence` and `audit_attestation_receipt_hash`,
`recorded_at_rfc3339_nanos_utc`, `valid_from_rfc3339_nanos_utc` and
`expires_at_rfc3339_nanos_utc` byte-for-byte. It also copies the artifact's
`proposal_input_content_hash` and the nested proposal's `reviewed_by`, `reviewed_at` and
`validity_policy_version` byte-for-byte. Its directory fields copy the corresponding category
batch, while `directory_record_hash` and `release_directory_member_count` copy the exact matched
record. No binding-specific proposal/clock/version/validity override is permitted.

`src/data_gateway/board.rs` is the only strict loader/validator for both the proposal input and the
resulting artifact and returns a private-construction typed verified artifact. Production BR-174
startup and every reload read both fixed checked-in files in the same loader operation. It requires
the proposal raw bytes to equal `canonical_json(parsed_proposal) + LF`, the nested artifact proposal
to equal that parsed proposal field-for-field, `canonical_json(nested_proposal) + LF` to equal the
same raw proposal bytes, and both proposal hashes to recompute identically. Proposal/artifact drift,
including a human edit after capture, rejects until a fresh audited artifact is generated.

Startup fails closed before generation if either file is absent, duplicated, expired at startup,
non-canonical, hash
mismatched, not exactly pinned to `b2b68df78156df1d67824e5c44c0cb01b752f55a`, or inconsistent
with `config/chain.toml`. No fallback artifact or last-known-good cache is used. The read-only audit
tool emits a proposed artifact to an explicit caller path; a human must review and commit it through
PR. Renewal or binding changes require fresh live evidence, a new hash and the same Gate C/D checks.
Rollback may restore an older checked-in artifact only when its revision, expiry and chain config
still validate; otherwise board generation remains disabled rather than weakening the contract.

Threat boundary: the current TDX directory contract has no provider cryptographic signature and
does not attest a network endpoint. This design must not manufacture or claim either property.
Protection against a hand-written artifact is layered: fixed non-injectable production Gateway
capture, complete self-recomputable records, Prepared/Committed hash-chain audit, checked-in
human-reviewed proposal and artifact, config-activation receipt, and a Gate D live refetch. Gate D
on the release/capture host verifies the referenced Prepared/Committed records and attestation
against the original BR-174 audit root. A production activation on another host is not required to
possess that external capture audit root; it trusts the reviewed checked-in repository boundary and
verifies the artifact/proposal/config hashes plus its local config-activation receipt. Runtime
success therefore means “self-consistent reviewed capture”, never “provider-signed truth”.

Runtime config must match the artifact triple and its per-entry `binding_audit_hash` exactly. Each
binding's release-time `release_directory_member_count` is audit context only; it is not reused as
a runtime count or historical as-of fact. Runtime stores its separately observed
`actual_constituent_count`.

Release configuration includes only human-reviewed, live-proven bindings. Rules without one
continue direct-mention selection and explicitly record that board expansion is not configured.
A canonical, unexpired artifact with both independently live-audited directory batches may have
`bindings=[]`; that artifact is valid for Gate B/Gate C direct-mention execution and proves only
that no board binding was activated. It is distinct from `direct_only_unverified`, which remains
invalid for activation. Gate D still requires at least one human-reviewed verified binding and a
successful real `--first-binding` constituent canary; an empty binding array can never satisfy
Gate D.

Because TDX block evidence has no provider publication time, downstream evidence must record:

```text
source_at = absent
as_of_basis = observed_at
```

It must not label local collection time as board publication time. Provider empty constituents are
invalid/unavailable evidence, never “verified no members”.

Both record and batch evidence must agree on provider, batch ID, `source_at` absence and observed
value. `unix-ms:<n>` and other upstream evidence times are retained as opaque strings; the Gateway
may validate their format but must not rewrite them to RFC3339 or infer publication time.

### 5.1 Immutable config activation

`config_hash` means exactly:

```rust
#[derive(Serialize)]
struct ChainRuleSnapshotEntryPreimage<'a> {
    chain_id: &'a str,
    category: &'a str,
    priority: u32,
    logic: &'a str,
    board_keyword: &'a str,
    keywords_in_config_order: &'a [String],
    generic: bool,
    enabled: bool,
    provider_board_binding_audit_hash: Option<&'a str>,
}

#[derive(Serialize)]
struct ChainRulesSnapshotPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.chain_rules_snapshot.v1"
    rules_sorted: &'a [ChainRuleSnapshotEntryPreimage<'a>],
}

#[derive(Serialize)]
struct ExecutableInputFilePreimage<'a> {
    relative_path: &'a str,
    byte_len: u64,
    content_sha256: &'a str,
}

#[derive(Serialize)]
struct ExecutableRevisionPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.executable_revision.v1"
    input_manifest_version: &'a str, // exactly "selection-executable-inputs-v1"
    files_sorted: &'a [ExecutableInputFilePreimage<'a>],
}

#[derive(Serialize)]
struct SelectionConfigSnapshotPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.selection_config_snapshot.v1"
    schema_version: &'a str,
    chain_config_bytes_hash: &'a str,
    chain_rules_snapshot: &'a ChainRulesSnapshotPreimage<'a>,
    chain_rules_sorted_content_hash: &'a str,
    board_artifact: &'a ArtifactHashPreimage<'a>,
    board_artifact_content_hash: &'a str,
    binding_audit_hashes_sorted: &'a [String],
    relation_schema_version: &'a str,
    feature_version: &'a str,
    admission_version: &'a str,
    upstream_revision: &'a str,
    executable_revision: &'a str,
}

#[derive(Serialize)]
struct ConfigActivationContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.config_activation.v1"
    config_hash: &'a str,
    activated_at_rfc3339_nanos_utc: &'a str,
    effective_from_rfc3339_nanos_utc: &'a str,
    activation_file_content_hash: &'a str,
    reviewed_by: &'a str,
    reviewed_at_rfc3339_nanos_utc: &'a str,
    artifact_valid_from: &'a str,
    artifact_expires_at: &'a str,
    executable_revision: &'a str,
}
```

`chain_config_bytes_hash = hex(sha256(exact checked-in config/chain.toml bytes))`; line endings,
comments and whitespace are intentionally release evidence, so a byte change changes the hash.
After strict TOML parsing, enabled and disabled rules are both represented in
`ChainRuleSnapshotEntryPreimage`; `rules_sorted` uses `(priority descending, chain_id UTF-8)`
and rejects duplicate `chain_id`. `keywords_in_config_order` preserves the exact TOML order and
rejects duplicate/blank values. A configured provider board contributes its already verified
binding audit hash; absence is JSON `null`.
`chain_rules_sorted_content_hash = sha256_json(ChainRulesSnapshotPreimage)`. Both subhashes and
their exact JSON/hex outputs have golden vectors.
`SelectionConfigSnapshotPreimage.chain_rules_snapshot` contains those complete parsed rules, and
`board_artifact` contains the complete canonical `ArtifactHashPreimage`; their adjacent hash fields
must recompute exactly. Therefore the config-activation recovery envelope is the executable
historical rule/binding snapshot, not merely a list of hashes. A pending fact after config reload
or restart reconstructs its exact chain matching and board request from its receipted activation
envelope without reading current `chain.toml` or the current artifact. Missing nested bytes or a
nested/hash mismatch makes that historical activation unusable and stops recovery.

`executable_revision` is deliberately not a Git commit SHA. It is
`sha256_json(ExecutableRevisionPreimage)` over the exact bytes of every regular file under
`src/`, every root `Cargo*.toml`/`Cargo.lock`/`build.rs` that exists, and every regular file under
`config/`, except `config/selection/selection_activation.v1.json`. Relative paths use `/`, are
unique UTF-8 without `.`/`..`, and sort by UTF-8 bytes; symlinks, missing roots, an unreadable file,
or a file mutation between hash and activation fails startup. `byte_len` is the exact byte count
and `content_sha256` is lowercase SHA-256 of those bytes. The board artifact and `chain.toml` are
therefore intentionally committed both by their domain hashes and by the executable-input
revision. The sole excluded activation file is runtime authorization metadata, never compiled
logic. `.git`, `target`, `data`, docs, tests and generated outputs are outside the fixed roots.
This gives a reproducible source/build-input identity without the commit-SHA → activation-file →
commit-SHA cycle. The enumerated path set, exact preimage bytes and hash have a release golden
fixture; an implementation may not substitute `git status`, mtime or directory traversal order.

`config_snapshot_json = canonical_json(SelectionConfigSnapshotPreimage)`;
`config_snapshot_json_hash = hex(sha256(config_snapshot_json UTF-8 bytes))`; and
`config_hash = sha256_json(SelectionConfigSnapshotPreimage)`. The latter two hashes MUST be
byte-for-byte equal because they commit the identical compact typed bytes; both names are retained
only to distinguish the stored JSON integrity field from the domain configuration identity.
The activation loader requires its checked-in bytes to equal the compact fixed-order JSON encoding
of its strict schema followed by exactly one LF. `activation_file_content_hash` hashes those exact
bytes, including that LF. `config_activation_content_hash =
sha256_json(ConfigActivationContentPreimage)`.

At normal/review/canary startup and every explicit config reload, the owner strictly loads the
chain file and obtains the board artifact only through the private-construction typed verified
artifact returned by `src/data_gateway/board.rs`. Config activation must not deserialize, normalize
or validate the board JSON through a second parser; it serializes the loader's already verified
complete `ArtifactHashPreimage`, including both nested directory batches and records. It may hash
the raw checked-in artifact bytes only as part of the executable-input manifest and compare them
with the sole loader's canonical bytes; it may not interpret them independently. A typed/raw
canonical-byte mismatch fails before activation.

The owner then creates the immutable sorted snapshot and reuses the exact verified activation or
commits a new
`config_activation` run through the same locked Prepared → stage-manifest → Committed → receipt
protocol. Its recovery envelope stores the exact canonical snapshot JSON, artifact validity and
activation content; the activation run manifest stores the snapshot JSON hash, config hash,
activation-file hash, activation-content hash and envelope hash. No source ingress begins before
that receipt exists. The immutable
snapshot bytes are reconstructed only from the envelope and must re-hash to the manifest value;
there is no second snapshot JSON column whose bytes could diverge.

`activated_at` is the first durable local activation attempt time and `effective_from` is the
operator-reviewed prospective effective instant; `effective_from` may not precede `activated_at`
and neither value is recalculated on restart. The operator input is the checked-in
`config/selection/selection_activation.v1.json`, whose strict schema contains exactly, in this
order, `schema_version`, `expected_config_hash`, `effective_from`, `reviewed_by` and
`reviewed_at`. Startup computes the snapshot first, requires the file's expected hash to match, and
rejects missing/unknown/duplicate fields, invalid chronology or an unreviewed value. The activation
file is the one explicitly excluded input to `executable_revision` and therefore is not part of
`config_hash`, avoiding a hash cycle; its exact content hash is part
of `ConfigActivationContentPreimage` under the exact field name `activation_file_content_hash`.
Neither the run-manifest row nor its content hash is an input to `ConfigActivationContentPreimage`;
the later run manifest may safely reference both the file hash and activation-content hash.

Startup then looks up exactly one verified, unexpired activation receipt by the computed
`config_hash`: exact content reuses that activation, while different content or multiple receipts
for the hash is an integrity conflict. A new run/time is created only for a new config hash. The
checked-in snapshot matching the current executable is the sole current activation; historical
receipts are never selected by “latest” ordering. Source ingress fails closed with
`config_not_yet_effective` until its local attempted time is at or after `effective_from`, and its
receipt time must also be at or after both the effective instant and activation receipt. This
prevents future-effective facts from entering early, normal restarts from manufacturing new
activation chronology, and executable-only upgrades from reusing an old hash.

Every source batch/fact stores `config_activation_run_id`, `config_hash` and
`generation_market_date` from the activation that was current at ingress. Eligibility requires the
ingress receipt time to be at or after the joined activation receipt time. Generation reads that
stored canonical snapshot only; it never joins “current config”. A new artifact/config activation
applies only to facts first ingressed under the new activation. Existing logical facts, pending
attempts and completed/no-relation subjects stay bound to their first activation, so renewal,
rollback or rule drift cannot replay them with current board membership.

### 5.2 Prospective-only constraint

Board evidence proves membership only at local collection time. Schema-v2 generation is therefore
prospective only: a fresh event is associated with a constituent batch collected during that live
evaluation run. It is forbidden to use current membership to reconstruct, backfill or replay
historical event candidates. A terminal sample and its commit receipt must precede D1/D3/D5
settlement. Historical board membership requires a future provider as-of contract and separate
Gate A.

The immutable generation window ends when `generation_market_date` ends in Asia/Shanghai. Relation
and selection-market requests may retry only while the local date equals that stored date and the
source fact remains bound to its original config activation. Once the date changes, the next
scheduler pass writes a receipted `failed_non_retryable` manifest with
`prospective_window_closed` and performs no board/market request. This is a prospective-evidence
boundary, not a new source-staleness decision. Outcome settlement for an already committed sample
continues on its stored schedule.

## 6. Candidate relation and de-duplication

Two formal relation variants exist:

- `DirectMention`
- `ProviderBoardConstituent`

Schema-v2 uses one `relation_schema_version = event-relation-v2` for every terminal sample. For the
same `(event_id, chain_id, canonical_stock_code)`, all resolved evidence is normalised into one
ordered set:

1. `DirectMention` evidence, ordered by exact-code before exact-name;
2. `ProviderBoardConstituent`, ordered by provider, kind and canonical board code.

The evidence-set content hash is part of terminal sample content, not `sample_key`; this prevents
late evidence from creating a second logical cohort row. `DirectMention` is only the display label
when present; it does not discard board evidence or create a second sample. A board attempt
failure does not invalidate an independently complete direct mention. Across chains, samples remain
separate because chain identity is part of the logical ID.

Logical sample and evaluation-attempt identities are separate:

```text
sample_key = sha256_json(SampleKeyPreimage)
evaluation_attempt_id = sha256_json(EvaluationAttemptPreimage)
```

Within a run, exact attempt replay is idempotent; a later run with the same failure appends a
different attempt. Evidence failures append attempts and never reserve `sample_key`. After complete T0 evidence, the
first committed `Admitted` or `HardRejected` terminal sample is authoritative. Later different
terminal content is a conflict; it is never a mutable “upgrade”.

Within one generation run, relation collection reaches a barrier before computing terminal sample
content. A configured relation identity must be either resolved or non-retryably rejected before
any terminal sample for that `(event, chain)` is staged. A retryable direct/board acquisition leaves
the generation manifest in `pending_dependency` with zero terminal samples; even an otherwise
complete direct mention waits. A later run may append new attempts and terminalize only after the
same immutable event/config snapshot reaches the barrier. Thus the terminal sample always contains
the complete ordered relation snapshot for its configured relation identities; there is no
“complete sample with a pending relation” state and no late mutation.

Thus one `(event, chain, code)` can never split into multiple cohort rows as relation evidence
arrives over time.

Stable ordering:

1. event provider publication time;
2. event ID;
3. chain priority descending;
4. chain ID;
5. relation rank (`DirectMention`, then `ProviderBoardConstituent`);
6. stock code.

No global Top-N is applied before evidence collection. Duplicate provider membership rows or
conflicting names/codes fail the constituent batch; a map overwrite is forbidden.

### 6.1 Canonical identity registry

Every BR-174 identity is SHA-256 over compact UTF-8 JSON from a dedicated `Serialize` struct with
the declared field order below. Structs contain no maps or flattened values. `Option::None` is JSON
`null` (the only `Absent` encoding) and differs from an empty string. Enum values use the lowercase
strings listed here. Every list uses the ordering explicitly declared for that field: fields named
`*_sorted` use their accompanying scalar/tuple/table-key order, while fields named
`*_in_provider_order` or `*_in_relation_order` preserve the exact evidenced order. If a list has
no field-specific order, it sorts unique scalar values by UTF-8 bytes. All list variants reject
duplicates rather than silently removing them. Only fields explicitly named
`*_rfc3339_nanos_utc` and parsed semantic market/publication times are normalized to UTC RFC3339
nanoseconds with `Z`. Opaque Gateway evidence fields such as `source_at`,
`record_observed_at`, `batch_observed_at` and Magic TDX `unix-ms:<n>` retain their exact admitted
bytes and are never rewritten. All values are checked against
`tests/fixtures/selection/br174_hash_vectors_v1.json`.

```rust
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct DirectMentionSourcePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.direct_source.v1"
    source_fact_key: &'a str,
    field: &'a str,       // "title" | "summary" | "content"
    mention_kind: &'a str,// "exact_code" | "exact_name"
    normalized_value: &'a str,
    byte_start: u32,
    byte_end: u32,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct BoardRelationSourcePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_source.v1"
    artifact_content_hash: &'a str,
    binding_audit_hash: &'a str,
    provider: &'a str,
    kind: &'a str,
    code: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct BoardNotConfiguredSourcePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.board_source_not_configured.v1"
    source_fact_key: &'a str,
    chain_id: &'a str,
    config_hash: &'a str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct RelationEvidenceEntryPreimage<'a> {
    relation_rank: u8, // 0=direct mention, 1=provider board constituent
    relation_key: &'a str,
    relation_kind: &'a str,
    relation_attempt_id: &'a str,
    relation_attempt_content_hash: &'a str,
}

#[derive(Serialize)]
struct RelationEvidenceSetPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.relation_evidence_set.v1"
    source_fact_key: &'a str,
    event_id: &'a str,
    chain_id: &'a str,
    canonical_stock_code: &'a str,
    entries_in_relation_order: &'a [RelationEvidenceEntryPreimage<'a>],
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct BindingStatePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.binding_state.v1"
    state: &'a str,       // "direct_not_applicable" | "not_configured" | "verified"
    artifact_content_hash: Option<&'a str>,
    binding_audit_hash: Option<&'a str>,
    provider: Option<&'a str>,
    kind: Option<&'a str>,
    code: Option<&'a str>,
    name: Option<&'a str>,
    error_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
struct RelationKeyPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.relation_key.v1"
    event_id: &'a str,
    chain_id: &'a str,
    config_hash: &'a str,
    relation_kind: &'a str, // "direct_mention" | "provider_board_constituent"
    relation_source_identity_hash: &'a str,
    typed_binding_state_hash: &'a str,
    relation_schema_version: &'a str,
}

#[derive(Serialize)]
struct RelationAttemptPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.relation_attempt.v2"
    stage_run_id: &'a str,
    relation_key: &'a str,
    request_hash: Option<&'a str>,
    provider_batch_id: Option<&'a str>,
    provider_observed_at: Option<&'a str>,
    result_code: &'a str,
    error_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
struct SampleKeyPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.sample_key.v1"
    event_id: &'a str,
    chain_id: &'a str,
    stock_code: &'a str,
    relation_schema_version: &'a str,
    feature_version: &'a str,
    evaluation_market_date: &'a str,
}

#[derive(Serialize)]
struct EvaluationAttemptPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.evaluation_attempt.v1"
    stage_run_id: &'a str,
    sample_key: &'a str,
    market_request_hash: &'a str,
    provider_batch_id: Option<&'a str>,
    provider_observed_at: Option<&'a str>,
    result_code: &'a str,
    error_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
struct OutcomeAttemptPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.outcome_attempt.v2"
    stage_run_id: &'a str,
    sample_key: &'a str,
    phase: &'a str, // "t0_close" | "d1_settled" | "d3_settled" | "d5_settled"
    stored_due_date: &'a str,
    request_hash: Option<&'a str>,
    provider_batch_id: Option<&'a str>,
    provider_observed_at: Option<&'a str>,
    result_code: &'a str,
    error_fingerprint: Option<&'a str>,
}
```

Every relation attempt persists both canonical JSON and hash for its source and binding state:

- direct mention:
  `relation_source_identity_json = canonical_json(DirectMentionSourcePreimage)`,
  `relation_source_identity_hash = sha256_json(DirectMentionSourcePreimage)`, and
  `BindingStatePreimage.state="direct_not_applicable"` with every optional field NULL;
- missing provider-board object:
  source JSON/hash use `BoardNotConfiguredSourcePreimage`, and
  `BindingStatePreimage.state="not_configured"` with every optional field NULL;
- verified provider board:
  source JSON/hash use `BoardRelationSourcePreimage`, and
  `BindingStatePreimage.state="verified"` requires artifact/binding/provider/kind/code/name while
  `error_fingerprint=NULL`.

For all three, `typed_binding_state_json = canonical_json(BindingStatePreimage)` and
`typed_binding_state_hash = sha256_json(BindingStatePreimage)`. No `invalid_config` runtime
variant exists: a partial/conflicting/expired config fails config activation before source ingress.
`relation_kind` selects the exact allowed source preimage parser, and the relation row's duplicated
event/chain/config/binding fields must agree. Stage and generation-receipt validation reject
noncanonical JSON, a hash-only value, or any state/NULL combination outside this matrix.

`request_hash` and `error_fingerprint` are themselves typed hashes, never free-form concatenations:

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderCapabilityHashPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.provider_capability.v1"
    provider: &'a str,
    capability_name: &'a str,
    contract_version: &'a str,
    upstream_revision: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalNewsRequestParametersPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.global_news_request.v1"
    feed_identity: &'a str,
    limit: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardConstituentRequestParametersPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.board_request.v1"
    artifact_content_hash: &'a str,
    binding_audit_hash: &'a str,
    provider: &'a str,
    kind: &'a str,
    code: &'a str,
    name: &'a str,
    limit: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct T0MarketRequestParametersPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.t0_market_request.v1"
    canonical_stock_code: &'a str,
    canonical_market: &'a str,
    evaluation_market_date: &'a str,
    quote_max_age_secs: u64,
    daily_interval: &'a str,
    daily_limit: u32,
    intraday_interval: &'a str,
    intraday_limit: u32,
    adjustment: &'a str, // exactly "none"
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeTradingDateVectorPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.outcome_trading_dates.v1"
    t0: &'a str,
    d1: &'a str,
    d2: &'a str,
    d3: &'a str,
    d4: &'a str,
    d5: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeMarketRequestParametersPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.outcome_market_request.v2"
    sample_key: &'a str,
    canonical_stock_code: &'a str,
    canonical_market: &'a str,
    phase: &'a str,
    stored_due_date: &'a str,
    calendar_version: &'a str,
    calendar_hash: &'a str,
    trading_date_vector: &'a OutcomeTradingDateVectorPreimage<'a>,
    trading_date_vector_hash: &'a str,
    applicable_trading_dates: &'a [&'a str],
    window_start: &'a str,
    window_end: &'a str,
    interval: &'a str,   // exactly "day"
    adjustment: &'a str, // exactly "none"
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestHashPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.request.v1"
    request_kind: &'a str,
    canonical_subject: &'a str,
    parameters_json_hash: &'a str,
    provider_capability_hash: &'a str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct RawSecurityIdentityPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.raw_security_identity.v1"
    provider: &'a str,
    exchange: &'a str,
    code: &'a str,
    asset_class: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderAvailableEvidencePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.provider_available_evidence.v1"
    evidence_kind: &'a str,
    provider: &'a str,
    source: Option<&'a str>,
    source_at: Option<&'a str>,
    observed_at: Option<&'a str>,
    batch_id: Option<&'a str>,
    batch_content_hash: Option<&'a str>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeProviderAvailableEvidencePreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.outcome_provider_evidence.v1"
    request_hash: &'a str,
    calendar_hash: &'a str,
    trading_date_vector_hash: &'a str,
    expected_trading_dates: &'a [&'a str],
    returned_trading_dates: &'a [&'a str],
    provider_evidence: &'a ProviderAvailableEvidencePreimage<'a>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderErrorDetailPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.provider_error_detail.v1"
    error_kind: &'a str,
    provider: &'a str,
    operation: &'a str,
    error_code: Option<&'a str>,
    http_status: Option<u16>,
    timeout_ms: Option<u64>,
    invariant_id: Option<&'a str>,
    diagnostic_code: &'a str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct T0FeaturePreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.t0_feature.v1"
    feature_version: &'a str,
    evaluation_window: &'a str, // "intraday" | "post_close"
    ma5: Option<&'a str>,
    ma10: Option<&'a str>,
    ma20: Option<&'a str>,
    five_day_return: Option<&'a str>,
    volume_vs_5d: Option<&'a str>,
    volume_vs_20d: Option<&'a str>,
    intraday_volume_pace: Option<&'a str>,
    price_vs_ma5: Option<&'a str>,
    price_vs_ma10: Option<&'a str>,
    price_vs_ma20: Option<&'a str>,
    evaluation_price: &'a str,
    observed_volume: &'a str,
    latest_settled_market_date: &'a str,
    latest_settled_close: &'a str,
    latest_settled_volume: &'a str,
    prior_5d_average_volume: &'a str,
    prior_20d_average_volume: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AdmissionStructuredDetailPreimage<'a> {
    MovingAverageNonpositive {
        ma5: &'a str,
        ma10: &'a str,
        ma20: &'a str,
    },
    TrendAlignmentFailed {
        ma5: &'a str,
        ma10: &'a str,
        ma20: &'a str,
    },
    PriceBelowMa5 {
        value: &'a str,
        inclusive_min: &'a str,
    },
    PriceMa20DistanceOutOfRange {
        value: &'a str,
        inclusive_min: &'a str,
        inclusive_max: &'a str,
    },
    FiveDayReturnOutOfRange {
        value: &'a str,
        inclusive_min: &'a str,
        inclusive_max: &'a str,
    },
    SettledVolumeConfirmationFailed {
        volume_vs_5d: &'a str,
        volume_vs_20d: &'a str,
        inclusive_min: &'a str,
    },
    IntradayVolumeConfirmationFailed {
        intraday_volume_pace: &'a str,
        inclusive_min: &'a str,
    },
}

#[derive(Serialize)]
struct ErrorFingerprintPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.error.v1"
    failed_stage: &'a str,
    reason_code: &'a str,
    retryable: bool,
    available_evidence_hash: Option<&'a str>,
    detail_hash: &'a str,
}
```

All financial numbers in `T0FeaturePreimage` and
`AdmissionStructuredDetailPreimage` use a canonical finite-decimal JSON string: reject NaN,
infinity and negative zero, then format the IEEE-754 value with Rust `ryu` shortest round-trip
format without trimming or locale conversion. Positive zero is exactly `"0"`. Dates are
`YYYY-MM-DD`; hashes are lowercase 64-hex. The typed admission enum is the complete set of hard
rejection detail variants for admission-v1. Missing/non-finite features are provider/evaluation
evidence failures and therefore remain on the evaluation attempt; they are not converted into a
hard-rejection detail variant.

`ProviderErrorDetailPreimage.error_kind` is exactly one of
`transport | protocol | timeout | invalid_data | unsupported | integrity`.
`timeout` alone requires `timeout_ms`; `protocol` may carry `http_status`; `integrity` alone
requires `invariant_id`; all other disallowed option fields are JSON `null`.
`diagnostic_code` is a non-empty lowercase ASCII snake-case value from the checked-in
`provider_error_codes_v1.json` registry. Gate B implements one exhaustive typed `match` from every
Gateway/provider error variant and failed stage to `(error_kind, diagnostic_code, retryable)`;
the fixture and compile-time coverage test are the authority. Raw/display error strings, URLs,
headers, cookies, tokens, query values and filesystem paths are not inputs to any BR-174 identity
and are never persisted in these JSON/hash columns, so no redaction algorithm can diverge. An
unmapped/new upstream error variant produces the fixed non-retryable integrity detail
`error_kind="integrity"`, `diagnostic_code="provider_error_mapping_missing"` and
`invariant_id="provider-error-codes-v1"`; it cannot borrow a free-form message or be silently
classified retryable.

`ProviderAvailableEvidencePreimage.evidence_kind` is exactly
`board_constituents | t0_market_bundle | outcome_daily_bars`. When the hash is present,
`provider` is required and at least one of `source/source_at/observed_at/batch_id/
batch_content_hash` is non-NULL. Complete success variants require every field except the
provider-optional `source_at`; partial failure variants populate only observed fields. A row with
no provider evidence stores JSON/SQL NULL rather than hashing an all-NULL object.

Every remaining semantic JSON/hash pair has one closed construction:

- source-batch `Available/VerifiedEmpty`:
  `available_evidence_json = canonical_json(FeedBatchEvidencePreimage)` and
  `available_evidence_hash = sha256_json(FeedBatchEvidencePreimage)`;
- source-batch `Unavailable` with partial evidence:
  `available_evidence_json = canonical_json(FeedAvailableEvidencePreimage)` and its hash is
  `sha256_json` of that type; with no partial evidence both are NULL;
- relation/evaluation/outcome attempt with evidence:
  `available_evidence_json = canonical_json(ProviderAvailableEvidencePreimage)` and
  `available_evidence_hash = sha256_json(ProviderAvailableEvidencePreimage)`; with none both are
  NULL;
- a relation raw identity:
  `raw_identity_json = canonical_json(RawSecurityIdentityPreimage)` and
  `raw_identity_hash = sha256_json(RawSecurityIdentityPreimage)`;
- a terminal T0 sample:
  `t0_feature_json = canonical_json(T0FeaturePreimage)` and
  `t0_feature_hash = sha256_json(T0FeaturePreimage)`;
- a hard-rejection child:
  `structured_detail_json = canonical_json(AdmissionStructuredDetailPreimage)` and
  `structured_detail_hash = sha256_json(AdmissionStructuredDetailPreimage)`;
- any provider failure:
  `error_detail_json = canonical_json(ProviderErrorDetailPreimage)`,
  `error_detail_hash = sha256_json(ProviderErrorDetailPreimage)`, and
  `error_fingerprint = sha256_json(ErrorFingerprintPreimage)` with the identical detail/evidence
  hashes.

All paired JSON and hash columns are both NULL or both non-NULL. Stage/receipt validation parses
the JSON as the named `deny_unknown_fields` type, verifies the exact canonical bytes and recomputes
the hash; the row's duplicated provider/evidence/feature/reason fields must equal the parsed
preimage. Successful variants require error-detail/fingerprint fields NULL. Failure variants
require the provider-error pair and fingerprint, except a source-batch `Unavailable` stores its
error-detail pair in the feed attempt and carries the same `error_detail_hash` as
`FeedAttemptContentPreimage.detail_hash`. No row may persist only a bare hash whose typed preimage
cannot be recovered from the row/envelope.

For every request,
`parameters_json_hash = sha256_json(<RequestKind>ParametersPreimage)`,
`provider_capability_hash = sha256_json(ProviderCapabilityHashPreimage)`, and
`request_hash = sha256_json(RequestHashPreimage)`. The mappings are closed:

- feed fetch: `request_kind=global_news`,
  `canonical_subject=RegisteredFeedIdentityPreimage.feed_identity`;
- board expansion: `request_kind=board_constituents`,
  `canonical_subject=binding_audit_hash`;
- T0 evaluation: `request_kind=t0_market_evidence`,
  `canonical_subject=canonical_stock_code/evaluation_market_date`;
- settlement: `request_kind=outcome_market_evidence`,
  `canonical_subject=sample_key/phase/stored_due_date`.

The slash-delimited subjects are display-independent ASCII because every component is already a
validated canonical identity/date and none permits `/`; their exact strings have golden vectors.
Parameter values are copied from the actual typed Gateway request before provider execution and
recomputed against the attempt row during stage/receipt validation. Provider capability
fields use this exact checked-in mapping; every row uses upstream revision
`b2b68df78156df1d67824e5c44c0cb01b752f55a`:

| request | provider | capability_name | contract_version |
|---|---|---|---|
| Eastmoney feed | `eastmoney` | `GlobalNews-Eastmoney` | `magic-market-core.NewsProvider.global_news.v0.2.0` |
| CLS feed | `cailianpress` | `GlobalNews-CLS` | `magic-market-core.NewsProvider.global_news.v0.2.0` |
| Jin10 feed | `jin10` | `GlobalNews-Jin10` | `magic-market-core.NewsProvider.global_news.v0.2.0` |
| ThePaper feed | `thepaper` | `GlobalNews-ThePaper` | `magic-market-core.NewsProvider.global_news.v0.2.0` |
| board expansion | `magic-tdx` | `MagicTdx-BoardConstituents` | `magic-tdx-rs.BlockProvider.board_constituents.v0.2.0` |
| T0 quote/daily/intraday bundle | `magic-tdx` | `MagicTdx-T0MarketBundle` | `stock-analysis.MagicTdxSelectionGateway.t0_market_evidence.v1` |
| outcome daily bars | `magic-tdx` | `MagicTdx-UnadjustedDailyBars` | `magic-market-core.MarketDataProvider.bars.v0.2.0` |

The T0 row is one downstream atomic bundle capability: its parameter preimage lists every
underlying typed request, and the bundle is unavailable if any required member is unavailable. A
version/provider/revision change changes the hash and requires new vectors.

Provider error detail is first redacted of secrets, serialized as its typed structured detail, then
hashed; display text is not an identity. These definitions replace every informal `hash(...)`
expression elsewhere in this document.

### 6.2 Gate A amendment: recoverable typed request evidence

The final `request_hash` alone is not enough evidence to perform the stage/read-back/receipt
recomputation required above. Every provider request owned by ingress, generation or settlement
therefore persists the exact canonical request inputs alongside the final hash:

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestEvidencePreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.request_evidence.v1"
    request_kind: &'a str,
    canonical_subject: &'a str,
    parameters_schema: &'a str,
    parameters_json: &'a str,
    parameters_json_hash: &'a str,
    provider_capability_json: &'a str,
    provider_capability_hash: &'a str,
    request_hash: &'a str,
}
```

`request_evidence_json = canonical_json(RequestEvidencePreimage)` and
`request_evidence_hash = sha256_json(RequestEvidencePreimage)`. The row's pair is both NULL or both
non-NULL. When non-NULL, the validator requires byte-for-byte canonical JSON, recomputes
`request_evidence_hash`, and compares it with the projected pair before using any nested field.

The request contract is the following closed matrix. A validator accepts only one complete row;
independently valid values from different rows cannot be mixed:

| `request_kind` | `parameters_schema` / parser | exact `canonical_subject` construction | provider capability mapping |
|---|---|---|---|
| `global_news` | `global-news-request-v1` / `GlobalNewsRequestParametersPreimage` | exactly `parameters.feed_identity` | the registered feed row selects exactly one Eastmoney/CLS/Jin10/ThePaper tuple from the capability table above |
| `board_constituents` | `board-constituents-request-v1` / `BoardConstituentRequestParametersPreimage` | exactly `parameters.binding_audit_hash` | exactly the `magic-tdx` / `MagicTdx-BoardConstituents` tuple above |
| `t0_market_evidence` | `t0-market-request-v1` / `T0MarketRequestParametersPreimage` | exactly `parameters.canonical_stock_code + "/" + parameters.evaluation_market_date` | exactly the `magic-tdx` / `MagicTdx-T0MarketBundle` tuple above |
| `outcome_market_evidence` | `outcome-market-request-v2` / `OutcomeMarketRequestParametersPreimage` | exactly `parameters.sample_key + "/" + parameters.phase + "/" + parameters.stored_due_date` | exactly the `magic-tdx` / `MagicTdx-UnadjustedDailyBars` tuple above |

Every selected capability tuple includes the exact provider, capability name, contract version and
upstream revision from the preceding table. A `request_kind`/schema swap, subject copied from a
different parameter set, unregistered feed-to-capability mapping or cross-row capability tuple is
invalid even when every nested JSON/hash pair is internally self-consistent.

Validation parses the selected `parameters_json` with `deny_unknown_fields`, requires exact
canonical compact bytes, and recomputes `parameters_json_hash`. It separately parses
`provider_capability_json` as the strict typed `ProviderCapabilityHashPreimage`, requires its exact
canonical compact bytes, recomputes `provider_capability_hash`, and requires the nested provider,
capability name, contract version and upstream revision to equal the registered mapping table
above. The two projected nested hashes must equal the hashes copied into the reconstructed
`RequestHashPreimage`; its hash must equal both `RequestEvidencePreimage.request_hash` and the row's
projected request-hash column. A generic JSON object, a hash-only value, or an unregistered
capability tuple is invalid.

The persistent row-content preimages gain `request_evidence_json` and
`request_evidence_hash` immediately after their projected request-hash field:

- every registered source-batch feed attempt requires all three request fields, including
  Available, VerifiedEmpty, transport/error Unavailable and a pre-call capability Unavailable;
  request evidence proves the exact planned typed request and does not fabricate provider response
  evidence. Its `request_evidence_hash` is also copied into `FeedAttemptContentPreimage`, which
  therefore binds the request preimage rather than only its final digest;
- a direct mention and a `not_configured` board relation made no provider request, so
  `request_hash`, `request_evidence_json`, and `request_evidence_hash` are all NULL;
- a verified board relation that calls the provider requires all three;
- every T0 evaluation attempt requires all three, including provider errors after the typed
  request was constructed;
- a settled/error outcome provider attempt requires all three;
- `ExpectedWait` performs no provider call, so its three request fields are all NULL.

This last rule resolves the prior contradiction where `ExpectedWait` was described as having no
provider request but its row and `OutcomeAttemptPreimage` still required a request hash.
`RelationAttemptPreimage.request_hash` and `OutcomeAttemptPreimage.request_hash` are correspondingly
`Option<&str>`; evaluation remains non-optional. The attempt ID still hashes the exact optional
state, so a no-call wait cannot collide with a provider attempt. No request preimage may be
reconstructed from display text, defaults, the current configuration, or a later gateway call.
This amendment supersedes every earlier hash-only feed/relation/evaluation/outcome table sentence
and field registry entry in this document.

Every table also has a dedicated fixed-order `<TableName>RowContentPreimage` containing every
immutable semantic/evidence column named in §7, including local attempted/staged chronology, but
excluding SQLite `rowid` and the row's own `content_hash`. Its domain is
`stock_analysis.br174.<table_name>_row.v1`, except the four request-bearing attempt rows amended
here, whose exact domains are
`stock_analysis.br174.selection_source_batch_attempts_row.v2`,
`stock_analysis.br174.selection_relation_attempts_row.v2`,
`stock_analysis.br174.selection_evaluation_attempts_row.v2`, and
`stock_analysis.br174.selection_outcome_attempts_row.v2`; the full-calendar sample row uses
`stock_analysis.br174.selection_samples_row.v2`. DDL review and a compile-time field-list test
require
one-to-one coverage between persistent columns and the preimage; an added/reordered/omitted column
is a schema-version change and fails the golden vectors. Run-row hashes use only those verified row
content hashes.

The fixed field registry, in serialization order, is:

1. `SelectionSourceBatchAttemptRowContentPreimage`: domain, source_batch_attempt_id,
   ingress_run_id, config_activation_run_id, config_hash, generation_market_date,
   registered_feed_identity, registered_feed_snapshot_hash, request_hash, request_evidence_json,
   request_evidence_hash, feed_attempt_content_hash, status_kind, record_count, provider, source,
   source_at, observed_at, batch_id, batch_content_hash, failed_stage, reason_code, retryable,
   available_evidence_json, available_evidence_hash, error_detail_json, error_detail_hash,
   error_fingerprint, attempted_at.
2. `SelectionSourceFactRowContentPreimage`: domain, source_fact_key, event_id, payload_schema,
   config_activation_run_id, config_hash, generation_market_date, provider_source, item_id, title,
   summary, content, publisher, canonical_url, published_at, instruments_json, topics_json,
   language, record_provider, record_source, record_source_at, record_observed_at, record_batch_id,
   record_batch_content_hash, provider_content_hash, first_ingress_run_id, ingress_gate_version,
   ingress_gate_input_json, ingress_gate_input_hash, ingress_decision, ingress_reason_code,
   ingress_retryable, ingress_gate_receipt_json, ingress_gate_receipt_hash.
3. `SelectionSourceFactAttemptRowContentPreimage`: domain, source_fact_attempt_id, ingress_run_id,
   source_batch_attempt_id, provider_ordinal, source_fact_key, acquired_record_json,
   acquired_record_hash, batch_evidence_json, batch_evidence_hash, event_projection_id,
   attempt_result, conflict_hash, attempted_at.
4. `SelectionRelationAttemptRowContentPreimage`: domain, relation_attempt_id, relation_key,
   generation_run_id, source_fact_key, event_id, chain_id, config_activation_run_id, config_hash,
   relation_schema_version, relation_kind, relation_source_identity_json,
   relation_source_identity_hash, typed_binding_state_json, typed_binding_state_hash,
   optional request_hash, request_evidence_json, request_evidence_hash, result_code, failed_stage,
   retryable, raw_identity_json, raw_identity_hash,
   canonical_stock_code, canonical_stock_name, canonical_market, artifact_content_hash, binding_audit_hash,
   provider_board_kind, provider_board_code, provider_board_name, provider_source,
   provider_source_at, provider_observed_at, provider_batch_id, provider_batch_content_hash,
   actual_constituent_count, available_evidence_json, available_evidence_hash, error_detail_json,
   error_detail_hash, error_fingerprint, attempted_at.
5. `SelectionEvaluationAttemptRowContentPreimage`: domain, evaluation_attempt_id, sample_key,
   generation_run_id, source_fact_key, event_id, chain_id, canonical_stock_code,
   canonical_stock_name, canonical_market, relation_evidence_set_hash, market_request_hash,
   request_evidence_json, request_evidence_hash, result_code, failed_stage, retryable, provider,
   source, source_at, observed_at, batch_id,
   batch_content_hash, available_evidence_json, available_evidence_hash, terminal_decision_hash,
   error_detail_json, error_detail_hash, error_fingerprint, attempted_at.
6. `SelectionSampleRowContentPreimage`: domain, sample_key, generation_run_id, source_fact_key,
   source_fact_content_hash, source_fact_attempt_id, source_batch_attempt_id, event_id, chain_id,
   config_activation_run_id, config_hash, matched_keyword, canonical_stock_code,
   canonical_stock_name, canonical_market, relation_schema_version, relation_evidence_json,
   relation_evidence_set_hash, feature_version, t0_feature_json, t0_feature_hash, market_provider,
   market_source, market_source_at, market_observed_at, market_batch_id, market_batch_content_hash,
   admission_version, decision_kind, rejection_count, rejection_row_hashes_in_ordinal_order,
   evaluation_market_date, t0_due_date, d1_due_date, d2_due_date, d3_due_date, d4_due_date,
   d5_due_date, calendar_version, calendar_hash, trading_date_vector_json,
   trading_date_vector_hash, staged_at.
7. `SelectionRejectionRowContentPreimage`: domain, sample_key, ordinal, generation_run_id,
   reason_code, rule_id, retryable, structured_detail_json, structured_detail_hash, provider,
   source, source_at, observed_at, batch_id, batch_content_hash, created_at.
8. `SelectionSampleOutcomeRowContentPreimage`: domain, sample_key, phase, outcome_run_id,
   due_trading_date, open, high, low, close, volume, amount, return_from_t0_close,
   cumulative_mfe, cumulative_mae, volume_ratio, provider, source, source_at, observed_at, batch_id,
   batch_content_hash, created_at.
9. `SelectionOutcomeAttemptRowContentPreimage`: domain, outcome_attempt_id, sample_key, phase,
   stored_due_date, outcome_run_id, optional request_hash, request_evidence_json,
   request_evidence_hash,
   result_code, reason_code, retryable, provider, source, source_at, observed_at, batch_id,
   batch_content_hash, available_evidence_json, available_evidence_hash, error_detail_json,
   error_detail_hash, error_fingerprint, settled_outcome_content_hash, attempted_at.
10. `SelectionRecoveryEnvelopeRowContentPreimage`: domain, stage_run_id, subject_kind,
    logical_subject_key, payload_schema, payload_json, payload_json_hash, in_memory_payload_hash,
    config_activation_run_id, config_hash, enveloped_at.
11. `SelectionRunStageRowContentPreimage`: exactly the fields of
    `RunManifestContentPreimage`; `subject_id` is the persisted `stage_run_id`. The table's
    `manifest_content_hash` is this preimage's own hash and is excluded from the preimage.
12. `SelectionCommitReceiptRowContentPreimage`: exactly the fields of
    `CommitReceiptContentPreimage`. The receipt table's `content_hash` is this preimage's own hash
    and is excluded from its preimage like every other row's own `content_hash`; golden vectors
    cover both the canonical preimage bytes and resulting hash.

Every optional field above is present as JSON `null`; variant CHECK matrices decide which are
required. Gate B may not add a persistent semantic column that is absent from this registry.

## 7. Append-only schema v2

Existing `selection_candidates` and `selection_outcomes` are legacy-v1 only. Schema-v2 shadow
visibility is the read model:

```sql
SELECT s.*
FROM selection_samples AS s
JOIN selection_source_facts_v2 AS f ON f.source_fact_key = s.source_fact_key
JOIN selection_v2_commit_receipts AS ar
  ON ar.subject_kind = 'config_activation'
 AND ar.subject_id = s.config_activation_run_id
JOIN selection_v2_commit_receipts AS ir
  ON ir.subject_kind = 'ingress_run'
 AND ir.subject_id = f.first_ingress_run_id
JOIN selection_v2_commit_receipts AS gr
  ON gr.subject_kind = 'generation_run'
 AND gr.subject_id = s.generation_run_id
WHERE f.ingress_decision = 'admitted'
  AND s.decision_kind = 'admitted';
```

No v2 row is written to the v1 candidate/inbox/run/feature foreign-key graph. Hard-rejected rows
remain available only to the separately named research report/backtest queries, never this shadow
visibility view.

Migration precondition: `DatabaseManager` configures every pooled SQLite connection with
`PRAGMA foreign_keys=ON` and `PRAGMA synchronous=FULL` outside a transaction, immediately reads
back `foreign_keys=1` and `synchronous=2`, and rejects the connection otherwise. WAL and busy
timeout remain explicit. A migration does not start until the pool-wide invariant and the
pre-migration `PRAGMA foreign_key_check` both pass.

### 7.0 Exact key and direct-SQL constraint matrix

All foreign keys are `ON UPDATE RESTRICT ON DELETE RESTRICT`; those staged parent/child rows in one
transaction are `DEFERRABLE INITIALLY DEFERRED`. Exact keys are:

| table | primary key | additional foreign/unique constraints |
|---|---|---|
| `selection_source_batch_attempts` | `source_batch_attempt_id` | FK `ingress_run_id -> recovery_envelopes.stage_run_id`; UNIQUE `(ingress_run_id, registered_feed_identity)` |
| `selection_source_facts_v2` | `source_fact_key` | FK `first_ingress_run_id -> recovery_envelopes.stage_run_id`; config-activation receipt checked by trigger |
| `selection_source_fact_attempts` | `source_fact_attempt_id` | FK source batch, source fact and ingress envelope; UNIQUE `(source_batch_attempt_id, provider_ordinal)` |
| `selection_relation_attempts` | `relation_attempt_id` | FK source fact and generation envelope; UNIQUE `(generation_run_id, relation_key)` |
| `selection_evaluation_attempts` | `evaluation_attempt_id` | FK source fact and generation envelope; UNIQUE `(generation_run_id, sample_key)`; `sample_key` is not an FK because evidence-failure attempts intentionally have no terminal sample |
| `selection_samples` | `sample_key` | FK source fact, source-fact attempt, source-batch attempt and generation envelope; UNIQUE `(source_fact_key, chain_id, canonical_stock_code, config_hash)` |
| `selection_rejections` | `(sample_key, ordinal)` | FK sample and generation envelope |
| `selection_sample_outcomes` | `(sample_key, phase)` | FK sample and outcome envelope |
| `selection_outcome_attempts` | `outcome_attempt_id` | FK sample and outcome envelope; UNIQUE `(outcome_run_id, sample_key, phase)` |
| `selection_v2_recovery_envelopes` | `stage_run_id` | UNIQUE `(stage_run_id, payload_json_hash, in_memory_payload_hash)` |
| `selection_v2_run_stages` | `subject_id` | FK `subject_id -> recovery_envelopes.stage_run_id`; UNIQUE `(subject_kind, subject_id)` plus the config-activation partial unique index |
| `selection_v2_commit_receipts` | `(subject_kind, subject_id)` | FK `(subject_kind, subject_id) -> run_stages(subject_kind, subject_id)` and FK `subject_id -> recovery_envelopes.stage_run_id` |

Every domain-table INSERT trigger requires a same-kind recovery envelope, matching run/config
fields and absence of an existing receipt; UPDATE/DELETE always abort. The manifest INSERT trigger
recomputes the envelope payload, exact allowed table set, row keys/hashes, staged row count and
run-kind optional matrix. The receipt INSERT trigger then performs a kind-specific closure:

- config activation: verify the config snapshot/file/content hashes and immutable legacy-cutover
  snapshot carried by its envelope/manifest;
- ingress: verify the activation receipt, registered-feed snapshot, provider ordinals, all feed/
  fact children, aggregate source-batch hash and every source fact's ingress-gate input/receipt
  hash, including permanently rejected facts. `admitted` requires NULL reason/retryable;
  `rejected` requires a checked-in non-empty gate reason and `retryable=false`; input/receipt
  config, publication/batch/observation/evaluation time and gate version must equal the immutable
  attempt/evidence/activation fields. Only the later generation eligibility query filters to
  `admitted`;
- generation: verify the activation receipt and, for every source/sample lineage, the source
  fact's `first_ingress_run_id` ingress receipt, require
  `source_fact.ingress_decision='admitted'` in both every generation-domain INSERT trigger and the
  receipt trigger, and verify matching source-fact/source-batch attempts, complete relation set,
  terminal-decision hash and rejection matrix. A receipted but ingress-rejected fact can never
  parent a relation/evaluation/sample row, even through direct SQL;
- outcome: verify the activation, source ingress and generation receipts in the sample lineage,
  the required preceding receipted phases, and the exact outcome status/cardinality matrix:
  every run has exactly one attempt for its `(outcome_run_id, sample_key, phase)`; `settled` has
  exactly one matching outcome whose content hash equals the attempt's
  `settled_outcome_content_hash`; `expected_wait`, `failed_retryable` and
  `failed_non_retryable` have zero outcome rows and the corresponding typed attempt result.

Thus a structurally plausible direct-SQL row without its complete upstream receipt lineage cannot
obtain a downstream receipt. The external audit-chain validation in §7.6 remains an additional
mandatory trust boundary.

### 7.1 `selection_source_batch_attempts`

One row per `(ingress_run_id, registered_feed_identity)`. It persists the exact registered feed
snapshot, request identity, typed status and content hash before relation evaluation:

Every row requires the §6.2 `global-news-request-v1` request-evidence pair. Its feed identity and
limit must equal the corresponding registered snapshot entry, and its provider capability must
equal that feed's checked-in tuple below. This remains required for every terminal status because
the typed request is constructed before either the real provider call or a pre-call capability
failure. Provider response evidence remains independently absent on failure.
`FeedAttemptContentPreimage.request_evidence_hash` must equal the row's
`request_evidence_hash`; stage/read-back/receipt validation recomputes the nested parameters,
capability, final request hash, feed-attempt content hash and projection equality.

The production registered-feed registry is a checked-in constant with exactly these rows:

| feed_name | gateway_provider/provider_id | source_contract | capability_name | max_limit |
|---|---|---|---|---:|
| `eastmoney_global_news` | `eastmoney` / `eastmoney` | `eastmoney-web` | `GlobalNews-Eastmoney` | 20 |
| `cls_global_news` | `cailianpress` / `cailianpress` | `cls-v1` | `GlobalNews-CLS` | 20 |
| `jin10_global_news` | `jin10` / `jin10` | `jin10-flash-v1` | `GlobalNews-Jin10` | 20 |
| `thepaper_global_news` | `thepaper` / `thepaper` | `thepaper-finance-v1` | `GlobalNews-ThePaper` | 20 |

Every row also uses upstream revision
`b2b68df78156df1d67824e5c44c0cb01b752f55a`.
`configuration_hash = sha256_json(RegisteredFeedConfigurationPreimage)` and
`feed_identity = sha256_json(RegisteredFeedIdentityPreimage)`. Runtime registration with a
missing/extra/different row fails startup; it cannot silently change snapshot membership.

The owner builds `RegisteredFeedEntryPreimage` rows sorted uniquely by `feed_identity`; `ordinal`
must equal the zero-based position in that sorted slice.
`registered_feed_snapshot_json = canonical_json(RegisteredFeedSnapshotPreimage)` and
`registered_feed_snapshot_hash = sha256_json(RegisteredFeedSnapshotPreimage)`. The identical JSON
and hash are stored in `SourceIngressStageInputPreimage`; every feed row stores that hash. The
ingress receipt trigger parses the envelope snapshot, recomputes its hash, and requires exactly one
attempt row for every entry and no unregistered row.

- `Available`: `record_count IS NOT NULL AND record_count > 0` and full
  provider/source/source_at/observed/batch evidence required;
- `VerifiedEmpty`: `record_count IS NOT NULL AND record_count = 0` and the same full batch evidence
  required;
- `Unavailable`: `record_count IS NULL`, non-empty failed stage/reason code, explicit upstream
  retryability boolean,
  canonical error-detail JSON/hash and recomputable error fingerprint required; complete
  evidence/content fields remain NULL, while an optional non-empty
  `FeedAvailableEvidencePreimage` hash retains only evidence actually obtained before failure.

Variant CHECK constraints forbid complete success evidence on `Unavailable` and forbid missing
complete evidence on `Available/VerifiedEmpty`. Success rows require canonical
`available_evidence_json`, `available_evidence_hash=evidence_hash` and NULL error detail;
unavailable rows require `evidence_hash` and
`source_content_hash` NULL and may carry only the partial `available_evidence_hash`.
Success also requires failed-stage/reason/retryability/error-fingerprint NULL; `Unavailable`
requires all four and its fingerprint must hash the identical stage/reason/retryability/evidence/
detail values.
`(ingress_run_id, feed_identity)` is unique, and the source batch content hash includes the complete
registered-feed set in stable feed identity order. A global complete, empty or no-relation query
inner-joins the `ingress_run` receipt and requires every registered feed row to be
`Available/VerifiedEmpty`.

DDL declares `record_count INTEGER` without `NOT NULL`. Startup must read
`PRAGMA table_xinfo(selection_source_batch_attempts)`, require the `record_count` row to have
`notnull=0`, and validate the canonical table CHECK as the exact three-way matrix above. The Rust
stage validator and receipt trigger repeat the same `IS NULL`/`IS NOT NULL` conditions. Merely
finding a column named `record_count`, accepting an older `NOT NULL DEFAULT 0`, or relying on
`record_count > 0` where SQL NULL could pass through three-valued logic is a schema mismatch.

The per-feed evidence/content hashes have a closed, non-circular construction:

1. `evidence_hash = sha256_json(FeedBatchEvidencePreimage)` over the exact provider batch evidence.
   `batch_quality` is the literal `complete`; no inferred source time or batch identity is allowed.
2. For every admitted provider record, in the exact order returned by the Gateway, compute
   `record_hash = sha256_json(FeedSourceRecordHashPreimage)` with a zero-based
   `provider_ordinal`, the already-computed `source_fact_key`, and the provider-owned
   `provider_content_hash`.
3. `source_content_hash = sha256_json(FeedSourceContentPreimage)` over `feed_identity`,
   `evidence_hash`, and those record hashes in provider order. `Available` requires a non-empty
   list whose length equals `record_count`; `VerifiedEmpty` requires an empty list and
   `record_count = 0`. `Unavailable` has NULL `evidence_hash`, NULL `source_content_hash`, and no
   record list.
4. For an `Available` feed, construct each `AcquiredGlobalNewsRecordPreimage` only now, with
   `record_batch_content_hash` set exactly to step 3's `source_content_hash`; then compute and
   persist its JSON/hash pair. The first immutable source-fact row copies the same value into
   `record_batch_content_hash`. This is a derived per-feed ordered-batch content hash, not a field
   claimed to have been supplied by upstream and never the aggregate
   `source_batch_content_hash`. The construction is acyclic because step 3 uses only
   `FeedSourceRecordHashPreimage` (source-fact key + provider-owned content hash), never the
   acquired-record hash. `VerifiedEmpty`/`Unavailable` have no acquired child records.
5. `feed_attempt_content_hash = sha256_json(FeedAttemptContentPreimage)` then commits the typed
   attempt result. Its `evidence_hash` and `source_content_hash` fields are the outputs from steps
   1 and 3 for `Available/VerifiedEmpty`; its `available_evidence_hash` equals step 1 for those
   success variants. For `Unavailable`, the first two are JSON `null` and
   `available_evidence_hash` is either JSON `null` or the hash of
   `FeedAvailableEvidencePreimage`; its `detail_hash` equals the row's
   `error_detail_hash`, and its `failed_stage/reason_code/retryable/error_fingerprint` equal the
   row values. Success variants require all five failure fields JSON `null`; `Unavailable`
   requires them and recomputes the fingerprint from `ErrorFingerprintPreimage`.
6. Only after every feed attempt hash exists, construct `SourceBatchContentPreimage` with
   the immutable registered-feed snapshot hash, `feed_attempt_hashes_in_registered_feed_order`
   ordered by that snapshot, and source-record hashes/event-projection IDs in snapshot-feed order
   then zero-based provider order. Then compute
   `source_batch_content_hash = sha256_json(SourceBatchContentPreimage)`. Empty child-ID slices
   serialize as `[]`, never `null`.

Consequently `source_content_hash` never contains the aggregate source-batch hash and
`SourceBatchContentPreimage` never feeds back into a per-feed hash. Provider order is evidence:
implementations MUST NOT sort records before hashing. A provider that cannot attest a complete
ordered batch is `Unavailable`, not `VerifiedEmpty`.

`SourceBatchContentPreimage`, `FeedAttemptContentPreimage` and every child record hash contain no
`ingress_run_id`, `stage_run_id`, `source_batch_attempt_id` or other locally allocated attempt
identity. The aggregate hash is therefore a content/acquisition identity: an exact concurrent
replay with identical registered-feed snapshot, provider evidence/records/order, projection IDs
and aggregator observation time has the same ingress logical subject even when each process
allocated a different run ID. A genuinely later provider acquisition normally has new provider or
aggregator observation evidence and is a different source batch while still reusing immutable
source facts. Run-scoped attempt IDs remain stored and audited but never weaken the locked duplicate
recheck.

`source_batch_attempt_id = sha256_json(FeedAttemptKeyPreimage)`. The logical
`feed_attempt_content_hash = sha256_json(FeedAttemptContentPreimage)` from §4.1 is one persistent
column used by `SourceBatchContentPreimage`; it is not the table row hash. The table row
`content_hash` is exclusively
`sha256_json(SelectionSourceBatchAttemptRowContentPreimage)` from §6.1 and therefore covers every
persistent semantic/evidence field exactly once. In that row preimage,
`available_evidence_hash` is exactly step 1's `evidence_hash` for success or the optional partial
evidence hash for `Unavailable`, and `available_evidence_json` is the matching canonical typed
preimage from §6.1; `batch_content_hash` is exactly step 3's
`source_content_hash` for success and NULL for `Unavailable`. The differently named persistent
columns follow the repository-wide provider batch schema and MUST NOT be independently recomputed
or populated from `source_batch_content_hash`. For `Unavailable`, each non-NULL
provider/source/source-at/observed/batch field in the row must equal the corresponding
`FeedAvailableEvidencePreimage` field; fields not obtained remain NULL, at least one evidence field
is required when `available_evidence_hash` is non-NULL, and the trigger recomputes that hash.
`Unavailable` additionally requires canonical `error_detail_json/error_detail_hash`, and
`FeedAttemptContentPreimage.detail_hash` and error fingerprint must equal that row's values;
success requires all error fields NULL.

No-loss is enforced twice before an ingress receipt can exist. `stage_source_ingress()` inserts all
source-fact attempts in the same FULL transaction as the feed rows and verifies, per feed identity,
that an `Available` row has exactly `record_count` distinct child attempts whose record identities
and batch evidence hash belong to that feed and whose unique `provider_ordinal` values are exactly
the contiguous set `0..record_count-1`, while `VerifiedEmpty` and `Unavailable` have exactly zero
child attempts. A UNIQUE constraint on `(source_batch_attempt_id, provider_ordinal)` and the
receipt trigger enforce that same contiguous provider order. The ingress-receipt INSERT trigger
recomputes each `FeedSourceRecordHashPreimage` in ordinal order, recomputes
`FeedSourceContentPreimage`, requires every child acquired-record JSON and first source-fact
`record_batch_content_hash` to equal that recomputed per-feed hash, and repeats the grouped
count/identity/batch-hash invariant for every
feed in the immutable registered-feed snapshot; it aborts on a missing, extra, duplicated,
reordered or cross-feed child. Thus a subset or reordered copy of a provider batch cannot be
receipted as a complete ingress run even through direct SQL.

### 7.2 `selection_source_facts_v2`

One row per deterministic `source_fact_key`, containing payload schema
`global-news-source-fact-v2`, immutable provider-owned content, logical content hash, deterministic
event projection identity, `config_activation_run_id`, `config_hash`,
`generation_market_date`, `first_ingress_run_id`, ingress gate
version/input/result/receipt and audit hashes. The activation and ingress receipts are both required
before the fact is eligible. The first source-schema-valid logical content is authoritative
regardless of whether its immutable ingress decision is admitted or rejected. A later attempt only
appends an exact replay or a conflict; it never replaces that content or re-runs the first ingress
decision. Exact identity/content replay is idempotent; identity with different provider-owned content conflicts.
UPDATE/DELETE are denied.

### 7.3 `selection_source_fact_attempts`

One row per `source_fact_attempt_id`, containing the complete acquired
`GlobalNewsRecord + BatchEvidence`, required `source_batch_attempt_id` foreign key, feed-attempt
evidence hash, zero-based `provider_ordinal`, acquisition batch/observation identity, ingestion
`stage_run_id`, attempted time and audit/content hashes. Same-run exact replay is idempotent; a
later batch/run appends a row even when logical content is unchanged. A conflicting attempt is
retained with a typed conflict result and cannot mutate the logical source fact.
`source_fact_attempt_id = sha256_json(SourceFactAttemptPreimage)`; the referenced logical
`source_fact_key` and provider-content hash use the corresponding §4.1 preimages.

The existing `selection_event_inbox.payload_json` is a lossy v1 `MarketEvent` payload and is not
upgraded in place. At cutover:

1. the monitor owner enters a quiesce barrier: stop v1 acquisition/evaluation tasks, wait for every
   in-flight v1 writer to finish, acquire the migration lock, record row counts and install INSERT,
   UPDATE and DELETE denial triggers on `selection_event_inbox`, `selection_event_completions`,
   `selection_runs`, `selection_candidates`, `selection_feature_snapshots` and
   `selection_visibility_receipts`; startup fails if any old writer remains enabled;
2. pending v1 rows are reported as `legacy_excluded`, never re-evaluated using current board
   membership or fabricated source facts;
3. one fresh source item after the cutover enters only the v2 source-fact inbox;
4. existing committed v1 admitted candidates may finish only their existing legacy T0/D1 settlement
   contract; they do not receive guessed D3/D5 or rejected cohorts;
5. the only post-cutover v1 write whitelist is the existing committed-candidate T0/D1 settlement
   transaction into legacy outcome tables. The permanent conditional INSERT guard described below
   is installed at cutover and is never replaced or redefined.

Under the same quiesce/migration/audit lock, the first config-activation envelope persists
`legacy_cutover_snapshot = LegacyCutoverSnapshotPreimage` and
`legacy_cutover_snapshot_hash = sha256_json(LegacyCutoverSnapshotPreimage)`. Its
`tables_sorted` contains, by table name, the exact row count and maximum SQLite rowid (0 when empty)
for `selection_event_inbox`, `selection_event_completions`, `selection_runs`,
`selection_candidates`, `selection_feature_snapshots`, `selection_visibility_receipts` and
`selection_outcomes`. `frozen_graph_trigger_set_hash` hashes the complete immutable trigger
registry as `sha256_json(LegacyTriggerSetPreimage)`: entries are sorted uniquely by trigger name
and contain the canonical trigger name, target table, operation and canonical SQL bytes. The
registry contains all INSERT/UPDATE/DELETE denial triggers on the six frozen graph tables,
permanent UPDATE/DELETE denial on `selection_outcomes`, and the one permanent conditional
`selection_outcomes` INSERT guard. No post-cutover DDL is allowed to add, remove or replace a
registered legacy trigger. Later config activations copy and verify the first receipted snapshot
bytes/hash; they never recalculate a new cutover. The verified config-activation receipt is
therefore the immutable carrier for report `legacy_excluded` counts, and startup rechecks exact
trigger registry membership and hash before enabling v2.

No v1 row is UPDATE-migrated, replayed into v2, or joined to current provider membership.
`selection_candidates` gains an immutable `sample_schema` discriminator solely to mark every
existing row `legacy-v1`; the migration uses the constant column default/table rebuild and does not
issue semantic per-row UPDATEs. Its constraint permits no `schema-v2` value. The physical graph
denial triggers above protect every acquisition edge, not only the candidate table. The legacy
`due_outcomes` query must filter `legacy-v1` and may only finish those pre-cutover candidates. The
new v2 scheduler reads only receipted `selection_samples` and never the projection table, so one
candidate cannot be settled by both writers.

The outcome whitelist is physical, not prose. At cutover, `selection_outcomes` receives permanent
UPDATE/DELETE denial triggers and an INSERT guard that requires:

```sql
NEW.phase IN ('t0_close', 'd1_settled')
AND EXISTS (
  SELECT 1
  FROM selection_candidates c
  JOIN selection_visibility_receipts v ON v.run_id = c.run_id
  WHERE c.candidate_id = NEW.candidate_id
    AND c.sample_schema = 'legacy-v1'
)
```

The existing unique `(candidate_id, phase)` and repository due-date validation remain mandatory.
`append_outcome` becomes a private method reachable only through
`LegacyV1OutcomeSettlementRepository`; its caller audit permits only the monitor's legacy drain
owner and tests. The drain state is derived, not mutated:

```text
Pending  := receipt-verified, as_of-independent terminal anti-join count > 0
Complete := receipt-verified, as_of-independent terminal anti-join count = 0
```

The anti-join ranges over the immutable cutover candidate graph and asks whether any committed
`legacy-v1` candidate lacks either `t0_close` or `d1_settled`; a current due set of zero is
insufficient. The graph is frozen, rows are append-only, phases are unique and the guard accepts
only a missing legacy phase, so `Pending -> Complete` is monotonic and `Complete -> Pending` is
impossible. The same conditional guard remains installed in both states: once `Complete`, every
possible INSERT fails its missing-phase/uniqueness predicate, which is an effective unconditional
denial without changing the hashed trigger set. Startup derives this state under a pinned SQLite
snapshot, verifies the immutable trigger registry, disables the legacy drain owner when
`Complete`, and fails if an old binary/public writer or missing/different guard is detected.

Repository construction requires explicit runtime `SelectionStoreMode::Production | Test`.
Production accepts canonical real A-share codes and rejects `TEST_CODE_`; Test accepts only
`TEST_CODE_` and rejects real symbols. This is not selected with `cfg(test)`: `monitor --test`
constructs Test mode with its physically isolated database/audit root, while normal/review/live
canary construct Production mode.

### 7.4 `selection_relation_attempts`

One row per `attempt_id`. Common fields include logical `relation_key`, relation kind, event source
fact identity, chain/config evidence, request hash, result stage/code, true upstream retryability,
available provider/batch evidence, failure detail hash, attempt content hash and generation
`generation_run_id`.

Variant constraints:

- `DirectMention`: exact mention kind and security-master evidence required; all board fields NULL.
- configured board attempt: binding object/audit hash required.
- resolved board attempt: strict constituent evidence and actual count required.
- rejected/unsupported attempt: only evidence available before the failed stage is populated;
  missing fields remain NULL.
- every provider board member, including one that later fails canonicalisation, stores the exact
  `raw_identity_json/raw_identity_hash` pair; direct mention stores the pair built from its
  source-bound raw instrument identity. A value rejected for control characters cannot be copied
  into canonical stock columns but remains JSON-escaped in this typed evidence.

Resolved/direct success requires canonical `available_evidence_json/hash` when provider evidence
exists and NULL error detail/fingerprint. Rejected/unsupported attempts use the exact partial
provider evidence pair or NULL and require canonical error detail/hash/fingerprint. The relation
receipt reparses raw identity/evidence/error JSON and recomputes all hashes before using a row in a
relation evidence set.

### 7.5 `selection_evaluation_attempts`

One row per `evaluation_attempt_id`, after canonical identity exists. It includes `sample_key`,
canonical code/name/market, relation evidence-set hash, market request hash, failed stage, result
code, true retryability, evidence available before failure, content hash and `generation_run_id`.

`relation_evidence_json` is the exact compact serialization of
`RelationEvidenceSetPreimage`; `relation_evidence_set_hash =
sha256_json(RelationEvidenceSetPreimage)`. Entries are direct-first by `relation_rank`, then
`relation_key` UTF-8 bytes, and duplicates fail. Every entry binds the already staged relation
attempt row content hash. An evaluation attempt and its matching terminal sample MUST store the
same set hash; the generation-receipt trigger recomputes it from the referenced relation attempts
and rejects any mismatch.

Missing market/feature evidence stays NULL. A successful complete attempt records the terminal
decision hash that is staged into `selection_samples`:
`terminal_decision_hash` MUST equal the exact matching
`SelectionSampleRowContentPreimage` hash stored as `selection_samples.content_hash`. Retryable and
evidence failures keep it NULL and never occupy the terminal sample identity. `stage_generation`
and the generation-receipt trigger require exactly one successful evaluation attempt for every
sample in the run and re-check this equality; an attempt pointing at a different/missing terminal
row aborts the stage/receipt.

A successful evaluation has canonical complete T0
`available_evidence_json/hash` and NULL error detail/fingerprint. A retryable or non-retryable
evidence failure stores the exact partial pair (or both NULL), canonical provider
`error_detail_json/error_detail_hash`, and
`error_fingerprint=sha256_json(ErrorFingerprintPreimage)`; it has no terminal decision hash. The
generation-receipt trigger replays this variant matrix and all three JSON parsers.

### 7.6 `selection_samples`

One row per `sample_key`, containing only `admitted | hard_rejected` terminal decisions. Common
required fields:

- `generation_run_id`, sample/logical identity and final decision;
- complete source-bound news record and batch evidence;
- chain/config/matched-keyword evidence;
- canonical security identity;
- `event-relation-v2` ordered relation evidence set and content hash;
- complete T0 feature payload and market provider/source/source_at/observed/batch/hash;
- admission version, immutable calendar version/hash plus the complete canonical
  `OutcomeTradingDateVectorPreimage` for `[T0,D1,D2,D3,D4,D5]`, its canonical JSON/hash and the six
  duplicated date columns;
- `rejection_count`, sample content hash and `staged_at`; authoritative committed chronology comes
  only from the joined generation receipt and is never UPDATE-filled into the append-only sample.

`t0_feature_json` is exactly `canonical_json(T0FeaturePreimage)` and its hash is the corresponding
`sha256_json`; feature version/window, every canonical decimal and the duplicated market-evidence
fields must agree. All required daily features are non-NULL. Intraday requires non-NULL
`intraday_volume_pace`; post-close permits it to be NULL. Feature missing/non-finite/mismatch is an
evaluation-attempt failure and cannot produce either terminal decision.

Relation CHECK matrix:

- a direct-only sample has direct evidence and NULL board evidence;
- a board sample references one committed resolved relation attempt and has binding/constituent
  evidence;
- a merged sample satisfies both and retains the direct-first ordered evidence set.

Decision CHECK matrix:

- `Admitted` has no rejection rows;
- `HardRejected` has at least one continuous-ordinal rejection row;
- both decisions have full T0-comparable feature/market evidence;
- no third/evidence-failure terminal decision exists.

Before hashing the sample, the owner computes every rejection row content hash in ordinal order.
`rejection_row_hashes_in_ordinal_order` is `[]` for `Admitted`; for `HardRejected` its length
equals `rejection_count` and element `n` is the content hash of rejection ordinal `n`. Thus the
sample hash commits the complete child list without a cycle: rejection rows reference
`sample_key`, not the later sample row content hash.

The terminal decision is computed in memory. One SQLite transaction stages the sample and all hard
rejection reasons; only after commit may the audit/receipt choreography publish it to authoritative
research queries. Identical identity/content is idempotent; different content conflicts. UPDATE and
DELETE are rejected.

SQLite cannot enforce a cross-table child count with a parent CHECK alone. The only write API is
`stage_generation()`: it inserts the parent with immutable `rejection_count`, inserts children under
a trigger that permits them only for `hard_rejected` and `ordinal < rejection_count`, then verifies
inside the same transaction that `COUNT(*) = rejection_count`, `MIN(ordinal)=0` and
`MAX(ordinal)=rejection_count-1` before commit. `admitted` requires `rejection_count=0`;
`hard_rejected` requires `rejection_count>0`. There is no public standalone rejection writer.

The concrete enforcement is:

1. `selection_samples` has `CHECK ((decision_kind='admitted' AND rejection_count=0) OR
   (decision_kind='hard_rejected' AND rejection_count>0))` and unique logical tuple;
2. `selection_rejections` has primary key `(sample_key, ordinal)`, deferred `ON DELETE RESTRICT`
   sample FK, `CHECK (ordinal>=0 AND retryable=0)`;
3. its INSERT trigger requires the existing parent to be `hard_rejected`, requires
   `NEW.ordinal=(SELECT COUNT(*) ...)`, and requires `NEW.ordinal < rejection_count`;
4. repository validation runs the exact count/min/max query before the stage transaction commits;
5. the generation-receipt INSERT trigger repeats the decision/count/min/max invariant over every
   sample in that run, recomputes the ordered rejection row hashes and aborts on any mismatch.

SQLite triggers can enforce envelope/manifest/row relationships but cannot authenticate the
external JSONL audit chain. Direct SQL could therefore fabricate a structurally valid receipt row;
the database row alone is never sufficient authority. Before any v2 authoritative query is
enabled at startup and for every receipt-bound read,
`VerifiedSelectionReadModel` acquires the audit lock, begins a SQLite read transaction, immediately
pins its snapshot by reading the receipt high-water rowid, then validates the complete
production/test JSONL chain and every receipt visible in that same snapshot against the referenced
Prepared/Committed records, manifest and staged hashes. The authoritative query executes through
the verified receipt-key set in that same transaction; only after the query result is materialized
does it end the SQLite transaction and release the audit lock. A concurrent direct-SQL receipt
insert is therefore absent from the pinned snapshot and must pass validation on the next read.
Any unknown, missing, duplicate or mismatched receipt disables the entire v2 authoritative read
model and returns an explicit integrity error; it does not skip one bad row. Repository methods
return receipted data only through this verified read model. Gate D also runs the same corruption
and receipt↔audit verification against a query-only database copy and requires zero mismatches.

### 7.7 `selection_rejections`

Hard-rejection reasons only:

- sample ID and continuous ordinal from zero;
- reason code, rule ID, retryable=false and structured detail;
- source/provider/source_at/observed/batch/hash when applicable;
- audit/content hashes and created time.

Evidence/source failures belong to attempt tables, not this table. A rejection may not exist without
its `HardRejected` sample. The ordered reason list is part of the sample content hash.
`reason_code` maps one-to-one to the same-named snake-case
`AdmissionStructuredDetailPreimage` variant and `rule_id` is the checked-in admission-v1 rule
registry entry for that variant. The JSON/hash pair is canonical as defined in §6.1; duplicated
decimal/source fields must agree, `retryable=false`, and an unknown reason/variant/rule tuple aborts
the stage and receipt.

### 7.8 `selection_sample_outcomes`

Phases:

- `t0_close`
- `d1_settled`
- `d3_settled`
- `d5_settled`

Each row includes:

- sample ID, phase and due trading date;
- open/high/low/close, volume and amount for the phase;
- return from T0 close;
- cumulative MFE/MAE through the phase;
- volume ratio against the persisted T0 baseline;
- provider/source/source_at/observed/batch/hash evidence;
- `outcome_run_id`, audit/content hashes and created time.

Only `Admitted` and `HardRejected` with complete T0-comparable evidence are settled. Each phase is
independently idempotent and retryable. A later phase never overwrites an earlier phase.

Outcome math is fixed and does not use pre-event T0 movement:

- `t0_close` stores the full-day raw OHLCV/amount, but `return_from_t0_close=0`,
  `cumulative_mfe=0`, `cumulative_mae=0` and `volume_ratio=1`;
- every return/MFE/MAE value is a dimensionless ratio rather than percentage points:
  `return_from_t0_close = phase_close / t0_close - 1`,
  `cumulative_mfe = max(D1..phase high) / t0_close - 1`, and
  `cumulative_mae = min(D1..phase low) / t0_close - 1`; for example ten percent is canonical
  decimal `0.1`, never `10` or `"10%"`;
- `d1_settled` return uses D1 close versus T0 close, MFE/MAE use D1 high/low versus T0 close;
- `d3_settled` and `d5_settled` use complete D1..D3/D5 daily windows, close return at the phase date
  versus T0 close, and cumulative max high/min low versus T0 close;
- phase `volume_ratio = phase_day_volume / persisted_t0_full_day_volume`; non-positive or
  non-finite baseline/phase volume is invalid evidence.

All prices, volume, amount, ratios and derived values are serialized with the existing
`canonical_f64` contract (`finite`, no negative zero, Rust `f64::to_string()` shortest round-trip
form) before entering a preimage. A mathematically zero derived value is normalized to positive
`0` before serialization. Re-parsing a stored decimal and serializing it again must reproduce the
same bytes; alternate spellings such as `0.10`, exponent aliases, `-0`, `NaN` or infinities fail
stage validation.

T0 full-day high/low is never treated as post-event excursion. A future intraday post-event T0
metric requires a separate event-time 5-minute-window Gate A and cannot be inferred from daily bars.

### 7.9 `selection_outcome_attempts`

One row per settlement request attempt:

```text
outcome_attempt_id = sha256_json(OutcomeAttemptPreimage)
result = settled | expected_wait | error
```

The row retains sample/phase/stored due date, optional request hash plus its optional typed request
evidence pair, result/reason, true retryability, available provider evidence, local attempted time,
`outcome_run_id` and content hash. Local
attempted time never fills provider time. The same run/content replay is idempotent; a later
scheduler run
appends a new attempt even with the same wait/error reason.

`settled` requires complete outcome evidence and is atomically staged with exactly one matching
`selection_sample_outcomes` row. Its `settled_outcome_content_hash` MUST equal that matching row's
`SelectionSampleOutcomeRowContentPreimage` hash stored as
`selection_sample_outcomes.content_hash`. `stage_outcome()` and the outcome-receipt trigger
recompute and require the same equality. `expected_wait` and `error` keep
`settled_outcome_content_hash` NULL and never write placeholder outcomes. Reports count only
received outcome attempts whose run has a matching commit receipt.

`settled` requires canonical complete outcome `available_evidence_json/hash` and NULL
error-detail/fingerprint. It also requires `reason_code=NULL` and `retryable=NULL`.
`expected_wait` has no provider call/evidence/error detail and therefore `request_hash`,
`request_evidence_json`, `request_evidence_hash` and all provider/error fields are NULL;
it requires the sole checked-in `OutcomeWaitReasonCodeV1` token `market_session_unsettled` and
`retryable=NULL`. It does not add a mutable retry-time column. The read model deterministically
derives `next_eligible_at` as the stored due date at
`15:00:00.000000001 Asia/Shanghai` (canonical RFC3339 nanoseconds with `+08:00`) and suppresses the
same subject while scheduler `as_of_instant < next_eligible_at`. Therefore at most one receipted
pre-close wait exists for one sample/phase/due date; repeated ticks before the threshold emit no
new claim/run/receipt. The status itself, not an error retryability flag, keeps the subject
eligible after that deterministic instant. `settled_bar_missing` is a checked-in provider/data
error reason, never an
ExpectedWait: it requires the real partial evidence available from the read and a non-NULL
retryability classification. Every `error` requires a checked-in provider/evidence `reason_code`
and non-NULL `retryable`, carries its exact partial provider evidence pair (or both NULL),
canonical `error_detail_json/error_detail_hash`, and recomputable `error_fingerprint`. These
required/NULL rules are enforced in the attempt CHECK, stage validator and
outcome-receipt trigger; no other field combination is valid.

### 7.10 `selection_v2_recovery_envelopes`

One immutable envelope is durably committed with `synchronous=FULL` before any Prepared audit
append. It contains:

- UUIDv7 `stage_run_id`, run kind and `logical_subject_key`;
- exact payload schema
  `config-activation-stage-v1 | source-ingress-stage-v2 | generation-stage-v3 |
  outcome-claim-stage-v2 | outcome-stage-v3`;
- the complete canonical compact JSON of the typed in-memory stage input, including all provider
  records/evidence, config snapshot, attempts, decisions, rejection reasons and outcome data needed
  to reproduce domain rows without network re-fetch;
- payload JSON hash, `in_memory_payload_hash`, config activation/hash where applicable,
  `enveloped_at` and envelope row content hash.

The envelope binding matrix is exact: every kind requires non-empty
`config_activation_run_id` and `config_hash`. For `config_activation`,
`config_activation_run_id == stage_run_id`; for ingress/generation/outcome it equals the already
receipted activation carried by the source/sample lineage. No envelope kind permits either value
to be NULL. The payload's corresponding fields must equal the row fields.

The five stage-input preimages and their exact field order are:

```rust
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCutoverTableWatermarkPreimage<'a> {
    table_name: &'a str,
    max_rowid: i64,
    row_count: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTriggerDefinitionPreimage<'a> {
    trigger_name: &'a str,
    target_table: &'a str,
    operation: &'a str, // "insert" | "update" | "delete"
    canonical_sql: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyTriggerSetPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.legacy_trigger_set.v1"
    triggers_sorted: &'a [LegacyTriggerDefinitionPreimage<'a>],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCutoverSnapshotPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.legacy_cutover_snapshot.v1"
    captured_at_rfc3339_nanos_utc: &'a str,
    tables_sorted: &'a [LegacyCutoverTableWatermarkPreimage<'a>],
    pending_inbox_count: u64,
    committed_legacy_candidate_count: u64,
    legacy_outcome_row_count: u64,
    frozen_graph_trigger_set_hash: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigActivationStageInputPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.config_activation_stage.v1"
    stage_run_id: &'a str,
    logical_subject_key: &'a str,
    config_snapshot: &'a SelectionConfigSnapshotPreimage<'a>,
    config_snapshot_json_hash: &'a str,
    config_hash: &'a str,
    activation: &'a ConfigActivationContentPreimage<'a>,
    activation_content_hash: &'a str,
    legacy_cutover_snapshot: &'a LegacyCutoverSnapshotPreimage<'a>,
    legacy_cutover_snapshot_hash: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceIngressStageInputPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.source_ingress_stage.v2"
    stage_run_id: &'a str,
    logical_subject_key: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    generation_market_date: &'a str,
    aggregator_observed_at_rfc3339_nanos_utc: &'a str,
    source_batch_content_hash: &'a str,
    registered_feed_snapshot_json: &'a str,
    registered_feed_snapshot_hash: &'a str,
    source_batch_attempt_rows:
        &'a [SelectionSourceBatchAttemptRowContentPreimage<'a>],
    source_fact_rows: &'a [SelectionSourceFactRowContentPreimage<'a>],
    source_fact_attempt_rows:
        &'a [SelectionSourceFactAttemptRowContentPreimage<'a>],
    planned_run_status: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationStageInputPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.generation_stage.v3"
    stage_run_id: &'a str,
    logical_subject_key: &'a str,
    source_fact_key: &'a str,
    source_fact_content_hash: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    generation_market_date: &'a str,
    relation_attempt_rows: &'a [SelectionRelationAttemptRowContentPreimage<'a>],
    evaluation_attempt_rows:
        &'a [SelectionEvaluationAttemptRowContentPreimage<'a>],
    sample_rows: &'a [SelectionSampleRowContentPreimage<'a>],
    rejection_rows: &'a [SelectionRejectionRowContentPreimage<'a>],
    planned_run_status: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedOutcomeDueDatabaseObjectBindingPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.outcome_due_database_object.v1"
    manifest_root_canonical_path: &'a str,
    manifest_root_device: u64,
    manifest_root_inode: u64,
    manifest_root_mode: u32,
    database_relative_path: &'a str,
    database_device: u64,
    database_inode: u64,
    database_mode: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedOutcomeDueDatabaseBindingPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.outcome_due_database_binding.v1"
    scope: &'a str, // exactly "production" or "test"
    object_binding: &'a VerifiedOutcomeDueDatabaseObjectBindingPreimage<'a>,
    object_binding_hash: &'a str,
    database_relative_path: &'a str,
    sqlite_application_id: u32,
    sqlite_user_version: u32,
    sqlite_schema_hash: &'a str,
    receipt_snapshot_high_water_rowid: i64,
    receipt_snapshot_high_water_content_hash: Option<&'a str>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedOutcomeAuditPrefixPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.selection_audit_prefix.v1"
    record_hashes_in_file_order: &'a [&'a str],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedOutcomeReceiptTuplePreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.outcome_due_receipt_tuple.v1"
    receipt_role: &'a str,
    outcome_phase: Option<&'a str>,
    subject_kind: &'a str,
    subject_id: &'a str,
    logical_subject_key: &'a str,
    run_status: &'a str,
    committed_at_rfc3339_nanos_utc: &'a str,
    receipt_content_hash: &'a str,
    run_manifest_content_hash: &'a str,
    committed_audit_record_hash: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedOutcomeDueSnapshotPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.verified_outcome_due_snapshot.v1"
    database_binding: &'a VerifiedOutcomeDueDatabaseBindingPreimage<'a>,
    database_binding_hash: &'a str,
    selection_audit_high_water_record_ordinal: u64,
    selection_audit_high_water_record_hash: &'a str,
    selection_audit_prefix_hash: &'a str,
    receipt_tuples_sorted: &'a [VerifiedOutcomeReceiptTuplePreimage<'a>],
    sample_key_preimage: &'a SampleKeyPreimage<'a>,
    sample_key: &'a str,
    logical_subject_key: &'a str,
    canonical_stock_code: &'a str,
    canonical_market: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    outcome_phase: &'a str,
    stored_due_date: &'a str,
    calendar_version: &'a str,
    calendar_hash: &'a str,
    trading_date_vector: &'a OutcomeTradingDateVectorPreimage<'a>,
    trading_date_vector_hash: &'a str,
    applicable_trading_dates: &'a [&'a str],
    expected_provider_bar_count: u32,
    provider_request_hash: &'a str,
    t0_outcome_content_hash: Option<&'a str>,
    t0_close: Option<&'a str>,
    t0_volume: Option<&'a str>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeClaimDueBindingPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br178.outcome_claim_due_binding.v1"
    verified_due_snapshot: &'a VerifiedOutcomeDueSnapshotPreimage<'a>,
    verified_due_snapshot_hash: &'a str,
    same_subject_high_water_receipt_hash: Option<&'a str>,
    outcome_attempt_ordinal: u32,
    previous_same_subject_attempt_receipt_hashes: &'a [&'a str],
    selection_audit_high_water_record_hash: &'a str,
    sample_key_preimage: &'a SampleKeyPreimage<'a>,
    sample_key: &'a str,
    canonical_stock_code: &'a str,
    canonical_market: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    config_activation_receipt_hash: &'a str,
    source_ingress_run_id: &'a str,
    source_ingress_receipt_hash: &'a str,
    generation_run_id: &'a str,
    generation_receipt_hash: &'a str,
    outcome_phase: &'a str,
    t0_market_date: &'a str,
    stored_due_date: &'a str,
    calendar_version: &'a str,
    calendar_hash: &'a str,
    trading_date_vector: &'a OutcomeTradingDateVectorPreimage<'a>,
    trading_date_vector_hash: &'a str,
    applicable_trading_dates: &'a [&'a str],
    expected_provider_bar_count: u32,
    preceding_outcome_receipt_hashes: &'a [&'a str],
    t0_outcome_content_hash: Option<&'a str>,
    t0_close: Option<&'a str>,
    t0_volume: Option<&'a str>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeClaimStageInputPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.outcome_claim_stage.v2"
    stage_run_id: &'a str, // claim_id
    logical_subject_key: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    planned_outcome_run_id: &'a str,
    due_binding: &'a OutcomeClaimDueBindingPreimage<'a>,
    due_binding_hash: &'a str,
    provider_request_evidence: &'a RequestEvidencePreimage<'a>,
    provider_request_hash: &'a str,
    claim_lock_key: &'a str,
    planned_run_status: &'a str, // exactly "claimed"
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeStageInputPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br174.outcome_stage.v3"
    stage_run_id: &'a str,
    logical_subject_key: &'a str,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    outcome_claim_id: &'a str,
    outcome_claim_receipt_content_hash: &'a str,
    outcome_claim_due_binding_hash: &'a str,
    outcome_claim_provider_request_hash: &'a str,
    sample_key_preimage: &'a SampleKeyPreimage<'a>,
    sample_key: &'a str,
    outcome_phase: &'a str,
    stored_due_date: &'a str,
    outcome_attempt_rows: &'a [SelectionOutcomeAttemptRowContentPreimage<'a>],
    outcome_rows: &'a [SelectionSampleOutcomeRowContentPreimage<'a>],
    planned_run_status: &'a str,
}
```

Every type reachable from those five stage inputs, including each row-content, config, activation,
sample-key and rejection-detail type, also derives strict `Serialize + Deserialize` with
`deny_unknown_fields`; maps, flattening and untagged catch-all variants are prohibited. Gate B may
implement field-for-field owned recovery DTO mirrors (`String`, `Vec<T>`) instead of borrowed
deserialization, but a compile-time field-order/schema test must prove exact parity with these
preimages. Restart selects the type only from the exact `payload_schema`, parses to EOF, validates
the exact domain token and enum/NULL matrix, serializes the typed value back to canonical compact
JSON, and requires those bytes and their SHA-256 to equal `payload_json` and
`payload_json_hash`. Parsing into `serde_json::Value`, ignoring unknown/duplicate fields, accepting
trailing input or hashing the original bytes without typed canonical reserialization is forbidden.

`OutcomeClaimStageInputPreimage` is the durable acquisition intent, not provider evidence and not a
settled outcome. `VerifiedSelectionReadModel` constructs its opaque `VerifiedOutcomeDue` from one
pinned, fully audit-verified read snapshot and freezes the complete
`VerifiedOutcomeDueSnapshotPreimage`; `verified_due_snapshot_hash` is exactly
`sha256_json(VerifiedOutcomeDueSnapshotPreimage)`. The production database binding is only the
fixed `SELECTION_RELEASE_DATABASE_RELATIVE_PATH = "data/stock_analysis.db"` beneath the pinned
manifest root. Test scope uses its TEST_CODE-only database capability and canonical relative path.
`object_binding_hash` is exactly
`sha256_json(VerifiedOutcomeDueDatabaseObjectBindingPreimage)` constructed from no-follow pinned
directory/database descriptors; the nested and outer relative paths must match byte-for-byte and
both objects must retain the recorded regular/directory type and identity during the read.
`database_binding_hash` is exactly
`sha256_json(VerifiedOutcomeDueDatabaseBindingPreimage)`. A zero receipt high-water uses rowid `0`
and NULL content hash; a due snapshot necessarily has source/generation receipts and therefore
rejects that empty matrix. The schema hash is over the exact ordered
`sqlite_schema(type,name,tbl_name,sql)` rows for the twelve v2 objects. No caller/CWD/environment
database path, display filename or current connection can be substituted.

The selected audit high-water is the last record in the complete validated selection-audit prefix
visible to that read. `selection_audit_high_water_record_ordinal` is its zero-based file ordinal,
`selection_audit_high_water_record_hash` is that record's canonical record hash, and
`selection_audit_prefix_hash` is
`sha256_json(VerifiedOutcomeAuditPrefixPreimage { record_hashes_in_file_order:
ordered_record_hashes[0..=ordinal] })`.
Every receipt tuple's committed audit hash must occur exactly once at or before that high-water.
The exact relevant receipt set contains one config-activation receipt, one source-ingress receipt,
one generation receipt, the phase-required preceding outcome receipts and every prior closed
attempt receipt for the same outcome logical subject. Each element is the complete
`VerifiedOutcomeReceiptTuplePreimage` copied from a verified receipt+manifest join. Tuples are
sorted by the closed key
`(receipt_role_ordinal, outcome_phase_ordinal_or_255, committed_at RFC3339-nanos UTC bytes,
canonical UUIDv7 subject_id bytes, receipt_content_hash bytes)` where role ordinals are
`config_activation=0, source_ingress=1, generation=2, preceding_outcome=3,
same_subject_attempt=4`, and phase ordinals are `t0_close=0, d1_settled=1, d3_settled=2,
d5_settled=3`. Duplicate sort keys, duplicate receipt hashes, an extra role, a missing required
tuple or a tuple outside the selected sample/config/logical subject fails the whole read model.
Activation/ingress/generation tuples require `outcome_phase=NULL`; preceding-outcome tuples require
their actual earlier phase; same-subject-attempt tuples require the snapshot's current phase.
`OutcomeClaimDueBindingPreimage.selection_audit_high_water_record_hash` must equal the nested
snapshot value, and every duplicated sample/config/phase/calendar/request field must equal its
nested snapshot projection byte-for-byte.

The claim owner then closes that read transaction, acquires the fixed per-subject claim lock,
reacquires the selection-audit lock and a fresh SQLite revalidation transaction in the order
specified by §7.13.1, and recomputes every due-snapshot and
`OutcomeClaimDueBindingPreimage` field. The revalidation transaction must prove that the original
database identity/schema binding and frozen receipt/audit high-water prefix remain intact, then
query current state and require the exact same relevant receipt tuples. A later unrelated append
may extend the current database/audit high-water but must have the frozen prefix as an exact prefix;
it does not rewrite the stored due snapshot. The transaction is explicitly committed/closed
without writes before generic persistence begins; §7.13's envelope, domain+manifest and receipt
writes then use three separate SQLite `synchronous=FULL` transactions while the selection-audit
lock remains held. The verified snapshot hash binds the exact relevant receipt set, while
`same_subject_high_water_receipt_hash` binds the latest receipted outcome attempt for this logical
subject or JSON `null` when none exists. A receipt advance, active/partial claim, different
preceding-phase set or changed sample/config lineage makes the old due capability unusable.
Unrelated subjects advancing the global database do not invalidate it; they may advance the
physical database high-water but cannot change the due snapshot's relevant tuple set.

The due binding's phase matrix is exact. T0 requires zero preceding outcome receipt hashes and all
three T0 baseline fields are NULL. D1/D3/D5 require, respectively, the ordered receipted hashes for
`[T0]`, `[T0,D1]` and `[T0,D1,D3]`, plus the exact receipted T0 outcome content hash and canonical
positive close/volume decimals. The canonical `OutcomeTradingDateVectorPreimage` always contains
all six immutable sample dates `[T0,D1,D2,D3,D4,D5]`; its hash is recomputed at sample stage,
verified due read, claim revalidation and outcome receipt. `applicable_trading_dates` is exactly
the byte-identical vector prefix of length `1/2/4/6` for `T0/D1/D3/D5`; `stored_due_date` is its
last element, `t0_market_date` is its first, and `expected_provider_bar_count` equals its length.
The due snapshot, due binding and typed Magic-TDX request must carry the same complete vector,
vector hash and prefix bytes. `claim_lock_key` and the fixed lock filename are exactly the
recomputed `logical_subject_key`; a caller-supplied alternate lock key is rejected.

The typed Magic-TDX request is built before any provider call and its canonical request hash is
retained in both due snapshot and claim. The Gateway's outcome-specific available-evidence
preimage must echo that request hash, vector hash and expected prefix, and retains the provider's
actual returned dates in provider order. Admission requires
`returned_trading_dates == applicable_trading_dates` element-for-element and byte-for-byte; a
missing, extra, duplicate, reordered or alternate-form date is explicit partial/invalid evidence,
never a settled capability. Partial errors retain the actual returned date list rather than
rewriting it to the request.

`previous_same_subject_attempt_receipt_hashes` contains exactly the
`CommitReceiptContentPreimage` `content_hash` of every prior **closed outcome-run receipt** whose
logical subject equals this exact sample/phase/config/stored-due-date subject. It excludes the
claim receipt, activation/ingress/generation receipts and preceding-phase settlement receipts.
Elements follow their corresponding `same_subject_attempt` tuples ordered by
`(committed_at RFC3339-nanos UTC bytes ASC, canonical UUIDv7 subject_id bytes ASC,
receipt_content_hash bytes ASC)`. `same_subject_high_water_receipt_hash` is the final element or
NULL when the list is empty, and `outcome_attempt_ordinal` is exactly `len + 1` in canonical
unsigned base-10 form. No gap, duplicate, reorder, omitted prior receipt or ordinal overflow is
accepted. A closed ExpectedWait/failed-retryable attempt therefore advances the next claim's
lineage; recovery of an existing claim retains its original ordinal and history. Claim recovery
must reproduce the same bytes, due snapshot/binding hashes, claim UUIDv7 and preallocated
outcome-run UUIDv7; current time may determine only the later market-session result and actual
attempt timestamp, never a replacement request or identity.

The outcome stage is self-contained recovery evidence, but it does not make its duplicated sample
identity authoritative. Before constructing the stage, the repository loads the receipted
`selection_samples` row and reconstructs the exact `SampleKeyPreimage`; it then requires
`sha256_json(sample_key_preimage) == sample_key`, copies that sample's
`config_activation_run_id/config_hash`, and checks the phase due date against the sample's immutable
calendar column. At stage write, staged read-back, and immediately before receipt insertion, the
repository re-queries the same sample and requires all preimage fields, config lineage and due date
to remain equal. Missing/unreceipted sample lineage is an explicit failure. The envelope-retained
preimage permits crash recovery without inventing fields, while the authoritative join prevents a
self-consistent forged envelope from substituting another sample identity.

Every outcome stage must additionally load the exact receipted claim, require
`stage_run_id == claim.planned_outcome_run_id`, and require its logical subject, due-binding hash
and provider-request hash to match byte-for-byte. For `settled` or provider `error`, the attempt's
typed request pair must equal the claim request. For `expected_wait`, the attempt request/provider
fields remain NULL because no call happened, but the enclosing outcome stage still references the
claim's precomputed request identity. A claim may close only once and only through the exact
matching receipted outcome stage.

Every row slice is sorted by that table's logical primary key before serialization. The activation
and outcome-claim variants require zero domain rows; ingress permits only its three listed row
types, generation only its four, and outcome only its two. Outcome additionally requires
`outcome_attempt_rows.len() == 1`; that attempt must match the envelope
`(stage_run_id, sample_key, outcome_phase)`. `outcome_rows.len() == 1` only for `settled`, and is
zero for every other allowed outcome status. `planned_run_status` must satisfy §7.11's
subject-kind matrix and must equal the later manifest status. Exact compact JSON bytes for all five
structs, including empty slices and all optional row fields as JSON `null`, are golden vectors.

For the selected run kind, `payload_json = canonical_json(<Kind>StageInputPreimage)` and
`payload_json_hash = hex(sha256(payload_json UTF-8 bytes))`.
`in_memory_payload_hash` is a different value defined by `RunPayloadPreimage` in §7.11; neither
hash aliases the other. The recovery-envelope row stores both, and
`recovery_envelope_content_hash =
sha256_json(SelectionRecoveryEnvelopeRowContentPreimage)`.

The complete compact stage JSON and its exact SHA-256 are stored in the envelope. Unknown fields,
non-canonical JSON, slice-order/row-count/hash mismatch or a different payload for an existing run
ID fail. The envelope is recovery evidence, not an authoritative source/sample/outcome row; every
production consumer still requires a commit receipt. It is append-only, retained with the audit,
and included by hash in the run manifest/receipt.

Crash handling is now closed:

- envelope commit fails: no Prepared record exists and the run was never accepted;
- crash after envelope but before Prepared: recovery validates the envelope and resumes the same
  run without re-fetching;
- crash after Prepared but before domain stage: recovery reconstructs exact domain rows from the
  envelope;
- crash after stage/Committed: recovery re-hashes the envelope and rows before continuing.
- a partial outcome-claim envelope/Prepared/manifest/Committed state is recovered to the exact
  claim receipt before market-session evaluation or provider work; a receipted claim with no
  outcome artifacts resumes its exact acquisition intent, while a claim with an outcome envelope
  resumes the outcome stage without provider re-fetch.

### 7.11 `selection_v2_run_stages`

One append-only manifest row exists for every config activation, ingress, generation, outcome claim
or outcome run, including VerifiedEmpty/no-relation/zero-sample runs. It stores `subject_kind`,
`subject_id`,
`in_memory_payload_hash`, prepared audit record hash, expected staged-row count,
`staged_db_content_hash`, recovery-envelope hash, manifest content hash and `staged_at`. Activation
manifests additionally store config/snapshot/activation hashes, artifact validity and executable
revision; generation manifests store the immutable source-fact/config/date identity; outcome-claim
manifests store the full due/request binding and planned outcome-run identity; outcome manifests
store sample/phase/due plus exact claim-receipt identity. Every domain row in that transaction has the same run
ID. The manifest is inserted atomically with those rows and provides the recovery anchor when a run
has zero domain rows. Its terminal run status is one of:

- `completed`: every configured relation identity reached resolved/non-retryable and all eligible
  terminal samples were staged;
- `activated`: the exact config snapshot passed validation and may be referenced by later ingress;
- `verified_no_relation`: the immutable config snapshot matched no chain and therefore produced no
  formal relation;
- `pending_dependency`: at least one relation/evaluation attempt is retryable; no terminal sample
  for its incomplete `(event, chain)` is staged;
- `failed_non_retryable`: no sample was fabricated and all failure attempts are queryable.
- `settled`: an outcome run atomically staged one complete outcome with its settled attempt;
- `expected_wait`: an outcome run recorded that the stored due phase is not yet complete;
- `failed_retryable`: an outcome run recorded an explicit retryable provider/evidence failure.
- `claimed`: an outcome-claim run durably reserved one exact verified due/request binding and one
  preallocated outcome-run UUIDv7; it does not assert that a provider call or settlement occurred.

The subject-kind matrix is strict: config activation permits only `activated`; ingress permits
`completed | failed_non_retryable`; generation permits
`completed | verified_no_relation | pending_dependency | failed_non_retryable`; outcome claim
permits only `claimed`; outcome permits
`settled | expected_wait | failed_retryable | failed_non_retryable`. A status outside its row kind
fails the manifest CHECK and golden vector. Outcome attempt result maps one-to-one:
`settled -> settled`, `expected_wait -> expected_wait`, retryable error ->
`failed_retryable`, non-retryable error -> `failed_non_retryable`. No free-form `error` status
exists. Every outcome-claim manifest has `expected_staged_row_count=1` because it stages only the
recovery envelope; its receipt trigger revalidates the complete due/request binding and planned
outcome-run UUIDv7. For every outcome manifest, `expected_staged_row_count` equals the recovery envelope,
exactly one outcome-attempt row, and the conditional outcome row: `3` for `settled`, otherwise
`2`. The manifest/receipt validators require exactly one attempt with the run/sample/phase identity;
for `settled` they require exactly one outcome with the same sample/phase/run and require
`attempt.settled_outcome_content_hash == outcome.content_hash`, where `outcome.content_hash` is
`sha256_json(SelectionSampleOutcomeRowContentPreimage)`; for `expected_wait`,
`failed_retryable` and `failed_non_retryable` they require zero outcome rows and NULL
`settled_outcome_content_hash`. The error attempt's persisted `retryable` flag must be `true` for
`failed_retryable` and `false` for `failed_non_retryable`. SQLite has no deferred triggers:
domain/envelope rows may be inserted in any order first, but the manifest MUST be inserted last in
that transaction and its immediate INSERT trigger validates the complete matrix. A direct writer
that inserts the manifest before its complete domain rows fails immediately. The receipt is
inserted last in the later audit-commit transaction, and its immediate INSERT trigger independently
revalidates the same complete matrix.

A partial unique index on `selection_v2_run_stages(config_hash)` where
`subject_kind='config_activation' AND run_status='activated'` permits exactly one staged activation
per config hash. An unreceipted match is recovered; a receipted exact match is reused; content
conflict is fatal.

The manifest, not a fake `RelationAttemptKind`, is the durable representation of no-chain and
zero-candidate completion. “Latest receipted manifest” everywhere in BR-176/BR-178 means the single
generation/outcome-run row returned after joining receipt to manifest for the same
`logical_subject_key`, ordered by
`receipt.committed_at DESC, receipt.subject_id DESC` and `LIMIT 1`; `subject_id` is compared as its
validated lowercase canonical UUIDv7 UTF-8 bytes. A shared repository helper owns this query, and
same-committed-at golden/integration tests require the greater subject ID to win. Pending-fact
scheduling uses that helper for the same source fact/config logical subject; a
completed/no-relation/failed-non-retryable run is not repeated. A new config snapshot is a new
prospective subject and never backfills old facts.

Each run allocates a UUIDv7 `stage_run_id` before acquisition. It is chronology/attempt identity,
not provider time. `logical_subject_key` is separately
`sha256_json(RunLogicalSubjectPreimage)`:

```rust
#[derive(Serialize)]
struct RunLogicalSubjectPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.run_logical_subject.v1"
    subject_kind: &'a str,
    source_fact_key: Option<&'a str>,
    config_hash: Option<&'a str>,
    sample_key: Option<&'a str>,
    outcome_phase: Option<&'a str>,
    stored_due_date: Option<&'a str>,
    ingress_source_batch_hash: Option<&'a str>,
}
```

Its required/NULL matrix is exact:

| `subject_kind` | `source_fact_key` | `config_hash` | `sample_key` | `outcome_phase` | `stored_due_date` | `ingress_source_batch_hash` |
|---|---|---|---|---|---|---|
| `config_activation` | NULL | required | NULL | NULL | NULL | NULL |
| `ingress_run` | NULL | required | NULL | NULL | NULL | required |
| `generation_run` | required | required | NULL | NULL | NULL | NULL |
| `outcome_subject` (for `outcome_claim` / `outcome_run` manifests) | NULL | required | required | required | required | NULL |

The validator never accepts a merely well-shaped 64-hex logical key. For generation it constructs
the preimage from the stage's authoritative `source_fact_key/config_hash`; for outcome claim and
outcome run it uses the
stage's `config_hash/sample_key/outcome_phase/stored_due_date` only after the authoritative sample
join described in §7.10 succeeds. It serializes the exact preimage, hashes it, and compares the
result with `logical_subject_key`. A shared validation module owns this operation and is invoked
by envelope creation, staged database read-back, and the receipt path. The receipt INSERT trigger
then binds the already revalidated envelope/manifest/sample projections in the same transaction;
it does not substitute SQLite string-shape checks for the Rust canonical hash recomputation.

For ingress, `ingress_source_batch_hash` is exactly
`sha256_json(SourceBatchContentPreimage)` from §7.1. That preimage is independent of all local run
and attempt IDs, so two concurrent processes that observed the exact same acquisition cannot obtain
different logical keys merely by allocating different UUIDv7 values. For outcome claim and
outcome run,
`outcome_phase/stored_due_date` must equal the immutable sample calendar schedule. Config
activation's logical key is therefore the config hash; generation's is source fact + config;
the claim and settlement run share the one sample + phase + stored due date + config logical key.
For both manifest kinds the serialized `RunLogicalSubjectPreimage.subject_kind` token is exactly
`outcome_subject`, so a claim cannot reserve a namespace different from the outcome it must close;
the manifest's own `subject_kind` remains `outcome_claim` or `outcome_run`. The recovery-envelope writer, manifest
CHECK, commit-receipt trigger and `VerifiedSelectionReadModel` all reconstruct this preimage from
the authoritative referenced rows and reject an extra/missing field or mismatch. Every one of the
five manifest variants and four logical-subject variants has exact JSON/hex golden vectors.

Audit/receipt `subject_id` is the `stage_run_id`; the manifest also stores
`logical_subject_key`. Under the global OS lock, immediately before Prepared append, the repository
re-queries that logical key. An already receipted `completed`/`verified_no_relation` generation or
settled outcome is an idempotent no-op; an unreceipted matching stage enters recovery; only a
receipted `pending_dependency`, `expected_wait` or `failed_retryable` may be followed by a new run
ID while its BR-177/schedule boundary permits. Outcome is stricter: a provider read cannot begin
until the exact outcome claim is receipted and its subject lock remains held. Two processes that
read the same due snapshot may race only to the non-blocking subject-lock attempt; at most one can
create or recover the claim, and the loser performs no provider read. A due capability whose
same-subject high-water changed while it waited fails as superseded rather than creating another
claim.

The global BR-176 scheduler preserves the existing `DEFAULT_PENDING_LIMIT=200`. On each selection
or settlement tick it first drains across all five run kinds two disjoint generic recovery queues:
envelope-only runs having no manifest ordered by
`enveloped_at, stage_run_id`, then manifested but unreceipted runs ordered by
`staged_at, stage_run_id`. A `NOT EXISTS` manifest predicate places each run in exactly one queue;
recovery failure stops the tick before new provider work. Outcome recovery additionally classifies
each not-yet-closed claim into exactly one of `claim_partial` (no claim receipt),
`outcome_recovery` (matching outcome envelope/manifest/Committed exists without outcome receipt),
or `claim_active` (claim receipt exists with no outcome artifacts). Claims are visited by
`claim_enveloped_at, claim_id` ascending; within one claim, partial claim recovery precedes
outcome recovery, which precedes any provider replay. A lock-busy claim is live-owned and therefore
not recoverable by this process; it is skipped without timeout while other subjects may proceed.
It then reads at most 200 receipted
ingress-admitted facts ordered by first ingress receipt `committed_at, source_fact_key`. It excludes
logical subjects already receipted as completed/no-relation/failed-non-retryable for their immutable
config hash and permits at most one new run per logical subject per tick. Reaching the limit leaves
later facts pending; it never marks them empty or dropped. Retryable dependency manifests remain
eligible on a later tick with a new run ID only while BR-177's stored prospective generation date
is still active; the first tick after that date writes the no-provider-call terminal boundary
required by BR-177. There is no retry-count terminalization or silent expiry.

Run hashes use these exact fixed-order types:

```rust
#[derive(Serialize)]
struct RunRowLogicalPrimaryKeyPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.run_row_logical_pk.v1"
    table_ordinal: u8,
    key_parts: &'a [String],
}

#[derive(Serialize)]
struct RunRowHashPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.run_row.v1"
    table_ordinal: u8,
    table_name: &'a str,
    logical_primary_key: &'a str,
    row_content_hash: &'a str,
}

#[derive(Serialize)]
struct RunPayloadPreimage<'a> {
    domain: &'static str, // config-activation/ingress/generation/outcome domain listed below
    subject_kind: &'a str,
    subject_id: &'a str,
    logical_subject_key: &'a str,
    source_fact_key: Option<&'a str>,
    config_activation_run_id: &'a str,
    config_hash: &'a str,
    config_snapshot_json_hash: Option<&'a str>,
    config_activation_content_hash: Option<&'a str>,
    config_activation_file_content_hash: Option<&'a str>,
    config_effective_from_rfc3339_nanos_utc: Option<&'a str>,
    artifact_valid_from: Option<&'a str>,
    artifact_expires_at: Option<&'a str>,
    executable_revision: Option<&'a str>,
    legacy_cutover_snapshot_hash: Option<&'a str>,
    generation_market_date: Option<&'a str>,
    aggregator_observed_at_rfc3339_nanos_utc: Option<&'a str>,
    ingress_source_batch_content_hash: Option<&'a str>,
    outcome_phase: Option<&'a str>,
    stored_due_date: Option<&'a str>,
    outcome_claim_id: Option<&'a str>,
    planned_outcome_run_id: Option<&'a str>,
    outcome_claim_receipt_content_hash: Option<&'a str>,
    outcome_claim_due_binding_hash: Option<&'a str>,
    outcome_claim_provider_request_hash: Option<&'a str>,
    rows: &'a [String], // RunRowHashPreimage hashes in canonical order
}

#[derive(Serialize)]
struct StagedDbPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.staged_db.v1"
    subject_kind: &'a str,
    subject_id: &'a str,
    expected_staged_row_count: u32,
    rows: &'a [String],
}

#[derive(Serialize)]
struct RunManifestContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.run_manifest.v1"
    subject_kind: &'a str,
    subject_id: &'a str,
    in_memory_payload_hash: &'a str,
    prepared_record_hash: &'a str,
    expected_staged_row_count: u32,
    staged_db_content_hash: &'a str,
    recovery_envelope_content_hash: &'a str,
    logical_subject_key: &'a str,
    run_status: &'a str,
    source_fact_key: Option<&'a str>,
    config_activation_run_id: Option<&'a str>,
    config_hash: Option<&'a str>,
    config_snapshot_json_hash: Option<&'a str>,
    config_activation_content_hash: Option<&'a str>,
    config_activation_file_content_hash: Option<&'a str>,
    config_effective_from_rfc3339_nanos_utc: Option<&'a str>,
    artifact_valid_from: Option<&'a str>,
    artifact_expires_at: Option<&'a str>,
    executable_revision: Option<&'a str>,
    legacy_cutover_snapshot_hash: Option<&'a str>,
    generation_market_date: Option<&'a str>,
    aggregator_observed_at_rfc3339_nanos_utc: Option<&'a str>,
    ingress_source_batch_content_hash: Option<&'a str>,
    outcome_phase: Option<&'a str>,
    stored_due_date: Option<&'a str>,
    outcome_claim_id: Option<&'a str>,
    planned_outcome_run_id: Option<&'a str>,
    outcome_claim_receipt_content_hash: Option<&'a str>,
    outcome_claim_due_binding_hash: Option<&'a str>,
    outcome_claim_provider_request_hash: Option<&'a str>,
    staged_at_rfc3339_nanos_utc: &'a str,
}
```

Payload domains are exactly `stock_analysis.br174.config_activation_payload.v1`,
`stock_analysis.br174.ingress_payload.v1`,
`stock_analysis.br174.generation_payload.v1`,
`stock_analysis.br174.outcome_claim_payload.v1` and
`stock_analysis.br174.outcome_payload.v2`. Table ordinals are one-based checked-in constants:
1 source-batch attempt, 2 source fact, 3 source-fact attempt, 4 relation attempt, 5 evaluation
attempt, 6 sample, 7 rejection, 8 sample outcome, 9 outcome attempt, 10 recovery envelope.
`logical_primary_key =
sha256_json(RunRowLogicalPrimaryKeyPreimage)` with exact key parts:
`[source_batch_attempt_id]`, `[source_fact_key]`, `[source_fact_attempt_id]`,
`[relation_attempt_id]`, `[evaluation_attempt_id]`, `[sample_key]`,
`[sample_key, ordinal base-10 without sign or leading zero]`, `[sample_key, phase]`,
`[outcome_attempt_id]`, and `[stage_run_id]` respectively. Key parts retain exact UTF-8 bytes;
arrays, delimiters and numeric formatting are therefore unambiguous and golden-vectored.

`RunPayloadPreimage.rows` contains the canonical domain rows that the envelope must
reconstruct and does not contain the envelope itself. `in_memory_payload_hash =
sha256_json(RunPayloadPreimage)`.

`StagedDbPreimage.rows` contains the already committed recovery-envelope row plus those domain
rows. Rows sort by `(table_ordinal, logical_primary_key UTF-8 bytes)`.
`expected_staged_row_count = StagedDbPreimage.rows.len()` and therefore equals
`1 + domain_row_count`; a config-activation or outcome-claim run has count 1. The manifest and receipt INSERT
triggers both require that equality. `staged_db_content_hash =
sha256_json(StagedDbPreimage)`. The run-manifest row is excluded from both preimages to avoid a
circular hash; after its fields are populated,
`run_manifest_content_hash = sha256_json(RunManifestContentPreimage)`. Before an audit commit or
recovery receipt, the repository re-queries the envelope and every domain row for the subject,
revalidates their run ID and row content hashes, and recomputes both hashes and the count.

The `RunPayloadPreimage` and `RunManifestContentPreimage` optional-field matrix is exact:

- `config_activation`: `source_fact_key`, generation/aggregator/ingress-batch/outcome/due/claim fields
  are NULL; `config_activation_run_id=subject_id`; config hash, snapshot JSON hash, activation
  content/file hash, effective time, artifact validity, executable revision and
  legacy-cutover-snapshot hash are required.
  `RunPayloadPreimage.rows=[]`.
- `ingress_run`: `source_fact_key`, snapshot/activation-content/file/effective/artifact/executable
  /legacy-cutover and outcome/due/claim fields are NULL; activation run/hash, generation market date,
  aggregator-observed time and ingress source-batch content hash are required.
- `generation_run`: source fact key, activation run/hash and generation market date are required;
  snapshot/activation-content/file/effective/artifact/executable/legacy-cutover,
  aggregator/ingress-batch and outcome/due/claim fields are NULL.
- `outcome_claim`: activation run/hash, outcome phase/stored due date, `outcome_claim_id=subject_id`,
  planned outcome-run ID, due-binding hash and provider-request hash are required; the claim-receipt
  hash is NULL and `rows=[]`.
- `outcome_run`: activation run/hash, outcome phase/stored due date, claim ID, claim receipt hash,
  due-binding hash and provider-request hash are required; planned outcome-run ID is NULL and
  `subject_id` must equal the referenced claim's planned outcome-run ID. Source fact,
  snapshot/activation-content/file/effective/artifact/executable/legacy-cutover, generation and
  aggregator/ingress-batch fields are NULL.

For ingress, the aggregate fields MUST equal `SourceIngressStageInputPreimage`; the batch hash MUST
also equal `sha256_json(SourceBatchContentPreimage)` recomputed from persisted provider-ordinal
children and `RunLogicalSubjectPreimage.ingress_source_batch_hash`. Every required/NULL condition
is a manifest CHECK plus a golden-vector assertion; no optional field may carry a value outside
its subject kind.

### 7.12 `selection_v2_commit_receipts`

Generic append-only receipt:

```text
(subject_kind = config_activation | ingress_run | generation_run | outcome_claim | outcome_run,
 subject_id, logical_subject_key, content_hash, in_memory_payload_hash,
 recovery_envelope_content_hash, prepared_audit_hash, run_manifest_content_hash,
 staged_db_content_hash, committed_audit_hash, committed_at)
```

Config activations and outcome claims reference their recovery envelope/manifest; source batch/fact attempts and first source facts carry their ingress run ID; relation/evaluation
attempts and samples carry their `generation_run_id`; outcome attempts and outcomes carry
their settlement run ID. Authoritative source/relation/report/backtest queries must inner join the
matching receipt.
For v2 admitted rows the generation receipt is also the BR-157 physical visibility receipt; there is
no second candidate projection or v1 visibility receipt. All twelve new tables receive
UPDATE/DELETE denial triggers and test/production code checks.

Receipt content is exact:

```rust
#[derive(Serialize)]
struct CommitReceiptContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.commit_receipt.v1"
    subject_kind: &'a str,
    subject_id: &'a str,
    logical_subject_key: &'a str,
    in_memory_payload_hash: &'a str,
    recovery_envelope_content_hash: &'a str,
    prepared_audit_hash: &'a str,
    run_manifest_content_hash: &'a str,
    staged_db_content_hash: &'a str,
    committed_audit_hash: &'a str,
    committed_at_rfc3339_nanos_utc: &'a str,
}
```

`selection_v2_commit_receipts.content_hash =
sha256_json(CommitReceiptContentPreimage)`. `prepared_audit_hash` is exactly the manifest's
`prepared_record_hash`; `committed_audit_hash` is exactly the persisted Committed audit record
hash. All manifest/envelope/staged hashes are copied from the verified rows, never recomputed from
display text.

`committed_at` is copied from the already-persisted committed audit record's `recorded_at`; it is
not a new retry wall clock. A receipt replay therefore derives identical bytes/hash. Receipt
insertion triggers re-check the matching manifest and, for generation runs, enforce for every
sample that admitted has `rejection_count=0` and no children while hard-rejected has exactly
`rejection_count>0` continuous children, the evaluation terminal hash matches, and the referenced
source fact/attempt/batch belong to one verified ingress receipt. For outcome runs it requires the
sample's activation/ingress/generation receipts, all preceding outcome receipts and exact settled
attempt/outcome hash equality. It also requires exact-one referenced `outcome_claim` receipt,
requires the outcome subject ID to equal that claim's preallocated outcome-run ID, and recomputes
the same logical subject, due binding and provider request hashes. A claim receipt with no matching
outcome receipt remains only an active recovery intent; it is never treated as settlement. This
makes corrupt staged rows permanently invisible even if a
caller bypasses repository validation. For ingress runs the trigger also enforces §7.1's exact
feed/child no-loss matrix. Because SQLite cannot authenticate the external audit file, normal,
review, test, live-canary and report/backtest startup remain fail-closed until
`VerifiedSelectionReadModel` has validated the complete chain and every receipt. That validation
uses the exact audit-lock + pinned SQLite snapshot choreography in §7.6 and the query consumes only
the verified keys from that same transaction; a DB-only receipt is never trusted.

### 7.13 Audit choreography and recovery

All five run kinds use the same locked, fully durable order:

1. build and validate the complete typed stage input in memory;
2. acquire the production/test selection-audit OS file lock and retain it through step 10;
3. validate the full existing audit chain, re-query the logical subject and recover an existing
   same-run envelope/Prepared/stage/Committed state instead of duplicating it;
4. insert the complete recovery envelope in its own SQLite transaction with verified
   `foreign_keys=ON`, `synchronous=FULL`, commit, then read it back and verify its bytes/hash;
5. append the matching Prepared audit record, flush the file and call `sync_data`; do not begin the
   domain transaction unless this durable append succeeds;
6. atomically stage all domain rows plus the run manifest in SQLite, commit under FULL, then
   re-query every row and recompute `staged_db_content_hash`;
7. append the matching Committed audit record, flush and call `sync_data`; do not insert a receipt
   unless the durable audit record and its record hash read back correctly;
8. insert the exact commit receipt in a separate SQLite FULL transaction whose triggers revalidate
   envelope, manifest, staged rows and decision matrix, then commit;
9. read back/re-hash the receipt and require that its committed audit hash exists in the validated
   on-disk chain;
10. release the OS lock.

Run-kind phase pairs are exact:

```text
config_activation -> V2ConfigActivationPrepared / V2ConfigActivationCommitted
ingress_run       -> V2IngressPrepared          / V2IngressCommitted
generation_run    -> V2GenerationPrepared       / V2GenerationCommitted
outcome_claim     -> V2OutcomeClaimPrepared     / V2OutcomeClaimCommitted
outcome_run       -> V2OutcomePrepared          / V2OutcomeCommitted
board_binding_audit -> V2BoardBindingAuditPrepared / V2BoardBindingAuditCommitted
gate_d_canary     -> V2GateDCanaryVerified
```

`V2OutcomeClaimPrepared` and `V2OutcomeClaimCommitted` are permanent
`schema_version=1` parser variants using the same generic Prepared/Committed content preimages and
hash-chain rules. They are not provider-success phases: they prove only one durable acquisition
intent. Once either appears in production, every release and rollback parser must retain both
variants and the claim recovery owner.

`board_binding_audit` is the release-attestation capture described in §5, not a database stage run:
it has no recovery envelope, domain-row manifest or `selection_v2_commit_receipts` row. Its
authoritative completion proof is the nested `BoardAuditAttestationReceiptPreimage` in the
checked-in artifact. The two new phase strings are permanent `schema_version=1` parser variants
and obey the same record-hash chain. Gate D reconstructs the Prepared/Committed content preimages,
requires the receipt's two exact record hashes in the fixed original production audit root, and
requires their phase, subject/run identity, content-hash link, order and previous-record chain to
validate.
`gate_d_canary` is likewise not a database stage run. It is the single post-scan release
attestation defined in Gate D: all referenced config/ingress/generation receipts already exist,
the zero-delivery scan has completed, and only then may its one permanent audit phase be appended
and synced. Its subject/content/time/root requirements are verified from
`GateDCanaryVerifiedPreimage`; it has no Prepared half and cannot authorize selection, delivery or
orders. The new phase string is also a permanent `schema_version=1` parser variant.

#### 7.13.1 Cross-process outcome acquisition claim

`OutcomeSettlementOwner` is the only module allowed to consume `VerifiedOutcomeDue`. Its public
result algebra is frozen and closed:

```rust
enum OutcomeSettlementOwnerResult {
    Receipted(CommitReceipt),
    LiveOwnedSkip,
    Superseded,
}
```

`Receipted` is returned only after exact receipt read-back. `LiveOwnedSkip` means the non-blocking
subject lock was busy; `Superseded` means fresh revalidation proved the due capability's relevant
receipt lineage advanced. Both skip variants emit the existing structured scheduler observation
with `logical_subject_key`, `verified_due_snapshot_hash` and a stable skip reason, but create no
claim, attempt, provider call or receipt. They are not success, verified empty, provider
Unavailable/error, or a swallowed `Option::None`; callers count and expose them separately.
Integrity failures are `Err(OutcomeIntegrityError)` and are not variants of this normal result
enum. Callers do not receive lock handles, path parameters, claim DTO constructors or a provider
client. The implementation owns the following exact sequence:

1. Close the read-model SQLite transaction, recompute the outcome logical-subject key from the
   opaque due capability and attempt one non-blocking exclusive OS advisory lock at
   `env!("CARGO_MANIFEST_DIR")/data/locks/production/selection-outcome-claims/<logical_subject_key>.lock`.
   The root, namespace and filename derivation have no caller, CLI, environment or CWD override.
   Each path component is opened no-follow from a pinned manifest root; a symlink, non-regular lock
   object, path escape, identity change or permissions mismatch fails closed. The lock file is
   retained permanently and is never used as state.
2. While holding that subject lock, acquire the selection-audit lock, then open a fresh SQLite
   read transaction. Revalidate the complete audit chain, frozen database/audit high-water prefix,
   sample lineage, relevant receipt-set hash, same-subject high-water, absence of a matching
   outcome/claim, full T0..D5 vector/prefix and all due/request bytes. Allocate exactly one claim
   UUIDv7 and one planned outcome-run UUIDv7 only in memory if no recoverable intent exists. Close
   this read transaction before any generic persistence transaction begins. Lock busy returns
   `LiveOwnedSkip`; relevant-lineage advance returns `Superseded`.
3. Persist the exact `OutcomeClaimStageInputPreimage` using the existing generic choreography,
   while continuously retaining the selection-audit lock: (a) recovery envelope in its own SQLite
   `synchronous=FULL` transaction, commit and read-back; (b)
   `V2OutcomeClaimPrepared` append/sync, then the zero-domain-row `claimed` manifest in a second
   SQLite FULL transaction, commit and read-back; (c) `V2OutcomeClaimCommitted` append/sync, then
   the exact claim receipt in a third SQLite FULL transaction, commit and read-back. No transaction
   spans those boundaries and the pre-write revalidation transaction is never reused as any of
   them. This is not a second mutable claim table or an unaudited lease. Release the
   selection-audit lock after receipt verification, but retain the subject lock.
4. Evaluate the single shared market session. If it returns ExpectedWait, make no provider call.
   Otherwise invoke the exact typed Magic-TDX request from the claim. No selection-audit lock or
   SQLite transaction is held during either the session check or provider/network I/O.
5. Build the outcome stage using only the exact claim plus the real result/partial evidence, then,
   still holding the subject lock, reacquire the selection-audit lock and first use a read-only
   revalidation transaction which is closed before persistence. Execute the normal outcome
   choreography with three distinct SQLite FULL transactions—envelope; domain rows+manifest;
   receipt—separated by the durable Prepared and Committed appends while the audit lock remains
   held. Read back the receipt, require its claim reference and every hash to match, release the
   audit lock and finally the subject lock, then return `Receipted(receipt)`.

The lock order is globally fixed:

```text
one outcome-subject claim lock
  -> selection-audit OS lock
    -> SQLite transaction
```

No code may acquire a claim lock while holding the selection-audit lock or a SQLite transaction,
and no worker may hold two subject locks. Generic ingress/generation/audit paths never request a
claim lock, so their audit → SQLite order cannot form a cycle. Recovery and normal settlement use
the same order. A debug/test lock-order tracker plus an architecture test rejects reverse nesting;
provider test doubles assert both audit and SQLite guards are absent at every invocation.

The subject lock, not a timestamp, proves active ownership. Process death releases it
automatically. There is no lease column, heartbeat, timeout, stale-age threshold, lock-file delete
or “steal” operation. A lock-busy claim remains live-owned even when its intent is old; the current
process skips it and may continue to another subject. A lock-free old intent is recovered
regardless of age. Network delivery is necessarily at-least-once across a process death at the
request/response boundary, but never concurrent for one subject: recovery repeats the exact
request bytes/hash under the same claim and planned run ID. If the provider response already
reached the outcome recovery envelope, recovery performs zero provider calls. The internal
claim/manifest/audit/receipt transition is exactly-once by identity and content conflict; the
external provider transport is only serial at-least-once. No documentation, metric or push may
describe external exactly-once delivery. A transport replay retains the claim's logical attempt
ordinal and full previous-attempt receipt evidence and never creates a replacement logical claim.

Claim state is append-only and derived, never updated:

```text
claim_partial:
  claim envelope/Prepared/manifest/Committed exists, exact claim receipt absent
claim_active:
  exact claim receipt exists, no matching outcome envelope or receipt
outcome_recovery:
  exact claim receipt and matching outcome envelope/manifest/Committed exist,
  matching outcome receipt absent
claim_closed:
  exact matching outcome receipt exists and references the claim receipt/hash
```

The predicates are mutually exclusive; any mixed/duplicate/cross-claim state is startup-fatal.
Both partial and active claims are “unreceipted outcome ownership” for due-query purposes and are
excluded from new due work. A closed `expected_wait` or `failed_retryable` claim may become due only
through a later fresh receipt-verified due snapshot whose same-subject high-water includes that
outcome receipt. Thus two stale due capabilities cannot create sequential duplicate runs after
waiting on a live owner.

Recovery enumerates claims by `claim_enveloped_at, claim_id` ascending and tries one subject lock at
a time. Within a claim it first completes claim receipt, then completes an existing outcome run
without provider work, then and only then replays active acquisition. It always reuses the stored
claim UUIDv7, planned outcome-run UUIDv7, due-binding bytes/hash and request bytes/hash. Recovery
never allocates a new run for an existing claim and never silently closes it. If immutable sample,
receipt, audit-prefix, calendar-vector, database binding or config lineage no longer validates,
recovery fails closed with a structured `OutcomeIntegrityError` diagnostic, writes no outcome
attempt/manifest/receipt and does not close the claim. The tick stops before later due/provider
work; startup/read-model health remains failed until the immutable corruption is repaired by an
audited roll-forward. `failed_non_retryable` is reserved for a normally validated lineage whose
real provider/domain result is explicitly non-retryable. Only that normal outcome receipt may
close the claim. An independent outcome receipt, cross-claim content or lineage conflict is
integrity-fatal rather than inferred as completion.

No notification, relation or market evaluation can run before the relevant ingress receipt.
Generation consumes only source facts joined to both their config-activation and ingress receipts.
Outcome consumes only samples joined to their generation receipt.

The selection-audit lock is an exclusive OS advisory lock on the existing `.lock` path, not an
in-memory mutex. The writer exposes a locked session so nested appends do not reacquire it. For
ordinary run stages, provider/network work is completed before that audit-lock acquisition. Outcome
acquisition follows §7.13.1: its subject claim lock spans the call, but both claim and outcome
Prepared/Committed audit pairs are complete audit-lock critical sections on opposite sides of the
call. Only recovery-envelope stage, validation, audit append, domain stage and receipt occur inside
an audit-lock critical section. A process death releases either OS lock automatically. A second
process must then validate the full audit chain and re-query the subject before recovery; it may
never steal or time-expire a live owner. This gives one committed record per
`(phase, subject_id, content_hash)` without a stale-owner heuristic.

The board-binding capture is the one deliberate network-between-phases exception: it holds the
same production audit lock only while validating/appending/syncing Prepared, releases it before
either provider call, then reacquires it, revalidates the complete chain and appends/syncs
Committed. It never holds the lock across network I/O. A concurrent intervening append is handled
by the normal previous-record hash on the newly appended Committed record; the receipt binds the
exact Prepared and Committed record hashes. Prepared without Committed is non-release evidence and
must never emit an artifact.

The authoritative audit remains:

```text
data/audit/production/selection-audit.jsonl
data/audit/production/selection-audit.lock
```

Tests use `data/audit/test/...` or a temporary root and TEST_CODE-only schema. The audit writer must
validate the complete SHA-256 chain, hold the cross-process lock, flush and `sync_data`; audit and
database evidence are retained at least five years and have no cleanup path.

The production writer has exactly one zero-argument constructor. It resolves
`env!("CARGO_MANIFEST_DIR")/data/audit/production` and accepts no caller path, environment
variable, CWD-relative root or `Production` enum argument. The old
`for_environment(root, Production)` interface is deleted. A temporary root is accepted only by a
`#[cfg(test)]` TEST_CODE constructor which always appends its own `test` namespace and cannot
produce a production writer. Existing production pipeline, legacy settlement, schema-v2 recovery,
read-model and release callers all use the same zero-argument constructor. Production resolution
rejects `..` and any existing symlink component. This fixed-path constructor does not replace Gate
D's later descriptor-pinned/openat identity proof; it only removes the caller-controlled root
before any v2 receipt can be trusted.

The on-disk audit remains `schema_version=1` and
`domain=stock_analysis.selection_audit.v1`. BR-174 only adds enum phase values; it must not add,
remove or reorder fields in `SelectionAuditRecord` or `SelectionAuditContext`, because validation
re-serializes the strict v1 struct and any default field would invalidate the production prefix.
V2 choreography uses the existing `subject_id` and `content_hash` fields:

```rust
#[derive(Serialize)]
struct PreparedAuditContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.prepared_audit_content.v1"
    subject_kind: &'a str,
    subject_id: &'a str,
    logical_subject_key: &'a str,
    recovery_envelope_content_hash: &'a str,
    in_memory_payload_hash: &'a str,
}

#[derive(Serialize)]
struct CommittedAuditContentPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.committed_audit_content.v1"
    subject_kind: &'a str,
    subject_id: &'a str,
    logical_subject_key: &'a str,
    recovery_envelope_content_hash: &'a str,
    prepared_record_hash: &'a str,
    run_manifest_content_hash: &'a str,
    staged_db_content_hash: &'a str,
}
```

Prepared/committed `content_hash` is `hex(sha256(serde_json::to_vec(preimage)))` under §6.1.

The staged run rows and receipt store `in_memory_payload_hash`, `prepared_record_hash`,
`staged_db_content_hash` and the final committed record hash for independent verification. No hash
component is stuffed into a semantically unrelated context field. The validator must prove an
existing real v1 prefix, append a BR-174 prepared/committed pair, restart, and validate the complete
cross-version phase chain without rewriting one historical byte.

The enum extension is a permanent forward-compatibility migration and lands in a standalone commit.
Once any BR-174 phase exists in production audit, rollback must retain the new parser variants; an
old binary that cannot deserialize them is prohibited. Behavioral code/config may be reverted or
disabled, but the audit compatibility commit is never reverted. Release evidence includes old
prefix + new phase + restart validation and a rollback binary built with the retained parser.

Failure/recovery:

- envelope failure writes no Prepared audit or domain row;
- crash after envelope before Prepared resumes from the exact canonical envelope;
- Prepared audit failure leaves only a non-authoritative recovery envelope;
- crash/failure after durable Prepared before domain stage reconstructs exact rows from the envelope;
- domain stage failure writes no Committed audit/receipt;
- committed-audit failure leaves staged domain rows invisible and retryable from the envelope;
- receipt failure leaves a durable Committed audit plus staged rows invisible until the same-run
  receipt recovery completes;
- crash after ingress receipt but during notification projection leaves the already receipted source
  fact pending and queryable; notification replay follows its own existing dedup contract;
- crash after ingress receipt but before relation generation leaves one authoritative, queryable
  pending fact for later generation;
- retry with identical content validates the chain, finds or appends the one committed record and
  completes the missing receipt;
- a different content hash conflicts;
- report/backtest never reads staged/unreceipted rows;
- outcome receipt failure never makes an outcome or its attempt visible.

Crash recovery must find an existing validated record by `(phase, subject_id, content_hash)` while
holding the same audit lock for the whole recovery window. `SelectionAuditWriter` therefore gains a
read-only lookup that returns the
persisted record/record hash after validating the full chain; callers must not blindly append a
second committed record. Same identity with a different content hash conflicts.

### 7.14 Gate A amendment: schema migration and validator seam

One deep validation module owns the canonical request, attempt ID, sample key, run logical subject
and row-content recomputation. Its interface has two operations:

1. pure payload validation, which consumes the complete typed stage input and returns validated
   canonical hashes without database or network access;
2. authoritative lineage validation, which consumes the pure result plus one pinned SQLite
   transaction and proves activation/source/sample/receipt equality.

Envelope creation calls both operations before durable write. Staged read-back reconstructs every
typed row from SQL columns and calls both again. The receipt path calls both a third time under the
global audit lock and in the same pinned database transaction immediately before receipt INSERT.
The SQL trigger remains an independent relational/NULL/cardinality guard. No caller may construct
a `Validated*Stage` capability directly, and no read model consumes an unvalidated stage.
For outcome claim/run ownership, the first authoritative revalidation transaction is a read-only
claim/supersession gate and is closed before the generic writer starts. The generic writer then
repeats the necessary validation inside its own envelope FULL transaction, domain+manifest FULL
transaction and receipt FULL transaction; those three transactions are distinct, while the
selection-audit lock remains continuously held as specified in §7.13.1. “Calls both before durable
write” never authorizes reusing the read transaction for an INSERT or collapsing these transaction
boundaries.

This amendment changes canonical field order and hashes. The affected version boundary is exact:

- `FeedAttemptContentPreimage` uses
  `stock_analysis.br174.feed_attempt_content.v2`;
- `RelationAttemptPreimage` and `OutcomeAttemptPreimage` use
  `stock_analysis.br174.relation_attempt.v2` and
  `stock_analysis.br174.outcome_attempt.v2`;
- the four request-bearing attempt-row content preimages use the exact `*.v2` domains listed in
  §6.2;
- `OutcomeMarketRequestParametersPreimage` uses
  `stock_analysis.br174.outcome_market_request.v2` and schema `outcome-market-request-v2`;
- `SelectionSampleRowContentPreimage` uses
  `stock_analysis.br174.selection_samples_row.v2` because the full T0..D5 vector and D2/D4
  columns are now immutable sample evidence;
- source-ingress, generation, outcome-claim and outcome envelopes use payload schemas
  `source-ingress-stage-v2`, `generation-stage-v3`, `outcome-claim-stage-v2`,
  `outcome-stage-v3` and matching stage-input
  domains `stock_analysis.br174.source_ingress_stage.v2`,
  `stock_analysis.br174.generation_stage.v3`,
  `stock_analysis.br174.outcome_claim_stage.v2`,
  `stock_analysis.br174.outcome_stage.v3`;
- config activation and every unaffected preimage retain their declared `v1` domain.

The amended schema revision accepts only those exact affected-v2/unaffected-v1 combinations.
Startup, recovery and receipt validation reject an affected `v1` domain inside a v2 stage, a v2 row
inside a v1 stage, a stage discriminator/domain mismatch, and any mixed v1/v2 row slice.
Production continues to use only the fixed
`SELECTION_RELEASE_DATABASE_RELATIVE_PATH = "data/stock_analysis.db"`; this migration introduces no
`DATABASE_PATH`, alternate live namespace or selection-only database override.

#### 7.14.1 Fixed-database migration policy

The affected pre-release table set is closed:

1. `selection_source_batch_attempts`;
2. `selection_source_facts_v2`;
3. `selection_source_fact_attempts`;
4. `selection_relation_attempts`;
5. `selection_evaluation_attempts`;
6. `selection_samples`;
7. `selection_rejections`;
8. `selection_sample_outcomes`;
9. `selection_outcome_attempts`;
10. `selection_v2_recovery_envelopes`;
11. `selection_v2_run_stages`;
12. `selection_v2_commit_receipts`.

Startup first performs schema introspection without mutating the database. Exactly three complete
states are recognized:

1. all twelve affected tables are absent and the selection audit has no v2 phase: normal database
   initialization creates the fresh amended schema transactionally. This is first-time schema
   creation, not migration;
2. all twelve tables are present with the amended columns/domains and the exact five payload
   schemas (including `generation-stage-v3`, `outcome-claim-stage-v2` and `outcome-stage-v3`):
   normal validation continues;
3. all twelve tables are present with the exact pre-amendment schema: selection generation and
   settlement stay disabled until one of the two migration paths below succeeds.

Any partial table set, mixed schema revision, unexpected affected object, or all-absent table set
with an existing v2 audit phase fails closed. Fresh initialization repeats `integrity_check` and
`foreign_key_check` before and after DDL and never changes a non-v2 schema object or row.
Both fresh initialization read-back and the already-amended state must additionally prove via
`PRAGMA table_xinfo(selection_source_batch_attempts)` that `record_count.notnull == 0` and via the
canonical table-DDL/CHECK validator that Available is `IS NOT NULL AND > 0`, VerifiedEmpty is
`IS NOT NULL AND = 0`, and Unavailable is `IS NULL`. Column-name presence or a legacy
`NOT NULL DEFAULT 0` definition cannot qualify as amended.

**Empty pre-release path.** In-place transactional rebuild is allowed only while the global
database maintenance lock is held, every producer of any table in `stock_analysis.db` is quiesced,
the connection pool is closed, WAL is checkpointed, and the migration connection has exclusive
access. In that same lock window the command proves:

- every one of the twelve affected tables exists in the expected pre-amendment schema and has
  `COUNT(*) = 0`;
- recovery envelopes, run manifests/stages and commit receipts are therefore independently empty,
  not inferred empty from an attempt count;
- the selection audit prefix contains no Prepared/Committed BR-174 stage record that claims one of
  those absent rows;
- `PRAGMA integrity_check` is exactly `ok` and `PRAGMA foreign_key_check` returns zero rows.

The command creates and fsyncs a full SQLite online-backup snapshot before DDL, then rebuilds only
the allow-listed affected tables, indexes and triggers in one transaction. Any nonzero table,
audit-prefix claim, unexpected schema object, active writer, WAL/SHM instability or failed check
aborts without committing DDL. The full backup is retained until release verification finishes.

**Non-empty pre-release path.** Automatic or in-place migration is prohibited. Normal startup never
copies or switches the global database. It fails closed for schema-v2 generation/settlement and
reports the exact nonempty affected table names and counts; it never logs row payloads, account data
or holdings. Only the separately built/reviewed
`selection-v2-request-evidence-migrate` operator command, invoked while the normal monitor is
stopped and after its own release gate, may:

1. acquire the global database-maintenance and selection-audit locks, quiesce every reader/writer
   owner that can retain a SQLite connection, close all pools, checkpoint WAL, and require the
   fixed main database to be a regular no-follow file with no live `-wal`/`-shm` sidecar;
2. capture the source database identity, full selection-audit prefix hash/high-water and a
   pre-migration manifest, then use SQLite's online-backup API to make a complete, standalone copy
   of the **entire** fixed production database on the same filesystem;
3. validate the standalone copy before modification, rebuild only the twelve affected v2 tables
   and their allow-listed indexes/triggers, and never reinterpret or backfill an old request hash;
   old pre-release v2 rows remain only in the sealed full-database rollback copy bound by the
   migration manifest;
4. revalidate the candidate and atomically exchange its main file with
   `data/stock_analysis.db` using a same-filesystem exchange primitive; a platform without an
   atomic exchange primitive fails closed. After exchange, fsync both files and the parent
   directory. The exchanged old main file becomes the immutable rollback/audit copy.

Before and after the candidate's v2-only DDL, the migration manifest proves preservation of every
non-v2 object. It contains:

- the exact ordered `sqlite_schema(type,name,tbl_name,sql)` rows plus `PRAGMA table_xinfo`,
  `foreign_key_list`, `index_list` and `index_xinfo` output for every non-v2 table/index/trigger/view;
- per non-v2 table row count and a domain-separated row-multiset hash. Each SQLite cell is encoded
  with an explicit storage-class tag and exact value bytes (`NULL`, signed i64, f64 `to_bits`, UTF-8
  text bytes, or BLOB bytes); complete encoded rows are sorted bytewise before hashing, preserving
  duplicates and distinguishing integer, real, text and blob representations;
- `application_id`, the expected `user_version` transition, source/candidate database identity
  hashes, selection-audit prefix hash/high-water, and full `integrity_check` and
  `foreign_key_check` results.

Only the allow-listed v2 schema-object set and the declared `user_version` may differ. Any
non-v2 schema, row count, typed row-multiset hash, foreign key, account/position/order/audit table,
external audit prefix or database-identity mismatch rejects the candidate before exchange.
Candidate migration also rejects any non-v2 schema object that depends on an affected v2 object.

Unreceipted old envelopes are never replayed into amended tables. The migration manifest lists their
subject IDs and hashes and requires an explicit audited disposition before exchange; the sealed
rollback database remains the authoritative evidence for every pre-amendment v2 row and the retained
version-pinned parser validates that database independently. Runtime report/backtest/read-model
queries never attach, join or fall back to the rollback database.

Full-file rollback is safe only before the production database is reopened: while the global
quiesce lock remains held, the exchanged live file still equals its recorded post-exchange database
identity and no table has received a write. In that exact state the same atomic exchange may restore
the old complete file. Once any post-exchange write occurs—whether selection, account, position,
order, audit or unrelated—full-file rollback is prohibited because it would lose real production
state. The rollback copy becomes immutable audit evidence; disable new selection generation and
roll forward the v2 schema/parser on the current live database. There is no destructive downgrade.

## 8. Trading-day settlement

D1, D3 and D5 mean the first, third and fifth A-share trading days after the evaluation market date.
They are fixed when the terminal sample is created, not recalculated later.

Formal selection uses a new immutable `SelectionCalendarSnapshot` built only from the checked-in,
source-cited `config/a_share_market_holidays.csv`; the mutable `TRADING_HOLIDAYS`/`add_holidays`
global is not an authoritative outcome scheduler. Snapshot creation:

- rejects invalid/duplicate dates and unsupported coverage;
- requires cited coverage through the computed D5 horizon;
- computes and stores `calendar_version`, sorted-calendar SHA-256, all six
  T0/D1/D2/D3/D4/D5 dates, canonical `OutcomeTradingDateVectorPreimage` bytes and its hash;
- fails closed on parse/read/coverage error;
- does not use a poisonable process-global lock.

`selection_samples` stores the immutable schedule. Due queries read those dates directly. Restart,
environment changes, runtime holiday injection or later calendar-file versions cannot change an
existing sample. A new calendar version applies only to newly created samples.

`due_v2_outcomes(as_of, limit)` requires `1..=200`, first passes
`VerifiedSelectionReadModel`, then reads only terminal samples whose config activation and
source-ingress and generation receipts are verified. It returns only the earliest missing phase
per sample whose stored due date is `<= as_of`; D1/D3/D5 additionally require every preceding
phase, including `t0_close`, to have a verified outcome receipt. It excludes a phase with an
existing receipted outcome or an unreceipted recovery
envelope/manifest for the same outcome logical subject, and excludes a logical subject whose latest
receipted manifest, selected by the shared
`committed_at DESC, subject_id DESC LIMIT 1` helper from §7.11, is `settled` or
`failed_non_retryable`. Latest `failed_retryable` remains due on a later receipt-verified tick.
Latest `expected_wait` is suppressed until the deterministic
`stored_due_date 15:00:00.000000001 Asia/Shanghai` instant defined in §7.9, so scheduler ticks
before that instant cannot create another claim/run/receipt. Once eligible, a new due snapshot
must include the prior wait receipt in its same-subject lineage. There is at most one new run for
that logical subject per receipt-verified due snapshot. The anti-join also excludes every matching outcome-claim recovery
envelope, Prepared/manifest/Committed state or receipted claim not yet closed by its exact outcome
receipt. Such work is visible only to §7.13.1 recovery, never returned as a new due capability.
Each opaque due capability binds its canonical
`VerifiedOutcomeDueSnapshotPreimage`, relevant receipt tuples, database/audit high-water proof and
same-subject high-water; claim
creation revalidates all of them after acquiring the subject lock, so a capability read before another
process's claim/outcome receipt cannot be reused. It returns at most `limit` rows ordered by
`stored_due_date, sample_key, phase` ascending.
`phase` uses fixed ordinal
`t0_close < d1_settled < d3_settled < d5_settled`. No legacy candidate or outcome table participates.

Settlement requirements:

- daily/historical evidence no more than one trading day stale;
- requested sample code and returned code identical;
- complete requested validation window and unique continuous trading dates. The validation window
  always starts at the stored T0 evaluation date and ends at the stored phase due date, so its
  expected provider-bar count is exactly `1/2/4/6` for `T0/D1/D3/D5`. This is required to apply
  BR-171 to the T0→D1 adjacency as well as every later adjacency. A D1-only one-row request is
  invalid because it cannot prove that first adjacent move. The request's expected dates are the
  exact `[T0]`, `[T0,D1]`, `[T0,D1,D2,D3]` or `[T0,D1,D2,D3,D4,D5]` prefix of the sample's
  canonical vector; the admitted provider dates must be byte-identical to that prefix;
- positive finite prices and valid OHLCV/amount;
- BR-171 manual-confirmation gate for adjacent moves above ±20%;
- unadjusted series and lifecycle consistency;
- immutable batch evidence.

`settled` is not a public row-construction API. The production acquisition seam is a
Magic-TDX-only exact-window operation owned by the historical-bars Gateway. It accepts the
receipt-verified sample/code/market/phase/due/window request and returns an
`AdmittedOutcomeDailyBars` capability whose fields and constructor are private. That capability
contains the complete admitted daily window, its provider-ordered content hash, the exact typed
request evidence and complete available evidence. It is constructed only after the same complete
price/OHLC/volume/amount, date continuity, duplicate, unadjusted-series, lifecycle, freshness and
BR-171 confirmation gates used by admitted historical daily bars. Tencent, Sina, Baidu, a single
settled bar, a caller-built evidence struct, or a caller-built outcome row cannot construct it.

The existing public `CompletedSessionTerminal::Settled { outcome, evidence }` input is deleted.
`VerifiedSelectionReadModel` instead returns an opaque `VerifiedOutcomeDue` bound to the receipted
sample preimage, activation/config lineage, immutable schedule, all preceding phase receipts and,
for D1/D3/D5, the receipted T0 close and volume. The settlement function consumes
`VerifiedOutcomeDue` plus `AdmittedOutcomeDailyBars` and computes the outcome row internally:

- T0 return, cumulative MFE and cumulative MAE are exactly `0`, and volume ratio is exactly `1`;
- D1 uses the exact D1 bar and the receipted T0 baseline, while the admitted validation window also
  contains the exact T0 bar for continuity and BR-171;
- D3/D5 require the complete ordered T0-through-due validation window, but compute cumulative
  MFE/MAE only over D1-through-due. T0 high/low is never included in excursion math;
- every later-phase volume ratio uses the receipted T0 full-session volume.

The capability and verified due must match sample, canonical code/market, phase, stored due date,
window, provider capability and the already receipted claim request hash byte-for-byte. The
dedicated owner must implement §7.13.1 and keep the fixed subject lock through outcome receipt; a
caller cannot invoke the Gateway directly from a due capability. A Gateway error becomes only a
typed outcome error attempt under that same claim. Until both capabilities, the claim choreography
and the dedicated v2 settlement owner exist,
production schema-v2 settlement remains explicitly disabled; the direct public settled input may
not be retained as a compatibility fallback.

After the claim receipt exists, if the due trading day has not completed, return `ExpectedWait`
without a provider call and close that claim through the planned outcome run. The resulting wait
receipt activates the deterministic pre-close suppression above; it is not written again on each
tick. Missing, partial, conflicting or
bad data is an explicit retryable/non-retryable outcome attempt and does not write a placeholder.
No D1/D3/D5 outcome can commit before the sample and source-ingress receipts or before every
preceding phase receipt. The outcome-receipt trigger repeats that phase chain and requires the
persisted receipted `t0_close` close/volume as the baseline; phase market dates must equal the
stored schedule exactly.

## 9. Reporting and backtest

The schema-v2 report renders separately:

- admitted sample count and fully settled count;
- hard-rejected sample count and fully settled count;
- relation-attempt failure count by typed reason/retryability;
- evaluation-attempt evidence-failure count by typed reason/retryability;
- outcome-attempt expected-wait/error count by phase and reason;
- excluded legacy v1 inbox and admitted-only outcome counts.

Returns, MFE/MAE and volume ratios are raw facts. “Success rate”, threshold quality or strategy
superiority must not be rendered until a separately reviewed calibration design proves sample size,
label and confidence interval. Relation/evaluation/outcome failure attempts never enter return
denominators.

The backtest binary reads only immutable, receipted, prospective schema-v2 samples/outcomes. It may
compare cohorts but may not reconstruct historical board membership, mutate admission thresholds,
production state, visibility or deduplication state.

The shared `V2ReportFilter` is exact:

```rust
struct V2ReportFilter {
    evaluation_date_from: Option<NaiveDate>, // inclusive
    evaluation_date_to: Option<NaiveDate>,   // inclusive
    chain_id: Option<String>,
    canonical_stock_code: Option<String>,
    decision: Option<AdmittedOrHardRejected>,
    phase: Option<V2OutcomePhase>,
    limit: u32, // 1..=10_000
}
```

Invalid/reversed dates, unknown identities or limits outside `1..=10_000` fail before querying.
`v2_report()` uses one `VerifiedSelectionReadModel` snapshot and builds a `BaseSampleSet` by
inner-joining the source fact plus activation, source-ingress and generation receipts, then
applying date/chain/code/decision filters.
Outcome rows and receipts are LEFT JOINed so unsettled samples remain in the base cohort. The phase
filter does not remove a base sample: it selects the one outcome/attempt phase projected for that
sample; `None` selects all four fixed phases.

All cohort counts, fully-settled counts, raw-return denominators and attempt aggregates are computed
over the complete filtered `BaseSampleSet` before pagination. With `phase=None`, “fully settled”
means all four receipted phases; with `Some(phase)`, it means that selected phase is receipted.
`limit` applies only to distinct base samples, not expanded sample-phase rows. The sample page sorts
by `evaluation_market_date, event_id, chain_id, canonical_stock_code, sample_key` ascending and
takes the first `limit`; its child outcomes then sort by fixed phase ordinal. There is no offset or
unstable default ordering.

Outcome-attempt aggregates join the complete base set and obey the optional phase filter.
Relation/evaluation attempts obey date/chain/code filters; when `decision` is set, only attempts
linked to a terminal sample in that selected decision cohort are included, while sample-less
evidence failures remain visible only when `decision=None`. Phase never filters relation/evaluation
attempts. Attempts are grouped by exact `(stage, result_code, reason_code, retryable)` and never
reduce a return denominator. `legacy_excluded` is the frozen cutover snapshot's total pending-inbox
and admitted-only-outcome counts read from the verified config-activation envelope's
`LegacyCutoverSnapshotPreimage`, explicitly unfiltered by v2 fields and never unioned into v2
rows. A report fails if later activations do not carry the identical snapshot hash. The legacy
`selection::report`, `opportunity::news_outcome` and v1 due queries have physical
table allow-lists that exclude all schema-v2 tables; the new report/backtest query has the inverse
allow-list.

## 10. Failure matrix

| Condition | Result |
|---|---|
| source fact missing batch/record identity, publication, or evidence | source-inbox rejection; no sample |
| source fact future or stale at first ingress observation | immutable ingress rejection; no relation/sample |
| ingress-admitted fact retries across a day boundary | reuse immutable ingress receipt; do not reclassify stale |
| feed `VerifiedEmpty` lacks evidence or a registered feed lacks terminal status | source batch unavailable; no global complete/empty claim |
| notification simhash suppresses a projection | source fact still enters v2 inbox/evaluation |
| chain not matched | verified no relation |
| explicit provider board triple missing | relation rejection `board_binding_not_configured`; direct mention unaffected |
| explicit provider board triple partial/conflicting | configuration snapshot unavailable |
| provider board live audit not proven | relation rejection `board_binding_unverified` |
| provider request hash lacks its exact typed parameters/capability preimage | stage rejection; no envelope/attempt visibility |
| constituent result empty (`InvalidData`) | non-retryable relation rejection; no security sample |
| constituent result length equals 10,000 | relation rejection `board_constituents_may_be_truncated` |
| constituent batch partial or board identity conflict | relation rejection preserving upstream retryability |
| constituent security cannot canonicalize | relation rejection; raw provider identity stays audit-only |
| canonical security exists but later market evidence conflicts | per-security evaluation attempt failure |
| market/K-line/5m evidence incomplete or bad | evaluation attempt failure |
| complete evidence fails admission | hard rejection |
| sample/rejection/audit commit fails | no visibility; retry |
| outcome claim lock is held by another process | owner returns observable `LiveOwnedSkip`; zero claim/attempt/provider/receipt; no lease/steal and not success/empty/error |
| due capability relevant high-water advances before claim | owner returns observable `Superseded`; no claim/attempt/provider/receipt; obtain a fresh verified due snapshot later |
| outcome claim envelope/Prepared/manifest/Committed is partial | exclude from due; recover exact claim receipt under the same subject lock |
| receipted claim has no outcome artifacts | exclude from due; replay exact market-session/request intent under the same subject lock |
| outcome envelope exists without receipt | exclude from due; finish exact outcome recovery with zero provider re-fetch |
| claim/outcome run ID, due binding, request hash or receipt link conflicts | startup/stage failure; no inferred closure or replacement claim |
| immutable sample/config/receipt/audit/calendar/database lineage is corrupt during claim recovery | integrity failure; no `failed_non_retryable`, no new outcome artifacts and claim remains unclosed |
| fixed claim-lock namespace is overridden, symlinked or identity-changed | fail closed before claim/provider work |
| due trading phase not completed | at most one pre-close receipted `expected_wait`; no placeholder/provider call; suppress until due-date 15:00:00.000000001 Asia/Shanghai |
| outcome request/provider dates differ from the applicable canonical vector prefix | explicit partial/invalid evidence retaining actual returned dates; no settled capability |
| outcome stage sample/config/logical-subject lineage differs from receipted sample | stage/receipt rejection; no outcome visibility |
| outcome data unavailable/invalid | receipted outcome attempt error; no placeholder |
| process dies during outcome provider request | OS releases subject lock; recover same claim/request serially; provider delivery may repeat but never concurrently |
| all twelve v2 tables absent and no v2 audit phase | fresh amended schema creation at the fixed production database |
| partial/mixed v2 table set or audit-only v2 phase | startup failure; no schema mutation |
| existing non-empty schema lacks the amended canonical evidence fields | normal startup failure; separately gated full-database operator migration only |
| board capability unsupported | one explicit disabled banner; no zero-sample health claim |

## 11. Old modules

| Module | Decision | Reason |
|---|---|---|
| `src/selection/pipeline.rs` BR-155/156/157 pipeline | adopt and deepen | owns governed source fact, admission and visibility |
| `src/news/aggregator/{feed,mod}.rs` | adopt and change internal schema | preserve raw facts and typed feed request evidence; replace evidence-erasing feed status and make simhash notification-only |
| `src/bin/monitor/news_aggregator_init.rs` | adopt, split, then delete old API | `tick_news_aggregator_batch` currently mutates notification simhash before returning; replace with raw fetch plus receipt-gated notification projection |
| `src/bin/monitor/main.rs` news tick owner | adopt and reorder | durable source ingress receipt must precede NewsFlash/NewsAI simhash and selection evaluation |
| `src/bin/monitor/selection_shadow.rs` | adopt and replace adapter | split source ingress, receipted generation and v2 settlement; legacy due adapter remains v1-only until drained |
| `src/data_gateway/board.rs::BoardDataGateway` | adopt and deepen | only downstream owner of Magic board providers |
| `BoardDataGateway::memberships` | preserve | BR-170 still owns the reverse security-to-board lookup |
| `src/data_gateway/magic_tdx_selection.rs` | adopt and deepen | owns same-provider market evidence |
| `src/selection/schema_v2.rs` request/stage preimages | adopt and version | persist typed request evidence plus outcome config/sample lineage; new field order requires new golden vectors |
| `src/database/selection_v2_repository.rs` | adopt and deepen | one validator seam for envelope write, typed SQL read-back and pre-receipt authoritative revalidation |
| `src/database/selection_v2.rs` DDL/triggers | adopt and migrate | add request-evidence pairs; fresh-create or empty-only in-place rebuild at the fixed database, otherwise fail startup |
| `DatabaseManager` global pool/maintenance owner | adopt and deepen | prove all SQLite owners quiesced and preserve every non-v2 table during the separately gated full-database operator migration |
| `selection-v2-request-evidence-migrate` operator command | new, separately released | full online backup, v2-only candidate DDL, non-v2 preservation manifest and same-filesystem atomic exchange; never invoked by monitor startup |
| generation/outcome Magic TDX request constructors | adopt and deepen | capture canonical typed parameters and capability identity before the real provider call; no post-hoc reconstruction |
| `VerifiedSelectionReadModel` | adopt and deepen | revalidate request/sample/logical-subject evidence before exposing receipted rows |
| `src/opportunity/chain_mapper.rs:754-825` fuzzy/search/fetch Top-N | reject for formal acquisition; delete only after all callers are audited | violates exact binding/completeness and may still serve a legacy display caller before cutover |
| `src/opportunity/mod.rs` legacy candidate generation | reject; delete after caller audit | defaults/missing evidence and different identity model |
| `src/database/selection.rs` | adopt and migrate | owns v1 freeze plus v2 inbox/attempt/sample/outcome/receipt storage |
| `src/selection/outcome.rs` | preserve for pre-cutover `legacy-v1` only, then freeze | v2 due work reads only receipted `selection_samples` |
| `src/opportunity/news_outcome.rs` | reject and delete after caller audit | legacy opportunity outcomes cannot own schema-v2 settlement |
| `src/bin/monitor/dryrun_report.rs` legacy opportunity backfill callers | reject and delete with `news_outcome` | v26 dry-run reporting must consume the v2 report and may not keep a hidden legacy outcome acquisition owner |
| `src/selection/audit.rs` | adopt; enum-only v1 extension | preserve canonical historical JSON and hash chain |
| `src/selection/report.rs` | adopt and replace query contract | expose BR-178 receipt-verified dual-cohort report; keep no schema-v1/v2 union |
| `src/bin/selection_live_probe.rs` | adopt and deepen | adds exact binding/directory/constituent evidence |
| `src/bin/selection_backtest.rs` | adopt and deepen | change legacy visible-admitted query to receipted prospective dual cohort |
| `src/calendar.rs` mutable global | preserve for existing consumers; reject for v2 schedule | environment/runtime mutation cannot change immutable outcome dates |
| AI beneficiary proposal | reject | unsupported causal claim |
| v1 `selection_event_inbox` | freeze/read-only | lossy payload cannot be upgraded or replayed |
| legacy `selection_outcomes` | settle existing v1 T0/D1, then freeze/read-only | admitted-only history lacks source-bound/rejected identity |

After BR-174 production integration is proven, replaced legacy candidate acquisition and its unused
configuration/rollback switches are deleted under BR-164. Account, holding, cash, order and audit
data are outside this deletion scope.

The pre-implementation caller audit is executable and must be rerun after migration:

```bash
rg -n \
  "SelectionEventBatch|FeedAttemptStatus|SelectionAuditPhase|\
tick_news_aggregator_batch|evaluate_news_batch|settle_due_outcomes|\
due_outcomes\\(|visible_samples\\(|append_outcome\\(|append_candidate|\
stock_analysis::selection::report|selection::report|news_outcome::|\
selection_(event_inbox|event_completions|runs|candidates|feature_snapshots|\
outcomes|visibility_receipts)" \
  src
```

The baseline callers requiring disposition are:

```text
src/bin/monitor/main.rs:4154              legacy selection settlement owner
src/bin/monitor/main.rs:6286,6301         old aggregator/evaluate order
src/bin/monitor/news_aggregator_init.rs:83,602 old simhash-mutating batch API and test
src/bin/monitor/selection_shadow.rs:41,109 old generation/settlement adapters
src/bin/monitor/dryrun_report.rs:158,207  legacy opportunity outcome backfill
src/bin/selection_backtest.rs:10          `stock_analysis::selection::report` legacy import
```

Gate B creates `tools/compliance/lib/check_br174_legacy_callers.sh` and the reviewed symbolic
allow-list `tools/compliance/fixtures/br174_legacy_allowed_callers.txt`. The script runs the
multiline audit above, canonicalizes each match to `(file, enclosing Rust item or SQL migration
name, matched contract)`, and fails on every entry not present in the exact allow-list or every
allow-list entry no longer present. After cutover the only production allowances are:

- `src/database/selection.rs` v1 DDL and physical-freeze/trigger verification only; the immutable
  cutover-snapshot reader reads the receipted config-activation envelope, not live v1 tables;
- private `LegacyV1OutcomeSettlementRepository` plus the monitor legacy drain owner while the
  receipt-verified all-terminal anti-join still finds a committed legacy-v1 candidate missing T0
  or D1;
- permanent v1-compatible `SelectionAuditPhase` parser variants.

No old aggregator/evaluator, public v1 writer, dryrun `news_outcome`, v1 report/backtest or direct
v1 table consumer is allowed. When the `as_of`-independent anti-join proves every committed
legacy-v1 candidate has both T0 and D1, the settlement caller allowance is removed and the drain
owner is disabled. The permanent conditional outcome INSERT guard remains byte-for-byte unchanged;
no trigger is installed, removed or replaced and the immutable trigger-registry hash is unchanged.

## 12. Implementation slices

1. Land the amended canonical request/outcome-claim/outcome stage preimages, strict typed recovery DTOs, shared
   validator seam, fresh/empty-only fixed-database schema initialization and its fail-closed startup
   state matrix before any producer or repository consumer compiles against them. The separately
   gated non-empty operator migration command is a distinct release slice with full-database
   preservation tests; monitor startup does not call it.
2. Replace feed-attempt completeness with evidence-bearing variants and append every raw source fact
   to the v2 inbox independent of notification simhash.
3. Extend `BoardDataGateway` with exact complete constituent acquisition and live evidence.
4. Add schema-v2 inbox/relation/evaluation/outcome-attempt, terminal sample/rejection/outcome and
   run-manifest/commit-receipt repository APIs plus append-only triggers.
5. Add exact board/direct relation resolution and full ordered relation evidence to selection.
6. Compute admission in memory, then atomically stage only terminal admitted/hard-rejected samples;
   expose v2 admitted visibility only through the generation-receipt join.
7. Add immutable calendar snapshot plus D3/D5 due dates, the fixed cross-process outcome-claim
   owner/lock namespace, claim audit phases and exact claim→outcome receipt transition before any
   v2 settlement provider call is enabled.
8. Add settlement phases behind that owner and prove provider I/O holds neither audit lock nor
   SQLite transaction.
9. Add prospective receipt-bound dual-cohort report/backtest.
10. Wire monitor and live probe; unsupported capability emits one explicit banner.
11. Audit every old caller, then delete only the proven-replaced candidate acquisition and unused
   configuration.

Implementation may be parallelised only where files do not overlap. Database migration lands before
outcome/report consumers compile against it.

## 13. Acceptance evidence

### Unit and integration

- binding config: complete triple, missing, partial, conflict and unverified audit;
- binding artifact: fixed path, strict canonical hash, exactly two independent provider-owned
  `concept`/`industry` directory-batch evidence objects in category UTF-8 order with distinct batch
  IDs, complete provider-ordered record preimages, contiguous ordinals and record evidence equal to
  its batch; missing/duplicate/reordered/cross-category/singular-aggregate/opaque-hash-only evidence,
  zero/equal-limit/count-length mismatch and batch-ID collision all reject. The request limit is
  exactly `10_000`; `9_999`, `10_001` and every other value reject;
- binding proposal: fixed canonical path/schema/hash, unique sorted exact triples, non-empty reviewer,
  exact `selection-board-binding-validity-v1` chronology/window, one-to-one proposal→artifact
  binding derivation, empty list allowed only under the Gate B/C rule. The sole loader rereads the
  fixed proposal raw bytes on every startup/reload and rejects any raw/canonical/nested-preimage/hash
  drift from the artifact;
- binding directory validation: all records in both batches are checked even when `bindings=[]`;
  whitespace-only/non-trim-stable names, wrong category kind, zero member count, non-contiguous
  ordinal, duplicate identity, wrong `tdx:{category}:{name}` code or record/batch evidence mismatch
  reject even when the bad record is not selected by the proposal;
- binding evidence freshness: capture accepts only observed times not in the future and at most
  exactly 300 seconds old at `recorded_at`; 301 seconds, malformed/non-canonical/overflow
  `unix-ms`, RFC3339 precision/offset drift, sign, whitespace, leading zero or trailing input reject;
- binding connection policy: the audit CLI accepts only `--output`, reuses
  `BoardDataGateway::production_tdx`, recomputes the exact
  `selection-board-tdx-production-v1` policy hash and records JSON `null` endpoint evidence;
  host/IP/port/source/resolver/retry/timeout and audit-root overrides reject;
- binding release attestation: a live capture writes and syncs one
  `V2BoardBindingAuditPrepared`/`V2BoardBindingAuditCommitted` pair, recomputes the attestation and
  nested receipt hashes and emits no artifact for Prepared-only or chain/phase/subject/run/hash/root
  mismatch. Gate D validates the exact two record hashes and attestation link against the fixed
  original production audit root;
- binding audit: exact-one triple lookup in the corresponding category records, recomputed
  `directory_record_hash`, matched `release_directory_member_count`, byte-identical outer
  audit-command/connection-policy/receipt/time/validity fields, not-found/duplicate/hash/count
  mismatch rejection, empty
  live-audited binding list valid only for Gate B/C, duplicate/missing/expired/upstream-pin/config
  mismatch, human-reviewed renewal and valid rollback;
- binding audit command: exact `selection-board-binding-audit-v1`; attempts to inject provider
  evidence, records, binding selection, proposal path/policy, IDs, hashes, counts, recorded time,
  validity or precomputed binding/artifact hashes through CLI/config reject before output; binding
  selection and validity come only from the fixed checked-in PR-reviewed proposal;
- config activation: consumes only the private-construction typed verified artifact from
  `src/data_gateway/board.rs`; a second parser, typed/raw canonical mismatch or missing nested
  directory record bytes fails before activation;
- feed attempts: evidenced Available/VerifiedEmpty, unavailable, missing evidence, missing registered
  feed and global completeness; receipt rejects missing/extra/duplicate/cross-feed child attempts
  and non-contiguous/reordered provider ordinals, independently recomputes per-feed and aggregate
  hashes, and requires exact Available `record_count`, while VerifiedEmpty requires zero and
  Unavailable requires SQL NULL;
- source inbox: every provider/batch/item independent of simhash, exact replay idempotency,
  cross-batch unchanged replay attempt, changed-content conflict and v1 cutover exclusion;
- hash registry: exact source/feed/board-artifact/binding/relation/sample/evaluation/outcome/run/
  receipt golden preimages, including `BoardBindingProposalInputPreimage`, every
  `DirectoryBoardRecordPreimage` record hash, both category-ordered
  `DirectoryBatchContentPreimage` content hashes and the complete nested artifact, per-binding
  proposal/hash/reviewer/validity/connection-policy/attestation-receipt/record-hash/
  release-member-count fields, `BoardAuditPreparedContentPreimage`,
  `BoardAuditAttestationContentPreimage`, `BoardAuditCommittedContentPreimage` and
  `BoardAuditAttestationReceiptPreimage`, feed batch evidence,
  provider-ordered feed source-record/content hashes, JSON `null` absence and field/list ordering;
  a superseded singular-directory `v1`, detached directory digest or incomplete record list must
  fail;
- typed request identity: feed/board/T0/outcome attempts round-trip their exact parameter and provider
  capability JSON through row columns, stage envelope, SQL read-back and receipt validation;
  unknown parameter schema/field, non-canonical JSON, wrong capability tuple/revision,
  cross-kind parameter-schema swaps, canonical-subject/parameter mismatches,
  parameters/capability/request hash mismatch and a self-consistent forged final request hash all
  reject. Direct/not-configured/no-call ExpectedWait require the exact all-NULL request matrix;
- amended version vectors: golden fixtures assert the exact v2 domains for feed attempt,
  relation/outcome attempt IDs and all four request-bearing attempt rows, the
  `selection_samples_row.v2` domain, `outcome-market-request-v2`, `generation-stage-v3`,
  `outcome-claim-stage-v2` and `outcome-stage-v3`. Each old-domain-in-new-stage,
  new-domain-in-old-stage, discriminator/domain mismatch and mixed row-slice fixture rejects at
  envelope write, SQL read-back, restart recovery and receipt;
- generation logical subject: the canonical UUIDv7 plus exact
  `source_fact_key/config_hash` preimage hash passes; any other 64-hex logical key, malformed UUID,
  or cross-config/source substitution rejects at envelope write, read-back and receipt;
- outcome authoritative identity: canonical UUIDv7, exact persisted `SampleKeyPreimage`, activation
  run/config hash, phase and immutable due date pass; a self-consistent envelope with a forged
  sample key, logical key, config hash or due date rejects against the receipted sample at all three
  validation points;
- verified due snapshot golden vectors: exact fixed database binding, receipt-snapshot rowid/hash,
  audit ordinal/tail/prefix hash and the complete role-sorted receipt tuples reproduce one
  `VerifiedOutcomeDueSnapshotPreimage` hash. Extra/missing/duplicate/reordered tuples, a
  claim-receipt substituted into `previous_same_subject_attempt_receipt_hashes`, wrong
  committed-at/UUIDv7 order, database/schema substitution, audit prefix drift or forged
  high-water rejects before claim creation;
- outcome calendar vector: sample stage stores canonical `[T0,D1,D2,D3,D4,D5]`; due, claim and
  typed request round-trip the same vector/hash and exact phase prefix. Provider success is
  admitted only when returned dates are byte-identical to that prefix; missing/extra/reordered/
  duplicate/alternate-spelling dates preserve actual provider dates in an explicit error and
  cannot settle;
- ingress gate: initial stale/future rejection and fresh-at-ingress cross-day retry remains eligible;
- five-run crash matrix: config activation, ingress, generation, outcome claim and outcome each inject
  `after_envelope_before_prepared`, `after_prepared_before_stage`,
  `after_stage_before_committed` and `after_committed_before_receipt`; restart must reconstruct the
  exact same run/rows from envelope bytes without provider refetch and produce one authoritative
  receipt. Ingress additionally injects after receipt but before notification/relation;
- strict recovery payload parsing: all five stage schemas round-trip through their typed
  `Deserialize + deny_unknown_fields` recovery DTO and reproduce byte-identical canonical JSON.
  Unknown/duplicate/nested-extra fields, wrong domain, trailing input, non-canonical field order,
  schema/type mismatch and an untyped-JSON/hash-only recovery implementation all reject before
  Prepared/staging;
- constituents: strict under-limit batch, empty, max-limit truncation risk, partial, duplicate and
  identity mismatch;
- relation: distinct direct spans/values and board binding identities, direct/member merge and
  stable ordering;
- relation attempt: zero/one/multiple exact match without fabricated stock identity;
- attempt identity: same-run replay idempotent, later same-error run appends a new attempt;
- run manifest: evidenced global empty, unavailable feed, verified-no-relation and zero-sample
  generation all remain receipted and recoverable;
- recovery queue: envelope-only ordered by `enveloped_at,stage_run_id`, manifested-unreceipted
  ordered by `staged_at,stage_run_id`, disjoint membership and fail-before-new-provider-work;
- cross-process outcome claim: two independent OS processes synchronize after reading the same
  `VerifiedOutcomeDue`, then race the fixed subject lock. A counting provider proves maximum
  concurrency exactly one; SQL/audit evidence proves exactly one claim ID, one planned outcome-run
  ID and one claim→outcome receipt link. The loser returns exactly `LiveOwnedSkip` or `Superseded`,
  emits its structured skip observation, and makes zero claim/attempt/provider/receipt writes;
- owner result algebra: compile-time exhaustive matching permits only
  `Receipted(CommitReceipt) | LiveOwnedSkip | Superseded`; receipt read-back is required for the
  first variant, skips are counted separately, and neither skip can be interpreted as success,
  verified empty, provider error or `None`;
- claim transaction boundaries: transaction-ID instrumentation proves the initial revalidation
  transaction closes before persistence and that envelope, domain+manifest and receipt use three
  different FULL transactions; the selection-audit lock remains held across those three
  transactions but no SQLite/audit guard reaches provider I/O;
- claim crash matrix: kill the owner after claim envelope, Prepared, manifest, Committed, claim
  receipt, provider request dispatch, provider response and outcome envelope/Prepared/stage/
  Committed. Restart must obtain the OS-released lock, reuse byte-identical claim/due/request
  preimages, logical attempt ordinal, previous-attempt receipt list and planned outcome-run ID, and
  produce one internally exactly-once closure. A response already in the outcome envelope causes
  zero provider re-fetch; a kill at request/response may cause one later **serial** replay of the
  same request but never a concurrent call or new logical claim;
- stale/live claim: an arbitrarily old lock-free active claim is recovered, while an equally old
  claim whose lock is still held by another OS process is not stolen after any duration. Tests
  assert there is no lease/heartbeat/timeout column, environment switch or lock-file deletion;
- claim due exclusion and exact replay: claim-partial, claim-active and outcome-recovery fixtures
  are absent from normal due results and appear in exactly one stable recovery class ordered by
  `claim_enveloped_at,claim_id`; closed ExpectedWait/failed-retryable becomes eligible only through
  a new verified due snapshot containing the previous receipt high-water. Mutating request hash,
  attempt ordinal, previous-attempt receipts, sample/config/phase/window/T0 binding, claim ID or
  planned run ID rejects before provider/stage;
- pre-close wait suppression: ticks before, exactly at and after `15:00:00 Asia/Shanghai` prove the
  gate remains incomplete at exactly 15:00, the first pre-close claim closes with one receipted
  ExpectedWait, every later tick before `15:00:00.000000001` produces no new claim/run/receipt, and
  the first later eligible due snapshot includes that wait receipt. The assertion is at most one
  pre-close wait per sample/phase/due date across restart and two processes;
- corrupt claim lineage: mutate each immutable sample/config/receipt/audit-prefix/calendar-vector/
  database-binding component after a claim receipt. Recovery returns integrity failure, emits the
  structured diagnostic, writes no `failed_non_retryable` or other outcome artifact and leaves the
  claim unclosed. A separately valid provider/domain non-retryable error still produces exactly one
  `failed_non_retryable` receipt and closes its matching claim;
- claim lock namespace/order: CWD/env/caller-root changes, symlink/path escape and alternate lock
  filenames cannot address production; injected audit→claim, SQLite→claim or provider-under-audit/
  SQLite nesting fails. Different subjects can progress while one subject is live-owned and no
  worker ever holds two subject locks;
- sample/rejection: append-only, idempotent replay, content conflict and terminal-only matrix;
- database pool: every production/test connection reads `foreign_keys=1` and `synchronous=2`;
- store mode: production rejects `TEST_CODE_`, test rejects real symbols, including
  `cargo run --bin monitor -- --test`;
- admission: admitted, hard-rejected and evidence-failure evaluation attempts;
- settlement: weekend/holiday D1/D3/D5, full immutable T0..D5 schedule, independent retry, outcome
  attempts, T0-zero baseline, D1..phase MFE/MAE and phase/T0 volume ratio;
- outcome capability seam: no public settled row/evidence DTO or constructor remains;
  `AdmittedOutcomeDailyBars` and `VerifiedOutcomeDue` cannot be forged, and any sample/code/market/
  phase/due/window/provider/request mismatch rejects before stage creation;
- outcome closure negatives: manifest/receipt reject zero or two attempts, `settled` with zero or
  two outcomes, every non-settled status carrying an outcome, and every status/result,
  retryable-flag or settled-content-hash mismatch; inserting a manifest before its complete domain
  rows also fails immediately. The full §7.9 required/NULL matrix is exercised: `settled` rejects
  non-NULL reason/retryable/error detail/fingerprint or missing complete evidence; ExpectedWait
  rejects any reason other than `market_session_unsettled` and any non-NULL
  retryable/provider/evidence/error-detail/fingerprint; error rejects a missing/unregistered
  reason, NULL retryable, missing/mismatched error-detail/fingerprint pair, or a half-present
  partial-evidence JSON/hash pair. `settled_bar_missing` as ExpectedWait is rejected and the same
  reason is accepted only as a typed error with a non-NULL retryability classification and its
  actual partial-evidence contract;
- schema amendment initialization/migration: all twelve tables absent plus no v2 audit phase creates
  the fresh amended schema; every partial-presence permutation and an audit-only phase fails closed.
  Exact amended schema starts normally. Exact pre-amendment schema rebuilds in place only when all
  twelve counts and the v2 audit-phase count are zero; a parameterized fixture placing one row in
  each affected table independently denies that path. Normal startup never runs the non-empty
  operator migration and never reads a `DATABASE_PATH` override. Fresh/amended fixtures assert
  `record_count.notnull=0` from `table_xinfo` and the exact Available non-NULL-positive,
  VerifiedEmpty non-NULL-zero and Unavailable-NULL CHECK matrix; named-column-only,
  `NOT NULL DEFAULT 0`, omitted `IS NOT NULL`, and SQL-three-valued-logic variants all reject;
- full-database operator migration: an isolated production-shaped TEST_CODE database contains
  sentinel account, position, cash, order, delivery-audit and unrelated application rows plus
  nonempty old v2 rows. The operator command requires global quiescence, makes a complete SQLite
  online backup, changes only allow-listed v2 schema objects, and proves byte-identical non-v2
  schema manifests, counts and typed row-multiset hashes plus clean integrity/foreign keys before
  same-filesystem atomic exchange. Mutation injection for each SQLite storage class, a changed
  non-v2 table/index/trigger/view, a non-v2 dependency on v2, active writer, WAL/SHM sidecar,
  symlink, cross-filesystem candidate, unresolved envelope or audit-prefix drift rejects before
  exchange;
- migration crash/rollback: failpoints after backup, after v2 DDL, before exchange, after exchange
  before directory fsync and after fsync restart into either the complete old or complete new fixed
  database, never a mixed file set. While still quiescent and byte-identical to the post-exchange
  baseline, atomic exchange restores the full rollback file. A single injected post-exchange write
  to selection, account, position, order, audit or an unrelated table permanently denies full-file
  rollback and requires disable-generation plus roll-forward recovery;
- migration release evidence records the old/new golden-vector fixture hashes, amended
  domain/payload-schema tokens, source/candidate/rollback database identities, non-v2 manifest
  hash, selection-audit prefix hash/high-water, migration binary SHA and rollback binary SHA, and
  proves the retained parser validates the sealed pre-amendment database while the runtime
  read-model/report never attaches or joins it;
- disabled-owner rollback: an isolated TEST_CODE database pre-seeds a fully receipted due v2
  sample, starts the real monitor selection scheduler with
  `STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE=0`, proves zero config/ingress/generation provider calls
  and zero new generation runs, while the dedicated settlement owner produces exactly one
  receipted claim plus its exactly linked outcome run (or the fixture's explicit `expected_wait`)
  and a valid audit chain;
- report: both cohorts, relation/evaluation/outcome attempts and legacy exclusion;
- due/report query: BR-178 exact filter/order/limit, duplicate phase suppression, unreceipted-run
  exclusion and physical v1/v2 table allow-list isolation;
- audit: real existing v1 prefix, BR-174 enum-only append, restart, retained-parser rollback and
  complete-chain validation;
- production audit constructor: the old caller-path production constructor is absent; changing CWD,
  environment or an arbitrary root cannot change or construct the production writer, while the
  isolated TEST_CODE constructor cannot address the production namespace;
- receipt trust: startup and each authoritative read fail closed on DB-only, missing-audit,
  mismatched, duplicate or malformed receipts;
- cutover: v1 pending inbox is never replayed; committed v1 candidate only finishes legacy T0/D1;
- cutover quiesce: every v1 acquisition graph table rejects INSERT/UPDATE/DELETE after the barrier,
  and only the explicit legacy T0/D1 outcome transaction remains writable until drained;
- cutover trigger/drain negatives: startup fails when the exact legacy trigger registry has a
  missing trigger, extra trigger or changed canonical SQL; a real `Pending -> Complete` fixture
  proves the owner is disabled while the same conditional guard SQL/hash remains unchanged,
  `Complete -> Pending` is impossible on the frozen append-only graph, and direct legacy outcome
  INSERT is rejected after `Complete`;
- cutover query isolation: v1 due query cannot see v2 samples and v2 due query cannot see v1
  candidates;
- architecture: no provider construction or financial/news hosts outside Gateway.

`selection_v2_crash_recovery` uses isolated TEST_CODE databases/audits and a counting typed provider
fixture only to exercise failure paths. For every run-kind/failpoint pair it asserts envelope bytes
match the golden stage-input vector, exactly one internal Prepared/Committed pair and receipt exist,
and the final authoritative row set equals the no-failpoint control. Restart performs zero
additional provider calls once outcome response evidence is enveloped. The separate OS-process
claim test permits a serial same-request replay only for the injected request/response crash
boundary and proves it retains the same logical attempt ordinal/claim/run IDs. No fixture enters a
production path.

### Commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
cargo test --test selection_v2_crash_recovery -- --test-threads=1
cargo test --test selection_v2_outcome_claim_cross_process -- --test-threads=1
cargo test --test selection_v2_disabled_settlement_owner -- --test-threads=1
bash tools/compliance/check.sh
bash tools/compliance/lib/check_br174_legacy_callers.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Release evidence uses these bounded forms rather than leaving the daemon running indefinitely:

Gate B extracts the currently duplicated runtime path rules into one
`RuntimeArtifactRoots::resolve()` used by `save_push_log`, `AuditDispatcher::for_runtime`,
event-bus construction and `selection_live_probe`. It resolves and canonicalizes the push-log root
(including `PUSH_LOG_DIR` and production/test default), authoritative delivery-audit root
(including `EVENT_AUDIT_DIR` namespace handling), and event-bus root (the fixed
`runtime_data_path(test_mode, "event_bus")` rule, with source `fixed_runtime_data_path`).
For each root the canary JSON stores `runtime.<kind>_root`, `<kind>_root_source`
(`default | env | fixed_runtime_data_path`) and `<kind>_root_binding_hash`, which hashes the
fixed-order tuple `(domain="stock_analysis.br174.runtime_artifact_root.v1", kind, source,
canonical_absolute_root)`.
Before canary `started_at`, `RuntimeArtifactRoots::open_pinned_for_scan()` resolves those same
bindings, opens each root with the same component-wise no-follow handle/identity protocol as
`ProductionEvidenceRoots::open_pinned()`, records an object-binding hash over
`(kind,path_binding_hash,device,inode,file_type)`, and retains the exact handles through generation,
flush, scan and canary audit append. Every runtime writer exposes and uses a fixed cross-process OS
snapshot lock for its push-log, delivery-audit, event-bus or order-audit store. After generation,
the canary drains its in-process writer channels and acquires all four locks in that fixed order; a
writer without the registered lock protocol makes Gate D unsupported/fail closed.

The order-audit root is resolved and bound by the same shared resolver; it is not an implicit
hard-coded scan path. The permanent canary preimage contains both the path-binding hash and the
object-binding hash for all four runtime roots. Consequently the verifier can prove which pinned
directory objects were scanned, rather than merely proving that the producer named the expected
paths. A missing object-binding field, a path/object hash mismatch, or two runtime kinds resolving
to an unregistered shared object fails closed.

While the locks remain held it flushes/fsyncs writer state, captures `scan_cutoff_at`, enumerates
the pinned roots, and opens every child by name relative to its pinned root with no-follow
semantics. The snapshot manifest stores the sorted entry set and, per child, name, identity,
size/change metadata, high-water length and content SHA-256. Scanning reads exactly those pinned
bytes; a second enumeration/read must reproduce the same entry set, metadata, high-water and
content hash before attestation. Child create/remove, same-inode append/truncate/overwrite,
root rename/recreate or symlink change fails. The locks and handles remain held until
`V2GateDCanaryVerified` payload and audit record have both synced.

No separate release `--verify-no-delivery` command or persisted canary input exists. The
`--canary-live-selection` release mode owns the entire operation in one process: it pins the
production evidence paths/objects plus all runtime roots first without beginning a SQLite
transaction, captures `started_at`, and executes real ingress/generation. It then enters the
registered writer-freeze protocol described above and, while the order-audit lock remains held,
calls `snapshot_sqlite_after_writes()` through the pre-pinned descriptor VFS. Only that
post-generation immutable backup may prove the exact-one DB/receipt/audit identities and bind them
to the same activation/config. The canary then captures `scan_cutoff_at` and scans the exact
inclusive `[started_at, scan_cutoff_at]` window before emitting JSON. It derives the two stage run
IDs, source-fact key and sample key only from the just-committed typed results in memory. It accepts
no canary, database, identity, root or time-window argument. No hard-coded `data/push_log`,
`data/event_bus`, `data/event_audit*`, alternate namespace or caller-supplied directory can satisfy
the scan.

Successful zero-delivery proof is itself appended and `sync_data`'d under the fixed selection audit
root as the permanent phase `V2GateDCanaryVerified` before JSON is emitted:

```rust
#[derive(Serialize)]
struct GateDCanaryVerifiedPreimage<'a> {
    domain: &'static str, // "stock_analysis.br174.gate_d_canary_verified.v1"
    scope: &'static str, // "release_gate_d"
    initial_audit_tail_record_hash: Option<&'a str>,
    initial_audit_tail_at_rfc3339_nanos_utc: Option<&'a str>,
    activation_run_id: &'a str,
    config_hash: &'a str,
    activation_committed_audit_hash: &'a str,
    board_artifact_content_hash: &'a str,
    ingress_run_id: &'a str,
    generation_run_id: &'a str,
    source_fact_key: &'a str,
    sample_key: &'a str,
    ingress_committed_audit_hash: &'a str,
    generation_committed_audit_hash: &'a str,
    production_database_binding_hash: &'a str,
    production_database_snapshot_sha256: &'a str,
    production_database_logical_high_water_hash: &'a str,
    production_database_snapshot_at_rfc3339_nanos_utc: &'a str,
    selection_audit_root_binding_hash: &'a str,
    authoritative_delivery_audit_root_binding_hash: &'a str,
    authoritative_delivery_audit_root_object_binding_hash: &'a str,
    push_log_root_binding_hash: &'a str,
    push_log_root_object_binding_hash: &'a str,
    event_bus_root_binding_hash: &'a str,
    event_bus_root_object_binding_hash: &'a str,
    order_audit_root_binding_hash: &'a str,
    order_audit_root_object_binding_hash: &'a str,
    started_at_rfc3339_nanos_utc: &'a str,
    scan_cutoff_at_rfc3339_nanos_utc: &'a str,
    all_files_parsed: bool,
    push_log_matches: u64,
    event_bus_delivery_matches: u64,
    authoritative_delivery_audit_matches: u64,
    order_audit_matches: u64,
    sink_calls: u64,
    order_calls: u64,
}

#[derive(Serialize)]
struct GateDCanaryPayloadLinePreimage<'a> {
    schema_version: u16, // exactly 1
    domain: &'static str, // "stock_analysis.br174.gate_d_canary_payload.v1"
    content_hash: &'a str,
    canary: &'a GateDCanaryVerifiedPreimage<'a>,
}
```

All four match counts and both call counters must be zero and `all_files_parsed=true` before the
append. `gate_d_canary_content_hash = sha256_json(GateDCanaryVerifiedPreimage)` and
`gate_d_canary_subject_id` equals that hash. The audit record subject is exactly that subject ID,
its content hash is exactly the canary content hash, and
`canonical_nanos_utc(outer_recorded_at) >= scan_cutoff_at`.

The full preimage is not left only in stdout. While still holding the selection-audit OS lock and
all runtime snapshot locks, the producer appends exactly one compact canonical
`GateDCanaryPayloadLinePreimage` line to fixed
`data/audit/production/gate-d-canary-payloads.jsonl`, flushes and `sync_data`s that file, then
appends/syncs the `V2GateDCanaryVerified` record. The payload line's nested canary recomputes to its
`content_hash`; the outer audit record binds the same hash/subject. A crash before the audit append
may leave a retained inert orphan payload, but it is never authoritative. Missing payload for an
audit record, duplicate payload content hashes, non-canonical/invalid lines, or more than one audit
record for the same canary subject fail closed. The verifier holds the same lock, validates the
payload JSONL high-water/prefix, and requires exact-one payload for every canary audit record;
orphan payloads are reported and excluded from latest selection. The emitted JSON is generated
from the exact just-synced typed payload and also contains its content hash and tamper-evident audit
record hash.

Before `started_at`, the producer briefly locks/validates the selection audit chain and records its
initial tail hash/time (both `None` only for an empty chain); it does not hold that lock across
provider work. At final append it re-locks/revalidates, requires the immediate current tail to be
the previous record used by the new audit record, and enforces:

```text
initial_audit_tail_at <= started_at
started_at <= ingress receipt time <= generation receipt time
generation receipt time <= production database snapshot time <= scan_cutoff_at
scan_cutoff_at <= canonical outer canary recorded_at
canonical immediate pre-append tail time <= canonical outer canary recorded_at
canonical outer canary recorded_at - scan_cutoff_at <= GATE_D_CANARY_APPEND_MAX_SECS (30)
canonical outer canary recorded_at <= verifier_now
0 <= verifier_now - canonical outer canary recorded_at <= GATE_D_CANARY_MAX_AGE_SECS (300)
```

The verifier performs the same instant comparisons without signed/saturating age arithmetic.
Clock rollback, inverted start/cutoff, receipt outside the window, non-monotonic initial/current
tail, future outer timestamp, a 31-second cutoff-to-append delay, and negative/stale age are
mandatory negative vectors. A snapshot timestamp before the generation receipt, a backup hash or
logical-high-water hash mismatch, and attempting to begin the canary SQLite transaction before its
own writes commit are also mandatory failures.

`--verify-v2-receipts` independently selects the latest `V2GateDCanaryVerified` record for
`scope=release_gate_d` by `(canonical outer recorded_at DESC, record_hash DESC)`, requires it to be
no older than fixed `GATE_D_CANARY_MAX_AGE_SECS=300`, loads and reconstructs the exact-one canonical
payload, and revalidates all
referenced receipts, path/root bindings and fixed DB identities. Its output exposes
`latest_gate_d_canary_record_hash`; Gate D cross-checks that value against the canary JSON. A copied
old JSON, altered identity/time/root/count, older record after a newer canary, stale canary, missing
record or random identity cannot satisfy the record hash. Negative vectors cover each mutation.
`V2GateDCanaryVerified` parsing remains permanently backward compatible after first production use.

The first command below is a Gate D requirement, not a Gate B/C activation requirement. A
live-audited canonical artifact with `bindings=[]` may pass direct-mention Gate B/C tests, but
`--first-binding` must fail closed before a constituent request with
`verified_binding_required_for_gate_d`. Gate D evidence therefore requires a non-empty verified
binding list and one successful real constituent acquisition from that first binding.

Every Gate D probe resolves the proposal, artifact and original board-capture audit root from their
fixed checked-in/policy paths through `ProductionEvidenceRoots::open_pinned()`. Release modes accept
no database, proposal, artifact, selection-audit-root, board-audit-root or other path
argument and do not consult path environment variables for those production evidence roots: they
always use the fixed production database
`SELECTION_RELEASE_DATABASE_RELATIVE_PATH`, proposal, artifact, selection audit root, board audit
root anchored at `env!("CARGO_MANIFEST_DIR")`. Their JSON includes each canonical
path, `source="fixed_cargo_manifest_dir"`, a domain-separated binding hash over
`(kind, source, canonical_absolute_path)`, the raw artifact SHA-256, nested artifact content hash,
attestation-receipt hash, original-audit-root verification result, active `config_hash` and the
current DB activation receipt. That receipt must be authoritative in this exact fixed
database/audit snapshot and must bind both the same config hash and nested artifact hashes.
Each path binding hash is exactly
`sha256_json(ProductionEvidencePathBindingPreimage)` and is recomputed by the release verifier
rather than trusted from a prior JSON document.

A separate diagnostic mode may accept an explicit artifact or database path for local inspection,
but it always emits `release_eligible=false`, cannot query or assert a production activation
receipt/original audit root, cannot emit any `*_verified=true` field, is mutually exclusive with
every Gate D mode and cannot produce release evidence. Negative release-mode tests cover arbitrary
databases, changed CWD, `..`, a symlink at any database/audit-root component, and environment
overrides.

```bash
cargo run --bin selection_live_probe -- \
  --first-binding \
  --json > target/br174-selection-board-probe.json

jq -e '
  .release_eligible == true
  and .upstream_revision == "b2b68df78156df1d67824e5c44c0cb01b752f55a"
  and .provider == "tdx"
  and .source == "tdx-block-files"
  and .source_at == null
  and (.observed_at | type == "string")
  and (.batch_id | length > 0)
  and (.batch_content_hash | length == 64)
  and (.actual_constituent_count > 0 and .actual_constituent_count < 10000)
  and (.binding.code | length > 0)
  and (.binding.name | length > 0)
  and (.binding.kind == "industry" or .binding.kind == "concept")
  and (.binding.binding_audit_hash | length == 64)
  and .artifact.path == "config/selection/provider_board_bindings.v1.json"
  and (.artifact.raw_bytes_sha256 | length == 64)
  and (.artifact.content_hash | length == 64)
  and (.artifact.audit_attestation_receipt_hash | length == 64)
  and .artifact.path_source == "fixed_cargo_manifest_dir"
  and (.artifact.path_binding_hash | length == 64)
  and .artifact.original_audit_root_verified == true
  and (.config_hash | length == 64)
  and .activation_receipt.is_current_receipted_activation == true
  and (.activation_receipt.committed_audit_hash | length == 64)
  and .activation_receipt.config_hash == .config_hash
  and .activation_receipt.nested_board_artifact_content_hash == .artifact.content_hash
  and .activation_receipt.executable_input_artifact_raw_sha256
      == .artifact.raw_bytes_sha256
  and .production_database.source == "fixed_cargo_manifest_dir"
  and (.production_database.canonical_path | length > 0)
  and (.production_database.binding_hash | length == 64)
  and (.production_database.snapshot_sha256 | length == 64)
  and (.production_database.logical_high_water_hash | length == 64)
  and (.production_database.snapshot_at_rfc3339_nanos_utc | type == "string")
  and .board_audit_root.source == "fixed_cargo_manifest_dir"
  and (.board_audit_root.binding_hash | length == 64)
  and .selection_audit_root.source == "fixed_cargo_manifest_dir"
  and (.selection_audit_root.binding_hash | length == 64)
  and .proposal.source == "fixed_cargo_manifest_dir"
  and (.proposal.binding_hash | length == 64)
' target/br174-selection-board-probe.json

cargo run --bin selection_live_probe -- \
  --canary-live-selection \
  --json > target/br174-selection-live-canary.json

jq -e '
  .release_eligible == true
  and (.decision == "admitted" or .decision == "hard_rejected")
  and .used_real_global_news == true
  and .used_synthetic_or_mock == false
  and (.source.provider | length > 0)
  and (.source.item_id | length > 0)
  and (.source.batch_id | length > 0)
  and (.source.content_hash | length == 64)
  and (.source.source_fact_key | length == 64)
  and (.ingress_run_id | length > 0)
  and (.generation_run_id | length > 0)
  and (.sample_key | length == 64)
  and (.board.binding_audit_hash | length == 64)
  and (.market.provider == "tdx")
  and (.market.batch_id | length > 0)
  and .artifact.path == "config/selection/provider_board_bindings.v1.json"
  and (.artifact.raw_bytes_sha256 | length == 64)
  and (.artifact.content_hash | length == 64)
  and (.artifact.audit_attestation_receipt_hash | length == 64)
  and .artifact.path_source == "fixed_cargo_manifest_dir"
  and (.artifact.path_binding_hash | length == 64)
  and .artifact.original_audit_root_verified == true
  and (.config_hash | length == 64)
  and .activation_receipt.is_current_receipted_activation == true
  and (.activation_receipt.committed_audit_hash | length == 64)
  and .activation_receipt.config_hash == .config_hash
  and .activation_receipt.nested_board_artifact_content_hash == .artifact.content_hash
  and .activation_receipt.executable_input_artifact_raw_sha256
      == .artifact.raw_bytes_sha256
  and (.ingress_receipt.committed_audit_hash | length == 64)
  and (.generation_receipt.committed_audit_hash | length == 64)
  and (.runtime.authoritative_delivery_audit_root | length > 0)
  and (.runtime.authoritative_delivery_audit_root_source == "default"
       or .runtime.authoritative_delivery_audit_root_source == "env")
  and (.runtime.authoritative_delivery_audit_root_binding_hash | length == 64)
  and (.runtime.authoritative_delivery_audit_root_object_binding_hash | length == 64)
  and (.runtime.push_log_root | length > 0)
  and (.runtime.push_log_root_source == "default" or .runtime.push_log_root_source == "env")
  and (.runtime.push_log_root_binding_hash | length == 64)
  and (.runtime.push_log_root_object_binding_hash | length == 64)
  and (.runtime.event_bus_root | length > 0)
  and .runtime.event_bus_root_source == "fixed_runtime_data_path"
  and (.runtime.event_bus_root_binding_hash | length == 64)
  and (.runtime.event_bus_root_object_binding_hash | length == 64)
  and (.runtime.order_audit_root | length > 0)
  and (.runtime.order_audit_root_binding_hash | length == 64)
  and (.runtime.order_audit_root_object_binding_hash | length == 64)
  and .production_database.source == "fixed_cargo_manifest_dir"
  and (.production_database.canonical_path | length > 0)
  and (.production_database.binding_hash | length == 64)
  and (.production_database.snapshot_sha256 | length == 64)
  and (.production_database.logical_high_water_hash | length == 64)
  and (.production_database.snapshot_at_rfc3339_nanos_utc | type == "string")
  and .selection_audit_root.source == "fixed_cargo_manifest_dir"
  and (.selection_audit_root.binding_hash | length == 64)
  and .canary_identity_binding_match == true
  and .canary_receipts_verified == true
  and .authoritative_database_identity_matches == 1
  and .all_files_parsed == true
  and .push_log_matches == 0
  and .event_bus_delivery_matches == 0
  and .authoritative_delivery_audit_matches == 0
  and .order_audit_matches == 0
  and (.gate_d_canary.content_hash | length == 64)
  and (.gate_d_canary.audit_record_hash | length == 64)
  and .gate_d_canary.scope == "release_gate_d"
  and .gate_d_canary.payload_synced == true
  and .gate_d_canary.chronology_valid == true
  and .sink_calls == 0
  and .order_calls == 0
' target/br174-selection-live-canary.json

cargo run --bin selection_live_probe -- \
  --verify-v2-receipts \
  --json > target/br174-receipt-verification.json
jq -e --slurpfile canary target/br174-selection-live-canary.json '
  .release_eligible == true
  and .audit_chain_valid == true
  and .fixed_board_audit_root_verified == true
  and .board_attestation_receipt_verified == true
  and .foreign_key_violations == 0
  and .receipt_mismatches == 0
  and .ingress_receipts > 0
  and .generation_receipts > 0
  and .latest_gate_d_canary_payload_exact_one == true
  and .latest_gate_d_canary_chronology_valid == true
  and .latest_gate_d_canary_age_secs >= 0
  and .latest_gate_d_canary_age_secs <= 300
  and .latest_gate_d_canary_record_hash
      == $canary[0].gate_d_canary.audit_record_hash
' target/br174-receipt-verification.json

cargo run --bin monitor -- --test 2>&1 | tee target/br174-monitor-test.log
rg -n "\\[v70\\] E2E 完成" target/br174-monitor-test.log

cargo run --bin monitor -- --review 2>&1 | tee target/br174-monitor-review.log
rg -n "\\[复盘\\] ======== 盘后分析完成" target/br174-monitor-review.log

bash -c '
  target/release/monitor > target/br174-monitor-live.log 2>&1 &
  pid=$!
  sleep 120
  kill -INT "$pid"
  wait "$pid"
'
rg -n "\\[selection-shadow\\]\\[BR-174\\] ingress committed" target/br174-monitor-live.log
rg -n "\\[selection-shadow\\]\\[BR-174\\] generation committed" target/br174-monitor-live.log
rg -n "监控已安全关闭" target/br174-monitor-live.log

STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE=0 \
  target/release/monitor --selection-config-preflight \
  > target/br174-switch-disabled.log 2>&1
test "$(rg -c \
  "\\[[0-9:]+ (INFO|WARN)\\] \\[selection-shadow\\]\\[BR-174\\] mode=disabled reason=operator_switch" \
  target/br174-switch-disabled.log)" -eq 1

set +e
STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE=invalid \
  target/release/monitor --selection-config-preflight \
  > target/br174-switch-invalid.log 2>&1
invalid_switch_rc=$?
set -e
test "$invalid_switch_rc" -ne 0
test "$(rg -c \
  "\\[[0-9:]+ ERROR\\] \\[selection-shadow\\]\\[BR-174\\] mode=invalid reason=invalid_operator_switch" \
  target/br174-switch-invalid.log)" -eq 1

cargo test --test unified_data_architecture \
  br174_shadow_has_no_sink_or_order_dependencies -- --exact

rg -n "resolve_stocks\\(|find_best_board_match|COMPONENT_KEEP_TOP_N" \
  src/opportunity src/selection src/bin
```

The canary consumes the real registered `GlobalNewsGateway` feed set, scans fresh governed facts and
checked-in exact chain/binding rules, then executes real ingress → relation → Magic TDX market
evidence → admission → receipt. It does not accept an injected event, fixture or symbol. If no fresh
fact reaches a terminal live decision it exits non-zero with `canary_no_live_match`; disabled,
verified-empty or zero-candidate output is not Gate D success. The bounded monitor command must exit
zero after SIGINT and end with `监控已安全关闭`. The architecture
test has an exact empty dependency allow-list for sink/notify/push/order ports from the BR-174
producer. The in-process `--canary-live-selection` verification recursively parses every push
artifact, event-bus JSONL, authoritative delivery-audit record and order-audit row whose timestamp
overlaps its internally captured canary window; it searches the exact ingress/generation run IDs
and source/sample identities, fails on an unreadable/bad record and requires four independent
match counts to be exactly zero before appending `V2GateDCanaryVerified`. Canary
`sink_calls/order_calls` are supplemental diagnostics, not the zero-push proof.
`--selection-config-preflight` is the real monitor startup parser/owner with network, DB, sink and
order construction forbidden by an architecture test; it exists only to prove the production
rollback switch and exits immediately after the once-only banner. These checks plus the static
dependency test are required evidence. The final `rg`
command must return no production match before `resolve_stocks` and its fuzzy/Top-N implementation
are deleted. If any caller remains, deletion is blocked and that caller must first migrate under
BR-164; the check is not suppressed.

The normal monitor run is bounded and shut down gracefully after evidence collection. Live evidence
must show the explicit config triple and real TDX constituent provider/source/observed/batch fields.
Missing `source_at` is displayed as absent. Test and production databases remain physically isolated:
production uses `data/stock_analysis.db`; `monitor --test` uses only its isolated test database,
TEST_CODE identities and `data/audit/test/**`.

Expected bounded production evidence includes:

```text
[selection-shadow][BR-174] ingress committed run_id=<id> facts=<n> available=<n> verified_empty=<n> unavailable=<n>
[selection-shadow][BR-174] generation committed run_id=<id> admitted=<n> hard_rejected=<n> relation_attempts=<n> evaluation_attempts=<n>
[selection-shadow][BR-174] outcome committed phase=t0_close|d1_settled|d3_settled|d5_settled run_id=<id> settled=<n> waited=<n> failed=<n>
[selection-shadow][BR-174] board expansion disabled reason_code=<typed-code> retryable=<bool>
```

The disabled banner is emitted once per unchanged capability state, not once per security. Raw logs
and the live-binding artifact are attached to the PR; secrets and account data are not.

SQLite release evidence runs against a copy opened query-only after graceful shutdown and includes:

```sql
PRAGMA foreign_key_check;
SELECT COUNT(*) AS invalid_terminal
FROM selection_samples
WHERE decision_kind NOT IN ('admitted', 'hard_rejected');
SELECT COUNT(*) AS invalid_rejection_matrix
FROM selection_samples s
LEFT JOIN (
  SELECT sample_key, COUNT(*) AS child_count, MIN(ordinal) AS min_ordinal,
         MAX(ordinal) AS max_ordinal
  FROM selection_rejections
  GROUP BY sample_key
) r ON r.sample_key = s.sample_key
WHERE (s.decision_kind = 'admitted' AND
       (s.rejection_count <> 0 OR COALESCE(r.child_count, 0) <> 0))
   OR (s.decision_kind = 'hard_rejected' AND
       (s.rejection_count <= 0 OR COALESCE(r.child_count, 0) <> s.rejection_count
        OR r.min_ordinal <> 0 OR r.max_ordinal <> s.rejection_count - 1));
SELECT COUNT(*) AS receipted_generation_samples
FROM selection_samples s
JOIN selection_v2_commit_receipts r
  ON r.subject_kind = 'generation_run' AND r.subject_id = s.generation_run_id;
SELECT s.decision_kind, o.phase, COUNT(*)
FROM selection_samples s
JOIN selection_sample_outcomes o ON o.sample_key = s.sample_key
JOIN selection_v2_commit_receipts sr
  ON sr.subject_kind = 'generation_run' AND sr.subject_id = s.generation_run_id
JOIN selection_v2_commit_receipts orc
  ON orc.subject_kind = 'outcome_run' AND orc.subject_id = o.outcome_run_id
GROUP BY s.decision_kind, o.phase
ORDER BY s.decision_kind, o.phase;
```

Expected `foreign_key_check`, `invalid_terminal` and `invalid_rejection_matrix` values are zero;
`receipted_generation_samples` must be greater than zero. Gate D additionally requires
the canary-created non-empty ingress/generation cohort, at least one verified provider-board
binding, a successful real `--first-binding` constituent probe and exact receipt/audit verification
above; an empty cohort or empty artifact binding list is failure, not success.

Coverage gates remain repository overall ≥80% and core trading/data links ≥95%.

## 14. Rollback

Rollback sets the existing `STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE=0` at the monitor owner. Missing
means enabled; `0` disables new generation and emits one operator-visible INFO/WARN startup banner;
any invalid value fails closed and emits an ERROR banner. The switch does not re-enable the old
opportunity path. A dedicated
settlement owner continues already committed schema-v2 outcome schedules, while append-only inbox,
attempt, sample, rejection, outcome, receipt and audit rows remain retained. Rollback must not
delete evidence, fabricate empty results or change account/order state.

The rollback switch does not disable outcome-claim recovery. Every partial/active claim must remain
excluded from new due work and, only while its immutable lineage validates, be recovered under the
fixed subject lock to an exact linked outcome receipt. Corrupt lineage remains unclosed and
integrity-fatal; rollback must not manufacture `failed_non_retryable` to clear it. Rollback never
deletes claim lock files, claim envelopes/manifests/receipts or provider attempt evidence and never
marks a claim closed by age. If an older behavioral binary cannot parse
`generation-stage-v3`, `outcome-claim-stage-v2`, `outcome-stage-v3`,
`outcome-market-request-v2`, `selection_samples_row.v2`,
`V2OutcomeClaimPrepared/V2OutcomeClaimCommitted` or enforce §7.13.1, it is not a valid rollback
binary after the first claim phase/receipt exists; disable new generation and roll forward the
settlement/recovery owner instead.

Git rollback is a scoped revert of BR-174 behavioral implementation commits. The standalone audit
enum/parser compatibility commit, including both outcome-claim audit phases, is permanent after
first production use and must remain in every
rollback binary. Schema tables remain readable; destructive database rollback is prohibited.

For the §7.14 schema amendment, restoring the complete rollback database is allowed only inside the
still-held global quiesce window and only while the fixed live database equals the recorded
post-exchange identity. The first write to **any** table, not merely the first amended selection
receipt, permanently closes that full-file rollback path. Reverting to code that cannot parse and
validate `RequestEvidencePreimage` or the amended `OutcomeStageInputPreimage` is then prohibited.
Disable new generation, retain the current live database, keep the settlement/recovery owner
running, and roll forward. The sealed complete pre-amendment database remains immutable audit
evidence; it is never attached to runtime queries, deleted, rewritten or merged by fabricating
missing request preimages.

The release/rollback checklist is blocking:

1. record the exact v1 and v2 golden-vector fixture hashes and every affected domain/payload-schema
   token from §7.14, plus the finalized design-document SHA-256 and the byte-identical replacement
   of every superseded frozen `DESIGN_SHA256`;
2. record release and rollback binary SHAs and prove each accepts only its declared schema revision;
3. validate the complete preserved v1 audit prefix and sealed full production database with the
   retained parser before any file exchange;
4. validate the complete amended v2 audit chain and reject mixed-version fixtures before enabling
   generation;
5. prove atomic restoration while global quiescence remains held and the live database has received
   no writes;
6. inject one post-exchange write into every safety-critical non-v2 category and prove full-file
   restoration is rejected, leaving only disable-new-generation plus roll-forward recovery.

## 15. PR evidence template

```text
Refs:
- spec: docs/superpowers/specs/2026-07-28-selection-evidence-closure-design.md

Data-Redlines:
- [2.1] no mock/default/fallback candidate evidence
- [2.2] missing provider/source times remain missing and explicit
- [2.3] full price/continuity/lifecycle/manual-confirmation gates
- [2.4] event, quote and daily freshness enforced
- [2.7] append-only source fact, attempt, sample/rejection/outcome/receipt/audit evidence
- [2.8] save/settle/report implementations operate on their real targets
- [2.10] BR-174 registered before implementation

OldModules:
- `src/opportunity/chain_mapper.rs:754-825` | reject after caller audit | fuzzy/Top-N acquisition
- `BoardDataGateway::memberships` | preserve | BR-170 reverse lookup
- `src/bin/monitor/main.rs` | adopt/reorder | receipt before projection/evaluation
- `src/bin/monitor/news_aggregator_init.rs` | adopt/split | raw acquisition before simhash
- `src/bin/monitor/selection_shadow.rs` | adopt/replace | v2 ingress/generation/outcome owner
- `src/bin/monitor/dryrun_report.rs` legacy backfill | reject/delete | remove hidden opportunity owner
- `src/selection/report.rs` | adopt/replace | BR-178 verified v2 query
- `src/database/selection.rs` | adopt/migrate | v1 physical freeze plus twelve v2 tables
- `src/selection/audit.rs` | adopt enum-only | preserve canonical v1 hash chain
- `src/bin/selection_live_probe.rs` | adopt/deepen | exact binding/constituent evidence
- `src/bin/selection_backtest.rs` | adopt/deepen | receipted prospective dual cohort
- `src/calendar.rs` mutable global | preserve other users/reject for v2 | immutable schedules
- formal BR-155/156/157 pipeline | adopt | governed production owner

Threshold-Proof:
- no new strategy threshold
- fixed 10,000 is the upstream provider contract maximum; equality fails closed because current
  upstream evidence has no total/truncated field

Business-Rules:
- BR-174
- BR-176
- BR-177
- BR-178

Rollback:
- set `STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE=0`; retain and settle immutable existing samples
- git revert the scoped implementation commits
```
