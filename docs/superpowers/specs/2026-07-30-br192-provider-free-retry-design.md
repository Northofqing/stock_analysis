# BR-192 Provider-Free Authorized Retry Design

**Status:** corrective Gate A candidate after multiple historical formal
reviews and the current three-way read-only precheck. All findings identified
through the current precheck are repaired in these worktree bytes, but no
worktree or prior staged identity is accepted authority. Exact staging plus two
fresh independent C0/I0 reviews remain required; Gate B/C/D pending.
**Date:** 2026-07-30
**Rule:** BR-192
**Data red lines:** 2.1-2.10; exact applicability and DoD are frozen in §0.2

### 0.1 Corrective ownership and supersession

This document and its same-date implementation plan **replace**, rather than
extend, every earlier BR-192 provider-free-retry Gate A/B draft that has not
passed Gate C. Those unaccepted drafts and partial code are implementation
debt to be conformed to this document; they are not upstream architecture and
must not be cited as an accepted prerequisite. The narrow durable-delivery,
Provider TopN and physical-isolation baseline needed by this slice is frozen
directly in §0.3 rather than incorporated by reference from any untracked or
unaccepted document. The retry authorization, scheduling, filtering,
mutex/ownership, cycle recovery, manual command and evidence contracts below
are the sole corrective source of truth.
The earlier contract Gate A review passed against design blob
`cdeec30f46c18bcbdb45ef12782943b90d1533e6`, plan blob
`6d04e26f563c9fbb455faef789daf84c17221fab` and BR-192 row SHA-256
`d3010a1a7a408f8b4ba976de32a0fc046ba6b4f09907d3f2374e1029106cb8ce`
with `Critical=0 / Important=0 / Minor=1`. Minor-1 was packaging-only and
was closed by the root's exact cached-row proof: index blob
`1682b36b3d52ab15c3326ea4d7ebee5628a22db7` contains that reviewed BR-192
row while the masked non-BR-192 digest was unchanged before and after
staging. This metadata closure changes no contract. A later metadata review
found one Important defect: the contract did not freeze the startup behavior
for counted PushKinds without a real binding producer. The first attempted
correction then treated five dirty-worktree/later-gate seams as fixed-HEAD
facts and retained a schema-v4 baseline after HEAD had reached schema v5. Its
formal verdict was `C0/I2/M0`. That revision attempted to repair both defects
directly against `HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496`; the later
reviews below found additional blocking gaps in that attempted correction.
Two fresh reviews of the later exact staged identities returned C1/I6/M1 and
C0/I3/M0. They rejected the all-disabled speculative stack, missing admission-
time freshness terminal, incomplete caller enforcement/evidence paths, a
nonexistent migration-test rename, contradictory v6/newer-version criteria,
an incoherent RED/GREEN test recipe and an inaccurate new-test file action.
Sections 0.2, 0.4, 1.1, 1.2, 2.0, 12, 14 and the matching plan/business-rule
row now repair those findings directly. The resulting identities require a
fresh independent Gate A review before implementation; Gate B/C/D and live
evidence remain pending.
The next exact reviews returned C0/I4/M0 and C1/I3/M1. They found that the
opaque permit was not implementably specified, expired rows could evade
terminalization, the written RED bodies actually passed, the first Gate-B file
action was not the required cutover-test creation, legacy/non-R-09 decisions
could bypass the producer catalog during retry, the fixed-HEAD multiline
command was invalid and incomplete, the clean HEAD dependency delta was not
frozen, and seven entrypoints were called six. Sections 1.1.2, 1.2, 2.1-2.4,
10 and the matching plan/business-rule row repair those findings; their new
identities require another fresh review.
The following fixed-object reviews returned C1/I7/M0 and C1/I3/M1. They found
that final pre-call no-call expiry had no legal durable attempt terminal, sink
results could race the expiry authority, cycle evidence remained
`Indeterminate`, Rule 2.3/R-09 empty semantics and fixed-HEAD caller/R-09
inventories were contradictory, freshness-clock and typed-outcome ownership
diverged, the public constant manifest was incomplete, and Task 1/Task 8
catalog ownership was reversed. Sections 0.2, 0.4, 1.1.2, 2.1-2.4, 4, 5, 9,
10.1 and the matching plan/business-rule row repair those findings. These
repairs have new identities and still require fresh independent C0/I0.
The next two independent reviews returned C1/I3/M0 and C2/I4/M0. They found a
wrong exact-row hash, floating fixed-source commands, an incomplete caller
inventory, an unconditional final pre-call recount, no exact Task-1 RED
command, BR-198 date and BR-200 occurrence-preflight contradictions, a
one-directional result/ownership relation, rollback exposure to legacy resume,
and a second begin-error-channel ambiguity. Sections 0.3-0.4, 1.1-1.2, 2.1,
4, 10-11 and 14 plus the matching plan/business-rule row repair those findings;
the new staged object identities require a fresh two-reviewer C0/I0 decision.
The subsequent three-way read-only precheck returned C2/I5/M0, C0/I1/M0 and
C0/I4/M1 across cross-rule, state-machine and executable-plan audits. It found
SourceOnly banner authority leakage, BR-198 dependency-closure drift, missing
BR-200 state mapping/rule IDs/capture tests, an unenforceable terminal-result
bijection, an unexecutable prerequisite exception, ambiguous initial-versus-
retry expiry, missing exact RED bodies, BR-202 Gate-D conflict and review-
history wording drift. Sections 0.1-0.4, 2, 10-12 and 14 plus the matching plan
and business-rule row repair those findings. Only the next exact staged
two-reviewer result may be called the current Gate-A decision.

### 0.2 Data-red-line applicability and Gate A DoD

Every Part 2 red line is a Gate A checkpoint even when this delivery-only slice
does not consume the corresponding financial domain:

| Rule | Applicability | Gate A DoD |
| --- | --- | --- |
| 2.1 Data source | Applies | Retry consumes only the already-frozen canonical envelope and calls zero Provider/Gateway/producer/renderer paths. Provider or storage failure is typed and never becomes mock/default/empty evidence. |
| 2.2 Missing data | Applies | Missing authorization, disposition, schedule, binding, audit ref, hash, timestamp or terminal payload remains missing and fails closed; no value is synthesized. |
| 2.3 Bad market data | Applies | The R-09 producer rejects an empty or partial pair, non-finite values, missing provider/source/metric/unit/date/order evidence, any row date unequal to the requested review-calendar business date, or internally inconsistent rows before rendering/freezing; verified empty is a typed `Failed`, never `NoData`. Retry does not recompute or reinterpret those validated immutable bytes. The price-positive, adjacent-change, time-continuity and split/dividend subchecks are N/A because this payload contains no price series. |
| 2.4 Market-data freshness | Applies | Retry acquires no new market data and must never relabel frozen evidence. `ReviewProviderTopN` persists its source business date and exact expiry; retry-candidate discovery, automatic retry admission and manual retry authorization reject `now >= expires_at`, atomically terminalize the retry schedule as audited `ExpiredFreshness`, and call zero sink. This retry expiry does not reject a BR-198 dispatcher-resolved prior-date initial acquisition. Manual authorization cannot extend or revive expiry. |
| 2.5 Test/live isolation | Applies | Production and invocation-unique `TEST_CODE` database, audit, push-log, lock and command authorities are physically disjoint as frozen in §0.3; cross-mode open is rejected before mutation. |
| 2.6 Order safety | N/A | This slice cannot create, validate or execute an order and has no cash/quantity/price authority. Its only external effect is delivery through the already-bound notification sink. |
| 2.7 Audit trail | Applies | Authorization, admission, ownership, uncertainty, cycle terminal slot and sink result are immutable/hash-bound and retained for at least five years. |
| 2.8 Fake implementation | Applies | `verify`, `reconcile`, `authorize`, `append` and delivery operations act on their named durable authorities; logging-only success is forbidden. |
| 2.9 Design contradiction | N/A to config | No `config/*.toml` field changes. The fixed 30/120/600-second retry schedule and cap of three are delivery-governance constants registered under BR-192, not financial thresholds or config overrides. |
| 2.10 Business rules | Applies | All retry filter, exact ordering, limits, mutex/fence, single-use ownership, terminal-slot and recovery rules are registered in the tracked BR-192 row before Gate B. |

The implementation PR must reproduce this table's applicability decisions in
its `Data-Redlines` evidence. An N/A entry is a scoped proof, not an omitted
checkpoint.

### 0.3 Self-contained adopted baseline

This tracked design/plan/business-rule triple is normatively self-contained.
Historical design files may explain provenance but are not prerequisites and
must not be cited to satisfy an acceptance criterion.

- The only upstream data eligible for the frozen
  `ReviewProviderTopN` envelope is one source-ordered, non-empty, complete
  `ProviderTopNRankings` page from
  `EastmoneyProviderTopNRankingRouter::new()` for the dispatcher's exact
  review-calendar `business_date`,
  canonical A-share identities, `limit=20`, and either `VolumeRatio` or
  `MainNetInflow`. The immutable envelope retains provider order, row business
  date, provider-declared total, inspected count, batch identity and observed
  time and explicitly says it is one response TopN, not a complete-market
  ranking. Retry never reacquires or rerenders it.
- Production SQLite, immutable audit and push-log authorities resolve only
  below the compile-time manifest root. Tests resolve only below one
  invocation-unique `data/test/TEST_CODE*` root which binds process identity
  and a per-invocation nonce. Runtime CWD, environment path overrides, caller
  paths, symbolic links, hard links or replaced ancestors cannot select or
  rebind an authority. Cross-mode or real-symbol TEST_CODE opens fail before
  mutation.
- SQLite uses `BEGIN IMMEDIATE`, WAL, `synchronous=FULL`, foreign keys and a
  five-second busy timeout. Immutable audit/push-log writers retain and verify
  their opened object identities, lock across validation/write/sync, reject
  partial tails or identity replacement, and never truncate production
  artifacts from a test.
- `DurableDeliveryCoordinator` remains the sole database authority, the
  immutable append port remains the sole audit authority, and the counted sink
  remains the sole external delivery authority. This corrective retry slice
  does not accept an external path, provider, renderer, clock, database,
  append port or sink selector on any production CLI.

### 0.4 Corrective release slice and real consumer

This Gate B slice is not provider-free infrastructure without a consumer. It
must create and enable exactly one real counted producer:
`push_templates::dispatch_r09_provider_top_n_outcome`, backed by
`CapitalDataGateway::provider_top_n_pair`. Those names are **TO BE BUILT**
against the fixed HEAD and are target-state contract, not evidence that dirty-
worktree code has been accepted. Candidate worktree code may be adopted only
after conformance to this design and all Gate B/C checks.

R-09 first performs the BR-194/BR-198 static date preflight and then the
accepted BR-200 durable occurrence preflight, both before catalog permit
acquisition, provider construction/acquisition, rendering or sink access.
BR-198 is not a separately executable prerequisite: this BR-192 Gate-B slice
owns its implementation atomically with the R-09 gateway/producer and unified
Magic dependency closure. The date preflight consumes an explicit
Asia/Shanghai observation created at the monitor context boundary; host-local
`chrono::Local` and the host `TZ` setting are not authorities. It rejects a
future review date as
`provider_top_n_future_date`; returns `ExpectedWait` for the current Shanghai
business date before 15:35; and permits the current date at/after 15:35 or the
dispatcher-resolved latest-settled prior review-calendar business date. This is
not a caller-selected arbitrary historical replay. Every returned ranking row
must bind its provider `f297` business date exactly to that requested review
date. The gateway records trusted `request_started_at` and
`capture_completed_at` observations,
preserves the provider capture timestamp's raw bytes, parses the complete
provider value, and requires `request_started_at <= provider_captured_at <=
capture_completed_at` in addition to business-date/Shanghai-midnight checks.
A same-date capture before request start or after completion is invalid.
Cached, current-date-substituted or fallback rows are forbidden. The durable
preflight
retains two closed `ProviderCaptureEvidenceV1` values, one per metric. Its
field manifest is exactly `raw_timestamp_bytes: Box<[u8]>`,
`parsed_timestamp: DateTime<FixedOffset>` and
`raw_timestamp_sha256: String`; the constructor/read validator requires
exactly 64 lowercase ASCII hex characters. The hash is
`SHA-256("stock_analysis.br198.provider_capture_raw.v1\0" ||
u64_be(raw_timestamp_bytes.len()) || raw_timestamp_bytes)`; parsing may not
trim, normalize, re-encode or replace the bytes. The closed pair field manifest
is exactly `volume_ratio_capture: ProviderCaptureEvidenceV1` followed by
`main_net_inflow_capture: ProviderCaptureEvidenceV1`; it is stored as the
single `capture_binding: ProviderTopNPairCaptureBindingV1` field in the
canonical counted binding. Compact serde JSON uses struct declaration order
and serializes each boxed byte slice as its exact integer array. There is no
second pair-only hash: the containing counted-binding canonical SHA-256 binds
the complete nested values, while read validation independently re-hashes both
raw fields. A byte mutation, even when it
parses to the same instant, invalidates the pair before durable prepare/sink.
calls `inspect_review_task_occurrence` for the exact business-date/R-09
occurrence. `Some(evidence)` maps exclusively through
`review_outcome_from_existing_durable`, and preflight error, missing hydration,
corrupt authority or ambiguous authority fails closed; all such branches use
zero permit/provider/renderer/sink. Only `None` may obtain the catalog-issued,
seam-bound `CountedProducerPermit`, call the provider, validate one complete
`ProviderTopNPair`, render once, freeze a `CountedDeliveryBinding`, and call the
durable counted entry. The full retry stack therefore has one concrete
consumer and one Gate-D receipt target. The other fourteen counted kinds
remain fail-closed. No second counted kind may be enabled in this slice.

The BR-200 occurrence mapping is executable and closed:

| occurrence evidence | R-09 outcome | retryable | next attempt | exact reason code | side effects |
| --- | --- | --- | --- | --- | --- |
| unique Delivered with valid hydration | reuse original Delivered | false | none | original durable reason | zero permit/provider/renderer/sink |
| Delivered with missing hydration | Failed | true | reconciliation schedule | `durable_occurrence_delivered_hydration_pending` | zero permit/provider/renderer/sink |
| Rejected or Uncertain | Failed | false | none | `durable_occurrence_terminal_failure` plus exact stored terminal state | zero permit/provider/renderer/sink |
| non-terminal decision | Failed | true | reconciliation schedule | `durable_occurrence_nonterminal_reconciliation_pending` | zero permit/provider/renderer/sink |
| corrupt, mismatched or ambiguous authority | Failed | false | none | exact typed durable invariant reason | zero permit/provider/renderer/sink |
| no occurrence | continue normal R-09 path | n/a | n/a | none | permit then provider then renderer then durable sink |

Producer, schedule-transition and hydration evidence for R-09 persist the
ordered rule ID vector exactly as `[BR-110, BR-140, BR-192, BR-194, BR-198,
BR-200]`. Missing, duplicate, reordered or additional IDs fail the verifier;
hydration must join the same frozen vector.

The consumer is reachable from the accepted BR-194 SourceOnly review phase,
not merely defined as a helper. Fixed HEAD already contains
`ReviewTask::R09`, its unique membership in the closed `ReviewTask::ALL` set
after `R08` and before `A10`, label `R-09`, and dependency class `SourceOnly`.
Gate B preserves those enum/`ALL`/label/classification bytes and semantics; it
only wires the real R-09 producer and its central dispatch under the combined
BR-194/BR-198/BR-200 rules above and the existing `--test --review`
dual-disable before any durable/provider access. The central
`dispatch_post_session_review` SourceOnly phase invokes
`dispatch_r09_provider_top_n_outcome(business_date, observed_at)` exactly once
when R09 is runnable and merges its typed outcome by stable `ReviewTask` order
with duplicate rejection. The function accepts no `BannerCtx`, banner text,
AccountMode value or broker snapshot, and neither reads nor constructs those
authorities.
The dispatcher returns `Failed` for provider-verified empty, missing, partial,
stale or invalid evidence, and `Delivered` only after the durable receipt is
confirmed. It never maps an empty dual batch to `NoData`. R-09 validates finite
values, non-empty dual
batches and exact provider/source/metric/unit/date/order evidence as frozen by
BR-194; missing fields remain missing and fail closed. Initial delivery for a
valid prior review-calendar date remains runnable even when the envelope's
retry expiry is already past; `expires_at` is still the first Asia/Shanghai
midnight after `source_business_date` and governs retry only. A rejected
closed-day initial delivery therefore has zero retry eligibility and expiry is
never extended. Gate A/B/C/PR evidence therefore names BR-192, BR-194, BR-198,
BR-200 and the future BR-202 release-evidence batch. Every planned BR-192
Gate-B source path cites literal `BR-202` so the later coverage owner can find
the accepted surface, but BR-192 does not claim or mutate the current BR-202
Code cell and does not treat the current BR-202 candidate as accepted
authority. The no-spec-on-unverified-gate rule postpones BR-202 Gate A and all
later BR-202 progression until BR-192 Gate C. That later, independently
accepted BR-202 Gate-A object may register the already-accepted BR-192 paths
and its Gate B may create the isolated wrapper. BR-192 Gate D is then minted
only by `tools/coverage/run_isolated_gate.sh`; raw coverage commands are
diagnostic.
BR-192 Gate B and release are blocked until BR-200 has independently accepted
Gate C evidence while R-09 remains disabled. BR-198's supporting design records
the date/capture contract but has no standalone Gate B/C progression: its code,
tests and evidence are part of this BR-192 Task-8/Gate-B atomic change because
the required R-09 artifacts do not exist before BR-192 creates them. Untracked
candidate implementations are not accepted authority, and no BR-198 status may
claim completion before the containing BR-192 gate is accepted.

Clean fixed HEAD has only a path-based `magic-tdx-rs` declaration and cannot
build this target by itself. The combined BR-192/BR-198 release closure must
preserve BR-198's complete unified dependency identity: fourteen direct Magic
manifest rows and exactly fifteen Magic lockfile packages, including only the
transitive `magic-market-transport`. Every package is version `0.2.0` from Git
repository `https://github.com/Northofqing/magic-market-data-rs.git` at exact
revision `5f1ce93656a55854c844065390520cd4aecd9a14`; the transport crate must not
become an application-level provider API. Task 8 therefore replaces the
fixed-HEAD path row and installs/verifies the complete fourteen-direct/fifteen-
lockfile release set atomically rather than shrinking it to the five packages
that R-09 itself imports. No dirty-worktree dependency row is an accepted
prerequisite, and this slice does not adopt unrelated capital gateway
capabilities merely because their crates are present.

## 1. Scope and invariants

This change lets a long-lived monitor retry a counted delivery only from the
already-frozen `DeliveryEnvelope`. A retry cycle must never call a Provider,
Gateway, producer or renderer. It may call exactly one already-bound
authoritative sink after winning transactional admission and after all
admission audit bytes have reached the immutable append authority.

The following are hard invariants:

1. Only `RejectedDurable` with exact persisted R-09 producer provenance from
   the current enabled catalog can be considered for retry. v5/null provenance
   and all fourteen disabled kinds are ineligible before automatic/manual
   authorization append.
2. A row-level boolean is never sufficient authority. Admission must validate
   the decision's unique active `retry_authorization_bindings` row, its
   appended and applied `retry_authorizations` record, the current appended
   rejection disposition and the exact appended `Applied` authorization event
   that authorized the binding CAS.
3. `UncertainAuditPending`, `UncertainTaskTransitionPending` and
   `UncertainManualReview` never enter the retry candidate list, never receive
   manual retry authorization and never reach a retry sink call.
4. `Delivered`, accepted/manual terminal states and manually rejected terminal
   states never retry.
5. One cycle-global attempted set covers both already-`Reserved` work and
   newly-admitted retry work. One decision identity can cause at most one sink
   call in a cycle, even if its state or reservation generation changes.
6. Cross-process correctness comes from SQLite `BEGIN IMMEDIATE`, retained
   reservation generations and attempt fencing, not from an in-process mutex.
7. Every admission result, including `Deferred` and `NoLongerEligible`, has a
   cycle-bound durable SQLite outbox record and an immutable append
   acknowledgement.
8. Test and production databases, WAL/SHM, push logs, immutable audit roots,
   event-audit roots and receipt roots remain physically isolated.
9. Every retry cycle is bound to the validated identity of the process-lifetime
   boot authority acquired before any coordinator/append authority is opened.
   The identity is passed explicitly into cycle creation; the coordinator never
   reads it from a global, environment variable or process helper.
10. A new cycle cannot coexist with any unresolved `retry_cycles.state='Running'`
    row. Before `begin_retry_cycle_before_spawn`, the guard owner resumes every
    same-boot `CompletionAppended|CompletionPending|FailureAppended|
    FailurePending` slot to its exact terminal CAS in the frozen total order
    below and with zero sink calls. In one `BEGIN IMMEDIATE`, begin then
    computes the next retained `cycle_ordinal`, derives the exact proposed
    cycle identity from the frozen identity preimage, and only then performs
    its global non-mutating Running-row check. If any unresolved row remains,
    the transaction proves zero proposed cycle/`Started` rows, rolls back and
    returns definite `RetryCycleAlreadyRunning` plus an exact bound
    `NoRetryCycleCommitted`; no insert or write occurs.
11. `ReviewProviderTopN` binds `source_business_date` and `expires_at` into the
    immutable envelope, decision, authorization binding and retry schedule.
    `expires_at` is the first Asia/Shanghai midnight strictly after
    `source_business_date`, represented as the exact corresponding UTC
    instant. Retry-candidate discovery excludes `now >= expires_at`; automatic
    retry admission and manual retry authorization recheck in the same
    `BEGIN IMMEDIATE` transaction. Initial R-09 acquisition is outside this
    retry-expiry gate and follows the BR-198 review-date preflight above.
    An expired eligible/rejected row transitions once to retained terminal
    `ExpiredFreshness`, appends/acknowledges immutable
    `RetryExpiredFreshness`, clears active authorization/schedule ownership,
    and performs zero provider, renderer and sink calls. Manual authority
    cannot change this instant or revive the row.

No financial, market-data or order threshold changes. Retry scheduling
constants govern delivery recovery only.

### 1.1 Closed counted-producer startup catalog

Gate B must materialize one immutable startup catalog whose key set is exactly
`PushKind::ALL`. Every one of the 15 counted kinds appears exactly once and is
in exactly one closed state:

- `EnabledDurableBinding { producer_seam }` means the named production seam
  constructs a real immutable `CountedDeliveryBinding` or canonical durable
  envelope before it can enter the counted sink;
- `DisabledNoProducer { reason_code }` means no admitted producer currently
  satisfies that evidence contract. It must fail visibly before provider/data
  acquisition and before any sink call. It may not fall back to the generic
  governor or synthesize a binding.

Startup validates the catalog before any counted producer, acquisition or
sink is enabled. A missing kind, duplicate kind, unknown state, empty producer
seam or empty reason is fatal. After validation it emits exactly one line per
kind in `PushKind::ALL` order. Enabled lines use this exact schema:

The accepted BR-200 prerequisite deliberately leaves
`ReviewTask::R09.br200_production_capability()` at
`ReviewTaskProductionCapability::DisabledNoProducer`. BR-192 Gate B must change
that capability to `EnabledSourceOnly` in the same atomic Task-8 source state
that installs the exact enabled catalog row, `CapitalDataGateway` pair loader
and complete R-09 producer. Changing only the capability, only the catalog or
only the producer is an invalid partial release and must fail tests/checkers
before provider I/O. R-04/R-08 capability values remain unchanged.

```text
[BR-192][counted-producer] push_kind=<PushKind> enabled=durable_binding producer=<producer_seam>
```

Disabled lines use this exact schema:

```text
[BR-192][counted-producer] push_kind=<PushKind> disabled=no_producer reason=capability_unavailable:<reason_code>
```

The first Gate B catalog is frozen as follows. This is a target-state contract,
not a claim of Gate D production evidence:

| PushKind | Startup state | Exact producer seam or reason code |
| --- | --- | --- |
| `HoldingPlan` | `DisabledNoProducer` | `holding_plan_counted_binding_unavailable` |
| `HoldingEvent` | `DisabledNoProducer` | `holding_event_counted_binding_unavailable` |
| `T0Advice` | `DisabledNoProducer` | `t0_advice_counted_binding_unavailable` |
| `CandidateTriggered` | `DisabledNoProducer` | `candidate_triggered_counted_binding_unavailable` |
| `CloseCall` | `DisabledNoProducer` | `close_call_counted_binding_unavailable` |
| `ForbiddenOps` | `DisabledNoProducer` | `forbidden_ops_counted_binding_unavailable` |
| `PaperTrade` | `DisabledNoProducer` | `paper_trade_counted_binding_unavailable` |
| `ReviewMarket` | `DisabledNoProducer` | `provider_capability_not_live_admitted` |
| `ReviewLhb` | `DisabledNoProducer` | `review_lhb_counted_binding_unavailable` |
| `ReviewSignal` | `DisabledNoProducer` | `review_signal_counted_binding_unavailable` |
| `ReviewFailure` | `DisabledNoProducer` | `review_failure_counted_binding_unavailable` |
| `TomorrowWatch` | `DisabledNoProducer` | `tomorrow_watch_counted_binding_unavailable` |
| `EventCalendar` | `DisabledNoProducer` | `event_calendar_counted_binding_unavailable` |
| `DailyReport` | `DisabledNoProducer` | `daily_report_counted_binding_unavailable` |
| `ReviewProviderTopN` | `EnabledDurableBinding` | `push_templates::dispatch_r09_provider_top_n_outcome` |

The enabled seam is a Gate-B deliverable and has no accepted fixed-HEAD
implementation. Gate B must build it together with
`CapitalDataGateway::provider_top_n_pair`; until that atomic change passes,
startup remains blocked rather than silently treating the row as enabled.

#### 1.1.1 Rejected worktree snapshot (non-authoritative history)

The following 2026-08-01 output is retained only to make the rejected metadata
attempt auditable. It came from a dirty worktree, is not a fixed-HEAD fact and
does not authorize any enabled catalog row. In particular, the R-08/R-09 names
have since changed even in the worktree. None of these lines may be cited by
Gate B/C/D or by a later design.

```text
$ nl -ba src/durable_delivery/model.rs | sed -n '155,192p'
   155	#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
   156	#[serde(rename_all = "PascalCase")]
   157	pub enum PushKind {
   158	    HoldingPlan,
   159	    HoldingEvent,
   160	    T0Advice,
   161	    CandidateTriggered,
   162	    CloseCall,
   163	    ForbiddenOps,
   164	    PaperTrade,
   165	    ReviewMarket,
   166	    ReviewLhb,
   167	    ReviewSignal,
   168	    ReviewFailure,
   169	    TomorrowWatch,
   170	    EventCalendar,
   171	    DailyReport,
   172	    ReviewProviderTopN,
   173	}
   174
   175	impl PushKind {
   176	    pub const ALL: [Self; 15] = [
   177	        Self::HoldingPlan,
   178	        Self::HoldingEvent,
   179	        Self::T0Advice,
   180	        Self::CandidateTriggered,
   181	        Self::CloseCall,
   182	        Self::ForbiddenOps,
   183	        Self::PaperTrade,
   184	        Self::ReviewMarket,
   185	        Self::ReviewLhb,
   186	        Self::ReviewSignal,
   187	        Self::ReviewFailure,
   188	        Self::TomorrowWatch,
   189	        Self::EventCalendar,
   190	        Self::DailyReport,
   191	        Self::ReviewProviderTopN,
   192	    ];
```

T0Advice constructs and then transfers the immutable binding:

```text
$ nl -ba src/bin/monitor/main.rs | sed -n '6077,6096p;7477,7483p'
  6077	                let binding = durable_delivery_runtime::CountedDeliveryBinding::new(
  6078	                    business_date,
  6079	                    decision_id,
  6080	                    source_binding_canonical,
  6081	                    durable_delivery_runtime::CountedDeliveryScope::Ticket {
  6082	                        instrument: decision_binding.instrument().clone(),
  6083	                    },
  6084	                    delivery_subject_hash,
  6085	                    durable_delivery_runtime::CountedDeliveryOrigin::Provider {
  6086	                        observed_at: Some(evidence.observed_at),
  6087	                        as_of: Some(business_date),
  6088	                        ordered_batch_ids: vec![decision_binding.evidence_batch_id().to_owned()],
  6089	                    },
  6090	                    None,
  6091	                    true,
  6092	                )?;
  6093	                messages.push(PreparedT0Advice {
  6094	                    code: plan.code,
  6095	                    text,
  6096	                    binding,
  7477	                                        let outcome = notify::push_counted_with_binding(
  7478	                                            &prepared.text,
  7479	                                            notify::PushKind::T0Advice,
  7480	                                            None,
  7481	                                            prepared.binding,
  7482	                                        )
  7483	                                        .await;
```

PaperTrade constructs and then transfers the immutable binding:

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '5503,5514p;5545,5551p'
  5503	            let binding = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
  5504	                report.terminal_binding.business_date(),
  5505	                schedule_occurrence_identity,
  5506	                source_binding_canonical,
  5507	                crate::durable_delivery_runtime::CountedDeliveryScope::Ticket {
  5508	                    instrument: report.terminal_binding.instrument().clone(),
  5509	                },
  5510	                delivery_subject_hash,
  5511	                crate::durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
  5512	                None,
  5513	                true,
  5514	            )?;
  5545	        match crate::notify::push_counted_with_binding(
  5546	            &item.text,
  5547	            crate::notify::PushKind::PaperTrade,
  5548	            None,
  5549	            item.binding,
  5550	        )
  5551	        .await
```

ReviewLhb constructs a provider/task binding and transfers it through the
source-only counted entry:

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '9070,9096p'
  9070	    let counted_binding = match crate::durable_delivery_runtime::CountedDeliveryBinding::new(
  9071	        prepared.business_date,
  9072	        prepared.task_identity,
  9073	        prepared.source_binding_canonical,
  9074	        crate::durable_delivery_runtime::CountedDeliveryScope::Global,
  9075	        prepared.delivery_subject_identity,
  9076	        crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
  9077	            observed_at: Some(prepared.provider_observed_at),
  9078	            as_of: Some(prepared.business_date),
  9079	            ordered_batch_ids: vec![prepared.batch_id],
  9080	        },
  9081	        Some(task_binding),
  9082	        true,
  9083	    ) {
  9084	        Ok(binding) => binding,
  9085	        Err(reason) => {
  9086	            log::error!("[R-04][BR-140][BR-192] counted binding rejected: {reason}");
  9087	            log_dispatcher_attempt("R-04", false, entries.len(), &reason);
  9088	            return ReviewTaskOutcome::failed(false, reason);
  9089	        }
  9090	    };
  9091	    let push_result = crate::notify::push_counted_source_only_with_binding(
  9092	        &prepared.rendered,
  9093	        crate::notify::PushKind::ReviewLhb,
  9094	        counted_binding,
  9095	    )
  9096	    .await;
```

EventCalendar constructs a provider/task binding and transfers it through the
generic counted entry:

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '8017,8044p'
  8017	    let counted_binding = match crate::durable_delivery_runtime::CountedDeliveryBinding::new(
  8018	        prepared.business_date,
  8019	        prepared.task_identity,
  8020	        prepared.source_binding_canonical,
  8021	        crate::durable_delivery_runtime::CountedDeliveryScope::Global,
  8022	        prepared.delivery_subject_identity,
  8023	        crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
  8024	            observed_at: Some(prepared.provider_observed_at),
  8025	            as_of: Some(prepared.business_date),
  8026	            ordered_batch_ids: prepared.ordered_batch_ids,
  8027	        },
  8028	        Some(task_binding),
  8029	        true,
  8030	    ) {
  8031	        Ok(binding) => binding,
  8032	        Err(reason) => {
  8033	            log::error!("[R-08][BR-140][BR-192] counted binding rejected: {reason}");
  8034	            log_dispatcher_attempt("R-08", false, prepared.item_count, &reason);
  8035	            return ReviewTaskOutcome::failed(false, reason);
  8036	        }
  8037	    };
  8038	    let push_result = crate::notify::push_counted_with_binding(
  8039	        &prepared.rendered,
  8040	        crate::notify::PushKind::EventCalendar,
  8041	        None,
  8042	        counted_binding,
  8043	    )
  8044	    .await;
```

ReviewProviderTopN constructs the canonical task/provider envelope and passes
it to the only public durable envelope entry:

```text
$ nl -ba src/bin/monitor/push_templates.rs | sed -n '6386,6406p;6517,6526p'
  6386	    let envelope = DeliveryEnvelope::new(
  6387	        prepared.binding.business_date.clone(),
  6388	        PushKind::ReviewProviderTopN,
  6389	        DeliverySubKind::None,
  6390	        "GLOBAL",
  6391	        prepared.binding.review_task_identity.clone(),
  6392	        prepared.binding.source_evidence_fingerprint.clone(),
  6393	        source_binding_canonical,
  6394	        prepared.binding.delivery_subject_identity.clone(),
  6395	        prepared.binding.rendered_content.clone(),
  6396	        false,
  6397	        Some(task_binding),
  6398	    )
  6399	    .and_then(|envelope| {
  6400	        envelope.with_provider_evidence(
  6401	            provider_observed_at,
  6402	            Some(prepared.binding.business_date.clone()),
  6403	            original_batch_ids,
  6404	        )
  6405	    })
  6406	    .map_err(|error| format!("R-09 durable delivery envelope rejected: {error}"))?;
  6517	    let envelope = match build_r09_delivery_envelope(&prepared) {
  6518	        Ok(envelope) => envelope,
  6519	        Err(error) => {
  6520	            return ReviewTaskOutcome::failed(
  6521	                false,
  6522	                format!("provider_top_n delivery envelope rejected: {error}"),
  6523	            );
  6524	        }
  6525	    };
  6526	    match crate::durable_delivery_runtime::deliver_envelope(envelope).await {

$ nl -ba src/bin/monitor/durable_delivery_runtime.rs | sed -n '817,824p'
   817	pub async fn deliver_envelope(
   818	    envelope: DeliveryEnvelope,
   819	) -> Result<DurableDispatchEvidence, String> {
   820	    ensure_startup_reconciled().await?;
   821	    let state = runtime_state()?;
   822	    tokio::task::spawn_blocking(move || deliver_envelope_blocking(state.as_ref(), envelope))
   823	        .await
   824	        .map_err(|error| format!("BR-192 counted delivery join failed: {error}"))?
```

#### 1.1.2 Authoritative fixed-HEAD baseline and target delta

The baseline is
`HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496`. The enabled R-09 row is the
explicit Gate-B target delta in §0.4, not a claim about this baseline. Its
bounded source blobs are:

```text
src/durable_delivery/model.rs                 1b5561865674a09266971469f703649c8d299c38
src/bin/monitor/main.rs                       80be9ddea0eb088194e2daab9a40bfa3067f00a5
src/bin/monitor/push_templates.rs             2388dce7887a95feee13eadbef6129efdb942f61
src/bin/monitor/notify.rs                     b7a15cbd46ef7620ec341d2eee98dd76f62560d8
src/bin/monitor/review_batch.rs               99d89da9454a13af6adc52cc239cc690b8770029
src/bin/monitor/v14_adapter.rs                535d17d964b40f97fa89adb765a0ecb9f02441bf
src/bin/monitor/durable_delivery_runtime.rs   a635b90237413577a51d5bc92ae29c40ae2afac4
src/durable_delivery/schema.rs                794491f8445374af44ee52e57ba2358db7f9c262
```

Reproduce the source binding and the 15-value closed key set with:

```bash
git rev-parse b4aeee68d2c0259cc968914b3d39e3a89a18a496
git rev-parse \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/model.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/main.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/push_templates.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/notify.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/review_batch.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/v14_adapter.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/durable_delivery_runtime.rs \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/schema.rs
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/model.rs | sed -n '155,192p'
```

Fixed HEAD has no accepted R-09 producer and still exposes generic counted
entrypoints. The baseline audit therefore inventories actual entrypoints and
every counted kind; it is not a three-name zero-match proof:

```bash
for name in push_governor push_governor_v3 push_governor_v3_with_sub_kind \
            dispatch dispatch_outcome deliver_counted_binding deliver_envelope; do
  git grep -n -E "(fn|\\.)[[:space:]]*${name}[[:space:]]*\\(" b4aeee68d2c0259cc968914b3d39e3a89a18a496 -- \
    src/bin/monitor/main.rs src/bin/monitor/review_batch.rs \
    src/bin/monitor/push_templates.rs src/bin/monitor/notify.rs \
    src/bin/monitor/v14_adapter.rs src/bin/monitor/durable_delivery_runtime.rs || true
done
for kind in $(git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/model.rs |
  sed -n '/pub const ALL:/,/];/p' |
  sed -n 's/.*Self::\([A-Za-z0-9_]*\),/\1/p'); do
  git grep -n -E "PushKind::${kind}|\"${kind}\"" b4aeee68d2c0259cc968914b3d39e3a89a18a496 -- src/bin/monitor || true
done
for path in src/bin/monitor/main.rs src/bin/monitor/review_batch.rs \
            src/bin/monitor/push_templates.rs src/bin/monitor/notify.rs \
            src/bin/monitor/v14_adapter.rs \
            src/bin/monitor/durable_delivery_runtime.rs; do
  git show "b4aeee68d2c0259cc968914b3d39e3a89a18a496:${path}" | rg -n -U \
    'PushKind::(HoldingPlan|HoldingEvent|T0Advice|CandidateTriggered|CloseCall|ForbiddenOps|PaperTrade|ReviewMarket|ReviewLhb|ReviewSignal|ReviewFailure|TomorrowWatch|EventCalendar|DailyReport|ReviewProviderTopN)[\s\S]{0,1200}(push_governor|dispatch|deliver_counted_binding|deliver_envelope)' || true
done
```

The checked-in Gate-B compliance checker replaces this exploratory inventory
with a syntax-aware/multiline-aware, `cfg(test)`-aware closed classification and
fails if any of the 15 kinds is absent from its report. The complete fixed-HEAD
classification below records every direct production call to the seven
generic/durable entries per kind unless it is explicitly marked test-only;
line numbers are bound to the source blobs above.
`support-only` means the kind exists in mapping or hydration code but has no
producer call into any of the seven generic/durable entries. Test-only callers
below are not production evidence and the checked-in classifier must exclude
the lexical `#[cfg(test)] mod tests` region rather than promoting its calls.

| Kind | Complete fixed-HEAD direct producer/caller classification |
| --- | --- |
| `HoldingPlan` | `push_templates.rs:2103` `dispatch_outcome` |
| `HoldingEvent` | `push_templates.rs:2122` `dispatch`; `main.rs:7783` `push_governor_v3`; `main.rs:7863` `notify::push_governor_v3` |
| `T0Advice` | `push_templates.rs:2140,2146` `dispatch`; `main.rs:8107` `notify::push_governor_v3` |
| `CandidateTriggered` | `push_templates.rs:7408` `dispatch` |
| `CloseCall` | test-only: `push_templates.rs:13985` `push_governor`; no fixed-HEAD production caller proven |
| `ForbiddenOps` | test-only: `push_templates.rs:13882` generic counted caller; no fixed-HEAD production caller proven |
| `PaperTrade` | `push_templates.rs:4932` `dispatch` |
| `ReviewMarket` | `main.rs:4638` `push_governor` |
| `ReviewLhb` | `main.rs:5452` `push_governor`; `push_templates.rs:6790` `dispatch_outcome` |
| `ReviewSignal` | `main.rs:5484,5679,8200` `push_governor` |
| `ReviewFailure` | test-only: `push_templates.rs:14246` `push_governor`; no fixed-HEAD production caller proven |
| `TomorrowWatch` | `main.rs:4689` `push_governor` |
| `EventCalendar` | `main.rs:4809` `push_governor`; `push_templates.rs:6481` `dispatch_outcome` |
| `DailyReport` | `main.rs:701,4592,6685,8550,8558,8567,9017` `push_governor_v3`/`push_governor_v3_with_sub_kind` |
| `ReviewProviderTopN` | support-only: `durable_delivery_runtime.rs:1661,1703`, `v14_adapter.rs:822`; zero producer call |

No production `ReviewProviderTopN` producer/dispatcher exists in fixed HEAD.
Enum mappings, hydration and tests are consumer/support code. Gate B must
create the named R-09 producer and gateway atomically with the permit guard;
a worktree-only wrapper, presentation token or delivery call is not accepted
evidence and cannot enable startup by itself.

An enabled catalog row still requires Gate D proof from a real push-log and
delivery audit join; otherwise release remains blocked rather than silently
reclassifying it as operational. A disabled row must emit its exact startup
banner even when no attempt occurs. If a disabled path is invoked later, it
must emit the same reason code and return before acquisition/sink. Enabling
any row other than the one R-09 row above is a business-rule change: update
BR-192 and this table first, name the real producer seam, add an end-to-end
binding test, and obtain fresh Gate A review.

### 1.2 Exhaustive permit and caller enforcement

The catalog is executable authority, not a startup banner registry. Its only
successful admission product is a non-serializable, non-cloneable
`CountedProducerPermit` bound to exact `PushKind`, exact `producer_seam` and
the validated process-lifetime catalog identity. Only the catalog module may
construct it. `ReviewProviderTopN` must acquire the permit before invoking
`CapitalDataGateway::provider_top_n_pair`; every disabled counted-specific
loader, including R-04 and R-08, checks the catalog before its provider call
and returns its exact reason without acquisition.

Every production counted entrypoint is covered, including multiline calls and
indirection through template IDs or labels. The enforced set is:

```text
notify::{push_governor,push_governor_v3,push_governor_v3_with_sub_kind}
push_templates::{dispatch,dispatch_outcome}
durable_delivery_runtime::{deliver_counted_binding,deliver_envelope}
all counted-specific loaders/callers in main.rs, review_batch.rs,
push_templates.rs, notify.rs and v14_adapter.rs
```

This is seven generic/durable entrypoints: three governors, two dispatchers
and two durable-delivery entries.

The generic governor/dispatch entries cannot manufacture a permit. For a
counted kind they accept only a previously validated durable binding carrying
the same catalog permit identity; otherwise they return exact
`counted_binding_required` before sink. `deliver_counted_binding` and
`deliver_envelope` revalidate kind/seam/permit/binding before reservation or
sink. This closes direct calls as well as startup wiring. “Before acquisition”
means before any provider/data call caused solely by that counted producer;
shared market data acquired for an independent non-counted business purpose
is not reclassified, but it cannot reach a counted sink without the permit.

The owning module and crossing API are frozen, not left to implementation
choice. `src/durable_delivery/counted_producer_catalog.rs` is the single
library authority used by the monitor, coordinator and production
authorization CLI. The module remains private, while only the exact opaque
types/functions listed in the root manifest are re-exported:

Implementation ownership is also exact. Task 1 privately creates the catalog
module, closed catalog, permit, attestation, denial, acquisition function and
the private `CountedDeliveryBinding::new_permitted` consumption wiring, and may
use them only through private library paths while that task is incomplete.
Task 1 may edit the durable-delivery root only to add the private
`mod counted_producer_catalog;` declaration; it does **not** change the public
root surface or expose a partial public API. Task 8 is the sole owner of the
atomic public root re-export and monitor/CLI integration edit: it re-exports
the already-defined items byte-for-byte from the final manifest in §9 and then
connects their production consumers. No intermediate task may duplicate,
rename or independently construct the permit.

```rust
pub struct CountedProducerPermit {
    push_kind: PushKind,
    producer_seam: &'static str,
    catalog_identity_sha256: String,
    private: CountedProducerPermitPrivate,
}

struct CountedProducerPermitPrivate(());

pub struct CountedProducerAttestation {
    push_kind: PushKind,
    producer_seam: String,
    catalog_identity_sha256: String,
    attestation_sha256: String,
    source_business_date: NaiveDate,
    expires_at: DateTime<Utc>,
}

pub enum CountedProducerDenied {
    CatalogInvalid { reason_code: &'static str },
    ProducerNotEnabled {
        push_kind: PushKind,
        producer_seam: String,
        reason_code: &'static str,
    },
}

pub fn acquire_counted_producer_permit(
    push_kind: PushKind,
    producer_seam: &'static str,
) -> Result<CountedProducerPermit, CountedProducerDenied>;

impl CountedProducerPermit {
    pub fn into_attestation(
        self,
        source_business_date: NaiveDate,
        expires_at: DateTime<Utc>,
    ) -> Result<CountedProducerAttestation, CountedProducerDenied>;
}
```

`CountedProducerPermit` has private fields and constructor, implements no
`Clone`, `Copy`, `Serialize`, `Deserialize` or `Default`, and is consumed once
by `CountedDeliveryBinding::new_permitted(permit, ...)`. That constructor is
the sole non-test binding constructor and persists a private
`CountedProducerAttestation`; fixed HEAD's public `CountedDeliveryBinding::new`
is removed from production visibility. A cfg(test)-only factory can mint a
TEST_CODE permit but cannot produce a production catalog identity. Compile-
fail contract tests prove external/private construction, `.clone()`, serde
serialization and use of the old constructor all fail.

The public permit, attestation and acquisition items above are usable across
the library/binary crate boundary, but only the permit and attestation are
authority-bearing opaque values: their fields and all constructors stay private
to the library module. `CountedProducerDenied` is intentionally a public,
constructible closed error enum, not an authority value; constructing or
matching a denial can never mint a permit or attestation. The private marker is
exactly the zero-sized
`CountedProducerPermitPrivate(())`; no constructor or field is visible outside
the catalog module. The sole permitted production caller of the public
consuming method is `CountedDeliveryBinding::new_permitted`; the all-caller
checker rejects any other call. That constructor calls
`CountedProducerPermit::into_attestation(self, source_business_date,
expires_at)` internally. The method validates the current catalog again,
requires `expires_at` to be the first
Asia/Shanghai midnight after `source_business_date`,
computes the exact attestation hash and returns the opaque
`CountedProducerAttestation`; the attestation implements no `Deserialize` or
public constructor and exposes only read-only accessors used by
`new_permitted`. `CountedProducerDenied` is the closed enum above: catalog
shape/hash failures use `CatalogInvalid`, while disabled kinds and seam
mismatch use `ProducerNotEnabled` with the catalog's exact reason code. No
caller may convert either case to a successful binding or infer a permit from
persisted bytes.

The coordinator and `authorize_delivery_retry_production` call the same
module-private `validate_persisted_counted_producer_attestation`; there is no
second catalog copy in a binary. This validator recomputes the exact catalog
and attestation hashes from the persisted companion row, accepts only the
enabled R-09 tuple, and returns typed `RetryProducerNotEnabled` otherwise.

`CountedDeliveryBinding` freezes
`push_kind,producer_seam,catalog_identity_sha256,attestation_sha256` into the
canonical `DeliveryEnvelope`. Hash construction uses a separate private
`CountedProducerAttestationPreimageV1` which contains exactly
`schema_version,rule_id,push_kind,producer_seam,
producer_catalog_identity_sha256,source_business_date,expires_at` in that
declared order and contains no digest field. The stored
`attestation_canonical` BLOB and hash preimage are the literal domain bytes
`stock_analysis.durable_delivery.br192.counted_producer_attestation.v1\0`
followed by compact UTF-8 canonical JSON for that preimage.
`producer_attestation_sha256=sha256_hex(attestation_canonical)` is computed
first and is then copied into the separate persisted/envelope evidence
projection. The evidence projection is never itself the hash preimage. The
private validator rebuilds the preimage from the projection, requires and
strips the exact prefix, decodes and reserializes the JSON suffix byte-for-byte,
recomputes the digest and compares projection, typed columns and SQL UDF;
domain omission, duplication, alternate prefix or digest self-inclusion fails
closed.
The startup catalog identity is the lowercase SHA-256 of its exact 15 ordered
rows under domain
`stock_analysis.durable_delivery.br192.counted_producer_catalog.v1\0`.
`deliver_counted_binding` consumes the binding and produces a private
`PermittedDeliveryEnvelope`; raw `deliver_envelope(DeliveryEnvelope)` loses
production visibility and the private envelope entry accepts only that
wrapper. It rederives the catalog and attestation hashes and requires the
active catalog row to be the same enabled kind/seam before reservation or
sink. Terminal replay remains a separate typed replay API and cannot create a
new counted delivery.

`DeliveryEnvelope` gains one private compatibility-safe field owned by
`src/durable_delivery/model.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CountedProducerAttestationPreimageV1 {
    schema_version: u8,
    rule_id: String,
    push_kind: PushKind,
    producer_seam: String,
    producer_catalog_identity_sha256: String,
    source_business_date: NaiveDate,
    expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CountedProducerAttestationEvidenceV1 {
    schema_version: u8,
    rule_id: String,
    push_kind: PushKind,
    producer_seam: String,
    producer_catalog_identity_sha256: String,
    producer_attestation_sha256: String,
    source_business_date: NaiveDate,
    expires_at: DateTime<Utc>,
}

// inside DeliveryEnvelope
#[serde(default, skip_serializing_if = "Option::is_none")]
counted_producer_attestation: Option<CountedProducerAttestationEvidenceV1>,
```

The sole setter is a public consuming
`DeliveryEnvelope::with_counted_producer_attestation(attestation)` whose only
permitted production caller is `CountedDeliveryBinding::new_permitted`; the
all-caller checker rejects every other caller. It validates kind, business
date, exact expiry and attestation hash before storing the private projection.
Legacy v5 canonical bytes deserialize to `None` and reserialize byte-identically
because `None` is skipped. The coordinator reads this private projection to
insert `counted_producer_attestations` before the decision in the same
transaction. No public raw-field setter, serde builder or binary-local mirror
exists.

Only the attestation bytes—not the live permit—are persisted. Provider-free
retry revalidates those bytes against the active catalog. A v5 row or any
envelope without the four exact attestation fields is permanently typed
`RetryProducerNotEnabled { reason_code: "legacy_unpermitted_producer" }`;
migration leaves the companion row absent and never backfills or promotes it. Manual
authorization, automatic authorization,
candidate discovery and transactional admission all require exact
`PushKind::ReviewProviderTopN`, seam
`push_templates::dispatch_r09_provider_top_n_outcome`, current catalog hash
and attestation hash before creating/applying authorization or reserving a
sink attempt. The other fourteen kinds return typed
`RetryProducerNotEnabled` with zero authorization append, provider, renderer
or sink. `RetryCandidate` carries all four values so admission can rederive
them; it never trusts only a decision identity.

Gate B generates a multiline-aware, reproducible inventory over every value in
`PushKind::ALL`, all seven generic entry names above, template IDs and display
labels. The checker fails on an unclassified counted call, a missing kind, an
extra public counted entry, a disabled loader that acquires first, or an
enabled seam other than exact R-09. Single-line grep or a three-name zero-match
claim is insufficient.

## 2. Authoritative data model

### 2.0 Versioned additive migration

Fixed HEAD is already at durable-delivery SQLite `SCHEMA_VERSION=5`.
Schema v5 owns every pre-BR-192 authority, including the BR-194 terminal-replay
attempt/completion tables, both replay audit kinds, their foreign keys and the
fixed authority-manifest semantics. BR-192 adds exactly one `v5 -> v6`
migration; it must not reuse v5, rewrite the accepted v4-to-v5 migration,
allocate a competing schema version, or drop/rebuild/weaken any BR-194 object
or audit kind. Schema initialization has seven explicit paths, all inside the
coordinator's
existing migration transaction:

- a fresh `user_version=0` database creates the complete v6 schema directly;
- v1 migrates `v1 -> v2 -> v3 -> v4 -> v5 -> v6` without skipping validation;
- v2 migrates `v2 -> v3 -> v4 -> v5 -> v6`;
- v3 migrates `v3 -> v4 -> v5 -> v6`;
- v4 runs the accepted `v4 -> v5` migration, validates v5, then migrates to v6;
- v5 runs the one additive `v5 -> v6` migration; and
- v6 performs validation only and is safe to initialize repeatedly.

Any `user_version > 6` fails closed before schema mutation. SQLite cannot add a
validated foreign-key-bearing current-authorization column to the existing
`delivery_decisions` table with this migration's required semantics. The
v5-to-v6 migration therefore does **not** use `ALTER TABLE ... ADD COLUMN` for
that reference and does not rebuild the live decision authority. It creates the
v6 companion authorities `retry_authorization_bindings` and
`retry_attempt_bindings` plus authorization, authorization-event, schedule,
cycle, immutable cycle-failure-payload and cycle-audit objects. The unique
`Active` companion binding is the current authorization reference.

Fresh v6 and every upgrade path have the same manifest: neither has a
`delivery_decisions.current_retry_authorization_identity` column, and both have
the same companion tables, foreign keys, partial unique index and lifecycle
triggers. The v6 manifest contains every byte-compatible v5 BR-194 replay
table/index/audit-kind/trigger definition unchanged. BR-192 must not silently
redefine an accepted BR-194 trigger; any shared hash strengthening requires a
separately registered rule and fresh review. In every path
`retry_cycles.cycle_ordinal` is
`INTEGER NOT NULL UNIQUE CHECK(cycle_ordinal >= 1)`; ordinal/identity/
canonical-preimage fields are immutable, the retained row is nondeletable and
cycle insertion must rederive the identity from the exact ordinal and frozen
fields. The v5-to-v6 transaction:

1. begins with foreign-key enforcement enabled and snapshots all existing
   v5 schema objects, BR-194 replay/audit definitions, decision identities, row
   counts and canonical/hash columns;
2. creates every v6 BR-192 companion object without rewriting pre-existing v5
   objects;
3. deliberately copies **zero** legacy `retry_authorized` booleans into the new
   authority tables, because a boolean is not authorization evidence;
4. leaves all v5 data, including BR-194 replay attempts/completions and replay
   audit rows, in place and verifies their snapshotted values are byte-for-byte
   unchanged;
5. validates the complete fresh-v6 manifest, `foreign_key_check`, indexes,
   deterministic function-backed triggers and all new-table invariants; and
6. sets `user_version=6` only as the final statement before commit.

Any create, snapshot comparison, data-preservation or validation failure rolls
the whole migration back, including `user_version`. Object detection is used
only to diagnose an invalid/repeated initialization; a database claiming v5
with a missing or incompatible required baseline object fails before migration,
and a database claiming v6 with a missing or incompatible object fails closed.

Because historical v2/v3/v4/v5 binaries reject or cannot validate the newer
schema contract, rollback must not launch them against a v6 database. Section
14 uses
a newly built forward-compatible rollback binary that understands and validates
v6, retains all v5 baseline and v6 objects and records, but restores the
previous runtime behavior
with the retry runner disabled.

#### 2.0.1 Shared deterministic SQLite SHA-256 authority

BR-192 and BR-194 share one physically implementable database hash contract.
`Cargo.toml` enables rusqlite's `functions` feature in addition to `chrono`.
`src/durable_delivery/schema.rs` owns exactly one central
`register_durable_sql_functions(&Connection)` seam. Rust/rusqlite production,
invocation-unique TEST_CODE, fresh-schema, migrated, reopened and migration-
fixture connections call that seam at one descriptor-attestation-safe point:

```text
open SQLite handle
  -> attest and retain the main database descriptor
  -> run only the audited journaling-capability bootstrap sequence:
     install the fixed 5-second busy timeout; set journal_mode=WAL; set
     synchronous=FULL; execute BEGIN IMMEDIATE; ROLLBACK; read back only
     journal_mode and synchronous
  -> materialize WAL/SHM through that sequence
  -> re-attest main and attest/retain WAL/SHM
  -> validate the complete bound live connection
  -> register and self-test sha256_hex
  -> configure the attested connection
  -> inspect/create/migrate/validate schema
```

Before the existing main/WAL/SHM descriptor binding is complete, the sole
exception is the audited journaling-capability bootstrap sequence above, whose
only purpose is to materialize and validate WAL/SHM. No UDF callback, other
PRAGMA/configuration, schema read, DDL or application transaction/query may run
before complete binding. No schema inspection, migration, trigger execution or
application query runs before UDF registration/self-test. The sequence is
centralized, source-checked and tested to reject any additional statement. It
may not enable foreign keys, read `user_version`, `sqlite_master` or application
rows, execute DDL, register/invoke a UDF, or perform another PRAGMA/query/
transaction. Omitting or reordering any of its five steps fails sidecar
attestation and bootstrap. No caller may register an alias or a different
callback.

The registered scalar function has this frozen contract:

```text
name: sha256_hex
arity: 1
input: non-NULL SQLite BLOB
output: lowercase 64-character TEXT SHA-256 of the exact BLOB bytes
flags: SQLITE_UTF8 | SQLITE_DETERMINISTIC | SQLITE_INNOCUOUS
```

rusqlite 0.31 exposes all three exact `FunctionFlags` constants, including
`SQLITE_INNOCUOUS`; Task 2 includes a compile/runtime contract test against
that pinned dependency, so an unsupported dependency change fails rather than
silently dropping a flag.

NULL, TEXT, INTEGER, REAL or any non-BLOB input raises a function error rather
than coercing bytes. Registration is followed, before schema work, by the
fixed self-test
`SELECT sha256_hex(x'') =
'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'`.
Missing rusqlite feature support, duplicate/incompatible registration,
callback failure or self-test mismatch fails the connection open with the
existing typed configuration channel and stable reason
`durable_sha256_udf_unavailable`; it is never a WARN-only degradation.

Every v6 BR-192 authority trigger that joins canonical bytes to a stored digest,
including the retained BR-194 replay Started/Completed audit triggers and the
new BR-192 authorization/cycle/failure triggers, requires both byte-exact joins
and `sha256_hex(canonical_blob)=stored_lowercase_sha256`. The v5-to-v6 migration
does not drop/recreate accepted BR-194 trigger definitions; replay tables, rows,
audit kinds, foreign keys and manifest
identities remain unchanged. The v5 validator compares normalized trigger SQL
against this exact function name/arity and rejects a trigger that merely
compares two caller-supplied hash columns.

The checked-in BR-192 evidence verifier independently hashes every canonical
BLOB in Rust and validates the v6 trigger catalog. The BR-194 Python release
verifier does **not** call the Rust connection-local seam, register a Python
SQLite callback or execute any DML/trigger. It opens only its isolated
read-only verification copy, reads `PRAGMA user_version`, table rows and
`sqlite_master`, independently hashes returned BLOB bytes with
`hashlib.sha256`, and compares normalized trigger SQL text to the expected
catalog containing the literal `sha256_hex` predicates. Consequently Python
does not claim or emulate rusqlite `FunctionFlags`, including
`SQLITE_INNOCUOUS`.

The shared compliance checker validates `Cargo.toml`, the attestation-safe
central Rust registration order, every Rust/rusqlite connection constructor,
the accepted v4-to-v5 migration, the new v5-to-v6 migration, both verifiers and
the trigger catalog. Its mutation
suite must reject removal/renaming of the `functions` feature, registration or
self-test; registration before complete descriptor binding or after schema
work; Python `create_function`/DML/trigger execution; TEXT/NULL coercion;
removal of `SQLITE_DETERMINISTIC` or `SQLITE_INNOCUOUS`; replacement of
`sha256_hex(canonical)` with hash-column equality; weakening either BR-192 or
BR-194 verifier; and a fresh/migrated manifest mismatch.

### 2.1 Frozen rejection disposition

Add one immutable `counted_producer_attestations` companion row in the same
`BEGIN IMMEDIATE` transaction that first inserts an enabled counted decision:

```text
decision_identity                         PRIMARY KEY; deferred FK to delivery_decisions
push_kind                                 immutable; exactly ReviewProviderTopN
producer_seam                             immutable exact R-09 seam
producer_catalog_identity_sha256          immutable lowercase SHA-256
producer_attestation_sha256               immutable lowercase SHA-256
source_business_date                      immutable non-null ISO date
expires_at                                immutable non-null UTC timestamp
attestation_canonical                     immutable BLOB
created_at
```

The FK is `DEFERRABLE INITIALLY DEFERRED`. The companion must be inserted first
while no decision with that identity exists; its insert trigger rejects an
already-existing decision. The subsequent `delivery_decisions` insert trigger
requires the exact companion for every `ReviewProviderTopN` decision and joins
its kind/canonical identity before the transaction can commit. This
companion-first/deferred-FK protocol makes “same transaction as first decision
insert” database-enforceable rather than a repository convention. The
companion trigger recomputes `producer_attestation_sha256` from
`attestation_canonical` with `sha256_hex`, requires the exact field order/domain
from §1.2 and requires `expires_at` to be the first Asia/Shanghai midnight after
`source_business_date`. Every update or delete is rejected. The v5-to-v6 migration creates the empty companion table;
it does not synthesize rows for existing decisions. Consequently v5 decisions
and any new unpermitted decision have no attestation row and are typed
`RetryProducerNotEnabled`, never inferred from their envelope bytes or kind.

`delivery_disposition_payloads` remains the authority for a definite
rejection. A retry-eligible disposition must be:

- the row referenced by `delivery_decisions.current_disposition_identity`;
- `disposition='Rejected'`;
- appended successfully with a non-empty `immutable_audit_ref`;
- hash-valid against its stored canonical bytes;
- bound to the same decision identity and frozen envelope hash;
- definite rather than uncertain;
- bound in its frozen envelope to exact enabled-producer provenance:
  `push_kind=ReviewProviderTopN`, exact R-09 seam, current catalog identity
  hash and valid producer-attestation hash; and
- either the source of an automatically-created frozen-rejection
  authorization or the target of one authenticated manual authorization.

`delivery_decisions.retry_authorized` remains a compatibility projection, but
candidate selection and admission must not trust it. It is derived from the
presence of the unique active companion binding; this slice does not schedule
a speculative follow-up removal.

The producer-supplied `DeliveryEnvelope.retry_authorized` is not an admission
authority. Existing callers may continue constructing old envelopes during
the migration, but initial envelope insertion cannot make a decision a retry
candidate.
Existing v5 decisions have no producer-attestation companion row after
migration. They are never inferred from `push_kind`, template ID, task
identity, content or historical caller location and therefore cannot receive
either automatic or manual retry authorization.

### 2.2 Append-only retry authorization state machine

Add `retry_authorizations`:

```text
authorization_identity                    PRIMARY KEY
decision_identity                         REFERENCES delivery_decisions
rejection_disposition_identity             REFERENCES delivery_disposition_payloads
source_kind                               FrozenRejection | ManualOperator
command_identity                           nullable; required for ManualOperator
push_kind                                  immutable; exactly ReviewProviderTopN
producer_seam                              immutable exact R-09 seam
producer_catalog_identity_sha256           immutable lowercase SHA-256
producer_attestation_sha256                immutable lowercase SHA-256
authorization_canonical                    immutable BLOB
authorization_sha256                       immutable TEXT
append_state                               PendingAppend | Appended
immutable_audit_ref                        nullable until Appended
apply_state                                PendingApply | Applied | Invalidated
authorized_at                              authority-owned timestamp
created_at
applied_at
UNIQUE(decision_identity, rejection_disposition_identity)
```

Creation first joins these four producer fields to the frozen envelope and
current catalog. A mismatch, missing/null migrated-v5 field, disabled kind or
non-R-09 seam returns typed `RetryProducerNotEnabled` before inserting the
authorization row. Recovery repeats that join before append, apply and active-
binding creation; it never retargets an authorization to a later catalog.

Canonical identity/hash/source/decision/disposition/command/`authorized_at`
fields are immutable by trigger. For `ManualOperator`, `authorized_at` must
equal the validated PAM session's `validated_at`; `created_at` is only the
database insertion time and has no eligibility or authorization authority.
For `FrozenRejection`, `authorized_at` equals the frozen rejection's
`observed_at`.
Only these monotonic transitions are legal:

```text
PendingAppend/PendingApply
  -> Appended/PendingApply
  -> Appended/Applied

PendingAppend/PendingApply
  -> Appended/Invalidated
```

`Invalidated` is allowed only when recovery proves that the target disposition
is no longer current before application. It never authorizes retry.
Neither authorization table stores an append-authority timestamp: the append
port supplies no trustworthy time. The authoritative acknowledgement is only
the monotonic append state plus its immutable audit reference; `created_at` is
the repository row-creation time and `applied_at` records only the coordinator
projection transition.

The `apply_state` column is a retained materialized projection, not the
tamper-resistant transition authority. Add `retry_authorization_events`:

```text
authorization_event_identity               PRIMARY KEY
authorization_identity                     REFERENCES retry_authorizations
event_kind                                 Applied | Invalidated
from_apply_state                           PendingApply
to_apply_state                             Applied | Invalidated
target_disposition_identity                immutable
reason_code                                immutable
event_canonical                            immutable BLOB
event_sha256                               immutable TEXT
append_state                               Pending | Appended
immutable_audit_ref                        nullable until Appended
created_at
UNIQUE(authorization_identity,event_kind)
```

Its stable identity is domain-separated by authorization identity, event kind,
from/to states, target disposition identity and reason code. It does not depend
on a retry-cycle identity, so startup recovery can reconstruct and append the
same exact event before a cycle exists. Payload columns are immutable,
`Pending -> Appended` is the only acknowledgement transition, and rows cannot
be deleted. If an appended `Applied` event loses its projection CAS because the
target ceased to be current, it does not authorize retry; recovery appends the
distinct stable `Invalidated` event and only then CASes to `Invalidated`.

Add `retry_authorization_bindings`:

```text
binding_identity                           PRIMARY KEY
decision_identity                          REFERENCES delivery_decisions
authorization_identity                     REFERENCES retry_authorizations
rejection_disposition_identity              REFERENCES delivery_disposition_payloads
binding_generation                         positive, monotonic per decision
binding_state                              Active | Cleared
cleared_reason                             nullable until Cleared
created_at
cleared_at
UNIQUE(decision_identity,binding_generation)
UNIQUE(authorization_identity)
UNIQUE one Active row per decision          partial index
```

The binding's identities and generation are immutable, rows cannot be deleted,
and only `Active -> Cleared` is legal. An `Applied` authorization becomes
current only when one CAS creates its unique active binding while the decision
is still `RejectedDurable` and the target remains the current appended
rejection.

The transition that commits
`UncertainTaskTransitionPending -> UncertainManualReview` must, in the same
`BEGIN IMMEDIATE`, change any current `Active` retry authorization binding for
that decision to `Cleared`, persist
`cleared_reason='uncertain_manual_review'`/`cleared_at`, and clear the
compatibility `retry_authorized` projection. The authorization, authorization
event, cleared binding and every immutable attempt binding remain retained.
There is no committed `UncertainManualReview` row with an active retry
authorization. The same invariant applies whether uncertainty came from
same-cycle finalization, `JoinError` or prior-boot recovery.

Add immutable `retry_attempt_bindings`, keyed by attempt identity, to freeze
the authorization identity, rejection disposition identity, authorization
binding identity/generation, cycle identity, reservation generation,
attempt-owner identity, positive `INTEGER`/Rust `i64` fence token and one-based
`retry_ordinal` selected for an attempt. The table has
`UNIQUE(decision_identity,retry_ordinal)` and
`UNIQUE(cycle_identity,decision_identity,reservation_generation)`. This
historical binding is inserted by the winning admission transaction before the
decision becomes `Reserved` and can never update or delete.

Add one retained `retry_send_ownership` row keyed by attempt identity. It is
created by the pre-call ownership transaction or, when an appended start lacks
ownership, by the zero-sink quarantine transaction solely to retire that send
right. The quarantine case atomically inserts the exact consumed ownership
from the start/binding fields and advances it to `InterruptedUncertain`; no
`Started` quarantine row is externally observable. The row stores exact
decision, attempt-binding, execution-cycle, reservation generation, owner
identity, positive `i64` fence token, persisted `send_started_at`,
`send_consumed=true`, nullable exact `pre_call_freshness_observed_at`, nullable
write-once `terminal_sink_result_identity`, state
`Started|FreshnessExpiredBeforeExternalCall|TerminalRecorded|
InterruptedUncertain` and terminal timestamp/reason when present.
Identity/generation/owner/fence/start time and `send_consumed` are immutable;
`terminal_sink_result_identity` is NULL before terminal-result persistence and
may be written exactly once by the transaction that advances to
`TerminalRecorded`; it is immutable thereafter and references
`sink_results.result_event_identity` through a `UNIQUE DEFERRABLE INITIALLY
DEFERRED` foreign key. The terminal transaction updates the ownership pointer
first and inserts the result before commit. The deferred FK makes a missing
result fail commit, while the unique constraint prevents two ownership rows
from naming one result. The row cannot be deleted or reset. Only monotonic
`Started -> FreshnessExpiredBeforeExternalCall|TerminalRecorded|
InterruptedUncertain` is legal; the only direct
terminal insert is the atomic appended-start quarantine described above.
`FreshnessExpiredBeforeExternalCall` requires the private pre-call observation
at/after expiry, exact terminal reason/time, no sink result and the
same-transaction deferred `retry_pre_call_expiry_authorities` companion plus
Pending expiry authority. Its transition trigger rechecks absence of an exact
attempt/decision sink result immediately before the ownership update; an
earlier application read is insufficient. `TerminalRecorded` must persist the
pre-call observation and exact `terminal_sink_result_identity` in the same
transaction as the authoritative terminal sink result. The ownership update
trigger permits only a one-time `Started/NULL -> TerminalRecorded/non-NULL`
transition and freezes the pointer and terminal fields thereafter. The reverse
`sink_results` insert trigger applies to every retry-attempt result satisfying
the exact predicate `authoritative_for_state=1 AND late_after_fence=0`; it
requires one same-attempt ownership already in `TerminalRecorded` with the
same result identity, decision identity and fence token. Late or
non-authoritative results never satisfy this terminal relation. Every other ownership
state requires that identity to remain NULL. `InterruptedUncertain` requires no terminal
sink result and the exact typed reason
`ProcessInterruptedAfterSinkStart`.

When a sink result creates a new disposition, or any accepted/manual terminal
transition is committed, that same transaction clears the active companion
binding and resets the compatibility boolean. The authorization and its
`Applied` event remain immutable historical truth. A historical applied
authorization is therefore not required to match the decision's later current
disposition; only an `Active` binding must match it.

For `TypedRejection { retry_authorized: true }`, freezing the rejection creates
both one `FrozenRejection` authorization row **and** the decision's initial
`retry_schedules` row in the same `BEGIN IMMEDIATE` transaction as the
rejection disposition. That initial schedule is exactly
`automatic_attempts_started=0`,
`next_eligible_at=rejection.observed_at + 30s`, `exhausted_at=NULL`,
`source_business_date=envelope.source_business_date`,
`expires_at=envelope.expires_at`, `terminal_state='Active'`,
`last_attempt_binding_identity=NULL`, `version=0`. A committed frozen
rejection authorization without that exact schedule, or a schedule created in
a later transaction, is invalid and cannot be reconciled into an active
binding. The authorization becomes visible to candidate selection only after:

1. its exact canonical bytes are appended;
2. the append acknowledgement is committed; and
3. the stable `Applied` authorization event is appended and acknowledged; and
4. one CAS changes the authorization projection from `PendingApply` to
   `Applied` and creates its unique active companion binding on the
   still-current rejection.

For a manual command, the stable uniqueness domain is the current rejection:

```text
UNIQUE(decision_identity, rejection_disposition_identity)
```

The canonical manual command contains:

- schema/rule version;
- authorization, decision and current rejection disposition identities;
- frozen envelope and disposition SHA-256 values;
- operator identity hash;
- reason hash;
- external evidence hash;
- exact `RetryNamespaceHashPreimageV1` and its recomputed
  `namespace_sha256`;
- authenticated timestamp; and
- `source_kind=ManualOperator`.

The command never stores credentials, plaintext account data or holding lists.

### 2.3 Crash recovery and idempotency

Recovery drains authorizations to a fixed point before building candidates:

- crash after row insert, before append: re-append the stored exact bytes;
- crash after append, before acknowledgement: `append_exact` returns the same
  immutable ref, then the acknowledgement CAS completes;
- crash after authorization acknowledgement, before transition-event insert:
  recreate the same stable `Applied` or `Invalidated` event;
- crash after transition-event insert/append, before acknowledgement:
  `append_exact` returns the same event ref and acknowledges it;
- crash after transition-event acknowledgement, before apply/bind: apply the
  already-appended event and create the active binding if its target rejection
  remains current;
- current disposition changed before apply: append and acknowledge the stable
  invalidation event, then CAS to `Invalidated`; never retarget it;
- byte-identical replay: return the stored authorization identity, state and
  immutable ref;
- different bytes for the same decision/current rejection: fail closed with an
  immutable conflict; do not insert a second authority;
- two processes race the same command: the uniqueness constraint selects one
  row; both may append the same bytes, but only one acknowledgement/application
  CAS wins.

Pre-call expiry authority has stricter recovery precedence than generic retry
orphan quarantine. Before classifying any prior-boot `AttemptInFlight`, appended
start or consumed ownership as uncertain, recovery looks for the exact
same-attempt conjunction of immutable pre-call companion,
`FreshnessExpiredBeforeExternalCall` ownership and matching Pending/Appended
`ReservedExpiredBeforeSink` expiry. An exact conjunction resumes only the
stored expiry: append if Pending, acknowledge, then execute Transaction B in
§4. It is definite zero-call evidence and must never be changed to
`InterruptedUncertain` or `ProcessInterruptedAfterSinkStart`. A missing member,
identity/generation/fence/observation/canonical/hash mismatch, a result row, or
a partial Transaction-B after-state returns a typed `DurableDeliveryError` with
zero rewrite. Exact complete after-state is idempotent
`RetryExpiryDisposition::AlreadyTerminalized`.

The drain is a deterministic one-row state machine, not an unordered scan.
`PendingAppend` has strict priority. It rejects a row whose `created_at` or
`authorization_identity` is null, empty or ASCII-whitespace-only, and selects
exactly:

```sql
WHERE append_state = 'PendingAppend'
ORDER BY created_at COLLATE BINARY ASC,
         authorization_identity COLLATE BINARY ASC
LIMIT 1
```

Only when no `PendingAppend` row exists may it select one acknowledged
authorization awaiting application:

```sql
WHERE append_state = 'Appended' AND apply_state = 'PendingApply'
ORDER BY created_at COLLATE BINARY ASC,
         authorization_identity COLLATE BINARY ASC
LIMIT 1
```

That selected authorization owns its stable `Applied|Invalidated` event; event
insert/append/acknowledgement never performs a second unordered event lookup.
One call advances at most one selected authorization state and returns whether
it made durable progress. The caller permits exactly
`MAX_AUTHORIZATION_RECONCILE_STEPS_PER_RUN = 4096` successful progress returns,
then performs one non-mutating pending-row check. A remaining row returns typed
`DurableDeliveryError::AuthorizationReconciliationBoundExceeded {
max_steps: 4096, pending_authorization_identity }`, retains the exact pending
state, writes no candidate/admission state and makes zero sink calls. A false
progress return with no pending row is the only successful fixed point.

`PendingAppend` and `PendingApply` are authorization-reconciliation states, not
admission results: candidate SQL cannot return them. If reconciliation cannot
reach a fixed point, the cycle writes and appends
`AuthorizationReconciliationBlocked`, becomes `Failed`, performs no candidate
query and makes zero sink calls. An authorization append/apply error therefore
leaves recoverable authority state and never changes delivery state to
`Reserved`.

### 2.4 Persistent retry schedule

Add one `retry_schedules` row per decision occurrence:

```text
decision_identity                         PRIMARY KEY
automatic_attempts_started                non-negative
next_eligible_at                          nullable
exhausted_at                              nullable
source_business_date                      non-null ISO date
expires_at                                non-null UTC timestamp
terminal_state                            Active|ExpiredFreshness|Exhausted|Completed
last_attempt_binding_identity             nullable UNIQUE
                                          REFERENCES retry_attempt_bindings(attempt_identity)
version
```

Internal delivery-governance constants are:

```text
MAX_AUTOMATIC_RETRY_ATTEMPTS = 3
RETRY_BACKOFF_SECONDS = [30, 120, 600]
```

These are not financial thresholds and do not alter market selection, risk,
order or capital rules.

`retry_schedules` is a retained projection of the immutable companion attempt
bindings; its last-attempt column is not free-text authority. A database
trigger:

- rejects every `DELETE`;
- requires `automatic_attempts_started` never to decrease and never to exceed
  three;
- requires count zero iff `last_attempt_binding_identity` is null;
- for a non-zero count, requires that referenced binding to belong to the same
  decision and have `retry_ordinal=automatic_attempts_started`;
- requires each authority-changing update to increment `version` by exactly
  one;
- rejects clearing or changing a non-null `exhausted_at`;
- requires `expires_at` to equal the first Asia/Shanghai midnight after the
  immutable `source_business_date` and rejects every update to either field;
- allows only the monotonic `Active -> ExpiredFreshness|Exhausted|Completed`
  terminal transition and rejects deletion/reset;
- permits the first `exhausted_at` only when the attempt count is three; and
- rejects clearing or moving a non-null `next_eligible_at` earlier.

Repeated initialization must preserve the row and these constraints. A
schedule cannot be deleted and recreated to reset the cap.

The frozen-rejection transaction initializes the schedule, so the first
frozen-rejection-authorized retry is not eligible before
`rejection.observed_at + 30s`. A later manual authorization retains that same
schedule. When a manually targeted definite rejection never had a
`FrozenRejection` authorization, the manual-authorization transaction itself
must insert the initial zero-attempt schedule; it cannot be added after the
authorization commit. Manual eligibility uses
`max(rejection.observed_at + 30s, existing next_eligible_at when present,
authenticated_operator.validated_at)`; the
authorization row's `authorized_at` is byte-for-byte that authority-owned
timestamp, so a caller or delayed CLI process cannot move eligibility. It
cannot shorten an existing backoff. Every runner attempt, regardless of
authorization source, consumes the same cap. The winning
`admit_authorized_retry` transaction selects the next reservation generation,
deterministic attempt identity, next retry ordinal, attempt owner and fence
token; installs the authoritative attempt and immutable attempt binding;
increments `automatic_attempts_started`; points the schedule at that exact
binding; freezes the active authorization binding; and only then changes the
decision to `Reserved`, all in one `BEGIN IMMEDIATE`. A later definite rejection
sets the next
persisted eligibility using the delay associated with the number of attempts
already started. An accepted result is terminal. Any uncertain result is
terminal for automatic retry and retains its normal uncertain reservation
semantics.

The coordinator-owned expiry DTOs are public only through the sealed
`durable_delivery` root. They have no public constructors, `Default`,
`Deserialize` or writable fields; callers can only inspect read-only accessors
and pass the exact value back to the coordinator:

```rust
pub struct ExpirableRetrySchedule {
    decision_identity: String,
    rejection_disposition_identity: String,
    authorization_identity: Option<String>,
    push_kind: PushKind,
    producer_seam: String,
    producer_catalog_identity_sha256: String,
    producer_attestation_sha256: String,
    source_business_date: NaiveDate,
    expires_at: DateTime<Utc>,
    pre_start_reserved_attempt_identity: Option<String>,
}

pub struct CompleteRetryExpirySnapshot {
    ordered_rows: Vec<ExpirableRetrySchedule>,
    row_count: usize,
    ordered_rows_sha256: String,
}

pub struct PreparedRetryExpiredFreshness {
    expiry_event_identity: String,
    decision_identity: String,
    rejection_disposition_identity: String,
    authorization_identity: Option<String>,
    attempt_identity: Option<String>,
    source_business_date: NaiveDate,
    expires_at: DateTime<Utc>,
    freshness_observed_at: DateTime<Utc>,
    terminal_kind: RetryExpiryTerminalKind,
    event_canonical: Vec<u8>,
    event_sha256: String,
}

pub enum RetryExpiryUncertaintyReason {
    PendingStartAuthority,
    AppendedStartAuthority,
    SendOwnershipAuthority,
}

pub struct PreparedRetryExpiryUncertainty {
    decision_identity: String,
    attempt_identity: String,
    execution_cycle_identity: String,
    start_event_identity: Option<String>,
    ownership_identity: Option<String>,
    source_business_date: NaiveDate,
    expires_at: DateTime<Utc>,
    freshness_observed_at: DateTime<Utc>,
    reason: RetryExpiryUncertaintyReason,
    prepared_state_sha256: String,
}

pub enum RetryExpiryPreparationOutcome {
    ExpiryPrepared(PreparedRetryExpiredFreshness),
    StartAuthorityWins(PreparedRetryExpiryUncertainty),
}

pub enum RetryExpiryDisposition {
    AppendedAndTerminalized {
        decision_identity: String,
        terminal_state: RetryScheduleTerminalState,
    },
    AlreadyTerminalized {
        decision_identity: String,
        terminal_state: RetryScheduleTerminalState,
    },
    RoutedToUncertainty {
        decision_identity: String,
        attempt_identity: String,
        reason: RetryExpiryUncertaintyReason,
    },
}
```

`RetryExpiryTerminalKind` is the closed
`RejectedDurableExpired|ReservedExpiredBeforeSink|
ManualTargetExpiredBeforeAuthorization` set and
`RetryScheduleTerminalState` is the closed schedule state set above.
`PreparedRetryExpiredFreshness` is materialized from, and must byte-for-byte
match, one durable `retry_expiry_audit_outbox` row; acknowledgement accepts no
caller-supplied replacement bytes, hash, terminal kind or timestamp.

`PreparedRetryExpiryUncertainty` has no public constructor, `Clone`,
`Deserialize` or `Default`; it is coordinator-derived retained-state evidence,
not authority to call a sink. `RoutedToUncertainty` is returned only after its
uncertainty outboxes are append/acknowledged to a fixed point, the active
binding is cleared and the schedule leaves `Active`. That branch writes no
expiry-outbox row. Reason precedence is exactly `SendOwnershipAuthority >
AppendedStartAuthority > PendingStartAuthority`.

Expiry is actively drained; it is never implemented only as a candidate SQL
filter. `DurableDeliveryCoordinator` owns:

```rust
fn select_expirable_retry_schedules(
    &self,
) -> Result<CompleteRetryExpirySnapshot>;

fn prepare_retry_expired_freshness(
    &self,
    row: &ExpirableRetrySchedule,
) -> Result<RetryExpiryPreparationOutcome>;

fn reconcile_retry_expiry_preparation(
    &self,
    append: &dyn ImmutableAppendPort,
    prepared: RetryExpiryPreparationOutcome,
) -> Result<RetryExpiryDisposition>;
```

Every freshness-bearing coordinator operation (expiry selection/preparation,
admission, start preparation, ownership claim and the final pre-sink gate)
reads the coordinator's private `ProductionFreshnessClock`; none accepts a
caller timestamp. The production constructor always installs the system clock
and exposes no clock setter. Only a cfg(test)-only TEST_CODE constructor can
install a deterministic clock. Observation timestamps needed for decisions are
persisted in their canonical authority rows.

The `now`/`started_at`/`failed_at` parameters on cycle-lifecycle, append-
acknowledgement and quarantine APIs are audit ordering metadata only. They are
never read by schedule eligibility, source-business-date expiry, manual
authorization, candidate selection, start admission, ownership claim or the
pre-call freshness decision, and cannot satisfy or override any freshness
gate. Those gates exclusively read `ProductionFreshnessClock`.

`ExpiryPrepared` persists one immutable expiry outbox authority, so expiry can
be resumed outside the cycle that discovered it (including after manual
authorization or process restart). `StartAuthorityWins` instead persists and
append/acknowledges the existing uncertainty authority to a terminal fixed
point, clears the active binding and moves the schedule out of `Active`; it
writes no expiry-outbox row and can mint neither a send permit nor a sink call.
Its reason precedence is exactly `SendOwnershipAuthority >
AppendedStartAuthority > PendingStartAuthority`:

```text
retry_expiry_audit_outbox
  expiry_event_identity                    PRIMARY KEY
  decision_identity                        FK delivery_decisions
  rejection_disposition_identity           FK delivery_disposition_payloads
  authorization_identity                   nullable FK retry_authorizations
  attempt_identity                         nullable FK retry_attempt_bindings
  source_business_date                     non-null ISO date
  expires_at                               non-null UTC timestamp
  freshness_observed_at                    non-null UTC timestamp, >= expires_at
  terminal_kind                            RejectedDurableExpired|
                                           ReservedExpiredBeforeSink|
                                           ManualTargetExpiredBeforeAuthorization
  event_canonical                          immutable BLOB
  event_sha256                             lowercase SHA-256
  append_state                             Pending|Appended
  immutable_audit_ref                      nullable
  created_at                               immutable ordering metadata
  UNIQUE(decision_identity, expires_at)
```

The final pre-call no-sink exception additionally uses one private immutable
commit companion:

```text
retry_pre_call_expiry_authorities
  authority_identity                       PRIMARY KEY = authority_sha256
  expiry_event_identity                    UNIQUE deferred FK expiry outbox
  decision_identity                        FK delivery_decisions
  attempt_identity                         UNIQUE FK retry_send_ownership
  execution_cycle_identity                 FK retry_cycles
  reservation_generation                   positive INTEGER/Rust i64
  source_business_date                     non-null ISO date
  expires_at                               non-null UTC timestamp
  freshness_observed_at                    non-null UTC timestamp, >= expires_at
  authority_canonical                      immutable BLOB
  authority_sha256                         lowercase SHA-256
  created_at                               = freshness_observed_at
```

`expiry_event_identity` is a `DEFERRABLE INITIALLY DEFERRED` foreign key to
the expiry row. The companion is therefore allowed to be inserted first
inside the owning transaction, but that transaction cannot commit unless the
exact expiry row also exists. Its canonical bytes are
`b"stock_analysis.durable_delivery.br192.pre_call_expiry_authority.v1\0" ||
serde_json::to_vec` of the ordered fields
`schema_version=1,rule_id="BR-192",expiry_event_identity,
decision_identity,attempt_identity,
execution_cycle_identity,reservation_generation,source_business_date,
expires_at,freshness_observed_at`. `authority_identity` is deliberately not a
canonical input, so the identity rule is non-self-referential;
`authority_sha256=sha256_hex(authority_canonical)` and
`authority_identity=authority_sha256`. The private validator performs exact
domain removal, deny-unknown-fields decode, decode→reserialize equality and
typed-column equality before insert. Update/delete are forbidden.

The companion insert trigger requires the exact Active schedule, exact
`AttemptInFlight` attempt, exact `Started` ownership with null pre-call and
terminal fields, its exact Appended `SinkAttemptStarted` for the same execution
cycle/reservation generation, no terminal sink result and no existing expiry
row. Production code cannot construct or insert the companion outside
`execute_prepared_retry_sink`; no public DTO, constructor or raw connection is
exposed for it.

`event_canonical` is exactly
`b"stock_analysis.durable_delivery.br192.retry_expired_freshness.v1\0" ||
serde_json::to_vec` of the ordered fields
`schema_version=1,rule_id="BR-192",decision_identity,
rejection_disposition_identity,authorization_identity(nullable),attempt_identity,
source_business_date,expires_at,freshness_observed_at,terminal_kind`. Dates use ISO `YYYY-MM-DD` and
timestamps use UTC `Z` with nine fractional digits. The stored
`event_sha256=sha256_hex(event_canonical)` and
`expiry_event_identity=event_sha256`; the PK therefore names the exact typed
bytes, while the logical UNIQUE prevents different bytes for the same
decision/expiry. The private validator requires/strips the exact domain prefix
and decode→reserialize equality of the JSON suffix.

For ordinary drain, admission and manual authorization,
`freshness_observed_at` is captured from the coordinator-owned production
clock inside the owning `BEGIN IMMEDIATE` immediately before the freshness-
dependent write. The final pre-call gate is the sole timing exception: the
coordinator samples that same private clock at the external-call linearization
point and, when expired, begins Transaction A in §4 immediately with no await,
I/O or caller code between the sample and `BEGIN IMMEDIATE`; Transaction A
revalidates every current authority before persisting the exact sample.
Production callers cannot provide it. Every insert requires
`freshness_observed_at >= expires_at`, and the exact value is duplicated in the
typed row and canonical bytes. Insert otherwise requires the exact current Active schedule, exact current appended
rejection disposition, appended/applied authorization and enabled producer
attestation. The database recomputes `sha256_hex(event_canonical)` and validates
private decode→canonical reserialization plus every duplicated typed field.
`RejectedDurableExpired` requires a non-null authorization and no attempt.
`ReservedExpiredBeforeSink` requires a non-null authorization and names the
exact current retry attempt. Normal expiry preparation is legal only while no
prepared Pending/Appended `SinkAttemptStarted` logical slot and no send-
ownership row exists. The sole exception is the final pre-call freshness
transaction in §4: it may atomically pair the exact attempt's
`FreshnessExpiredBeforeExternalCall` ownership terminal with the expiry row,
because the live single-use permit was consumed without beginning the external
call. Payload/key fields
are immutable, delete is forbidden, and only exact `Pending -> Appended` with a
valid immutable ref is allowed. Schedule terminalization, active-binding clear
and optional pre-start reservation release occur only after the same row is
Appended. A crash after prepare or append acknowledgement resumes the same
identity/bytes; no cycle identity or caller timestamp is needed to reconstruct
it.

`ManualTargetExpiredBeforeAuthorization` requires both authorization and
attempt to be null. It is prepared only when an authenticated manual command
targets a valid current R-09 definite rejection that has no prior
FrozenRejection authorization/schedule and is already expired at the owning
transaction's freshness observation. That same transaction inserts the
zero-attempt Active schedule and Pending expiry outbox together, inserts no
authorization/binding, then the ordinary append/ack/terminalize protocol makes
the schedule `ExpiredFreshness`. This closes the audit path without inventing
an authorization that never became eligible.

The selector materializes every `terminal_state=Active AND expires_at<=now`
row in one validated snapshot, rejects missing keys and orders
`expires_at,decision_identity,rejection_disposition_identity` with BINARY text
order and no `LIMIT`, `OFFSET`,
cursor or caller cardinality. The runner drains this snapshot to an empty
fixed point before Reserved processing and before each candidate snapshot.
Candidate acquisition is a closed `Candidates|ExpiredFound` result: in the
same read transaction it checks for expired Active rows first and may never
silently exclude one. `ExpiredFound` returns the complete expiry snapshot,
which is reconciled before candidate query is retried.

For `RejectedDurable`, expiry append/ack terminalizes the schedule, clears the
active authorization binding and retains the definite rejection. For a
pre-start `Reserved` retry with neither a prepared `SinkAttemptStarted` logical
slot nor send ownership, it additionally records retained attempt terminal
authority whose effective state is `ExpiredFreshnessBeforeSink`, retains the
schema-v5-compatible base attempt row, releases the retained reservation and
then terminalizes the schedule under the exact relation defined in §4.

Expiry preparation, `SinkAttemptStarted` preparation/acknowledgement and send-
ownership claim form one SQLite-enforced total order. Each is performed in an
owning `BEGIN IMMEDIATE` and rechecks the opposing authority before its write:

For every rule in this total order, **current attempt** has one exact meaning:
the `retry_schedules.last_attempt_binding_identity` names the immutable attempt
binding for the same decision and ordinal, the decision's current retry attempt
and reservation generation match that binding, and the current `Active`
authorization binding/generation, owner and fence all match. A start slot
belongs to that attempt only when its validated canonical
`attempt_identity` and its typed decision/cycle/event ordinal resolve to that
same binding and reservation generation. Rows for an older attempt that is no
longer the schedule's current binding are historical and are ignored by the
current-attempt predicate; a partially mismatched row is corruption, not
history, and fails closed.

The exact migration trigger set is
`trg_retry_expiry_insert_requires_no_start_or_ownership`,
`trg_retry_start_insert_rejects_expiry_authority`,
`trg_retry_start_append_rejects_expiry_authority` and
`trg_retry_send_ownership_rejects_expiry_authority`, plus
`trg_retry_pre_call_expiry_authority_insert_requires_started_ownership`,
`trg_retry_pre_call_expiry_authority_update_immutable`,
`trg_retry_pre_call_expiry_authority_delete_immutable` and
`trg_retry_send_ownership_pre_call_expiry_requires_authority`, plus
`trg_retry_sink_result_insert_rejects_pre_call_expiry_authority`, and the
terminal-result bijection triggers
`trg_retry_send_ownership_terminal_result_once`,
`trg_retry_send_ownership_terminal_result_immutable` and
`trg_retry_sink_result_requires_terminal_ownership`.
Post-validation requires all twelve exact SQL definitions. The expiry reverse trigger is
`BEFORE INSERT ON sink_results` and rejects the exact attempt/decision when
**any one** of these already exists: its pre-call expiry companion, its
`FreshnessExpiredBeforeExternalCall` ownership, or its
`ReservedExpiredBeforeSink` expiry outbox. It does not wait for the complete
triple; this makes the reverse sink-result ordering safe at every intermediate
statement inside the transaction. The ownership-transition trigger and the
expiry-insert exception independently recheck that no exact attempt/decision
sink result exists at their own write boundaries. The expiry trigger
permits an existing
ownership row only for the same transaction's exact attempt/observation in
state `FreshnessExpiredBeforeExternalCall`, the exact deferred companion and
the exact Appended start for that ownership's execution cycle/reservation
generation. Start rows from an
older terminal attempt are historical evidence and do not conflict with the
current attempt; any Pending or different-cycle start for the current attempt
still conflicts.

- if any Pending/Appended `SinkAttemptStarted` logical slot or ownership row
  for the current attempt
  already exists, ordinary expiry preparation writes no expiry-outbox row and
  routes the retained attempt to the existing uncertainty/quarantine path. A Pending slot
  is conservative evidence because the immutable append may already have
  happened even when SQLite acknowledgement has not;
- if the Pending/Appended expiry-outbox row is inserted first, database
  triggers reject every later start-slot insertion, `Pending -> Appended` start
  acknowledgement and ownership insertion for that decision/attempt. The
  expiry row therefore remains reconcilable to terminal state and can never be
  superseded or stranded; and
- the only exception is the same-transaction pre-call protocol: insert the
  immutable deferred companion, transition exact ownership `Started` to
  `FreshnessExpiredBeforeExternalCall`, then insert the exact expiry row. The
  ownership-update trigger requires that companion and zero result; the expiry-
  insert trigger requires both companion and terminal ownership and again
  requires zero result; the reverse sink-result trigger rejects a result after
  any member exists; and the deferred foreign key rejects commit unless all
  three exist. That protocol is not available to
  recovery, callers or an already uncertain attempt and invokes zero external
  sink calls; and
- Reserved processing checks current time and absence of expiry authority
  before start preparation, then checks both again in the ownership-claim
  transaction. Equality with expiry is ineligible for start.

Thus, outside the exact live-permit final-pre-call exception, only a start
preparation/ownership transaction that committed before ordinary expiry
preparation dominates and routes to uncertainty. The exact exception instead
commits the complete no-call triple and follows Transaction B, never
uncertainty. A later start cannot dominate an already prepared expiry and is
never relabelled as valid.

Admission and manual authorization repeat the exact comparison and producer-
provenance join in their owning `BEGIN IMMEDIATE`. At `now >= expires_at`, the
coordinator prepares and reconciles the immutable `RetryExpiredFreshness`
event, then atomically changes the retained schedule from `Active` to
`ExpiredFreshness`, invalidates/clears the active binding and returns typed
`NoLongerEligible::ExpiredFreshness { expires_at }`. It neither increments the
attempt count nor calls provider, renderer or sink. Repeated discovery/drain/
reconciliation is byte-identical and produces no second logical event. Each
future enabled counted kind must first register an explicit source-owned
expiry derivation; there is no generic/default expiry.

After three automatic attempts, admission returns
`NoLongerEligible::RetryAttemptsExhausted` exactly once, persists an exhausted
projection and removes the identity from future candidate lists. Manual
authorization does not reset the counter or bypass exhaustion.

### 2.5 Immutable-reference validity

Every new nullable immutable reference follows the schema-v3 accepted-audit-ref
rule. `Appended` requires a non-null value containing at least one character
other than the complete ASCII whitespace set space (`0x20`), tab (`0x09`), LF
(`0x0A`) and CR (`0x0D`):

```sql
length(trim(
  immutable_audit_ref,
  char(32) || char(9) || char(10) || char(13)
)) > 0
```

This applies to authorization, authorization-event, cycle-audit,
`SinkAttemptStarted` and every other v6 immutable-ref column. `Pending` requires
the ref to be null. The schema-v3 Rust predicate is reused/centralized rather
than duplicated and is called before every v6 acknowledgement; SQL
constraints/triggers remain the final authority. Migration tests cover `NULL`,
empty, space-only, tab-only, LF-only, CR-only, mixed ASCII whitespace and a
valid non-whitespace ref for every new table.

## 3. Typed admission contract

Remove `ForeignLiveAttempt`; it is not reachable from a method restricted to
`RejectedDurable`. A competing process is represented by the state/generation
observed after losing the transaction.

```rust
pub enum RetryDeferral {
    DailyBudgetFull,
    BusinessDateClaimedByOther,
    RollingHeadReserved,
    RollingHeadUncertain,
    RollingCooldown { eligible_at: DateTime<Utc> },
    RetryBackoff { eligible_at: DateTime<Utc> },
}

pub enum RetryIneligibility {
    RetryNotAuthorized,
    AuthorizationDoesNotMatchCurrentRejection,
    StateChanged {
        observed_state: DecisionState,
        reservation_generation: i64,
    },
    RetryAttemptsExhausted,
    ExpiredFreshness { expires_at: DateTime<Utc> },
    UncertainAuditPending,
    UncertainTaskTransitionPending,
    UncertainManualReview,
    TerminalState { observed_state: DecisionState },
}

pub enum RetryAdmission {
    Reacquired {
        decision_identity: String,
        attempt_identity: String,
        cycle_identity: String,
        reservation_generation: i64,
        authorization_identity: String,
        rejection_disposition_identity: String,
        authorization_binding_identity: String,
        binding_generation: i64,
        retry_ordinal: i64,
        owner_instance_identity: String,
        fence_token: i64,
    },
    Deferred(RetryDeferral),
    ExpiredFreshnessPrepared {
        expires_at: DateTime<Utc>,
        prepared: PreparedRetryExpiredFreshness,
    },
    NoLongerEligible(RetryIneligibility),
}
```

```text
admit_authorized_retry(cycle_identity, decision_identity)
  -> Result<RetryAdmission, DurableDeliveryError>
```

Freshness expiry is an expected business outcome, not an operational error.
If automatic admission itself first observes expiry, it returns
`RetryAdmission::ExpiredFreshnessPrepared { expires_at, prepared }`. The runner
must append/ack/reconcile those exact prepared bytes before it may persist and
report the final cycle
`RetryAdmission::NoLongerEligible(RetryIneligibility::ExpiredFreshness {
expires_at })`; append/reconcile failure typed-fails the cycle and never claims
terminal expiry. The manual entry owns its append/reconcile capabilities and returns
`ProductionRetryAuthorizationOutcome::NoLongerEligible` carrying the same
typed ineligibility; an expiry drain returns
`RetryExpiryDisposition::AppendedAndTerminalized|AlreadyTerminalized`; and the
final pre-call gate returns
`RetrySinkExecutionOutcome::ExpiredBeforeExternalCall(prepared)` before the
normal expiry reconcile. The implementation must not add
`DurableDeliveryError::RetryExpiredFreshness` or encode expected expiry in an
error string. `DurableDeliveryError` remains reserved for operational failure
to validate, persist, append, acknowledge or CAS those authorities.

The existing `DurableDeliveryError` gains one retry-specific variant:

```rust
RetryAttemptStartAuditUnavailable {
    attempt_identity: String,
    reason_code: String,
}
```

`reason_code` is exactly one of `missing_start_event`, `pending_append` or
`missing_immutable_ref`. It is returned by preparation/read-back/validation
only after the retry admission transaction has already created the immutable
attempt binding, and always before a sink capability or single-use execution
permit exists. The provider-free runner converts it directly, without a
`String` intermediate, through
`RetryCycleFailure::from_retry_attempt_start_audit_unavailable(&error)`.

`RetryCycleFailure` is an opaque library-owned DTO: all fields and its private
`RetryCycleFailureTypedFieldsV1` are private, it implements neither
`Deserialize` nor `Default`, and it has no raw-values constructor, builder or
struct-update surface. It exposes only read-only `reason()` and
`typed_fields_sha256()` accessors. `RetryCycleOperation` is the public closed
operation vocabulary used by the one constructor that needs an operation; it
derives `Serialize`/`Deserialize` with `rename_all="snake_case"` and
`deny_unknown_fields` and has exactly these variants:

```rust
pub enum RetryCycleOperation {
    AuthorizationReconciliation,
    CycleAuditReconciliation,
    ReservedAttemptRecovery,
    PriorBootCycleRecovery,
    CandidateSnapshot,
    RetryAdmission,
    AttemptStartPreparation,
    AttemptStartAppendAcknowledge,
    SendOwnershipClaim,
    SinkExecution,
    PostSinkReconciliation,
    CompletionPreparation,
    CompletionAppendAcknowledge,
    CompletionTerminalization,
    FailureQuarantine,
    FailurePreparation,
    FailureAppendAcknowledge,
    FailureTerminalization,
}
```

The monitor binary can construct failures only through these six public
associated functions, with these exact names and signatures:

```rust
impl RetryCycleFailure {
    pub fn from_retry_attempt_start_audit_unavailable(
        error: &DurableDeliveryError,
    ) -> Result<Self>;

    pub fn from_authorization_reconciliation_blocked_sha256(
        authorization_identity_sha256: &str,
    ) -> Result<Self>;

    pub fn from_cycle_operation_failed(
        operation: RetryCycleOperation,
        error_sha256: &str,
    ) -> Result<Self>;

    pub fn from_panic_sha256(panic_sha256: &str) -> Result<Self>;

    pub fn from_join_error_sha256(join_error_sha256: &str) -> Result<Self>;

    pub fn from_process_interrupted_owner_boot_identity_sha256(
        owner_boot_identity_sha256: &str,
    ) -> Result<Self>;
}
```

Here `Result` is the already-public `durable_delivery::Result<T>` alias.
The first constructor accepts only
`DurableDeliveryError::RetryAttemptStartAuditUnavailable`; another variant is
an exact typed error. The other five arguments named `*_sha256` accept only
lowercase 64-hex and never raw error, panic, JoinError, boot identity or
authorization content. No constructor accepts a caller-supplied
`RetryCycleFailureReason`, `schema_version`, `rule_id`, canonical bytes or
digest override. The constructors fix `schema_version=1`,
`rule_id="BR-192"`, the matching closed reason and build one reason-specific
canonical preimage:

```text
RetryAttemptStartAuditUnavailableV1:
  schema_version,rule_id,reason,attempt_identity,inner_reason
AuthorizationReconciliationBlockedV1:
  schema_version,rule_id,reason,authorization_identity_sha256
CycleOperationFailedV1:
  schema_version,rule_id,reason,operation,error_sha256
PanicV1:
  schema_version,rule_id,reason,panic_sha256
JoinErrorV1:
  schema_version,rule_id,reason,join_error_sha256
ProcessInterruptedV1:
  schema_version,rule_id,reason,owner_boot_identity_sha256
```

The public opaque `RetryCycleFailure` implements neither `Deserialize` nor
`Default`. Its private reason-specific preimages do implement crate-private
`Serialize`/`Deserialize` with `deny_unknown_fields`, solely so coordinator
recovery can validate persisted canonical bytes after a process restart; no
decoder or raw-byte constructor is exported.
Each preimage has exact field order, fixed matching reason, and its own domain
prefix
`stock_analysis.durable_delivery.br192.retry_cycle_failure.<reason>.v1\0`.
`attempt_identity` is preserved byte-for-byte after ASCII-whitespace non-empty
validation; `inner_reason` is exactly `missing_start_event`,
`pending_append` or `missing_immutable_ref`; every other `*_sha256` field is
lowercase 64-hex. `operation` is the exact snake_case serialization of the
closed `RetryCycleOperation` variant. `typed_fields_sha256` is the SHA-256 of
the reason prefix plus repository-canonical UTF-8 bytes.

A crate-private accessor gives the coordinator the validated canonical
preimage. `prepare_retry_cycle_failed` persists a complete immutable
`retry_cycle_failure_payloads` row, never only a digest:

```text
failure_payload_identity                  PRIMARY KEY
cycle_identity                            UNIQUE REFERENCES retry_cycles
failure_reason                            closed RetryCycleFailureReason token
typed_preimage_canonical                  immutable BLOB
typed_preimage_sha256                     immutable lowercase 64-hex
failure_envelope_canonical                immutable BLOB
failure_envelope_sha256                   immutable lowercase 64-hex
created_at
```

The canonical envelope has exact ordered fields
`schema_version,rule_id,cycle_identity,failure_reason,
typed_preimage_sha256,typed_preimage_length`. Its separate domain is
`stock_analysis.durable_delivery.br192.retry_cycle_failure_envelope.v1\0`.
The payload row is inserted in the same transaction that prepares the unique
Pending `Failed` audit slot and changes the cycle to
`Running/FailurePending`; all payload columns are immutable, rows cannot be
deleted, and the cycle/outbox store the matching payload identity and envelope
hash. The coordinator validates the private typed bytes by closed reason,
requires canonical decode→reserialize byte equality, recomputes both hashes,
and rejects every reason/preimage/envelope/cycle/hash disagreement.

Same-boot finalization may compare those stored bytes to the in-memory opaque
DTO, but prior-boot recovery must not require a caller-supplied DTO,
reconstruct from display text or manufacture raw fields. It loads only the
complete stored payload row, privately decodes the closed typed preimage and
envelope, recomputes both hashes and validates the Pending/Appended `Failed`
bytes before terminal CAS. Only after that validation may the library
reconstruct its opaque read-only failure evidence for the returned cycle DTO.
The finalizer therefore first performs one authoritative boundary
classification in the coordinator transaction. Only when there is no
acknowledged `SinkAttemptStarted` and no consumed/send-ownership row may it use
the narrow unchanged-state branch: it prepares the canonical `Failed` outbox
with `reason_code="retry_attempt_start_audit_unavailable"`, the payload identity
and both matching hashes, append/acknowledges that event, and only then marks
the cycle `Failed`. That branch does not change the decision, attempt, binding,
schedule, their canonical hashes or pending `SinkAttemptStarted` bytes; a later
cycle may reconcile those exact bytes.

If either an acknowledged start or consumed ownership exists, the coordinator
rejects the unchanged-state exception. The same failure is routed through the
ordinary post-claim finalizer, which first quarantines every qualifying
same-cycle attempt, advances retained ownership to `InterruptedUncertain` and
append/acknowledges all uncertainty before preparing the matching `Failed`
slot. This boundary decision is based only on persisted rows in the same
transaction; a caller flag cannot select the narrow branch. Both branches make
zero recovery sink calls.
`DurableDeliveryError` and its `Result<T>` alias are already public root
exports. They remain the sole public operational-error channel for every
coordinator, evidence-verifier, retry-runner, guard and terminal-finalizer API;
no private error type may leak through a public signature and no named failure
may be converted to `String` or a generic configuration/storage error.
`RetryCycleFailure` remains the separate opaque, audited business-failure value
returned only by `retry_cycle_blocking`; it is never used as an operational
error wrapper. Task 1 adds these exact variants and fields to the existing
enum:

```rust
AuthorizationReconciliationBoundExceeded {
    max_steps: usize,
    pending_authorization_identity: String,
}
RetryCycleAuditReconciliationBoundExceeded {
    max_steps: usize,
    pending_cycle_event_identity: String,
}
RetryCycleAlreadyRunning
RetryCycleGuardCompareExchangeInvariant
RetryCycleOrdinalExhausted {
    max_ordinal: i64,
}
RetryEvidenceQueryCountOutOfRange {
    requested: usize,
    min: usize,
    max: usize,
}
RetryEvidenceConflictingDuplicate {
    logical_tuple_sha256: String,
    canonical_bytes_mismatch: bool,
    canonical_hash_mismatch: bool,
}
RetryEvidenceResultBoundExceeded {
    max: usize,
    attempted_distinct_count: usize,
}
```

`RetryCycleOrdinalExhausted` is returned only when the retained maximum ordinal
is exactly `i64::MAX`; its `max_ordinal` field is exactly `i64::MAX`, no cycle
or `Started` row is written, and callers/tests must pattern-match this variant
rather than parse display text.

The two reconciliation-bound identities are the exact selected immutable row
identities. Evidence conflict output exposes only the lowercase
domain-separated SHA-256 of the full logical tuple, never its raw identities
or canonical bytes. The logical-tuple hash has this single closed preimage
schema; no map, alternate field order or concatenated/debug string is valid:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryEvidenceLogicalTuplePreimageV1 {
    schema_version: u8,
    rule_id: String,
    decision_identity: String,
    retry_ordinal: i64,
    attempt_identity: String,
    sink_result_identity: String,
    authorization_identity: String,
    rejection_disposition_identity: String,
}
```

The only accepted field order is
`schema_version,rule_id,decision_identity,retry_ordinal,attempt_identity,
sink_result_identity,authorization_identity,rejection_disposition_identity`.
Validation requires `schema_version=1`, `rule_id="BR-192"`,
`retry_ordinal in 1..=3`, and every identity non-empty under the repository
ASCII space/tab/LF/CR rule while preserving accepted identity bytes exactly.
Encoding is the repository canonical compact UTF-8 JSON, exactly
`serde_json::to_vec` of that struct declaration: no map, insignificant
whitespace, BOM, trailing newline, alternate numeric spelling or Unicode
normalization is accepted. Decode then reserialize must be byte-identical.
The literal domain prefix, including its final NUL byte, is:

```text
stock_analysis.durable_delivery.br192.retry_evidence_logical_tuple.v1\0
```

`logical_tuple_sha256` is lowercase hex
`SHA-256(prefix || canonical_utf8_json(preimage))`. The bounded-map typed key
and this hash are both rebuilt from the same validated preimage; neither is
accepted from an artifact. Duplicate comparison recomputes the preimage,
canonical bytes and logical hash before selecting a conflict branch.

The frozen golden vector uses the compact canonical bytes
`{"schema_version":1,"rule_id":"BR-192","decision_identity":"decision-001","retry_ordinal":2,"attempt_identity":"attempt-002","sink_result_identity":"sink-result-003","authorization_identity":"authorization-004","rejection_disposition_identity":"rejection-disposition-005"}`
and produces
`0794e8feda8a5af2c7828be49f35248dd81d44cd21b7408d29db7a7e20e98151`.
An exact recomputation/golden test and a mutation test independently change
the domain, schema, one field, field order and encoding and require every
mutation to miss or be rejected.

At least one conflict flag is `true`: changed canonical bytes yields
`(true,false)`, changed retained hash yields `(false,true)`, and both changed
yields `(true,true)`; `(false,false)` is rejected before construction of
`RetryEvidenceConflictingDuplicate`. Query bounds always report
`min=1,max=256`; inserting the 257th distinct complete join reports
`max=256,attempted_distinct_count=257`. The new
`RetryCycleFailureReason`, `RetryCycleOperation` and `RetryCycleFailure`
remain listed explicitly in the frozen root-export manifest, while callers
pattern-match the new variants through the already-exported
`DurableDeliveryError`.

`DecisionNotFound` is not an admission payload: `AdmissionResult` has a
non-null foreign key to an existing decision. Candidate discovery joins
`delivery_decisions`, so every candidate identity is guaranteed to exist. A
direct/stale caller naming a missing identity returns the existing typed
`DurableDeliveryError::DecisionNotFound` before the admission transaction
creates any `AdmissionResult` or cycle audit.

Inside one `BEGIN IMMEDIATE`, admission:

1. loads the stored canonical envelope and validates its hash;
2. loads the frozen policy and requires the envelope policy version, push kind,
   sub-kind, cooldown scope, window mode and budget-counting fields to match;
3. proves the state is still `RejectedDurable`;
4. validates the current appended rejection disposition;
5. validates its unique active binding, appended/applied authorization identity
   and exact appended `Applied` transition event;
6. checks the persistent attempt cap and `next_eligible_at`;
7. checks claim, rolling head, cooldown and budget ownership;
8. inserts a cycle-bound immutable admission-result outbox event for every
   result;
9. for a winner, selects the next retained reservation generation,
   deterministic attempt identity, retry ordinal, attempt owner and fence token;
10. inserts the authoritative attempt plus immutable retry-attempt binding
    containing the exact authorization, disposition, cycle, binding generation,
    reservation generation, owner and fence;
11. increments the retained schedule, points its last-attempt FK at that binding
    and computes the exact persisted backoff;
12. inserts the exact `RejectedDurable -> Reserved` state event and changes the
    decision to `Reserved`; and
13. commits only after the BR-192 database-operation post-validation proves the
    attempt, binding, schedule, state and admission event agree.

A legitimate reachable wait is `Deferred`; an invalid or terminal state is
`NoLongerEligible`. Neither is logged-only. Pending authorization states are
resolved or fail the cycle before this function is called and therefore have
no typed admission variants.

Post-validation distinguishes active authority from history:

- before applying generic base-attempt predicates, an exact effective
  `ExpiredFreshnessBeforeSink` relation from §4 is validated as terminal
  compatibility history: its decision no longer points to the attempt, its
  fence is revoked, reservations are released, schedule/binding are terminal,
  no result exists and its optional pre-call companion/ownership triple is
  exact; no current-attempt or uncertainty rule may select it;
- every `Active` binding must be unique for its decision, target the decision's
  current appended rejection, reference an `Appended/Applied` authorization
  with its appended `Applied` event, and belong either to `RejectedDurable` or
  to the retry-origin Reserved/in-flight attempt whose immutable attempt binding
  references it;
- every `Cleared` binding must retain valid immutable foreign keys and a legal
  clear transition, but may differ from the decision's later current
  disposition;
- every retry-origin `Reserved` or in-flight decision must have exactly one
  immutable attempt binding whose authorization/disposition/cycle/binding
  generation/reservation generation/owner/fence match the authoritative
  attempt and whose identity/ordinal are the schedule's exact current binding;
- every retry-origin `AttemptInFlight` must have exactly one matching retained
  send-ownership row with `send_consumed=true`, exact attempt/binding/
  execution-cycle/reservation-generation/owner/positive-i64-fence equality,
  and every such `Started` ownership row must point to the current
  `AttemptInFlight`;
- every `TerminalRecorded` ownership must have non-NULL
  `terminal_sink_result_identity` and join exactly that authoritative terminal
  sink result by result, decision and attempt identity; every authoritative
  retry sink result must reverse-join exactly one such same-cycle ownership;
  every other ownership state must have NULL result identity and zero terminal
  results, while every `InterruptedUncertain` ownership also requires exact
  `ProcessInterruptedAfterSinkStart` reason;
- there is no committed retry-origin `Reserved` state without that binding, and
  no committed binding/schedule increment without the matching `Reserved`
  state; and
- historical authorizations and attempt bindings remain valid after a new
  disposition or terminal state and must never be rejected merely for no
  longer matching current state.

## 4. Cycle-bound immutable audit

Add `retry_cycles` as a materialized projection and
`retry_cycle_audit_outbox` as the append-only authority.

`retry_cycles` records cycle identity, a retained
`cycle_ordinal INTEGER NOT NULL UNIQUE CHECK(cycle_ordinal>=1)`, namespace
hash, the non-null validated owner boot identity passed explicitly by the
pre-open boot authority, scheduled time, start time, terminal state
(`Running|Completed|Failed`), sorted candidate-query count, queried-candidate
count/hash, provider/renderer counts, tri-state sink-call evidence, terminal
phase/reason, immutable failure-payload identity, typed-preimage/envelope
digests and completion time. The ordinal and row cannot be updated or deleted.
Sink-call evidence is
exactly `NotStarted/NULL`, `Confirmed/n` or
`Indeterminate/NULL`. Clean execution that proves no boundary was crossed may
record `Confirmed/0`; panic, cancellation, process death or `JoinError` after
start/claim evidence records `Indeterminate/NULL`. Recovery may separately
prove that it made zero new sink calls, but must not rewrite an interrupted
cycle to `Confirmed/0`. The §4 Transaction-A recount is not such an inference:
after the exact no-call ownership terminal is durable, it derives
the exact ownership/result bijection from all same-cycle rows. If any other
`Started` or `InterruptedUncertain` ownership remains it checked-updates
`Indeterminate/NULL` to itself; otherwise it restores `Confirmed(n)`.

Cycle identity has one canonical construction. Inside the same
`BEGIN IMMEDIATE`, before the global Running check and before any insert or
write, the coordinator reads
`COALESCE(MAX(cycle_ordinal),0)`, rejects `i64::MAX` with
`RetryCycleOrdinalExhausted { max_ordinal: i64::MAX }`, and uses checked
addition to obtain the next positive ordinal. It then constructs this exact
ordered preimage:

```text
schema_version=1
rule_id="BR-192"
namespace_sha256
owner_boot_identity
scheduled_for
started_at
cycle_ordinal
```

`scheduled_for` and `started_at` are UTC strings produced only by
`to_rfc3339_opts(SecondsFormat::Nanos, true)`, so they always use `Z` and nine
fractional digits. `cycle_ordinal` is a JSON integer. The preimage is a
declared-field struct serialized with `serde_json::to_vec` as compact UTF-8
JSON in exactly the order above, with no map, whitespace, BOM, trailing
newline or Unicode normalization. `namespace_sha256` is lowercase 64-hex and
all non-hash text retains its validated bytes. The domain literal is exactly
`stock_analysis.durable_delivery.br192.retry_cycle_identity.v1\0`, and
`cycle_identity` is lowercase
`SHA-256(domain || canonical_preimage_bytes)`. Database values and the
`Started` payload must rederive to the same identity and ordinal. Exact
recomputation plus domain/schema/field-order/timestamp-encoding/ordinal
mutation tests reject every alternative.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "confirmed_count", deny_unknown_fields)]
pub enum RetryCycleSinkCalls {
    NotStarted,
    Confirmed(usize),
    Indeterminate,
}
```

Terminal phase is exactly
`NotPrepared|CompletionPending|CompletionAppended|FailurePending|
FailureAppended|Terminalized`. Its invariants are:

- `Running/NotPrepared` has no `Completed` or `Failed` terminal slot and no
  failure payload;
- `Running/CompletionPending|CompletionAppended` has exactly one canonical
  `Completed` slot, no `Failed` slot and no failure payload;
- `Running/FailurePending|FailureAppended` has exactly one canonical `Failed`
  slot and its complete immutable failure payload, and has no `Completed` slot;
- `Completed/Terminalized` retains the exact appended `Completed` slot and has
  no failure payload; and
- `Failed/Terminalized` retains the exact appended `Failed` slot, a closed
  `RetryCycleFailureReason`, one immutable `retry_cycle_failure_payloads` row
  and lowercase 64-hex typed-preimage plus envelope hashes.

The first committed terminal slot irrevocably chooses the terminal kind. Once
either a Pending or Appended `Completed` exists, no path may insert, append or
terminalize `Failed`, including an error while appending, acknowledging or
CASing completion. Conversely, once `Failed` exists, completion is forbidden.
Missing, empty or ASCII-whitespace-only boot identity fails before the cycle
transaction or any `retry_cycles`/`Started` insert.

`retry_cycle_audit_outbox` records:

```text
cycle_event_identity                      PRIMARY KEY
cycle_identity                            REFERENCES retry_cycles
decision_identity                         nullable REFERENCES delivery_decisions
event_ordinal                             non-negative logical slot ordinal
event_kind                                Started | CandidateObserved |
                                          AuthorizationReconciliationBlocked |
                                          DuplicateSuppressed |
                                          AdmissionResult | SinkAttemptStarted |
                                          OrphanRecovered | Completed | Failed
event_canonical                           immutable BLOB
event_sha256                              immutable TEXT
append_state                              Pending | Appended
immutable_audit_ref                        nullable
created_at
```

Payload columns are immutable by trigger; only `Pending -> Appended` and the
matching immutable ref acknowledgement may change. Records are retained and
cannot be deleted. Logical event cardinality is independent of payload bytes:

```text
UNIQUE(cycle_identity,
       COALESCE(decision_identity,'__BR192_CYCLE_SCOPE__'),
       event_kind,
       event_ordinal)
UNIQUE(cycle_identity)
  WHERE event_kind IN ('Completed','Failed')
```

The canonical `Completed` outbox payload has ordered fields
`schema_version,rule_id,event_kind,cycle_identity,candidate_query_calls,
queried_candidate_count,sorted_candidate_sha256,provider_calls,renderer_calls,
sink_calls_state,sink_calls_count,completed_at`. `sorted_candidate_sha256` is
required lowercase 64-hex when `queried_candidate_count>0`, otherwise it is
exactly null; `sink_calls_count` follows the tri-state invariant. These exact
bytes are frozen in the same transaction that chooses `CompletionPending`.

The canonical `Failed` outbox payload has ordered fields
`schema_version,rule_id,event_kind,cycle_identity,failure_reason,
failure_payload_identity,typed_preimage_sha256,failure_envelope_sha256,
failed_at`. `failure_reason` is serialized only from the
closed `RetryCycleFailureReason`; it is never accepted as an arbitrary caller
string. The cycle row stores the same reason token/payload identity/hashes, and
post-validation requires byte-for-byte agreement among cycle, immutable
failure payload and outbox before commit.

Completion terminalization is deliberately split:

1. `prepare_retry_cycle_completed` validates complete observer evidence,
   proves every cycle-owned attempt is terminal or safely reconciled, inserts
   the unique canonical `Completed` outbox as `Pending`, freezes its evidence
   in the cycle, and changes only
   `Running/NotPrepared -> Running/CompletionPending`;
2. append authority appends those exact bytes;
3. acknowledgement changes both the exact outbox to `Appended` and the cycle
   to `Running/CompletionAppended` in one transaction; and
4. `terminalize_retry_cycle_completed` revalidates the immutable ref and exact
   canonical bytes before the sole
   `Running/CompletionAppended -> Completed/Terminalized` CAS.

An error before `prepare_retry_cycle_completed` commits may be converted to a
typed `CycleOperationFailed` only after the coordinator proves that neither
terminal slot exists. An error after `CompletionPending` exists must return or
reload that exact recoverable completion phase and resume only its append,
acknowledgement or terminal CAS. It must never invoke failure preparation.

Failure terminalization is deliberately split:

1. `prepare_retry_cycle_failed` recomputes the opaque failure's private typed
   preimage, complete canonical envelope and both digests, persists the
   immutable full payload row, inserts the unique canonical `Failed` outbox as
   `Pending`, first proves that no terminal slot exists, and changes only
   `Running/NotPrepared -> Running/FailurePending`;
2. append authority appends those exact bytes;
3. acknowledgement changes both the exact outbox to `Appended` and the cycle
   to `Running/FailureAppended` in one transaction; and
4. `terminalize_retry_cycle_failed` revalidates the immutable ref, persisted
   typed-preimage/envelope canonical bytes, both hashes, closed reason and
   uncertainty fixed point before the sole
   `Running/FailureAppended -> Failed/Terminalized` CAS.

An append/ack failure leaves `Running/FailurePending`; a crash after ack leaves
`Running/FailureAppended`. Neither is reported as terminal `Failed`. A
completion append/ack failure similarly leaves `Running/CompletionPending`;
a crash after completion acknowledgement leaves
`Running/CompletionAppended`. Same-boot, `JoinError` and boot recovery always
inspect the committed terminal slot before constructing any failure:

1. `CompletionPending|CompletionAppended` validates and resumes only the exact
   stored `Completed` bytes through append/ack/CAS with zero sink calls;
2. `FailurePending|FailureAppended` validates the complete stored failure
   payload and resumes only the already-prepared `Failed` bytes; and
3. only `NotPrepared` with no terminal slot may quarantine uncertainty to a
   fixed point and then prepare `ProcessInterrupted`.

A slot kind/phase mismatch, two terminal kinds, mutated canonical bytes or an
unresolvable immutable ref fails closed and performs no alternative terminal
insert.

Global events (`Started`, `AuthorizationReconciliationBlocked`,
`OrphanRecovered`, `Completed`, `Failed`) have null `decision_identity` and
ordinal zero. `CandidateObserved`, `DuplicateSuppressed` and
`AdmissionResult` have a decision identity and ordinal zero.
`SinkAttemptStarted` has a decision identity and uses its positive reservation
generation as the ordinal. The sentinel is reserved by a CHECK and cannot be a
decision identity.

`cycle_event_identity` is domain-separated only by the logical slot
(cycle/scope/kind/ordinal), not by the payload hash. Inserting the same slot
with byte-identical canonical payload returns the stored row. A different
canonical payload or hash for that slot is an immutable conflict and fails the
cycle closed. Post-validation requires exactly one `Started`, at most one
authorization-blocked event, exactly one of `Completed` or `Failed` for a
terminal cycle, one `AdmissionResult` for every candidate that reached
admission, one `DuplicateSuppressed` for every attempted-set duplicate, and an
exact attempt-binding match for every `SinkAttemptStarted`: the outbox
`cycle_identity` owns the current execution slot, while the event's
`admission_cycle_identity` must match the immutable binding's cycle.

Every candidate that reaches admission receives exactly one `AdmissionResult`
per cycle, including all reachable `Deferred` and `NoLongerEligible` variants.
Pending authorization is instead represented by authorization state plus a
cycle `AuthorizationReconciliationBlocked` event. A same-cycle candidate
already in the attempted set does not call admission; it receives the
independent `DuplicateSuppressed` event and is never disguised as
`StateChanged`. Events include rule ID, cycle identity, observation time,
hashes rather than sensitive identities, authorization/disposition hashes
where applicable, reservation generation, retry counter, next eligible time
and zero provider/renderer calls.

The retry sink boundary is deliberately split:

```text
coordinator.prepare_retry_attempt(...)
  -> RetryAttemptPreparationOutcome
coordinator.reconcile_prepared_retry_attempt_audit(prepared, append)
  -> AppendedSinkAttemptStarted
coordinator.claim_retry_sink_execution(prepared, appended_start)
  -> RetrySinkClaimOutcome
coordinator.execute_prepared_retry_sink(permit, sink)
  -> RetrySinkExecutionOutcome
```

These are `DurableDeliveryCoordinator` methods. None is a free function and
none may open a coordinator through global state. The explicit `&self`
capability owns every database read, CAS and terminal write.

The append acknowledgement crossing that boundary is a frozen canonical DTO:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendedSinkAttemptStarted {
    pub cycle_event_identity: String,
    pub cycle_identity: String,
    pub admission_cycle_identity: String,
    pub attempt_identity: String,
    pub decision_identity: String,
    pub authorization_identity: String,
    pub rejection_disposition_identity: String,
    pub authorization_binding_identity: String,
    pub retry_ordinal: i64,
    pub binding_generation: i64,
    pub reservation_generation: i64,
    pub owner_instance_identity: String,
    pub fence_token: i64,
    pub started_at: DateTime<Utc>,
    pub event_canonical: Vec<u8>,
    pub event_sha256: String,
    pub immutable_audit_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RetrySendOwnershipState {
    Started,
    FreshnessExpiredBeforeExternalCall,
    TerminalRecorded,
    InterruptedUncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedRetrySinkOutcome {
    pub decision_identity: String,
    pub attempt_identity: String,
    pub sink_result_identity: String,
    pub decision_state: DecisionState,
    pub ownership_state: RetrySendOwnershipState,
    pub pre_call_freshness_observed_at: DateTime<Utc>,
}

pub enum RetryAttemptPreparationOutcome {
    Prepared(PreparedRetryAttempt),
    Expiry(RetryExpiryPreparationOutcome),
}

pub enum RetrySinkClaimOutcome {
    Claimed(SinkExecutionPermit),
    Expiry(RetryExpiryPreparationOutcome),
}

pub enum RetrySinkExecutionOutcome {
    Persisted(PersistedRetrySinkOutcome),
    ExpiredBeforeExternalCall(PreparedRetryExpiredFreshness),
}
```

Its `event_canonical` bytes are the repository canonical UTF-8 JSON encoding
of `SinkAttemptStartedCanonicalV1` with exact ordered fields:
`schema_version=1`, `rule_id="BR-192"`,
`event_kind="SinkAttemptStarted"`, `cycle_event_identity`,
`cycle_identity`, `admission_cycle_identity`, `attempt_identity`,
`decision_identity`,
`authorization_identity`, `rejection_disposition_identity`,
`authorization_binding_identity`, `retry_ordinal`, `binding_generation`,
`reservation_generation`, `owner_instance_identity`, `fence_token` and
`started_at`. The logical-slot identity is lowercase hex:

```text
SHA-256(
  b"stock_analysis.durable_delivery.br192.retry_cycle_event.v1\0"
  || canonical_utf8_json(
       cycle_identity,
       decision_identity,
       "SinkAttemptStarted",
       reservation_generation
     )
)
```

`coordinator.validate_appended_sink_attempt_started(prepared, appended)` fails
closed unless
all identities, ordinal, generations, owner and fence equal the
`PreparedRetryAttempt`. Here `cycle_identity` is the current execution/recovery
cycle that owns the outbox slot, while `admission_cycle_identity` must equal
the attempt binding's cycle identity frozen into that prepared token; on the
initial execution they are equal, and after a process restart they may differ.
It also requires positive ordinal/generations/fence, a
lowercase 64-hex SHA-256 equal to `SHA-256(event_canonical)`, canonical bytes
that deserialize with `deny_unknown_fields` to the exact
`SinkAttemptStarted` logical slot and payload, and a non-empty immutable ref
after trimming only ASCII space/tab/LF/CR. `cycle_event_identity` must equal the
domain-separated identity derived from cycle, decision, event kind
`SinkAttemptStarted` and `reservation_generation`; it is never accepted merely
because it exists. `started_at` must equal the persisted outbox row's
`created_at`, the canonical payload's `started_at` and the
`PreparedRetryAttempt.started_at` byte-for-byte after canonical timestamp
normalization. The append acknowledgement carries no append-authority
timestamp because that authority does not provide one.

`coordinator.prepare_retry_attempt(execution_cycle_identity, attempt_identity)`
never calls
`begin_attempt`, selects a generation, creates an attempt/binding or changes the
schedule. It loads the immutable binding installed by admission, revalidates its
admission cycle, authorization, disposition, generation, ordinal, owner and
fence against the already-`Reserved` decision/attempt/schedule, validates the
current execution cycle is `Running`, idempotently inserts that execution
cycle's exact `SinkAttemptStarted` outbox row and returns
`RetryAttemptPreparationOutcome::Prepared`, carrying both cycle identities and
the persisted outbox `created_at` as `started_at`. If the private clock instead
observes expiry before any current-attempt start authority exists, the same
transaction writes only the exact Pending expiry authority and returns
`Expiry(ExpiryPrepared(...))`. If a Pending/Appended current-attempt start or
ordinary ownership already exists, it writes no expiry row and returns
`Expiry(StartAuthorityWins(...))`. It has no sink capability.
`coordinator.reconcile_prepared_retry_attempt_audit` appends and acknowledges
that exact row and has no sink capability. Appending `SinkAttemptStarted` is
audit evidence, not permission to send.

`coordinator.claim_retry_sink_execution(prepared, appended_start)` has no
sink capability. In one `BEGIN IMMEDIATE` it revalidates the complete
prepared/start DTO and database read-back. If its private clock now observes
expiry, the already-Pending/Appended start wins: the method returns
`RetrySinkClaimOutcome::Expiry(StartAuthorityWins(...))`, writes no expiry row,
mints no permit and routes the retained attempt through uncertainty. Otherwise
it CASes exactly
`Reserved -> AttemptInFlight` for the current attempt/binding/reservation
generation/owner/positive-i64 fence, and inserts the matching immutable
`retry_send_ownership` with `send_consumed=true` and the persisted start time.
The same transaction changes cycle sink-call evidence from `NotStarted` or
`Confirmed(n)` to `Indeterminate/NULL`; once a consumed permit exists, a later
panic/crash may not report zero merely because no result was recorded.
Only that CAS winner receives
`RetrySinkClaimOutcome::Claimed(SinkExecutionPermit)` with a non-`Clone`,
non-serializable, single-consumption permit; a loser returns a typed state/fence
loss and makes zero sink calls.

`coordinator.execute_prepared_retry_sink(permit, sink)` takes the permit by
value and is the only retry API with a sink capability. It performs one final
read-only validation that the decision is `AttemptInFlight`, the exact
ownership remains `Started`, and attempt/binding/fence/start evidence still
match. After that validation and with no intervening await, provider, renderer,
filesystem, database transaction or other blocking operation, it reads the
private production clock at the external-call linearization point.

If `pre_call_freshness_observed_at >= expires_at`, it invokes no external sink
and executes the following exact two-transaction protocol.

**Transaction A — establish immutable no-call expiry authority.** In one
`BEGIN IMMEDIATE`, the coordinator revalidates the unchanged consumed permit
and all of these preconditions: the decision is `AttemptInFlight` and names the
exact current attempt; the attempt/binding/generation/owner/fence agree; the
schedule is `Active` and its last binding is that attempt; its exact
`SinkAttemptStarted` is `Appended`; ownership is `Started`; the execution cycle
is `Running/NotPrepared`; no authoritative sink result exists; and no pre-call
companion or expiry authority exists. It then, in order:

1. inserts the immutable deferred `retry_pre_call_expiry_authorities` row;
2. advances ownership exactly once to
   `FreshnessExpiredBeforeExternalCall`, persisting the observation and exact
   terminal reason/time;
3. inserts the matching Pending `ReservedExpiredBeforeSink` expiry outbox; and
4. recomputes and CASes the cycle's sink-call evidence from retained rows.

That recount treats a same-cycle `Started` or `InterruptedUncertain` ownership
as ambiguous. If any exists, the only valid evidence is
`Indeterminate/NULL`. Otherwise `n` is the count of same-cycle
`TerminalRecorded` ownership rows. Each `TerminalRecorded` ownership must join
exactly one authoritative sink result, and every authoritative sink result for
the cycle must join exactly one same-cycle `TerminalRecorded` ownership. An
orphan/extra result, an ownership with zero or multiple results, or a result
joined to zero or multiple ownership rows is corruption and rolls the
transaction back. Only that bijection may produce `Confirmed(n)`.
`FreshnessExpiredBeforeExternalCall` contributes zero and is not ambiguous, so
the all-expiry case closes to `Confirmed(0)` while earlier definite same-cycle
results produce `Confirmed(n>0)`. A missing/double result, stale evidence CAS,
failed statement or post-validation mismatch rolls back the companion,
ownership, expiry and cycle-evidence changes together. The decision, schedule,
reservation and base attempt state intentionally remain unchanged until the
exact expiry bytes are appended. On commit the method returns
`ExpiredBeforeExternalCall(prepared)`; its caller may only append/acknowledge
and reconcile those exact persisted bytes.

**Transaction B — terminalize only appended expiry authority.** After the
exact Pending row is appended and acknowledged as `Appended`, normal expiry
reconciliation enters one `BEGIN IMMEDIATE`. It requires the exact appended
expiry ref/hash/canonical bytes and `ReservedExpiredBeforeSink` kind; exact
decision/current-attempt/binding/generation/owner/fence and original rejection;
the `Active` schedule and authorization binding; the exact companion plus
freshness-terminal ownership; no sink result; the current reserved budget,
cooldown and reservation generation; and cycle evidence equal to the recount
defined above. It then performs only these checked mutations:

- retain `delivery_attempts.state='AttemptInFlight'` for schema-v5
  compatibility, but CAS `fence_revoked_at` from null to the exact freshness
  observation;
- release each exact current `Reserved` reservation, set its `released_at` to
  that observation, release the current rolling head, and retain all rows and
  generations;
- use the sole restricted direct decision transition
  `AttemptInFlight -> RejectedDurable`, preserve the original current rejection
  disposition, clear current attempt/budget/cooldown pointers, checked-
  increment the fence generation, and persist exact reason
  `retry_expired_freshness_before_external_call`;
- clear the exact `Active` authorization binding with
  `cleared_reason='expired_freshness'`, `cleared_at` equal to the observation
  and `retry_authorized=false`, retaining the authorization and its event; and
- CAS the schedule `Active -> ExpiredFreshness`, clear `next_eligible_at`,
  increment `version` exactly once, and preserve attempts started, last binding,
  source date and expiry.

Every affected-row count is exactly one and full post-validation rederives all
joins before commit; otherwise Transaction B rolls back completely. A replay
that observes the exact full after-state returns
`RetryExpiryDisposition::AlreadyTerminalized`. Any partial after-state,
mismatched reason/time/ref/hash/generation or drift is a typed
`DurableDeliveryError` and is never patched piecemeal.

Because schema v5 restricts the base attempt state to its existing closed set,
the retained `AttemptInFlight` attempt row is compatibility history, not live
send authority. Adding a new base state would require a table rebuild;
`Rejected` would falsely claim a sink result and `Uncertain` would contradict
the definite zero-call proof. The effective terminal state
`ExpiredFreshnessBeforeSink` is therefore defined by the exact conjunction of:
an appended `ReservedExpiredBeforeSink` expiry; schedule
`ExpiredFreshness`; decision no longer pointing to the attempt; revoked fence
at the exact observation; released reservations; no sink result; and either
no start/ownership for ordinary pre-start expiry or the exact companion plus
`FreshnessExpiredBeforeExternalCall` ownership for final pre-call expiry. All
selectors, recovery classifiers, post-validators and cycle evidence rules must
recognize and exclude this effective terminal relation **before** inspecting
the base attempt state. It cannot be claimed, resent or relabelled uncertain.

Otherwise the same observed timestamp is retained in memory and the method
immediately invokes the external sink exactly once. `record_sink_result` then
derives the prospective authoritative/non-late sink-result identity and opens
one transaction that continues to require `AttemptInFlight`. It first persists
the exact pre-call timestamp and prospective result identity in the ownership
row while advancing `Started -> TerminalRecorded`, then inserts the matching
authoritative/non-late sink result, and finally validates the cycle-wide
result/`TerminalRecorded` ownership relation is bijective before commit. The
deferred FK makes the temporary pointer-first state legal only inside that
transaction, while the reverse trigger rejects any result-first insert
immediately. A crash before commit leaves no claimed freshness fact and follows
the existing conservative `InterruptedUncertain` recovery; it can never be
converted into a definite expiry. Before returning to its caller the method
proves no
nonterminal consumed ownership remains, counts that joined authoritative retry
sink-result set, and changes
`Indeterminate/NULL -> Confirmed(n)`. A later claim may change
`Confirmed(n) -> Indeterminate/NULL` again. Execute
returns only `RetrySinkExecutionOutcome`, whose successful variant contains
the stored sink-result identity, decision state, ownership state and exact
pre-call freshness observation. It never returns the bare port result.

A failpoint after the ownership pointer/state update but before the matching
authoritative/non-late result insert must roll back that entire result
transaction. The decision remains `AttemptInFlight`, no authoritative result
row exists, ownership remains `Started` with a NULL terminal-result pointer,
and recovery conservatively records
`ProcessInterruptedAfterSinkStart`, reaches manual review and makes zero sink
calls while the interrupted cycle remains `Indeterminate/NULL`. A
pending/missing/mismatched start event, lost CAS, repeated execute or
ownership mismatch makes zero sink calls. No convenience method may combine
or bypass these four stages. The same-process repetition test must not forge or
clone a permit: it proves the permit is neither `Clone` nor serializable,
invokes the claim twice, receives only one permit, consumes it once and
observes no second external call.

Cycle `Failed` is also durable. Logs and the returned summary are observers;
they are not the audit authority.

## 5. Runner data flow

One provider-free cycle is:

1. receive the cycle identity whose `Running` row and `Started` outbox were
   already committed by the async parent before the long blocking task spawned;
2. reconcile pending disposition, authorization, state, task and cycle audit
   bytes to a fixed point; a blocked authorization records
   `AuthorizationReconciliationBlocked`, fails the cycle and yields zero sinks;
3. first resume every recoverable terminal slot, then reconcile every exact
   pre-call companion + `FreshnessExpiredBeforeExternalCall` ownership +
   Pending/Appended `ReservedExpiredBeforeSink` expiry through append/ack and
   Transaction B; only after that higher-priority pass may it quarantine any
   remaining prior-boot retry send starts/in-flight attempts with no terminal
   authoritative result, then snapshot only safely recoverable already-
   `Reserved` identities in the exact order frozen below;
4. create one cycle-global `BTreeSet<String> attempted_decision_identities`;
5. for each already-`Reserved` identity:
   - insert it into the attempted set before any sink call;
   - if already present, record a duplicate suppression event;
   - if it is retry-origin with no prior-boot start/ownership, use the
     four-stage prepare -> append/ack -> claim -> execute seam;
   - if an exact pre-call expiry triple is present, resume expiry and never
     enter this generic branch; otherwise, if any prior-boot
     `SinkAttemptStarted` is pending/appended or the decision is
     `AttemptInFlight` without a terminal authoritative result, append/ack the
     retained bytes, transition through typed
     `ProcessInterruptedAfterSinkStart` uncertainty and make zero sink calls;
   - otherwise preserve the existing ordinary Reserved recovery seam;
   - resume it at most once;
6. reconcile all resulting disposition, authorization, authorization-event,
   binding, state, task and cycle bytes to a fixed point;
7. only now query authorized retry candidates in the exact order frozen below,
   so a same-cycle newly rejected and newly authorized decision is observable;
8. for each retry candidate:
   - if the identity is already in the attempted set, append the independent
     `DuplicateSuppressed` cycle event and do not admit;
   - otherwise call `admit_authorized_retry` once;
   - record and append every typed result;
   - for `Reacquired`, reconcile admission bytes, insert the identity into the
     attempted set, then use the admission-returned attempt identity for prepare
     -> append/ack start event -> claim send ownership -> execute once;
9. never revisit an identity after a sink call even if reconciliation changes
   its state or generation;
10. reconcile to a fixed point;
11. if no terminal slot exists, prepare exactly one terminal kind; append,
    acknowledge and terminalize it. If a slot already exists, resume only that
    exact `Completed` or `Failed` slot; and
12. return observer evidence containing candidate-query count/identities and
    actual provider, renderer and sink probe deltas.

`retry_cycle_blocking` returns
`std::result::Result<RetryCycleEvidence, RetryCycleFailure>`, never
`Result<_, String>`. Every surrounding runner, boot-authority, namespace,
guard, terminal-finalizer and prior-boot recovery function returns the existing
`durable_delivery::Result<T>` alias and propagates the exact
`DurableDeliveryError` variant without `Display` parsing, `.to_string()`,
`map_err` to text or a generic variant. The retry-only runtime-state accessor
is a typed sibling of the legacy monitor accessor: it factors and invokes the
typed namespace/cache/build primitives directly and must not call a
`Result<_, String>` accessor.

`RetryCycleGuard::acquire` returns
`RetryCycleGuardCompareExchangeInvariant` for the impossible atomic outcome;
an already-held guard becomes `RetryCycleAlreadyRunning`. A definite
`RetryCycleBeginOutcome::NotCommitted` consumes its opaque proof, releases the
guard and returns the exact embedded error. A commit-ambiguous begin returns
its exact error and leaves the guard latched. Completion/failure finalizers and
guard-release verification likewise preserve exact variants; the final
monitor logging boundary may render an error only after the typed return and
required durable audit state exist.

`RetryCycleFailure` is constructed at the typed business-failure boundary,
validates its enum reason and lowercase 64-hex typed-field digest, and passes
unchanged into `finish_cycle_failed_and_append`. Runtime calls only the six
frozen public associated constructors and supplies `RetryCycleOperation` for a
cycle operation; panic, `JoinError`, orphan and generic operation failures pass
only their own redacted stable lowercase 64-hex digest. In particular the
panic and join branches call `from_panic_sha256(&str)` and
`from_join_error_sha256(&str)` exactly. No runtime path can name a private
field/preimage, reconstruct a failure reason by parsing display text, or call
short-form `panic`/`join_error` constructors.

### 5.1 Exact stable ordering

The three primary cycle-work selectors reject rows missing a required key
rather than placing nulls implicitly. Collation is SQLite binary byte order for
canonical identity strings and RFC3339 UTC timestamps:

1. safely recoverable `Reserved` work requires non-null
   `current_attempt_identity` and orders by
   `business_date ASC, created_at ASC, decision_identity ASC,
   current_attempt_identity ASC`; the final attempt identity is the total-order
   tie-break;
2. different-boot `Running` cycles require non-null `scheduled_for`,
   `started_at`, `cycle_identity` and order first by terminal recovery priority
   `CompletionAppended=0, CompletionPending=1, FailureAppended=2,
   FailurePending=3, NotPrepared=4`, then
   `scheduled_for ASC, started_at ASC, cycle_identity ASC`; and
3. authorized retry candidates require
   `next_eligible_at IS NOT NULL`, `next_eligible_at<=now`,
   `exhausted_at IS NULL`, `terminal_state='Active'`, `now < expires_at` and
   non-null source-date/expiry/decision/disposition/authorization
   identities, then order by
   `next_eligible_at ASC, decision_identity ASC,
   rejection_disposition_identity ASC, authorization_identity ASC`.

Each primary selector is exactly one complete, validated, unbounded snapshot.
Its production SQL contains no `LIMIT`, `OFFSET`, keyset predicate, caller
cursor or caller-supplied cardinality. Before returning any row, it validates
every qualifying row and all required sort keys, sorts by the complete tuple
above and freezes the final vector length and SHA-256. One invocation therefore
returns every row qualifying in that SQLite read snapshot; exhaustion means
consuming exactly that vector length, with one cycle event for every position.
There is no continuation token or silently omitted tail.

The different-boot `Running` snapshot is completely recovered before a new
cycle may begin. The `Reserved` snapshot is completely consumed before the
retry-candidate snapshot is taken. The retry-candidate snapshot is taken once
after authorization reconciliation and Reserved processing; a concurrently
committed authorization after that read belongs to the next cycle, while every
identity in the frozen snapshot is attempted or receives explicit
`DuplicateSuppressed`/admission evidence in the current cycle. The exclusive
runner lock prevents another retry cycle from adding prior-boot or Reserved
work during these phases. A selector read/validation failure returns no partial
vector, makes zero sink calls and leaves the phase unadvanced.

No selector uses `rowid`, unordered hash/set iteration, caller order, implicit
NULL ordering, any form of SQL/result truncation, or a mutable local timestamp.
The cycle-global `BTreeSet` is suppression evidence rather than a source of
query order. Tests seed 257 qualifying rows with equal leading keys for each
selector, proving the complete tail, final tie-break, frozen count/hash and
phase exhaustion; source/static tests reject `LIMIT`, `OFFSET`, cursor and
caller-cardinality parameters in all three selector bodies.

The one-row cycle-audit drain is independently total-ordered. It rejects null,
empty or ASCII-whitespace-only `created_at`, `cycle_identity`,
`cycle_event_identity` or `event_kind`; a non-null `decision_identity` must also
be non-empty and ASCII-whitespace-stable. It selects exactly:

```sql
WHERE append_state = 'Pending'
ORDER BY created_at COLLATE BINARY ASC,
         cycle_identity COLLATE BINARY ASC,
         CASE WHEN decision_identity IS NULL THEN 0 ELSE 1 END ASC,
         COALESCE(decision_identity, '') COLLATE BINARY ASC,
         event_kind COLLATE BINARY ASC,
         event_ordinal ASC,
         cycle_event_identity COLLATE BINARY ASC
LIMIT 1
```

The explicit case places cycle-scoped null before decision-scoped identity; it
is the only allowed null ordering. `cycle_event_identity` is the final total
tie-break. A caller permits exactly
`MAX_CYCLE_AUDIT_RECONCILE_STEPS_PER_RUN = 4096` successful one-row
acknowledgements, followed by one non-mutating pending-row check. If a pending
row remains, typed
`DurableDeliveryError::RetryCycleAuditReconciliationBoundExceeded {
max_steps: 4096, pending_cycle_event_identity }` leaves the event and terminal
phase unchanged, creates no opposite terminal slot, makes zero sink calls and
keeps the cycle guard latched. No pending row is the only successful fixed
point.

A crash after the admission commit but before prepare leaves one `Reserved`
decision with its authoritative attempt, immutable attempt binding and schedule
increment already committed. The next boot first terminalizes the prior-boot
cycle through its normal `ProcessInterrupted` recovery, then the new startup
cycle's Reserved phase loads that same binding/attempt identity. It preserves
the binding's admission-cycle identity, prepares and appends a
`SinkAttemptStarted` logical slot owned by the new execution cycle, wins the
pre-call CAS and proceeds once. It must not allocate another generation,
attempt identity, retry ordinal or schedule increment. It never appends new
bytes to the already-terminal prior cycle.

Once a start event for a boot has been appended, start/ownership evidence alone
must never be treated as proof that resending is safe. A crash after the
pre-call CAS but before the actual external call is conservatively
indistinguishable from a completed remote call. A crash after a remote accept
but before `record_sink_result`, a same-process repeated execute and a
second-process execute attempt follow the same no-resend rule. Startup/cycle
recovery makes zero sink calls, records the typed reason
`ProcessInterruptedAfterSinkStart`, moves through
`UncertainAuditPending -> UncertainTaskTransitionPending ->
UncertainManualReview`, and marks send ownership `InterruptedUncertain`.
Only a terminal authoritative result already committed by
`record_sink_result` avoids quarantine. No sink-query idempotency capability is
assumed; without one, even a proven crash-before-actual-call remains
conservatively uncertain.

The same-cycle duplicate test is not allowed to pass because the candidate was
never queried. It proves the post-Reserved query contains the newly rejected
decision. A test-only `AttemptedSetMode::Disabled` harness (not compiled into
production) proves the attempted set is an optimization/audit feature rather
than the no-resend authority: the second path must still lose the admission or
pre-call CAS and the sink count remains one. With production tracking enabled,
the same fixture also emits one `DuplicateSuppressed` event.

Startup invokes this exact cycle once before `producer_ready=true`. The
periodic task uses the same function. Neither path recursively invokes
`ensure_startup_reconciled`.

## 6. Scheduling and cancellation

### 6.1 Frozen namespace hash

Every cycle namespace hash is produced by one exported helper and one canonical
preimage; callers cannot hash paths, debug strings or ad-hoc concatenations:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RetryNamespaceKind {
    Production,
    Test,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryNamespaceHashPreimageV1 {
    pub schema_version: u8,
    pub rule_id: String,
    pub namespace_kind: RetryNamespaceKind,
    pub test_code: Option<String>,
}

pub fn retry_namespace_sha256(
    preimage: &RetryNamespaceHashPreimageV1,
) -> Result<String>;
```

The only valid `schema_version` is `1` and the only valid `rule_id` is
`BR-192`. Production requires `test_code=None`; Test requires one non-empty,
ASCII-whitespace-trimmed `TEST_CODE_*` value. Canonical bytes are UTF-8 JSON
from this exact struct field order
`schema_version,rule_id,namespace_kind,test_code`, with no unknown fields and
with the repository canonical serializer; the hash is lowercase hex:

```text
SHA-256(
  b"stock_analysis.durable_delivery.br192.retry_namespace.v1\0"
  || canonical_utf8_bytes
)
```

The validator rebuilds canonical bytes and recomputes the hash. UTF-8 is
preserved exactly; field boundaries cannot collide. The helper and types are
re-exported from `durable_delivery`, and both runner and manual evidence paths
must use them.

Before coordinator/append open, runtime acquires one
`RetryRunnerBootAuthority`: it owns the exclusive process-lifetime runner lock
and one validated `owner_boot_identity`. `RuntimeState` retains that authority
for its entire lifetime and owns one `Arc<AtomicBool> retry_cycle_running`.
Before guard acquisition, the async parent reads the identity from that
retained authority and canonically derives and validates the complete namespace
preimage plus its hash. Namespace/preimage/hash or owner-boot validation errors
therefore occur before a running flag can be acquired. The parent then acquires
the guard. Before creating a cycle it calls the same-boot safe-terminal
resumer:

```rust
pub fn resume_same_boot_retry_cycle_terminal_slots(
    &self,
    append: &dyn ImmutableAppendPort,
    current_owner_boot_identity: &str,
    now: DateTime<Utc>,
) -> Result<Vec<RetryCycleEvidence>>;
```

The selector is exactly
`state='Running' AND owner_boot_identity=current_owner_boot_identity AND
terminal_phase IN ('CompletionAppended','CompletionPending',
'FailureAppended','FailurePending')`. It rejects missing ordering keys,
materializes the complete snapshot, and orders by phase priority
`CompletionAppended=0, CompletionPending=1, FailureAppended=2,
FailurePending=3`, then `scheduled_for ASC, started_at ASC,
cycle_identity ASC`, all text comparisons `COLLATE BINARY`. Each row is passed
only to `resume_retry_cycle_terminal_slot`, which validates and reuses the
stored kind, bytes and hashes, performs append/acknowledgement if necessary and
finishes the exact terminal CAS. The resumer has only coordinator and immutable
append authority; it cannot construct or receive a sink/provider/renderer
capability. Any selector, append, acknowledgement or CAS failure returns the
original typed error, creates no new cycle or `Started` slot and deliberately
leaves the acquired same-boot guard latched for restart recovery.

Only after the resumer reaches an empty fixed point does the parent call the
single named coordinator API:

```text
begin_retry_cycle_before_spawn(
  namespace_sha256,
  owner_boot_identity,
  scheduled_for,
  now
)
  -> durable_delivery::Result<RetryCycleBeginOutcome>

RetryCycleBeginOutcome =
  Started(cycle_identity)
  | NotCommitted(error: DurableDeliveryError, proof: NoRetryCycleCommitted)
```

`Ok(NotCommitted { .. })` is the only definite no-commit branch and carries
the single-use release proof. `Err(DurableDeliveryError)` is the sole
commit-ambiguous operational channel, carries no release proof and leaves the
guard latched. No `CommitAmbiguousError` or second operational error enum
exists.

The coordinator validates the explicit boot identity again. In the same
`BEGIN IMMEDIATE`, before any insert/write, it computes the next ordinal and
the exact proposed cycle identity as frozen above. It then selects the first
unresolved global Running row by
`started_at COLLATE BINARY ASC, cycle_identity COLLATE BINARY ASC`. Missing,
empty or ASCII-whitespace-only ordering keys are an exact typed integrity
failure. If a row exists, begin queries and requires zero `retry_cycles` rows
for the proposed identity and zero proposed `Started` outbox rows inside that
same transaction, constructs the proof described below, performs no write and
rolls back. It returns definite `RetryCycleAlreadyRunning` with that
`NoRetryCycleCommitted`. Otherwise the same short transaction persists the
exact ordinal/identity, validated boot identity and input timestamps in the
`Running` row together with its logical-slot-unique `Started` outbox. It has no
fallback that reads global runtime state, the environment or a process helper.
`NoRetryCycleCommitted` is an opaque, non-cloneable and non-serializable
coordinator-issued proof. Its private canonical tuple has exact field order
`schema_version=1,rule_id="BR-192",namespace_sha256,owner_boot_identity,
scheduled_for,started_at,proposed_cycle_ordinal,proposed_cycle_identity,
selected_running_cycle_ordinal,selected_running_cycle_identity,
selected_running_namespace_sha256,
selected_running_owner_boot_identity,selected_running_scheduled_for,
selected_running_started_at,selected_running_terminal_phase,
selected_running_state="Running",
selected_running_row_sha256,
proposed_cycle_row_count=0,proposed_started_row_count=0`. It uses the same
timestamp/compact-JSON rules and the exact domain
`stock_analysis.durable_delivery.br192.no_retry_cycle_committed.v1\0`; its
private lowercase SHA-256 is always recomputed, never caller supplied.
`selected_running_row_sha256` binds the complete selected retained row. Its
declared-field compact-JSON preimage order is exactly
`schema_version=1,rule_id="BR-192",cycle_identity,cycle_ordinal,
namespace_sha256,owner_boot_identity,scheduled_for,started_at,state,
terminal_phase,candidate_query_calls,queried_candidate_count,
sorted_candidate_sha256,provider_calls,renderer_calls,sink_calls_state,
sink_calls_count,failure_reason,failure_payload_identity,
failure_typed_fields_sha256,failure_envelope_sha256,completed_at`; nullable
fields are JSON `null`, integer/count fields are JSON integers, timestamps use
the same UTC-nanosecond encoding, and text retains validated bytes. Its exact
domain is
`stock_analysis.durable_delivery.br192.retry_cycle_running_witness.v1\0`;
the value is lowercase `SHA-256(domain || canonical_preimage_bytes)`.

`consume_no_retry_cycle_committed` consumes the proof and opens a fresh
`BEGIN IMMEDIATE`. It validates the private canonical bytes/hash, recomputes
`MAX(cycle_ordinal)+1`, rederives the proposed identity from the exact original
input tuple, requires the next ordinal/identity to match, reselects the same
retained Running witness byte-for-byte, and again requires zero proposed
cycle/`Started` rows. It completes read-only. Any concurrent insert, ordinal
change, selected-row terminal/state/field change, identity mismatch or
non-zero proposed-row count rejects proof consumption; the caller cannot
release the guard and `Drop` leaves it latched. The guard may be cleared by
`release_after_verified_no_cycle(proof)` only after this exact consumption. A
commit-ambiguous begin error carries no proof and leaves the guard latched.
Once the cycle transaction commits, no pre-cycle release path exists.
The async parent retains the cycle identity independently of the join handle.
No long-lived closure starts and no sink is reachable until the durable identity
exists. There is no second `begin_retry_cycle` alias.

`RetryCycleGuard` owns the running flag for the **whole** blocking owner
closure. It remains outside the inner `catch_unwind` scope, so a panic while
executing or returning from the sink boundary cannot drop/release it before
finalization. The same owner closure performs retry work, any sink execution,
completion prepare/append/ack plus terminal CAS, or failure quarantine plus
failure-payload/outbox prepare/append/ack plus terminal CAS. Only after the
coordinator proves either a terminal `Completed|Failed` row, a durable
`Running/CompletionPending|CompletionAppended` row with its complete immutable
completion payload and exact Pending/Appended outbox, or a durable
`Running/FailurePending|FailureAppended` row with its complete immutable failure
payload and exact Pending/Appended outbox may
`RetryCycleGuard::release_after_verified_safe_state` clear the running flag.
Release at a Pending/Appended safety point does not authorize another cycle by
itself: the next guard owner must first run the same-boot safe-terminal resumer,
and the transactional global Running-row exclusion remains the final admission
authority.
`Drop` without that proof leaves the flag latched `true`, emits a fatal
same-boot isolation error and forbids a second cycle until process restart; it
never silently clears the mutex.

Normal success prepares, appends and acknowledges exact `Completed` bytes and
performs its terminal CAS before guard release. A normal error, caught panic or
start-audit-unavailable failure may use the failure finalizer only while the
coordinator proves the terminal phase is `NotPrepared`. Once
`CompletionPending` or `CompletionAppended`
exists, any append, acknowledgement, terminal-CAS, returned-error or caught-
panic path must resume those exact stored completion bytes through
`Completed/Terminalized`; it cannot create a `CycleOperationFailed` payload or
a `Failed` outbox. Before the failure finalizer may prepare, append or
acknowledge `Failed`, it transactionally classifies whether the narrow
start-audit-unavailable exception is admissible. That exception requires no
acknowledged start and no consumed ownership and preserves the exact
decision/attempt/binding/schedule rows, canonical hashes and pending start
bytes. Otherwise the unchanged-state exception is rejected and the ordinary
path transactionally selects every nonterminal attempt owned by the same
`execution_cycle_identity` that has any appended/acknowledged
`SinkAttemptStarted`, consumed/`Started` send ownership, or
`AttemptInFlight`, and no authoritative terminal sink result. It persists
`ProcessInterruptedAfterSinkStart`, creates or advances retained ownership to
`InterruptedUncertain`, creates the exact three uncertainty outboxes and
append/acknowledges them to a fixed point with zero sink calls. Only then may
it prepare, append and acknowledge `Failed` for the same cycle, followed by
the independent terminal CAS, before the closure verifies the guard-release
postcondition and returns. The boundary classification is derived from
persisted state, never a runtime boolean, and acknowledged-start or consumed
ownership always dominates the reason code.
Before that selection inspects the compatibility base attempt state, it must
exclude every exact effective `ExpiredFreshnessBeforeSink` relation from §4.
If its expiry is only Pending/Appended but not yet terminalized, the higher-
priority expiry resumer completes Transaction B first; it is never quarantine
input. A partial or mismatched purported relation fails closed instead of
falling through to uncertainty.
If uncertainty append/reconciliation fails, the exact pending outbox remains
and the cycle is not falsely finalized as `Failed`; recovery resumes this
ordering. Because that state has not reached `FailurePending`, the unsafe Drop
path keeps the same-boot running flag latched rather than opening a second
cycle.
Cancelling the async waiter does not release the guard while blocking work
continues.

The coordinator boundary is explicit:

```rust
pub fn quarantine_same_cycle_attempts_before_failure(
    &self,
    cycle_identity: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>>;
```

It has no sink capability. The library-owned
`prepare_retry_cycle_failed(cycle_identity, &RetryCycleFailure, now)` privately
recomputes and persists the reason-specific canonical typed preimage, complete
failure envelope and both hashes, freezes their identity/hashes in the
canonical Pending `Failed` outbox and moves only to
`Running/FailurePending`. It rechecks that the single terminal phase is exactly
`NotPrepared`, that no completion/failure terminal slot exists, and
that this same-cycle selector is empty and every created uncertainty outbox is
acknowledged. After exact append/ack,
`terminalize_retry_cycle_failed(cycle_identity, now)` loads those complete
bytes from SQLite, privately decodes and canonically reserializes them, repeats
both hash validations and performs the sole terminal CAS. Completed cycles use
the separate `prepare_retry_cycle_completed`,
`terminalize_retry_cycle_completed` and
`resume_retry_cycle_terminal_slot` methods and cannot carry failure evidence.
The resume method selects the already committed terminal kind and exact stored
bytes; it cannot choose a new kind. This makes the ordering and typed failure
mapping database authority invariants rather than runtime prose.

If awaiting the blocking task returns `JoinError`, the async parent uses its
retained cycle identity and a separately constructed
`RetryCycleRecoveryCapabilities` containing only coordinator and append
authority to invoke idempotent orphan recovery. The parent never clones or
projects its sink-bearing execution capabilities into recovery. Recovery first
loads the terminal phase. `CompletionPending|CompletionAppended` resumes the
exact stored completion append/ack/CAS and returns; it never enters failure
quarantine. `FailurePending|FailureAppended` resumes the exact stored failure
append/ack/CAS and returns. Only `NotPrepared` quarantines every
nonterminal attempt with appended start or consumed send ownership as
`ProcessInterruptedAfterSinkStart` uncertainty and makes zero sink calls.
Phase one appends and acknowledges every uncertainty/state/task outbox to a
verified fixed point. Only after that fixed point may phase two persist the
complete typed `JoinError` failure payload, insert stable `OrphanRecovered`
plus pending `Failed`, append/acknowledge those exact bytes, and then perform
`Running/FailureAppended -> Failed/Terminalized`. If that recovery itself
cannot finish phase one, no failure payload/Failed outbox is prepared. If it
cannot finish phase two, it leaves `Running/FailurePending` or
`Running/FailureAppended` plus the exact stored payload/outbox for startup
recovery and returns an explicit error.

Each monitor boot holds the provider-free runner's exclusive process-lifetime
lock through its retained boot authority and records that exact identity on
every cycle. After acquiring that lock and before coordinator/append open, the
new runtime fixes the current boot identity. Startup recovery receives that
identity explicitly: a `Running` row with the same boot identity is never
orphan-converged. For a different prior-boot identity, recovery first validates
and resumes `CompletionAppended`, then `CompletionPending`, then
`FailureAppended`, then `FailurePending`, using only their exact SQLite bytes
and making zero sink calls. Only `NotPrepared` may quarantine any appended-
start/`AttemptInFlight` attempt lacking a terminal authoritative result,
append/acknowledge every uncertainty outbox to a fixed point, and then persist a
complete typed `ProcessInterrupted` failure payload and prepare/append/ack/
terminalize the cycle before a new cycle. Existing terminal slots load the
full canonical typed preimage/envelope bytes from SQLite, recompute all hashes
and resume their exact prior kind/reason/bytes without an in-memory DTO. Thus a real
process death converges on the next startup without misclassifying same-boot
work or resending an indeterminate external call. No recovery helper may
independently derive or read a current boot identity.

Same-boot safe-terminal recovery is separate from orphan recovery. It matches
only the explicit current boot identity and only the four already-chosen
terminal phases; it never touches same-boot `NotPrepared`, never reclassifies
an attempt and never prepares a new terminal kind. A wrong/current-identity
mismatch returns no row, after which the begin transaction still observes the
unresolved global Running row and rejects the new cycle before `Started`.
Prior-boot recovery retains its existing `owner_boot_identity<>current`
predicate and must converge before the same-boot resumer/begin sequence.

```text
recover_prior_boot_running_cycles(
  current_owner_boot_identity,
  now
)
  -> quarantined_cycle_identities

prepare_prior_boot_retry_cycle_failed(
  quarantined_cycle_identity,
  now
)
  -> failure_payload_identity
```

This coordinator boundary validates the explicit current identity before its
transaction and limits its CAS predicate to
`state='Running' AND owner_boot_identity<>current_owner_boot_identity`.
The first method may create only uncertainty/state/task outboxes and quarantine
ownership; it cannot create a failure payload, `OrphanRecovered` or `Failed`.
It is callable only for `NotPrepared`; a recoverable completion or failure
phase is routed to `resume_retry_cycle_terminal_slot` before this method.
The runtime appends and acknowledges every returned cycle's phase-one outboxes
to a fixed point, then calls the second method. That method transactionally
rechecks the fixed point, persists the canonical `ProcessInterrupted` payload
and Pending `OrphanRecovered`/`Failed` slots. Runtime then
append/acknowledges phase two, performs the terminal CAS, and only then starts
the startup cycle. A phase-one append/ack failure makes zero phase-two writes.

The periodic task starts with:

```text
interval_at(Instant::now() + 30s, 30s)
```

and uses `MissedTickBehavior::Skip`. Therefore startup completion is not
followed by an immediate periodic retry. The task participates in the existing
shutdown `select!`; shutdown stops new cycles but cannot pretend a running
blocking cycle was cancelled. The blocking cycle finishes, records its terminal
audit and releases the guard.

The scheduler may wake later than `next_eligible_at`; admission always re-reads
the persisted schedule and policy. Logs never alter eligibility.

## 7. Cross-process fencing

The unit-test binary provides an ignored child test. The parent creates one
nonce-bound `Fixture` root, prepares one authorized rejection and launches two
copies of the current test executable with:

- the same TEST_CODE database path;
- the same test namespace;
- separate coordinator owners;
- a file-based ready/gate barrier inside the TEST_CODE root; and
- a process-safe sink counter file inside the TEST_CODE root protected by
  `fs2::FileExt::lock_exclusive`.

Both children race `admit_authorized_retry` and the four-stage
prepare/append/claim/execute sink seam. Assertions:

- exactly one next reservation generation exists;
- admission committed exactly one authoritative attempt, immutable attempt
  binding and schedule increment before either child can prepare;
- exactly one authoritative attempt owns the fence;
- exactly one `Reserved -> AttemptInFlight` CAS inserts retained
  `send_consumed=true` ownership and yields one non-clone permit;
- the locked counter contains exactly one sink-call record;
- the loser returns a typed state/generation loss with zero sink calls; and
- both child processes exit successfully.

The parent owns cleanup through the existing `Fixture`/`OwnedTestPaths`; child
tests never delete shared paths.

## 8. Provider and renderer exclusion proof

All retry-cycle functions and helpers live in one brace-delimited
`provider_free_retry` module. Sink-capable execution receives a narrow
`RetryCycleCapabilities` projection containing only
`Arc<DurableDeliveryCoordinator>`, `Arc<dyn ImmutableAppendPort>`, the existing
`AuthoritativeSink` (`Arc<dyn AuthoritativeSinkPort>`), namespace/boot hashes
and no running flag or guard. Only the outer `RetryCycleGuard` may own or
mutate the raw `Arc<AtomicBool>` after acquisition; neither
`RetryCycleCapabilities` nor `retry_cycle_blocking` can access it. The
capability projection never receives the full `RuntimeState`, a callback, an
unbounded generic capability or `super::*`.

Recovery is a separate boundary:

```rust
#[derive(Clone)]
struct RetryCycleRecoveryCapabilities {
    coordinator: Arc<DurableDeliveryCoordinator>,
    append: Arc<dyn ImmutableAppendPort>,
}
```

JoinError, orphan, startup and cancellation-recovery helpers accept only
`&RetryCycleRecoveryCapabilities`; this type contains no sink, provider,
renderer, runtime state, namespace adapter or callback. The async parent builds
it directly from the coordinator and append fields before spawn. It must not
obtain it by cloning `RetryCycleCapabilities`, and the sink-bearing projection
is not `Clone` merely to support recovery.

The checked-in static cutover test extracts and scans the complete module
slice, including imports, both capability structs and every helper body. Its
production dependency allowlist is the coordinator/append/sink types above,
BR-192 retry model types, chrono/Tokio and standard-library synchronization/
collections/hash primitives. A token/path-aware scan rejects provider/gateway/
producer/renderer/market-data dependency identifiers, `push_templates`,
`render_*`, `envelope_from_binding`, `deliver_counted_binding`, provider
constructors and calls to helpers outside the allowlisted module graph; it does
not confuse audited field names such as `provider_calls` with a provider
capability. A compile-time signature assertion proves the
public entry accepts only `RetryCycleCapabilities`; a source-graph assertion
proves every local call resolves inside the scanned module or to the explicit
allowlist. The same assertion freezes the execution capability's exact fields
and rejects any `AtomicBool`, raw running flag or guard field/reference in
`RetryCycleCapabilities` or `retry_cycle_blocking`. Scanning only
`retry_cycle_blocking` is insufficient.

The runtime test constructs the original rejected envelope through a
`CountingProvider` and `CountingRenderer` defined in the runtime test module.
Those helpers increment atomics at the real test provider-load and render
boundaries. After freezing the initial rejection, the test resets both
counters, runs startup and periodic retry cycles, and asserts:

```text
provider delta = 0
renderer delta = 0
sink delta = 1 for the winning cycle
```

The returned evidence is populated from these measured deltas in tests, not
from hard-coded zero literals. Production evidence reports zero because the
cycle has no such capabilities; the static dependency test enforces that
architecture.

## 9. Authenticated manual command and TEST_CODE injection

The CLI is a syntax-only production adapter. Its sole library call is:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionRetryAuthorizationRequest {
    pub decision_identity: String,
    pub operator_identity: String,
    pub reason: String,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProductionRetryAuthorizationOutcome {
    Authorized {
        authorization_identity: String,
        authorization_event_identity: String,
        namespace_sha256: String,
        evidence_sha256: String,
        immutable_ref: String,
        authorized_at: DateTime<Utc>,
    },
    NoLongerEligible {
        decision_identity: String,
        ineligibility: RetryIneligibility,
    },
}

pub fn authorize_delivery_retry_production(
    request: ProductionRetryAuthorizationRequest,
) -> Result<ProductionRetryAuthorizationOutcome>;
```

There is no request timestamp, target/root, authority, resolver, coordinator,
append port, evidence bytes or test selector. This library-owned entry
validates syntax, obtains a real opaque PAM attestation from the library-owned
authentication module, constructs the private fixed manifest-root resolver
internally, reads evidence only after authentication, applies the authorization
and returns redacted persisted identities/hashes/ref. The binary and retry
command cannot construct or inject any part of that authority graph.

Before inserting manual authority, the entry validates the persisted producer
companion against the current library catalog and asks its
`DurableDeliveryCoordinator` to drain persisted expiry outboxes to a fixed
point. The coordinator's owning `BEGIN IMMEDIATE` then rechecks exact current
disposition/companion/schedule and obtains a fresh
`freshness_observed_at` from its private `ProductionFreshnessClock` immediately
before the freshness-dependent write, after all blocking I/O. Freshness requires
`freshness_observed_at < expires_at`; PAM `validated_at` remains only
`authorized_at` and can never satisfy this gate. Equality or later atomically
creates the zero-attempt schedule when absent, prepares only the stable Pending
expiry outbox, commits it, append/acknowledges the exact bytes and terminalizes
freshness before returning typed `NoLongerEligible::ExpiredFreshness`; no
authorization row is inserted. A crash resumes that persisted outbox before a
later command can apply authority.

The coordinator is the sole owner and reader of the production freshness
clock. `retry_command.rs`, the binary and the public authorization wrapper own
no clock and cannot sample, construct, inject or forward a freshness timestamp;
after blocking authentication/evidence work they invoke the coordinator's
freshness-bearing method. The production clock is private and non-injectable
from request/CLI/env/root exports. Only a cfg(test)-bound TEST_CODE coordinator
clock exists for equality and midnight race tests. No provider, PAM, evidence
or filesystem I/O occurs between the coordinator's final freshness observation
and its transaction write; the rejection event persists that exact
observation.

Task 6 extends `src/auth/operator.rs` with the only production attestation
factory:

Current source fact at Gate A is reproducible only from fixed baseline
`b4aeee68d2c0259cc968914b3d39e3a89a18a496`, whose
`src/auth/operator.rs` blob is
`b0ec1f0b218466493dabb0a6e560099d07e19cf2`:

```text
$ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/auth/operator.rs | rg -n 'pub struct OperatorAuthConfig|pub fn load_auth_config|pub fn require_monitor_operator_auth|fn try_pam_auth|pam::Authenticator::with_password|auth\.authenticate\(\)'
26:pub struct OperatorAuthConfig {
51:pub fn load_auth_config() -> OperatorAuthConfig {
78:pub fn require_monitor_operator_auth() -> Result<(), OperatorAuthError> {
137:fn try_pam_auth(cfg: &OperatorAuthConfig, password: &str) -> Result<(), pam::PamError> {
140:    let mut auth = pam::Authenticator::with_password(&cfg.pam_service)?;
143:    auth.authenticate()
148:fn try_pam_auth(_cfg: &OperatorAuthConfig, _password: &str) -> Result<(), pam::PamError> {
```

Therefore the existing API authenticates but discards success evidence. Task 6
deepens this real PAM boundary rather than creating a second authentication
source:

```rust
#[must_use]
pub struct OperatorAuthAttestation {
    subject: String,
    pam_service: String,
    authentication_mechanism: &'static str,
    session_nonce: zeroize::Zeroizing<[u8; 32]>,
    validated_at: DateTime<Utc>,
}

pub fn authenticate_monitor_operator(
    claimed_operator: &str,
) -> Result<OperatorAuthAttestation, OperatorAuthError>;
```

The type is public only as an opaque return handle: all fields, constructors
and consuming accessors are private or `pub(crate)`; it implements neither
`Clone`, `Default`, `Serialize` nor `Deserialize`. After validating
`MONITOR_AUTH_REQUIRED=1`, configured-subject equality and TTY, this function
performs the real `try_pam_auth`. Only after PAM returns success does the auth
module capture `validated_at = Utc::now()`, read a fresh 32-byte nonce from the
OS CSPRNG (`/dev/urandom` on the Unix PAM build), and construct the attestation
with the exact subject, configured PAM service and fixed mechanism
`"pam-password-v1"`. PAM failure, disabled auth, subject mismatch, non-TTY or
nonce failure returns an error and no attestation. Existing
`require_monitor_operator_auth()` preserves monitor compatibility by calling
this strict function when auth is required and discarding the attestation; it
does not synthesize a second success path.

The authentication result is a module-private canonical DTO:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticatedRetryOperator {
    principal_identity_sha256: String,
    authority_kind: RetryCommandAuthorityKind,
    validated_at: DateTime<Utc>,
    session_evidence_canonical: Vec<u8>,
    session_evidence_sha256: String,
}
```

The two SHA fields are lowercase 64-hex hashes; the retained canonical session
evidence contains only hashed principals and authentication metadata—no
credential, plaintext operator, PAM response, holding or account data. The
session-evidence hash covers domain-separated
canonical `RetryOperatorSessionEvidenceV1` fields in this order:
`schema_version=1`, `rule_id="BR-192"`, `authority_kind`,
`auth_required`, `expected_principal_sha256`, `claimed_principal_sha256`,
`stdin_is_tty`, `stdout_is_tty`, `pam_service`, `authentication_mechanism`,
`session_nonce_sha256`, `validated_at`. Its digest is lowercase hex
`SHA-256(b"stock_analysis.durable_delivery.br192.retry_operator_session.v1\0"
|| canonical_utf8_bytes)`. `validated_at` is the authentication authority's
time and is not copied from the command. Expected/claimed principal hashes,
PAM service/mechanism, nonce hash and `validated_at` are derived only by
consuming the opaque `OperatorAuthAttestation`; the retry-command module cannot
replace any of them. The module-private
`validate_authenticated_retry_operator(authenticated, expected_kind)` parses
`session_evidence_canonical` with `deny_unknown_fields`, recomputes the session
evidence hash, validates both principal hashes/timestamp and requires both
DTO/evidence authority kinds to equal the kind already checked against the
target. The DTO exists only from successful authentication through completion
of that single command; only its hashes/kind/timestamp may be copied into
canonical authorization evidence. The persisted authorization's
`authorized_at`, canonical authorization observation time and manual schedule
eligibility input all equal `AuthenticatedRetryOperator.validated_at`
byte-for-byte. No process `Utc::now()`, request field, filesystem timestamp or
append timestamp may substitute for or adjust it, including when execution is
delayed after authentication.

`ProductionRetryCommandTargetResolver`, `RetryCommandAuthorityKind`,
`RetryCommandTarget`, `AuthenticatedRetryOperator`,
`ResolvedRetryCommandTarget` and their constructors/validator are all
module-private; `RetryCommandTarget::Test` is compiled only under `#[cfg(test)]`
and the non-test target is Production-only. The production entry first
validates syntax and fixed Production target intent, calls
`authenticate_monitor_operator`, then consumes the opaque attestation exactly
once to create and validate `AuthenticatedRetryOperator`. The retry-command
module has no function accepting a subject, time, service, mechanism or nonce
separately and cannot construct an attestation or operator from raw values.
Only then may it construct the private fixed-root resolver, create the resolved
target, open coordinator and immutable append authority and read the evidence
path.

`ResolvedRetryCommandTarget` is a private capability aggregate with:
the validated target, fixed resolved database root, fixed immutable-append
root, fixed evidence path, opened coordinator, opened append authority, exact
evidence bytes and their canonical SHA-256. It has no public constructor and
does not implement `Serialize`, `Deserialize` or `Clone`; serialization applies
to the canonical DTOs, never live capabilities. Its module-private constructor
is reachable only through:

```rust
struct ResolvedRetryCommandTarget {
    target: RetryCommandTarget,
    authenticated_operator: AuthenticatedRetryOperator,
    resolved_database_root: PathBuf,
    resolved_immutable_append_root: PathBuf,
    resolved_evidence_path: PathBuf,
    namespace_preimage: RetryNamespaceHashPreimageV1,
    namespace_sha256: String,
    coordinator: Arc<DurableDeliveryCoordinator>,
    append: Arc<dyn ImmutableAppendPort>,
    evidence_bytes: Vec<u8>,
    evidence_sha256: String,
}
```

```rust
fn resolve_after_authorization(
    &self,
    target: &RetryCommandTarget,
    authenticated: &AuthenticatedRetryOperator,
    evidence_path: &Path,
) -> Result<ResolvedRetryCommandTarget>;
```

The resolver rechecks target/authority compatibility and fixed-root
confinement, constructs the exact `RetryNamespaceHashPreimageV1` from that
fixed target and calls the sole `retry_namespace_sha256` helper before opening
or reading anything. The resulting private target retains both the preimage
and digest, and authorization canonical bytes must include both; coordinator
application recomputes the digest and rejects any target/canonical/preimage/
digest disagreement. The production resolver resolves only fixed manifest-root
production authorities. A production target can never be constructed with
`Test` authority, a TEST_CODE root or test capability. Resolved capabilities
live only for one command execution and are dropped after append/apply
reconciliation. The public function cannot accept pre-opened coordinator,
append or evidence capabilities.

Production uses `authenticate_monitor_operator`, which:

1. requires `MONITOR_AUTH_REQUIRED=1`;
2. loads the fixed authentication configuration and rejects a claimed operator
   that differs from `load_auth_config().expected_operator`;
3. requires stdin and stdout TTYs;
4. invokes the real PAM password authentication path; and
5. only after success creates the opaque attestation with authority-owned time,
   subject, PAM service/mechanism and fresh OS nonce.

The auth module owns a `#[cfg(test)] pub(crate)` TEST_CODE attestation factory;
it is the only non-PAM constructor and cannot compile into production.
Injectable target-resolver support and `RetryCommandTarget::Test` likewise
exist only inside `#[cfg(test)]` modules. They are absent from non-test builds
and from the durable-delivery root. Test injection succeeds only with the
identical nonce-bound TEST_CODE subject/target/root. Production tests exercise
the public production entry; TEST_CODE tests exercise only these cfg(test)
seams. No feature, environment variable, CLI flag or public constructor
enables them in a production build.

Authentication and target validation occur before resolving or opening any
coordinator, append authority or evidence file. Empty reason/evidence, operator mismatch,
authentication disabled, non-TTY, uncertain state, terminal state, stale
disposition, conflicting bytes and exhausted retry schedule all fail without
mutation.

Production-binary process tests cover authentication unset, authentication set
to `0`, non-TTY and operator mismatch. Each supplies an evidence path that does
not exist. Operator mismatch is checked before TTY/PAM, so that negative path is
deterministic and never prompts. The tests compare metadata-only
existence/type/filesystem identity snapshots for every protected production
artifact before and after; they do not open or hash production content.

The authorization binary also owns a structured
`br192_authorization_cli_request_surface_is_timestamp_and_capability_free`
unit test. `clap::CommandFactory` must report exactly
`decision,operator,reason,evidence-file`; the exact request literal used by
`run` is serialized and its object keys must be exactly
`decision_identity,operator_identity,reason,evidence_path`. This compile-time
literal plus structured key comparison rejects a request timestamp or injected
capability without banning the legitimate
`ProductionRetryAuthorizationOutcome::Authorized { authorized_at, .. }`
output field.

Tests prove the Production command and cfg(test)-only nonce-bound application
produce distinct namespace preimages/hashes, any test-code/root/preimage/hash
tampering fails before open or mutation, and persisted manual authorization
evidence exactly retains the validated resolved namespace pair. They also prove
the public root contains no constructible/injectable authority, resolver,
target, session or resolved capability—the exported attestation is opaque and
originates only from the real auth function—the CLI source constructs only
`ProductionRetryAuthorizationRequest` and calls
`authorize_delivery_retry_production`, and a delayed command cannot change the
authority-owned authorization time.

Owning tasks define their types/constants first and use private module paths
internally. Task 1 privately owns the catalog/permit definitions and performs
no root export; Task 8, the final integration task, performs the sole atomic
root-export edit for BR-192. Existing unrelated `durable_delivery` root exports remain
unchanged; no current symbol is deleted or duplicated. The exact ordered
BR-192 cross-module contract that must be present after that edit is:

```text
DurableDeliveryCoordinator
ImmutableAppendPort
AuthoritativeSink
AuthoritativeSinkPort
DecisionState
CountedProducerPermit
CountedProducerAttestation
CountedProducerDenied
acquire_counted_producer_permit
MAX_AUTOMATIC_RETRY_ATTEMPTS
RETRY_BACKOFF_SECONDS
RetryAuthorizationSource
RetryCandidate
ExpirableRetrySchedule
CompleteRetryExpirySnapshot
PreparedRetryExpiredFreshness
PreparedRetryExpiryUncertainty
RetryExpiryUncertaintyReason
RetryExpiryPreparationOutcome
RetryExpiryDisposition
RetryExpiryTerminalKind
RetryScheduleTerminalState
RetryCandidateSnapshot
RetryCycleEvidence
RetryCycleSinkCalls
RetryCycleFailureReason
RetryCycleOperation
RetryCycleFailure
RetryCycleTerminalPhase
RetryCycleBeginOutcome
NoRetryCycleCommitted
RetryNamespaceKind
RetryNamespaceHashPreimageV1
retry_namespace_sha256
RetryDeferral
RetryIneligibility
RetryAdmission
PreparedRetryAttempt
RetryAttemptPreparationOutcome
AppendedSinkAttemptStarted
RetrySendOwnershipState
SinkExecutionPermit
RetrySinkClaimOutcome
PersistedRetrySinkOutcome
RetrySinkExecutionOutcome
OperatorAuthAttestation
authenticate_monitor_operator
ProductionRetryAuthorizationRequest
ProductionRetryAuthorizationOutcome
authorize_delivery_retry_production
pub const MAX_RETRY_EVIDENCE_RESULTS: usize = 256
RetryEvidencePushKind
RetryEvidenceQuery
VerifiedRetryEvidence
verify_br192_retry_evidence
```

The coordinator operations are methods and therefore are not re-exported as
free functions. Runtime/CLI code must not import private module paths. A
compile-contract test imports this exact manifest only through
`stock_analysis::durable_delivery`, then type-checks method calls for the exact
prepare/reconcile/validate/claim/execute signatures through the imported
`DurableDeliveryCoordinator`. No earlier task may publish a partial set of new
BR-192 symbols.

A multiline-aware machine gate scans each complete `pub use ...;` token range
and the `pub mod` form for private command/evidence authority symbols. It must
return zero matches even when rustfmt splits an export across lines; a
single-line `pub use .*` grep is not acceptable leakage evidence.

## 10. Test isolation

Reuse the existing `Fixture`, `MemoryAppendPort`, `StaticSink`,
`prepare_reserved`, `reconcile_terminal`, `envelope`, `rejection` and
`uncertainty` helpers in `src/durable_delivery/tests.rs`.

New helpers are defined only in these locations:

- `RetryTestBuilder`, `CrossProcessRetryHarness` and `ProcessCountingSink` in
  `src/durable_delivery/tests.rs`;
- `CountingProvider`, `CountingRenderer` and `RuntimeRetryFixture` in
  `src/bin/monitor/durable_delivery_runtime.rs`'s test module; and
- the cfg(test)-only TEST_CODE attestation factory in
  `src/auth/operator.rs` plus the recording retry-command resolver and
  `TestRetryEvidenceTarget` in their owning library test modules, all with at
  most `pub(crate)` visibility.

`RetryTestBuilder` composes the existing helpers; it does not replace them.
Every new helper stores all files below the parent Fixture's nonce-bound
`data/test/TEST_CODE_*` root. Parent Drop cleanup remains authoritative.

Extend `ProductionStorageSnapshot` to capture before/after existence, type and
filesystem identity for:

- production durable-delivery DB, journal, WAL and SHM;
- provider-free retry runner process-lock path;
- production push-log root;
- durable-delivery immutable append/audit root;
- event-audit chain root; and
- observation event-bus root.

Authoritative receipts are retained inside the durable-delivery SQLite and
immutable append authorities; there is no invented standalone receipt path.

Tests refuse to start if a protected production file that would need reading
already exists. They use metadata/retained directory handles only and never
open or hash protected production content.

### 10.1 Checked-in production evidence verifier

Gate D uses a checked-in read-only `verify_br192_retry_evidence` binary, not
shell text matching or ad-hoc SQLite. It resolves fixed manifest-root production
authorities and:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RetryEvidencePushKind {
    ReviewProviderTopN,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryEvidenceQuery {
    pub business_date: NaiveDate,
    pub push_kind: RetryEvidencePushKind,
    pub require_count: usize,
}

pub const MAX_RETRY_EVIDENCE_RESULTS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedRetryEvidence {
    pub durable_push_kind: RetryEvidencePushKind,
    pub verified_retry_count: usize,
    pub exact_join: bool,
    pub decision_identity_sha256: String,
    pub attempt_identity_sha256: String,
    pub sink_result_identity_sha256: String,
    pub decision_state: DecisionState,
    pub ownership_state: RetrySendOwnershipState,
}

pub fn verify_br192_retry_evidence(
    query: &RetryEvidenceQuery,
) -> Result<Vec<VerifiedRetryEvidence>>;
```

The public function always resolves only fixed manifest-root production
read-only authorities. There is no public target, root or test selector. The
only injected seam is defined inside the owning library module:

```rust
#[cfg(test)]
pub(crate) struct TestRetryEvidenceTarget {
    pub(crate) test_code: String,
    pub(crate) root: PathBuf,
}

#[cfg(test)]
pub(crate) fn verify_br192_retry_evidence_test(
    target: &TestRetryEvidenceTarget,
    query: &RetryEvidenceQuery,
) -> Result<Vec<VerifiedRetryEvidence>>;
```

The test-only target has no non-test constructor or export and must reject
production roots, real-symbol authority and a nonmatching `TEST_CODE_*` nonce
before open. Its positive fixture is a library unit test, not an external
integration test. `require_count` is accepted only in
`1..=MAX_RETRY_EVIDENCE_RESULTS`; zero or 257 and above return typed
`DurableDeliveryError::RetryEvidenceQueryCountOutOfRange {
requested, min: 1, max: 256 }` before resolving or opening any authority.
The CLI applies the same closed parser range and the library repeats the check.
Returned rows are persisted exact joins; each row's `durable_push_kind` must
equal the requested enum, `verified_retry_count` must equal the final validated
result-vector length and be at least `require_count`, and `exact_join` is
emitted only as literal `true` after the full join validation succeeds. A
partially validated row is never serialized. Results never contain rendered
text, raw account data, filesystem paths or credentials. The production CLI
serializes these exact
`VerifiedRetryEvidence` objects directly and has no second summary/wrapper
schema, so Gate D's required
`durable_push_kind`/`verified_retry_count`/`exact_join` fields cannot diverge
from the library contract.

The normative production authorities are exact and are not caller-selectable:

```text
counted pending/commit artifacts:
  data/push_log/YYYY-MM-DD/<artifact>_audit_pending.json
  data/push_log/YYYY-MM-DD/<artifact>_committed.json
counted delivery observation envelope:
  data/event_bus/YYYY-MM-DD.jsonl
  event_type = "push.delivery.audit"
retry-cycle observation envelope:
  data/event_bus/YYYY-MM-DD.jsonl
  event_type = "delivery.retry.cycle"
```

The counted Markdown pre-send log may coexist at
`data/push_log/YYYY-MM-DD/HHMMSS_<unique>.md`, but it is not a receipt. The
authoritative receipt is the typed remote receipt joined through SQLite,
pending/commit artifacts, immutable append refs and the exact
`push.delivery.audit` envelope. There is no standalone receipt file or fallback
event type. All production paths are below the compile-time manifest root;
TEST_CODE mirrors them only below its invocation-unique isolated root.

The verifier streams artifact candidates rather than collecting an unbounded
filesystem/path/result vector. It validates each complete join and inserts it
into a bounded map keyed by the typed
`RetryEvidenceLogicalTuplePreimageV1` above after exact validation and hash
recomputation. Replaying an artifact whose complete canonical bytes and hashes
are byte-identical to the already-validated entry is accepted, deduplicated
and does not increase the verified count; it is not a duplicate-match error.
Reusing the same logical tuple with any different canonical bytes or hash is a
conflicting duplicate and returns typed
`DurableDeliveryError::RetryEvidenceConflictingDuplicate {
logical_tuple_sha256, canonical_bytes_mismatch,
canonical_hash_mismatch }` with the exact conflict flags frozen above. Any
duplicate authority row inside one join remains an ambiguous-match error.
Attempting to insert the 257th distinct complete join returns typed
`DurableDeliveryError::RetryEvidenceResultBoundExceeded {
max: 256, attempted_distinct_count: 257 }`, emits no partial JSON and performs
no write. Once the input stream is exhausted, it
sorts the at-most-256 final rows with SQLite binary semantics by the persisted
non-null, non-empty keys
`decision_identity ASC, retry_ordinal ASC, attempt_identity ASC,
sink_result_identity ASC, authorization_identity ASC,
rejection_disposition_identity ASC`. The last identity is the total tie-break;
filesystem enumeration, JSONL discovery order, SQL planner order and hash-map
iteration are never observable. Only after that sort does it assign the same
final vector length to every `verified_retry_count` and serialize the vector.

1. parses counted `.json` push-log pending/commit artifacts with
   `serde(deny_unknown_fields)`;
2. filters the exact JSON field
   `durable_push_kind == "ReviewProviderTopN"`;
3. validates the pending artifact hash, exact commit marker and exact
   `data/event_bus/YYYY-MM-DD.jsonl` event with
   `event_type="push.delivery.audit"`;
4. opens durable SQLite read-only and joins the terminal decision to the
   immutable retry-attempt binding, the historical rejection disposition,
   appended/applied authorization and its immutable transition event, binding
   generation, schedule `last_attempt_binding_identity` plus exact retry
   ordinal, reservation generation/fence owner, authoritative sink result and
   immutable state/cycle audit refs; and
5. emits only redacted identities/hashes/counts as JSON.

It exits non-zero for zero matches, a conflicting duplicate, an ambiguous
match (including duplicate authority rows inside one join), missing or
ASCII-whitespace-only refs, hash mismatch, non-terminal state, any
write-capable open or any join disagreement. A byte-identical artifact replay
instead remains a successful single deduplicated result. The cfg(test)-only
TEST_CODE unit fixture proves both cases plus the parser and joins; the
production CLI and public library surface expose no path override. The verifier
is the authoritative Gate D command.

Binary wiring and exact-join correctness are separate contracts. The binary
test
`br192_verify_evidence_cli_has_exact_production_arguments_and_library_call`
validates the structured Clap arguments and direct
`verify_br192_retry_evidence(&query)` call. The library test
`br192_test_evidence_verifier_exact_join_is_read_only_and_nonce_bound`
independently exercises the full counted-artifact/SQLite join. A verifier must
not require binary invocation and deep join implementation to appear in the
same file or within a fixed character distance.

## 11. Required tests

1. frozen-rejection authorization cannot be replaced by the envelope boolean;
2. disposition/auth identity/hash/current-reference mismatch fails closed;
3. pending append/apply authorization reconciliation, zero-candidate/zero-sink
   blocking, crash recovery and invalidation; the 4096th-progress-plus-pending
   boundary pattern-matches the exact
   `AuthorizationReconciliationBoundExceeded` fields;
4. authorization `Applied`/`Invalidated` events append before their projection
   CAS and survive every insert/append/ack/CAS crash point;
5. byte-identical manual replay and conflicting-byte rejection;
6. all deferral and ineligibility variants create exactly one logical-slot
   immutable cycle audit; byte-identical replay returns the stored event and
   different bytes for the same slot fail closed; a direct missing-decision
   caller returns typed `DurableDeliveryError::DecisionNotFound` before
   admission/audit, while candidate discovery cannot return a missing decision
   identity because it joins `delivery_decisions`;
7. post-Reserved candidate query, one cycle-global attempt and independent
   `DuplicateSuppressed`, plus a test-only no-set control that still cannot
   bypass transactional send ownership;
8. two-thread and real two-process admission and pre-call-CAS fencing;
9. pre-open boot-authority acquisition, missing/empty/ASCII-whitespace boot
   identity rejection before insert, exact persisted boot identity, same-boot
   safe-terminal exact resumption, transactional global `Running` exclusion,
   prior-boot orphan recovery, pre-spawn durable cycle identity, guard survival
   across async cancellation, caught-panic durable failure and JoinError
   recovery; ordinary-error-after-claim and
   panic-after-claim both quarantine every same-cycle nonterminal start and
   append/ack uncertainty before preparing failure, append/ack exact `Failed`
   bytes while still Running, then terminalize by CAS; namespace/hash/boot
   validation errors occur before guard acquisition, a fallible begin that
   first derives the next ordinal/identity and then proves no proposed
   cycle/Started slot committed returns a witness/input/ordinal/identity-bound
   opaque `NoRetryCycleCommitted`; `release_after_verified_no_cycle` consumes
   it only after a fresh transaction rederives every fact, while concurrent
   change, a commit-ambiguous begin and every unsafe post-claim Drop remain
   latched;
10. delayed first periodic tick;
11. persisted 30/120/600-second backoff and three-attempt exhaustion, including
    schedule delete/decrement/version/exhaustion/earlier-eligibility rejection
    and exact schedule-to-attempt-binding FK/ordinal validation; a
    frozen-rejection authorization initializes the zero-attempt schedule in
    the same transaction with `next_eligible_at=observed_at + 30s`;
    admission atomically creates the attempt/binding/schedule/`Reserved`
    relation, and a crash after that commit but before prepare resumes the same
    attempt without a new generation, ordinal, schedule increment or second
    sink call; exact source-date-to-next-Shanghai-midnight expiry is immutable,
    candidate and admission/manual transaction boundaries both reject equality
    and later times, terminalize one audited `ExpiredFreshness`, cannot be
    revived by PAM authority and make zero provider/renderer/sink calls;
12. fresh v0 creation, v1-to-v6, v2-to-v6, v3-to-v6, v4-to-v6 and the single
    v5-to-v6 migration, repeated v6 initialization and newer-version
    rejection all converge to one byte-compatible v6 manifest; every path
    registers and self-tests the same deterministic `sha256_hex(BLOB)` UDF
    before schema creation/migration/validation; v5-to-v6 preserves every
    BR-194 replay row, terminal-replay object, audit-kind definition and
    manifest semantic; canonical-byte/hash trigger mutations fail closed;
13. frozen envelope hash and all policy fields are revalidated;
14. provider and renderer measured deltas remain zero and the entire
    provider-free module/import/helper dependency graph passes its allowlist;
15. each of `UncertainAuditPending`,
    `UncertainTaskTransitionPending` and `UncertainManualReview` is absent from
    candidates and receives zero sink calls;
16. uncertainty after a retry never re-enters automatic retry;
17. disposition/audit/task/cycle append failure after a sink result never
    resends;
18. active bindings are cleared atomically on new disposition/terminal and on
    entry into `UncertainManualReview`, while historical authorization,
    cleared bindings and attempt bindings remain valid;
19. prepared attempts with pending/missing `SinkAttemptStarted` refs receive
    zero sink calls; an exact matrix covers
    `missing_start_event|pending_append|missing_immutable_ref`: before an
    acknowledged start and before consumed ownership, the narrow failure
    finalizer preserves byte-identical decision/attempt/binding/schedule rows,
    their canonical hashes and exact pending start bytes while terminalizing
    only the cycle; after either acknowledged start or consumed ownership, that
    unchanged-state exception is rejected, the ordinary same-cycle quarantine
    advances ownership to `InterruptedUncertain`, append/acknowledges all
    uncertainty and only then prepares `Failed`, with zero recovery sink calls;
    preparation cannot create an attempt, binding, generation or schedule
    update, and every field/hash/ref/generation/fence mismatch in
    `AppendedSinkAttemptStarted` is rejected;
    all fence fields are positive SQLite `INTEGER`/Rust `i64`, and
    DTO/canonical/outbox `started_at` values exactly match;
20. every real accepted/manual terminal and pending terminal state receives
    zero retry sink calls;
21. all v6 immutable refs reject empty and space/tab/LF/CR-only values in SQL
    and Rust;
22. namespace preimages are deterministic across UTF-8 input, separate
    Production/Test, preserve field boundaries and reject invalid/tampered
    hashes;
23. authenticated-operator serialization/validation covers principal hash,
    authority kind, authority timestamp and session evidence; target
    compatibility/authentication happens before resolver open, and a resolved
    production target can never contain test authority/root/capabilities;
    caller requests contain no timestamp and delayed execution preserves exact
    authority `validated_at` in row/canonical/schedule/outcome; the opaque
    attestation exists only after the real PAM call succeeds, binds the
    configured subject/service/mechanism plus OS-CSPRNG nonce, and the retry
    command cannot construct or replace its time or fields;
24. manual command resolution derives the sole namespace preimage/hash after
    auth/target compatibility but before all opens/reads, persists both in
    canonical authorization evidence, and rejects Production/Test or tampered
    preimage/hash disagreement; the non-test public surface exposes only the
    library-owned production request/outcome/entry and no injectable authority,
    resolver, target, session token or resolved capability;
25. pre-call crash, remote-accepted/post-call-pre-result crash, same-process
    repeated execute, two-process claim CAS, startup/JoinError quarantine and
    ordinary-error/caught-panic same-cycle finalization all produce at most one
    external call; every indeterminate start/in-flight attempt records
    `ProcessInterruptedAfterSinkStart`, creates or advances retained ownership
    to `InterruptedUncertain`, append/acknowledges uncertainty before cycle
    failure preparation and makes zero recovery sink calls; the interrupted
    cycle retains `Indeterminate/NULL`, then follows
    `FailurePending -> FailureAppended -> Failed/Terminalized`; normal success
    follows `CompletionPending -> CompletionAppended ->
    Completed/Terminalized`; once either Pending slot exists no error, panic,
    JoinError or startup path can insert the opposite terminal kind; the cycle
    guard remains held across sink execution and every completion/failure
    append, acknowledgement and terminal CAS, while an unsafe exit before any
    terminal safety point leaves the same-boot flag latched; an exit after a
    verified terminal safety point may release only because the next guard
    owner resumes that exact same-boot slot before begin and the begin
    transaction independently proves no global Running row before inserting
    the sole next `Started`;
26. successful execution returns
    `RetrySinkExecutionOutcome::Persisted(PersistedRetrySinkOutcome)`, whose result
    identity, terminal decision state and `TerminalRecorded` ownership state
    match one transactionally persisted join; a failpoint after the ownership
    pointer/state update but before the authoritative/non-late result insert
    rolls back both writes, leaves `AttemptInFlight`/`Started` with a NULL
    terminal-result pointer, and recovery conservatively reaches uncertainty
    with zero resend; a separate negative test proves a result-first insert is
    rejected immediately by the reverse trigger;
27. the public root re-export compile contract imports every runtime/CLI retry
    command, prepared/appended/claim/execute DTO and API without private-module
    access; a monitor-boundary compile/use test imports
    `RetryCycleOperation` and calls all six exact public
    `RetryCycleFailure` constructors while proving its fields remain private
    and observation remains read-only; a separate compile/signature test proves
    the runner, boot authority, namespace accessor, both guard release methods,
    completion/failure finalizers and prior-boot recovery all return
    `durable_delivery::Result<T>`, while only `retry_cycle_blocking` returns
    `std::result::Result<_, RetryCycleFailure>`; source/runtime tests reject
    `Result<_, String>`, `.to_string()`/text `map_err`, and stale
    `RetryCycleFailure::panic|join_error` calls inside that boundary;
28. authenticated production binary negative paths mutate nothing and never
    read a missing evidence file;
29. injected TEST_CODE command and evidence verification succeed only through
    cfg(test) library seams in their isolated namespace; the public evidence
    API has no target/root/test selector and the production CLI calls the
    production-only verifier; every serialized row contains the exact
    `durable_push_kind`, final `verified_retry_count` and `exact_join=true`;
    direct-library and CLI tests reject `require_count=0` and `257` before
    authority resolution/open and pattern-match exact requested/min/max
    variants, accept the exact 1/256 boundaries, and a streamed 257th distinct
    complete join pattern-matches exact max/attempted-distinct fields and fails
    with no partial serialization;
    a byte-identical artifact replay is accepted and deduplicated, while the
    same logical tuple with changed canonical bytes, changed retained hash, or
    both is rejected with separate exact conflict-flag variant tests for
    `(true,false)`, `(false,true)` and `(true,true)`; `(false,false)` is
    rejected before typed variant construction; the logical-tuple hash is
    recomputed from the frozen domain/preimage/encoding, matches the frozen
    golden vector and changes or rejects exact domain/schema/field-order/
    encoding mutations;
    and
30. every protected production artifact is unchanged before/after each test
    group;
31. the counted-producer registry covers `PushKind::ALL` exactly once in its
    stable order and rejects a missing kind, duplicate kind, unknown state,
    empty producer seam or empty reason before any acquisition/sink;
32. all fourteen disabled rows emit the exact design §1.1
    `disabled=no_producer reason=capability_unavailable:<reason_code>` startup
    line once and before acquisition/sink, and a later attempted invocation
    returns the identical reason with provider/data/sink counters all zero;
33. the sole enabled `ReviewProviderTopN` row emits only exact
    `enabled=durable_binding producer=push_templates::dispatch_r09_provider_top_n_outcome`,
    obtains a kind/seam-bound permit before gateway acquisition, freezes the
    source expiry, and remains Gate D blocked until a real push-log plus exact
    `push.delivery.audit` join exists; and
34. the syntax/multiline-aware all-15 inventory covers every generic counted
    entrypoint, template ID/label and caller in §1.2, rejecting an unclassified
    call, acquisition-before-guard or a second enabled producer;
35. fixed-HEAD `ReviewTask::R09`, its unique `ALL` position, `R-09` label and
    `SourceOnly` classification remain byte/semantically unchanged; Gate B
    wires only the producer/central-dispatch edges, and a provider-verified
    empty pair returns typed `Failed` with zero binding and zero sink calls;
36. final-pre-call expiry exercises Transaction-A failpoints after companion
    insert, ownership transition, expiry insert and cycle recount, proving every
    earlier write rolls back; successful cases prove `Confirmed(0)` and
    `Confirmed(n>0)` from exact ownership/result joins, then crash after
    Transaction A, Pending append and Appended acknowledgement in separate real
    child processes and prove recovery resumes expiry before quarantine;
37. Transaction-B failpoints after fence revocation, reservation/head release,
    direct decision CAS, binding clear and schedule CAS prove full rollback;
    exact replay is `AlreadyTerminalized`, partial/drift is a typed error, and
    every selector/recovery/post-validator ignores the effective terminal base
    `AttemptInFlight` row before generic state classification;
38. real two-connection result/expiry races cover both orders: a committed
    authoritative result makes companion/ownership/expiry writes fail, while
    any committed companion, freshness-terminal ownership or expiry row makes
    `trg_retry_sink_result_insert_rejects_pre_call_expiry_authority` reject the
    result; ownership and expiry triggers each independently recheck zero result
    and the all-twelve-trigger manifest is exact;
39. compile/source tests prove only `DurableDeliveryCoordinator` owns and reads
    `ProductionFreshnessClock`, while `retry_command.rs` owns no clock and
    cannot pass a freshness timestamp; and
40. the root compile contract imports
    `pub const MAX_RETRY_EVIDENCE_RESULTS: usize = 256`, and fixed-HEAD inventory
    tests bind the `review_batch.rs`/`v14_adapter.rs` blobs and classify
    `CloseCall`, `ForbiddenOps` and `ReviewFailure` calls after
    `#[cfg(test)] mod tests` as test-only rather than production callers;
41. the fixed-HEAD inventory test accounts for every direct production call in
    §1.2, including all repeated HoldingEvent, T0Advice, ReviewSignal and
    DailyReport call sites, and fails on any omitted or extra entry;
42. BR-198 tests prove future/same-day-15:35/prior-review-calendar ordering,
    every provider `f297` equals the requested review date, host `TZ` cannot
    change the explicit Asia/Shanghai review date or 15:35 boundary, closed-day
    initial acquisition remains runnable, and retry expiry is never extended;
43. BR-200 tests prove durable occurrence inspection precedes permit/provider/
    renderer/sink, terminal evidence replays without reacquisition, corrupt or
    ambiguous authority fails closed, and one business-date occurrence cannot
    create a second R-09 decision;
44. every authoritative result and `TerminalRecorded` ownership form an exact
    bidirectional join through write-once `terminal_sink_result_identity`, with
    orphan, duplicate, mismatch, second-result and result-on-nonterminal cases
    rolling back atomically; and
45. the public begin signature uses only
    `durable_delivery::Result<RetryCycleBeginOutcome>`, while rollback tests
    prove retry-origin Reserved work either retains the four-stage seam or
    remains untouched and never reaches legacy `resume_deliverable`;
46. BR-198 tests independently reject capture before the requested date,
    before the trusted request-start observation, after the trusted request-
    completion observation, capture crossing the Shanghai observation-date
    midnight and invalid provider capture timestamps; the exact raw provider
    timestamp bytes remain bound to evidence, with atomic pair rejection and
    zero durable/sink writes;
    a separate round-trip/mutation test proves both exact raw timestamp arrays
    and their domain-separated hashes survive the pair binding and any byte
    change is rejected before durable/sink;
47. a prior-date initial R-09 acquisition is explicitly admitted despite an
    elapsed retry expiry, while retry-candidate discovery, automatic retry
    admission and manual retry authorization reject the same elapsed expiry;
48. BR-200 tests separately assert Delivered-missing-hydration retryability,
    Rejected/Uncertain terminal behavior and nonterminal retryability,
    including exact reason, next-attempt and zero-call fields;
49. producer, transition and hydration evidence preserve exact ordered rule
    IDs `[BR-110,BR-140,BR-192,BR-194,BR-198,BR-200]`; and
50. SourceOnly R-09 accepts only `(business_date, observed_at)` and static tests
    reject `BannerCtx`, banner text, AccountMode and broker-snapshot authority.

The selector inventory also contains one owning library test per exact
full-snapshot contract:

```text
br192_reserved_selector_returns_all_257_rows_in_total_order_without_truncation
br192_prior_boot_selector_returns_all_257_rows_before_new_cycle
br192_retry_candidate_selector_returns_all_257_rows_in_one_frozen_snapshot
br192_primary_selector_source_rejects_limit_offset_cursor_and_caller_cardinality
br192_start_audit_unavailable_prestart_reason_matrix_preserves_rows_hashes_and_zero_sink
br192_start_audit_unavailable_post_ack_or_consumed_rejects_exception_and_quarantines
br192_authorization_reconcile_bound_returns_exact_typed_variant
br192_cycle_audit_reconcile_bound_returns_exact_typed_variant
br192_retry_cycle_ordinal_is_positive_unique_immutable_and_non_deletable
br192_retry_cycle_identity_rederives_from_retained_ordinal_and_exact_fields
br192_counted_producer_attestation_is_same_transaction_immutable_and_hash_valid
br192_counted_producer_attestation_preimage_excludes_digest_and_has_golden_hash
br192_counted_producer_attestation_projection_rejects_preimage_or_digest_mutation
br192_v5_decisions_gain_no_synthetic_producer_attestation
br192_delivery_envelope_none_attestation_preserves_v5_canonical_bytes
br192_delivery_envelope_attestation_setter_has_one_permitted_production_caller
br192_retry_schedule_persists_exact_source_date_expiry_and_terminal_state
br192_retry_expiry_outbox_replays_prepare_append_ack_and_terminalize_crashes
br192_expiry_preparation_outcome_is_total_and_never_uses_durable_error_for_business_expiry
br192_prepare_retry_attempt_expired_before_start_returns_expiry_prepared_without_start
br192_claim_retry_sink_execution_expired_after_appended_start_returns_uncertainty_without_permit
br192_start_first_uncertainty_reconciliation_clears_active_schedule_to_fixed_point
br192_admission_expiry_is_appended_and_terminalized_before_cycle_advances
br192_historical_terminal_start_does_not_block_current_attempt_expiry
br192_pre_call_expiry_companion_requires_complete_same_transaction_triple
br192_pre_call_expiry_companion_is_canonical_immutable_and_commit_deferred
br192_pre_call_expiry_transaction_a_rolls_back_every_failpoint
br192_pre_call_expiry_cycle_recount_confirms_zero_and_prior_results
br192_pre_call_expiry_with_other_started_keeps_cycle_indeterminate_atomically
br192_pre_call_expiry_with_interrupted_uncertain_keeps_cycle_indeterminate_atomically
br192_pre_call_expiry_recovery_precedes_prior_boot_uncertainty
br192_pre_call_expiry_transaction_b_rolls_back_every_failpoint
br192_pre_call_expiry_transaction_b_is_exactly_idempotent
br192_effective_expired_attempt_is_excluded_before_base_state_classification
br192_sink_result_and_pre_call_expiry_race_is_total_in_both_orders
br192_sink_result_reverse_trigger_rejects_each_partial_expiry_authority
br192_pre_call_ownership_and_expiry_triggers_each_recheck_zero_result
br192_schema_v6_contains_all_nine_expiry_ordering_triggers
br192_schema_v6_fresh_and_v1_v2_v3_v4_v5_upgrade_paths_validate
br192_schema_v6_cycle_ordinal_manifest_is_identical_across_v0_v1_v2_v3_v4_v5
br192_schema_v6_repeated_initialization_is_idempotent
br192_schema_newer_than_v6_fails_before_mutation
br192_v5_to_v6_preserves_br194_replay_manifest_audit_kinds_and_rows
br192_durable_sha256_udf_is_registered_before_every_schema_path
br192_durable_sha256_udf_registration_follows_complete_descriptor_binding
br192_durable_sha256_udf_never_runs_before_wal_shm_attestation
br192_wal_materialization_is_the_only_pre_binding_sqlite_exception
br192_wal_materialization_rejects_omitted_reordered_or_extra_sqlite_steps
br192_rusqlite_031_exposes_utf8_deterministic_and_innocuous_function_flags
br192_durable_sha256_udf_rejects_null_text_and_wrong_type
br192_v6_authority_triggers_recompute_canonical_sha256
br192_v6_authority_triggers_reject_bytes_hash_and_combined_mutations
br192_python_br194_verifier_uses_hashlib_without_sql_callback_or_trigger_execution
br192_cycle_begin_derives_identity_before_running_check_and_binds_no_commit_proof
br192_no_commit_branch_queries_zero_proposed_cycle_and_started_before_rollback
br192_consume_no_commit_proof_rederives_next_identity_and_rejects_concurrent_change
br192_no_commit_proof_rejects_domain_schema_field_order_encoding_and_witness_mutations
br192_empty_running_check_atomically_persists_exact_ordinal_identity_and_started
br192_cycle_ordinal_exhaustion_returns_exact_typed_variant_without_write
br192_evidence_verifier_accepts_and_deduplicates_byte_identical_artifact_replay
br192_evidence_verifier_rejects_zero_conflicting_duplicate_mismatch_and_write_capable_target
br192_evidence_query_count_bounds_apply_before_authority_open
br192_evidence_stream_rejects_257th_distinct_join_without_partial_output
br192_evidence_query_count_zero_returns_exact_typed_variant_before_authority_open
br192_evidence_query_count_257_returns_exact_typed_variant_before_authority_open
br192_evidence_logical_tuple_hash_matches_frozen_golden_and_recomputes
br192_evidence_logical_tuple_hash_rejects_domain_schema_field_order_and_encoding_mutations
br192_evidence_conflicting_canonical_bytes_returns_exact_typed_variant
br192_evidence_conflicting_canonical_hash_returns_exact_typed_variant
br192_evidence_conflicting_canonical_bytes_and_hash_returns_exact_typed_variant
br192_evidence_conflicting_duplicate_zero_flags_rejected_before_variant_construction
br192_evidence_257th_distinct_returns_exact_typed_variant_without_partial_output
br192_retry_cycle_failure_public_typed_constructors_compile_from_monitor_boundary
br192_retry_cycle_failure_typed_constructors_bind_exact_reason_fields_and_hashes
br192_retry_cycle_failure_constructors_reject_wrong_variant_invalid_digest_and_unknown_operation
br192_retry_runtime_boundary_uses_only_durable_result_and_exact_failure_constructors
br192_begin_not_committed_propagates_exact_typed_error_after_proof_release
br192_begin_commit_ambiguous_propagates_exact_typed_error_and_latches_guard
br192_guard_failures_return_exact_typed_variants_without_string_downgrade
br192_counted_producer_registry_covers_all_kinds_once
br192_counted_producer_registry_rejects_missing_duplicate_and_empty_state
br192_missing_producers_emit_exact_startup_banners_before_acquisition
br192_enabled_producers_name_binding_seams_and_never_emit_disabled_banner
br192_disabled_producer_attempt_returns_before_acquisition_and_sink
br192_r09_enabled_producer_acquires_permit_before_gateway_and_freezes_expiry
br192_r09_fixed_head_task_surface_is_preserved_and_empty_is_failed
br192_expired_freshness_terminal_is_single_audited_and_not_revivable
br192_all_counted_callers_require_catalog_permit_or_binding
br192_production_freshness_clock_is_coordinator_owned_only
br192_root_exports_max_retry_evidence_results_as_256
br192_fixed_head_inventory_excludes_cfg_test_counted_callers
br192_fixed_head_inventory_classifies_every_counted_entry_call
br192_pre_call_expiry_requires_result_terminal_ownership_bijection
br192_cycle_begin_public_signature_uses_only_durable_result_error_channel
br192_rollback_preserves_four_stage_retry_origin_reserved_recovery
br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable
br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297
br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink
br192_br198_same_day_1535_boundary_precedes_terminal_preflight
br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry
br192_br198_host_tz_cannot_change_shanghai_review_date_or_1535_boundary
br192_br198_capture_before_trusted_request_start_fails_pair_before_durable_sink
br192_br198_capture_after_trusted_request_completion_fails_pair_before_durable_sink
br192_br198_capture_raw_bytes_round_trip_and_mutation_rejects_pair_before_durable_sink
br192_br200_r09_delivered_preflight_precedes_permit_gateway_renderer_and_sink
br192_br200_r09_rejected_uncertain_and_nonterminal_preflight_are_provider_free
br192_br200_r09_missing_hydration_and_corrupt_or_ambiguous_authority_fail_closed
br192_br200_r09_no_occurrence_orders_preflight_then_permit_then_provider_then_renderer_then_sink
br192_br200_r09_startup_barrier_failure_is_provider_free
br192_br200_business_date_once_claim_prevents_second_r09_decision
```

The reboot suite is a real parent/child-process protocol, not two in-process
opens. A writer child receives one parent-created nonce-bound TEST_CODE root and
boot identity, persists either `CompletionPending`, `CompletionAppended`,
`FailurePending` or `FailureAppended`, writes a ready receipt containing only
the root, cycle identity, phase and expected hashes, and exits without
returning an in-memory terminal DTO. A recovery child uses a distinct boot
identity, opens that exact root through the production-shaped recovery entry,
makes zero provider/renderer/sink calls, validates the stored bytes/hashes and
resumes only the existing terminal kind: Pending performs exact append then
acknowledgement and CAS; Appended performs only validation and CAS.

For each FailurePending and FailureAppended fixture, the parent clones the
isolated fixture and corrupts exactly one of: typed-preimage bytes,
typed-preimage hash, envelope bytes, envelope hash, payload identity, or the
Failed binding (`cycle_identity`, payload identity/hash or reason binding).
Each recovery child must exit non-zero, leave the original terminal phase and
bytes unchanged, create no opposite/new terminal slot, emit no successful
append acknowledgement and make zero sink calls. The unmodified positive
fixture must terminalize using only SQLite payload/outbox bytes. The
`NotPrepared` positive case separately proves strict two-phase ordering: no
failure payload or Failed outbox exists until all uncertainty outboxes are
appended and acknowledged.

The declared exact tests are:

```text
br192_precycle_namespace_error_never_acquires_guard
br192_begin_not_committed_proof_releases_guard
br192_no_commit_proof_concurrent_change_latches_outer_guard
br192_begin_commit_ambiguous_error_latches_guard
br192_same_boot_completion_pending_blocks_new_started_until_exact_resume_terminalizes
br192_same_boot_failure_appended_blocks_new_started_until_exact_resume_terminalizes
br192_begin_db_admission_rejects_any_global_running_before_second_started
br192_same_boot_safe_terminal_selector_is_current_identity_exact_and_totally_ordered
br192_normal_error_after_claim_quarantines_same_cycle_attempt_before_failed
br192_caught_panic_after_claim_quarantines_same_cycle_attempt_before_failed
br192_completion_pending_error_resumes_completed_without_failed_slot
br192_completion_pending_reboot_resumes_completed_without_failed_slot
br192_completion_appended_reboot_resumes_completed_without_failed_slot
br192_failure_pending_reboot_uses_only_persisted_bytes
br192_failure_appended_reboot_uses_only_persisted_bytes
br192_failure_reboot_corruption_matrix_fails_closed
```

These exact names are owned once by the
`src/bin/monitor/durable_delivery_runtime.rs` test module. The broader counted
cutover harness may invoke or compose their fixtures but must not redeclare the
same bare test names. The CompletionPending and CompletionAppended reboot tests
are both real parent/writer/recovery-child protocols; the similarly named
CompletionPending error test is a separate same-process terminal-kind
regression and cannot substitute for the reboot parent.

## 12. Failure modes

| Failure | Durable result |
| --- | --- |
| authorization reconciliation remains pending | `AuthorizationReconciliationBlocked`, cycle failed; candidate query skipped; zero sink calls |
| authorization transition event pending append/ack | retain `PendingApply`; recover exact stable event; zero sink calls |
| authorization does not target current rejection | typed ineligibility plus immutable cycle event |
| direct admission names a missing decision | typed `DurableDeliveryError::DecisionNotFound` before transaction/audit; candidate query cannot produce this case |
| envelope/hash/policy mismatch | fail closed, durable failed-cycle event |
| budget full | `Deferred::DailyBudgetFull`; zero sink/provider/renderer calls |
| claim owned elsewhere | `Deferred::BusinessDateClaimedByOther`; zero calls |
| rolling head reserved/uncertain | exact typed deferral; zero calls |
| cooldown/backoff active | exact persisted `eligible_at`; zero calls |
| source evidence reaches `expires_at` before discovery, admission or manual authorization | append/ack exact `RetryExpiredFreshness`; retained schedule becomes terminal `ExpiredFreshness`, active binding is cleared, typed `NoLongerEligible::ExpiredFreshness { expires_at }`; zero provider/renderer/sink calls and no manual revival |
| manual target is already expired and has no authorization/schedule | one `BEGIN IMMEDIATE` creates the zero-attempt schedule plus nullable-authorization `ManualTargetExpiredBeforeAuthorization` Pending expiry, no authorization; exact append/ack/CAS returns typed expired ineligibility |
| expiry prepare wins before start prepare | Pending expiry authority commits first; all later start insertion/ack and ownership writes are rejected by exact triggers; expiry resumes to terminal fixed point |
| Pending/Appended start or ordinary ownership wins before ordinary expiry prepare | expiry writes no outbox row; retained attempt follows uncertainty because immutable start append may already have happened, except that the live permit holder may still execute the exact final-pre-call Transaction-A no-call protocol |
| ownership was claimed fresh but final pre-call observation reaches expiry | invoke zero sink; Transaction A atomically persists companion, `FreshnessExpiredBeforeExternalCall`, Pending `ReservedExpiredBeforeSink` and the exact cycle recount (`Indeterminate/NULL` if another same-cycle `Started|InterruptedUncertain` remains, otherwise `Confirmed(0)` or prior-result `Confirmed(n)`); append/ack exact expiry; Transaction B revalidates the same branch, revokes fence, releases reservation/head, restores the decision's original `RejectedDurable`, clears binding and terminalizes schedule while retaining the schema-v5 base attempt as effective terminal history |
| sink result races pre-call expiry authority | `BEGIN IMMEDIATE` plus all twelve triggers establish a total order: result first makes every expiry member fail its zero-result recheck; any companion/expiry-terminal ownership/expiry row first makes the reverse `BEFORE INSERT ON sink_results` trigger reject the result; never both |
| retry count exhausted | one durable `RetryAttemptsExhausted`; removed from candidates |
| competing process wins | loser observes changed state/generation; zero sink calls |
| namespace/preimage/hash or owner-boot validation fails before guard acquisition | no guard, cycle or `Started` slot exists; return the typed error |
| identity-first begin transaction selects retained Running, proves zero proposed cycle/`Started` rows and rolls back | coordinator returns witness/input/ordinal/identity-bound opaque `NoRetryCycleCommitted`; a fresh transaction must rederive and consume the exact proof before guard release |
| concurrent cycle insert or selected Running mutation occurs before no-commit proof consumption | proof consumption rejects; outer guard remains latched |
| begin result is commit-ambiguous or no no-commit proof is available | guard remains latched; no second same-boot cycle is admitted |
| retained retry-cycle ordinal is `i64::MAX` | return exact `RetryCycleOrdinalExhausted { max_ordinal: i64::MAX }`; no cycle or `Started` row is written |
| process dies after admission commit and before prepare | retain the same `Reserved` attempt/binding/schedule; startup prepares that identity without allocating or sending twice |
| audit append fails before sink | retained Reserved/pending audit; no sink call this cycle |
| prepared attempt start event pending/missing/non-ref before acknowledged start and consumed ownership | typed `DurableDeliveryError::RetryAttemptStartAuditUnavailable { attempt_identity, reason_code }`; persisted-state classification admits only the narrow unchanged-state branch; runner prepares and append/acks the cycle `Failed` event with `retry_attempt_start_audit_unavailable` while the cycle remains Running, then terminalizes by CAS; decision/attempt/binding/schedule, canonical hashes and exact pending start bytes remain byte-identical; zero sink calls |
| the same start-audit-unavailable reason after acknowledged start or consumed ownership | persisted-state classification rejects the unchanged-state exception; ordinary same-cycle quarantine advances retained ownership to `InterruptedUncertain`, append/acks all uncertainty and only then prepares/appends/acks/terminalizes the matching `Failed` slot; zero recovery sink calls |
| completion outbox append or acknowledgement fails | cycle remains `Running/CompletionPending` with complete immutable completion bytes and exact Pending outbox; guard may release only after validating this safety point; the next same-boot guard owner resumes only Completed to terminal before begin, and the begin transaction otherwise creates zero new `Started` |
| completion outbox is acknowledged but terminal CAS has not run | cycle remains `Running/CompletionAppended`; same-boot/startup/JoinError recovery validates exact stored completion bytes and performs only the Completed CAS before a later cycle can begin |
| failure outbox append or acknowledgement fails | cycle remains `Running/FailurePending` with complete immutable typed-preimage/envelope bytes and exact Pending outbox; guard may release only after validating this safety point; the next same-boot guard owner resumes only Failed to terminal before begin; never reports terminal Failed early |
| failure outbox is acknowledged but terminal CAS has not run | cycle remains `Running/FailureAppended`; same-boot/startup/JoinError recovery uses only persisted failure payload/outbox bytes, recomputes both hashes and performs the CAS before a later cycle can begin |
| crash after prior-boot start append or pre-call CAS, including before actual call | an exact companion + `FreshnessExpiredBeforeExternalCall` + Pending/Appended expiry is reconciled first as definite zero-call expiry through Transaction B; only a remaining unmatched appended start/consumed ownership forces typed `ProcessInterruptedAfterSinkStart` uncertainty; recovery makes zero sink calls |
| ordinary cycle error or caught panic after same-cycle start/claim | common failure finalizer quarantines every qualifying same-cycle attempt, append/acks all uncertainty before `Failed`, and makes zero sink calls; pending uncertainty prevents premature `Failed` |
| remote accepts, process dies before result recording | same conservative uncertainty; never infer rejection/acceptance and never resend |
| repeated/current or competing process execute | only `Reserved -> AttemptInFlight` CAS winner receives the consumed permit; losers make zero sink calls |
| `record_sink_result` fails after ownership becomes `TerminalRecorded` with its prospective result pointer but before the authoritative/non-late result insert | the single ownership/result transaction rolls back completely; no sink result remains, decision stays `AttemptInFlight`, ownership stays `Started` with a NULL terminal-result pointer, and recovery records `ProcessInterruptedAfterSinkStart` uncertainty with zero resend |
| definite sink rejection | new frozen rejection; later retry only through its appended/applied authorization |
| sink timeout/transport/cancel/write-after-loss | uncertainty; retained reservation; never auto-retry |
| result/disposition/task append fails after sink | exact pending/uncertain recovery only; never resend |
| runner panic | caught inside blocking closure; same-cycle indeterminate attempts are quarantined and their uncertainty append/acked before exact `Failed` bytes |
| blocking `JoinError` | async parent recovers the retained cycle identity through Pending append, acknowledgement and terminal CAS; interrupted cycle sink calls remain Indeterminate |
| boot identity missing/empty/ASCII-whitespace-only | pre-open boot authority or coordinator validation fails before cycle/`Started` insert |
| same-boot safe-terminal `Running` cycle is observed | startup orphan recovery does not mutate it; before any new begin the current-identity exact resumer terminalizes its frozen slot with zero sink, while same-boot `NotPrepared` remains untouched and causes transactional global Running exclusion |
| begin observes any unresolved global `Running` row after same-boot/prior-boot recovery | after deriving the proposed ordinal/identity, return definite `RetryCycleAlreadyRunning` plus exact witness-bound `NoRetryCycleCommitted`; zero new cycle/`Started` bytes and the guard releases only after a fresh transaction rederives and consumes that proof |
| process dies with prior-boot `Running` cycle | next lock-owning startup first resumes an existing completion/failure terminal slot exactly; only `NotPrepared` append/acks all uncertainty to a fixed point and then records a complete `ProcessInterrupted` payload; all recovery uses persisted bytes without an in-memory DTO |
| cycle owner exits before terminal or durable completion/failure Pending/Appended safety point | guard does not clear the same-boot running flag; later cycle admission is blocked until restart/recovery |
| shutdown cancels async waiter | blocking cycle retains guard, finishes audit, then releases |
| manual authentication/target validation fails, including PAM/TTY/nonce failure | no attestation and no filesystem/database mutation |
| authenticated operator or resolved target mismatches kind/root/session evidence | fail before authority/evidence open or mutation; production never accepts test authority |
| caller attempts to supply authorization time or production authority/resolver/target | impossible in the public request/API; production library owns those values and persists exact PAM `validated_at` |
| deterministic `sha256_hex` UDF registration or fixed-vector self-test fails after complete main/WAL/SHM descriptor binding | coordinator bootstrap fails with `durable_sha256_udf_unavailable` before connection configuration/schema inspection/migration/validation and before any business mutation; retained descriptor lifecycle is closed normally |
| UDF registration is attempted before main/WAL/SHM binding or after schema work | bootstrap-order test/checker rejects the implementation; no coordinator is published |
| Python BR-194 verifier attempts `create_function`, DML or trigger execution | release checker/verifier source gate rejects it; Python remains a read-only catalog/row reader and uses `hashlib.sha256` on returned BLOB bytes |
| v6 BR-192 authority trigger observes canonical bytes whose recomputed digest differs from the stored lowercase SHA-256 | triggering mutation is rejected atomically; no authority projection, outbox or manifest state advances |
| schema v5-to-v6 migration/post-validation fails | transaction rolls back; `user_version` remains v5 and every pre-existing BR-194 row/object/audit-kind/manifest semantic remains unchanged |

## 13. Old-module disposition

| Existing module/behavior | Action | Reason |
| --- | --- | --- |
| earlier BR-192 provider-free-retry Gate A/B drafts and partial code that never passed Gate C | replace | This corrective design/plan/business-rule triple is the sole retry source of truth; unaccepted spec-on-spec chaining is forbidden. |
| `reacquire_rejected` boolean API | replace | It cannot prove frozen authorization identity and erases typed deferrals. |
| existing `ReconcileSummary` | adopt unchanged | Its current eight fields and single constructor remain startup reconciliation evidence; retry candidates use a new coordinator query and `RetryCycleEvidence`, avoiding a fabricated non-existent summary field. |
| monolithic retry use of `resume_deliverable` | replace for retry origin | Retry must use prepare -> append/ack start -> pre-call ownership CAS -> execute; indeterminate starts quarantine and ordinary non-retry Reserved recovery is unchanged. |
| existing disposition/state/task immutable outboxes | adopt | Preserve exact-byte recovery and append acknowledgement. |
| fixed-HEAD R-09 hydration/support code | adopt | It supplies task identity only; Gate B still builds and tests the real producer seam. |
| worktree `CapitalDataGateway::provider_top_n_pair` and `dispatch_r09_provider_top_n_outcome` candidates | inspect then conform | They are useful candidate code but have no authority until the exact Gate-B implementation, permit, expiry and tests pass. |
| generic counted callers in `notify.rs`, `push_templates.rs`, `main.rs`, `review_batch.rs` and `v14_adapter.rs` | guard/disable | Every call must be classified by the catalog; fourteen paths stop before counted-specific acquisition and R-09 alone receives a permit. |
| `Fixture`, `MemoryAppendPort`, `StaticSink`, existing helper functions | adopt | Extend established physical-isolation and deterministic sink patterns. |
| producer `DeliveryEnvelope.retry_authorized` | reject as authority | Compatibility projection only; cannot authorize admission. |
| observer-only retry summaries | replace | Governance decisions require durable cycle-bound audit. |
| recursive startup reconciliation | reject | Startup and periodic work share one non-recursive cycle. |
| immediate `tokio::time::interval` first tick | replace | `interval_at` prevents immediate duplicate retry after startup. |

## 14. Rollback

Rollback restores the previous runtime behavior in new v6-aware code; it is not
a schema downgrade or data deletion:

Task 8 creates the reviewed forward patch
`tools/release/disable_br192_periodic_retry.patch`. Its only `diff --git`
target is `src/bin/monitor/main.rs`, and its only semantic change removes the
single periodic retry-runner installation while retaining the startup catalog,
R-09 producer/gateway, BR-200 occurrence preflight, v6 schema/UDF/manifest,
retry-origin classifier and all reconciliation APIs. The release gate runs
`git apply --check` against the exact accepted release source and rejects a
second file, catalog edit, dependency edit, schema edit, R-09 edit or generic-
delivery restoration. Rollback applies this artifact forward from the accepted
release SHA; it never reverts the atomic Task-8 commit.

Task 8 also creates
`tools/release/verify_br192_forward_rollback.sh`. Normal Gate B and Gate C run
this verifier against either the literal fully staged Task-8 candidate commit
object or the literal accepted release commit before deployment, not against a
pre-Task-8 branch HEAD and not only after an incident. The candidate commit is
created with `git write-tree` + `git commit-tree` without moving the branch;
its tree must equal the subsequent reviewed implementation commit tree. The
verifier enforces one exact patch target, applies the patch in an isolated
detached worktree, rejects edits to R-09/BR-200/catalog/dependency/schema/UDF/
manifest/reconciliation/audit/startup-cycle authorities, builds the rollback
release binary, and runs all twelve exact `br192_br198_*` tests, the seven exact
`br192_br200_r09_*` tests plus the BusinessDateOnce claim test, schema-v6 and
v5-preservation tests, the complete counted-producer catalog suite, exact Magic
release-identity test, the two exact v6 four-stage retry-origin recovery tests,
and exact startup-cycle isolation test
`durable_delivery_runtime::tests::br192_startup_and_periodic_paths_share_one_cycle_algorithm`.
A stale, multi-file,
semantically broad, unbuildable or zero-test patch blocks release.

1. stop the monitor;
2. create a rollback branch at the exact accepted release SHA, apply and stage
   the reviewed forward patch, then build and deploy that new forward-compatible
   rollback binary while retaining v6 schema recognition, deterministic
   `sha256_hex` registration and complete shared-manifest validation;
3. disable only the periodic BR-192 retry runner at its single main startup
   call site. Preserve the exact validated 15-row catalog bytes, identity and
   enabled R-09 row unchanged so retained attestations, expiry outboxes and
   initial R-09 durable delivery remain verifiable; never route R-09 to legacy
   generic delivery. A catalog change is a new Gate-A decision and is forbidden
   until every retained/pending authority has terminalized;
4. retain `counted_producer_attestations`, `retry_authorizations`,
   authorization events/bindings, attempt bindings, `retry_schedules`,
   `retry_expiry_audit_outbox`, `retry_pre_call_expiry_authorities`,
   `retry_send_ownership`, `retry_cycles`,
   `retry_cycle_failure_payloads` and every authorization/expiry/cycle audit
   outbox for the five-year audit period;
5. do not restore automatic `reacquire_rejected`;
6. leave authorized rejections non-deliverable until a corrected release,
   while preserving every `ExpiredFreshness` terminal and its audit; and
7. restart the v6-aware rollback binary with a retained retry-origin
   classifier. Every retry-origin `Reserved` row must continue through the
   four-stage `prepare_retry_attempt -> append/ack start ->
   claim_retry_sink_execution -> execute_prepared_retry_sink` path, including
   its expiry and uncertainty branches; it must never enter legacy
   `resume_deliverable`. If the rollback binary cannot provide that four-stage
   seam, it leaves the retry-origin row untouched and fail-closed for the next
   corrected release. Existing pending terminal bytes continue only through
   their exact idempotent reconciliation path.

Rollback must never delete a reservation generation, attempt fence,
authorization, receipt, disposition or audit record.
An historical v2/v3/v4/v5 binary must never be launched against the v6 database,
and rollback must not lower `user_version`, unregister `sha256_hex`, or remove
v5 baseline or v6 companion columns, tables, triggers or indexes. The rollback
binary must preserve all BR-194 replay objects, audit kinds and manifest
semantics introduced before or alongside BR-192.

## 15. Gate A evidence

The historical independent review of the exact identities recorded in §0.1
found `Critical=0 / Important=0 / Minor=1`; its packaging-only Minor-1 was
closed by the cached-row proof there. Later exact reviews found C0/I1, C0/I2,
C1/I6/M1, C0/I3/M0 and then a final RED over the newer staged contract. This
revision closes the identified design-side findings with the exact two-transaction
effective expiry terminal, precedence-safe recovery, atomic cycle recount,
exact reverse result triggers, typed expiry, coordinator-only clock, fixed-HEAD
R09/test caller truth, current-attempt predicate, permit-task ownership, public
evidence bound and complete blob binding. These amendments invalidate all
earlier identities for Gate
progression. A fresh independent review of the new exact design/plan/row
identities must return C0/I0 before Gate B may resume. Gate B/C/D and live
evidence remain pending.
The historical reviewer confirmed the following unchanged baseline:

- all state transitions and append/apply recovery paths above are represented
  in the implementation plan;
- this corrective document explicitly replaces every unaccepted BR-192 retry
  Gate A/B draft instead of depending on one as an accepted specification;
- BR-192 in `docs/business_rules.md` will be updated before logic changes;
- every changed/new file is listed and every planned Gate-B source path cites
  BR-202 in the same staged slice;
- validation commands are executable; raw JSON coverage is diagnostic and
  only the BR-202 isolated wrapper can mint Gate-D authority; and
- no task relies on an undefined helper, silent zero-test filter or production
  artifact access;
- the fixed-HEAD repository baseline is schema v5 and BR-192 companion
  authority objects land through exactly one coherent v5-to-v6 transaction;
  fresh/v1/v2/v3/v4/v5 paths converge to one v6 manifest while preserving every
  v5 BR-194 replay object, audit kind, row and manifest semantic;
- `rusqlite` enables its `functions` feature, one central
  `register_durable_sql_functions(&Connection)` seam registers and self-tests
  deterministic BLOB-only `sha256_hex` only after complete main/WAL/SHM
  descriptor binding and before connection configuration/fresh-schema
  creation/migration/validation on every Rust/rusqlite durable connection;
  every v6 BR-192 authority trigger, the BR-192 Rust verifier and shared checker
  recompute the same canonical SHA-256, while the Python BR-194 read-only
  verifier uses `hashlib.sha256` and catalog inspection without registering a
  SQL callback or firing a trigger;
- the exact public operational-error inventory includes
  `RetryCycleOrdinalExhausted { max_ordinal: i64 }`; exhaustion occurs only at
  `i64::MAX`, returns that exact field value and performs no cycle/`Started`
  write;
- the pre-call `Reserved -> AttemptInFlight` CAS, retained consumed ownership,
  single-use permit and coordinator-owned
  `record_sink_result(AttemptInFlight)` precondition close the
  post-send/pre-result double-send window; the public execute method returns
  only `RetrySinkExecutionOutcome`, never a bare sink result;
- all prepare/reconcile/validate/claim/execute operations are
  `DurableDeliveryCoordinator` methods with explicit database authority;
- the authoritative-result/`TerminalRecorded` fault point proves full
  transaction rollback and conservative zero-resend recovery;
- startup and JoinError recovery first resume every exact pre-call expiry
  triple through Transaction B and exclude its effective terminal attempt;
  they quarantine only remaining qualifying prior-boot attempts, while both
  ordinary cycle error and caught panic use one common
  finalizer to quarantine every qualifying same-cycle appended-start,
  consumed/`Started` ownership or in-flight attempt without a terminal result
  as `ProcessInterruptedAfterSinkStart`; all uncertainty is append/acknowledged
  before cycle `Failed`, recovery makes zero sink calls and convergence occurs
  only through the three uncertainty/manual-review stages;
- terminal preparation persists complete canonical terminal bytes and, for
  failure, the reason-specific typed preimage/envelope plus both hashes; a fresh process
  can terminalize
  `CompletionPending|CompletionAppended|FailurePending|FailureAppended` solely
  from those SQLite bytes; prior-boot `NotPrepared` recovery append/acks
  uncertainty to a fixed point before it creates any failure payload or Failed
  outbox;
- before every new begin, the current-identity exact same-boot selector resumes
  all four safe terminal phases in frozen total order with zero sink calls; the
  begin `BEGIN IMMEDIATE` then computes the next retained positive unique
  ordinal, derives the exact proposed identity, and rejects any remaining
  global Running row before inserting a cycle or `Started`; that branch proves
  zero proposed cycle/Started rows and returns an input/ordinal/identity/
  witness-bound opaque proof whose fresh transactional consumption must
  rederive every fact, so concurrent change cannot release the guard;
- the retry guard encloses sink execution and completion/failure
  append/ack/terminal-CAS work; its running flag can clear only after a
  terminal row, a verified durable completion/failure Pending/Appended safety
  point, or successful fresh-transaction consumption of the exact
  coordinator-issued `NoRetryCycleCommitted` proof after an identity-first
  failed begin, never merely when retry work or an unwind ends;
- JoinError, orphan, cancellation and startup recovery accept only
  `RetryCycleRecoveryCapabilities` containing coordinator and append; no
  sink/provider/renderer-bearing capability is clonable into recovery;
- authorization append acknowledgement stores no untrusted append-authority
  timestamp;
- the only production operator attestation constructor runs the real PAM path,
  captures `validated_at` and an OS-CSPRNG nonce only after success, and exposes
  no public constructor/accessor that lets retry code manufacture authority;
  its canonical session evidence binds subject hashes, configured PAM service,
  mechanism, nonce hash and the exact authority time;
- a missing direct admission identity returns the typed coordinator error
  before an `AdmissionResult` or audit can be created, while candidate
  discovery's decision join makes that absence unrepresentable;
- all fence representations are positive SQLite `INTEGER`/Rust `i64`, and the
  start timestamp is byte-identical across prepared DTO, appended DTO,
  canonical bytes and persisted outbox `created_at`;
- manual namespace preimage/hash derivation occurs after authentication and
  exact target compatibility but before any open/read, and canonical
  authorization evidence binds both;
- the non-test public authorization surface is exactly the library-owned
  production request/outcome/entry, exposes no injectable authority/resolver/
  target/session/resolved capability, and persists the PAM authority's exact
  `validated_at` without any caller timestamp;
- the non-test evidence surface exposes only the production query/result
  types and production verifier; TEST_CODE target/root injection is cfg(test)
  library-only and its positive fixture is not an external integration test;
- `VerifiedRetryEvidence` and the directly serialized production CLI output
  carry exact `durable_push_kind`, final `verified_retry_count` and
  `exact_join=true` fields that match Gate D acceptance;
- `logical_tuple_sha256` has one literal NUL-terminated domain, one closed
  version/rule/field schema, one exact struct order and compact UTF-8 JSON
  encoding; the bounded-map key and hash are recomputed from that same
  validated preimage, and exact golden/recomputation plus
  domain/schema/field/order/encoding mutation tests are listed;
- bytes-only, hash-only and bytes-plus-hash duplicate conflicts pattern-match
  exact flags `(true,false)`, `(false,true)` and `(true,true)`, while the
  `(false,false)` branch is rejected before the typed error can be
  constructed;
- all six `RetryCycleFailure` reasons have one named public associated
  constructor with an exact signature callable from the monitor root
  boundary; the only operation input is the closed `RetryCycleOperation`,
  digest inputs are validated lowercase 64-hex, DTO/preimage fields remain
  private and observation remains read-only; the runtime recipe uses those
  exact names, including `from_panic_sha256` and `from_join_error_sha256`;
- every retry runner/finalizer/guard/boot/namespace operational failure uses
  the existing `durable_delivery::Result<T>`/`DurableDeliveryError` channel,
  definite no-commit and commit-ambiguous begin paths preserve their exact
  variants, guard contention/invariant use their exact variants, and
  compile/source plus runtime tests reject `Result<_, String>`,
  `.to_string()`/text `map_err` and stale constructor names in that boundary;
- the public root re-export compile contract covers the exact runtime/CLI
  command, verifier and prepare/append/claim/execute DTO list, and the sole
  atomic BR-192 root-export edit occurs only after all owning tasks have
  defined their symbols while preserving every unrelated existing export.
