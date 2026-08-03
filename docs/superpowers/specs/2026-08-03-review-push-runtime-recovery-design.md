# BR-204 Review / Push Runtime Recovery Design

**Status:** Gate A candidate; Gate B is prohibited until BR-203 P4 has completed and been committed,
then a fresh independent review of the exact BR-203/BR-204 authority reports C0/I0/M0 and the
reviewed docs are committed byte-identically. BR-204 never edits a BR-203 shared path while any
BR-203 slice remains incomplete.

**Business rules:** BR-110, BR-139, BR-140, BR-145, BR-159, BR-164, BR-192, BR-194,
BR-200, BR-203, BR-204

**Narrow supersession:** BR-204 supersedes BR-139/BR-194 only where they fix R-03 to
`AccountMetricsIncomplete`/`LegacyAccountGate`. R-03 becomes a post-SourceOnly
`HistoricalMonitoredUniverseSource`; every other task classification, test isolation, counted
delivery rule and account-data prohibition remains unchanged. BR-200's “sole live consumer” remains
the closed counted-terminal-preflight profile: R-04 is still its sole live counted consumer and
R-03 gains no counted identity by implication. The frozen BR-139/BR-194/BR-200 rows are not edited;
their checkers, capability table and exact test manifest receive one BR-204 additive profile that
rejects every partial or mixed transition.

## 1. Objective and scope

Restore truthful production `monitor --review` operation before continuing the wider repository
migration. This slice fixes three connected runtime defects:

1. a previously delivered durable occurrence is currently counted and logged as if the current
   process sent it;
2. R-03 is unconditionally rejected by a static legacy account gate even though a historical
   post-close report can consume an exact-date user-confirmed real-position snapshot without a
   broker API or current account balances;
3. a generic Feishu success is not sufficient release evidence unless the accepted platform
   receipt is durably joined to the push occurrence.

R-08 remains explicitly unavailable while the official CFFEX HTTPS live probe fails. This design
does not authorize HTTP, formula, cache, or third-party substitution. BR-196 live `--test`
acceptance remains isolated and requires a separately reviewed non-production Feishu target; the
production target must not be reused.

## 2. Impacted paths

- `src/bin/monitor/review_batch.rs`
- `src/bin/monitor/push_templates.rs`
- `src/bin/monitor/main.rs`
- `src/bin/monitor/notify.rs`
- `src/bin/monitor/l6_sink.rs`
- `src/bin/monitor/v14_adapter.rs`
- `src/push_l4/dispatcher.rs`
- `src/durable_delivery/{model.rs,coordinator.rs,schema.rs,generic.rs,mod.rs}`
- `src/data_gateway/{mod.rs,market_capabilities.rs,review.rs}`
- `src/portfolio/{mod.rs,store.rs}`
- `src/database/user_position_snapshot.rs`
- `src/event/delivery_settlement.rs`
- `tools/compliance/lib/check_br194_review_dependency.sh`
- `tools/release/verify_br194_review_join.py`
- focused monitor/review process tests, generic delivery-settlement tests and exact checker
  mutation tests
- `docs/business_rules.md`

No account snapshot, position snapshot, delivery audit, push log, or durable-delivery row is
deleted or rewritten.

## 3. Runtime contracts

### 3.1 Current-send versus durable reuse

`ReviewTaskOutcome` must distinguish:

- `DeliveredNow { count }`: this invocation reached an authoritative sink, received an accepted
  typed receipt, and committed the associated audit;
- `ReusedDelivered { count, decision_identity }`: an exact current-business-date durable terminal
  was verified before provider/renderer/sink, so this invocation made zero such calls. Both fields
  come only from the validated immutable schedule-hydration transition basis; callers cannot supply
  or override either value.
- `DeliveryUncertain { decision_identity, reason_code, physically_accepted }`: the exact occurrence
  is sealed for manual/audit-only reconciliation. `physically_accepted=Some(true)` maps only from
  `PhysicallyDeliveredAuditFailed`; `None` maps from a recovered `UncertainManualReview`. This state
  is neither delivered nor resend-eligible.

Batch reporting exposes separate `sent_now` and `reused_delivered` counts. Both satisfy the task's
terminal schedule, but only `DeliveredNow` increments `sent_now`. Manual `--review` exits
successfully only when `sent_now + reused_delivered > 0` and no task is in delivery-uncertain or
manual-review state. Independent retryable provider failures, expected publication waits, verified
no-data and explicit disabled capabilities keep their own schedules but do not erase a separately
confirmed delivery. A batch with no new/reused delivery exits 2. The completion log states both
counts and never calls reuse a new push. Retry and idempotency behavior remain unchanged.
The transition-audit wire uses status `delivery_uncertain` and carries the durable
`decision_identity`, closed `reason_code`, and tri-state `physically_accepted`; it never serializes
that state as `failed`, `delivered`, or retryable.

### 3.2 R-03 evidence-scoped monitored-universe report

R-03 becomes an explicit `HistoricalMonitoredUniverseSource` dependency, not the broad
`SourceOnly` classification and not a current-account dependency. Its data flow is:

```text
exact-review-date user_position_snapshot (origin=actual_user_confirmed)
  + STOCK_LIST identities -> MarketCapabilitiesGateway identity batch (origin=watchlist)
  -> origin-preserving stable union
  -> exact review-date whole-market upper-limit pool batch
  -> complete-batch intersection -> stable dedup -> limit(20)
  -> industry-chain aggregation
  -> registered R-03 presentation
  -> governed Feishu delivery
  -> review transition audit
```

The existing database namespaces are the source classification: append-only
`user_position_snapshot`/items are `actual_user_confirmed`; `paper_trades` and its derived open
positions are `paper`; `STOCK_LIST` is `watchlist`. `actual_user_confirmed` means a user-confirmed
historical real-account capture with immutable local evidence; it is not an authenticated broker
batch and cannot authorize orders, cash, net asset value, current account metrics or broker identity.
R-03 consumes only the first and third classes for a non-transactional historical report.
It never reads `paper_trades`, paper-engine positions, or the mutable legacy `stock_position`
projection. This prevents virtual positions from being presented as actual holdings without
requiring a broker integration that does not exist.

The database exposes `user_position_snapshot_for_business_date(date)`. It parses both
`effective_at` and `confirmed_at`, converts them to fixed Asia/Shanghai `+08:00`, and requires only
the `effective_at` date to equal the typed review date. `confirmed_at` may be later than the business
date and cannot change the snapshot's business date. Among same-business-date candidates the reader
selects the greatest `(effective_at instant, confirmed_at instant, snapshot_id)` using timestamp
order and binary string order, preserving BR-146 latest-wins semantics. The reader never uses
`latest` and never substitutes a neighboring date. The selected
snapshot must have a non-empty source, valid domain-separated evidence SHA-256, and internally valid
items. It is historical review evidence, not a claim that the positions are current at invocation
time. If absent, R-03 continues with the
independently complete watchlist component and explicitly audits/renders
`actual_positions_excluded=no_exact_date_user_confirmed_snapshot`; it must not silently substitute
another date. The watchlist API retains provider, source time/observation time, and batch identity
alongside projected securities. The upper-limit pool retains its own provider evidence.

The watchlist component has a closed state: `Available`, `VerifiedEmpty`, `NotConfigured`, or
`Failed`. An absent `STOCK_LIST` is `NotConfigured`, never `VerifiedEmpty`; a configured provider
failure is `Failed` and remains retryable. `NotConfigured` may be explicitly excluded only when the
exact-date actual component is valid. If actual is absent and watchlist is `NotConfigured`, R-03 is
`Disabled(no_monitored_universe_configured)`, not `NoData`. `NoData` requires either a complete
upper-limit batch whose intersection is empty or a provider-proven `VerifiedEmpty` watchlist with no
actual candidates.

Candidate order is exact-date actual positions by canonical code order, then configured watchlist
order. The complete whole-market upper-limit batch is acquired and validated without a candidate
limit. Only after intersection are duplicate codes stable first-wins (preserving
`actual_user_confirmed` over `watchlist`) and the first 20 surviving securities selected. This
retains BR-159's limit-after-complete-market-batch rule. The report evidence note states counts for
actual snapshot and watchlist inputs and `paper_positions_excluded=true`. Missing/invalid watchlist
evidence or upper-limit evidence is a typed retryable failure before rendering/sink; a complete
verified-empty upper-limit batch is `NoData`.

Production validates every snapshot/watchlist security through `env_guard` and rejects any
`TEST_CODE` identity. Test mode requires `TEST_CODE` identities and physically isolated database,
review-audit, push-log, delivery-audit and sink namespaces; a real symbol in test mode fails before
provider or persistence.

### 3.3 Generic Feishu receipt authority

For production Feishu delivery, `Pushed` is allowed only after a transport returns a typed accepted
receipt with non-empty `message_id` and `platform_msg_id`, and a durable append binds the receipt
hash to the exact pre-sink occurrence and push-log/content identity. Before the sink call, the
business identity makes the CAS transition `Reserved -> AttemptInFlight`; a restart that observes
`AttemptInFlight` cannot call the sink and must classify it as manual-review uncertainty. Sink
rejection is durably appended as `RejectedDurable`; a retry never mutates that row and may reserve
only the next monotonic attempt generation under the same schedule key after the registered retry
policy admits it. Sink acceptance commits/seals the schedule identity across all generations even if
later persistence fails. If the
receipt join, L7 append or delivery hash-chain append then fails, settlement is
`PhysicallyDeliveredAuditFailed`: enter AuditDegraded, retain an incident, perform audit-only
reconciliation and never call the sink again for that identity. Test/dry-run paths create no
production receipt authority.

The existing in-memory L4 `reserve/commit/rollback` remains a cooldown projection and is not crash
authority. The sole persistent owner is extended inside the existing `DurableDeliveryCoordinator`
with a separate `GenericDeliveryOccurrenceV1`; no second SQLite authority is created. Its
pre-provider schedule key binds production/test namespace, business date, review task identity when
present, `PushKind`, stable template ID and governance scope identity. This key is computable before
provider, renderer, new decision or sink. Rendered-content SHA-256 and source/source-batch identity
are immutable payload/conflict fields joined under that key after acquisition; they are not lookup
key fields. A second payload for the same schedule key that differs from the stored immutable payload
is a non-retryable invariant conflict and cannot call the sink. The R-03 entry uses a dedicated
binding that accepts only typed `ReviewTask::R03`, the coordinator-derived `review_task_identity`,
and validated hydration; generic callers cannot manufacture this binding or its count. Its closed
states reuse the existing `Reserved`, `AttemptInFlight`, accepted-audit-pending, `Delivered`,
rejected and `UncertainManualReview` semantics. The occurrence row owns attempts and receipt joins;
the existing append-only delivery-audit chain owns audit artifacts, and the coordinator owns the
incident row consumed by audit-only reconciliation. Each transition is generation-bound CAS and a
CAS miss makes zero sink/audit append calls. A schema migration is additive and retains every old
decoder, row and trigger. Exact `Delivered` plus valid stored hydration is the only generic reuse
authority; all other terminal/nonterminal states keep their registered retry/manual-review behavior.

The implementation introduces an internal `FeishuTransportResult` rather than changing truth at a
boolean edge: `Accepted(TypedReceipt)` or `Rejected(TypedSinkError)`. MagicLaw CLI populates the
accepted form through the existing parser. The HTTP webhook remains a non-authoritative transport
until it returns both identifiers and therefore cannot produce production `Pushed`. L6 and legacy
callers may receive a compatibility projection only after settlement; they cannot manufacture an
accepted result from `bool` or HTTP status. The pre-sink push-log is an immutable content artifact,
not delivery proof; the occurrence/content hash is frozen before transport and the receipt audit is
appended after acceptance. Crash recovery treats every nonterminal reservation conservatively as
uncertain and never as resend eligibility.

The implementation must reuse the existing append-only delivery audit and BR-145 settlement
mechanisms. It must not introduce a logging-only `save/push/notify` function, invent IDs, or accept
an HTTP status alone as a platform receipt.

## 4. Failure modes

| Failure | Required behavior |
| --- | --- |
| exact pre-provider schedule terminal exists | validate stored hydration, return `ReusedDelivered`; zero provider/renderer/new-decision/sink |
| same schedule key receives a different content/source payload | non-retryable invariant conflict; no sink |
| durable terminal/hydration mismatch | non-retryable invariant failure; no downstream calls |
| exact-date actual snapshot absent | continue watchlist-only with explicit exclusion; never substitute another date |
| actual snapshot malformed/evidence mismatch | retryable failure; do not silently drop the component |
| watchlist `NotConfigured` | continue only with valid exact-date actual input and audit exclusion; without actual input return `Disabled(no_monitored_universe_configured)` |
| watchlist `VerifiedEmpty` | provider-proven empty component; with no actual candidates this may be `NoData` |
| watchlist identity evidence incomplete | retryable failure before upper-limit join/sink |
| virtual/paper positions exist | exclude and label them; never merge into actual holdings |
| upper-limit provider unavailable/stale/partial/conflicting | retryable failure; no render/sink |
| Feishu CLI returns no complete receipt | sink failure; no confirmed delivery |
| accepted sink followed by receipt/L7/delivery-audit failure | `PhysicallyDeliveredAuditFailed`; seal identity, enter AuditDegraded, audit-only reconcile, never resend |
| restart finds pre-sink `InFlight` without a terminal receipt join | uncertain/manual review; never automatically resend |
| CFFEX official HTTPS probe fails | R-08 remains explicit provider-unsupported failure |

## 5. Old module disposition

| Module | Adopt/reject | Reason |
| --- | --- | --- |
| durable terminal preflight / hydration | adopt and deepen | already proves exact occurrence and zero-call reuse; outcome label is currently misleading |
| exact-date `user_position_snapshot` | adopt | append-only user-confirmed real-account evidence for historical review; never claimed current |
| `portfolio::get_positions()` in R-03 | reject | mutable local projection has no exact snapshot/evidence identity |
| `paper_trades` / paper-engine positions | reject from R-03 actual universe | virtual holdings remain independently classified |
| watchlist security metadata Gateway | adopt and deepen | real provider identity exists; evidence must survive projection |
| `ReviewDataGateway::r03_upper_limit_pool` | adopt | exact-date, evidence-preserving whole-market batch |
| generic boolean `push_wechat` success | reject as confirmed-delivery authority | boolean does not bind platform receipt to the occurrence; migrate through `GenericDeliveryOccurrenceV1` |
| typed MagicLaw Feishu receipt | adopt | already requires real message and platform IDs |
| CFFEX HTTP/formula/cache alternatives | reject | not official admitted HTTPS evidence |

## 6. Validation and release evidence

Gate B begins with failing focused tests for current-send/reuse and R-03 exclusion. Required checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
cargo run --bin monitor -- --review
```

Focused Gate-B tests are frozen before implementation and include exact-date snapshot tie-breaking,
evidence mismatch, `TEST_CODE` cross-mode rejection, watchlist `NotConfigured` versus
`VerifiedEmpty`, complete-batch-before-limit ordering, delivered-now versus reuse, rejected sink
release, accepted-then-audit-failed no-resend settlement, restart-from-`InFlight` uncertainty, L6
compatibility and checker mutations for each missing BR-204 transition. Every named-test command
must list and run a non-zero exact test set.

Production evidence must join one invocation's `sent_now`, push content identity, accepted Feishu
receipt, delivery audit, and task transition. A repeated invocation for the same business date must
show `sent_now=0`, `reused_delivered>=1`, and zero new sink calls for reused counted tasks. Passing
unit tests or a log-only success is not Gate D evidence.

## 7. Rollback

Rollback is a reviewed forward-disable change from the exact release SHA. It disables the R-03
producer before acquisition/render/sink while retaining `DeliveredNow`/`ReusedDelivered`, typed
receipt parsing, `PhysicallyDeliveredAuditFailed`, uncertainty sealing, schema readers and all
append-only databases/audit files. It must not restore reused-as-new display, boolean delivery
authority, automatic resend, local mutable positions, or CFFEX/BR-196 without their independent
authorities. Only after all incidents and nonterminal reservations are reconciled may unused
producer code be removed under a new Gate-A design.
