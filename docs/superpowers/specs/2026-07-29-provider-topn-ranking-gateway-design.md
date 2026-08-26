# Provider Top-N Ranking Gateway and R-09 Review Design

**Status:** Gate B implementation/cutover present; Gate B verification and Gate C/D pending; upstream pinned at `d7dfa3140919525f3280bed87136602a78fa17ad`
**Date:** 2026-07-29
**Rules:** BR-192, AGENTS §§2.1–2.4, 2.7, 2.8, 2.10

## 1. Intent

Restore a useful, evidence-bound post-close ranking report without reviving the
unproved complete-market ranking contract retired by BR-190. The upstream
contract is deliberately narrow: one exact Eastmoney response page for either
volume ratio or main net inflow. It proves the returned page, not complete
universe coverage.

This slice does not restore the intraday I-10 loop, BR-073/BR-150 virtual
buying, R-02 market breadth, or any claim that Top-N rows are a complete market
ranking.

## 2. Reproducible current-state evidence

Commands run against the current downstream worktree before this design:

```bash
rg -n -C 3 \
  "BR-190|post_close_main_flow|PostCloseMainFlowFact|ProviderTopN" \
  docs/business_rules.md src/data_gateway src/bin/monitor
```

Historical pre-design facts (superseded where §2.1 records the implemented
cutover):

- `CapitalDataGateway::post_close_main_flow` and
  `PostCloseMainFlowFact` have no consumer outside `capital.rs`.
- That dead path still consumes the old upstream `PostCloseFlow` contract and
  requires a batch `source_at`, which contradicts the new Provider Top-N
  evidence semantics.
- BR-190 runtime sites only emit the stable
  `provider_capability_not_live_admitted` marker; they do not fetch data.

The production `--review` call chain was traced with:

```bash
rg -n -C 5 \
  "run_strict_review_only_inner|dispatch_post_session_review|ReviewTask::ALL" \
  src/bin/monitor/main.rs src/bin/monitor/push_templates.rs \
  src/bin/monitor/review_batch.rs
```

Observed production chain:

```text
run_review_only
  -> run_strict_review_only_inner
  -> dispatch_post_session_review
  -> typed ReviewTask outcomes
  -> review transition audit
```

The legacy inline review is not the production `--review` owner and is not an
acceptable integration target.

### 2.1 Implementation reconciliation (2026-07-30)

The following design targets now exist in the worktree:

- `src/durable_delivery/{model,schema,coordinator,tests}.rs` implements the
  durable state machine, policy catalog, reservations, leases/fences, typed
  results, immutable outbox, manual boundary and all-date reconciliation;
- `src/event/durable_delivery_append.rs` is the exact-byte immutable append
  adapter;
- `src/bin/monitor/durable_delivery_runtime.rs` composes the coordinator,
  authoritative sink, producer-readiness barrier and typed
  `CountedDeliveryBinding`;
- `src/data_gateway/capital.rs`, `src/bin/monitor/push_templates.rs` and
  `src/bin/monitor/review_batch.rs` implement the source-limited R-09
  acquisition, binding, delivery and optional BR-140 hydration path;
- `src/bin/monitor/notify.rs` rejects every counted generic-governor call with
  `counted_binding_required`; legacy v14 dedup commit/rollback also rejects
  counted kinds.

The former standalone daily-report router is not part of the current
architecture. Its typed sub-kind vocabulary and registered policy mapping
live in `notify.rs` and `durable_delivery_runtime.rs`.

This reconciliation is code-presence evidence, not a Gate B/C/D pass.
Cross-process acceptance tests named later in this document, full validation,
coverage, controlled live evidence and independent gate review remain open.

## 3. Upstream contract

The pinned upstream revision must export:

- `magic_market_core::ProviderTopNRankingRequest`;
- `magic_market_core::ProviderTopNRankingEntry`;
- `magic_market_core::ProviderTopNRankingCapabilities`;
- the provider-neutral acquisition trait
  `magic_market_core::ProviderTopNRankings`;
- `magic_eastmoney_rs::EastmoneyClient::provider_top_n_a_share_request`,
  which is the only admitted constructor for Eastmoney's canonical A-share
  filter identity;
- the non-forgeable concrete route
  `magic_market_composition::EastmoneyProviderTopNRankingRouter`.

`ProviderTopNRankings` is an acquisition seam, not a downstream admission
router. Production downstream code constructs
`EastmoneyProviderTopNRankingRouter::new()` with zero arguments. The
composition crate creates its production `EastmoneyClient` internally and
exposes no public client injection or generic source-registration path.

The exact immutable upstream merge revision is
`d7dfa3140919525f3280bed87136602a78fa17ad`, merged by upstream PR #4 after
the independent 0C/0I review, complete local release preflight and green
GitHub audit/checks/coverage jobs. Downstream `Cargo.toml` now pins all fourteen
Magic dependencies, including `magic-market-composition`, to that exact
revision and `Cargo.lock` resolves the same immutable commit. The previous
baseline `660902ff93a07f18367dc16879cf67732accd25a` does not contain this API
and is not an admitted downstream dependency. The retired `PostCloseFlow`
consumer symbols have no current source references. R-02, I-10, BR-073 and
BR-150 remain disabled throughout.

The following remain false and are not inferred from Top-N admission:

- `MarketRankingCapabilities`;
- `SignalCapabilities.market_rankings`;
- complete-universe coverage.

For each metric the provider and Router must prove:

1. current Shanghai trading date and capture after 15:35;
2. one exact response page with `min(requested_limit, declared_total)` rows;
3. unique A-share instrument identities;
4. exact provider response order, represented as `source_order_ordinal`;
5. non-increasing values in provider order;
6. every row's `latest_trading_date` equals the requested date;
7. identical request `filter_identity`, declared total and inspected count;
8. complete batch/record provider, batch ID and observation evidence;
9. absent batch and record `source_at`.

## 4. Downstream data flow

```text
R-09 strict review task
  -> CapitalDataGateway::provider_top_n(metric, review_date, 20)
  -> spawn_blocking
     -> EastmoneyClient::provider_top_n_a_share_request(...)
     -> EastmoneyProviderTopNRankingRouter::new()
     -> EastmoneyProviderTopNRankingRouter::route(...)
     -> Core and concrete-route batch validation
     -> downstream projection validation
     -> durable acquisition audit
  -> require both VolumeRatio and MainNetInflow batches
  -> render source-limited report without reordering
  -> DurableDeliveryCoordinator::deliver(canonical DeliveryEnvelope)
     -> one SQLite BEGIN IMMEDIATE prepare transaction
        -> immutable replayable binding
        -> cooldown reservation
        -> shared daily-budget slot
     -> one fenced authoritative remote sink attempt
     -> typed sink port: Accepted | Rejected | Uncertain
     -> one local result transaction
     -> replay frozen delivery audit and disposition-specific R-09 transition
production startup gate
  -> DurableDeliveryCoordinator::reconcile_all_pending()
     -> enumerate every unresolved business date without a date predicate
     -> finish all locally progressable audit/transition/recovery work
        without provider or sink calls
     -> while producers remain frozen, resume only returned durable deliverables
        through resume_deliverable, then rerun all-date reconcile
     -> open producer readiness only after no startup deliverable/local pending
        remains; manual/non-expired-foreign boundaries stay explicitly reported
  -> DurableDeliveryCoordinator::inspect_pending_for_date(review_date)
     -> read-only diagnostic only; cannot mutate or satisfy startup
  -> DurableDeliveryCoordinator::resume_deliverable(decision_identity)
     -> use the stored envelope to resume Reserved/authorized Rejected work
     -> provider_calls=0; at most one newly fenced authoritative sink call
```

The request helper is a static constructor and does not authorize downstream
wire/filter construction. The zero-argument composition Router is created,
used and dropped inside `spawn_blocking`, so its internally owned provider
client and blocking runtime are never dropped in Tokio's async context.
Downstream tests inject already admitted `GatewayBatch<ProviderTopNFact>`
fixtures only at a private R-09 loader seam; they do not inject or replace the
upstream client, source or Router.

`DurableDeliveryCoordinator` now replaces the earlier R-09-private
`ProviderTopNDeliveryCoordinator` design and is the sole delivery-state owner
for every counted PushKind. R-09 is a consumer of that common owner, not a
license for a second budget, cooldown or transition journal. The current
library interface is intentionally small:

- `prepare(envelope, append, now) -> PrepareOutcome`;
- `resume_deliverable(decision_identity, sink, append, now) -> ResumeOutcome`;
- `reconcile_all_pending(append, now) -> ReconcileSummary`;
- `inspect_pending_for_date(business_date) -> Vec<decision_identity>`;
- `resolve_uncertain(command, append, now) -> DecisionState`.

The monitor adapter's `deliver_envelope` composes prepare, locking,
reservation, fenced sink invocation, typed result persistence, delivery audit
and any bound task transition. `reconcile_all_pending` has no date argument:
it enumerates every
unresolved date in the physical store, replays stored canonical bytes only,
and never calls a provider or sink. `inspect_pending_for_date` is a read-only
operator diagnostic and is forbidden from changing a decision, lease,
reservation, audit outbox or startup-readiness flag. A date-scoped query can
therefore never substitute for the all-date startup gate.
`resume_deliverable` is the sole cross-restart path
that may call the authoritative sink for a stored `Reserved` decision or an
explicitly retry-authorized `RejectedDurable` decision; it renders nothing and
performs no provider acquisition.
`resolve_uncertain` is the only write interface for an authorized human
resolution and requires operator identity plus evidence. Every outcome has a
generic immutable delivery disposition; a BR-140 task transition exists only
when the envelope carries an admitted task binding. The monitor's
`DurableDispatchEvidence` therefore contains delivery state plus an optional
`ScheduleHydration`, never an invented task result for a non-task caller.
Callers may perform only launch/mode/acquisition checks that end before a
counted `DeliveryEnvelope` exists, such as physical test isolation or an
unavailable producer. Once a canonical counted envelope exists, all content
and delivery governance enters this seam; every definite denial follows the
durable pre-sink path in §6.2. Callers may not reserve or release
cooldown/budget, call a counted sink, publish its delivery audit, or append its
delivery-linked task transition themselves.

The sink is an implemented internal port whose result is exactly:

```text
Accepted(TypedReceipt)
Rejected(DefiniteRejection)
Uncertain(UncertainReason)
```

`TypedReceipt` retains the real channel, provider, provider message ID,
platform message ID when present, provider acceptance time, locally measured
latency, attempt identity and raw-receipt hash. `Rejected` is permitted only
when the adapter can prove the remote sink did not accept the message.
Timeouts, connection loss after write, malformed responses, worker
cancellation and transport errors are `Uncertain`, never definite rejection.
The current `push_wechat(...) -> bool` and
`push_l6::SinkResult::{Ok, Err(String)}` adapters lose this distinction and
receipt data; every counted caller must be upgraded before activation. They
must not be wrapped by mapping `false`/`Err` to `Rejected`.

The L6 fan-out authority is also fixed for Gate B. Exactly one configured
remote `AuthoritativeSinkPort` may determine the disposition of a counted
decision. The existing `ConsoleSink` becomes an observer: it may render a
redacted view of a durable state/result, but it is not a delivery attempt,
cannot return acceptance, consumes no budget/cooldown, and cannot turn an
authoritative remote failure into success. Zero or more than one configured
authoritative remote adapter is a definite local configuration rejection
before `AttemptInFlight`, with zero remote calls. Magiclaw/Feishu is the one
authoritative remote adapter at cutover and must return the typed result above
with its real receipt fields. A future multi-remote policy would require a
separate per-sink attempt/result design and is outside BR-192; it must not be
smuggled through the observer fan-out.

## 5. Projection

`ProviderTopNFact` retains:

- metric kind;
- `source_order_ordinal`;
- complete typed `InstrumentId` (exchange, asset class and code) and name;
- finite value and exact unit;
- `latest_trading_date`;
- `filter_identity`;
- `provider_declared_total`;
- `inspected_row_count`.

The surrounding `GatewayBatch` retains provider, source, observation time and
batch ID. `source_at` remains `None`.

Renderers may display only the code, but validation, uniqueness and audit
identity always use the complete typed instrument identity.

No close, change percent, ratio percent, market breadth, provider tie rank,
trade action, score, or forecast is inferred.

## 6. R-09 business behavior

- Request exactly 20 rows for each admitted metric.
- Preserve upstream order; do not sort again.
- Both complete, non-empty batches are mandatory for one report. Acquisition
  audit is written independently for each request, but rendering, report
  binding and delivery are atomic across the pair.
- Any failed, partial, stale, conflicting or unsupported batch makes R-09
  `Failed`; it is never converted to `NoData`, and one successful metric is
  never rendered or delivered alone.
- A successful report says:
  `Eastmoney 单响应 TopN；不代表全市场完整排序`.
- Volume ratio is rendered as a multiple; main net inflow is rendered in
  yuan-derived Chinese display units while the stored fact remains yuan.
- The two batch IDs and acquisition outcomes are audited; raw account or
  holding data is not involved.
- Before delivery, R-09 must produce one canonical
  `ProviderTopNReportBinding` inside the generic `DeliveryEnvelope`. It
  contains both metric/request fingerprints, both original batch IDs,
  `observed_at`/`as_of`, the ordered typed-instrument/value projection and its
  hash, replayable rendered content bytes and SHA-256,
  `ReviewProviderTopN` template ID, delivery subject identity and the complete
  canonical BR-140 task-transition basis. Every later result first freezes the
  mandatory generic delivery disposition; because R-09 has this admitted task
  binding, the same transaction also freezes its optional BR-140 task
  transition without reacquisition or current-time reconstruction. The
  replayable projection and
  content remain only in the coordinator's authorized SQLite binding;
  transition and delivery observation logs retain hashes and counts.
  `deliver` makes this binding, its cooldown reservation and its budget slot
  durable in one prepare transaction before any sink call.
- For the generic decision formula in §10.2, R-09's
  `schedule_occurrence_identity` is the review-task identity and its
  `source_evidence_fingerprint` is the fixed-metric-order hash of
  `review_date`, both request fingerprints, both batch IDs and the ordered
  projection hash. The final decision identity additionally binds the
  registered policy version, subject and rendered-content hash exactly as
  §10.2 specifies.
- The deterministic review-task identity remains the existing identity for
  `(review_date, R-09)`. One shared helper derives it for the pre-delivery
  binding. Each generic disposition derives an identity independent of that
  task; each corresponding BR-140 task transition derives its own append
  identity from the stored task identity using §6.3. Neither algorithm may be
  copied into the dispatcher.
- The delivery governor receives the report decision identity through a
  dedicated non-security delivery-subject parameter. `SignalEvent.code`
  remains `None`; the report identity must not be stored or validated as a
  stock/board code. The delivery audit's redacted subject hash must therefore
  join back to the binding. No raw instrument list is copied into transition
  or delivery logs.
- The dedicated delivery subject is an audit/join identity only. It must not
  replace the Global cooldown key with a per-decision key; otherwise two
  different captures on one date could both pass the one-report-per-day gate.
- Delivery uses an independent daily PushKind, so it cannot consume or block
  R-02's future complete-market cooldown key.
- `ReviewProviderTopN` has a Global `BusinessDateOnce` reservation for the
  validated business date, with a nominal catalog duration of 86,400 seconds
  that is never interpreted as rolling expiry, and is exempt from the shared
  intraday daily push budget under BR-237.

### 6.1 Canonical envelope and one physical store

The implemented coordinator uses exactly one dedicated SQLite database,
`data/durable_delivery.sqlite3`, for all production counted PushKinds. A test
process uses a path-safe `TEST_CODE` namespace rooted at
`data/test/<TEST_CODE>/durable_delivery.sqlite3` plus a separate immutable
audit root. Opening the production path in test mode or a test path in
production fails before any sink call. No delivery transaction spans SQLite
plus JSONL, the main application database, or another connection.

Every coordinator process opens this database with WAL, `foreign_keys=ON`,
`synchronous=FULL` and `busy_timeout=5000`. One state change uses one
connection and one `BEGIN IMMEDIATE ... COMMIT`; SQLite's writer lock is the
cross-process mutex. Busy/commit failure is explicit and cannot fall back to
the current process-local atomics or mutex tables.

The schema implemented in `src/durable_delivery/schema.rs` is governed by the
following contract:

```text
delivery_decisions(
  decision_identity TEXT PRIMARY KEY,
  business_date TEXT NOT NULL,
  push_kind TEXT NOT NULL,
  sub_kind TEXT NOT NULL,
  cooldown_scope TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN (
    'Reserved','AttemptInFlight',
    'AcceptedAuditPending','AcceptedTaskTransitionPending','Delivered',
    'RejectedAuditPending','RejectedTaskTransitionPending','RejectedDurable',
    'UncertainAuditPending','UncertainTaskTransitionPending',
    'UncertainManualReview',
    'ManualRejectedAuditPending','ManualRejectedTaskTransitionPending',
    'ManualResolvedRejected')),
  envelope_version INTEGER NOT NULL,
  envelope_canonical BLOB NOT NULL,
  envelope_sha256 TEXT NOT NULL,
  task_binding_present INTEGER NOT NULL CHECK(task_binding_present IN (0,1)),
  transition_basis_canonical BLOB,
  transition_basis_sha256 TEXT,
  reservation_generation INTEGER NOT NULL CHECK(reservation_generation >= 0),
  current_budget_reservation_identity TEXT,
  current_cooldown_reservation_identity TEXT,
  current_attempt_identity TEXT,
  current_disposition_identity TEXT,
  fence_generation INTEGER NOT NULL CHECK(fence_generation >= 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
)

delivery_policy_catalog(
  push_kind TEXT NOT NULL,
  sub_kind TEXT NOT NULL,
  cooldown_scope TEXT NOT NULL,
  base_cooldown_secs INTEGER,
  override_cooldown_secs INTEGER,
  window_mode TEXT NOT NULL CHECK(window_mode IN
    ('None','Rolling','BusinessDateOnce')),
  counts_against_daily_budget INTEGER NOT NULL CHECK(
    counts_against_daily_budget=1),
  policy_version INTEGER NOT NULL,
  PRIMARY KEY(push_kind,sub_kind)
)

cooldown_reservations(
  cooldown_reservation_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
  attempt_identity TEXT,
  business_date TEXT NOT NULL,
  push_kind TEXT NOT NULL,
  sub_kind TEXT NOT NULL,
  cooldown_scope TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  policy_version INTEGER NOT NULL,
  effective_cooldown_secs INTEGER,
  window_mode TEXT NOT NULL CHECK(window_mode IN
    ('Rolling','BusinessDateOnce')),
  reserved_at TEXT NOT NULL,
  accepted_at TEXT,
  blocked_until TEXT,
  released_at TEXT,
  state TEXT NOT NULL CHECK(state IN
    ('Reserved','Accepted','Uncertain','Released')),
  UNIQUE(decision_identity,reservation_generation)
)

cooldown_heads(
  push_kind TEXT NOT NULL,
  sub_kind TEXT NOT NULL,
  cooldown_scope TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  current_reservation_identity TEXT REFERENCES cooldown_reservations,
  state TEXT NOT NULL CHECK(state IN
    ('Reserved','Accepted','Uncertain','Released')),
  blocked_until TEXT,
  version INTEGER NOT NULL,
  PRIMARY KEY(push_kind,sub_kind,cooldown_scope,scope_key)
)

business_date_once_claims(
  business_date TEXT NOT NULL,
  push_kind TEXT NOT NULL,
  sub_kind TEXT NOT NULL,
  scope_key TEXT NOT NULL,
  decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions,
  policy_version INTEGER NOT NULL,
  claimed_at TEXT NOT NULL,
  audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox,
  PRIMARY KEY(business_date,push_kind,sub_kind,scope_key)
)

daily_budget_reservations(
  budget_reservation_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
  attempt_identity TEXT,
  business_date TEXT NOT NULL,
  slot_no INTEGER NOT NULL CHECK(slot_no BETWEEN 1 AND 30),
  reserved_at TEXT NOT NULL,
  accepted_at TEXT,
  released_at TEXT,
  state TEXT NOT NULL CHECK(state IN
    ('Reserved','Accepted','Uncertain','Released')),
  UNIQUE(decision_identity,reservation_generation)
)

delivery_attempts(
  attempt_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  attempt_no INTEGER NOT NULL CHECK(attempt_no > 0),
  owner_instance_identity TEXT NOT NULL,
  fence_token INTEGER NOT NULL CHECK(fence_token > 0),
  lease_expires_at TEXT NOT NULL,
  lease_heartbeat_at TEXT NOT NULL,
  fence_revoked_at TEXT,
  state TEXT NOT NULL CHECK(state IN
    ('AttemptInFlight','Accepted','Rejected','Uncertain')),
  started_at TEXT NOT NULL,
  UNIQUE(decision_identity,attempt_no),
  UNIQUE(decision_identity,fence_token)
)

sink_results(
  result_event_identity TEXT PRIMARY KEY,
  attempt_identity TEXT NOT NULL REFERENCES delivery_attempts,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  result_kind TEXT NOT NULL CHECK(result_kind IN
    ('Accepted','Rejected','Uncertain')),
  observed_at TEXT NOT NULL,
  fence_token INTEGER NOT NULL,
  authoritative_for_state INTEGER NOT NULL CHECK(
    authoritative_for_state IN (0,1)),
  late_after_fence INTEGER NOT NULL CHECK(late_after_fence IN (0,1)),
  authority_audit_identity TEXT NOT NULL UNIQUE
    REFERENCES immutable_audit_outbox,
  late_receipt_audit_identity TEXT UNIQUE
    REFERENCES immutable_audit_outbox,
  result_canonical BLOB NOT NULL,
  result_sha256 TEXT NOT NULL,
  channel TEXT,
  provider TEXT,
  message_id TEXT,
  platform_message_id TEXT,
  accepted_at TEXT,
  latency_ms INTEGER,
  frozen_delivery_audit_canonical BLOB,
  frozen_delivery_audit_sha256 TEXT,
  delivery_audit_ref TEXT,
  UNIQUE(attempt_identity,result_sha256),
  CHECK(late_after_fence=0 OR late_receipt_audit_identity IS NOT NULL)
)

manual_resolutions(
  resolution_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL UNIQUE REFERENCES delivery_decisions,
  attempt_identity TEXT NOT NULL REFERENCES delivery_attempts,
  disposition TEXT NOT NULL CHECK(disposition IN ('Accepted','Rejected')),
  operator_identity TEXT NOT NULL,
  reason TEXT NOT NULL,
  evidence_canonical BLOB NOT NULL,
  evidence_sha256 TEXT NOT NULL,
  receipt_canonical BLOB,
  frozen_delivery_audit_canonical BLOB,
  frozen_delivery_audit_sha256 TEXT,
  immutable_audit_ref TEXT NOT NULL,
  resolved_at TEXT NOT NULL
)

delivery_disposition_payloads(
  disposition_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  attempt_identity TEXT REFERENCES delivery_attempts,
  resolution_identity TEXT REFERENCES manual_resolutions,
  denial_identity TEXT,
  disposition TEXT NOT NULL CHECK(disposition IN
    ('Accepted','Rejected','Uncertain','ManualAccepted','ManualRejected')),
  disposition_canonical BLOB NOT NULL,
  disposition_sha256 TEXT NOT NULL,
  append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
  immutable_audit_ref TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(decision_identity,disposition_identity)
)

task_transition_payloads(
  transition_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  disposition_identity TEXT NOT NULL REFERENCES delivery_disposition_payloads,
  task_binding_sha256 TEXT NOT NULL,
  transition_canonical BLOB NOT NULL,
  transition_sha256 TEXT NOT NULL,
  append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
  immutable_audit_ref TEXT,
  UNIQUE(decision_identity,transition_identity)
)

immutable_audit_outbox(
  audit_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  attempt_identity TEXT REFERENCES delivery_attempts,
  audit_kind TEXT NOT NULL CHECK(audit_kind IN (
    'DecisionStateChanged','LeaseGranted','LeaseHeartbeat',
    'FenceRevoked','RecoveryClassified','SinkResultAuthorityClassified',
    'LateReceiptObserved','BudgetReservationChanged',
    'CooldownReservationChanged','BusinessDateOnceClaimed',
    'DecisionIdentityConflict')),
  predecessor_audit_identity TEXT REFERENCES immutable_audit_outbox,
  audit_canonical BLOB NOT NULL,
  audit_sha256 TEXT NOT NULL,
  append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
  immutable_audit_ref TEXT,
  created_at TEXT NOT NULL
)

delivery_state_events(
  event_seq INTEGER PRIMARY KEY AUTOINCREMENT,
  state_event_identity TEXT NOT NULL UNIQUE,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  from_state TEXT,
  to_state TEXT NOT NULL,
  actor TEXT NOT NULL,
  operator_identity TEXT,
  evidence_canonical BLOB NOT NULL,
  evidence_sha256 TEXT NOT NULL,
  audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox
)

delivery_attempt_events(
  attempt_event_identity TEXT PRIMARY KEY,
  attempt_identity TEXT NOT NULL REFERENCES delivery_attempts,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  event_kind TEXT NOT NULL CHECK(event_kind IN (
    'LeaseGranted','LeaseHeartbeat','FenceRevoked',
    'RecoveryClassified','SinkResultAuthorityClassified',
    'LateReceiptObserved')),
  event_canonical BLOB NOT NULL,
  event_sha256 TEXT NOT NULL,
  audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox
)

cooldown_reservation_events(
  event_identity TEXT PRIMARY KEY,
  cooldown_reservation_identity TEXT NOT NULL REFERENCES cooldown_reservations,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  from_state TEXT,
  to_state TEXT NOT NULL,
  event_canonical BLOB NOT NULL,
  event_sha256 TEXT NOT NULL,
  audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox
)

daily_budget_reservation_events(
  event_identity TEXT PRIMARY KEY,
  budget_reservation_identity TEXT NOT NULL REFERENCES daily_budget_reservations,
  decision_identity TEXT NOT NULL REFERENCES delivery_decisions,
  from_state TEXT,
  to_state TEXT NOT NULL,
  event_canonical BLOB NOT NULL,
  event_sha256 TEXT NOT NULL,
  audit_identity TEXT NOT NULL UNIQUE REFERENCES immutable_audit_outbox
)
```

The schema also requires these SQLite partial unique constraints:

```sql
CREATE UNIQUE INDEX uq_active_budget_per_decision
ON daily_budget_reservations(decision_identity)
WHERE state IN ('Reserved','Accepted','Uncertain');

CREATE UNIQUE INDEX uq_active_budget_slot
ON daily_budget_reservations(business_date,slot_no)
WHERE state IN ('Reserved','Accepted','Uncertain');

CREATE UNIQUE INDEX uq_budget_attempt
ON daily_budget_reservations(attempt_identity)
WHERE attempt_identity IS NOT NULL;
```

`delivery_policy_catalog` is the coordinator's closed, versioned metadata
catalog for all fifteen counted PushKinds and all eighteen
`(push_kind, sub_kind)` policy rows. The row count exceeds the kind count
because `DailyReport` has four distinct rows: `NONE`, `FactorIC`,
`SectorTier` and `CapitalVerify`. The caller supplies no free-form scope,
sub-kind or cooldown override. Prepare selects the exact catalog row inside
`BEGIN IMMEDIATE` and rejects a canonical envelope whose embedded policy
version or derived fields differ.

`task_binding_present=0` requires both transition-basis columns to be null;
`task_binding_present=1` requires both to be non-null and hash-matched.
`delivery_disposition_payloads` is mandatory for every authoritative result,
pre-sink rejection and manual resolution. `task_transition_payloads` is
optional and may be inserted only for a hash-matched task binding. A generic
disposition must never be represented only by a BR-140 transition, and a
non-task envelope must never receive a synthetic task identity or
`ScheduleHydration`.

`cooldown_heads` are used only for `Rolling` policy rows.
`cooldown_reservations` are mutable operational projections for both Rolling
and BusinessDateOnce evidence. The latter is one retained row per decision
reservation generation, not immutable history: its state may move exactly
`Reserved -> Accepted|Uncertain|Released` and its optional `attempt_identity`
may be populated once by compare-and-set when the generation starts its
attempt. It is never deleted or reused after `Released`.
`cooldown_reservation_events` plus their mandatory
`immutable_audit_outbox` entries are the append-only history. For rolling
windows, prepare reads the head under the same write transaction: `Reserved`
or `Uncertain` always conflicts, and `Accepted` conflicts while
`blocked_until > admission_at`; only an expired accepted head or a released
head may point to a newly inserted generation. Acceptance anchors
`blocked_until = accepted_at + effective_cooldown_secs`; a definite rejection
or proved no-sink outcome releases the current projection and appends an
event. A BusinessDateOnce admission never reads or advances
`cooldown_heads`; otherwise the prior date's accepted Global head would
incorrectly block the next validated business date. For Global day-scoped
R-09, `BusinessDateOnce` instead owns one durable, immutable
`business_date_once_claims` mapping:

```text
(business_date,push_kind,sub_kind,scope_key) -> decision_identity
```

Prepare evaluates the validated calendar date and all definite local
preconditions under the same `BEGIN IMMEDIATE` transaction before slot
allocation. If the key is absent and admission can reserve, it inserts the
claim, its `BusinessDateOnceClaimed` audit outbox row and the generation-one
reservations atomically. If the key exists with the byte-identical decision,
idempotent replay is allowed and an explicitly retry-authorized
`RejectedDurable` decision may allocate a new reservation generation. If it
exists with any different decision identity, the newcomer follows the durable
no-reservation rejection path with zero sink calls. The claim is never updated,
deleted or released: a definite rejection may release its cooldown/budget
generation, but the same-date claim remains. Only a different, validated next
business date has a different primary key and may create a new claim. Elapsed
wall-clock time, a `Released` projection, process restart or a new evidence
capture cannot admit a second decision for the claimed date.

Every `business_date` and daily-budget reset boundary is the validated
Asia/Shanghai calendar date, not UTC epoch-day arithmetic.
`daily_budget_reservations` is likewise a retained mutable projection per
reservation generation. It has:

- a partial unique index on `decision_identity` for
  `Reserved|Accepted|Uncertain`, so one decision cannot hold two active slots;
- a partial unique index on `(business_date,slot_no)` for
  `Reserved|Accepted|Uncertain`, so two decisions cannot own one active slot;
- a one-way compare-and-set from null `attempt_identity` to the attempt
  created for that generation;
- no delete or row reuse after `Released`.

Every re-admission of a retry-authorized `RejectedDurable` increments
`reservation_generation` and inserts new cooldown/budget identities. Released
rows remain queryable; only their slot number may be assigned to a new row.
The append-only `daily_budget_reservation_events` and immutable audit outbox,
not the mutable row itself, are the five-year history. The coordinator
allocates only slots 1 through 30.
`delivery_attempts` are keyed by attempt and `sink_results` are append-only
result observations under that attempt. This preserves an initial
uncertain/rejected observation and any later fenced callback without
overwriting either. A definite rejection can be preserved while a later retry
reuses the exact decision and creates a new monotonically numbered attempt.
Manual evidence is similarly append-only in `manual_resolutions`; it never
replaces the original `Uncertain` sink result. Every delivery disposition is
frozen in `delivery_disposition_payloads`; every task-bound BR-140 transition
is separately frozen in `task_transition_payloads`. Their returned hash-chain
references are filled by idempotent compare-and-set from `Pending` to
`Appended`, never by rebuilding payload bytes.

Updates/deletes of canonical envelope bytes, result bytes, disposition bytes,
task-transition bytes, `business_date_once_claims` or append-only event rows
are blocked by SQLite triggers. Every decision-state mutation inserts
`delivery_state_events` and a
matching pending immutable-audit outbox row in the same transaction. Lease
grant/heartbeat, fence revocation, expired-lease recovery classification,
sink-result fence/authority classification and every late receipt do the same
through distinct `delivery_attempt_events`; cooldown, BusinessDateOnce claim
and budget mutations do so through their event/claim rows. Thus the authority
decision
`authoritative_for_state=0|1` is itself frozen, not inferred later from the
current projection. The outbox replays exact canonical bytes to the
tamper-resistant audit hash chain and retains the reference for at least five
years under AGENTS §2.7.
Every outbox row names the immediately preceding critical audit for that
decision when one exists. A transaction containing several events links them
in semantic order. Reconcile appends only rows whose predecessor is already
`Appended`; missing predecessors or cycles fail closed rather than reordering
lease, fence, authority or state facts.

An operational transaction may commit with its audit outbox row `Pending`, but
no terminal outcome, task hydration, reservation release reuse, recovered
attempt decision or late-receipt manual resolution may be exposed until every
audit row required by that transition is `Appended`. `reconcile_all_pending`
appends/verifies those bytes and compare-and-sets the immutable references;
identity-equal/byte-different replay is a fatal conflict. SQLite is
operational authority, not a claim that ordinary WAL storage itself is
tamper-resistant.

`DeliveryEnvelope` canonical bytes include the rendered body and hash,
decision identity, template ID, PushKind, business date, cooldown scope/key,
delivery subject, original provider `observed_at`/`as_of`, original batch IDs
the exact policy-catalog version, canonical sub-kind, retry authorization and
complete BR-140 transition basis when one exists. R-09 embeds its complete
`ProviderTopNReportBinding`. That basis contains the original review task
identity, review date, source/batch identities and stable outcome mapping
needed to freeze any disposition; it is not a placeholder transition.
Reconcile never renders again and never substitutes current time, current
batch IDs or a new task-transition basis.

### 6.2 Prepare and sink-result transaction boundaries

`deliver` first validates and hashes a complete canonical envelope, then uses
one `BEGIN IMMEDIATE` admission transaction. Business denial is not a
transaction rollback. The transaction has exactly one of three outcomes:

1. **Idempotent replay.** If `decision_identity` already exists, the stored
   envelope version/bytes/hash, policy version, derived scope/sub-kind and
   optional task-binding presence/hash must all match byte-for-byte. A match
   returns the persisted delivery state and optional persisted
   `ScheduleHydration` without allocating, denying or calling a sink. A
   mismatch leaves the existing decision untouched, inserts a canonical
   `DecisionIdentityConflict` audit-outbox event containing both hashes, and
   fails closed after that audit is appended. It is never treated as dedup,
   retry or a new decision.
2. **No-reservation rejection.** For a new identity, an invalid registered
   policy projection, cooldown conflict, a `BusinessDateOnce` key already
   claimed by a different decision, full daily budget, zero/multiple
   authoritative sinks, or other definite local pre-sink denial atomically
   inserts the immutable envelope and a decision in `RejectedAuditPending`
   with `reservation_generation=0` and null budget/cooldown/attempt
   identities. The same transaction derives a stable
   `denial_identity = SHA256("delivery-pre-sink-denial-v1",
   decision_identity,envelope_sha256,policy_version,denial_kind,
   denial_evidence_sha256)`, freezes a mandatory generic `Rejected`
   disposition, freezes an optional task transition only when the envelope
   has a valid task binding, and appends matching decision-state/audit-outbox
   events. It commits with `sink_calls=0`. Reconcile appends the generic
   disposition first, the optional task transition second, then exposes
   `RejectedDurable` and optional hydration. Retry authorization is a field of
   the frozen disposition; it cannot be inferred from the denial text.
3. **Admitted reservation.** The transaction selects the exact
   `(push_kind,sub_kind)` catalog row, derives and verifies scope/cooldown,
   checks the current cooldown head for a Rolling policy or inserts/verifies
   the immutable same-decision claim for `BusinessDateOnce` as described in
   §6.1. It increments
   `reservation_generation`, inserts new cooldown and budget projection rows
   plus their append-only events/audit outbox, allocates one active slot, and
   inserts the decision/state event as `Reserved`. A no-cooldown kind omits
   only the cooldown row; decision idempotency and budget admission still
   apply. A `BusinessDateOnce` claim is never omitted or released.

Only a storage/commit failure leaves no new durable decision; callers may
retry the same bytes. Policy, cooldown, budget and authoritative-sink denials
must use the atomic no-reservation rejection branch and may not be described
as “rollback whole prepare.” An identity-equal/byte-different envelope is the
audited conflict branch, not a rejected replacement. Successful prepare is
exactly `Reserved`: binding, applicable reservation generation and budget slot
are durable together. There is no `BindingDurable` state with contradictory
“no slot held” semantics.

A second transaction first revalidates the exact authoritative-sink
configuration snapshot. If cardinality is no longer one, it inserts no attempt
and atomically changes `Reserved -> RejectedAuditPending`, freezes a generic
local `Rejected` disposition plus optional task transition, and releases the
active generation with audit events. Otherwise it changes `Reserved` to
`AttemptInFlight`, increments `delivery_decisions.fence_generation`, and
inserts one immutable `delivery_attempts` row with the process-unique
`owner_instance_identity`, new fence token and lease deadline immediately
before the remote call. It also compare-and-sets that generation's
budget/cooldown rows from null `attempt_identity` to the new attempt and
freezes lease/state/reservation audit events. A retry after definite rejection
first inserts a new reservation generation and then a new attempt number/fence
token under the same decision; it never overwrites a prior result or Released
reservation. The remote call is never inside a SQLite transaction.

The implemented attempt lease is
`CoordinatorConfig::attempt_lease_secs=120`; configuration validation rejects
values outside 30 through 900 seconds. It is not currently exposed as a
`config/strategy.toml` field. Any later threshold exposure must cite this
section and satisfy AGENTS §2.9 bidirectionally.
A process instance identity is a fresh 128-bit random value
created once at startup and never reused. While awaiting the authoritative
remote call, the owner renews `lease_expires_at` by compare-and-set on
`decision_identity`, `AttemptInFlight`, `current_attempt_identity`,
`owner_instance_identity` and `fence_token`. Lease-renewal failure does not
authorize another sink call.

Any process may contend for expired-attempt recovery, but `BEGIN IMMEDIATE`
and the compare-and-set make exactly one process the recovery winner. Recovery
is forbidden while `lease_expires_at` is in the future. The winning
transaction rechecks the same attempt/fence, increments
`fence_generation` to revoke the old token, records `fence_revoked_at`, freezes
the mandatory generic `Uncertain` disposition and an optional task transition,
enters `UncertainAuditPending`, and inserts pending state, fence-revocation and
recovery-classification hash-chain audit events; it never calls the sink. The
decision cannot advance to manual review until those audit events and the
generic disposition are appended.

An original remote call may finish after another process revoked its token.
Its result transaction must compare the current state, attempt identity and
fence token before changing any decision or reservation:

- if the token is still current, the typed result is authoritative and follows
  the normal rules below; the authority determination itself is frozen in the
  same transaction;
- if the token was revoked, the old owner may append the real result as
  `authoritative_for_state=0, late_after_fence=1`, but it cannot change state,
  release reservations, append a terminal transition, or invoke the sink
  again. The authority classification and the late-receipt observation are two
  independently identified canonical events with two independently injectable
  pending five-year hash-chain audit outbox rows. Failure/appending of one
  cannot imply, satisfy or suppress the other; the inserted result references
  both;
- a late `Accepted(TypedReceipt)` remains visible as immutable acceptance
  evidence while the decision stays uncertain. Only
  `resolve_uncertain(Accepted)` may consume that exact receipt/hash and settle
  it. A late `Rejected` or `Uncertain` observation likewise cannot release the
  charged slot.

- `Accepted(receipt)`: while the process retains the typed receipt, it retries
  only one local `BEGIN IMMEDIATE` result transaction. That transaction stores
  the complete receipt, `accepted_at`, latency and a complete frozen canonical
  delivery-audit payload, freezes the mandatory generic `Accepted`
  disposition and an optional task transition, marks the attempt `Accepted`,
  changes
  budget/applicable cooldown to `Accepted`, and changes the decision to
  `AcceptedAuditPending`. It also freezes the state/result authority and both
  reservation events. It is not atomic with the remote sink and does not claim
  to be.
- `Rejected(reason)`: one result transaction stores the definite rejection,
  freezes the mandatory generic `Rejected` disposition plus an optional task
  transition, marks the attempt `Rejected`, changes the decision to
  `RejectedAuditPending`, and changes budget/applicable cooldown to `Released`
  with append-only events. No user-visible acceptance is counted. The decision
  becomes `RejectedDurable` only after the coordinator appends the generic
  disposition, all required state/reservation audits and any bound task
  transition. A later retry must reuse the byte-identical envelope/provider
  evidence and reacquire reservations under the same decision; it may not
  create a new decision to evade cooldown.
- `Uncertain(reason)`: one result transaction stores the uncertainty, changes
  the attempt to `Uncertain`, freezes the mandatory generic `Uncertain`
  disposition plus an optional task transition, changes the decision to
  `UncertainAuditPending`, and changes budget/applicable cooldown to
  `Uncertain` with append-only events. The decision becomes
  `UncertainManualReview` only after its generic disposition, required audit
  events and any bound task transition are appended. Automatic release and
  automatic resend are forbidden.

If the accepted-result transaction cannot commit while the process is alive,
the coordinator retains the typed receipt in memory and retries that local
transaction only; it never calls the sink again. If the process crashes first,
the in-memory typed receipt is lost and **no receipt is durable**
(`persisted_receipt=false`). The only durable state is `AttemptInFlight`;
recovery waits for lease expiry, revokes the fence, treats the attempt as
uncertain and keeps both reservations charged. This crash path has zero
automatic resends and requires manual resolution based on new external
evidence. A real persisted receipt belongs only to the non-crash path where the
same live process still holds the receipt and eventually commits the local
accepted-result transaction; the crash probe must not invent or require one.

### 6.3 Legal state machine and manual resolution

The implemented legal transitions are:

```text
Reserved -> AttemptInFlight
Reserved -> RejectedAuditPending
AttemptInFlight -> AcceptedAuditPending
AttemptInFlight -> RejectedAuditPending
AttemptInFlight -> UncertainAuditPending
AcceptedAuditPending -> AcceptedTaskTransitionPending  # task binding
AcceptedAuditPending -> Delivered                       # no task binding
AcceptedTaskTransitionPending -> Delivered
RejectedAuditPending -> RejectedTaskTransitionPending  # task binding
RejectedAuditPending -> RejectedDurable                 # no task binding
RejectedTaskTransitionPending -> RejectedDurable
UncertainAuditPending -> UncertainTaskTransitionPending # task binding
UncertainAuditPending -> UncertainManualReview          # no task binding
UncertainTaskTransitionPending -> UncertainManualReview
UncertainManualReview -> AcceptedAuditPending           # manual accepted CAS
UncertainManualReview -> ManualRejectedAuditPending
ManualRejectedAuditPending -> ManualRejectedTaskTransitionPending # task binding
ManualRejectedAuditPending -> ManualResolvedRejected              # no task binding
ManualRejectedTaskTransitionPending -> ManualResolvedRejected
RejectedDurable -> Reserved                      # authorized retry only
```

Any other transition fails closed. Byte-identical replay of an already durable
disposition, audit or task transition is idempotent; the same identity with
different bytes is a conflict. Every `*AuditPending` state means the mandatory
generic delivery disposition and all audit-outbox rows required by the
originating transaction are not yet fully appended. Only after they are
appended may the coordinator either enter the optional
`*TaskTransitionPending` state or skip directly to the non-task terminal
state. Every `*TaskTransitionPending` state means a task binding exists and
only its frozen task transition remains. A non-task decision never enters one
of those states and receives `ScheduleHydration=None`.

`RejectedDurable -> Reserved` is one new `BEGIN IMMEDIATE` transaction that
verifies the unchanged envelope/disposition retry authorization, increments
`reservation_generation`, inserts new cooldown/budget identities and their
events, reserves one slot, and appends the state audit. For
`BusinessDateOnce`, it must also find the retained date claim mapped to this
same decision identity; absence or a different identity fails closed. It never
creates a second same-date claim. Failure leaves the decision rejected and
invokes no sink.

Reservation projection is exact:

| Decision state | Budget/cooldown state |
| --- | --- |
| `Reserved`, `AttemptInFlight` | `Reserved`; one active slot |
| `AcceptedAuditPending`, `AcceptedTaskTransitionPending`, `Delivered` | `Accepted`; one active slot |
| `UncertainAuditPending`, `UncertainTaskTransitionPending`, `UncertainManualReview` | `Uncertain`; one active slot |
| `RejectedAuditPending`, `RejectedTaskTransitionPending`, `RejectedDurable`, `ManualRejectedAuditPending`, `ManualRejectedTaskTransitionPending`, `ManualResolvedRejected` | `Released`; zero active slots |

No state may hold a second slot. An accepted or manually accepted Global
reservation remains accepted.
For a pre-sink no-reservation rejection with `reservation_generation=0`,
“Released” means both reservation identities remain null and no projection row
exists; the coordinator must not fabricate a Released row merely to satisfy
the table.

`resolve_uncertain` accepts only a decision currently in
`UncertainManualReview`, after the original generic `Uncertain` disposition,
all required audits and any bound task transition have been durably appended.
It requires an authenticated operator identity,
reason, resolution timestamp and canonical external evidence. Before changing
SQLite it idempotently appends/verifies a five-year immutable
manual-resolution authorization audit under the resolution identity; the
following SQLite transaction stores that reference. An audit success followed
by SQLite failure changes no reservation and is safely retried with the same
bytes. “Accepted”
requires a real provider/channel receipt or independently verifiable
acceptance evidence; in one transaction it records `disposition='Accepted'`,
freezes the canonical delivery-audit payload, the mandatory generic
`ManualAccepted` disposition and an optional accepted task transition, and
strictly compare-and-sets `UncertainManualReview -> AcceptedAuditPending`.
There is no `ManualResolvedAccepted` projection. The transaction changes
budget/cooldown from `Uncertain` to `Accepted`, retains the same generation and
slot, and inserts state/reservation audit events; normal accepted reconciliation
then reaches `Delivered`.

“Rejected” requires evidence that no delivery occurred; one transaction
records `disposition='Rejected'` in `manual_resolutions`, freezes the mandatory
generic `ManualRejected` disposition and an optional manual-rejected task
transition, enters `ManualRejectedAuditPending`, changes budget/cooldown to
`Released`, frees the active slot and inserts state/reservation audit events.
After the generic disposition and required audits are appended, a non-task
decision enters `ManualResolvedRejected`; a task-bound decision first enters
`ManualRejectedTaskTransitionPending` and reaches the same terminal state only
after that task transition is appended. The resolution command, evidence hash,
actor and state transition retain the immutable audit reference. No timer,
process restart or ordinary retry may resolve or release uncertainty.

Every envelope freezes exactly one generic delivery disposition per result,
denial or manual resolution:

| Generic disposition | Mandatory immutable payload |
| --- | --- |
| authoritative `Accepted` | decision/envelope hashes, attempt/fence identity, typed receipt/result hashes, accepted timestamp, retryable=false |
| definite `Rejected` including pre-sink denial | decision/envelope hashes, attempt or denial identity, typed reason/evidence hash, retry authorization and result/denial timestamp |
| `Uncertain` including expired/fenced attempt | decision/envelope hashes, uncertainty reason, attempt/fence identity, recovery evidence, manual-action-required=true and result/recovery timestamp |
| `ManualAccepted` | decision/envelope hashes, original uncertain disposition, resolution/operator audit reference and real receipt or independent acceptance-evidence hash |
| `ManualRejected` | decision/envelope hashes, original uncertain disposition, resolution/operator audit reference and proof-of-no-delivery hash |

The generic identity is
`SHA256("delivery-disposition-v1", decision_identity,
attempt-or-denial-or-resolution_identity, disposition,
canonical_result_or_evidence_sha256)`. It does not contain or require a task
identity. Multiple rejected attempts therefore have distinct append-only
generic dispositions under one decision.

Only when `task_binding_present=1`, the same transaction additionally freezes
one BR-140 task transition derived from the stored task basis. Its identity is
`SHA256("BR-140-disposition-v1", task_identity, decision_identity,
attempt-or-denial-or-resolution_identity, task_disposition)`, and its canonical
bytes include the original transition-basis hash plus the generic disposition
identity/hash. For manual acceptance `task_disposition=Accepted`; for manual
rejection it is `ManualRejected`. The coordinator is the sole owner that
freezes, appends and replays both payload classes, but never substitutes one
for the other. `ScheduleHydration` is derived only from an appended persisted
task transition; non-task decisions terminate with no hydration. Callers may
not construct either payload or infer an outcome from a sink boolean.

### 6.4 Reconcile and crash matrix

`reconcile_all_pending()` is the only mutating reconciliation entrypoint. It
has no business-date argument and starts by enumerating every unresolved
decision across all stored dates in stable `(business_date,
decision_identity)` order. Production startup keeps every counted producer and
new reservation admission frozen, invokes this all-date pass before starting
any producer, and repeats it until it reaches an all-date fixed point: no
locally progressable audit/disposition/task payload remains pending and every
expired attempt visible to the pass has been fenced and classified. Only then
does startup pass returned stored deliverable identities to
`resume_deliverable`, still with all producers frozen, and rerun
`reconcile_all_pending()` after each typed result. Startup may publish
producer-ready state only after no startup deliverable or locally progressable
pending work remains. Manual-review decisions and non-expired foreign attempts
remain explicit non-progressable boundaries rather than being silently
filtered by date. A
non-expired `AttemptInFlight` owned by another live process remains unchanged
as a verified non-progressable lease boundary; it is reported separately and
cannot be misreported as locally reconciled work. For each expired attempt the
pass performs the fenced recovery transaction in §6.2, entering
`UncertainAuditPending`, marking its applicable cooldown and budget
reservation `Uncertain`, and returning no deliverable retry.

`inspect_pending_for_date(business_date)` is a read-only diagnostic over a
validated date. It cannot append audit bytes, revoke a fence, change a state,
hydrate a task, reserve/release capacity or set producer readiness. The R-09
preflight may call it for operator visibility, but neither that call nor any
date-filtered SQL query satisfies startup reconciliation. This prevents
today's startup from stranding an unresolved previous-business-date decision.

Before any state progression, reconcile drains every pending
`immutable_audit_outbox` row for decision state, lease/heartbeat, fence
revocation, recovery classification, sink-result authority, late receipt and
reservation mutations. It appends/verifies exact canonical bytes to the
five-year hash chain and compare-and-sets the returned reference. It then
appends the mandatory pending `delivery_disposition_payloads` row. For
`AcceptedAuditPending`, it also appends/verifies the frozen delivery-audit
bytes. Manual acceptance is already in this same state by strict CAS and uses
the same path.

After those generic/audit appends:

- accepted with no task binding enters `Delivered`; accepted with a binding
  enters `AcceptedTaskTransitionPending`, appends its frozen task transition,
  then enters `Delivered`;
- rejected with no task binding enters `RejectedDurable`; rejected with a
  binding enters `RejectedTaskTransitionPending`, appends its frozen task
  transition, then enters `RejectedDurable`;
- uncertain with no task binding enters `UncertainManualReview`; uncertain
  with a binding enters `UncertainTaskTransitionPending`, appends its frozen
  task transition, then enters `UncertainManualReview`;
- manually rejected with no task binding enters `ManualResolvedRejected`;
  task-bound manual rejection enters
  `ManualRejectedTaskTransitionPending`, appends its frozen task transition,
  then enters `ManualResolvedRejected`.

Each state advance freezes its own new state-event audit row; if that append is
pending, reconcile repeats before exposing terminal outcome or hydration. The
event bus is an observation after the durable audit; it is not a state owner.

Every reconcile operation uses the original envelope, receipt, decision and
transition identities and reports `provider_calls=0` and `sink_calls=0`.
Accepted/pending/manual-accepted reconciliation reports the one active
`budget_reservation_identity` and generation; manual-rejected hydration
reports no active reservation while preserving all Released rows. It creates
no observation time, provider batch, message body, sink attempt or decision
identity.
`ReconcileSummary` also returns two typed lists: locally pending decisions and
deliverable decisions. It may list stored `Reserved` work and
retry-authorized `RejectedDurable` work as deliverable, but it cannot execute
either remote call itself.

`resume_deliverable(decision_identity)` is the only restart-safe remote resume
operation. It loads and verifies the stored envelope/hash and policy version;
it never calls a provider or renderer. For `Reserved`, it performs the fenced
attempt transaction then invokes the authoritative sink at most once. For a
retry-authorized `RejectedDurable`, it first reacquires the exact cooldown head
and one daily-budget slot in one `BEGIN IMMEDIATE`, enters `Reserved`, and then
uses the same fenced path. Two processes concurrently resuming the same
decision serialize on SQLite: only the process whose transaction installs the
current attempt/fence may call the sink; the loser returns the persisted
current state with `sink_calls=0`. `Uncertain*`, accepted/pending, terminal
manual and `Delivered` states are never remotely resumable.

| Last provable state/result | Automatic action |
| --- | --- |
| no decision; sink never called | normal caller retry |
| storage failure before admission commit | retry same bytes; no durable decision and zero sink |
| cooldown/budget/configuration pre-sink denial | atomically persist no-reservation `RejectedAuditPending`; append generic disposition/audits and optional task transition; zero sink |
| exact decision/envelope replay | return persisted state and optional hydration; zero allocation/sink |
| same decision identity with different canonical bytes | preserve original, append conflict audit, fail closed; zero sink |
| `Reserved` | reconcile lists deliverable; `resume_deliverable` uses stored envelope, installs one fence and invokes sink once |
| non-expired `AttemptInFlight` owned by another process | leave unchanged; zero sink |
| expired `AttemptInFlight` | revoke fence, freeze generic uncertainty plus optional task transition and all audit rows, retain reservations; no sink |
| original call returns `Accepted` after fence revocation | append late real receipt and authority decision to audit as non-authoritative evidence; remain uncertain; no automatic resend/release |
| explicit `Rejected` durable | generic disposition/audits and optional task transition already appended, Released generation retained; optional authorized same-decision `resume_deliverable` |
| explicit `Uncertain` durable | retain reservations; require audited manual resolution |
| sink accepted but local result transaction is retrying | retain receipt in process; retry local SQLite only |
| sink accepted, then process crashes before local result commit | in-memory receipt is lost and `persisted_receipt=false`; after lease expiry revoke fence, enter uncertainty, prohibit resend and require manual evidence |
| `AcceptedAuditPending` | append required audit/generic disposition/delivery audit, then optional accepted task transition; zero provider/sink |
| `AcceptedTaskTransitionPending` | append only the frozen task transition, then deliver |
| `RejectedAuditPending` | append required audit/generic rejection, then optional rejected task transition |
| `RejectedTaskTransitionPending` | append only the frozen task transition, then reject durably |
| `UncertainAuditPending` | append required audit/generic uncertainty, then optional uncertain task transition |
| `UncertainTaskTransitionPending` | append only the frozen task transition, then require manual review |
| `ManualRejectedAuditPending` | append required audit/generic manual rejection, then optional task transition |
| `ManualRejectedTaskTransitionPending` | append only the frozen task transition, then hydrate manual rejected |
| `Delivered` | hydrate terminal caller/schedule outcome; no side effects |
| `ManualResolvedRejected` | hydrate terminal rejected outcome with released reservations |

### 6.5 Review-date and retry contract

R-09 is current-date-only because the concrete upstream route rejects any
request whose date differs from the current Asia/Shanghai calendar date.

- On a current-date run before 15:35, R-09 returns `ExpectedWait` for 15:35 and
  performs no provider request.
- On a weekend, holiday or explicit historical `--review`, the strict review
  business date may be the latest completed trading day and therefore differ
  from the current Shanghai date. R-09 must reject this locally, before Router
  construction or network access, as non-retryable
  `provider_top_n_current_date_only`. Retrying the same historical task
  identity cannot make it valid; a later trading day is a different task.
- A malformed local review date, unsupported fixed metric or immutable
  capability/configuration mismatch is non-retryable.
- Transport, TLS, timeout, rate-limit and blocking-worker failures are
  retryable. A current-date provider batch that is partial, empty, stale,
  malformed or inconsistent in source/order/cardinality/filter/date/evidence,
  or that supplies forbidden `source_at`, is also a retryable R-09 failure as
  required by BR-192. Acquisition-audit persistence failure is retryable and
  fail-closed.
- Push denial and deduplication remain typed non-delivered, non-retryable task
  outcomes when no pending R-09 delivery state exists. An explicit sink
  rejection is retryable from the original `RejectedDurable` decision after
  its reservations are durably released and reacquired. A durable sink
  acceptance followed by delivery-audit or transition failure is not a new
  delivery attempt: `reconcile_all_pending` must finish the original identity
  with zero provider and sink calls. A stored `Reserved` or retry-authorized
  `RejectedDurable` decision may cross a restart only through
  `resume_deliverable`, with zero provider calls and the fencing rules in
  §6.2. An uncertain sink outcome is non-retryable by automation and requires
  audited manual reconciliation.
- A deduplicated release run can satisfy Gate D only by joining the same report
  decision identity to a prior durable sink-acceptance receipt and matching
  delivery audit. A new acquisition/decision identity may not borrow another
  decision's receipt.

## 7. Test/live isolation

`monitor --test` and `monitor --test --review` must not construct the
composition Router, call Eastmoney or accept real symbols. Their process-level
R-09 outcome is explicitly `Disabled` with
`test_environment_external_provider_blocked`, with zero acquisition, binding
or delivery side effects. Focused unit tests exercise the renderer, binding
and task failure mapping through a private loader and `TEST_CODE` batch
factory. Only production `monitor --review`, without test mode, may exercise
the real R-09 provider path.

No test fixture may be written to production push, audit or database
namespaces.

Test-mode delivery-coordinator and budget-ledger fixtures are private,
`TEST_CODE`-scoped and physically rooted under the test namespace. They must
prove the same legal state transitions without invoking a production sink.

## 8. Failure modes

| Failure | Result |
| --- | --- |
| test mode, including `--test --review` | `Disabled`, zero network/side effects |
| historical/weekend/holiday review date | non-retryable `provider_top_n_current_date_only`, zero network |
| malformed local date / unsupported fixed metric / capability false | non-retryable R-09 Failed |
| current date before 15:35 | `ExpectedWait(15:35)`, zero network |
| transient transport/TLS/timeout/rate limit | retryable R-09 Failed |
| either metric absent, empty, partial, stale or malformed | whole R-09 retryable Failed |
| source/order/cardinality/filter/date mismatch | retryable invalid-evidence failure |
| batch or record `source_at` present | retryable invalid-evidence failure |
| acquisition audit persistence failure | retryable fail-closed failure |
| startup has locally progressable unresolved work on any stored business date | run all-date reconcile to a fixed point with zero provider/sink calls; counted producers and new reservations remain frozen on failure |
| date-scoped pending inspection | read-only diagnostic; cannot mutate state or open producer readiness |
| exact decision/envelope dedup | return persisted outcome/optional hydration; no new disposition, reservation or sink |
| decision identity matches but canonical bytes/policy/task binding differ | preserve original, append immutable conflict audit, fail closed |
| push/policy/cooldown/budget denied before reservation | atomic no-reservation `RejectedAuditPending`, generic rejection and optional task transition; zero sink |
| zero or multiple authoritative remote sinks | same no-reservation rejection; zero remote calls; Console observer cannot satisfy delivery |
| explicit sink rejection before acceptance | `RejectedDurable`; release reservations; retry same decision with zero provider calls |
| different decision for a claimed `BusinessDateOnce` date, including after the first generation is Released | durable no-reservation rejection; retained original claim; zero sink |
| timeout/transport/cancel without provable rejection | `UncertainManualReview`; retain reservations; no automatic retry |
| non-expired `AttemptInFlight` seen by another process | leave owned attempt unchanged; zero sink |
| expired `AttemptInFlight` | atomically revoke fence, freeze generic uncertainty/optional task transition and pending audits, retain reservations; no automatic retry |
| fenced original call returns late `Accepted` | retain real receipt plus immutable non-authoritative classification audit; manual accepted resolution only |
| sink accepted; delivery audit unavailable | `AcceptedAuditPending`; replay frozen audit with zero provider/sink calls |
| generic disposition/state/lease/fence audit unavailable | remain corresponding `*AuditPending` or audit-outbox pending; reconcile exact bytes with zero provider/sink calls |
| task transition audit unavailable | remain corresponding `*TaskTransitionPending`; reconcile original binding with zero provider/sink calls |
| sink acceptance result transaction cannot commit before crash | uncertainty after lease expiry + charged reservations; automatic resend forbidden |
| unauthorized/manual resolution without evidence | non-retryable fail-closed rejection; no state or reservation change |
| blocking worker panic/cancel | audited retryable join failure |
| delivery-state or budget identity conflict | non-retryable fail-closed conflict |

No failure returns a default value, fabricated row or unqualified empty
collection.

## 9. Old-module disposition

| Module/path | Decision | Reason |
| --- | --- | --- |
| `CapitalDataGateway::post_close_main_flow` | deleted | dead consumer; old complete-flow/source-time semantics |
| `PostCloseMainFlowFact` | deleted | carried close/change/ratio fields not present in new contract |
| `PostCloseFlowRouter` downstream use | deleted | replaced by the concrete composition Router |
| earlier private `ProviderTopNDeliveryCoordinator` design | reject before implementation | all counted PushKinds require one generic owner |
| process-local daily atomics / counted cooldown table | deleted from counted path | cannot enforce cross-process hard limit or restart safety |
| counted bool/`SinkResult::Ok/Err` adapters | unreachable for counted kinds | cannot distinguish definite rejection from uncertain delivery or retain receipt |
| L6 Console plus remote fan-out as one bool | authority split implemented in durable runtime | Console/legacy fan-out cannot acknowledge counted delivery; one typed Magiclaw/Feishu result is authoritative |
| external counted delivery/transition append | reject | coordinator alone freezes and replays delivery audit and linked transition |
| BR-190 full-market markers | retain | complete-market and intraday capability remains unavailable |
| R-02 market overview | retain disabled | Top-N cannot prove indices, turnover or breadth |
| I-10 and BR-073/BR-150 | retain retired | post-close facts do not authorize intraday or trading actions |

## 10. Wiring matrix

### 10.1 R-09 catalogs

| Catalog/seam | Current disposition |
| --- | --- |
| `ReviewTask` enum | contains `R09` |
| `ReviewTask::ALL` | includes `R09`; array size updated |
| task label/source/reason mapping | `R-09` / `eastmoney_provider_top_n` / stable failure category |
| preflight | current-date R-09 waits until 15:35; historical/weekend runs reject before network |
| dispatcher | join R-09 exactly once and always emit a typed outcome |
| delivery disposition audit | coordinator always freezes/stores/replays the generic immutable disposition, independent of task binding |
| optional task transition | R-09's admitted BR-140 binding freezes a separate task transition; callers only apply its optional `ScheduleHydration` |
| delivery coordinator | implemented as generic `DurableDeliveryCoordinator`; no private R-09 coordinator |
| dedicated SQLite | implemented one-store schema, migrations, WAL/lock handling and test/prod path guard |
| `PushKind` enum | contains `ReviewProviderTopN` |
| level/banner/cooldown/scope/label | Important, banner required, 86,400 seconds, Global, source-limited label |
| daily budget | one slot reserved before sink; accepted/uncertain retain it; definite rejection releases it |
| v14 signal mapping | stable `review_provider_top_n` kind |
| delivery subject | non-security report identity; `SignalEvent.code=None` |
| stable template ID | `review_provider_top_n_v1` |
| mode behavior | factual review is not blocked by missing intraday Quote/MoneyFlow/OrderBook |
| isolated test modes | both `--test` and `--test --review` have zero network; fixtures are unit-test-only |
| restart recovery | before any producer, run no-argument all-date reconcile to a fixed point with provider/sink zero-call; R-09 date-scoped inspection is read-only, and only `resume_deliverable` may invoke one fenced sink from a stored envelope after startup readiness |
| old exports/symbols | zero references to `PostCloseMainFlowFact`, `post_close_main_flow`, `PostCloseFlowRouter` |

### 10.2 Every counted PushKind: identity and cooldown contract

The current `src/durable_delivery/model.rs::compiled_policy_catalog` contains
the following fifteen kinds, including `ReviewProviderTopN`. Former direct
`notify::push_governor*` counted calls have been migrated to a typed binding or
an explicit unavailable boundary; any future direct call is rejected by the
generic governor and cannot bypass the coordinator.

The compiled/seeded catalog therefore has **15 distinct `push_kind` values
and 18 distinct `(push_kind, sub_kind)` rows**: fourteen kinds have one
`NONE` row, while `DailyReport` has four rows.

Every decision identity is exactly:

```text
SHA256(
  "durable-delivery-decision-v1",
  policy_version,
  business_date,
  push_kind,
  canonical_sub_kind,
  cooldown_scope,
  scope_key,
  schedule_occurrence_identity,
  source_evidence_fingerprint,
  delivery_subject_hash,
  rendered_content_sha256
)
```

`source_evidence_fingerprint` is the ordered hash of the real immutable batch,
event, decision or account-evidence identities named below; it is never local
write time. `canonical_sub_kind` is `NONE` unless the matrix permits another
value. `schedule_occurrence_identity` is mandatory and stable across restart.
A replay with the same decision identity but different canonical bytes is a
non-retryable conflict.

| Counted PushKind | Required occurrence/evidence identity | Scope key and sub-kind | Catalog cooldown/override | `BEGIN IMMEDIATE` admission |
| --- | --- | --- | --- | --- |
| `HoldingPlan` | holding-plan decision ID plus ordered quote/risk batch IDs | `PerTicket`, canonical typed instrument; `NONE` | rolling 1,800s; no override | query same policy/sub-kind/ticket head, reserve if released or accepted window expired |
| `HoldingEvent` | source alert/event ID plus event category and evidence IDs | `Global`, `GLOBAL`; `NONE` | no cooldown; no override | no cooldown row, but decision idempotency and one daily slot remain mandatory |
| `T0Advice` | T0 plan decision ID plus typed instrument and ordered evidence batch IDs | `PerTicket`, canonical typed instrument; `NONE` | rolling 1,800s; no override | same-key rolling head query/reservation |
| `CandidateTriggered` | candidate lifecycle transition ID plus typed instrument and admitted-selection evidence ID | `PerTicket`, canonical typed instrument; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `CloseCall` | close-call schedule occurrence plus validated close-call batch ID | `Global`, `GLOBAL`; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `ForbiddenOps` | risk-decision ID plus typed instrument, account snapshot ID and ordered rule IDs | `PerTicket`, canonical typed instrument; `NONE` | rolling 3,600s; no override | same-key rolling head query/reservation |
| `PaperTrade` | paper order/fill business ID plus typed instrument and paper-account snapshot ID | `PerTicket`, canonical typed instrument; `NONE` | rolling 300s; no override | same-key rolling head query/reservation |
| `ReviewMarket` | BR-140 review-task identity plus ordered market-review batch IDs | `Global`, `GLOBAL`; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `ReviewLhb` | BR-140 review-task identity plus accepted LHB batch IDs | `Global`, `GLOBAL`; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `ReviewSignal` | BR-140 review-task identity plus ordered signal/position evidence IDs | `Global`, `GLOBAL`; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `ReviewFailure` | BR-140 review-task identity plus failed-sample batch IDs | `Global`, `GLOBAL`; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `TomorrowWatch` | BR-140 R-07 review-task identity plus accepted watch-candidate batch IDs | `Global`, `GLOBAL`; `NONE` | `BusinessDateOnce` for the validated business date, nominal 86,400s; no rolling expiry/override; daily-budget exempt | immutable date-key claim maps to one decision before slot allocation; same-date replay/conflict makes zero new sink calls, while the next validated business date creates an independent claim regardless of the prior Accepted wall time |
| `EventCalendar` | BR-140 review-task identity plus complete event-calendar batch IDs | `Global`, `GLOBAL`; `NONE` | rolling 86,400s; no override | same-key rolling head query/reservation |
| `DailyReport` | producer schedule occurrence plus its ordered real evidence IDs; producer identity is mandatory for legacy direct callers | `Global`, `GLOBAL`; exactly `NONE`, `FactorIC`, `SectorTier` or `CapitalVerify` | `NONE` and `FactorIC`: rolling 86,400s; `SectorTier` and `CapitalVerify`: registered 1,800s override; no caller-defined override | select exact sub-kind catalog row, partition cooldown head by sub-kind, resolve `override.or(base)` inside transaction, then reserve |
| `ReviewProviderTopN` | exact R-09 task identity, both request fingerprints/batch IDs, ordered projection hash and content hash | `Global`, `GLOBAL`; `NONE` | `BusinessDateOnce` for the validated business date, nominal 86,400s; no rolling expiry/override | immutable date-key claim maps to one decision before slot allocation; Released keeps claim, same decision may retry with a new generation, different same-date decision is rejected, and only the next validated business date may create a new claim |

The DailyReport row deliberately preserves all three typed sub-kinds. `FactorIC`
inherits the 86,400-second base window; `SectorTier` and `CapitalVerify` retain
their existing registered 1,800-second overrides. `NONE` is the canonical
value for existing general/direct DailyReport producers, not an empty string.
The override is selected from `delivery_policy_catalog`; a caller cannot pass
an arbitrary duration. The compiled catalog and seeded SQLite rows must have
the same policy version/hash or startup remains admission-frozen.

### 10.2.1 BR-245 R-07 business-date admission correction

The 2026-08-18 production failure proves that R-07 is not a rolling signal.
The stored v4 policy row was exactly `Global/Rolling/86400/counts_budget=1`.
The 2026-08-17 decision prefix `adee` was Accepted at
`2026-08-17T15:20:35.788Z` (23:20:35.788 Asia/Shanghai), leaving its Accepted
head blocked through `2026-08-18T15:20:35.788Z`. The next business-date R-07
decision prefix `1b990` ran at `2026-08-18T13:00:45Z` (21:00:45
Asia/Shanghai) and became `RejectedDurable` with `CooldownConflict`; it has no
attempt or receipt. The compiled pre-fix fact is reproducible without a
single-line caller inference:

```text
$ rg -n -C 1 'TomorrowWatch' src/durable_delivery/model.rs
435:        (TomorrowWatch, Global, Some(86_400), Rolling),
```

BR-245 changes only the policy authority. `TomorrowWatch` uses the existing
global `BusinessDateOnce` claim keyed by the validated R-07 business date and
is exempt from the shared intraday daily budget. The nominal 86,400-second
catalog value is descriptive compatibility data and is never an expiry
calculation for this row. A prior business day's Accepted timestamp cannot
deny the next business day. Within one business date, the existing immutable
claim still admits exactly one decision: replay reuses its durable state,
while different canonical bytes/identity fail closed before the sink.

Data flow remains `R-07 canonical envelope -> compiled/sealed policy catalog
-> BEGIN IMMEDIATE BusinessDateOnce claim -> existing durable sink/audit
state machine`. No new R-07 coordinator branch, clock fallback or caller-side
deduplication is introduced. Existing rolling policies remain unchanged.

Failure handling is fail-closed: an invalid business date, mismatched catalog,
same-date conflict or schema migration error produces no sink call. The schema
upgrade replays only `delivery_policy_catalog`; it must prove that all existing
decision, attempt, claim, sink-result, cooldown and immutable-outbox rows are
byte-for-byte unchanged. Rollback is the scoped PR revert; production durable
history is never deleted. The old `Rolling` row is rejected rather than kept
as a second module because dual policy authority would make admission depend
on startup/schema state.

Acceptance is machine-checkable with focused durable-delivery tests that:

1. deliver an R-07 decision at 2026-08-17 23:20 Asia/Shanghai, then admit and
   deliver a distinct 2026-08-18 decision at 21:00 with one sink call;
2. replay/conflict on 2026-08-18 with zero additional sink calls;
3. migrate the immediately previous schema and compare complete authority-table
   rows before and after, allowing changes only in `delivery_policy_catalog`
   and `PRAGMA user_version`.

For every rolling row, the same `BEGIN IMMEDIATE` transaction selects the
catalog row and cooldown head, checks `Reserved`/`Uncertain` or unexpired
`blocked_until`, inserts a new mutable reservation-generation projection plus
its append-only audit event, advances the head and allocates the daily slot.
This is a query plus constraint under the SQLite writer lock, not a
process-local precheck. Only catalog rows with
`counts_against_daily_budget=true` compete for the shared 30-slot
Shanghai-business-date budget. BR-237 review rows and BR-245 TomorrowWatch are
explicitly exempt; exemption does not bypass cooldown/date claims, durable
state, sink authority or audit.

Current counted caller-family disposition:

| Counted caller family | Current Gate B disposition |
| --- | --- |
| `push_templates.rs` counted dispatchers | producers with complete immutable evidence construct registered bindings/envelopes; unsupported producers fail closed before acquisition or sink |
| `main.rs` former direct counted governor calls | migrated to explicit binding or stable `capability_unavailable`; generic counted calls cannot deliver |
| typed DailyReport sub-kinds | retained in `notify.rs` and mapped by `durable_delivery_runtime.rs`; the deleted standalone router is not restored |
| `v14_adapter.rs` | legacy commit/rollback remains for uncounted governance and explicitly rejects counted kinds |
| R-09 dispatcher | uses `ReviewProviderTopN`; no private ledger, sink or transition owner |

A compile-time catalog test and a fixed `rg` process check must prove that no
one of these kinds can reach `push_wechat`, L6 `SinkRouter::route`,
`event::publish_delivery`, `record_cooldown`, `DAILY_BUDGET_COUNT` or
`v14_adapter::{commit,rollback}_dedup_for_event` outside the coordinator.

### 10.3 Existing layer migration and activation order

| Existing path | Required disposition |
| --- | --- |
| `src/bin/monitor/push_templates.rs` | counted catalog is owned by the coordinator; former process-local counted budget/cooldown owner is absent, and incomplete producers return explicit unavailable outcomes |
| `src/bin/monitor/notify.rs` | generic counted entrypoints reject with `counted_binding_required`; `push_counted_with_binding` delegates to the runtime coordinator |
| `src/bin/monitor/durable_delivery_runtime.rs` | owns runtime composition, typed Magiclaw/Feishu authoritative sink, producer gate, schedule hydration and physical namespace selection |
| `src/bin/monitor/l6_sink.rs` and `src/push_l6/**` | remain legacy uncounted/observer infrastructure; they cannot acknowledge a counted delivery |
| `src/bin/monitor/v14_adapter.rs` | preserves uncounted governance; counted reserve/commit/rollback calls are rejected |
| `src/event/durable_delivery_append.rs` | implements idempotent exact-byte append for generic disposition, delivery and critical state/lease/fence/reservation audit payloads |
| `src/bin/monitor/review_batch.rs` | owns optional frozen BR-140 task-transition mapping and schedule hydration without becoming a delivery owner |
| `src/bin/monitor/main.rs` manual/recurrent review | consumes coordinator outcomes/hydrations and does not append a second R-09 transition |

The implementation and production activation follow one all-or-nothing
cutover:

1. create/migrate the dedicated database, seed/version-check all eighteen
   `(push_kind, sub_kind)` policy rows representing fifteen distinct kinds,
   validate the lease config, and start in admission-frozen mode;
2. implement the one authoritative typed remote adapter, Console observer and
   coordinator tests without routing
   production counted traffic;
3. migrate all fourteen existing kinds plus R-09 and the event/review adapters;
4. in the same activation change, remove/disable every counted legacy
   counter, cooldown and dedup settle path;
5. with producers frozen, run no-date-filter startup reconciliation over every
   unresolved business date, resume only its returned durable identities, and
   rerun all-date reconciliation after each result until the startup fixed
   point; prove a previous-business-date local pending record is settled with
   provider/sink calls both zero, then start counted producers and enable new
   reservations.

The current worktree has reached the fail-closed cutover shape: R-09 and
producers with admitted immutable bindings use the coordinator, the generic
governor rejects counted kinds, and producers whose evidence contracts remain
incomplete report stable `capability_unavailable` before acquisition or sink.
No unsupported producer may dual-write, split traffic or choose a ledger by
environment variable. Gate B still requires independent proof that the old
process-local counted budget/cooldown paths are unreachable and that all
required process tests pass. Falling through to a default 30-minute cooldown,
omitting R-09 from `ReviewTask::ALL`, or retaining a direct counted sink call
is blocking.

## 11. Validation and acceptance

The following commands address tests and modules that currently exist. They
are validation entrypoints, not a claim that this revision has passed Gate B:

```bash
cargo test --lib data_gateway::capital
cargo test --lib durable_delivery::tests
cargo test --bin monitor br192_
cargo test --test durable_delivery_counted_cutover
cargo test --test br192_candidate_counted_binding
cargo test --test br192_main_fail_closed_counted_producers
cargo test --test br192_monitor_test_counted_cleanup
cargo test --test br192_t0_counted_binding
cargo test --test br192_paper_trade_counted_binding
cargo test --test br192_paper_trade_quote_freshness
cargo test --test magic_market_release_revision
cargo test --test monitor_help_isolation
cargo check --bin monitor
```

Documentation-to-tree assertions:

```bash
test -f src/durable_delivery/coordinator.rs
test -f src/event/durable_delivery_append.rs
test -f src/bin/monitor/durable_delivery_runtime.rs
test ! -e src/bin/monitor/daily_report_router.rs
rg -n 'rev = "d7dfa3140919525f3280bed87136602a78fa17ad"' Cargo.toml
rg -n 'counted_binding_required' src/bin/monitor/notify.rs
```

The following process-level acceptance targets are still absent from `tests/`
and therefore remain Gate B blockers:

- `durable_delivery_process_isolation`;
- `durable_delivery_fencing_process_race`;
- `durable_delivery_startup_reconcile_previous_business_date`;
- `durable_delivery_business_date_once_claim`.

`durable_delivery_startup_reconcile_previous_business_date` must seed a pending
audit/disposition/task row under the previous validated business date, start a
fresh production-shaped coordinator process on the following date, and prove
the no-argument startup pass converges that old row before its producer-ready
barrier opens. Provider and sink spies must both remain at zero. Running only
`inspect_pending_for_date(today)` must leave the old row and producer-ready
barrier unchanged.

The two required `durable_delivery_business_date_once_claim` cases must use
separate processes on one test-isolated SQLite file. The first admits a decision,
drives its reservation generation to `Released`, then proves a different
same-date decision receives a durable no-reservation rejection, the original
claim remains byte-identical and both provider/sink retry counts are zero. The
second proves the claimed decision alone can enter an authorized new
generation, and that a new claim is possible only after the calendar fixture
validates the next business date; restart and elapsed wall-clock time cannot
create a second same-date claim.

The static migration check must return no direct counted sink/audit/counter
owner outside the coordinator:

```bash
rg -n \
  'DAILY_BUDGET_COUNT|DAILY_BUDGET_DAY|record_cooldown|push_wechat\\(|SinkRouter::route|publish_delivery\\(|commit_dedup_for_event|rollback_dedup_for_event' \
  src/bin/monitor src/event src/push_l6
```

Every reported line must either be the implemented coordinator/runtime typed
sink adapter, an uncounted-only path proven by the catalog test, or a test. A
counted production caller outside the binding/coordinator path is blocking.

Gate C remains:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

Gate D build/runtime commands remain:

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
cargo run --release --bin monitor -- --test
cargo run --release --bin monitor -- --review
```

Gate D must not silently use the design date, wall-clock “today”, or different
dates for SQLite/event/push-log evidence. The release operator sets one
explicit `RELEASE_BUSINESS_DATE`. The planned `delivery_evidence` read-only
validator is not yet implemented and is a Gate D blocker; when added, it must
first prove the date is a completed Asia/Shanghai trading day represented in
the coordinator store:

```bash
export RELEASE_BUSINESS_DATE=2026-07-30

cargo run --release --bin delivery_evidence -- \
  validate-release-business-date \
  --db data/durable_delivery.sqlite3 \
  --business-date "${RELEASE_BUSINESS_DATE}" \
  --timezone Asia/Shanghai \
  --require-completed-trading-day \
  --format json
```

The validation command must exit nonzero unless it emits:

```json
{
  "release_business_date": "2026-07-30",
  "timezone": "Asia/Shanghai",
  "calendar_status": "completed_trading_day",
  "store_date_present": true,
  "validated": true
}
```

The JSON date must equal the exported value byte-for-byte. Only after that
check may the same shell variable be used for every production evidence read:

```bash
cargo run --release --bin delivery_evidence -- \
  show-decision \
  --db data/durable_delivery.sqlite3 \
  --push-kind ReviewProviderTopN \
  --business-date "${RELEASE_BUSINESS_DATE}" \
  --format json

sqlite3 -json data/durable_delivery.sqlite3 "
SELECT d.decision_identity,d.state,
       q.claimed_at,q.audit_identity AS business_date_once_claim_audit_identity,
       c.cooldown_reservation_identity,
       c.reservation_generation AS cooldown_generation,
       c.state AS cooldown_state,
       b.budget_reservation_identity,
       b.reservation_generation AS budget_generation,
       b.attempt_identity AS budget_attempt_identity,
       b.state AS budget_state,b.slot_no,
       s.result_kind,s.channel,s.provider,s.message_id,s.accepted_at,
       s.delivery_audit_ref,
       p.disposition_identity,p.immutable_audit_ref AS disposition_audit_ref,
       t.transition_identity,t.immutable_audit_ref AS task_transition_audit_ref
FROM delivery_decisions d
JOIN business_date_once_claims q
  ON q.decision_identity=d.decision_identity
 AND q.business_date=d.business_date
 AND q.push_kind=d.push_kind
 AND q.sub_kind=d.sub_kind
 AND q.scope_key=d.scope_key
JOIN cooldown_reservations c
  ON c.cooldown_reservation_identity=d.current_cooldown_reservation_identity
 AND c.state='Accepted'
JOIN daily_budget_reservations b
  ON b.budget_reservation_identity=d.current_budget_reservation_identity
 AND b.state='Accepted'
JOIN delivery_attempts a
  ON a.attempt_identity=d.current_attempt_identity
JOIN sink_results s
  ON s.attempt_identity=a.attempt_identity
 AND s.authoritative_for_state=1
JOIN delivery_disposition_payloads p
  ON p.disposition_identity=d.current_disposition_identity
 AND p.append_state='Appended'
JOIN task_transition_payloads t
  ON t.disposition_identity=p.disposition_identity
 AND t.append_state='Appended'
WHERE d.push_kind='ReviewProviderTopN'
  AND d.business_date='${RELEASE_BUSINESS_DATE}'
  AND s.result_kind='Accepted'
  AND NOT EXISTS (
    SELECT 1 FROM immutable_audit_outbox o
    WHERE o.decision_identity=d.decision_identity
      AND o.append_state='Pending'
  );"

rg -n 'review_provider_top_n_v1|ReviewProviderTopN' \
  "data/event_bus/${RELEASE_BUSINESS_DATE}.jsonl" \
  "data/push_log/${RELEASE_BUSINESS_DATE}"
```

`delivery_evidence` must emit one object with at least:

```json
{
  "decision_identity": "<same nonblank SHA-256>",
  "states": [
    "Reserved",
    "AttemptInFlight",
    "AcceptedAuditPending",
    "AcceptedTaskTransitionPending",
    "Delivered"
  ],
  "sink_result": "Accepted",
  "authoritative_remote_sink_count": 1,
  "attempt": {
    "fence_token": 1,
    "late_after_fence": false
  },
  "receipt": {
    "channel": "<real>",
    "provider": "<real>",
    "message_id": "<real>",
    "accepted_at": "<real provider/local evidence>"
  },
  "budget": {
    "budget_reservation_identity": "<same active generation>",
    "reservation_generation": 1,
    "attempt_identity_match": true,
    "slot_no": 1,
    "state": "Accepted",
    "released_history_preserved": true
  },
  "policy": {
    "sub_kind": "NONE",
    "scope_key": "GLOBAL",
    "window_mode": "BusinessDateOnce",
    "policy_version_match": true,
    "business_date_once_claim_identity": "<required>",
    "business_date_once_claim_decision_match": true,
    "business_date_once_claim_audit_ref": "<five-year immutable audit reference>"
  },
  "cooldown": {
    "cooldown_reservation_identity": "<same active generation>",
    "reservation_generation": 1,
    "scope": "Global",
    "state": "Accepted",
    "event_chain_complete": true
  },
  "delivery_audit_ref": "<five-year immutable audit reference>",
  "disposition": {
    "kind": "Accepted",
    "audit_ref": "<five-year immutable audit reference>"
  },
  "task_transition_audit_ref": "<five-year immutable audit reference>",
  "pending_immutable_audit_outbox_rows": 0,
  "critical_state_lease_fence_authority_audits_complete": true,
  "binding_hash_match": true,
  "frozen_delivery_audit_hash_match": true,
  "frozen_disposition_hash_match": true,
  "frozen_task_transition_hash_match": true
}
```

This evidence must join the same decision identity across the immutable
BusinessDateOnce claim, Global reservation, budget reservation
identity/generation/attempt, typed sink result, receipt, generic disposition,
delivery audit and R-09's optional `Delivered` task transition. It must prove
the claim audit and every required state, lease, fence, authority and
reservation audit outbox row is appended. It must also join the day's
controlled `push_log` and
delivery event-bus redacted subject; the `rg` output is supporting evidence,
while the evidence tool must verify their hashes/identity rather than merely
matching text. A new acquisition identity, `Deduped`,
denial, uncertain result or startup banner cannot substitute for an accepted
R-09 delivery.

Fault injection must be physically test-isolated. The planned
`delivery_reconcile_probe` binary is not yet implemented and is a Gate D
blocker. Its required command contract is:

```bash
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_AUDIT --scenario accepted-audit-fail --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_TRANSITION --scenario accepted-transition-fail --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_RECEIPT --scenario accepted-result-commit-crash --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_REJECT --scenario definite-reject --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_MANUAL --scenario manual-resolve-accepted --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_BUDGET --scenario concurrent-31 --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_FENCE --scenario late-accepted-after-fence --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_LEASE --scenario live-owner-versus-recovery --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_RESUME --scenario concurrent-resume --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_NO_TASK --scenario non-task-all-dispositions --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_DENIAL --scenario pre-sink-budget-denial --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_HISTORY --scenario reject-reacquire-budget-generation --format json
cargo run --bin delivery_reconcile_probe -- \
  --test TEST_CODE_BR192_AUDIT_OUTBOX --scenario critical-audit-reconcile --format json
```

Each probe returns structured fields, not log-text assertions. The output is
not one fixed record with sentinel defaults: it is a common envelope plus a
`scenario_evidence.kind` discriminant whose exact object shape is selected by
the requested scenario. Fields that do not apply to that variant are absent,
not encoded as misleading `false`, zero, generation one or an empty history.
For example, `concurrent-resume` returns:

```json
{
  "schema_version": 1,
  "scenario": "<name>",
  "decision_identity_unchanged": true,
  "provider_calls": {"reconcile": 0, "resume": 0},
  "budget": {
    "active_slot_count": 1,
    "active_reservation_identity": "<required>",
    "active_reservation_generation": 1,
    "released_reservations": []
  },
  "task_binding_present": true,
  "disposition": {
    "global_reservation": "Accepted",
    "generic_disposition_audit_ref": "<required>",
    "task_transition_audit_ref": "<required>"
  },
  "scenario_evidence": {
    "kind": "concurrent_resume",
    "total_sink_calls": 1,
    "winner_sink_calls": 1,
    "loser_sink_calls": 0,
    "new_fence_count": 1,
    "new_attempt_count": 1
  }
}
```

The JSON decoder uses a closed enum: the normalized scenario name and
`scenario_evidence.kind` must be the matching pair below, unknown keys or
variants fail, and every listed field is required with its native
boolean/integer/string/array type:

| Scenario | `scenario_evidence.kind` | Required scenario-specific evidence |
| --- | --- | --- |
| `accepted-audit-fail` | `accepted_audit_failure` | injected audit stage, pending-before count, pending-after-reconcile count, reconcile sink-call count |
| `accepted-transition-fail` | `accepted_transition_failure` | frozen transition hash, pending-before count, pending-after-reconcile count, reconcile sink-call count |
| `accepted-result-commit-crash` | `accepted_result_commit_crash` | original sink-call count, `in_memory_receipt_lost=true`, `persisted_receipt=false`, lease-expired and fence-revoked booleans/audit references, post-recovery uncertain state, uncertainty-disposition audit reference, `manual_resolution_required=true` and automatic resend count |
| `definite-reject` | `definite_reject` | authoritative sink-call count, released budget identity/generation and generic rejection audit reference |
| `manual-resolve-accepted` | `manual_resolution` | prior state, resolution=`Accepted`, resolver evidence reference, retained slot identity and audit reference |
| `concurrent-31` | `concurrent_budget` | contender count, admitted slot count, rejected contender count and losing sink-call count |
| `late-accepted-after-fence` | `late_accepted_after_fence` | original authoritative sink-call count, old-token-revoked boolean, late-receipt-persisted boolean/reference, old-owner mutation count, automatic resend count, authority-classification audit reference and distinct late-receipt-observation audit reference |
| `live-owner-versus-recovery` | `live_owner_recovery` | lease-expired boolean, old-token-revoked boolean, owner sink-call count and recovery-contender sink-call count |
| `concurrent-resume` | `concurrent_resume` | total/winner/loser sink-call counts, new-fence count and new-attempt count |
| `non-task-all-dispositions` | `non_task_dispositions` | four typed disposition results, generic audit references, task-transition count and schedule-hydration count |
| `pre-sink-budget-denial` | `pre_sink_budget_denial` | sink-call count, reservation generation, absent reservation/attempt identities, generic rejection audit reference and optional real-task transition reference |
| `reject-reacquire-budget-generation` | `reject_reacquire_budget_generation` | first released identity/generation, second active identity/generation, released-history identities and active uniqueness counts |
| `critical-audit-reconcile` | `critical_audit_reconcile` | injected audit stages, pending immutable outbox count before reconcile, pending count after reconcile and reconcile provider/sink-call counts |

Common `budget` and `disposition` values are also scenario-dependent and must
agree with the selected variant. A missing applicable field, a present
inapplicable variant field, or a disagreement between common and
scenario-specific evidence makes the probe fail.

The audit/transition failures require `budget.active_slot_count=1` and an
accepted Global reservation. The accepted-result-commit crash requires
`budget.active_slot_count=1`, Global `Uncertain`, appended fence/recovery and
generic uncertainty audit references,
`scenario_evidence.manual_resolution_required=true`,
`scenario_evidence.in_memory_receipt_lost=true`,
`scenario_evidence.persisted_receipt=false`, an expired lease, a revoked fence,
post-recovery `UncertainManualReview` and zero automatic resends. It must not
output a receipt reference: only the non-crash local-commit retry may persist
the real in-memory receipt. Definite rejection requires
`budget.active_slot_count=0` and Global `Released`.
Manual accepted retains one slot; manual rejected releases it. The
`concurrent-31` result must report at most 30 active
`Reserved|Accepted|Uncertain` slots and
`scenario_evidence.losing_sink_calls=0`. `live-owner-versus-recovery` must show
that a non-expired foreign lease is not revoked and the recovery contender
performs zero sink calls.
`late-accepted-after-fence` must show exactly one original authoritative remote
call, one fence revocation after expiry, zero automatic resend, an immutable
late real receipt, separate appended authority-classification and
late-receipt-observation audit references, zero old-owner state/reservation
mutations and an `UncertainManualReview` decision until manual accepted
resolution.
`concurrent-resume` must run two processes against one stored `Reserved`
decision and report one new fence/attempt, one authoritative sink call total,
zero provider calls, `scenario_evidence.winner_sink_calls=1` and
`scenario_evidence.loser_sink_calls=0`. Repeating every
reconcile/resume/manual command must be idempotent; changing canonical bytes
under the same identity must fail.

`non-task-all-dispositions` must prove Rejected, Uncertain, ManualAccepted and
ManualRejected each append a generic disposition, never insert a task
transition, return `ScheduleHydration=None`, and reach the terminal states in
§6.3. `pre-sink-budget-denial` must prove one atomic
`RejectedAuditPending` decision with `reservation_generation=0`, no
cooldown/budget/attempt identity, one generic rejection, an optional task
transition only for its real task binding, and zero sink calls; byte-identical
replay returns the same identities while changed bytes fail conflict audit.
`reject-reacquire-budget-generation` must prove the Released first
`budget_reservation_identity` remains queryable, the retry has a larger
generation and new identity, and both active uniqueness constraints hold.
`critical-audit-reconcile` must inject hash-chain failure independently at
state, lease, heartbeat, fence, recovery, sink-result authority classification,
late-receipt observation, BusinessDateOnce claim, cooldown and budget events.
The authority-classification and late-receipt injection stages are separate
enum variants with separate expected audit identities; passing one may not
satisfy the other. The probe observes pending immutable outbox rows with no
premature terminal/hydration, then reconciles the exact bytes to zero pending
rows with provider/sink calls both zero.

The schema, coordinator, runtime binding and R-09 modules described above now
exist. The process-level acceptance tests, `delivery_evidence`,
`delivery_reconcile_probe`, controlled production records and Gate D joins do
not yet exist as verified release evidence. In particular, this document does
not claim an accepted R-09 receipt, controlled push-log join or complete
immutable delivery-audit/hash-chain evidence for a release business date.

## 12. Rollback

Rollback is a state protocol, not an immediate `git revert`. The coordinator
state protocol is implemented, but the planned `delivery_admin` operator
binary is not yet implemented and blocks release activation. It must execute
rollback in this order:

1. durably freeze new reservations in the dedicated SQLite database; all
   counted delivery entrypoints then fail closed before a sink;
2. keep the schema and a read-compatible coordinator deployed;
3. run reconciliation until all `immutable_audit_outbox`,
   `delivery_disposition_payloads` and bound `task_transition_payloads` rows
   are `Appended`; every `AcceptedAuditPending` or
   `AcceptedTaskTransitionPending` decision is `Delivered`; every
   `RejectedAuditPending` or `RejectedTaskTransitionPending` decision is
   `RejectedDurable`; every `UncertainAuditPending` or
   `UncertainTaskTransitionPending` decision is `UncertainManualReview`; and
   every `ManualRejectedAuditPending` or
   `ManualRejectedTaskTransitionPending` decision is
   `ManualResolvedRejected`;
4. resolve every `UncertainManualReview` decision through authenticated manual
   accepted/rejected evidence; automatic release is prohibited;
5. stop authoritative sink admission, wait for non-expired attempt leases or
   explicitly recover them after expiry, then verify `AttemptInFlight=0`, all
   audit/disposition/task-pending states `=0`, pending accepted states `=0`,
   uncertain `=0`, immutable audit outbox pending `=0`, and all active plus
   Released reservation generations agree with their result/event histories;
   every `BusinessDateOnce` claim still maps to its original decision and
   corresponding immutable claim audit;
6. prove the rollback binary either reads accepted/consumed budget and Global
   reservations from `durable_delivery.sqlite3` before any sink, or refuse to
   start it. A binary containing only the former process-local atomics/tables
   is not rollback-compatible;
7. only after those checks may the scoped code/dependency commit be reverted.

Concrete future commands are:

```bash
cargo run --release --bin delivery_admin -- \
  freeze-new-reservations --db data/durable_delivery.sqlite3
cargo run --release --bin delivery_admin -- \
  reconcile-all --db data/durable_delivery.sqlite3 --format json
cargo run --release --bin delivery_admin -- \
  pending --db data/durable_delivery.sqlite3 --format json
cargo run --release --bin delivery_admin -- \
  verify-rollback-reader --db data/durable_delivery.sqlite3 \
  --candidate-binary target/release/monitor
```

These planned commands must output signed/hashed operator and immutable audit
references. `pending` must return nonzero until the counts in step 5 are all
zero. The release that first activates the coordinator must already ship the
rollback-compatible reader; otherwise activation is prohibited.

Rollback never deletes or rewrites the dedicated database, receipts, bindings,
state events, `BusinessDateOnce` claims, acquisition evidence, delivery audits,
task transitions or five-year audit references. R-09 then disappears and
BR-190's explicit
unavailable markers remain; the old `PostCloseFlow` consumer is not restored.
