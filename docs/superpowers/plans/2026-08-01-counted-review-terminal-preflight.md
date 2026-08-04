# Counted Review Durable Terminal Preflight Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the sole live R-04 consumer reuse or fail closed from an exact
existing durable review occurrence before provider, renderer, new decision, or
sink access; retain R-08/R-09 durable identities while treating R-08 as typed
unsupported and R-09 as typed disabled, and expose a generic fixture-tested API.

**Architecture:** A generic read-only `DurableDeliveryCoordinator` query
validates the exact review occurrence and returns typed evidence. Live R-04
preserves typed invariant failures, maps all durable states through one closed
transition table, and permits initial acquisition only on exact `None`. R-08 is
typed `UnsupportedTask(R08)` and R-09 is `DisabledNoProducer(NoProducer)` before
partition. Synthetic `TEST_CODE` EventCalendar-identity and `BusinessDateOnce`
fixtures prove the generic contract without adding a production consumer.
BR-200 assigns no future R-08 enablement owner. A later BR-192 slice may
atomically enable R-09; BR-192 is not a BR-200 prerequisite.

The shared append-only checker must accept exactly two closed R-08 profiles:
the BR-200 baseline (`EventCalendar` identity plus typed
`UnsupportedTask(R08)` before partition), and a separately accepted BR-199
enabled profile containing the complete SourceOnly dependency, four-public-batch
provider binding, mandatory CFFEX capability, renderer, counted-delivery, and
zero-account-read authorities. Partial or mixed profiles fail. BR-200 does not
implement or own the BR-199 transition.

**Tech Stack:** Rust, Tokio `spawn_blocking`, rusqlite/SQLite durable delivery, chrono, serde, existing review scheduler/audit, shell compliance checks.

**Status:** Gate-A authority object; acceptance is external exact-row/design/
plan review plus a narrow commit. This stable text does not claim Gate B. Gate
B/C/D and release evidence remain pending; dirty-worktree candidate code is not
implementation evidence.

---

## Mandatory implementer brief

**Upstream debt**

- Fixed baseline `b4aeee68d2c0259cc968914b3d39e3a89a18a496` has BR-192 durable
  decisions/claims/hydration but no `inspect_review_task_occurrence`, no typed
  BR-200 mapping, and no provider-before-preflight enforcement.
- R-04 can reacquire after an earlier durable terminal. R-08 is
  `LegacyAccountGate` in fixed HEAD and its body reads local positions, virtual
  holdings and Yahoo; BR-200 therefore rejects it as typed unsupported before
  partition and assigns no future enablement owner.
  Fixed HEAD has no accepted R-09 producer. BR-200 provides a generic read-only
  query/fixture contract and a disabled R-09 identity for later BR-192 adoption.
- Current worktree BR-200 symbols are unreviewed candidate bytes. Execution must
  start from the accepted Gate-A commit in a clean isolated worktree and conform
  code to this plan rather than treating candidate existence as proof.
- The fixed baseline is factual evidence only. BR-200 Gate B additionally
  requires a literal `BR194_GATE_C_SHA` from an independently accepted BR-194
  Gate-C receipt. It must descend from the fixed baseline, pass BR-194 Gate C
  again in a clean detached worktree, and be an ancestor of the BR-200
  implementation base. Until that exists, BR-200 remains blocked before Gate B;
  this slice must not repair BR-194 debt in `main.rs`, `notify.rs`, or
  `v14_adapter.rs`.

**Rename impact**

- No existing public identifier is renamed.
- Add the typed `ReviewTaskOccurrenceEvidence`,
  `ReviewTaskOccurrenceInvariant`, `DurableOccurrenceFailureReasonCode`, and
  `ReviewTaskFailure::DurableOccurrence` contracts atomically with all matches,
  serialization tests, runtime adapters, and root exports.
- Any candidate string-only `review_outcome_from_existing_durable` behavior must
  move atomically to the typed mapping; no dual string/typed path remains.

**Production evidence**

- A repeated real R-04 review must emit a BR-200 reuse transition with
  exact hashed occurrence identity and `provider_calls=0`, `renderer_calls=0`,
  `sink_calls=0` on the second invocation.
- Review audit authority is `data/review_audit/YYYY-MM-DD.jsonl`; delivery audit
  remains `data/event_bus/YYYY-MM-DD.jsonl` with the existing
  `push.delivery.audit` event. No new delivery event is allowed on reuse.
- R-09 has no BR-200 producer, so startup must emit exactly once
  `[BR-200][R-09] disabled=no_producer reason=capability_unavailable:no_producer`;
  silence or duplicates are defects.
- BR-200 production evidence is limited to an authentic R-04 repeated-review
  run. R-08 must emit a typed unsupported outcome and R-09 its exact disabled
  outcome before partition, both with zero provider/renderer/new-decision/sink
  calls. Generic fixtures cannot manufacture production evidence, and BR-192 is
  not a BR-200 gate dependency.

## Reproducible fixed-HEAD evidence

Run these commands against literal factual baseline
`b4aeee68d2c0259cc968914b3d39e3a89a18a496`; dirty worktree bytes are not
evidence:

```bash
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/review_batch.rs \
  | nl -ba | sed -n '360,367p'
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/push_templates.rs \
  | nl -ba | sed -n '6386,6489p'
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:tools/compliance/lib/check_br194_review_dependency.sh \
  | nl -ba | sed -n '74,163p;165,342p'
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:tools/compliance/lib/check_br194_review_dependency.sh \
  | nl -ba | sed -n '590,656p'
git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:tools/compliance/lib/check_br194_review_dependency.sh \
  | shasum -a 256
```

Observed output is:

```text
review_batch.rs 360-367:
  R04|R09 -> SourceOnly
  R03|R08 -> LegacyAccountGate

push_templates.rs 6386-6489:
  6386 dispatch_r08_event_calendar_outcome
  6400 load_verified_r08_positions(date)
  6448-6449 event_calendar_virtual_holdings()
  6450-6452 spawn_blocking(yahoo::fetch_overnight_data)
  6481-6489 EventCalendar dispatch and outcome

checker 74-163/165-342:
  three review callers; preflight -> partition -> join -> account -> unique merge;
  complete R04 SourceOnly chain; dual test disable; replay/v5 schema/UDF,
  canonical/projection/text, independent verifier, and exact test assertions
checker 590-656:
  fixed mutation matrix executed through expect_mutation_detected
2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f  -
```

BR-194 being red does not grant R-08 a live producer. Gate B may append BR-200
assertions only after the 659-line checker. The first 659 current lines must be
byte-identical to fixed HEAD and retain the digest above; exact line 660 is
`# BR-200 APPEND-ONLY CONTRACT START`. Task 4 freezes a separate reviewed
SHA-256 over line 660 through EOF. Prefix and append digests, the unchanged
BR-194 self-test/mutation matrix, and the appended BR-200 mutation matrix must
all execute. Any early `exit`/`return`, boundary movement, reordering or rewrite
fails.

## Frozen mapping and rule vector

All implementation tasks use this exhaustive mapping:

| State/evidence | Outcome | Retryable | Reason code |
| --- | --- | --- | --- |
| Delivered + valid hydration | original Delivered count | false | no new failure |
| Delivered + missing hydration | Failed | true | `durable_occurrence_delivered_hydration_pending` |
| RejectedDurable / ManualResolvedRejected / UncertainManualReview | Failed | false | `durable_occurrence_terminal_failure` |
| Any pending/non-terminal state | Failed | true | `durable_occurrence_nonterminal_reconciliation_pending` |
| Corrupt/mismatch/ambiguous/invalid hydration or rule vector | typed Failed invariant | false | `durable_occurrence_invariant_violation` |
| None | continue initial acquisition | N/A | none |

The exact ordered live R-04 rule vector is:

```rust
pub const BR200_R04_ORDERED_RULE_IDS: &[&str] =
    &["BR-110", "BR-140", "BR-192", "BR-194", "BR-200"];
```

BR-198 is not live R-04 authority. A future R-09 vector may be
`[BR-110, BR-140, BR-192, BR-194, BR-198, BR-200]`, but only BR-192 may create
and enable that R-09 authority; BR-200 does not define it as current production
state. The evidence model uses validated variable-length `OrderedRuleIds`, not
an array whose length is fixed to R-04. It owns exact persisted strings,
preserves BINARY order, and rejects empty, normalized, duplicate, malformed, or
reordered values before task-policy comparison.

## Complete file ownership

| Task | Action | Path |
| --- | --- | --- |
| 1 | Modify | `src/durable_delivery/model.rs` |
| 1 | Modify | `src/durable_delivery/coordinator.rs` |
| 1 | Modify | `src/durable_delivery/mod.rs` |
| 1 | Modify | `src/durable_delivery/tests.rs` |
| 2 | Modify | `src/bin/monitor/durable_delivery_runtime.rs` |
| 3 | Modify | `src/bin/monitor/review_batch.rs` |
| 3 | Modify | `src/bin/monitor/push_templates.rs` |
| 4 | Modify | `src/bin/monitor/review_batch.rs` |
| 4 | Modify | `src/bin/monitor/push_templates.rs` |
| 4 | Modify | `src/bin/monitor/main.rs` |
| 4 | Modify | `src/durable_delivery/tests.rs` |
| 4 | Modify | `tests/monitor_help_isolation.rs` |
| 4 | Additive modify only | `tools/compliance/lib/check_br194_review_dependency.sh` |
| 4 | Create | `tools/release/disable_br200_review_consumers.patch` |
| 4 | Create | `tools/release/verify_br200_forward_rollback.sh` |
| 5 | Create | `tools/release/verify_br200_repeated_review.py` |

No schema, provider, notification, account, configuration, BR-192/BR-198/BR-202
document or business-rule registry edit belongs to this implementation slice.

### Task 0: Prove Gate-A authority before code

**Files:**

- Verify: `docs/superpowers/specs/2026-08-01-counted-review-terminal-preflight-design.md`
- Verify: `docs/superpowers/plans/2026-08-01-counted-review-terminal-preflight.md`
- Verify: `docs/business_rules.md`

- [ ] **Step 1: Refuse untracked or dirty Gate-A inputs**

```bash
git ls-files --error-unmatch \
  docs/superpowers/specs/2026-08-01-counted-review-terminal-preflight-design.md \
  docs/superpowers/plans/2026-08-01-counted-review-terminal-preflight.md \
  docs/business_rules.md
test -z "$(git status --porcelain)"
test "$(rg -c '^\| BR-200 \|' docs/business_rules.md)" -eq 1
rg -q '^\| BR-200 \| .*Gate-A contract object' docs/business_rules.md
git diff --check
```

Expected: all three paths are tracked, the worktree is clean, exactly one
BR-200 canonical row exists with stable contract-object status, and whitespace
validation exits 0. Lifecycle acceptance is never self-asserted by editing this
row.

- [ ] **Step 2: Record immutable execution base**

```bash
BR200_BASE_SHA=$(git rev-parse HEAD)
test -n "$BR200_BASE_SHA"
git show "$BR200_BASE_SHA:docs/superpowers/specs/2026-08-01-counted-review-terminal-preflight-design.md" >/dev/null
git show "$BR200_BASE_SHA:docs/superpowers/plans/2026-08-01-counted-review-terminal-preflight.md" >/dev/null
```

Expected: both documents resolve from the same accepted commit. Record
`BR200_BASE_SHA` in the PR.

- [ ] **Step 3: Bind exact Gate-A objects and prohibit unrelated rule edits**

The independent receipt records SHA-256 for the committed design blob, plan
blob, and the exact UTF-8 BR-200 row including one trailing LF. The row must be
unique. The accepted canonical row digest is
`432dfcb6b6a8047c0afc6c2e5e7d03a9ddb15858726c6e9751a3c47b30ef6885`;
Gate A recomputes it from the committed row and rejects drift. The narrow
Gate-A commit may change only these three paths, and every
added/removed Markdown table row in `docs/business_rules.md` must start with
`| BR-200 |`; any BR-194, BR-199, or other rule-row delta fails. Gate B may not
alter these accepted bytes.

- [ ] **Step 4: Pin and revalidate the BR-194 Gate-C execution base**

```bash
set -euo pipefail
: "${BR194_GATE_C_SHA:?copy the literal SHA from the accepted BR-194 Gate-C receipt}"
test "$(git rev-parse "${BR194_GATE_C_SHA}^{commit}")" = "${BR194_GATE_C_SHA}"
git merge-base --is-ancestor \
  b4aeee68d2c0259cc968914b3d39e3a89a18a496 \
  "${BR194_GATE_C_SHA}"
```

In a clean detached worktree at that SHA, rerun the complete BR-194 Gate-C
commands and bind their raw output plus the BR-194 C0/I0 receipt. The BR-200
implementation base must contain the accepted Gate-A objects and descend from
`BR194_GATE_C_SHA`. If either condition is absent, stop before Task 1.

- [ ] **Step 5: Require independent Gate-A review**

The reviewer brief begins with:

> **DO NOT trust the implementer's report as ground truth.** Independently
> inspect the exact design/plan/BR-200 row objects, check every named exact test
> has one declaration, and reject any circular prerequisite or unowned file.

Expected: `Critical=0 / Important=0`. Any finding returns to Gate A; no source
file is edited.

### Task 1: Add typed read-only durable occurrence authority

**Files:**

- Modify: `src/durable_delivery/model.rs`
- Modify: `src/durable_delivery/coordinator.rs`
- Modify: `src/durable_delivery/mod.rs`
- Modify: `src/durable_delivery/tests.rs`

- [ ] **Step 1: Add genuine RED coordinator tests**

Declare each name exactly once in `src/durable_delivery/tests.rs`. Bodies use the
existing nonce-bound `Fixture`, task-bound envelopes, retained claims, and
database row-count helpers. They assert returned values and before/after counts;
they must not use `todo!`, unconditional `panic!`, or a zero-test sentinel.

| Exact test declaration | Required RED body |
| --- | --- |
| `fn br200_business_date_once_fixture_reuses_delivered_without_writes()` | Create one exact synthetic `TEST_CODE` `BusinessDateOnce` Delivered claim/hydration, snapshot every durable table count, inspect, compare the returned evidence field-by-field, then assert every count is unchanged. The fixture constructs no production consumer. |
| `fn br200_r04_rolling_preflight_prefers_original_delivered_over_later_denial()` | Insert one valid Delivered Rolling occurrence and one later rejected duplicate, inspect once, require the original Delivered identity/hydration, and assert zero writes. |
| `fn br200_occurrence_query_rejects_claim_mismatch_as_typed_invariant_without_writes()` | Bind the retained claim to a different task identity, require `ClaimTaskMismatch`, and compare every durable count before/after. |
| `fn br200_occurrence_query_rejects_hash_identity_and_multiple_delivered_ambiguity_without_writes()` | Exercise canonical-hash mismatch, decision-identity mismatch, and two Delivered rows as independent fixture cases; require the exact closed invariant for each and zero writes. |
| `fn br200_occurrence_query_preserves_exact_ordered_rule_ids()` | Inspect a valid live R-04 occurrence and compare the validated ordered value byte-for-byte with `BR200_R04_ORDERED_RULE_IDS`; reject BR-198 in that vector and independently exercise a valid six-rule policy without changing the evidence type. |

Required assertions:

- The generic `BusinessDateOnce` fixture resolves only the exact claim and
  returns its Delivered hydration without changing counts in decisions, claims,
  schedules, attempts, results, or audit outboxes.
- R-04 Rolling selects the sole Delivered row over a later rejected duplicate.
- Claim/task mismatch, canonical hash/identity mismatch, multiple Delivered, and
  ambiguous non-Delivered matches return exact typed invariant kinds and perform
  zero writes.
- Returned live R-04 evidence binds `BR200_R04_ORDERED_RULE_IDS` exactly.

- [ ] **Step 2: Run RED commands**

```bash
cargo test --lib durable_delivery::tests::br200_business_date_once_fixture_reuses_delivered_without_writes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br200_r04_rolling_preflight_prefers_original_delivered_over_later_denial -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br200_occurrence_query_rejects_claim_mismatch_as_typed_invariant_without_writes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br200_occurrence_query_rejects_hash_identity_and_multiple_delivered_ambiguity_without_writes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br200_occurrence_query_preserves_exact_ordered_rule_ids -- --exact --test-threads=1
```

Expected: every command selects exactly one test and fails only because the
typed query/evidence/invariant API is absent or incomplete.

- [ ] **Step 3: Implement the minimal typed model**

Add these closed contracts to `src/durable_delivery/model.rs` and export them
from `src/durable_delivery/mod.rs`:

```rust
pub const BR200_R04_ORDERED_RULE_IDS: &[&str] =
    &["BR-110", "BR-140", "BR-192", "BR-194", "BR-200"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderedRuleIds(Box<[String]>);

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTaskOccurrenceInvariant {
    ClaimTaskMismatch,
    EnvelopeHashMismatch,
    EnvelopeIdentityMismatch,
    TaskBindingMismatch,
    MultipleDelivered,
    AmbiguousNonDelivered,
    HydrationIdentityMismatch,
    HydrationHashMismatch,
    InvalidHydrationSnapshotSize,
    RuleIdsMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewTaskOccurrenceEvidence {
    pub decision_identity: String,
    pub state: DecisionState,
    pub schedule_hydration: Option<ScheduleHydration>,
    pub rule_ids: OrderedRuleIds,
}
```

`OrderedRuleIds::try_from_persisted` must reject an empty vector, any value
outside the closed `BR-NNN` grammar, duplicates, trimming/normalization, and
order drift while retaining exact persisted BINARY order. A
`ReviewTaskOccurrencePolicy` supplies each task's expected ordered slice, so
live R-04 validates five rules and a future BR-192-owned R-09 may validate six
without changing the evidence ABI.

Add a typed `DurableDeliveryError::ReviewTaskOccurrenceInvariant` variant that
contains the closed invariant kind and a domain-separated identity hash, never
raw envelope bytes.

- [ ] **Step 4: Implement the read-only coordinator query**

Implement the exact five-argument signature frozen in design §3. Under the
existing descriptor-attested connection it must:

1. validate business date, compiled policy, sub-kind, and exact scope;
2. read the retained BusinessDateOnce claim when applicable;
3. load task-bound candidate decisions in BINARY decision-identity order;
4. recompute envelope canonical SHA-256 and validate every identity field;
5. validate exact task binding and any returned hydration identity/hash;
6. select exact claim, sole Delivered, unique non-Delivered, or `None` according
   to design §3;
7. return the typed invariant for every mismatch/ambiguity; and
8. perform no DML, reconciliation, hydration application, append, or sink call.

- [ ] **Step 5: Run GREEN and regression tests**

Run the five exact commands from Step 2, then:

```bash
cargo test --lib durable_delivery::tests -- --test-threads=1
cargo build --lib
```

Expected: non-zero test counts, all pass.

- [ ] **Step 6: Commit Task 1**

```bash
git add src/durable_delivery/model.rs src/durable_delivery/coordinator.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs
git diff --cached --check
git commit -m "feat: add BR-200 durable occurrence query"
```

### Task 2: Preserve the read-only runtime boundary

**Files:**

- Modify: `src/bin/monitor/durable_delivery_runtime.rs`

- [ ] **Step 1: Add genuine RED runtime tests**

Declare exactly once inside `durable_delivery_runtime::tests`:

| Exact test declaration | Required RED body |
| --- | --- |
| `async fn br200_runtime_preflight_requires_completed_startup_barrier_without_reconciliation_or_sink()` | Start with `producer_ready=false`; invoke the adapter; require the typed retryable runtime failure and zero reconcile, append, hydration-queue, provider, renderer, and sink counters. |
| `async fn br200_runtime_preflight_preserves_typed_invariant_and_never_queues_unvalidated_hydration()` | Inject one coordinator invariant and one invalid-hydration case; require the exact typed variants across `spawn_blocking`, with hydration-queue and sink counters remaining zero. |
| `async fn br200_runtime_preflight_accepts_live_r04_and_fixture_event_calendar_business_date_once_keys()` | Table-drive live R-04 plus synthetic `TEST_CODE` EventCalendar-identity and `BusinessDateOnce` keys; require the same generic read-only behavior and reject unsupported production tasks with the typed contract. Assert fixtures create no production caller, write, hydration queue, or sink side effect. |

The first fixture starts with `producer_ready=false` and asserts no coordinator
reconcile, append, hydration queue, provider, or sink call. The second injects a
typed coordinator invariant and invalid hydration and asserts the same typed
error crosses `spawn_blocking` without string downgrade. The third proves the
runtime adapter is a generic read-only seam for one live key and isolated
EventCalendar-identity/`BusinessDateOnce` fixtures; it does not add a production
consumer or production R-08 capability.

- [ ] **Step 2: Run RED commands**

```bash
cargo test --bin monitor durable_delivery_runtime::tests::br200_runtime_preflight_requires_completed_startup_barrier_without_reconciliation_or_sink -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br200_runtime_preflight_preserves_typed_invariant_and_never_queues_unvalidated_hydration -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br200_runtime_preflight_accepts_live_r04_and_fixture_event_calendar_business_date_once_keys -- --exact --test-threads=1
```

Expected: each runs one test and fails because the typed runtime seam is absent.

- [ ] **Step 3: Implement the runtime adapter**

Add one `inspect_review_task_occurrence` adapter. It checks the completed startup
barrier but never calls startup reconciliation, runs only the coordinator query
inside `spawn_blocking`, preserves typed invariant/runtime errors, and returns
raw validated evidence. It must not queue hydration; Task 3 applies hydration
only after Delivered validation.

- [ ] **Step 4: Run GREEN commands and commit**

Run the three exact commands from Step 2, then:

```bash
cargo test --bin monitor durable_delivery_runtime::tests -- --test-threads=1
git add src/bin/monitor/durable_delivery_runtime.rs
git diff --cached --check
git commit -m "feat: add BR-200 read-only runtime preflight"
```

### Task 3: Add typed mapping and review-audit projection

**Files:**

- Modify: `src/bin/monitor/review_batch.rs`
- Modify: `src/bin/monitor/push_templates.rs`

- [ ] **Step 1: Add the closed typed failure contract**

In `review_batch.rs`, define:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableOccurrenceFailureReasonCode {
    DeliveredHydrationPending,
    TerminalFailure,
    NonterminalReconciliationPending,
    InvariantViolation,
}

impl DurableOccurrenceFailureReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeliveredHydrationPending => "durable_occurrence_delivered_hydration_pending",
            Self::TerminalFailure => "durable_occurrence_terminal_failure",
            Self::NonterminalReconciliationPending => "durable_occurrence_nonterminal_reconciliation_pending",
            Self::InvariantViolation => "durable_occurrence_invariant_violation",
        }
    }
}
```

Add `ReviewTaskFailure::DurableOccurrence` and the matching
`ReviewTransitionFailure::DurableOccurrence` with exact reason code,
retryability, durable state, hashed decision identity, optional closed invariant
kind, and exact rule vector. Serialization uses `deny_unknown_fields`; raw
decision identity is never serialized.

Add the sole task/kind and capability authorities in `review_batch.rs`:

```rust
pub enum ReviewTaskDurableKindError {
    UnsupportedTask(ReviewTask),
}

pub enum ReviewTaskProductionCapability {
    EnabledSourceOnly,
    DisabledNoProducer(ReviewTaskNoProducerReason),
}

pub enum ReviewTaskNoProducerReason {
    NoProducer,
}

pub enum ReviewTaskCapabilityError {
    UnsupportedTask(ReviewTask),
}

impl ReviewTask {
    pub const fn durable_push_kind(
        self,
    ) -> Result<stock_analysis::durable_delivery::PushKind, ReviewTaskDurableKindError> {
        match self {
            Self::R04 => Ok(stock_analysis::durable_delivery::PushKind::ReviewLhb),
            Self::R08 => Ok(stock_analysis::durable_delivery::PushKind::EventCalendar),
            Self::R09 => Ok(stock_analysis::durable_delivery::PushKind::ReviewProviderTopN),
            other => Err(ReviewTaskDurableKindError::UnsupportedTask(other)),
        }
    }

    pub const fn review_task_production_capability(
        self,
    ) -> Result<ReviewTaskProductionCapability, ReviewTaskCapabilityError> {
        match self {
            Self::R04 => Ok(ReviewTaskProductionCapability::EnabledSourceOnly),
            Self::R09 => Ok(ReviewTaskProductionCapability::DisabledNoProducer(
                ReviewTaskNoProducerReason::NoProducer,
            )),
            Self::R02 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::R02)),
            Self::R03 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::R03)),
            Self::R05 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::R05)),
            Self::R06 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::R06)),
            Self::R08 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::R08)),
            Self::A10 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::A10)),
            Self::A01 => Err(ReviewTaskCapabilityError::UnsupportedTask(Self::A01)),
        }
    }
}
```

R-08 and R-09 durable kinds are identity only. `review_preflight` must consume
typed `UnsupportedTask(R08)` and R-09's closed disabled result before dependency
partition. R-08 emits a typed unsupported outcome; R-09 emits exact reason
`capability_unavailable:no_producer`; neither may construct a provider. BR-200
assigns no future R-08 owner. BR-192 is the future atomic R-09 enablement owner,
not a BR-200 prerequisite.

- [ ] **Step 2: Add genuine RED mapping/audit tests**

Declare exactly once in `push_templates::tests`:

| Exact test declaration | Required RED body |
| --- | --- |
| `fn br200_delivered_hydrated_reuses_terminal_snapshot_for_live_r04_and_generic_fixtures()` | Feed valid Delivered hydration for live R-04 plus generic R-08/BusinessDateOnce fixtures; require the original terminal count and exact snapshot while all provider, renderer, new-decision, and sink counters stay zero. |
| `fn br200_delivered_missing_hydration_is_retryable_failed_with_exact_reason_code()` | Feed Delivered without hydration; require retryable Failed and exactly `durable_occurrence_delivered_hydration_pending`, with zero downstream calls. |
| `fn br200_rejected_and_uncertain_are_nonretryable_terminal_failures()` | Table-drive RejectedDurable, ManualResolvedRejected, and UncertainManualReview; require nonretryable Failed and exactly `durable_occurrence_terminal_failure`. |
| `fn br200_nonterminal_states_are_retryable_reconciliation_failures()` | Table-drive every remaining nonterminal `DecisionState`; require retryable Failed and exactly `durable_occurrence_nonterminal_reconciliation_pending`. |
| `fn br200_corrupt_mismatch_and_ambiguous_are_typed_nonretryable_invariants()` | Table-drive every closed invariant kind; require a typed nonretryable invariant failure and no fallback. |
| `fn br200_generic_verified_empty_delivered_hydration_reuses_zero_snapshot()` | Feed a generic verified-empty Delivered fixture; require zero terminal count reuse and no downstream calls without granting R-08 production capability. |
| `fn br200_positive_snapshot_tasks_reject_zero_delivered_hydration()` | Feed zero-count Delivered hydration for R-04 and the positive-snapshot `BusinessDateOnce` fixture; require the exact nonretryable invalid-hydration invariant. |

Declare exactly once in `review_batch::tests`:

| Exact test declaration | Required RED body |
| --- | --- |
| `fn br200_occurrence_audit_binds_exact_reason_retryability_hash_and_rule_vector()` | Serialize every mapped live R-04 failure; compare exact reason code, retryability, durable state, hashed identity, invariant option, and ordered five-rule array; assert raw identity and BR-198 are absent. |
| `fn br200_occurrence_failure_wire_rejects_unknown_raw_identity_and_rule_drift()` | Deserialize payloads with an unknown field, raw identity, missing/reordered/extended rule vector, and unknown invariant; require rejection in every case. |
| `fn br200_review_task_durable_kind_and_production_capability_mapping_is_closed()` | Table-drive all nine tasks with explicit arms: R-04→ReviewLhb/EnabledSourceOnly, R-08 retains EventCalendar identity but capability is exact `UnsupportedTask(R08)`, R-09→ReviewProviderTopN/DisabledNoProducer(NoProducer), and exact `ReviewTaskCapabilityError::UnsupportedTask(task)` for the other six; reject `Option`, wildcard and identity-as-permission. |

- [ ] **Step 3: Run RED commands**

```bash
cargo test --bin monitor push_templates::tests::br200_delivered_hydrated_reuses_terminal_snapshot_for_live_r04_and_generic_fixtures -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_delivered_missing_hydration_is_retryable_failed_with_exact_reason_code -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_rejected_and_uncertain_are_nonretryable_terminal_failures -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_nonterminal_states_are_retryable_reconciliation_failures -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_corrupt_mismatch_and_ambiguous_are_typed_nonretryable_invariants -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_generic_verified_empty_delivered_hydration_reuses_zero_snapshot -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_positive_snapshot_tasks_reject_zero_delivered_hydration -- --exact --test-threads=1
cargo test --bin monitor review_batch::tests::br200_occurrence_audit_binds_exact_reason_retryability_hash_and_rule_vector -- --exact --test-threads=1
cargo test --bin monitor review_batch::tests::br200_occurrence_failure_wire_rejects_unknown_raw_identity_and_rule_drift -- --exact --test-threads=1
cargo test --bin monitor review_batch::tests::br200_review_task_durable_kind_and_production_capability_mapping_is_closed -- --exact --test-threads=1
```

Expected: every command selects one test and fails only on the missing typed
mapping/audit behavior.

- [ ] **Step 4: Implement one exhaustive mapper**

Implement one private `review_outcome_from_existing_durable` used by live R-04
and generic fixtures. It validates
decision/task/business-date/hydration canonical/hash/rule identity before
matching the complete `DecisionState` enum. It returns only the frozen mapping
at the top of this plan, hashes the decision identity with the existing review
audit domain, and applies in-memory hydration only for valid Delivered evidence.

The match must list every state explicitly; a wildcard arm is forbidden so a
future state fails compilation until classified.

- [ ] **Step 5: Run GREEN and audit regressions**

Run all ten exact commands from Step 3, then:

```bash
cargo test --bin monitor review_batch::tests -- --test-threads=1
cargo test --bin monitor push_templates::tests -- --test-threads=1
```

Expected: all pass and no serialized fixture contains a raw decision identity.

- [ ] **Step 6: Commit Task 3**

```bash
git add src/bin/monitor/review_batch.rs src/bin/monitor/push_templates.rs
git diff --cached --check
git commit -m "feat: map BR-200 durable review terminals"
```

### Task 4: Enforce producer ordering and initial-versus-retry semantics

**Files:**

- Modify: `src/bin/monitor/push_templates.rs` (sole live R-04 seam and inline injected-counter tests)
- Modify: `src/bin/monitor/review_batch.rs` (unsupported R-08/disabled R-09 capabilities before partition)
- Modify: `src/bin/monitor/main.rs` (single production startup-banner installation)
- Modify: `src/durable_delivery/tests.rs` (generic `BusinessDateOnce` fixture tests only)
- Modify: `tests/monitor_help_isolation.rs` (nonce-bound process startup proof)
- Modify: `tools/compliance/lib/check_br194_review_dependency.sh`
- Create: `tools/release/disable_br200_review_consumers.patch`
- Create: `tools/release/verify_br200_forward_rollback.sh`

- [ ] **Step 1: Add genuine RED cross-layer tests**

Declare the live R-04, unsupported R-08 and disabled R-09 names exactly once inside
`push_templates::tests`, where the private producer seam and injected adapters
are directly accessible. Declare the two `BusinessDateOnce` fixture names
exactly once in `src/durable_delivery/tests.rs`:

| Exact test declaration | Required RED body |
| --- | --- |
| `fn br200_r04_preflight_runs_before_ready_time_and_provider()` | Record `capture, br200, ready_time, provider` and require that exact trace for `None`; for every non-`None` result stop after `br200` with all downstream counters zero. |
| `fn br200_r08_is_unsupported_before_partition_without_downstream_calls()` | Drive only R-08; require exact typed `ReviewTaskCapabilityError::UnsupportedTask(R08)` before partition, retained EventCalendar identity mapping, and zero local-position/virtual/Yahoo/provider/renderer/new-decision/sink calls. |
| `fn br200_business_date_once_fixture_returns_none_without_creating_decision()` | In `src/durable_delivery/tests.rs`, inspect an absent synthetic `TEST_CODE` `BusinessDateOnce` key; require exact `None` and unchanged durable table counts. Do not instantiate a monitor dispatcher, gateway, provider, renderer, or sink. |
| `fn br200_no_occurrence_runs_live_r04_initial_provider_once()` | For live R-04 return exact `None`; require its provider once, renderer once, new durable prepare once, and sink at most once under the existing counted-delivery owner. |
| `fn br200_existing_occurrence_never_falls_through_to_live_r04_initial_acquisition()` | Table-drive every Delivered/terminal/nonterminal/invariant result for live R-04; require zero provider, renderer, new decision, and sink calls. |
| `fn br200_business_date_once_fixture_distinguishes_absence_from_existing_rejection()` | In `src/durable_delivery/tests.rs`, compare an absent synthetic key with an exact retained rejection; require `None` only for absence and typed non-Delivered evidence for rejection, with zero writes and no production consumer. |
| `fn br200_r09_remains_typed_disabled_without_provider_renderer_decision_or_sink()` | Drive only R-09; require `DisabledNoProducer(NoProducer)`/`capability_unavailable:no_producer`, removal before partition, retained ReviewProviderTopN/SourceOnly identity, and zero provider/renderer/new-decision/sink counters. |
| `fn br200_r09_no_producer_startup_banner_is_emitted_exactly_once()` | Start a nonce-bound `TEST_CODE` monitor subprocess, require the exact banner once before review scheduler/provider initialization, shut down gracefully, and prove R-09 provider/renderer/new-decision/sink counters stay zero. |

Use injected call counters and an ordered event trace for the live R-04 test and
the unsupported/disabled task tests. Every `Some`/error R-04 case and every R-08/R-09 case
asserts zero provider, renderer, new durable decision, and sink calls. The two
core tests use only nonce-bound `TEST_CODE` fixtures and durable table-count
helpers. They prove the generic API without creating a production R-08/R-09 path.

- [ ] **Step 2: Run RED commands**

```bash
cargo test --bin monitor push_templates::tests::br200_r04_preflight_runs_before_ready_time_and_provider -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_r08_is_unsupported_before_partition_without_downstream_calls -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br200_business_date_once_fixture_returns_none_without_creating_decision -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_no_occurrence_runs_live_r04_initial_provider_once -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_existing_occurrence_never_falls_through_to_live_r04_initial_acquisition -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br200_business_date_once_fixture_distinguishes_absence_from_existing_rejection -- --exact --test-threads=1
cargo test --bin monitor push_templates::tests::br200_r09_remains_typed_disabled_without_provider_renderer_decision_or_sink -- --exact --test-threads=1
cargo test --test monitor_help_isolation br200_r09_no_producer_startup_banner_is_emitted_exactly_once -- --exact --test-threads=1
```

Expected: every command runs one test and fails on the absent live ordering or
generic absence/evidence seam.

- [ ] **Step 3: Integrate each task in closed order**

- R-04: capture date/task identity, run BR-200, then 21:00 readiness, then
  `DragonTigerGateway`; valid Delivered reuse may finish before 21:00.
- R-08: retain EventCalendar identity mapping, return typed
  `ReviewTaskCapabilityError::UnsupportedTask(R08)` before dependency partition,
  and make zero local-position/virtual/Yahoo/provider/renderer/new-decision/sink
  calls. BR-200 assigns no future enablement owner.
- Every error from BR-200 is terminal for the current invocation; never convert
  it to `None` or provider fallback.
- Preserve R-09 in `ReviewTask::ALL`, label, identity, durable-kind mapping and
  SourceOnly dependency classification, but remove it in `review_preflight` as
  typed `DisabledNoProducer(NoProducer)` before partition. Do not add an R-09 dispatcher,
  gateway, provider, renderer, catalog permit, or sink. Emit exactly one
  `[BR-200][R-09] disabled=no_producer reason=capability_unavailable:no_producer`
  startup banner by calling one production `Once`-guarded installer from
  `main.rs` after CLI/test isolation is known and before audit/provider/database/
  scheduler initialization. The checker rejects missing/duplicate call sites,
  banner drift, and bypass of the `Once` guard. Later
  BR-192 work atomically changes capability plus adds all provider/catalog
  authorities; changing capability alone must fail the checker. BR-192 is not a
  prerequisite for accepting BR-200.

- [ ] **Step 4: Strengthen the compliance checker**

Only append to `check_br194_review_dependency.sh`. Its fixed-HEAD content is 659
lines with SHA-256
`2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f`.
Do not delete, rewrite, reorder, or weaken any fixed-HEAD line. Exact line 660
must be `# BR-200 APPEND-ONLY CONTRACT START`; append BR-200 parsing/assertions
only after it and require:

```text
R04: capture -> BR200 -> ready-time -> DragonTigerGateway
R08: EventCalendar identity retained -> UnsupportedTask(R08) before partition
     -> zero downstream calls
live R04 ordered rules: BR110,BR140,BR192,BR194,BR200; BR198 forbidden
forbidden before BR200: provider, renderer, new decision, sink
R09 profile: identity + SourceOnly + ReviewProviderTopN kind retained,
             DisabledNoProducer(NoProducer) before partition,
             no provider/gateway/renderer/sink
```

The appended checks must exit non-zero on missing/reordered calls, `Option` or a
wildcard capability arm, string downgrade of a typed invariant, R-08 unsupported
or R-09 disabled reason
drift, removal of existing BR-194 identity/classification assertions, or any
disabled-profile occurrence of
`dispatch_r09_provider_top_n_outcome`, `prepare_r09_provider_top_n_report`,
`render_r09_provider_top_n`, `provider_top_n_pair`, or an R-09
`CapitalDataGateway` call. It must not require an R-09 provider as a BR-200
prerequisite. Its later BR-192 enabled profile must require capability,
catalog permit, gateway, provider, renderer and counted-delivery seam together.
BR-200 assigns no future enablement owner to R-08; this slice only preserves its
durable EventCalendar identity while rejecting the task as unsupported.
The checker nevertheless recognizes a second, atomic `Br199Enabled` profile so
this baseline cannot permanently block the separately accepted BR-199 work. The
enabled profile requires all BR-199 SourceOnly authorities together: capability,
dependency classification, exact EventCalendar durable identity, all four
public provider batches with canonical binding/order, mandatory CFFEX capability
handling, renderer, counted-delivery seam, and absence of account/local/user/
virtual-holding reads. Capability-only, producer-only, partial-provider, mixed
profile, and wildcard states fail. BR-200 owns no R-08 transition.
The appended mutation matrix must prove both complete profiles pass, then prove
capability-only, producer-only, missing-each-public-batch, missing CFFEX
capability, missing renderer/counted-delivery, mixed profile, wildcard, and any
account/local/user/virtual-holding read each fail independently.

After the checker append is final, compute its line-660-through-EOF digest once
and write it with `apply_patch` into the new verifier as the exact literal
`BR200_EXPECTED_CHECKER_APPEND_SHA256=<lowercase-64hex>`. The verifier also owns
the fixed prefix digest as
`BR200_EXPECTED_CHECKER_PREFIX_SHA256=2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f`.
Neither value may come from caller environment or arguments. Before GREEN, run
this literal fixed-baseline no-deletion audit in addition to the checker itself:

```bash
set -euo pipefail
BR200_FIXED_HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496
BR200_CHECKER=tools/compliance/lib/check_br194_review_dependency.sh
BR200_VERIFY=tools/release/verify_br200_forward_rollback.sh
BR200_CHECKER_PREFIX_SHA256=2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f
BR200_CHECKER_APPEND_SHA256="$(sed -n -E 's/^BR200_EXPECTED_CHECKER_APPEND_SHA256=([0-9a-f]{64})$/\1/p' "${BR200_VERIFY}")"
test "$(rg -c '^BR200_EXPECTED_CHECKER_APPEND_SHA256=[0-9a-f]{64}$' "${BR200_VERIFY}")" -eq 1
test "$(rg -c '^BR200_EXPECTED_CHECKER_PREFIX_SHA256=2ac9baa210e5ff2521deaad3252422417e956c56e0bdd52a1902ab1a1503931f$' "${BR200_VERIFY}")" -eq 1
test "${#BR200_CHECKER_APPEND_SHA256}" -eq 64
BR200_CHECKER_BASE="$(mktemp "${TMPDIR:-/tmp}/br200-checker-base.XXXXXX")"
BR200_CHECKER_PREFIX="$(mktemp "${TMPDIR:-/tmp}/br200-checker-prefix.XXXXXX")"
BR200_CHECKER_LOG="$(mktemp "${TMPDIR:-/tmp}/br200-checker-selftest.XXXXXX.log")"
git show "${BR200_FIXED_HEAD}:${BR200_CHECKER}" >"${BR200_CHECKER_BASE}"
test "$(wc -l <"${BR200_CHECKER_BASE}" | tr -d ' ')" -eq 659
head -n 659 "${BR200_CHECKER}" >"${BR200_CHECKER_PREFIX}"
cmp "${BR200_CHECKER_BASE}" "${BR200_CHECKER_PREFIX}"
test "$(shasum -a 256 "${BR200_CHECKER_PREFIX}" | cut -d ' ' -f1)" = "${BR200_CHECKER_PREFIX_SHA256}"
test "$(sed -n '660p' "${BR200_CHECKER}")" = '# BR-200 APPEND-ONLY CONTRACT START'
test "$(tail -n +660 "${BR200_CHECKER}" | shasum -a 256 | cut -d ' ' -f1)" = "${BR200_CHECKER_APPEND_SHA256}"
! head -n 659 "${BR200_CHECKER}" | rg -n '^[[:space:]]*(exit|return)[[:space:]]+0([[:space:]]|$)'
bash "${BR200_CHECKER}" | tee "${BR200_CHECKER_LOG}"
rg -q '^BR-194 review dependency static contract: PASS$' "${BR200_CHECKER_LOG}"
rg -q '^BR-194 review dependency mutation matrix: PASS$' "${BR200_CHECKER_LOG}"
rg -q '^BR-200 counted review terminal preflight static contract: PASS$' "${BR200_CHECKER_LOG}"
rg -q '^BR-200 counted review terminal preflight mutation matrix: PASS$' "${BR200_CHECKER_LOG}"
rg -q '^BR-200 cross-version debt contract: PASS$' "${BR200_CHECKER_LOG}"
```

The byte-equality proof protects the complete fixed three-caller boundary,
preflight/partition/join/account/unique-merge order, R-04 SourceOnly chain,
dual-test-disable, exact replay/schema/verifier inventory, and the complete
mutation matrix. The appended block must include its own missing/reorder/rewrite,
early-exit/return and boundary mutation cases. A replacement checker, partial
copied subset or checker that avoids either mutation matrix is forbidden.
Reaching the append proves the unchanged prefix mutation matrix executed; after
the appended mutations execute, the checker prints exact
`BR-194 review dependency mutation matrix: PASS` and
`BR-200 counted review terminal preflight mutation matrix: PASS` records.
The appended checker also audits the touched `ReviewLhb`, `EventCalendar`, and
`ReviewProviderTopN` cross-version markers: active R-04 requires its real caller,
R-08 is active only under the complete separately accepted `Br199Enabled`
profile, R-09 must not be activated by BR-200, and every retained legacy marker
requires an explicit deletion-plan citation. Success prints exact
`BR-200 cross-version debt contract: PASS`.

Create the forward rollback patch and verifier. The patch contains exactly one
`diff --git` target, `src/bin/monitor/push_templates.rs`; it replaces only the
R-04 live install with an explicit typed
`br200_rollback_consumer_disabled` outcomes before provider access and adds the
exact patched-tree test
`br200_rollback_install_state_is_fail_closed_without_external_calls`. The
verifier checks target cardinality/forbidden paths, creates an isolated
worktree at a supplied literal accepted release SHA, runs `git apply --check`,
applies the patch, verifies the resulting one-file diff, builds the rollback
monitor, and runs that exact nonce-bound `TEST_CODE` test with zero provider,
renderer, new-decision and sink calls. Before and after patch application it
must itself verify its embedded prefix/append digests, the exact 659/660
boundary, all five PASS records above, and both mutation matrices. R-08/R-09 durable identities, typed R-08
unsupported state and exact R-09 disabled reason must remain byte-identical.

Define `run_br200_verifier_from_commit` in the same fail-closed Task-4/Gate-C
shell block. It takes one literal commit SHA, creates a `mktemp -d` parent and
clean detached worktree, verifies `HEAD` and an empty porcelain status, proves
`tools/release/verify_br200_forward_rollback.sh` and the shared checker have the
same blob hashes as that commit, executes the verifier from inside that
worktree, proves the worktree remains clean, then removes it. The function and
all callers use `set -euo pipefail`; cleanup preserves and returns any verifier
failure. Calling the verifier from the mutable caller worktree is forbidden.

```bash
set -euo pipefail
run_br200_verifier_from_commit() {
  br200_verify_sha="$1"
  br200_verify_parent="$(mktemp -d "${TMPDIR:-/tmp}/br200-verifier.XXXXXX")"
  br200_verify_worktree="${br200_verify_parent}/worktree"
  br200_verify_status=0
  git cat-file -e "${br200_verify_sha}^{commit}"
  git worktree add --detach "${br200_verify_worktree}" "${br200_verify_sha}"
  (
    set -euo pipefail
    cd "${br200_verify_worktree}"
    test -z "$(git status --porcelain)"
    test "$(git rev-parse HEAD)" = "${br200_verify_sha}"
    for path in \
      tools/release/verify_br200_forward_rollback.sh \
      tools/compliance/lib/check_br194_review_dependency.sh
    do
      test "$(git hash-object "${path}")" = "$(git rev-parse "HEAD:${path}")"
    done
    bash tools/release/verify_br200_forward_rollback.sh "${br200_verify_sha}"
    test -z "$(git status --porcelain)"
  ) || br200_verify_status=$?
  git worktree remove --force "${br200_verify_worktree}"
  rmdir "${br200_verify_parent}"
  test "${br200_verify_status}" -eq 0
}
```

- [ ] **Step 5: Run GREEN, checker, and isolation tests**

Run the eight exact commands from Step 2, then:

```bash
set -euo pipefail
cargo test --bin monitor push_templates::tests -- --test-threads=1
cargo test --test durable_delivery_counted_cutover -- --test-threads=1
bash tools/compliance/lib/check_br194_review_dependency.sh
cargo test --test monitor_help_isolation -- --test-threads=1
git diff --cached --quiet
git add src/bin/monitor/push_templates.rs src/bin/monitor/review_batch.rs src/bin/monitor/main.rs src/durable_delivery/tests.rs tests/monitor_help_isolation.rs tools/compliance/lib/check_br194_review_dependency.sh tools/release/disable_br200_review_consumers.patch tools/release/verify_br200_forward_rollback.sh
BR200_TASK4_STAGED="$(git diff --cached --name-only)"
BR200_TASK4_EXPECTED="$(printf '%s\n' \
  src/bin/monitor/push_templates.rs \
  src/bin/monitor/review_batch.rs \
  src/bin/monitor/main.rs \
  src/durable_delivery/tests.rs \
  tests/monitor_help_isolation.rs \
  tools/compliance/lib/check_br194_review_dependency.sh \
  tools/release/disable_br200_review_consumers.patch \
  tools/release/verify_br200_forward_rollback.sh)"
test "${BR200_TASK4_STAGED}" = "${BR200_TASK4_EXPECTED}"
BR200_TASK4_VERIFIED_TREE="$(git write-tree)"
BR200_TASK4_PARENT="$(git rev-parse HEAD)"
BR200_TASK4_CANDIDATE_SHA="$(printf '%s\n' 'BR-200 Task 4 verified candidate' | git commit-tree "${BR200_TASK4_VERIFIED_TREE}" -p "${BR200_TASK4_PARENT}")"
test "$(git rev-parse "${BR200_TASK4_CANDIDATE_SHA}^{tree}")" = "${BR200_TASK4_VERIFIED_TREE}"
run_br200_verifier_from_commit "${BR200_TASK4_CANDIDATE_SHA}"
```

Expected: all pass; fixture authorities remain under `TEST_CODE`; production
ordering is proven only for R-04, while unsupported R-08 and disabled R-09 stop
before partition. `run_br200_verifier_from_commit` creates a clean detached
worktree at the supplied SHA, proves the verifier/checker blobs equal that
commit tree, executes that tree's verifier, and proves it made no worktree
mutation. It never executes the caller's mutable verifier or the pre-Task-4
branch `HEAD`.

- [ ] **Step 6: Commit Task 4**

```bash
set -euo pipefail
test -n "${BR200_TASK4_VERIFIED_TREE}"
test "$(git write-tree)" = "${BR200_TASK4_VERIFIED_TREE}"
git add src/bin/monitor/push_templates.rs src/bin/monitor/review_batch.rs src/bin/monitor/main.rs src/durable_delivery/tests.rs tests/monitor_help_isolation.rs tools/compliance/lib/check_br194_review_dependency.sh tools/release/disable_br200_review_consumers.patch tools/release/verify_br200_forward_rollback.sh
test "$(git write-tree)" = "${BR200_TASK4_VERIFIED_TREE}"
git diff --cached --check
git commit -m "feat: enforce BR-200 review preflight ordering"
test "$(git rev-parse 'HEAD^{tree}')" = "${BR200_TASK4_VERIFIED_TREE}"
test -z "$(git status --porcelain)"
run_br200_verifier_from_commit "$(git rev-parse HEAD)"
```

### Task 5: Gate B/C/D verification and PR evidence

**Files:**

- Verify all paths in “Complete file ownership”
- Create: `tools/release/verify_br200_repeated_review.py`
- Record raw command/output evidence in the Draft PR

- [ ] **Step 1: Verify exact test declaration cardinality**

```bash
set -euo pipefail
export LC_ALL=C
export CARGO_TERM_COLOR=never
BR200_PLAN=docs/superpowers/plans/2026-08-01-counted-review-terminal-preflight.md
BR200_EXPECTED="$(mktemp "${TMPDIR:-/tmp}/br200-expected-tests.XXXXXX")"
BR200_COMMANDS="$(mktemp "${TMPDIR:-/tmp}/br200-command-tests.XXXXXX")"
BR200_DECLARATIONS="$(mktemp "${TMPDIR:-/tmp}/br200-declared-tests.XXXXXX")"
BR200_REGISTERED="$(mktemp "${TMPDIR:-/tmp}/br200-registered-tests.XXXXXX")"
BR200_EXACT_COMMANDS="$(mktemp "${TMPDIR:-/tmp}/br200-exact-commands.XXXXXX")"
BR200_TEST_LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/br200-test-proof.XXXXXX")"
BR200_CARGO_BIN="$(type -P cargo)"
test -n "${BR200_CARGO_BIN}"
test -x "${BR200_CARGO_BIN}"
"${BR200_CARGO_BIN}" -Vv
sed -n '/^BR200_TEST_MANIFEST_BEGIN$/,/^BR200_TEST_MANIFEST_END$/p' "${BR200_PLAN}" \
  | sed '1d;$d' | sort >"${BR200_EXPECTED}"
rg '^cargo test .* -- --exact --test-threads=1$' "${BR200_PLAN}" \
  | sed -E 's/^cargo test (--lib|--bin [^ ]+|--test [^ ]+) ([^ ]+) -- --exact.*/\2/' \
  | sed 's/.*:://' | sort >"${BR200_COMMANDS}"
rg --no-filename -o 'fn br200_[A-Za-z0-9_]+\(' \
  src/durable_delivery/tests.rs \
  src/bin/monitor/durable_delivery_runtime.rs \
  src/bin/monitor/review_batch.rs \
  src/bin/monitor/push_templates.rs \
  tests/monitor_help_isolation.rs \
  | sed -E 's/^fn ([^(]+)\(/\1/' | sort >"${BR200_DECLARATIONS}"
{
  cargo test --lib -- --list
  cargo test --bin monitor -- --list
  cargo test --test durable_delivery_counted_cutover -- --list
  cargo test --test monitor_help_isolation -- --list
} | sed -n -E 's/^(.*::)?(br200_[A-Za-z0-9_]+): test$/\2/p' \
  | sort >"${BR200_REGISTERED}"
rg '^cargo test .* -- --exact --test-threads=1$' "${BR200_PLAN}" \
  >"${BR200_EXACT_COMMANDS}"
test "$(wc -l <"${BR200_EXPECTED}" | tr -d ' ')" -eq 26
test "$(sort -u "${BR200_EXPECTED}" | wc -l | tr -d ' ')" -eq 26
test "$(wc -l <"${BR200_COMMANDS}" | tr -d ' ')" -eq 26
test "$(sort -u "${BR200_COMMANDS}" | wc -l | tr -d ' ')" -eq 26
test "$(wc -l <"${BR200_DECLARATIONS}" | tr -d ' ')" -eq 26
test "$(sort -u "${BR200_DECLARATIONS}" | wc -l | tr -d ' ')" -eq 26
test "$(wc -l <"${BR200_REGISTERED}" | tr -d ' ')" -eq 26
test "$(sort -u "${BR200_REGISTERED}" | wc -l | tr -d ' ')" -eq 26
test "$(wc -l <"${BR200_EXACT_COMMANDS}" | tr -d ' ')" -eq 26
cmp "${BR200_EXPECTED}" "${BR200_COMMANDS}"
cmp "${BR200_EXPECTED}" "${BR200_DECLARATIONS}"
cmp "${BR200_EXPECTED}" "${BR200_REGISTERED}"
! rg -v '^cargo test (--lib|--bin monitor|--test (durable_delivery_counted_cutover|monitor_help_isolation)) [A-Za-z0-9_:]+ -- --exact --test-threads=1$' "${BR200_EXACT_COMMANDS}"
BR200_COMMAND_ORDINAL=0
while IFS= read -r command; do
  BR200_COMMAND_ORDINAL=$((BR200_COMMAND_ORDINAL + 1))
  read -r -a argv <<<"${command}"
  test "${argv[0]}" = cargo
  test "${argv[1]}" = test
  case "${argv[2]}" in
    --lib)
      test "${#argv[@]}" -eq 7
      test_name="${argv[3]}"
      ;;
    --bin)
      test "${#argv[@]}" -eq 8
      test "${argv[3]}" = monitor
      test_name="${argv[4]}"
      ;;
    --test)
      test "${#argv[@]}" -eq 8
      case "${argv[3]}" in
        durable_delivery_counted_cutover|monitor_help_isolation) ;;
        *) exit 1 ;;
      esac
      test_name="${argv[4]}"
      ;;
    *) exit 1 ;;
  esac
  case "${test_name}" in *[!A-Za-z0-9_:]*|'') exit 1 ;; esac
  argv[0]="${BR200_CARGO_BIN}"
  BR200_ONE_TEST_LOG="${BR200_TEST_LOG_DIR}/$(printf '%02d' "${BR200_COMMAND_ORDINAL}").log"
  if ! command "${argv[@]}" >"${BR200_ONE_TEST_LOG}" 2>&1; then
    sed -n '1,240p' "${BR200_ONE_TEST_LOG}" >&2
    exit 1
  fi
  test "$(awk '$0 == "running 1 test" { n++ } END { print n + 0 }' "${BR200_ONE_TEST_LOG}")" -eq 1
  test "$(awk '/^running [0-9]+ tests?$/ { n++ } END { print n + 0 }' "${BR200_ONE_TEST_LOG}")" -eq 1
  test "$(awk -v expected="test ${test_name} ... ok" '$0 == expected { n++ } END { print n + 0 }' "${BR200_ONE_TEST_LOG}")" -eq 1
  test "$(awk '/^test result:/ { summaries++ } /^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in / { accepted++ } END { print (summaries == 1 && accepted == 1) ? 1 : 0 }' "${BR200_ONE_TEST_LOG}")" -eq 1
  ! rg -q '\.\.\. ignored$|[1-9][0-9]* ignored;' "${BR200_ONE_TEST_LOG}"
done <"${BR200_EXACT_COMMANDS}"
test "${BR200_COMMAND_ORDINAL}" -eq 26
```

The canonical manifest embedded once in this plan is:

```text
BR200_TEST_MANIFEST_BEGIN
br200_business_date_once_fixture_distinguishes_absence_from_existing_rejection
br200_business_date_once_fixture_returns_none_without_creating_decision
br200_business_date_once_fixture_reuses_delivered_without_writes
br200_corrupt_mismatch_and_ambiguous_are_typed_nonretryable_invariants
br200_delivered_hydrated_reuses_terminal_snapshot_for_live_r04_and_generic_fixtures
br200_delivered_missing_hydration_is_retryable_failed_with_exact_reason_code
br200_existing_occurrence_never_falls_through_to_live_r04_initial_acquisition
br200_generic_verified_empty_delivered_hydration_reuses_zero_snapshot
br200_no_occurrence_runs_live_r04_initial_provider_once
br200_nonterminal_states_are_retryable_reconciliation_failures
br200_occurrence_audit_binds_exact_reason_retryability_hash_and_rule_vector
br200_occurrence_failure_wire_rejects_unknown_raw_identity_and_rule_drift
br200_occurrence_query_preserves_exact_ordered_rule_ids
br200_occurrence_query_rejects_claim_mismatch_as_typed_invariant_without_writes
br200_occurrence_query_rejects_hash_identity_and_multiple_delivered_ambiguity_without_writes
br200_positive_snapshot_tasks_reject_zero_delivered_hydration
br200_r04_preflight_runs_before_ready_time_and_provider
br200_r04_rolling_preflight_prefers_original_delivered_over_later_denial
br200_r08_is_unsupported_before_partition_without_downstream_calls
br200_r09_no_producer_startup_banner_is_emitted_exactly_once
br200_r09_remains_typed_disabled_without_provider_renderer_decision_or_sink
br200_rejected_and_uncertain_are_nonretryable_terminal_failures
br200_review_task_durable_kind_and_production_capability_mapping_is_closed
br200_runtime_preflight_accepts_live_r04_and_fixture_event_calendar_business_date_once_keys
br200_runtime_preflight_preserves_typed_invariant_and_never_queues_unvalidated_hydration
br200_runtime_preflight_requires_completed_startup_barrier_without_reconciliation_or_sink
BR200_TEST_MANIFEST_END
```

Expected: manifest, command targets, source declarations, and Cargo-registered
tests are identical 26-element sets. Every strict reviewed command directly
runs exactly one named non-ignored test. Duplicate names, missing/unregistered
commands or declarations, ignored/filtered-only/zero-test success, or a 27th
`fn br200_*` declaration fails.

- [ ] **Step 2: Gate B focused verification**

```bash
set -euo pipefail
cargo test --lib durable_delivery::tests -- --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests -- --test-threads=1
cargo test --bin monitor review_batch::tests -- --test-threads=1
cargo test --bin monitor push_templates::tests -- --test-threads=1
cargo test --test durable_delivery_counted_cutover -- --test-threads=1
cargo build --lib
cargo build --bin monitor
```

Expected: non-zero counts and all pass.

- [ ] **Step 3: Gate C**

```bash
set -euo pipefail
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
BR200_CHECKER=tools/compliance/lib/check_br194_review_dependency.sh
BR200_GATE_C_LOG="$(mktemp "${TMPDIR:-/tmp}/br200-gate-c-checker.XXXXXX.log")"
bash "${BR200_CHECKER}" | tee "${BR200_GATE_C_LOG}"
rg -q '^BR-194 review dependency static contract: PASS$' "${BR200_GATE_C_LOG}"
rg -q '^BR-194 review dependency mutation matrix: PASS$' "${BR200_GATE_C_LOG}"
rg -q '^BR-200 counted review terminal preflight static contract: PASS$' "${BR200_GATE_C_LOG}"
rg -q '^BR-200 counted review terminal preflight mutation matrix: PASS$' "${BR200_GATE_C_LOG}"
rg -q '^BR-200 cross-version debt contract: PASS$' "${BR200_GATE_C_LOG}"
test -z "$(git status --porcelain)"
run_br200_verifier_from_commit "$(git rev-parse HEAD)"
bash tools/compliance/check.sh
git diff --check
```

Expected: every command exits 0. A freshness failure is blocking and follows the
repository backfill procedure; it is not waived for this delivery-only change.

- [ ] **Step 4: Release and isolated test smoke**

```bash
set -euo pipefail
cargo build --release --bin monitor
BR200_SMOKE_LOG="$(mktemp "${TMPDIR:-/tmp}/TEST_CODE_BR200_monitor_test.XXXXXX.log")"
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | tee "${BR200_SMOKE_LOG}"
test "$(awk -v line='[BR-200][R-09] disabled=no_producer reason=capability_unavailable:no_producer' 'index($0, line) { n++ } END { print n + 0 }' "${BR200_SMOKE_LOG}")" -eq 1
rg -q '\[R-04\].*test_environment_external_provider_blocked.*provider_calls=0.*renderer_calls=0.*new_decision_calls=0.*sink_calls=0' "${BR200_SMOKE_LOG}"
rg -q '\[R-08\].*UnsupportedTask\(R08\).*provider_calls=0.*renderer_calls=0.*new_decision_calls=0.*sink_calls=0' "${BR200_SMOKE_LOG}"
rg -q '\[R-09\].*DisabledNoProducer\(NoProducer\).*provider_calls=0.*renderer_calls=0.*new_decision_calls=0.*sink_calls=0' "${BR200_SMOKE_LOG}"
```

Expected: only `data/test/**`, `TEST_CODE` sinks, and the test durable namespace
may change. Test mode blocks R-04 before any external provider and is not live
R-04 Gate-D evidence. R-08 is typed unsupported and R-09 typed disabled before
partition, all external-call counters are zero, production durable/push/audit/
event/review watermarks do not change, and no real Feishu message is sent.

- [ ] **Step 5: Gate D coverage authority**

Run the repository baseline coverage commands recorded in
`docs/ENGINEERING_RULES_V2.md`; BR-202 is not a BR-200 prerequisite.

```bash
set -euo pipefail
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
```

Expected: global line coverage at least 80% and core trading/data paths at least
95%. The raw coverage JSON and threshold output are attached to the Draft PR.

- [ ] **Step 6: Real repeated-review evidence**

During a valid real R-04 review window with one controlled monitor owner, use
the repository-owned phased verifier. `BR200_BUSINESS_DATE` must come from the
admitted `ReviewRunContext`/calendar authority, never the wall clock:

```bash
set -euo pipefail
: "${BR200_BUSINESS_DATE:?set from admitted ReviewRunContext authority}"
BR200_GATE_D_STATE="$(mktemp -d "${TMPDIR:-/tmp}/br200-gate-d.XXXXXX")"
python3 tools/release/verify_br200_repeated_review.py begin \
  --state "${BR200_GATE_D_STATE}" \
  --business-date "${BR200_BUSINESS_DATE}" \
  --binary ./target/release/monitor
python3 tools/release/verify_br200_repeated_review.py after-first \
  --state "${BR200_GATE_D_STATE}"
python3 tools/release/verify_br200_repeated_review.py after-second \
  --state "${BR200_GATE_D_STATE}"
```

`begin` atomically binds HEAD, binary inode/size/SHA-256, production paths,
business date, and pre-first record/length/prefix/hash-chain/identity-set
watermarks. It rejects test/mock/fixture overrides and an already existing
matching R-04 occurrence. `after-first` itself launches the bound binary and
proves one authentic provider-backed positive-snapshot R-04 Delivered
occurrence, one new decision and counted delivery, matching authoritative
delivery audit/event-bus mirror/review audit, exact ordered five-rule authority,
and one R-09 banner. `after-second` launches the same binary, revalidates every
bound hash and prefix, proves reuse of the same task/decision/occurrence/provider
evidence identities, requires provider/renderer/new-decision/sink all zero,
allows only the one bound reuse-audit append, and requires the second process's
R-09 banner exactly once. Phase JSON uses canonical JSON, atomic rename/fsync,
and a SHA-256 predecessor chain. Existing evidence is never deleted or
manufactured; absence of authentic first-run data leaves Gate D blocked.

- [ ] **Step 7: Independent Gate review**

The fresh reviewer brief starts with:

> **DO NOT trust the implementer's report as ground truth.** Independently
> rerun claimed tests/build/checks, inspect production push/review audit evidence,
> verify provider-before-preflight is impossible, verify exact one-declaration
> test cardinality, verify `data/push_log/<DATE>/` plus exact
> `push.delivery.audit` event-bus evidence, rerun the cross-version debt check,
> and reject missing real evidence.

Expected: Gate B/C only after `Critical=0 / Important=0`; Gate D only after the
real evidence and accepted coverage authority exist.

- [ ] **Step 8: Complete Draft PR metadata**

```text
Refs: docs/superpowers/specs/2026-08-01-counted-review-terminal-preflight-design.md §0-12; BR-200
Data-Redlines: [2.1,2.2,2.3,2.4,2.5,2.6,2.7,2.8,2.9,2.10]
Threshold-Proof: no config or financial threshold changed
Business-Rules: BR-110, BR-140, BR-192, BR-194, BR-200
OldModules:
| module | adopt/reject | reason |
| BR-192 coordinator/claims/hydration/store | adopt | sole durable decision and retry authorities |
| ReviewRunContext/review_task_identity | adopt | canonical review-date occurrence boundary |
| R-04 provider loader | adopt behind BR-200 | sole live consumer; run only after exact None |
| R-08 legacy account/event-calendar loader | reject from live BR-200 | retain EventCalendar identity; typed UnsupportedTask(R08) before partition; zero downstream calls; no future enablement owner assigned by this slice |
| R-09 provider/gateway/renderer/sink | reject from BR-200 | retain ReviewProviderTopN/SourceOnly identity; DisabledNoProducer(NoProducer) before partition; later BR-192 atomic enablement is not prerequisite |
| process-local cooldown/dedup | reject | cannot prove cross-process durable occurrence |
| reconciliation inside BR-200 query | reject | may resume sink and violates read-only pre-provider boundary |
Rollback: set BR200_RELEASE_SHA to the literal accepted release SHA and apply the reviewed forward patch with the verifier below
Validation: raw Gate B/C/D commands and outputs
```

Do not merge while Gate D, production evidence, or any independent Critical or
Important finding remains open.

## Rollback execution

The PR records the literal accepted release commit in `BR200_RELEASE_SHA`.
Because BR-200 is implemented through several reviewed commits, rollback is a
new forward commit from that exact release, not a partial revert. The reviewed
patch disables only the live R-04 consumer and keeps every durable/query/schema/
audit byte plus the R-08 identity/unsupported capability and R-09
identity/disabled reason intact:

```bash
set -euo pipefail
test -n "${BR200_RELEASE_SHA:?set the literal accepted BR-200 release commit}"
git cat-file -e "${BR200_RELEASE_SHA}^{commit}"
run_br200_verifier_from_commit "${BR200_RELEASE_SHA}"
BR200_ROLLBACK_PARENT="$(mktemp -d "${TMPDIR:-/tmp}/br200-rollback.XXXXXX")"
BR200_ROLLBACK_WORKTREE="${BR200_ROLLBACK_PARENT}/worktree"
BR200_ROLLBACK_BRANCH="rollback/br200-review-consumers-${BR200_RELEASE_SHA:0:12}"
git worktree add --detach "${BR200_ROLLBACK_WORKTREE}" "${BR200_RELEASE_SHA}"
(
  set -euo pipefail
  cd "${BR200_ROLLBACK_WORKTREE}"
  test -z "$(git status --porcelain)"
  test "$(git rev-parse HEAD)" = "${BR200_RELEASE_SHA}"
  git switch -c "${BR200_ROLLBACK_BRANCH}"
  test "$(rg -c '^diff --git ' tools/release/disable_br200_review_consumers.patch)" -eq 1
  rg -q '^diff --git a/src/bin/monitor/push_templates.rs b/src/bin/monitor/push_templates.rs$' tools/release/disable_br200_review_consumers.patch
  git apply --check tools/release/disable_br200_review_consumers.patch
  git apply --index tools/release/disable_br200_review_consumers.patch
  test "$(git diff --cached --name-only | wc -l | tr -d ' ')" -eq 1
  test "$(git diff --cached --name-only)" = "src/bin/monitor/push_templates.rs"
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace --all-targets --all-features -- --test-threads=1
  cargo build --release --bin monitor
  bash tools/compliance/lib/check_br194_review_dependency.sh
  bash tools/compliance/check.sh
  git diff --cached --check
  git commit -m "rollback: disable BR-200 review consumers"
)
git worktree remove --force "${BR200_ROLLBACK_WORKTREE}"
rmdir "${BR200_ROLLBACK_PARENT}"
```

Never convert a BR-200 query failure to `None`, restore a provider fallback,
delete decisions/hydration/audit, or relabel an existing occurrence as a new
initial acquisition. R-08 remains typed `UnsupportedTask(R08)` with retained
EventCalendar identity and no downstream calls; R-09 remains
`DisabledNoProducer(NoProducer)` with SourceOnly/ReviewProviderTopN identity.
Only a future BR-192-owned atomic R-09 slice may change that R-09 capability.
