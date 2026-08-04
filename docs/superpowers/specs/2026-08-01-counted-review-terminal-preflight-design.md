# Counted Review Durable Terminal Preflight Design

**Status:** Gate-A authority object; acceptance is recorded externally by an
exact-row/design/plan `Critical=0 / Important=0` review and their narrow commit.
This stable status never self-promotes to Gate B. Gate B/C/D and release evidence
remain pending until their independent gates pass. Existing dirty-worktree code
is candidate code only and cannot satisfy any implementation gate.

**Date:** 2026-08-01
**Rules:** BR-110, BR-140, BR-192, BR-194, BR-200

## 0. Intent and boundaries

Repeated `monitor --review` runs must not fetch a provider, render a second
payload, reserve another counted-delivery decision, or contact a sink when the
durable delivery authority already owns the same review-task occurrence.
Durable state, not later provider bytes or process-local cooldown state, decides
whether an occurrence is complete, terminally failed, awaiting reconciliation,
or absent.

BR-200 has exactly one live production consumer: R-04. It does not change A-10,
account authority, market-data validation, provider selection, delivery budgets,
or the BR-192 retry state machine. R-08 retains its durable `EventCalendar`
identity mapping, but BR-200 assigns it no production capability:
`review_task_production_capability(R08)` returns the typed
`ReviewTaskCapabilityError::UnsupportedTask(R08)` before dependency partition
and makes zero local-position, virtual-holding, Yahoo, provider, renderer,
new-decision, and sink calls. R-09 retains its durable
`ReviewProviderTopN` identity and SourceOnly classification but is
`DisabledNoProducer(NoProducer)` before partition with exact reason
`capability_unavailable:no_producer` and the same zero-call boundary. BR-200 does
not make BR-192 a prerequisite: a later accepted BR-192 slice may atomically
enable R-09 under its own design, tests, provider/catalog authorities, and
release gates. BR-200 neither assigns an enablement owner to R-08 nor claims a
future R-08 production transition. It exposes only a generic read-only
occurrence API plus isolated identity fixtures.

BR-200 Gate A may be reviewed against the factual fixed HEAD below, but Gate B
has an additional hard execution-base prerequisite: a literal
`BR194_GATE_C_SHA` from an independently accepted BR-194 Gate-C receipt. That
commit must descend from the fixed factual HEAD, pass the complete BR-194
Gate-C commands in a clean detached worktree, and be an ancestor of the BR-200
implementation base. Until that receipt and commit exist, BR-200 is **Blocked
before Gate B**. BR-200 does not absorb or repair BR-194 implementation debt in
`main.rs`, `notify.rs`, or `v14_adapter.rs`.

### 0.1 Reproducible fixed-HEAD facts

All current-code claims in this design come from literal commit
`b4aeee68d2c0259cc968914b3d39e3a89a18a496`, not from the dirty worktree. This
commit is factual evidence only; it is not the Gate-B execution base. The
following commands and outputs are the Gate-A evidence:

```bash
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/review_batch.rs \
  | nl -ba | sed -n '360,367p'
```

```text
360 pub fn dependency(self) -> ReviewTaskDependency {
361     match self {
362         Self::R04 | Self::R09 => ReviewTaskDependency::SourceOnly,
363         Self::R03 | Self::R08 => ReviewTaskDependency::LegacyAccountGate,
364         Self::R02 | Self::R05 | Self::R06 | Self::A10 | Self::A01 => {
365             ReviewTaskDependency::UnclassifiedConservative
366         }
367     }
```

Thus fixed HEAD does not have an R-08 SourceOnly consumer. Its actual body also
reads local positions, virtual holdings, and Yahoo data:

```bash
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/push_templates.rs \
  | nl -ba | sed -n '6386,6489p'
```

Selected literal output lines from that complete range are:

```text
6386 pub async fn dispatch_r08_event_calendar_outcome(
6400     let positions = load_verified_r08_positions(date);
6425     let real_holdings = positions.map(|positions| {
6448     let virtual_holdings =
6449         event_calendar_virtual_holdings().map_err(|error| format!("虚拟观察数据源失败: {error}"));
6450     let overnight = match tokio::task::spawn_blocking(
6451         stock_analysis::data_provider::yahoo::fetch_overnight_data,
6481     let push_result = dispatch_outcome(
6482         crate::notify::PushKind::EventCalendar,
6489     ReviewTaskOutcome::from_push_outcome(push_result, 1)
```

The complete range is the command output; the excerpt records the deciding
facts without treating worktree candidates as accepted evidence. BR-194 being
red or mentioning R-08 does not grant BR-200 a live R-08 producer.

The shared BR-194 checker is also frozen from that commit:

```bash
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:tools/compliance/lib/check_br194_review_dependency.sh \
  | nl -ba | sed -n '74,163p;165,342p'
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:tools/compliance/lib/check_br194_review_dependency.sh \
  | nl -ba | sed -n '590,656p'
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:tools/compliance/lib/check_br194_review_dependency.sh \
  | shasum -a 256
```

```text
74-78   callers = run_review_only, attempt_post_session_review,
        run_strict_review_only_inner
79-104  dispatcher, R-04 SourceOnly chain, replay/schema validators
106-123 preflight -> partition -> join -> account outcomes -> unique merge
124-163 R-04/R-09 presence, conservative-provider exclusions,
        dual test disable, duplicate/ordered merge, complete R-04 chain
165-291 replay, v5 schema/UDF, calendar, R-04 canonical/projection/text,
        independent verifier assertions
293-342 exact BR-194 test inventory, including dual test disable and replay/schema tests
590-656 fixed validator-mutation matrix and `expect_mutation_detected` execution
2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f  -
```

Gate B may only append BR-200 checks after this exact 659-line fixed-HEAD
checker. Before append, the first 659 current lines must be byte-identical to
the fixed-HEAD file and retain SHA-256
`2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f`;
line 660 is the exact append-boundary sentinel
`# BR-200 APPEND-ONLY CONTRACT START`. The reviewed Task-4 evidence freezes the
SHA-256 of every byte from line 660 through EOF. Gate B, Gate C and the forward
rollback verifier compare both prefix and append digests. The verifier owns the
reviewed append digest as a literal lowercase 64-hex constant; callers may not
supply or override it through an environment variable or argument. Before and
after applying the rollback patch it executes the unchanged BR-194 checker
self-test/mutation matrix and the appended BR-200 mutation matrix, and rejects
an early `exit`/`return`, boundary movement, reordering or rewriting. This
protects the complete three-
caller boundary, partition order, R-04 chain, dual-test-disable and unique merge
assertions, plus the replay/schema/verifier and mutation assertions whose full-
file identity is bound by the prefix digest above.

## 1. Data-red-line applicability

| Rule | Applicability | Gate-A decision |
| --- | --- | --- |
| 2.1 | Applies | `Some` durable evidence and every query error make zero provider/renderer/sink calls. Only exact `None` may continue to a real provider. No mock, cached, or fabricated occurrence is accepted. |
| 2.2 | Applies | Missing hydration stays missing and maps to the explicit retryable failure `durable_occurrence_delivered_hydration_pending`; it is never synthesized. |
| 2.3 | Preserved | BR-200 consumes no price series and weakens no upstream validation. Invalid hydration/canonical/hash/task evidence is a typed invariant failure. |
| 2.4 | Applies | Durable observation and source business date remain distinct. BR-200 never relabels an existing occurrence as fresh and never turns a retry into a new acquisition. |
| 2.5 | Applies | Tests use nonce-bound `TEST_CODE` durable databases, logs, providers, and sinks. Production cannot open test authority and tests cannot open production authority. |
| 2.6 | N/A | BR-200 cannot construct or execute an order. Existing order safeguards remain unchanged. |
| 2.7 | Applies | Every mapped durable occurrence writes the exact reason code, retryability, state, hashed identity, and ordered rule IDs into the existing review transition audit; the read query itself performs no write. |
| 2.8 | Applies | `inspect` reads and validates the real durable store. Reconciliation, hydration, and notification are never simulated by logging. |
| 2.9 | N/A | No `config/*.toml` field or threshold changes. |
| 2.10 | Applies | Exact occurrence matching, Delivered preference, ambiguity rejection, state mapping, and provider-before/after ordering are registered as BR-200 before Gate B. |

## 2. Occurrence identity and capture boundary

The sole live BR-200 consumer captures the occurrence once, before provider
access:

```text
ReviewRunContext.review_date()
+ ReviewTask::R04
+ ReviewTask::durable_push_kind()
+ DeliverySubKind::None
+ scope_key = "GLOBAL"
+ review_task_identity(business_date, task)
```

The core query is task-agnostic and separately exercises a synthetic
`TEST_CODE` `BusinessDateOnce` key whose fixture-only kind is
`PushKind::ReviewProviderTopN`. That enum value is constructed only by the test
fixture in this slice. It proves the future API contract; it is not a production
R-09 caller or producer.

`ReviewTask::durable_push_kind()` is added by this slice as one closed identity
mapping: R-04 maps to `ReviewLhb`, R-08 maps to `EventCalendar`, R-09 maps to
`ReviewProviderTopN`, and each of the other six explicit tasks returns typed
`ReviewTaskDurableKindError::UnsupportedTask(task)`. Identity is not producer
permission.

The separate capability API is closed and total over all nine task variants:

```rust
pub const fn review_task_production_capability(
    self,
) -> Result<ReviewTaskProductionCapability, ReviewTaskCapabilityError>
```

It maps R-04 to `EnabledSourceOnly` and R-09 to
`DisabledNoProducer(NoProducer)`. R-02, R-03, R-05, R-06, R-08, A-10, and A-01
each have their own explicit match arm returning
`Err(ReviewTaskCapabilityError::UnsupportedTask(task))`; `Option`, `_ => None`,
and wildcard matching are forbidden. `review_preflight` turns typed unsupported
R-08 into a terminal unsupported-task outcome and removes R-09 as
`capability_unavailable:no_producer`, both before dependency partition.
Table-driven exact tests freeze all nine identity/capability outcomes and prove
R-08 unsupported and R-09 disabled paths have zero provider, renderer,
new-decision, and sink calls.

The shared checker has exactly two closed R-08 profiles so BR-200 cannot become
a permanent blocker for the separately accepted BR-199 design:

1. `Br200Baseline`: R-08 retains `EventCalendar` identity and is typed
   `UnsupportedTask(R08)` before partition with zero downstream calls.
2. `Br199Enabled`: R-08 is accepted only when one atomic change contains the
   complete BR-199 SourceOnly authority set: `EnabledSourceOnly`, SourceOnly
   dependency classification, the exact EventCalendar durable identity, the
   four public provider batches and their binding/order validation, mandatory
   CFFEX capability handling, renderer, counted-delivery seam, and zero account,
   local-position, user-confirmed-position, or virtual-holding reads.

Capability-only, producer-only, partial-provider, mixed baseline/enabled, and
wildcard states fail the checker. BR-200 neither implements nor owns the
transition to `Br199Enabled`; it merely prevents its append-only checker from
forbidding a later fully accepted BR-199 implementation.

The effective `business_date` comes only from the review calendar. The
observation wall clock, provider response date, database write time, or a later
invocation may not replace it. The exact task identity is derived once and must
be reused by the durable query, hydration validation, transition audit, and any
later initial delivery.

The execution order is closed:

1. Capture `business_date`, `observed_at`, task, durable kind, and task identity.
2. Apply test isolation and the closed capability mapping. Unsupported R-08 and
   disabled R-09 stop before dependency partition; test mode remains disabled
   before durable or provider access.
3. Require the normal startup reconciliation barrier to have completed. The
   BR-200 query itself must not run reconciliation because reconciliation may
   contact a sink.
4. Execute the read-only durable occurrence query.
5. `Some(evidence)` or any query error terminates this invocation before permit,
   provider, renderer, new durable decision, or sink.
6. Only exact `None` for R-04 may enter its unchanged readiness/provider path.

R-04 may therefore reuse a prior Delivered occurrence even before the current
invocation's 21:00 provider-ready time. There is no BR-200 production occurrence
query or acquisition sequence for R-08 or R-09. BR-200 assigns no future R-08
owner. Later BR-192 work may add the R-09 consumer only by atomically changing
capability and adding all required provider/durable authorities; BR-192 is not a
prerequisite for accepting BR-200's generic read-only API.

## 3. Read-only durable interface

`DurableDeliveryCoordinator::inspect_review_task_occurrence` is the sole store
query and is **TO BE BUILT** from the fixed accepted baseline. It accepts the
exact five-part key:

```rust
pub fn inspect_review_task_occurrence(
    &self,
    business_date: &str,
    push_kind: PushKind,
    sub_kind: DeliverySubKind,
    scope_key: &str,
    task_identity: &str,
) -> Result<Option<ReviewTaskOccurrenceEvidence>>;
```

`ReviewTaskOccurrenceEvidence` contains the decision identity, exact
`DecisionState`, optional persisted `ScheduleHydration`, and the exact ordered
BR-200 prerequisite rule vector. It carries no provider, renderer, budget, or
sink capability.

The query validates business date, compiled policy, scope, envelope canonical
SHA-256, envelope identity fields, exact task binding, claim ownership, and
hydration identity/hash before returning. It runs under the existing descriptor-
attested SQLite read boundary and performs zero inserts, updates, deletes,
reservations, hydration application, reconciliation, audit append, or sink calls.

For `BusinessDateOnce`, the retained claim must resolve to the same matching
decision. For Rolling tasks, exact task identity is the occurrence key; one
Delivered decision wins over later denied duplicates. Multiple Delivered rows,
multiple non-Delivered matches without a unique owner, claim mismatch, canonical
hash mismatch, identity mismatch, or hydration mismatch is a typed
`ReviewTaskOccurrenceInvariant`, never a string-only provider failure.

## 4. Closed terminal mapping

The monitor maps durable evidence through one typed function used by the live
R-04 consumer. Core fixtures apply the same mapping contract to generic R-08 and
`BusinessDateOnce` identities without creating production consumers. The
mapping is exhaustive:

| Durable evidence | Review outcome | Retryable | Exact reason code | External calls |
| --- | --- | --- | --- | --- |
| `Delivered` + valid hydration | reuse original `Delivered(snapshot_size)` | false | original durable reason/no new failure | zero |
| `Delivered` + missing hydration | `Failed` | true | `durable_occurrence_delivered_hydration_pending` | zero; local reconciliation only |
| `RejectedDurable` or `ManualResolvedRejected` | `Failed` | false | `durable_occurrence_terminal_failure` | zero |
| `UncertainManualReview` | `Failed` | false | `durable_occurrence_terminal_failure` | zero; manual review remains authoritative |
| Any pending/non-terminal state | `Failed` | true | `durable_occurrence_nonterminal_reconciliation_pending` | zero; local reconciliation only |
| Corrupt, claim/task mismatch, hash mismatch, invalid hydration (including zero snapshot for a positive-snapshot task), multiple Delivered, or ambiguous owner | typed `Failed` invariant | false | `durable_occurrence_invariant_violation` plus closed invariant kind | zero |
| No occurrence | continue initial path | N/A | none | provider is permitted only here |

Pending states include `Reserved`, `AttemptInFlight`, every `*AuditPending`, and
every `*TaskTransitionPending`. A pending state whose name contains Rejected or
Uncertain is still non-terminal and uses the reconciliation-pending mapping.

Delivered hydration must bind the same decision, business date, task identity,
task label, canonical bytes, canonical SHA-256, and ordered rules. A generic
provider-verified-empty fixture may reuse `snapshot_size=0`; live R-04 and the
generic positive-snapshot `BusinessDateOnce` fixture treat zero as an invariant
violation. This does not grant R-08 production capability. Hydration is applied
to the in-memory schedule only after validation. The read query never queues or
applies unvalidated hydration.

The durable failure is a typed `ReviewTaskFailure::DurableOccurrence`, not an
`ExistingSourceFailure` string. Its audit projection contains:

```text
reason_code
retryable
decision_identity_hash
durable_state
invariant_kind (only for invariant failure)
rule_ids
```

Raw decision identities, canonical envelopes, provider payloads, account data,
and credentials must not enter logs, JSONL, or PR evidence.

## 5. Ordered rule authority

Every live R-04 BR-200 occurrence evidence/mapping/audit binds this vector
byte-for-byte and in this order:

```text
[BR-110, BR-140, BR-192, BR-194, BR-200]
```

Missing, duplicate, reordered, or additional entries fail as
`durable_occurrence_invariant_violation` before provider or sink. BR-198 is not
R-04 authority and must not appear in live R-04 evidence. A future R-09
producer, owned atomically by BR-192 rather than this slice, must bind
`[BR-110, BR-140, BR-192, BR-194, BR-198, BR-200]`. No BR-200 R-09 code or
fixture may make that future vector current production authority.

The shared evidence model therefore uses a validated variable-length ordered
type, not `[&str; 5]`. `OrderedRuleIds` owns exact persisted strings, rejects an
empty list, non-`BR-NNN` values, duplicates, trimming/normalization, and order
drift, and preserves SQLite BINARY order. A task policy supplies its exact
expected ordered slice: five rules for live R-04 and six only for a future
BR-192-owned R-09 implementation. The generic API must not require a breaking
model change when a task has a different valid authority length.

## 6. Initial acquisition versus retry expiry

BR-200 distinguishes absence from retry without owning retry time:

- Exact `None` means no durable occurrence exists for the supplied generic key.
  Only live R-04 may then continue its existing initial acquisition path.
- Once any durable occurrence exists, BR-200 never falls through to a new
  provider acquisition. Rejected and uncertain occurrences are terminal for
  this invocation; pending states and missing Delivered hydration wait only for
  local reconciliation.
- BR-192 alone decides whether an explicitly authorized provider-free retry is
  still eligible. BR-200 does not read, authorize, extend, reset, or revive
  retry expiry and cannot call a provider during retry.
- Delivered reuse is terminal and independent of retry expiry.
- Generic EventCalendar-identity and `BusinessDateOnce` fixtures prove that absence returns `None` while
  a retained rejection returns typed evidence. It does not perform an initial
  provider acquisition. BR-200 assigns no owner to future R-08 behavior; future
  R-09 initial-versus-retry behavior belongs to BR-192.

## 7. Failure and concurrency semantics

- Startup barrier unavailable or runtime join failure is an explicit retryable
  local-runtime failure and makes zero provider/renderer/sink calls.
- Unsupported task/kind, invalid scope, or invalid identity is a non-retryable
  typed contract failure.
- Store corruption, ambiguity, claim mismatch, envelope mismatch, hydration
  mismatch, or rule-vector mismatch is a non-retryable typed invariant.
- Concurrent insert after an initial `None` remains governed by BR-192's durable
  prepare/claim transaction. BR-200 does not weaken the authoritative uniqueness
  constraints; a losing invocation must not contact a sink.
- Query/read failures are never converted to `None`.

## 8. Planned Gate-B paths

The following are implementation targets, not current accepted code claims:

| Path | Action | Responsibility |
| --- | --- | --- |
| `src/durable_delivery/model.rs` | modify | Typed occurrence evidence, invariant kind, and durable error. |
| `src/durable_delivery/coordinator.rs` | modify | Exact read-only occurrence query and validation. |
| `src/durable_delivery/mod.rs` | modify | Export only the frozen typed read contract. |
| `src/bin/monitor/durable_delivery_runtime.rs` | modify | Startup-barrier check, `spawn_blocking`, typed error preservation; no reconciliation or hydration application. |
| `src/bin/monitor/review_batch.rs` | modify | Typed durable-occurrence failure, exact reason/audit projection, task-specific rule authority, closed durable-kind/capability maps, unsupported R-08 and disabled R-09 preflight. |
| `src/bin/monitor/push_templates.rs` | modify | Shared mapping, sole live R-04 pre-provider seam, unsupported R-08/disabled R-09 proofs, and inline injected-counter ordering/initial-versus-retry tests. |
| `src/bin/monitor/main.rs` | modify | Install the production R-09 no-producer startup banner at exactly one pre-scheduler call site. |
| `src/durable_delivery/tests.rs` | modify | Coordinator read-only, claim, ambiguity, corruption, and rule-order tests. |
| `tests/monitor_help_isolation.rs` | modify | Nonce-bound process proof that the exact R-09 banner is emitted once and no R-09 external call occurs. |
| `tools/compliance/lib/check_br194_review_dependency.sh` | additive modify only | Preserve the exact 659-line fixed prefix and append BR-200 R-04, closed BR-200-baseline/BR-199-enabled R-08 profiles, and disabled R-09 enforcement after the hashed boundary. |
| `tools/release/disable_br200_review_consumers.patch` | create | Reviewed forward rollback that disables only the live R-04 BR-200 consumer without restoring provider-first acquisition. |
| `tools/release/verify_br200_forward_rollback.sh` | create | Own literal checker prefix/append digests; verify both mutation matrices before and after release-SHA-isolated patch apply/build/TEST_CODE zero-external-call proof. |
| `tools/release/verify_br200_repeated_review.py` | create | Capture pre-first/after-first/after-second watermarks and causally bind authentic R-04 provider, decision, occurrence, delivery, audit, zero-call reuse, and exact-once R-09 banner evidence. |

No schema migration, provider implementation, notification implementation,
configuration file, or account module belongs to BR-200.

## 9. Old modules

| Module | Decision | Reason |
| --- | --- | --- |
| Accepted `BR194_GATE_C_SHA` execution base | adopt as hard Gate-B prerequisite | BR-200 may build only after BR-194 Gate C is independently accepted and revalidated from its exact committed tree. |
| BR-192 coordinator, claims, hydration, and counted-delivery store | adopt | They remain the sole durable decision and retry authorities. |
| `ReviewRunContext` and `review_task_identity` | adopt | They freeze the business-date occurrence boundary. |
| R-04 provider loader | retain behind BR-200 | It is the sole live consumer and runs only after exact `None`. |
| R-08 legacy account/event-calendar loader | reject as unsupported | Preserve only its `EventCalendar` durable identity mapping; fixed HEAD is `LegacyAccountGate` and reads local positions, virtual holdings, and Yahoo, so typed `UnsupportedTask(R08)` fails before partition with zero downstream calls. BR-200 assigns no future enablement owner. |
| R-09 production dispatcher/gateway/provider | reject from BR-200 | Preserve identity and SourceOnly classification but fail closed as `DisabledNoProducer(NoProducer)`; later BR-192 may atomically enable the generic accepted API with its provider/catalog gates and is not a prerequisite. |
| Process-local cooldown/dedup | reject as occurrence authority | It cannot prove a prior process's decision or hydration. |
| Startup reconciliation inside BR-200 query | reject | It can resume a sink and violates the read-only pre-provider boundary. |

## 10. Gate progression and evidence

Gate A requires the exact design, plan, and registered BR-200 row plus an
independent `Critical=0 / Important=0` review. Gate B implements only the paths
listed above with genuine RED/GREEN tests. Gate C requires format, strict Clippy,
all targets/all features tests, focused compliance, and repository compliance.
Gate D additionally requires isolated coverage authority and real repeated-review
evidence.

Gate B first compares the canonical 26-name manifest against both source
declarations and the names actually reported by Cargo's `--list` harnesses for
the library, monitor binary, counted-cutover target, and monitor process target.
It then executes every reviewed exact command as a closed argv vector using the
resolved Cargo binary, never `bash -c`, `bash -lc`, or `eval`. Each command must
report the exact selected qualified test as `ok` and one exact summary with
`1 passed; 0 failed; 0 ignored; 0 measured`; source text, comments, duplicate
declarations, unregistered helpers, ignored/filtered-only tests, or zero-test
success cannot satisfy this gate.

A real Gate-D replay must run the repository-owned causal verifier in a
controlled single-owner window. It captures pre-first watermarks, derives the
business date from the admitted `ReviewRunContext` authority, proves the first
run used an authentic provider and created exactly one new decision/occurrence/
delivery/audit chain, then proves the second run reused the same hashed
occurrence with provider, renderer, new-decision, and sink counters all zero and
no new durable delivery records. The exact R-09 no-producer banner must occur
once per process. Same-day aggregate counts or old log files cannot satisfy
Gate D.

Coverage authority uses the repository baseline commands documented in
`docs/ENGINEERING_RULES_V2.md`: workspace/all-features `cargo llvm-cov` JSON
followed by `tools/coverage/check_thresholds.py`. BR-202 is not a BR-200 gate
dependency.

If no authentic durable occurrence exists, it must not be manufactured; Gate D
remains in progress.

## 11. PR evidence

```text
Refs: BR-200 design §0-12 and implementation plan
Data-Redlines: [2.1,2.2,2.3,2.4,2.5,2.6,2.7,2.8,2.9,2.10]
OldModules:
| module | adopt/reject | reason |
| BR-192 coordinator/claims/hydration/store | adopt | sole durable decision and retry authorities |
| ReviewRunContext/review_task_identity | adopt | canonical review-date occurrence boundary |
| R-04 provider loader | adopt behind BR-200 | sole live consumer; execute only after exact None |
| R-08 legacy account/event-calendar loader | reject as unsupported | retain only EventCalendar identity mapping; typed UnsupportedTask(R08) before partition with zero downstream calls; BR-200 assigns no future enablement owner |
| R-09 provider/gateway/renderer/sink | reject from BR-200 | retain identity; disabled as NoProducer before partition; BR-192 later owns atomic enablement but is not prerequisite |
| process-local cooldown/dedup | reject | cannot prove cross-process durable occurrence |
| reconciliation inside BR-200 query | reject | may resume sink and violates read-only pre-provider boundary |
Threshold-Proof: no config or financial threshold change
Business-Rules: BR-110, BR-140, BR-192, BR-194, BR-200
Rollback: design §12
Validation: exact Gate B/C/D commands and raw outputs
```

The PR stays Draft while Gate D or an independent Critical/Important finding is
open.

## 12. Actionable forward rollback

BR-200 spans multiple reviewed implementation commits, so reverting one commit
is not a valid rollback. Gate B creates
`tools/release/disable_br200_review_consumers.patch`; Gate C applies it to an
isolated worktree fixed at the exact accepted release SHA. The patch has one
`diff --git` target, `src/bin/monitor/push_templates.rs`, and changes only the
R-04 live consumer installation to an explicit typed
`br200_rollback_consumer_disabled` outcome before provider, renderer, new
decision, or sink access. The release checker already recognizes and validates
this fail-closed rollback form. It must not edit the durable model/query,
schema, audit, decisions, hydration, BR-194 replay, R-08/R-09 identities or
R-08 unsupported/R-09 disabled capabilities, dependency catalog, or provider
implementations.

Because R-09 remains a spec-only no-producer PushKind in BR-200, startup must
emit exactly once `[BR-200][R-09] disabled=no_producer reason=capability_unavailable:no_producer`.
Silence, a provider call, or a duplicate banner is a release blocker. The
appended checker also owns the cross-version debt proof: R-04's active marker
must have a real caller, R-08 is active only under the complete separately
accepted `Br199Enabled` profile, R-09 may not be mislabeled active by this
slice, and each retained legacy marker must have an explicit deletion plan.

Task 4 stages only its declared eight-path allowlist, freezes the complete index
with `git write-tree`, creates a non-moving candidate commit with `git
commit-tree`, and runs the forward verifier against that literal candidate SHA.
The Task-4 operator starts from a clean index byte-identical to `HEAD`, then
stages and proves that the complete cached-diff name set is byte-for-byte the
eight-path allowlist before `git write-tree`; an
unrelated pre-staged path is a hard failure rather than part of the candidate.
The pre-Task-4 branch `HEAD` is never a verifier input. The reviewed Task-4
commit must have a tree byte-identical to the candidate tree and is then
verified again by literal committed `HEAD`. Gate C accepts only that committed
tree.

The accepted-release fixture must pass `git apply --check`, apply the patch,
prove the exact one-file diff and the R-04 disabled consumer marker, build the
rollback monitor, and run a nonce-bound `TEST_CODE` R-04 process test that
observes zero provider/renderer/decision/sink calls. Failure of any check blocks
Gate C. The verifier embeds the reviewed fixed-prefix and append SHA-256 values,
checks the 659/660 boundary, runs both checker mutation matrices, and requires
their exact PASS records before and after patch application; no caller-provided
digest is accepted. An operator then creates a rollback branch from the literal accepted
release SHA, applies the same reviewed patch, reruns format/Clippy/tests/
compliance/release build, and commits the forward rollback. Existing durable
evidence is never deleted or relabelled as a new initial acquisition. R-08 and
R-09 durable identities, typed R-08 unsupported state, and the exact R-09
disabled reason remain byte-identical.
