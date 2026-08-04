# BR-192 Provider-Free Authorized Retry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add provider-free, audited and fenced retry of explicitly authorized durable rejections without ever retrying uncertainty.

**Architecture:** Frozen rejection dispositions are authorized by an append-only
authorization/event state machine plus v6 companion active and immutable
attempt bindings. Fixed HEAD already contains the accepted BR-194 v5 baseline;
BR-192 lands through one coherent v5-to-v6 migration and one v6 manifest; no
second or competing v4-to-v5
migration is permitted. A single startup/periodic cycle persists its identity before
blocking work, reconciles authorization bytes, processes Reserved work, then
requeries candidates with one cycle-global attempted set. A retry crosses only
the explicit prepare -> append/ack start -> transactional send-ownership claim
-> execute seam. Only the `Reserved -> AttemptInFlight` compare-and-swap winner
receives the single-use permit that may send the already-frozen envelope.

**Tech Stack:** Rust, Tokio, rusqlite/SQLite WAL, chrono, serde, fs2, existing BR-192 immutable append, event-audit and PAM operator authentication.

**Corrective plan ownership:** This plan replaces every earlier BR-192
provider-free-retry plan/Gate B recipe that has not passed Gate C. It does not
stack on or cite those drafts as accepted architecture. Existing partial code
must be conformed to these tasks. The earlier contract Gate A review passed
against design blob
`cdeec30f46c18bcbdb45ef12782943b90d1533e6`, plan blob
`6d04e26f563c9fbb455faef789daf84c17221fab` and BR-192 row SHA-256
`d3010a1a7a408f8b4ba976de32a0fc046ba6b4f09907d3f2374e1029106cb8ce`
with `Critical=0 / Important=0 / Minor=1`. Minor-1 was packaging-only and
was closed by the exact cached-row proof recorded below. Later exact reviews
returned C0/I1, C0/I2, C1/I6/M1 and C0/I3/M0. This revision corrects the
freshness terminal, real-consumer, exact-evidence, full-caller enforcement,
fixed-HEAD migration test, version criteria, RED/GREEN recipe and file-action
findings. Gate B must not proceed against the new identities until a fresh
independent review returns C0/I0; Gate B/C/D and live evidence remain pending.
The next exact reviews returned C0/I4/M0 and C1/I3/M1. This revision also
freezes the permit owner/API and persisted attestation, actively drains expiry,
uses genuine failing RED bodies, makes cutover-test creation the first Gate-B
file action, blocks legacy/non-R-09 retry provenance, replaces the invalid
fixed-HEAD multiline inventory, pins the clean-baseline dependency delta and
fixes the seven-entry cardinality. These changes again require fresh C0/I0.
Historical fixed-object reviews found C1/I7/M0 and C1/I3/M1 in the pre-call-
expiry terminal projection, result-race closure, cycle recount, Rule 2.3/R-09
semantics, fixed-HEAD caller/R-09 inventories, clock/typed-outcome ownership,
public manifest and Task ownership. This revision repairs those findings with
the two-transaction effective terminal, twelve-trigger result exclusion, cycle
`Confirmed(n)` recount, preserved fixed-HEAD R-09 declaration, coordinator-
owned clock, unified expiry outcome, complete manifest and Task-1-owned private
catalog. A fresh exact C0/I0 review is still required before Gate B.
The next independent fixed-object reviews returned C1/I3/M0 and C2/I4/M0.
This revision also repairs their exact-row hash, literal fixed-source commands,
complete direct-call inventory, conditional pre-call recount, executable RED
command, BR-198/BR-200 ordering, result/ownership bijection, rollback-origin
classifier and single begin-error-channel findings. The resulting staged
design/plan/rule objects again require two fresh independent C0/I0 reviews.
The subsequent three-way read-only precheck returned C2/I5/M0, C0/I1/M0 and
C0/I4/M1. This plan now repairs its SourceOnly signature, complete 14/15 Magic
identity, BR-200 mapping/rule IDs, capture and initial-versus-retry expiry
tests, deferred unique terminal-result bijection, prerequisite progression,
all named RED bodies, BR-202 isolated Gate-D authority and review-history
metadata. These worktree bytes still require exact staging and two fresh
independent C0/I0 reviews before any Gate B action.

---

## Data-red-line execution matrix

| Rule | Applicability | Plan DoD/evidence |
| --- | --- | --- |
| 2.1 Data source | Applies | Retry consumes only the already-frozen real ProviderTopN counted envelope; no mock/fabricated production fallback. |
| 2.2 Missing data | Applies | Missing batch/date/evidence/binding is a typed failure or explicit ineligibility; never filled. |
| 2.3 Bad-data validation | Applies | The sole R-09 producer validates both Provider TopN batches as complete and non-empty, every value as finite, and provider/source/metric/unit/business-date/order evidence as exact before render, durable admission or sink. Empty, partial, stale or malformed input is typed `Failed`, never `NoData`/verified-empty or filled. Price-series, adjacent-change, gap/duplicate and split/dividend subchecks remain scoped N/A because this source is a pair of ranked point-in-time facts, not a price series. |
| 2.4 Freshness | Applies | R-09 freezes source business date and next-Shanghai-midnight expiry. Retry-candidate discovery plus automatic retry-admission/manual-retry-authorization transactions reject `now >= expires_at`, append/ack terminal `ExpiredFreshness`, clear active retry authority and make zero provider/renderer/sink calls; manual auth cannot revive. A BR-198 dispatcher-resolved prior-date initial acquisition is explicitly outside this retry-expiry gate. |
| 2.5 Test/live isolation | Applies | Production and nonce-bound TEST_CODE roots are physically isolated and cannot be selected or rebound by caller/CWD/environment/link paths. |
| 2.6 Order safety | N/A | No order or broker-order path is added or invoked. |
| 2.7 Audit trail | Applies | Authorization, cycle, attempt, terminal payload, append acknowledgement and exact join remain durable and hash-bound. |
| 2.8 Fake implementation | Applies | `verify`, `push`, `reconcile` and retry mutations operate on their real target authority; logging-only success is forbidden. |
| 2.9 Design contradiction | N/A to config | No `config/*.toml` change. Fixed 30/120/600 seconds and cap three are registered BR-192 delivery-governance constants. |
| 2.10 Business rules | Applies | Exact filter/order/limit/mutex/fence/terminal-slot/recovery rules are registered in the BR-192 row before implementation. |

The PR must list every rule `2.1` through `2.10`; N/A entries retain these
scoped proofs. Gate A freezes this matrix. Gate B/C/D must report each
applicable proof and may not silently omit an N/A rule.

## Self-contained adopted baseline

This plan does not normatively incorporate an untracked or independently
unaccepted design. For this slice, the adopted upstream contract is frozen
here: the dispatcher's exact review-calendar `business_date`, canonical
A-share filter, `limit=20`, one `VolumeRatio` and one `MainNetInflow` request
through `EastmoneyProviderTopNRankingRouter::new()`, preserving provider order,
exact provider `f297` row date, declared total, inspected count, batch identity
and observed time. It is a single-response Provider TopN, never a complete-
market ranking. A future review date is rejected; the current date before
15:35 is `ExpectedWait`; the current date at/after 15:35 and the dispatcher-
resolved latest-settled prior review-calendar business date are runnable; this
is not caller-selected arbitrary historical replay. The observation is created
by an explicit Asia/Shanghai conversion at the monitor context boundary and is
independent of host-local `chrono::Local`/`TZ`. The gateway freezes trusted
`request_started_at` and `capture_completed_at` observations, preserves the complete provider
capture timestamp raw bytes, and requires `request_started_at <=
provider_captured_at <= capture_completed_at` plus the requested-date and
Shanghai-midnight checks. Same-date capture before request start or after
completion is rejected. Cached, current-date-substituted or fallback evidence
is forbidden. Initial
pair evidence uses two closed `ProviderCaptureEvidenceV1` values whose exact
field manifest is `raw_timestamp_bytes: Box<[u8]>`,
`parsed_timestamp: DateTime<FixedOffset>` and
`raw_timestamp_sha256: String`; construction/read requires exactly 64
lowercase ASCII hex characters. The exact hash is
`SHA-256("stock_analysis.br198.provider_capture_raw.v1\0" ||
u64_be(raw_timestamp_bytes.len()) || raw_timestamp_bytes)`. No trim,
normalization, re-encoding or byte replacement is allowed. The closed
`ProviderTopNPairCaptureBindingV1` field manifest is exactly
`volume_ratio_capture: ProviderCaptureEvidenceV1`, then
`main_net_inflow_capture: ProviderCaptureEvidenceV1`; the counted binding
stores it in one `capture_binding: ProviderTopNPairCaptureBindingV1` field.
Compact serde JSON follows declaration order and encodes boxed slices as exact
integer arrays. No separate pair-only hash exists: the containing counted-
binding canonical SHA-256 binds both nested values and read validation rehashes
each raw field, so even an equivalent-instant byte mutation fails closed.
acquisition for a valid prior date is
runnable even if its retry expiry has passed; `expires_at` remains the first
Shanghai midnight after `source_business_date`, so a rejected delivery then
has zero retry eligibility and expiry is never extended. Retry does not call
the provider, rerank, merge, fill or rerender.

The adopted storage baseline is also frozen here: production uses the compile-
time manifest-root authority; tests use a unique nonce-bound TEST_CODE root;
CWD, environment, caller path, symlink, hardlink and ancestor replacement
cannot choose or rebind either authority. Cross-mode and real-symbol TEST_CODE
access fail closed. SQLite uses `BEGIN IMMEDIATE`, WAL, FULL synchronous,
foreign keys and five-second busy timeout. Immutable append/audit/push-log
writers preserve single-writer identity, locking, sync and tail validation.
Coordinator, immutable append and authoritative sink remain the sole mutation
authorities. `DurableDeliveryCoordinator` also solely owns the private
`ProductionFreshnessClock`; command/runtime modules may invoke no-caller-time
coordinator methods but may not construct, read or inject a production clock.
Production CLI surfaces expose no path/test/capability/clock selector.

### Counted-producer catalog execution contract

Before any counted producer, acquisition or sink is enabled, Gate B adds one
closed startup catalog keyed exactly by all 15 `PushKind::ALL` values. The
catalog and exact first-release state table are normative in design §1.1.
The Gate-B release has fourteen `DisabledNoProducer` rows and one target-state
`EnabledDurableBinding` row for exact
`push_templates::dispatch_r09_provider_top_n_outcome`, backed by
`CapitalDataGateway::provider_top_n_pair`. Both seams are **TO BE BUILT**
against fixed HEAD; dirty-worktree candidates are not accepted evidence until
conformed and tested. No second counted kind may be enabled.
Startup must reject missing/duplicate kinds and empty enabled seams/reasons,
then emit one line per kind in stable `PushKind::ALL` order:

```text
[BR-192][counted-producer] push_kind=<PushKind> enabled=durable_binding producer=<producer_seam>
[BR-192][counted-producer] push_kind=<PushKind> disabled=no_producer reason=capability_unavailable:<reason_code>
```

Disabled paths return the identical reason before acquisition and sink. R-09
orders its authorities exactly as: BR-194/BR-198 static date preflight;
BR-200 `inspect_review_task_occurrence`; catalog permit; provider; renderer;
durable counted delivery. `Some(evidence)` maps only through
`review_outcome_from_existing_durable`; durable preflight error, missing
hydration, corrupt authority or ambiguous authority fails closed. Every branch
before `None` uses zero permit/provider/renderer/sink. Only `None` may obtain
the sole non-cloneable, kind/seam-bound `CountedProducerPermit` and acquire
data. Every generic counted entry revalidates the permit/binding, and the
all-15 caller inventory must cover multiline/template-ID/label indirection.
The enabled row still needs a real push-log plus exact `push.delivery.audit`
join at Gate D.

The exact BR-200 mapping is:

| occurrence | outcome | retryable | next attempt | reason code |
| --- | --- | --- | --- | --- |
| Delivered + valid hydration | original Delivered | false | none | original durable reason |
| Delivered + missing hydration | Failed | true | reconciliation schedule | `durable_occurrence_delivered_hydration_pending` |
| Rejected or Uncertain | Failed | false | none | `durable_occurrence_terminal_failure` + exact stored state |
| non-terminal | Failed | true | reconciliation schedule | `durable_occurrence_nonterminal_reconciliation_pending` |
| corrupt/mismatched/ambiguous | Failed | false | none | exact typed durable invariant reason |
| missing | continue | n/a | n/a | none |

Every existing-occurrence row has zero permit/provider/renderer/sink calls.
R-09 producer, schedule transition and hydration persist and verify the exact
ordered rule vector `[BR-110, BR-140, BR-192, BR-194, BR-198, BR-200]`.
BR-192 Gate B/release cannot proceed until BR-200 has independently accepted
Gate C evidence with R-09 still disabled. BR-198 is a supporting contract, not
a separately executable prerequisite: Task 8 implements its date/capture/
dependency requirements atomically with the R-09 gateway and producer that
BR-192 creates. BR-198 receives no standalone Gate-B/C acceptance credit;
untracked candidate code is not authority.
BR-192 reserves BR-202 as the future release-evidence owner without violating
the repository's no-spec-on-unverified-gate order: every planned BR-192 Gate-B
source path cites literal `BR-202`, but this batch neither claims nor mutates
the current BR-202 Code cell and gives the current BR-202 candidate zero
acceptance credit. BR-202 Gate A and all later BR-202 progression start only
after BR-192 Gate C; that later accepted Gate-A object registers the
already-accepted BR-192 paths and its Gate B creates the isolated wrapper.
BR-192 Gate D cannot be minted until BR-202 Gate B/C has delivered that
wrapper.

---

## Mandatory implementer brief

**Upstream debt**

- Durable schema v5 is the fixed-HEAD repository baseline and already owns every BR-194
  terminal-replay object, audit kind and manifest semantic. Task 2 owns one
  additive v5-to-v6 migration containing only the BR-192 companion authorities;
  it must preserve the accepted v4-to-v5 upgrade and every v5 BR-194 row/object
  unchanged. Fresh v0 plus v1/v2/v3/v4/v5 upgrades must converge to the same
  complete v6 manifest.
- `DurableDeliveryCoordinator::reacquire_rejected` currently trusts a
  row-level `retry_authorized` projection and cannot return typed deferrals.
- `DeliveryEnvelope.retry_authorized` still exists and is written during
  initial decision insertion; it must not remain an authority.
- startup reconciliation resumes Reserved work separately and has no
  cycle-global identity attempt guard.
- current `ProductionStorageSnapshot` protects SQLite artifacts but not push
  logs, immutable audit, event audit or receipt roots.

**Rename impact**

- Replace `reacquire_rejected` with `admit_authorized_retry`.
- Add only
  `begin_retry_cycle_before_spawn(namespace_sha256, owner_boot_identity,
  scheduled_for, now)`; do not add or call a second `begin_retry_cycle` alias.
- Before that begin, add the single recovery-only
  `resume_same_boot_retry_cycle_terminal_slots(append,
  current_owner_boot_identity, now)` coordinator method. It may resume only
  same-boot `CompletionAppended|CompletionPending|FailureAppended|
  FailurePending` to their exact stored terminal kind and has no sink
  capability. In one `BEGIN IMMEDIATE`, begin must compute
  `MAX(cycle_ordinal)+1`, derive the exact proposed cycle identity, and only
  then reject any remaining global Running row before any cycle/`Started`
  insertion. The no-commit branch must query zero proposed rows, roll back and
  return the exact bound proof; the empty branch atomically persists the same
  ordinal/identity with `Running` and `Started`.
- Acquire and retain the process-lifetime boot authority before opening the
  coordinator/append authority. Pass its validated identity explicitly to
  cycle creation and startup recovery; coordinator code must not read a boot
  identity from globals, environment variables or process helpers.
- Split retry candidate authority from the compatibility
  `retry_authorized` projection.
- Every use in `src/durable_delivery/coordinator.rs`,
  `src/bin/monitor/durable_delivery_runtime.rs`,
  `src/durable_delivery/tests.rs`,
  `tests/durable_delivery_counted_cutover.rs` must move with its owning type.
  Public root exports are not moved incrementally: Task 8 performs one atomic
  edit that preserves existing unrelated exports and exposes the frozen
  complete BR-192 manifest after every owner exists.

**Current compile inventory (must move atomically)**

- The authoritative source baseline is
  `HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496`. The exact source blobs used by
  this plan are:

  ```text
  src/durable_delivery/model.rs                 1b5561865674a09266971469f703649c8d299c38
  src/bin/monitor/main.rs                       80be9ddea0eb088194e2daab9a40bfa3067f00a5
  src/bin/monitor/review_batch.rs               99d89da9454a13af6adc52cc239cc690b8770029
  src/bin/monitor/push_templates.rs             2388dce7887a95feee13eadbef6129efdb942f61
  src/bin/monitor/notify.rs                     b7a15cbd46ef7620ec341d2eee98dd76f62560d8
  src/bin/monitor/v14_adapter.rs                535d17d964b40f97fa89adb765a0ecb9f02441bf
  src/bin/monitor/durable_delivery_runtime.rs   a635b90237413577a51d5bc92ae29c40ae2afac4
  src/durable_delivery/schema.rs                794491f8445374af44ee52e57ba2358db7f9c262
  src/durable_delivery/coordinator.rs           b99fbb3d3ad44017b1d06d07723e2b0b338c6821
  src/durable_delivery/mod.rs                   cf4e80421763c5f57162aa8518145a72f5329625
  src/durable_delivery/tests.rs                 35c794a196b4551d7a16920ef5a4a3c99a3f8fb5
  src/auth/operator.rs                          b0ec1f0b218466493dabb0a6e560099d07e19cf2
  Cargo.toml                                    2118a3e490efe2d3416b2554559ca0347947c533
  ```

  Reproduce them without reading the worktree:

  ```bash
  git rev-parse b4aeee68d2c0259cc968914b3d39e3a89a18a496
  git rev-parse \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/model.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/main.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/review_batch.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/push_templates.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/notify.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/v14_adapter.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/durable_delivery_runtime.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/schema.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/coordinator.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/mod.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/tests.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/auth/operator.rs \
    b4aeee68d2c0259cc968914b3d39e3a89a18a496:Cargo.toml
  ```

  Fixed HEAD already contains exactly one `ReviewTask::R09`, in
  `ReviewTask::ALL` after R08 and before A10, with label `R-09` and dependency
  `SourceOnly`. Gate B preserves those definitions and adds only the missing
  real producer plus central SourceOnly dispatch/typed merge.

  ```bash
  git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/review_batch.rs | sed -n '296,366p'
  git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/push_templates.rs | rg -n '^#\[cfg\(test\)\]|^mod tests|PushKind::(CloseCall|ForbiddenOps|ReviewFailure)'
  ```

  The fixed-object outputs include:

  ```text
  pub enum ReviewTask {
      R02,
      R03,
      R04,
      R05,
      R06,
      R08,
      R09,
      A10,
      A01,
  }
  ...
  Self::R04 | Self::R09 => ReviewTaskDependency::SourceOnly,
  ...
  8898:#[cfg(test)]
  8899:mod tests {
  13882:                crate::notify::PushKind::ForbiddenOps,
  13985:            let ok = crate::notify::push_governor(&banner_text, crate::notify::PushKind::CloseCall)
  14246:                crate::notify::push_governor(&banner_text, crate::notify::PushKind::ReviewFailure)
  ```

  Therefore the fixed object proves R-09 is `SourceOnly`, and fixes the
  test-only boundary at `8898/8899` with calls at `13882/13985/14246`; those
  test calls are not production callers.

- The existing sink seam is
  `AuthoritativeSink = Arc<dyn AuthoritativeSinkPort>` and
  `AuthoritativeSinkPort::deliver(&AuthoritativeDeliveryRequest) ->
  AuthoritativeSinkResult`; no `dyn AuthoritativeSink` or generic `SinkResult`
  type exists.

  ```text
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/model.rs | rg -n -U 'pub type AuthoritativeSink\s*=|trait AuthoritativeSinkPort[\s\S]{0,800}fn deliver'
  1086:pub trait AuthoritativeSinkPort: Send + Sync {
  1087:    fn sink_identity(&self) -> &str;
  1088:    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult;
  1091:pub type AuthoritativeSink = Arc<dyn AuthoritativeSinkPort>;
  ```

- `RuntimeState` currently has five struct literals: the runtime constructor
  and four test literals in
  `src/bin/monitor/durable_delivery_runtime.rs`. Adding retry fields requires
  updating all five atomically in the same compile step. This inventory is
  taken from the fixed-HEAD blob, not the dirty worktree or an earlier snapshot.

  ```text
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/durable_delivery_runtime.rs | rg -n 'RuntimeState\s*\{'
  66:struct RuntimeState {
  1096:    Ok(Arc::new(RuntimeState {
  1734:        let state = Arc::new(RuntimeState {
  3825:        let state = RuntimeState {
  3887:        let state = RuntimeState {
  3925:        let restarted = RuntimeState {
  ```

- `ReconcileSummary` remains its existing eight-field type. Its single
  constructor in `coordinator.rs` and both runtime consumers
  (`reconcile_startup_blocking`, `reconcile_current_decision`) continue to use
  the existing `progress_count`; retry candidates and cycle results belong to
  separate APIs. Do not invent `authorized_retry_candidates` on this summary.

  ```text
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/model.rs | sed -n '1161,1170p'
  pub struct ReconcileSummary {
      pub provider_calls: usize,
      pub sink_calls: usize,
      pub progress_count: usize,
      pub locally_pending_decisions: Vec<String>,
      pub deliverable_decisions: Vec<String>,
      pub non_progressable_foreign_attempts: Vec<String>,
      pub non_progressable_manual_reviews: Vec<String>,
      pub schedule_hydrations: Vec<ScheduleHydration>,
  }
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/coordinator.rs | rg -n 'ReconcileSummary\s*\{'
  5472:            Ok(ReconcileSummary {
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/bin/monitor/durable_delivery_runtime.rs | rg -n 'fn reconcile_startup_blocking|fn reconcile_current_decision|summary\.progress_count'
  1131:fn reconcile_startup_blocking(state: &RuntimeState) -> Result<StartupReconcileEvidence, String> {
  1144:        progress_count += summary.progress_count;
  1229:fn reconcile_current_decision(
  1240:        if summary.progress_count == 0 {
  ```

- Tokio's current `full` feature does not promise `test-util`. Task 8 adds
  explicit `test-util` to the existing Tokio feature set before using
  `#[tokio::test(start_paused = true)]`, `pause` or `advance`.

  ```text
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:Cargo.toml | rg -n '^tokio = '
  28:tokio = { version = "1.49.0", features = ["full", "rt"] }
  ```

- `src/durable_delivery/mod.rs` currently declares only `coordinator`, `model`
  and `schema`. Task 6 adds only the private command-module declaration, and
  Task 7 adds only the private evidence-module declaration. Task 8 adds both
  CLI sources, the sole atomic BR-192 root-export edit, both `[[bin]]` stanzas
  and both integration-test `CARGO_BIN_EXE_*` references in one compile step.

  ```text
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/mod.rs | rg -n '^mod (coordinator|model|schema);$'
  8:mod coordinator;
  9:mod model;
  10:mod schema;
  ```

- Existing schema-migration coverage must be preserved and extended for v6
  while retaining fresh/v1/v2/v3/v4/v5 input coverage, the existing v1/v2
  invalid-history regressions and byte-compatible preservation of the v5
  BR-194 manifest and rows.

  ```text
  $ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:src/durable_delivery/tests.rs | rg -n '^fn br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions'
  2132:fn br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions() {
  ```

  Do not rename that accepted BR-194 identifier. Extend its fixture/assertion
  coverage in place and add separately named BR-192 v6 companion tests.

- The current real operator-auth boundary has a public
  `require_monitor_operator_auth` and private platform-specific
  `try_pam_auth`; `OperatorAuthAttestation` and
  `authenticate_monitor_operator` are Task 6 additions, not current APIs.
  Task 6 must refactor the existing real PAM call instead of inventing a
  second authentication path.

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

**Atomic implementation constraints**

- Every new canonical/event/CLI/evidence enum or struct that accepts external
  serialized input derives `Serialize` and `Deserialize` with the existing
  debug/clone/equality traits and `deny_unknown_fields`. The opaque
  `RetryCycleFailure`, `RetryCycleEvidence`, live capabilities and consumed
  permits are explicitly not deserializable or `Default`.
- After Task 8, rerun each command above. Expected deltas are: all five
  `RuntimeState` literals compile with the new fields; `ReconcileSummary`
  remains exactly eight fields/one constructor/two `progress_count` consumers;
  Tokio includes `test-util`; `rusqlite` includes `functions`; the two new
  private modules and the one complete root export manifest exist; and the v6
  migration test retains fresh/v1/v2/v3/v4/v5 fixtures plus BR-194 preservation.
  The PAM inventory must show the
  existing real call remains the single underlying authentication path and the
  new attestation API delegates to it.

**Production evidence**

- startup log: `[DurableDelivery][BR-192] provider-free retry runner started`
- immutable record kinds:
  `RetryAuthorization`, `RetryAuthorizationEvent` and `RetryCycleAudit`
- observation event type: `delivery.retry.cycle`
- counted delivery event type remains `push.delivery.audit`
- exact production counted artifacts:
  `data/push_log/YYYY-MM-DD/*_audit_pending.json` and
  `data/push_log/YYYY-MM-DD/*_committed.json`
- exact observation authority:
  `data/event_bus/YYYY-MM-DD.jsonl`; no alternate event type/path counts
- a real retried counted terminal must join decision, immutable attempt binding,
  historical disposition/authorization/binding generation, reservation
  generation, attempt, sink result and immutable audit refs; otherwise Gate D
  remains Blocked rather than fabricating success.

## Exact file map

### Documentation

- Add `docs/superpowers/specs/2026-07-30-br192-provider-free-retry-design.md`
- Add `docs/superpowers/plans/2026-07-30-br192-provider-free-retry.md`
- Modify `docs/business_rules.md`

### Library

- Modify `src/auth/operator.rs`
  - owns real PAM success attestation, authority timestamp, subject, service,
    mechanism and OS-CSPRNG nonce
- Modify `src/durable_delivery/model.rs`
  - owns frozen retry DTOs, namespace preimage/hash helper, serde validators
    and the private compatibility-safe counted-producer attestation projection
    on `DeliveryEnvelope`
- Create `src/durable_delivery/counted_producer_catalog.rs` in Task 1
  - sole owner/constructor of the non-cloneable, non-serializable permit and
    canonical 15-row catalog identity; Task 8 only integrates this existing
    private module with the real producer and final root export
- Modify `src/durable_delivery/schema.rs`
  - owns central deterministic `sha256_hex` UDF registration/self-test,
    the one v5-to-v6 manifest/migration and atomic retry
    attempt-binding/schedule/Reserved constraints
- Modify `src/durable_delivery/coordinator.rs`
  - owns admission transaction, existing-binding preparation and appended-start
    validation/read-back plus the pre-call ownership CAS; every durable
    Rust/rusqlite connection preserves the existing bootstrap order
    `open → main attestation → WAL/SHM materialization+attestation → bound
    connection validation → UDF registration+self-test → configuration →
    schema/migration/validation`
- Create `src/durable_delivery/retry_command.rs`
  - consumes the opaque auth attestation and owns the unopened resolver
    boundary and private resolved capability target
- Create `src/durable_delivery/retry_evidence.rs`
- Modify `src/durable_delivery/mod.rs`
  - declares new modules privately in their owning tasks, then performs one
    atomic public-export edit in final integration Task 8, preserving every
    unrelated existing export
- Modify `src/durable_delivery/tests.rs`

### Runtime and CLI

- Modify `src/bin/monitor/durable_delivery_runtime.rs`
- Modify `src/bin/monitor/main.rs`
- Modify `src/bin/monitor/review_batch.rs`
- Modify `src/bin/monitor/push_templates.rs`
  - creates the real R-09 producer, acquires its permit before the gateway and
    freezes expiry/binding before durable dispatch
- Modify `src/bin/monitor/notify.rs`
  - guards `push_governor*`, counted sink and exact push-log/audit artifacts
- Modify `src/bin/monitor/v14_adapter.rs`
  - preserves exact counted-kind mapping and rejects unpermitted generic paths
- Create `src/data_gateway/capital.rs`
  - owns the target-state real `CapitalDataGateway::provider_top_n_pair`; a
    dirty-worktree file with this name is only candidate implementation
- Modify `src/data_gateway/mod.rs`
- Modify `src/lib.rs`
- Create `src/bin/authorize_delivery_retry.rs`
- Create `src/bin/verify_br192_retry_evidence.rs`
- Modify `Cargo.toml`
  - add `rusqlite` feature `functions` while retaining `chrono`; replace the
    fixed-HEAD path-only Magic TDX dependency and add the exact unified
    release set below
- Modify `Cargo.lock`

### Integration and isolation tests

- Create `tests/durable_delivery_counted_cutover.rs`
- Create `tests/br192_counted_producer_catalog.rs`
- Create `tests/magic_market_release_revision.rs`
  - retain the exact fourteen direct names, fifteen lockfile names, sole
    transitive transport, repository, revision and version assertion as the
    executable BR-192/BR-198 dependency authority
- Modify `tests/monitor_help_isolation.rs`
- Create `tools/release/disable_br192_periodic_retry.patch`
  - this checked-in forward patch targets only the single periodic runner
    installation in `src/bin/monitor/main.rs`; Task 8 verifies it applies to
    the exact release source and does not alter R-09, BR-200, schema, catalog,
    dependencies, reconciliation or audit code
- Create `tools/release/verify_br192_forward_rollback.sh`
  - validates the patch has one exact target, applies it to an isolated
    worktree at a literal accepted release SHA, proves the semantic diff,
    builds the rollback monitor and runs the patched-tree v6 recovery tests

### Shared compliance and release verification

- Modify `tools/compliance/check.sh`
- Create `tools/compliance/lib/check_br192_provider_free_retry.sh`
  - validates the complete v6 manifest and recomputes canonical hashes;
    logging-only/schema-name-only success is forbidden
- Modify `tools/compliance/lib/check_br194_review_dependency.sh`
  - recognizes the same v6 manifest without weakening any BR-194 invariant
- Modify `tools/release/verify_br194_review_join.py`
  - independently hashes returned canonical BLOBs with `hashlib.sha256` and
    inspects schema version/trigger catalog read-only; it registers no SQLite
    callback, executes no DML/trigger and continues to verify every preserved
    BR-194 replay join

No legacy provider is added. The only provider/Gateway production delta is the
new Magic/Eastmoney-backed R-09 `CapitalDataGateway` consumer above; retries
themselves still call zero provider, Gateway or renderer paths.

Starting from clean fixed HEAD, Task 8 applies the complete BR-198 unified
release dependency set; dirty-worktree declarations are not prerequisites:

```toml
magic-tdx-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-market-core = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-market-router = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-market-composition = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-eastmoney-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-ths-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-sina-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-cninfo-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-tencent-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-cls-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-jin10-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-thepaper-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-exchange-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
magic-baidu-rs = { git = "https://github.com/Northofqing/magic-market-data-rs.git", rev = "5f1ce93656a55854c844065390520cd4aecd9a14", version = "=0.2.0" }
```

`Cargo.lock` must resolve those fourteen direct packages plus the transitive
`magic-market-transport` package—exactly fifteen Magic packages—to that one
source revision and exact 0.2.0 package version. The transport crate is not a
direct dependency or application data-source API. R-09's new `capital.rs`
lands only the Top-N capability in this slice; unrelated fund-flow/northbound
candidate methods are not adopted merely because their crates are installed.

## Gate A documentation tracking

Historical Gate A tracking result:

- reviewed design blob:
  `cdeec30f46c18bcbdb45ef12782943b90d1533e6`;
- reviewed plan blob:
  `6d04e26f563c9fbb455faef789daf84c17221fab`;
- reviewed BR-192 row SHA-256:
  `d3010a1a7a408f8b4ba976de32a0fc046ba6b4f09907d3f2374e1029106cb8ce`;
- independent result: `Critical=0 / Important=0 / Minor=1`;
- Minor-1 closure: root exact-staged the reviewed row into index blob
  `1682b36b3d52ab15c3326ea4d7ebee5628a22db7`; the masked non-BR-192
  digest was equal before and after staging.

The first metadata review found C0/I1/M0 because these identities omitted the
closed counted-producer startup catalog. Its attempted repair was rejected at
C0/I2/M0. The next exact reviews found C1/I6/M1 and C0/I3/M0, then C0/I4/M0
and C1/I3/M1; a subsequent fixed-object pair found C1/I7/M0 and C1/I3/M1. Design
§0.2/§0.4/§1.1/§1.2/§2.0-§2.4/§10.1, this plan and the BR-192 row now repair the
real-consumer, expiry, exact-evidence, all-caller enforcement, fixed-HEAD test,
version, permit, provenance, expiry-drain, dependency, RED/GREEN and first-file-action defects and therefore create new contract
identities. Gate B must not resume until a fresh independent review of
the exact design/plan/row identities returns C0/I0. Gate B/C/D and live
evidence remain pending.

## Task order and dependency graph

```text
Task 1 BR registration/types/catalog contract
  -> Task 2 schema
    -> Task 3 authorization recovery
      -> Task 4 typed admission/cycle audit
        -> Task 5 unit and cross-process tests
          -> Task 6 authenticated command library types
            -> Task 7 read-only evidence library types
              -> Task 8 one atomic R-09 producer/root/runtime/CLI/static/isolation integration
                -> Task 9 Gate B/C/D evidence
```

Tasks must be completed in this order. The deferred runtime recipe below is
reference material for Task 8 and must not be compiled or committed before
Tasks 6 and 7 define every symbol in the frozen root manifest. A failed test
returns to the task that owns the violated state transition.

Every `BR-192 RED: named contract is not implemented` body below is an exact
temporary RED sentinel. The owning task must first prove its named command
selects exactly one test and fails at that sentinel, then replace every sentinel
in its owned files with the concrete assertions specified by that task before
claiming GREEN or committing. Empty bodies, `0 tests`, ignored parent tests,
unrelated compile failures and a sentinel that still exists at GREEN are
invalid evidence. The ignored process-child helpers are invoked by their real
parent harnesses; the parent must fail during RED. Task 8 must finally prove:

```bash
! rg -n -U '(async\s+)?fn\s+br192_[A-Za-z0-9_]+\(\)\s*\{\s*\}' src tests
! rg -n 'BR-192 RED:' src tests
```

The four root/constructor tests in Task 8 are stricter: their Step-2 RED bodies
are real compile/assertion bodies, never sentinels, as specified there.

### Task 1: Register the rule and freeze public types

**Files:**

- Create `tests/durable_delivery_counted_cutover.rs` as the first Gate-B file
  operation; no source or rule file may be edited first
- Modify `docs/business_rules.md`
- Modify `src/durable_delivery/model.rs`
- Modify the private `src/durable_delivery/counted_producer_catalog.rs`
  created and tested in Task 1; do not recreate or move its authority types
- Modify `src/durable_delivery/mod.rs` only to declare the catalog module
  privately; no BR-192 public re-export occurs in this task
- Test `src/durable_delivery/tests.rs`

- [ ] **Step 0: Create the cutover RED test before every other Gate-B edit**

The first filesystem mutation after Gate A acceptance must be an
`apply_patch` Add File for `tests/durable_delivery_counted_cutover.rs` with
this exact single test; do not copy the dirty-worktree file:

```rust
#[test]
fn br192_counted_cutover_requires_exact_catalog_permit_and_expiry_drain() {
    panic!("BR-192 RED: permit-bound R-09 and active expiry drain are absent");
}
```

Run it immediately and require `running 1 test`, the exact named failure and a
non-zero exit. `0 tests`, compile failure unrelated to this file, or a passing
test is invalid RED. The file remains an untracked failing sentinel throughout
Tasks 1-7 and is excluded from every intermediate `git add`/commit. Task 8
replaces it with the complete cross-module assertions before the first commit
that stages this file; the temporary panic is never committed. Tasks 1-7 run
only their listed targeted tests, and the first full `cargo test` is forbidden
until Task 8 has replaced this sentinel.

```bash
cargo test --test durable_delivery_counted_cutover br192_counted_cutover_requires_exact_catalog_permit_and_expiry_drain -- --exact --test-threads=1
```

Expected RED evidence is exactly `running 1 test`, the named test and
`BR-192 RED: permit-bound R-09 and active expiry drain are absent`; any other
compile error or test count is invalid and must be repaired before continuing.

- [ ] **Step 1: Amend BR-192 before logic changes**

Register:

- exact 14-disabled/one-enabled catalog, non-constructible
  `CountedProducerPermit`, exact R-09 producer/gateway seams and the exhaustive
  all-15 generic/counted-specific caller guard from design §1.1-§1.2;
- unique active companion binding, current appended rejection disposition,
  appended/applied authorization identity and its appended `Applied`
  authorization event as the only retry authority;
- immutable attempt binding freezing authorization, disposition, binding and
  reservation generations; active binding clear on new disposition/terminal
  while historical authority remains valid;
- exact stable order: Reserved rows require non-null current-attempt identity
  and sort by `business_date ASC, created_at ASC, decision_identity ASC,
  current_attempt_identity ASC`; prior-boot Running rows require all tie keys
  and sort by terminal-phase recovery priority
  `CompletionAppended, CompletionPending, FailureAppended, FailurePending,
  NotPrepared`, then `scheduled_for ASC, started_at ASC, cycle_identity ASC`;
  retry candidates require non-null identity/schedule keys, current eligibility
  and non-exhaustion, then sort by
  `next_eligible_at ASC, decision_identity ASC,
  rejection_disposition_identity ASC, authorization_identity ASC`; every one of
  the three selectors returns one complete validated snapshot and its frozen
  count/hash, with no SQL `LIMIT`, `OFFSET`, cursor, caller cardinality or
  continuation; no rowid, implicit null order, unordered iteration or partial
  vector is allowed; prior-boot and Reserved snapshots are fully consumed
  before the next phase, while the retry-candidate snapshot is taken once and
  later authorizations belong to the next cycle; retain one cycle-global
  attempted set across Reserved/retry;
- exact same-boot safe-terminal order before every new begin:
  `CompletionAppended, CompletionPending, FailureAppended, FailurePending`,
  then `scheduled_for ASC, started_at ASC, cycle_identity ASC`; the selector
  requires `owner_boot_identity=current`, materializes the complete validated
  snapshot, resumes only the stored terminal kind with coordinator+append and
  zero sink, and reaches an empty fixed point before begin. In the same
  `BEGIN IMMEDIATE`, begin first computes the next retained positive unique
  `cycle_ordinal`, derives the proposed identity from exact
  `namespace_sha256,owner_boot_identity,scheduled_for,started_at(now),
  cycle_ordinal`, then orders any remaining global Running row by
  `started_at ASC, cycle_identity ASC`. A retained row requires zero proposed
  cycle/`Started` queries before rollback and returns definite
  `RetryCycleAlreadyRunning + NoRetryCycleCommitted`; proof consumption in a
  fresh `BEGIN IMMEDIATE` must rederive the same next ordinal/identity,
  byte-match the selected Running witness and reprove zero proposed rows.
  Concurrent insert/state/identity mismatch rejects consumption and latches
  the guard. An empty global check atomically inserts only that exact
  ordinal/identity with `Running` and `Started`;
- Reserved processing before candidate re-query and independent
  `DuplicateSuppressed` evidence;
- mandatory prepare -> append/ack `SinkAttemptStarted` -> transactional
  `Reserved -> AttemptInFlight` send-ownership claim -> single-use execute seam;
- a retained consumed send-ownership row and prior-boot
  `SinkAttemptStarted`/`AttemptInFlight` quarantine as
  `ProcessInterruptedAfterSinkStart`; recovery makes zero sink calls and
  requires the uncertainty/manual-review path;
- ordinary cycle error and caught panic share one failure finalizer that
  quarantines every qualifying same-cycle start/consumed ownership/in-flight
  attempt and append/acknowledges uncertainty before cycle `Failed`;
- `retry_attempt_start_audit_unavailable` uses its unchanged-state exception
  only when persisted state proves no acknowledged start and no consumed
  ownership; all three closed reason codes preserve byte-identical
  decision/attempt/binding/schedule rows, hashes and pending bytes in that
  pre-start branch, while acknowledged-start or consumed-ownership state
  rejects the exception and uses ordinary quarantine before `Failed`;
- one irreversible terminal phase; completion and failure each use
  prepare/append/ack/CAS, and after either Pending/Appended slot exists no path
  may create the opposite kind; boot recovery resumes the exact stored kind;
- namespace/hash/owner validation occurs before guard acquisition; a begin
  rollback may release only with coordinator-issued
  `NoRetryCycleCommitted`, while commit-ambiguous and unsafe post-claim exits
  remain latched;
- production manual authorization is constructible only through the
  library-owned PAM/fixed-resolver entry, exposes injection only under
  `#[cfg(test)]`, and binds `authorized_at` exactly to authority
  `validated_at`;
- 30/120/600-second delivery-retry backoff and maximum three automatic
  attempts;
- immutable R-09 `source_business_date` and first-next-Shanghai-midnight
  `expires_at`; the complete expiry selector/drain terminalizes every excluded
  Active row, while discovery and admission/manual/pre-start/pre-claim
  transactions terminalize exact audited `ExpiredFreshness` at equality or
  later, make zero calls and cannot be revived;
- retry authority is restricted to exact persisted R-09 kind/seam/catalog/
  attestation provenance; v5/null provenance and all fourteen disabled kinds
  are typed-ineligible before automatic or PAM authorization append;
- retained monotonic retry schedules that cannot be deleted, reset or moved
  earlier;
- immutable audit for every deferral/ineligibility;
- pre-spawn durable cycle identity plus panic/JoinError/process-death recovery;
- ASCII space/tab/LF/CR-aware non-empty validation for every new immutable ref;
- three Uncertain states excluded from candidate and sink paths;
- evidence `require_count` is exactly `1..=256`; CLI and library both reject
  zero or 257+ before authority resolution/open, streamed exact joins retain at
  most 256 distinct results, and the 257th distinct complete join returns a
  typed bound error with no partial output; byte-identical artifact replay is
  accepted and deduplicated without increasing the verified count, while reuse
  of the same logical tuple with different canonical bytes or hash is a typed
  conflicting-duplicate error;
- TEST_CODE-only command and cross-process test isolation, including distinct
  writer/recovery boot processes for completion/failure Pending/Appended and
  the persisted failure corruption matrix.

Record that these are delivery-governance constants, not financial thresholds.
Register the single schema-v6 transition-event/hash authority and
forward-compatible rollback rule in the same BR-192 row.

- [ ] **Step 2: Write failing public-contract tests**

Add tests with these exact names:

```rust
#[test]
fn br192_retry_contract_has_no_foreign_live_attempt_variant() {
    panic!("BR-192 RED: retry state contract is not implemented");
}

#[test]
fn br192_each_uncertain_state_is_explicitly_ineligible() {
    panic!("BR-192 RED: uncertainty exclusions are not implemented");
}

#[test]
fn br192_retry_backoff_and_cap_are_fixed_delivery_governance() {
    panic!("BR-192 RED: retry backoff contract is not implemented");
}

#[test]
fn br192_retry_namespace_hash_is_canonical_domain_separated_and_utf8_stable() {
    panic!("BR-192 RED: namespace hash contract is not implemented");
}

#[test]
fn br192_retry_namespace_hash_rejects_invalid_fields_and_tampering() {
    panic!("BR-192 RED: namespace rejection contract is not implemented");
}

#[test]
fn br192_retry_error_contract_owns_exact_typed_variants_and_fields() {
    panic!("BR-192 RED: typed retry errors are not implemented");
}

#[test]
fn br192_retry_cycle_identity_hash_is_domain_separated_field_ordered_and_utf8_stable() {
    panic!("BR-192 RED: cycle identity hash is not implemented");
}

#[test]
fn br192_retry_cycle_identity_rejects_domain_schema_field_order_timestamp_and_ordinal_mutations() {
    panic!("BR-192 RED: cycle identity mutation rejection is not implemented");
}
```

These are deliberate, uncommitted RED sentinels. Step 4 must replace every
body with the exact assertions described below before Step 6 can pass or any
Task-1 commit can be created. An empty `{}` body is forbidden because it would
produce a false GREEN.

The second test enumerates:

```rust
[
    DecisionState::UncertainAuditPending,
    DecisionState::UncertainTaskTransitionPending,
    DecisionState::UncertainManualReview,
]
```

- [ ] **Step 3: Run the contract tests and prove RED**

```bash
cargo test --lib durable_delivery::tests::br192_retry_contract_has_no_foreign_live_attempt_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_each_uncertain_state_is_explicitly_ineligible -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_backoff_and_cap_are_fixed_delivery_governance -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_namespace_hash_is_canonical_domain_separated_and_utf8_stable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_namespace_hash_rejects_invalid_fields_and_tampering -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_error_contract_owns_exact_typed_variants_and_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_identity_hash_is_domain_separated_field_ordered_and_utf8_stable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_identity_rejects_domain_schema_field_order_timestamp_and_ordinal_mutations -- --exact --test-threads=1
```

Expected: every command exits non-zero. A compile failure caused by a named
missing BR-192 symbol is valid RED and need not contain a test-count line. If
compilation succeeds, output must contain `running 1 test` and that exact test
must fail; `0 tests` or a failure unrelated to the new contract is invalid
RED. Record all eight command outputs.

- [ ] **Step 4: Add the exact types**

Add the design enums to `model.rs`. Create the permit, attestation, denial and
acquisition definitions below directly in their sole owner
`counted_producer_catalog.rs`, and add only
`mod counted_producer_catalog;` to `src/durable_delivery/mod.rs`; do not root
re-export the module or any new symbol yet:

```rust
// Single library owner: src/durable_delivery/counted_producer_catalog.rs.
// Fields/constructors stay private; the exact opaque surface is root-exported.
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

// CountedProducerDenied is intentionally a public, constructible closed error
// value. It carries no authority: only the permit and attestation are opaque,
// privately constructible authority values, and a denial can mint neither.

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

// attestation_canonical is exactly
// b"stock_analysis.durable_delivery.br192.counted_producer_attestation.v1\0"
// || serde_json::to_vec(&CountedProducerAttestationPreimageV1). The preimage
// excludes producer_attestation_sha256. Compute the digest first, then project
// it into CountedProducerAttestationEvidenceV1 and DeliveryEnvelope.
// Validation rebuilds the preimage from the projection, strips the exact prefix,
// requires decode -> reserialize equality, recomputes the digest and compares
// projection, typed columns and SQL sha256_hex. The evidence projection itself
// is never serialized as the digest preimage.

// Add privately to DeliveryEnvelope. None must preserve legacy canonical bytes.
#[serde(default, skip_serializing_if = "Option::is_none")]
counted_producer_attestation: Option<CountedProducerAttestationEvidenceV1>,

// Coordinator-owned complete expiry API. None of these methods filters an
// expired Active row out of candidate visibility without terminalizing it.
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

pub enum RetryCandidateSnapshot {
    ExpiredFound(CompleteRetryExpirySnapshot),
    Candidates(CompleteRetryCandidateSnapshot),
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

pub enum RetryExpiryTerminalKind {
    RejectedDurableExpired,
    ReservedExpiredBeforeSink,
    ManualTargetExpiredBeforeAuthorization,
}

pub enum RetryScheduleTerminalState {
    Active,
    ExpiredFreshness,
    Exhausted,
    Completed,
}

impl DurableDeliveryCoordinator {
    pub fn select_expirable_retry_schedules(
        &self,
    ) -> Result<CompleteRetryExpirySnapshot>;
    pub fn prepare_retry_expired_freshness(
        &self,
        row: &ExpirableRetrySchedule,
    ) -> Result<RetryExpiryPreparationOutcome>;
    pub fn reconcile_retry_expiry_preparation(
        &self,
        append: &dyn ImmutableAppendPort,
        prepared: RetryExpiryPreparationOutcome,
    ) -> Result<RetryExpiryDisposition>;
}

`PreparedRetryExpiryUncertainty` has no public constructor, `Clone`,
`Deserialize` or `Default`; only the coordinator can derive it from retained
state. `reconcile_retry_expiry_preparation` may return
`RetryExpiryDisposition::RoutedToUncertainty` only after the uncertainty
outboxes are append/acknowledged to a fixed point, the active binding is
cleared and the schedule is no longer `Active`; this branch writes no expiry
outbox. The unique transaction priority is: reject partial/corrupt matches;
resume a complete pre-call triple/Appended expiry; preserve a terminal result;
route current start/ordinary ownership to uncertainty except for the live-
permit final-pre-call no-call protocol; prepare definite expiry only when no
current start/ownership exists; otherwise continue while fresh. Historical
terminal-attempt rows do not win this priority.

// Add to the existing DurableDeliveryError enum. The existing public root
// export already exposes DurableDeliveryError; do not add a duplicate export.
RetryAttemptStartAuditUnavailable {
    attempt_identity: String,
    reason_code: String,
}
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

pub const MAX_AUTOMATIC_RETRY_ATTEMPTS: u32 = 3;
pub const RETRY_BACKOFF_SECONDS: [i64; 3] = [30, 120, 600];

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetryCycleIdentityPreimageV1 {
    pub schema_version: u8,
    pub rule_id: String,
    pub namespace_sha256: String,
    pub owner_boot_identity: String,
    pub scheduled_for: String,
    pub started_at: String,
    pub cycle_ordinal: i64,
}

pub(crate) fn retry_cycle_identity_sha256(
    preimage: &RetryCycleIdentityPreimageV1,
) -> Result<String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryAuthorizationSource {
    FrozenRejection,
    ManualOperator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryCandidate {
    pub decision_identity: String,
    pub rejection_disposition_identity: String,
    pub authorization_identity: String,
    pub push_kind: PushKind,
    pub producer_seam: String,
    pub producer_catalog_identity_sha256: String,
    pub producer_attestation_sha256: String,
    pub next_eligible_at: DateTime<Utc>,
    pub source_business_date: NaiveDate,
    pub expires_at: DateTime<Utc>,
    pub automatic_attempts_started: u32,
}

pub enum RetryIneligibility {
    // Existing variants remain unchanged.
    RetryProducerNotEnabled {
        push_kind: PushKind,
        producer_seam: Option<String>,
        reason_code: String,
    },
    ExpiredFreshness { expires_at: DateTime<Utc> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryCycleFailureReason {
    RetryAttemptStartAuditUnavailable,
    AuthorizationReconciliationBlocked,
    CycleOperationFailed,
    Panic,
    JoinError,
    ProcessInterrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryCycleFailure {
    schema_version: u8,
    rule_id: &'static str,
    reason: RetryCycleFailureReason,
    typed_fields: RetryCycleFailureTypedFieldsV1,
    typed_fields_sha256: String,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryCycleTerminalPhase {
    NotPrepared,
    CompletionPending,
    CompletionAppended,
    FailurePending,
    FailureAppended,
    Terminalized,
}

// Opaque coordinator-issued proof: no Clone/Copy/Serialize/Deserialize/Default
// and no public constructor or field.
pub struct NoRetryCycleCommitted {
    private: NoRetryCycleCommittedPrivate,
}

// Exact private tuple, in this declaration/serialization order:
// schema_version=1, rule_id="BR-192", namespace_sha256,
// owner_boot_identity, scheduled_for, started_at,
// proposed_cycle_ordinal, proposed_cycle_identity,
// selected_running_cycle_ordinal, selected_running_cycle_identity,
// selected_running_namespace_sha256,
// selected_running_owner_boot_identity, selected_running_scheduled_for,
// selected_running_started_at, selected_running_terminal_phase,
// selected_running_state="Running",
// selected_running_row_sha256,
// proposed_cycle_row_count=0, proposed_started_row_count=0.
// It also retains the canonical bytes and lowercase SHA-256 privately.

// The outer durable_delivery::Result error channel carries no release proof;
// only Ok(NotCommitted { .. }) proves a definite rollback.
pub enum RetryCycleBeginOutcome {
    Started { cycle_identity: String },
    NotCommitted {
        error: DurableDeliveryError,
        proof: NoRetryCycleCommitted,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "confirmed_count", deny_unknown_fields)]
pub enum RetryCycleSinkCalls {
    NotStarted,
    Confirmed(usize),
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryCycleEvidence {
    pub cycle_identity: String,
    pub attempted_decision_identities: Vec<String>,
    pub retry_candidate_query_calls: usize,
    pub queried_candidate_identities: Vec<String>,
    pub admissions: Vec<(String, RetryAdmission)>,
    pub provider_calls: usize,
    pub renderer_calls: usize,
    pub sink_calls: RetryCycleSinkCalls,
    pub final_failure: Option<RetryCycleFailure>,
}
```

`RetryCycleIdentityPreimageV1` has one canonical encoding. In the same
`BEGIN IMMEDIATE` used by `begin_retry_cycle_before_spawn`, and before the
global Running check or any write, read
`COALESCE(MAX(cycle_ordinal),0)`. If it is `i64::MAX`, return
`RetryCycleOrdinalExhausted { max_ordinal: i64::MAX }` without a write;
otherwise checked-add one to obtain the next positive ordinal. Serialize the
declared struct fields in their exact order as compact UTF-8 JSON with
`serde_json::to_vec`: no map, whitespace, BOM, trailing newline or Unicode
normalization. `scheduled_for` and `started_at` are produced only by
`to_rfc3339_opts(SecondsFormat::Nanos, true)`, hence UTC `Z` with exactly nine
fractional digits; `cycle_ordinal` is a JSON integer and
`namespace_sha256` is lowercase 64-hex. Prefix the canonical bytes with the
exact domain
`stock_analysis.durable_delivery.br192.retry_cycle_identity.v1\0`; the
lowercase SHA-256 of `domain || canonical_bytes` is the sole
`cycle_identity`. The retained row and logical `Started` event must rederive
to that exact ordinal and identity.

The private `NoRetryCycleCommitted` tuple uses the exact field order recorded
above, the same timestamp and compact-JSON rules, and the exact domain
`stock_analysis.durable_delivery.br192.no_retry_cycle_committed.v1\0`.
Its `selected_running_row_sha256` is lowercase SHA-256 over
the exact witness domain followed by its canonical row bytes. The domain is
`stock_analysis.durable_delivery.br192.retry_cycle_running_witness.v1\0`;
the hash input is `domain || canonical_row_bytes`. Those row bytes are a
declared compact-JSON struct in this exact order:
`schema_version=1,rule_id="BR-192",cycle_identity,cycle_ordinal,
namespace_sha256,owner_boot_identity,scheduled_for,started_at,state,
terminal_phase,candidate_query_calls,queried_candidate_count,
sorted_candidate_sha256,provider_calls,renderer_calls,sink_calls_state,
sink_calls_count,failure_reason,failure_payload_identity,
failure_typed_fields_sha256,failure_envelope_sha256,completed_at`; nullable
fields are JSON `null`, counts are JSON integers, timestamps use UTC `Z` with
nine fractional digits and text retains validated bytes.
`begin_retry_cycle_before_spawn` constructs it only after the identity-first
transaction has selected one retained global Running witness and has queried
zero `retry_cycles` rows plus zero logical `Started` rows for the proposed
identity. It then rolls back without a write.
`consume_no_retry_cycle_committed` consumes the proof in a fresh
`BEGIN IMMEDIATE`, recomputes the private canonical bytes/hash, recomputes
`MAX(cycle_ordinal)+1` and the proposed identity from the original input tuple,
recomputes the selected full-row witness hash, byte-matches every selected
Running witness field including namespace and literal state, and re-queries
both zero counts. A concurrent cycle insert,
ordinal/identity change, witness terminal/state/field change, malformed proof
or non-zero proposed row count is an exact typed rejection; the outer guard
remains latched. Only successful read-only consumption permits
`release_after_verified_no_cycle`.

`DurableDeliveryError` and the existing public
`durable_delivery::Result<T>` alias own all nine new variants above. Every
public coordinator, evidence, retry-runner, guard and terminal-finalizer API
returns that alias. `RetryCycleFailure` is an opaque audited business-failure
value used only as the error side of
`std::result::Result<RetryCycleEvidence, RetryCycleFailure>` inside
`retry_cycle_blocking`; it is not a second operational error channel. Do not
define a private verifier/reconciliation/runtime error in a public signature,
export a second operational error enum, parse `Display`, or downgrade a named
failure to `String`, `InvalidConfiguration`, `Sqlite` or another generic
variant.
Freshness expiry is not an operational error variant: public admission/manual
outcomes report it only as
`RetryIneligibility::ExpiredFreshness { expires_at }` (and expiry
reconciliation uses `RetryExpiryDisposition`). Do not add
`DurableDeliveryError::RetryExpiredFreshness` or encode ineligibility as a
storage/runtime failure.
`br192_retry_error_contract_owns_exact_typed_variants_and_fields` constructs
and pattern-matches `RetryCycleAlreadyRunning` and
`RetryCycleGuardCompareExchangeInvariant` in addition to every field-bearing
variant, including
`RetryCycleOrdinalExhausted { max_ordinal: i64::MAX }`. A separate exact test
sets the retained maximum ordinal to `i64::MAX`, pattern-matches that public
variant and proves no cycle or `Started` row was written. No `String` payload
or catch-all reason is allowed for either guard condition or ordinal
exhaustion.

The two reconciliation bounds retain the exact immutable identity selected by
the final non-mutating pending-row check and use `max_steps=4096`. Evidence
query bounds use exact `requested,min=1,max=256`. The conflicting-duplicate
variant emits only a lowercase domain-separated hash of the full logical tuple
plus exact mismatch flags: bytes-only `(true,false)`, hash-only `(false,true)`,
both `(true,true)`; `(false,false)` is forbidden. The 257th distinct complete
join uses `max=256,attempted_distinct_count=257`.

Freeze the evidence logical-tuple preimage as this exact private struct:

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

Its only field/serialization order is
`schema_version,rule_id,decision_identity,retry_ordinal,attempt_identity,
sink_result_identity,authorization_identity,rejection_disposition_identity`.
Require `schema_version=1`, `rule_id="BR-192"`, `retry_ordinal in 1..=3`
and every identity non-empty by the ASCII space/tab/LF/CR rule while preserving
accepted bytes exactly. Encoding is repository-canonical compact UTF-8 JSON,
exactly `serde_json::to_vec` of this struct declaration: no map,
insignificant whitespace, BOM, trailing newline, alternate numeric spelling or
Unicode normalization. Decode then reserialize must be byte-identical. Compute
lowercase hex:

```text
SHA-256(
  b"stock_analysis.durable_delivery.br192.retry_evidence_logical_tuple.v1\0"
  || canonical_utf8_json(preimage)
)
```

The bounded-map typed key and this hash are rebuilt from the same validated
preimage; the artifact cannot supply either as authority. The frozen golden
canonical bytes are
`{"schema_version":1,"rule_id":"BR-192","decision_identity":"decision-001","retry_ordinal":2,"attempt_identity":"attempt-002","sink_result_identity":"sink-result-003","authorization_identity":"authorization-004","rejection_disposition_identity":"rejection-disposition-005"}`
and the expected digest is
`0794e8feda8a5af2c7828be49f35248dd81d44cd21b7408d29db7a7e20e98151`.
Tests must recompute this vector and independently mutate the domain, schema,
one field, field order and encoding.

Freeze namespace canonicalization as UTF-8 JSON emitted by the repository
canonical serializer in the exact struct field order
`schema_version,rule_id,namespace_kind,test_code`. Accept only
`schema_version=1`, `rule_id="BR-192"`, Production with no test code, or Test
with one ASCII-whitespace-trimmed `TEST_CODE_*` value. Compute lowercase hex
`SHA-256(b"stock_analysis.durable_delivery.br192.retry_namespace.v1\0" ||
canonical_utf8_bytes)`. Validation rebuilds the canonical bytes and recomputes
the hash; no caller may hash a path, debug string, global or ad-hoc
concatenation. Tests cover Production/Test separation, Unicode UTF-8
determinism, field-boundary collision resistance, invalid combinations and a
tampered digest.

Every design enum and every canonical/event/CLI/evidence struct that crosses a
deserialization boundary derives `Serialize` and `Deserialize` together with
the existing debug/clone/equality traits. `RetryCycleFailure` and
`RetryCycleEvidence` are deliberate exceptions: neither is deserializable or
`Default`, and neither accepts arbitrary persisted/caller bytes. Private
reason-specific persisted preimage/envelope types use crate-private
`Deserialize` with `deny_unknown_fields` only for coordinator recovery; no
decoder or raw-byte constructor is exported. The coordinator serializes and
persists its own complete canonical typed preimage plus canonical failure
envelope before creating the `Failed` outbox. Do not define
`ForeignLiveAttempt`.
`RetryAttemptStartAuditUnavailable.reason_code` accepts only
`missing_start_event`, `pending_append` or `missing_immutable_ref`; any other
value is a programmer/persistence mismatch. Preparation/read-back/validation
returns this typed error before any sink capability or execution permit exists.
The runtime converts it directly, without a `String` intermediate, through
`RetryCycleFailure::from_retry_attempt_start_audit_unavailable(&error)`.
That constructor matches the typed error variant, validates the exact
`attempt_identity` plus the closed inner reason, and stores a private
reason-specific preimage. The six declarations in Step 4 are the complete
public construction surface and use the existing public
`durable_delivery::Result<T>` alias. The first accepts only the exact typed
error variant; every `*_sha256` input accepts only lowercase 64-hex and never
raw error/panic/JoinError/boot/authorization content. No constructor accepts a
caller-supplied reason, schema, rule, canonical bytes or digest override.
`RetryCycleFailure` exposes read-only `reason()` and
`typed_fields_sha256()` accessors only; all fields and
`RetryCycleFailureTypedFieldsV1` remain private, and there is no raw-values
constructor, builder, struct update, `Deserialize` or `Default`.

The private preimage enum has one exact ordered canonical struct per reason:

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

These private preimage structs implement crate-private `Serialize` and
`Deserialize` with `deny_unknown_fields` for persisted recovery; none
implements `Default`, and the public opaque DTO implements neither
`Deserialize` nor `Default`.
Every struct fixes `schema_version=1`, `rule_id="BR-192"` and its matching
closed reason. `inner_reason` is exactly `missing_start_event`,
`pending_append` or `missing_immutable_ref`; identity values are preserved
byte-for-byte after ASCII-whitespace non-empty validation, while all other
fields named `*_sha256` are validated lowercase 64-hex. Each reason has a
distinct domain prefix
`stock_analysis.durable_delivery.br192.retry_cycle_failure.<reason>.v1\0`.
`operation` is the exact snake_case serialization of the closed
`RetryCycleOperation` variant.
`typed_fields_sha256` is computed from that prefix plus the repository
canonical UTF-8 bytes. A crate-private coordinator accessor returns the
validated canonical bytes. The coordinator also creates one private canonical
envelope with exact ordered fields
`schema_version,rule_id,cycle_identity,failure_reason,
typed_preimage_sha256,typed_preimage_length`, hashed under
`stock_analysis.durable_delivery.br192.retry_cycle_failure_envelope.v1\0`.
Before preparing or terminalizing `Failed`, it decodes the closed stored
preimage/envelope, requires canonical reserialization byte equality,
recomputes both hashes and rejects every
reason/preimage/envelope/cycle/hash disagreement.

The `retry_attempt_start_audit_unavailable` finalizer first asks the coordinator
to classify persisted start/ownership state in the failure transaction. For
each of `missing_start_event`, `pending_append` and `missing_immutable_ref`, only
the state with no acknowledged start and no consumed ownership may use the
narrow branch. That branch stores one immutable `retry_cycle_failure_payloads`
row containing both complete canonical byte arrays and hashes, binds the cycle
and canonical Pending `Failed` outbox to that payload, append/acknowledges those
exact bytes, and only then CASes the cycle to `Failed`, while leaving the exact
pending attempt-start bytes and byte-identical decision/attempt/binding/schedule
rows plus their canonical hashes unchanged for later reconciliation.

Acknowledged start or consumed ownership dominates the reason code and rejects
the unchanged-state exception. That case uses the ordinary post-claim
finalizer: quarantine and advance every qualifying same-cycle
start/ownership/in-flight attempt to retained `InterruptedUncertain`,
append/acknowledge all uncertainty, then prepare the matching `Failed` slot.
The caller cannot select either branch with a boolean; both make zero recovery
sink calls. The exact before/after boundary matrix is owned in Task 4, while the
runtime mapping and common finalizer assertions are completed in the deferred
Task 8 recipe.
A fresh prior-boot recovery process must validate and terminalize using only
those SQLite payload/outbox bytes; no in-memory failure DTO or display string
is available or required.

Before leaving Step 4, freeze and test the permit crossing contract:

- only `counted_producer_catalog.rs` constructs `CountedProducerPermit`;
- its private marker is exactly `CountedProducerPermitPrivate(())`; the closed
  `CountedProducerDenied` uses `CatalogInvalid` for catalog shape/hash failure
  and `ProducerNotEnabled` for a disabled kind or seam mismatch, preserving the
  catalog reason code;
- it derives the catalog hash from the exact 15 ordered rows and returns a
  permit only for exact `(ReviewProviderTopN,
  "push_templates::dispatch_r09_provider_top_n_outcome")`;
- the permit implements none of Clone/Copy/Serialize/Deserialize/Default and
  is consumed by the sole production
  `CountedDeliveryBinding::new_permitted`; fixed HEAD's public `new` becomes
  non-production/cfg(test)-only;
- the sole permitted production caller of the public consuming method is
  `CountedDeliveryBinding::new_permitted`; the syntax-aware all-caller checker
  rejects every other call. That constructor invokes
  `CountedProducerPermit::into_attestation(self,
  source_business_date, expires_at)`, which revalidates the current catalog,
  requires the exact next-Shanghai-midnight expiry and returns an opaque
  `CountedProducerAttestation` with read-only accessors, no public constructor
  and no `Deserialize` implementation;
- the binding freezes a private `CountedProducerAttestation`, then
  `deliver_counted_binding` creates private `PermittedDeliveryEnvelope`;
  raw `deliver_envelope(DeliveryEnvelope)` is no longer production-visible;
- `DeliveryEnvelope` stores only the private optional
  `CountedProducerAttestationEvidenceV1`; its sole consuming setter
  `with_counted_producer_attestation` validates kind/date/expiry/hash and may be
  called in production only by `CountedDeliveryBinding::new_permitted`.
  `#[serde(default, skip_serializing_if = "Option::is_none")]` must preserve
  legacy v5 canonical bytes exactly; coordinator insertion reads this private
  value to write the deferred companion before its decision;
- compile-fail fixtures attempt private struct construction, `.clone()`,
  `serde_json::to_vec(&permit)` and the retired constructor and must all fail;
  positive compile tests move exactly one permit through R-09; and
- retry tests persist the six attestation fields, prove v5 companion rows remain
  absent and typed-ineligible, and mutate kind/seam/catalog/attestation/source
  date/expiry one at a time
  across automatic authorization, PAM authorization, candidate discovery,
  admission and pre-sink claim. Every mutation yields
  `RetryProducerNotEnabled` with zero authorization/provider/renderer/sink.

The catalog module stays private, but the permit/attestation/denial types and
acquisition function must be in the exact root manifest so the monitor and
production authorization CLI share one authority. Compile-contract tests prove
the fields, marker, constructors and persisted-attestation validator remain
private and that no binary contains a second catalog literal.

- [ ] **Step 5: Freeze the final root export manifest without exporting yet**

Apart from Task 1's private `mod counted_producer_catalog;` declaration, do not
modify the public surface of `src/durable_delivery/mod.rs`. Freeze this exact
one-time Task 8 root manifest:

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

This is the complete BR-192 cross-module contract, not a replacement list for
the pre-existing `durable_delivery` API. Task 8 preserves every unrelated
existing root export and neither deletes nor duplicates an already-public
symbol.

The prepare/reconcile/validate/claim/execute operations are coordinator methods,
not root free functions. Each owning task defines its types/constants first and
uses private module paths internally. Only after Task 7 has created the
verifier types may Task 8 add both CLI sources, one complete `pub use` block
and the compile-contract test. The design and plan lists must remain
byte-for-byte equivalent in symbol order.

Root leakage is checked with a multiline-aware machine gate bounded to one
`pub use ...;` token range (and a separate `pub mod` alternative), so a symbol
split across formatted lines cannot evade the check and a later unrelated
statement cannot create a false match. Expected output is empty:

```bash
! rg -n -U 'pub\\s+(?:mod\\s+(?:retry_command|retry_evidence)\\s*;|use(?s:[^;])*\\b(?:RetryCommandAuthority|RetryCommandTargetResolver|RetryCommandTarget|AuthenticatedRetryOperator|ResolvedRetryCommandTarget|PamRetryCommandAuthority|ProductionRetryCommandTargetResolver|RetryEvidenceTarget|TestRetryEvidenceTarget|verify_br192_retry_evidence_test)\\b(?s:[^;])*;)' src/durable_delivery/mod.rs
```

The design/plan frozen manifests are byte-equivalent only when this command
exits zero with no output:

```bash
diff -u \
  <(awk '/BR-192 cross-module contract/{f=1} f&&/^```text$/{p=1;next} p&&/^```$/{exit} p{print}' docs/superpowers/specs/2026-07-30-br192-provider-free-retry-design.md) \
  <(awk '/Freeze this exact/{f=1} f&&/^```text$/{p=1;next} p&&/^```$/{exit} p{print}' docs/superpowers/plans/2026-07-30-br192-provider-free-retry.md)
```

- [ ] **Step 6: Run contract tests and commit**

```bash
cargo test --lib durable_delivery::tests::br192_retry_contract_has_no_foreign_live_attempt_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_each_uncertain_state_is_explicitly_ineligible -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_backoff_and_cap_are_fixed_delivery_governance -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_namespace_hash_is_canonical_domain_separated_and_utf8_stable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_namespace_hash_rejects_invalid_fields_and_tampering -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_error_contract_owns_exact_typed_variants_and_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_identity_hash_is_domain_separated_field_ordered_and_utf8_stable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_identity_rejects_domain_schema_field_order_timestamp_and_ordinal_mutations -- --exact --test-threads=1
```

Expected: all eight run one test and pass.

```bash
git add docs/superpowers/specs/2026-07-30-br192-provider-free-retry-design.md docs/superpowers/plans/2026-07-30-br192-provider-free-retry.md docs/business_rules.md src/durable_delivery/model.rs src/durable_delivery/counted_producer_catalog.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs
git commit -m "design: freeze BR-192 retry contract"
```

### Task 2: Add the v6 manifest, deterministic SHA-256 authority,
schedule and cycle-audit schema

**Files:**

- Modify `Cargo.toml`
- Modify `src/durable_delivery/schema.rs`
- Modify `src/durable_delivery/coordinator.rs`
- Test `src/durable_delivery/tests.rs`
- Modify `tools/compliance/check.sh`
- Create `tools/compliance/lib/check_br192_provider_free_retry.sh`
- Modify `tools/compliance/lib/check_br194_review_dependency.sh`
- Modify `tools/release/verify_br194_review_join.py`

- [ ] **Step 1: Write schema RED tests**

Add:

```rust
#[test]
fn br192_retry_authorization_payload_is_immutable_and_unique_per_rejection() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_authorization_state_transitions_are_monotonic() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_authorization_transition_events_append_before_apply_or_invalidate() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authorization_append_acknowledgements_store_no_untrusted_timestamp() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_active_binding_is_unique_and_historical_binding_survives_current_change() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_attempt_binding_freezes_authorization_disposition_cycle_generations_owner_and_fence() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_reserved_attempt_binding_and_schedule_relation_is_atomic() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_fence_is_positive_integer_i64_across_schema_and_contract() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_send_ownership_is_retained_consumed_and_monotonic() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_payload_is_append_only_and_retained() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_ordinal_is_positive_unique_immutable_and_non_deletable() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_identity_rederives_from_retained_ordinal_and_exact_fields() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_schedule_is_retained_and_all_authority_fields_are_monotonic() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_schedule_last_attempt_is_fk_bound_to_exact_retry_attempt_ordinal() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_frozen_rejection_atomically_initializes_schedule_from_observed_at() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_counted_producer_attestation_is_same_transaction_immutable_and_hash_valid() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_counted_producer_attestation_preimage_excludes_digest_and_has_golden_hash() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_counted_producer_attestation_projection_rejects_preimage_or_digest_mutation() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_v5_decisions_gain_no_synthetic_producer_attestation() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_delivery_envelope_none_attestation_preserves_v5_canonical_bytes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_delivery_envelope_attestation_setter_has_one_permitted_production_caller() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_schedule_persists_exact_source_date_expiry_and_terminal_state() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_expiry_outbox_replays_prepare_append_ack_and_terminalize_crashes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_expiry_prepare_wins_total_order_and_blocks_later_start_or_ownership() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pending_start_wins_total_order_and_routes_expiry_to_uncertainty() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_expiry_preparation_outcome_is_total_and_never_uses_durable_error_for_business_expiry() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_prepare_retry_attempt_expired_before_start_returns_expiry_prepared_without_start() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_claim_retry_sink_execution_expired_after_appended_start_returns_uncertainty_without_permit() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_start_first_uncertainty_reconciliation_clears_active_schedule_to_fixed_point() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_admission_expiry_is_appended_and_terminalized_before_cycle_advances() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_expired_freshness_terminal_is_single_audited_and_not_revivable() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_historical_terminal_start_does_not_block_current_attempt_expiry() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_expiry_canonical_binds_private_freshness_observation() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_final_pre_call_expiry_consumes_permit_without_external_sink() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_companion_requires_complete_same_transaction_triple() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_companion_is_canonical_immutable_and_commit_deferred() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_rejects_sink_result_at_every_interleaving() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_committed_pre_call_expiry_rejects_later_sink_result_cross_connection() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_restores_cycle_confirmed_existing_result_count_atomically() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_requires_result_terminal_ownership_bijection() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_terminal_result_ownership_pointer_is_unique_write_once_and_immutable() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authoritative_retry_result_requires_exact_terminal_ownership_reverse_join() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_terminal_result_bijection_rejects_cross_attempt_decision_fence_and_non_authoritative_rows_do_not_count() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_with_other_started_keeps_cycle_indeterminate_atomically() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_with_interrupted_uncertain_keeps_cycle_indeterminate_atomically() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pre_call_expiry_terminal_transaction_is_recoverable_and_idempotent() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_effective_expired_attempt_is_excluded_from_candidate_orphan_and_evidence() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_logical_slots_are_unique_and_conflicting_bytes_fail_closed() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_global_started_and_terminal_cardinality_is_exact() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_terminal_phase_forbids_opposite_terminal_kind_after_pending_slot() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_completed_append_error_leaves_running_completion_pending() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_completed_ack_before_terminal_cas_leaves_running_completion_appended() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_completed_terminal_cas_requires_exact_appended_completion_bytes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_failed_cycle_requires_closed_reason_and_exact_typed_field_digest() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_failure_is_opaque_non_deserializable_and_digest_is_recomputed() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_failure_payload_persists_complete_canonical_preimage_envelope_and_hashes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_prior_boot_terminalizer_uses_only_persisted_failure_payload_bytes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_failed_append_error_leaves_running_failure_pending() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_failed_ack_before_terminal_cas_leaves_running_failure_appended() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_failed_terminal_cas_requires_exact_appended_failure_bytes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_sink_calls_are_not_started_confirmed_or_indeterminate_never_default_zero() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_schema_v6_fresh_and_v1_v2_v3_v4_v5_upgrade_paths_validate() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_schema_v6_cycle_ordinal_manifest_is_identical_across_v0_v1_v2_v3_v4_v5() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_schema_v6_repeated_initialization_is_idempotent() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_schema_newer_than_v6_fails_before_mutation() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_all_v6_immutable_refs_reject_empty_and_ascii_whitespace_only() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_v5_to_v6_preserves_br194_replay_manifest_audit_kinds_and_rows() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_durable_sha256_udf_is_registered_before_every_schema_path() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_durable_sha256_udf_registration_follows_complete_descriptor_binding() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_durable_sha256_udf_never_runs_before_wal_shm_attestation() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_wal_materialization_is_the_only_pre_binding_sqlite_exception() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_wal_materialization_rejects_omitted_reordered_or_extra_sqlite_steps() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_rusqlite_031_exposes_utf8_deterministic_and_innocuous_function_flags() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_durable_sha256_udf_rejects_null_text_and_wrong_type() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_v6_authority_triggers_recompute_canonical_sha256() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_v6_authority_triggers_reject_bytes_hash_and_combined_mutations() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_python_br194_verifier_uses_hashlib_without_sql_callback_or_trigger_execution() { panic!("BR-192 RED: named contract is not implemented"); }
```

Use the existing `Fixture`, `envelope`, `MemoryAppendPort`,
`prepare_reserved`, `StaticSink`, `rejection` and `reconcile_terminal`.
Do not create a parallel fixture.

- [ ] **Step 2: Run schema tests and prove RED**

```bash
cargo test --lib durable_delivery::tests::br192_retry_authorization_payload_is_immutable_and_unique_per_rejection -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_authorization_state_transitions_are_monotonic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_authorization_transition_events_append_before_apply_or_invalidate -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_append_acknowledgements_store_no_untrusted_timestamp -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_active_binding_is_unique_and_historical_binding_survives_current_change -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_attempt_binding_freezes_authorization_disposition_cycle_generations_owner_and_fence -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_reserved_attempt_binding_and_schedule_relation_is_atomic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_fence_is_positive_integer_i64_across_schema_and_contract -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_send_ownership_is_retained_consumed_and_monotonic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_payload_is_append_only_and_retained -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_ordinal_is_positive_unique_immutable_and_non_deletable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_identity_rederives_from_retained_ordinal_and_exact_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_schedule_is_retained_and_all_authority_fields_are_monotonic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_schedule_last_attempt_is_fk_bound_to_exact_retry_attempt_ordinal -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_frozen_rejection_atomically_initializes_schedule_from_observed_at -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_counted_producer_attestation_is_same_transaction_immutable_and_hash_valid -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_counted_producer_attestation_preimage_excludes_digest_and_has_golden_hash -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_counted_producer_attestation_projection_rejects_preimage_or_digest_mutation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v5_decisions_gain_no_synthetic_producer_attestation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_delivery_envelope_none_attestation_preserves_v5_canonical_bytes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_delivery_envelope_attestation_setter_has_one_permitted_production_caller -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_schedule_persists_exact_source_date_expiry_and_terminal_state -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_expiry_outbox_replays_prepare_append_ack_and_terminalize_crashes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_expiry_prepare_wins_total_order_and_blocks_later_start_or_ownership -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pending_start_wins_total_order_and_routes_expiry_to_uncertainty -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_expiry_preparation_outcome_is_total_and_never_uses_durable_error_for_business_expiry -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_prepare_retry_attempt_expired_before_start_returns_expiry_prepared_without_start -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_claim_retry_sink_execution_expired_after_appended_start_returns_uncertainty_without_permit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_start_first_uncertainty_reconciliation_clears_active_schedule_to_fixed_point -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_admission_expiry_is_appended_and_terminalized_before_cycle_advances -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_historical_terminal_start_does_not_block_current_attempt_expiry -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_expiry_canonical_binds_private_freshness_observation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_final_pre_call_expiry_consumes_permit_without_external_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_companion_requires_complete_same_transaction_triple -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_companion_is_canonical_immutable_and_commit_deferred -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_rejects_sink_result_at_every_interleaving -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_committed_pre_call_expiry_rejects_later_sink_result_cross_connection -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_restores_cycle_confirmed_existing_result_count_atomically -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_requires_result_terminal_ownership_bijection -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_terminal_result_ownership_pointer_is_unique_write_once_and_immutable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authoritative_retry_result_requires_exact_terminal_ownership_reverse_join -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_terminal_result_bijection_rejects_cross_attempt_decision_fence_and_non_authoritative_rows_do_not_count -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_with_other_started_keeps_cycle_indeterminate_atomically -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_with_interrupted_uncertain_keeps_cycle_indeterminate_atomically -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_terminal_transaction_is_recoverable_and_idempotent -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_effective_expired_attempt_is_excluded_from_candidate_orphan_and_evidence -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_logical_slots_are_unique_and_conflicting_bytes_fail_closed -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_global_started_and_terminal_cardinality_is_exact -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_terminal_phase_forbids_opposite_terminal_kind_after_pending_slot -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_completed_append_error_leaves_running_completion_pending -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_completed_ack_before_terminal_cas_leaves_running_completion_appended -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_completed_terminal_cas_requires_exact_appended_completion_bytes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_failed_cycle_requires_closed_reason_and_exact_typed_field_digest -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_failure_is_opaque_non_deserializable_and_digest_is_recomputed -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_failure_payload_persists_complete_canonical_preimage_envelope_and_hashes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_prior_boot_terminalizer_uses_only_persisted_failure_payload_bytes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_failed_append_error_leaves_running_failure_pending -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_failed_ack_before_terminal_cas_leaves_running_failure_appended -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_failed_terminal_cas_requires_exact_appended_failure_bytes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_sink_calls_are_not_started_confirmed_or_indeterminate_never_default_zero -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_v6_fresh_and_v1_v2_v3_v4_v5_upgrade_paths_validate -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_v6_cycle_ordinal_manifest_is_identical_across_v0_v1_v2_v3_v4_v5 -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_v6_repeated_initialization_is_idempotent -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_newer_than_v6_fails_before_mutation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_all_v6_immutable_refs_reject_empty_and_ascii_whitespace_only -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v5_to_v6_preserves_br194_replay_manifest_audit_kinds_and_rows -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_is_registered_before_every_schema_path -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_registration_follows_complete_descriptor_binding -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_never_runs_before_wal_shm_attestation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_wal_materialization_is_the_only_pre_binding_sqlite_exception -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_wal_materialization_rejects_omitted_reordered_or_extra_sqlite_steps -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rusqlite_031_exposes_utf8_deterministic_and_innocuous_function_flags -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_rejects_null_text_and_wrong_type -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v6_authority_triggers_recompute_canonical_sha256 -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v6_authority_triggers_reject_bytes_hash_and_combined_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_python_br194_verifier_uses_hashlib_without_sql_callback_or_trigger_execution -- --exact --test-threads=1
```

Expected: each command reports `running 1 test` and fails because tables or
constraints are absent.

- [ ] **Step 3: Add schema in one migration transaction**

Set:

```rust
pub(crate) const SCHEMA_VERSION: i64 = 6;
```

Schema v5 is the fixed-HEAD repository baseline and already owns BR-194 replay
authority. Add exactly one `migrate_durable_delivery_v5_to_v6` transaction for
the BR-192 companion objects. Do not rewrite the accepted BR-194 v4-to-v5
migration or add a second feature-specific v5-to-v6 step. Preserve ordered
fall-through:

```text
user_version=0  -> create complete v6 directly
user_version=1  -> v1 -> v2 -> v3 -> v4 -> v5 -> v6
user_version=2  -> v2 -> v3 -> v4 -> v5 -> v6
user_version=3  -> v3 -> v4 -> v5 -> v6
user_version=4  -> accepted v4 -> v5 -> validate v5 -> v6
user_version=5  -> one additive v5 -> v6 transaction
user_version=6  -> validate only; repeat safely
user_version>6  -> fail before mutation
```

Each migration and its post-validation remain inside one transaction with
foreign-key enforcement enabled. Set `user_version=6` only after all required
v5 baseline and v6 companion tables, columns, foreign keys, indexes and
triggers validate and every preserved BR-194 replay row/object/audit kind
matches its pre-migration snapshot.

In `Cargo.toml`, retain `chrono` and enable the required rusqlite UDF feature:

```toml
rusqlite = { version = "0.31", features = ["chrono", "functions"] }
```

Implement one central seam in `src/durable_delivery/schema.rs`:

```rust
pub(crate) fn register_durable_sql_functions(
    connection: &rusqlite::Connection,
) -> durable_delivery::Result<()>;
```

It registers fixed-name, arity-one `sha256_hex` with
`SQLITE_UTF8 | SQLITE_DETERMINISTIC | SQLITE_INNOCUOUS`. The function accepts
only a non-null SQLite BLOB, returns exactly 64 lowercase hexadecimal
characters, rejects NULL/TEXT/INTEGER/REAL, and self-tests
`sha256_hex(x'')` against the fixed SHA-256 empty-input vector. Registration,
flag or self-test failure returns the stable operational failure
`durable_sha256_udf_unavailable`.

rusqlite 0.31 exposes all three exact `FunctionFlags`, including
`SQLITE_INNOCUOUS`; compile/runtime tests pin that achievable contract and
fail if a future dependency loses a flag.

Every production, test, fresh, migrated and reopened Rust/rusqlite durable
connection must call this seam only after the existing descriptor lifecycle:

```text
Connection::open_with_flags
  -> attest/retain main
  -> run only the audited journaling bootstrap: fixed 5-second busy timeout;
     journal_mode=WAL; synchronous=FULL; BEGIN IMMEDIATE; ROLLBACK; read back
     only journal_mode and synchronous
  -> materialize WAL/SHM through that sequence
  -> re-attest main and attest/retain WAL/SHM
  -> validate complete bound connection
  -> register+self-test sha256_hex
  -> configure attested connection
  -> schema creation/migration/trigger creation/manifest validation
```

Before complete descriptor binding, the sole permitted SQLite operations are
the audited journaling sequence above, invoked only after initial main-
descriptor attestation and solely to create/validate WAL/SHM for descriptor
attestation. It may not enable foreign keys, read `user_version`,
`sqlite_master` or application rows, execute DDL, register/invoke a UDF, or run
another PRAGMA/query/transaction. Omitting, reordering or adding a step fails
bootstrap. After main re-attestation plus retained WAL/SHM attestation and live-
binding validation, register/self-test `sha256_hex`; only then configure the
connection and perform schema work. Centralize and source-check that order so
no Rust caller bypasses it.

Every v6 BR-192 authority trigger and every preserved v5 BR-194 authority
trigger that accepts canonical bytes plus a stored digest must require
`sha256_hex(canonical_blob)=stored_lowercase_sha256`. Migration replaces only
no accepted BR-194 trigger: the existing v4-to-v5 definitions already contain
this recomputation and are preserved byte-for-byte. The Rust
BR-192 verifier and shared compliance checker recompute the identical BLOB
digest and fail closed on bytes-only, hash-only or combined mutation.

The Python BR-194 verifier is a separate achievable read-only contract: it
registers no SQL callback and never executes DML or fires a trigger. It reads
`PRAGMA user_version`, rows and `sqlite_master` from its isolated verification
copy, recomputes returned canonical BLOB hashes with `hashlib.sha256`, and
compares normalized trigger SQL to the expected catalog containing the literal
`sha256_hex` predicates. It does not claim rusqlite flags or call the Rust
connection-local seam.

Create:

```sql
CREATE TABLE counted_producer_attestations(
  decision_identity TEXT PRIMARY KEY
    REFERENCES delivery_decisions(decision_identity)
    DEFERRABLE INITIALLY DEFERRED,
  push_kind TEXT NOT NULL CHECK(push_kind='ReviewProviderTopN'),
  producer_seam TEXT NOT NULL
    CHECK(producer_seam='push_templates::dispatch_r09_provider_top_n_outcome'),
  producer_catalog_identity_sha256 TEXT NOT NULL
    CHECK(length(producer_catalog_identity_sha256)=64),
  producer_attestation_sha256 TEXT NOT NULL
    CHECK(length(producer_attestation_sha256)=64),
  source_business_date TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  attestation_canonical BLOB NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE retry_authorizations(
  authorization_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL
    REFERENCES delivery_decisions(decision_identity),
  rejection_disposition_identity TEXT NOT NULL
    REFERENCES delivery_disposition_payloads(disposition_identity),
  source_kind TEXT NOT NULL
    CHECK(source_kind IN ('FrozenRejection','ManualOperator')),
  command_identity TEXT,
  push_kind TEXT NOT NULL CHECK(push_kind='ReviewProviderTopN'),
  producer_seam TEXT NOT NULL
    CHECK(producer_seam='push_templates::dispatch_r09_provider_top_n_outcome'),
  producer_catalog_identity_sha256 TEXT NOT NULL
    CHECK(length(producer_catalog_identity_sha256)=64),
  producer_attestation_sha256 TEXT NOT NULL
    CHECK(length(producer_attestation_sha256)=64),
  authorization_canonical BLOB NOT NULL,
  authorization_sha256 TEXT NOT NULL,
  append_state TEXT NOT NULL
    CHECK(append_state IN ('PendingAppend','Appended')),
  immutable_audit_ref TEXT,
  apply_state TEXT NOT NULL
    CHECK(apply_state IN ('PendingApply','Applied','Invalidated')),
  authorized_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  applied_at TEXT,
  UNIQUE(decision_identity,rejection_disposition_identity),
  CHECK((source_kind='ManualOperator')=(command_identity IS NOT NULL)),
  CHECK(
    (append_state='PendingAppend' AND immutable_audit_ref IS NULL)
    OR
    (append_state='Appended'
      AND immutable_audit_ref IS NOT NULL
      AND length(trim(
        immutable_audit_ref,
        char(32) || char(9) || char(10) || char(13)
      )) > 0)
  ),
  CHECK(apply_state!='Applied' OR append_state='Appended')
);

CREATE TABLE retry_authorization_events(
  authorization_event_identity TEXT PRIMARY KEY,
  authorization_identity TEXT NOT NULL
    REFERENCES retry_authorizations(authorization_identity),
  event_kind TEXT NOT NULL CHECK(event_kind IN ('Applied','Invalidated')),
  from_apply_state TEXT NOT NULL CHECK(from_apply_state='PendingApply'),
  to_apply_state TEXT NOT NULL
    CHECK(to_apply_state IN ('Applied','Invalidated')),
  target_disposition_identity TEXT NOT NULL
    REFERENCES delivery_disposition_payloads(disposition_identity),
  reason_code TEXT NOT NULL,
  event_canonical BLOB NOT NULL,
  event_sha256 TEXT NOT NULL,
  append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
  immutable_audit_ref TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(authorization_identity,event_kind),
  CHECK((event_kind='Applied')=(to_apply_state='Applied')),
  CHECK(
    (append_state='Pending' AND immutable_audit_ref IS NULL)
    OR
    (append_state='Appended'
      AND immutable_audit_ref IS NOT NULL
      AND length(trim(
        immutable_audit_ref,
        char(32) || char(9) || char(10) || char(13)
      )) > 0)
  )
);

CREATE TABLE retry_authorization_bindings(
  binding_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL
    REFERENCES delivery_decisions(decision_identity),
  authorization_identity TEXT NOT NULL
    REFERENCES retry_authorizations(authorization_identity),
  rejection_disposition_identity TEXT NOT NULL
    REFERENCES delivery_disposition_payloads(disposition_identity),
  binding_generation INTEGER NOT NULL CHECK(binding_generation > 0),
  binding_state TEXT NOT NULL CHECK(binding_state IN ('Active','Cleared')),
  cleared_reason TEXT,
  created_at TEXT NOT NULL,
  cleared_at TEXT,
  UNIQUE(decision_identity,binding_generation),
  UNIQUE(authorization_identity),
  CHECK(
    (binding_state='Active' AND cleared_reason IS NULL AND cleared_at IS NULL)
    OR
    (binding_state='Cleared' AND cleared_reason IS NOT NULL AND cleared_at IS NOT NULL)
  )
);

CREATE UNIQUE INDEX retry_authorization_one_active_per_decision
ON retry_authorization_bindings(decision_identity)
WHERE binding_state='Active';

CREATE TABLE retry_attempt_bindings(
  attempt_identity TEXT PRIMARY KEY
    REFERENCES delivery_attempts(attempt_identity),
  decision_identity TEXT NOT NULL
    REFERENCES delivery_decisions(decision_identity),
  cycle_identity TEXT NOT NULL
    REFERENCES retry_cycles(cycle_identity),
  authorization_identity TEXT NOT NULL
    REFERENCES retry_authorizations(authorization_identity),
  rejection_disposition_identity TEXT NOT NULL
    REFERENCES delivery_disposition_payloads(disposition_identity),
  authorization_binding_identity TEXT NOT NULL
    REFERENCES retry_authorization_bindings(binding_identity),
  retry_ordinal INTEGER NOT NULL CHECK(retry_ordinal BETWEEN 1 AND 3),
  binding_generation INTEGER NOT NULL CHECK(binding_generation > 0),
  reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
  owner_instance_identity TEXT NOT NULL
    CHECK(length(trim(
      owner_instance_identity,
      char(32) || char(9) || char(10) || char(13)
    )) > 0),
  fence_token INTEGER NOT NULL CHECK(fence_token > 0),
  created_at TEXT NOT NULL,
  UNIQUE(decision_identity,retry_ordinal),
  UNIQUE(cycle_identity,decision_identity,reservation_generation)
);

CREATE TABLE retry_schedules(
  decision_identity TEXT PRIMARY KEY
    REFERENCES delivery_decisions(decision_identity),
  automatic_attempts_started INTEGER NOT NULL DEFAULT 0
    CHECK(automatic_attempts_started BETWEEN 0 AND 3),
  next_eligible_at TEXT,
  exhausted_at TEXT,
  source_business_date TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  terminal_state TEXT NOT NULL DEFAULT 'Active'
    CHECK(terminal_state IN (
      'Active','ExpiredFreshness','Exhausted','Completed'
    )),
  last_attempt_binding_identity TEXT UNIQUE
    REFERENCES retry_attempt_bindings(attempt_identity),
  version INTEGER NOT NULL DEFAULT 0,
  CHECK(exhausted_at IS NULL OR automatic_attempts_started=3),
  CHECK((terminal_state='Exhausted')=(exhausted_at IS NOT NULL)),
  CHECK(
    (automatic_attempts_started=0 AND last_attempt_binding_identity IS NULL)
    OR
    (automatic_attempts_started>0 AND last_attempt_binding_identity IS NOT NULL)
  )
);

CREATE TABLE retry_expiry_audit_outbox(
  expiry_event_identity TEXT PRIMARY KEY,
  decision_identity TEXT NOT NULL
    REFERENCES delivery_decisions(decision_identity),
  rejection_disposition_identity TEXT NOT NULL
    REFERENCES delivery_disposition_payloads(disposition_identity),
  authorization_identity TEXT
    REFERENCES retry_authorizations(authorization_identity),
  attempt_identity TEXT REFERENCES retry_attempt_bindings(attempt_identity),
  source_business_date TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  freshness_observed_at TEXT NOT NULL,
  terminal_kind TEXT NOT NULL CHECK(terminal_kind IN (
    'RejectedDurableExpired','ReservedExpiredBeforeSink',
    'ManualTargetExpiredBeforeAuthorization'
  )),
  event_canonical BLOB NOT NULL,
  event_sha256 TEXT NOT NULL CHECK(length(event_sha256)=64),
  append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
  immutable_audit_ref TEXT,
  created_at TEXT NOT NULL,
  UNIQUE(decision_identity,expires_at),
  CHECK(
    (terminal_kind='RejectedDurableExpired'
      AND authorization_identity IS NOT NULL AND attempt_identity IS NULL)
    OR
    (terminal_kind='ReservedExpiredBeforeSink'
      AND authorization_identity IS NOT NULL AND attempt_identity IS NOT NULL)
    OR
    (terminal_kind='ManualTargetExpiredBeforeAuthorization'
      AND authorization_identity IS NULL AND attempt_identity IS NULL)
  ),
  CHECK(expiry_event_identity=event_sha256),
  CHECK(freshness_observed_at>=expires_at),
  CHECK(
    (append_state='Pending' AND immutable_audit_ref IS NULL)
    OR
    (append_state='Appended'
      AND immutable_audit_ref IS NOT NULL
      AND length(trim(
        immutable_audit_ref,
        char(32) || char(9) || char(10) || char(13)
      )) > 0)
  )
);

CREATE TABLE retry_send_ownership(
  attempt_identity TEXT PRIMARY KEY
    REFERENCES retry_attempt_bindings(attempt_identity),
  decision_identity TEXT NOT NULL
    REFERENCES delivery_decisions(decision_identity),
  attempt_binding_identity TEXT NOT NULL UNIQUE
    REFERENCES retry_attempt_bindings(attempt_identity),
  execution_cycle_identity TEXT NOT NULL
    REFERENCES retry_cycles(cycle_identity),
  reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
  owner_instance_identity TEXT NOT NULL
    CHECK(length(trim(
      owner_instance_identity,
      char(32) || char(9) || char(10) || char(13)
    )) > 0),
  fence_token INTEGER NOT NULL CHECK(fence_token > 0),
  send_started_at TEXT NOT NULL,
  send_consumed INTEGER NOT NULL CHECK(send_consumed=1),
  pre_call_freshness_observed_at TEXT,
  terminal_sink_result_identity TEXT UNIQUE
    REFERENCES sink_results(result_event_identity)
    DEFERRABLE INITIALLY DEFERRED,
  ownership_state TEXT NOT NULL
    CHECK(ownership_state IN (
      'Started','FreshnessExpiredBeforeExternalCall',
      'TerminalRecorded','InterruptedUncertain'
    )),
  terminal_reason TEXT,
  terminal_at TEXT,
  created_at TEXT NOT NULL,
  CHECK(
    (ownership_state='Started'
      AND terminal_sink_result_identity IS NULL
      AND terminal_reason IS NULL AND terminal_at IS NULL)
    OR
    (ownership_state='FreshnessExpiredBeforeExternalCall'
      AND pre_call_freshness_observed_at IS NOT NULL
      AND terminal_sink_result_identity IS NULL
      AND terminal_reason='ExpiredFreshnessBeforeExternalCall'
      AND terminal_at=pre_call_freshness_observed_at)
    OR
    (ownership_state='TerminalRecorded'
      AND pre_call_freshness_observed_at IS NOT NULL
      AND terminal_sink_result_identity IS NOT NULL
      AND terminal_reason IS NOT NULL AND terminal_at IS NOT NULL)
    OR
    (ownership_state='InterruptedUncertain'
      AND terminal_sink_result_identity IS NULL
      AND terminal_reason IS NOT NULL AND terminal_at IS NOT NULL)
  )
);

-- The ownership pointer is written once, before the matching result insert in
-- the same transaction. The deferred FK rejects commit if the result never
-- appears; UNIQUE rejects two ownership rows pointing at one result.
CREATE TRIGGER trg_retry_send_ownership_terminal_result_once
BEFORE UPDATE OF ownership_state,terminal_sink_result_identity,
  pre_call_freshness_observed_at,terminal_reason,terminal_at
ON retry_send_ownership
WHEN NEW.ownership_state='TerminalRecorded'
BEGIN
  SELECT CASE WHEN NOT (
    OLD.ownership_state='Started'
    AND OLD.terminal_sink_result_identity IS NULL
    AND NEW.terminal_sink_result_identity IS NOT NULL
    AND NEW.pre_call_freshness_observed_at IS NOT NULL
    AND NEW.terminal_reason IS NOT NULL
    AND NEW.terminal_at IS NOT NULL
  ) THEN RAISE(ABORT,'BR-192 terminal result ownership must be written once') END;
END;

CREATE TRIGGER trg_retry_send_ownership_terminal_result_immutable
BEFORE UPDATE ON retry_send_ownership
WHEN OLD.ownership_state='TerminalRecorded' AND (
  NEW.ownership_state IS NOT OLD.ownership_state
  OR NEW.terminal_sink_result_identity IS NOT OLD.terminal_sink_result_identity
  OR NEW.pre_call_freshness_observed_at IS NOT OLD.pre_call_freshness_observed_at
  OR NEW.terminal_reason IS NOT OLD.terminal_reason
  OR NEW.terminal_at IS NOT OLD.terminal_at
)
BEGIN
  SELECT RAISE(ABORT,'BR-192 terminal result ownership is immutable');
END;

-- Only exact authoritative, non-late retry results participate in the
-- bijection. They may be inserted only after the ownership pointer is set;
-- the deferred FK then proves both rows exist at commit.
CREATE TRIGGER trg_retry_sink_result_requires_terminal_ownership
BEFORE INSERT ON sink_results
WHEN NEW.authoritative_for_state=1
 AND NEW.late_after_fence=0
 AND EXISTS (
   SELECT 1 FROM retry_attempt_bindings b
   WHERE b.attempt_identity=NEW.attempt_identity
 )
BEGIN
  SELECT CASE WHEN NOT EXISTS (
    SELECT 1 FROM retry_send_ownership o
    WHERE o.attempt_identity=NEW.attempt_identity
      AND o.decision_identity=NEW.decision_identity
      AND o.fence_token=NEW.fence_token
      AND o.ownership_state='TerminalRecorded'
      AND o.terminal_sink_result_identity=NEW.result_event_identity
  ) THEN RAISE(ABORT,'BR-192 authoritative retry result lacks exact terminal ownership') END;
END;

CREATE TABLE retry_cycles(
  cycle_identity TEXT PRIMARY KEY,
  cycle_ordinal INTEGER NOT NULL UNIQUE CHECK(cycle_ordinal >= 1),
  namespace_sha256 TEXT NOT NULL,
  owner_boot_identity TEXT NOT NULL,
  scheduled_for TEXT NOT NULL,
  started_at TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('Running','Completed','Failed')),
  terminal_phase TEXT NOT NULL DEFAULT 'NotPrepared'
    CHECK(terminal_phase IN (
      'NotPrepared','CompletionPending','CompletionAppended',
      'FailurePending','FailureAppended','Terminalized'
    )),
  candidate_query_calls INTEGER NOT NULL DEFAULT 0
    CHECK(candidate_query_calls BETWEEN 0 AND 1),
  queried_candidate_count INTEGER NOT NULL DEFAULT 0
    CHECK(queried_candidate_count >= 0),
  sorted_candidate_sha256 TEXT,
  provider_calls INTEGER NOT NULL DEFAULT 0 CHECK(provider_calls=0),
  renderer_calls INTEGER NOT NULL DEFAULT 0 CHECK(renderer_calls=0),
  sink_calls_state TEXT NOT NULL DEFAULT 'NotStarted'
    CHECK(sink_calls_state IN ('NotStarted','Confirmed','Indeterminate')),
  sink_calls_count INTEGER CHECK(sink_calls_count>=0),
  failure_reason TEXT CHECK(failure_reason IN (
    'retry_attempt_start_audit_unavailable',
    'authorization_reconciliation_blocked',
    'cycle_operation_failed','panic','join_error','process_interrupted'
  )),
  failure_payload_identity TEXT UNIQUE
    REFERENCES retry_cycle_failure_payloads(failure_payload_identity),
  failure_typed_fields_sha256 TEXT,
  failure_envelope_sha256 TEXT,
  completed_at TEXT,
  CHECK(
    (state='Running' AND terminal_phase='NotPrepared'
      AND failure_reason IS NULL
      AND failure_payload_identity IS NULL
      AND failure_typed_fields_sha256 IS NULL
      AND failure_envelope_sha256 IS NULL
      AND completed_at IS NULL)
    OR
    (state='Running'
      AND terminal_phase IN ('CompletionPending','CompletionAppended')
      AND failure_reason IS NULL
      AND failure_payload_identity IS NULL
      AND failure_typed_fields_sha256 IS NULL
      AND failure_envelope_sha256 IS NULL
      AND completed_at IS NOT NULL)
    OR
    (state='Running'
      AND terminal_phase IN ('FailurePending','FailureAppended')
      AND failure_reason IS NOT NULL
      AND failure_payload_identity IS NOT NULL
      AND length(failure_typed_fields_sha256)=64
      AND length(failure_envelope_sha256)=64
      AND completed_at IS NULL)
    OR
    (state='Completed' AND terminal_phase='Terminalized'
      AND failure_reason IS NULL
      AND failure_payload_identity IS NULL
      AND failure_typed_fields_sha256 IS NULL
      AND failure_envelope_sha256 IS NULL
      AND completed_at IS NOT NULL)
    OR
    (state='Failed' AND terminal_phase='Terminalized'
      AND failure_reason IS NOT NULL
      AND failure_payload_identity IS NOT NULL
      AND length(failure_typed_fields_sha256)=64
      AND length(failure_envelope_sha256)=64
      AND completed_at IS NOT NULL)
  ),
  CHECK(
    (sink_calls_state='NotStarted' AND sink_calls_count IS NULL)
    OR
    (sink_calls_state='Confirmed' AND sink_calls_count IS NOT NULL)
    OR
    (sink_calls_state='Indeterminate' AND sink_calls_count IS NULL)
  )
);

CREATE TABLE retry_cycle_failure_payloads(
  failure_payload_identity TEXT PRIMARY KEY,
  cycle_identity TEXT NOT NULL UNIQUE
    REFERENCES retry_cycles(cycle_identity),
  failure_reason TEXT NOT NULL CHECK(failure_reason IN (
    'retry_attempt_start_audit_unavailable',
    'authorization_reconciliation_blocked',
    'cycle_operation_failed','panic','join_error','process_interrupted'
  )),
  typed_preimage_canonical BLOB NOT NULL,
  typed_preimage_sha256 TEXT NOT NULL CHECK(length(typed_preimage_sha256)=64),
  failure_envelope_canonical BLOB NOT NULL,
  failure_envelope_sha256 TEXT NOT NULL CHECK(length(failure_envelope_sha256)=64),
  created_at TEXT NOT NULL
);

CREATE TABLE retry_cycle_audit_outbox(
  cycle_event_identity TEXT PRIMARY KEY,
  cycle_identity TEXT NOT NULL REFERENCES retry_cycles(cycle_identity),
  decision_identity TEXT REFERENCES delivery_decisions(decision_identity),
  event_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(event_ordinal >= 0),
  event_kind TEXT NOT NULL CHECK(event_kind IN (
    'Started','CandidateObserved','AuthorizationReconciliationBlocked',
    'DuplicateSuppressed','AdmissionResult','SinkAttemptStarted',
    'OrphanRecovered','Completed','Failed'
  )),
  event_canonical BLOB NOT NULL,
  event_sha256 TEXT NOT NULL,
  append_state TEXT NOT NULL CHECK(append_state IN ('Pending','Appended')),
  immutable_audit_ref TEXT,
  created_at TEXT NOT NULL,
  CHECK(decision_identity IS NULL OR decision_identity!='__BR192_CYCLE_SCOPE__'),
  CHECK(
    (
      event_kind IN (
        'Started','AuthorizationReconciliationBlocked','OrphanRecovered',
        'Completed','Failed'
      )
      AND decision_identity IS NULL
      AND event_ordinal=0
    )
    OR
    (
      event_kind IN (
        'CandidateObserved','DuplicateSuppressed','AdmissionResult'
      )
      AND decision_identity IS NOT NULL
      AND event_ordinal=0
    )
    OR
    (
      event_kind='SinkAttemptStarted'
      AND decision_identity IS NOT NULL
      AND event_ordinal>0
    )
  ),
  CHECK(
    (append_state='Pending' AND immutable_audit_ref IS NULL)
    OR
    (append_state='Appended'
      AND immutable_audit_ref IS NOT NULL
      AND length(trim(
        immutable_audit_ref,
        char(32) || char(9) || char(10) || char(13)
      )) > 0)
  )
);

CREATE TABLE retry_pre_call_expiry_authorities(
  authority_identity TEXT PRIMARY KEY,
  expiry_event_identity TEXT NOT NULL UNIQUE
    REFERENCES retry_expiry_audit_outbox(expiry_event_identity)
    DEFERRABLE INITIALLY DEFERRED,
  decision_identity TEXT NOT NULL
    REFERENCES delivery_decisions(decision_identity),
  attempt_identity TEXT NOT NULL UNIQUE
    REFERENCES retry_send_ownership(attempt_identity),
  execution_cycle_identity TEXT NOT NULL
    REFERENCES retry_cycles(cycle_identity),
  reservation_generation INTEGER NOT NULL CHECK(reservation_generation > 0),
  source_business_date TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  freshness_observed_at TEXT NOT NULL,
  authority_canonical BLOB NOT NULL,
  authority_sha256 TEXT NOT NULL CHECK(length(authority_sha256)=64),
  created_at TEXT NOT NULL,
  CHECK(authority_identity=authority_sha256),
  CHECK(authority_sha256=sha256_hex(authority_canonical)),
  CHECK(freshness_observed_at>=expires_at),
  CHECK(created_at=freshness_observed_at)
);

-- BR-192 expiry/start total order. A Pending start is already conflicting
-- authority because its immutable append may have happened before SQLite ack.
CREATE TRIGGER trg_retry_expiry_insert_requires_no_start_or_ownership
BEFORE INSERT ON retry_expiry_audit_outbox
WHEN NEW.terminal_kind='ReservedExpiredBeforeSink'
 AND (
   EXISTS (
     SELECT 1
     FROM retry_cycle_audit_outbox s
     JOIN retry_attempt_bindings b
       ON b.attempt_identity=NEW.attempt_identity
      AND b.decision_identity=NEW.decision_identity
      AND b.reservation_generation=s.event_ordinal
     WHERE s.decision_identity=NEW.decision_identity
       AND s.event_kind='SinkAttemptStarted'
   )
   OR EXISTS (
     SELECT 1 FROM retry_send_ownership o
     WHERE o.decision_identity=NEW.decision_identity
       AND o.attempt_identity=NEW.attempt_identity
   )
 )
 AND NOT (
   EXISTS (
     SELECT 1 FROM retry_send_ownership o
     WHERE o.decision_identity=NEW.decision_identity
       AND o.attempt_identity=NEW.attempt_identity
       AND o.ownership_state='FreshnessExpiredBeforeExternalCall'
       AND o.pre_call_freshness_observed_at=NEW.freshness_observed_at
   )
   AND EXISTS (
     SELECT 1 FROM retry_pre_call_expiry_authorities a
     WHERE a.expiry_event_identity=NEW.expiry_event_identity
       AND a.decision_identity=NEW.decision_identity
       AND a.attempt_identity=NEW.attempt_identity
       AND a.source_business_date=NEW.source_business_date
       AND a.expires_at=NEW.expires_at
       AND a.freshness_observed_at=NEW.freshness_observed_at
   )
   AND NOT EXISTS (
     SELECT 1 FROM sink_results r
     WHERE r.decision_identity=NEW.decision_identity
       AND r.attempt_identity=NEW.attempt_identity
   )
   AND EXISTS (
     SELECT 1
     FROM retry_cycle_audit_outbox s
     JOIN retry_attempt_bindings b
       ON b.attempt_identity=NEW.attempt_identity
      AND b.decision_identity=NEW.decision_identity
      AND b.reservation_generation=s.event_ordinal
     WHERE s.decision_identity=NEW.decision_identity
       AND s.event_kind='SinkAttemptStarted'
       AND s.append_state='Appended'
       AND s.cycle_identity=(
         SELECT o.execution_cycle_identity
         FROM retry_send_ownership o
         WHERE o.attempt_identity=NEW.attempt_identity
       )
   )
   AND NOT EXISTS (
     SELECT 1
     FROM retry_cycle_audit_outbox s
     JOIN retry_attempt_bindings b
       ON b.attempt_identity=NEW.attempt_identity
      AND b.decision_identity=NEW.decision_identity
      AND b.reservation_generation=s.event_ordinal
     JOIN retry_send_ownership o
       ON o.attempt_identity=NEW.attempt_identity
     WHERE s.decision_identity=NEW.decision_identity
       AND s.event_kind='SinkAttemptStarted'
       AND (s.append_state!='Appended'
            OR s.cycle_identity!=o.execution_cycle_identity)
   )
 )
BEGIN
  SELECT RAISE(ABORT,'BR-192 expiry conflicts with start/ownership authority');
END;

CREATE TRIGGER trg_retry_start_insert_rejects_expiry_authority
BEFORE INSERT ON retry_cycle_audit_outbox
WHEN NEW.event_kind='SinkAttemptStarted'
 AND EXISTS (
   SELECT 1 FROM retry_expiry_audit_outbox e
   WHERE e.decision_identity=NEW.decision_identity
 )
BEGIN
  SELECT RAISE(ABORT,'BR-192 start conflicts with expiry authority');
END;

CREATE TRIGGER trg_retry_start_append_rejects_expiry_authority
BEFORE UPDATE OF append_state ON retry_cycle_audit_outbox
WHEN OLD.event_kind='SinkAttemptStarted'
 AND OLD.append_state='Pending' AND NEW.append_state='Appended'
 AND EXISTS (
   SELECT 1 FROM retry_expiry_audit_outbox e
   WHERE e.decision_identity=OLD.decision_identity
 )
BEGIN
  SELECT RAISE(ABORT,'BR-192 start acknowledgement conflicts with expiry authority');
END;

CREATE TRIGGER trg_retry_send_ownership_rejects_expiry_authority
BEFORE INSERT ON retry_send_ownership
WHEN EXISTS (
  SELECT 1 FROM retry_expiry_audit_outbox e
  WHERE e.decision_identity=NEW.decision_identity
)
BEGIN
  SELECT RAISE(ABORT,'BR-192 ownership conflicts with expiry authority');
END;

CREATE TRIGGER trg_retry_pre_call_expiry_authority_insert_requires_started_ownership
BEFORE INSERT ON retry_pre_call_expiry_authorities
WHEN EXISTS (
       SELECT 1 FROM retry_expiry_audit_outbox e
       WHERE e.expiry_event_identity=NEW.expiry_event_identity
     )
 OR NOT EXISTS (
       SELECT 1
       FROM retry_send_ownership o
       JOIN retry_schedules rs
         ON rs.decision_identity=o.decision_identity
       JOIN retry_attempt_bindings b
         ON b.attempt_identity=o.attempt_identity
       JOIN delivery_attempts da
         ON da.attempt_identity=o.attempt_identity
        AND da.decision_identity=o.decision_identity
       JOIN retry_cycle_audit_outbox s
         ON s.cycle_identity=o.execution_cycle_identity
        AND s.decision_identity=o.decision_identity
        AND s.event_kind='SinkAttemptStarted'
        AND s.event_ordinal=o.reservation_generation
        AND s.append_state='Appended'
       WHERE o.attempt_identity=NEW.attempt_identity
         AND o.decision_identity=NEW.decision_identity
         AND o.execution_cycle_identity=NEW.execution_cycle_identity
         AND o.reservation_generation=NEW.reservation_generation
         AND o.ownership_state='Started'
         AND o.pre_call_freshness_observed_at IS NULL
         AND o.terminal_reason IS NULL
         AND o.terminal_at IS NULL
         AND da.state='AttemptInFlight'
         AND b.decision_identity=NEW.decision_identity
         AND b.reservation_generation=NEW.reservation_generation
         AND rs.terminal_state='Active'
         AND rs.last_attempt_binding_identity=NEW.attempt_identity
         AND rs.source_business_date=NEW.source_business_date
         AND rs.expires_at=NEW.expires_at
     )
 OR EXISTS (
       SELECT 1 FROM sink_results r
       WHERE r.attempt_identity=NEW.attempt_identity
     )
BEGIN
  SELECT RAISE(ABORT,'BR-192 invalid pre-call expiry authority');
END;

CREATE TRIGGER trg_retry_pre_call_expiry_authority_update_immutable
BEFORE UPDATE ON retry_pre_call_expiry_authorities
BEGIN
  SELECT RAISE(ABORT,'BR-192 pre-call expiry authority is immutable');
END;

CREATE TRIGGER trg_retry_pre_call_expiry_authority_delete_immutable
BEFORE DELETE ON retry_pre_call_expiry_authorities
BEGIN
  SELECT RAISE(ABORT,'BR-192 pre-call expiry authority is retained');
END;

CREATE TRIGGER trg_retry_send_ownership_pre_call_expiry_requires_authority
BEFORE UPDATE OF ownership_state,pre_call_freshness_observed_at,
  terminal_reason,terminal_at ON retry_send_ownership
WHEN NEW.ownership_state='FreshnessExpiredBeforeExternalCall'
 AND (
   NOT EXISTS (
     SELECT 1 FROM retry_pre_call_expiry_authorities a
     WHERE a.decision_identity=NEW.decision_identity
       AND a.attempt_identity=NEW.attempt_identity
       AND a.execution_cycle_identity=NEW.execution_cycle_identity
       AND a.reservation_generation=NEW.reservation_generation
       AND a.freshness_observed_at=NEW.pre_call_freshness_observed_at
   )
   OR EXISTS (
     SELECT 1 FROM sink_results r
     WHERE r.decision_identity=NEW.decision_identity
       AND r.attempt_identity=NEW.attempt_identity
   )
 )
BEGIN
  SELECT RAISE(ABORT,'BR-192 pre-call expiry ownership lacks authority or has sink result');
END;

CREATE TRIGGER trg_retry_sink_result_insert_rejects_pre_call_expiry_authority
BEFORE INSERT ON sink_results
WHEN EXISTS (
       SELECT 1 FROM retry_pre_call_expiry_authorities a
       WHERE a.decision_identity=NEW.decision_identity
         AND a.attempt_identity=NEW.attempt_identity
     )
  OR EXISTS (
       SELECT 1 FROM retry_send_ownership o
       WHERE o.decision_identity=NEW.decision_identity
         AND o.attempt_identity=NEW.attempt_identity
         AND o.ownership_state='FreshnessExpiredBeforeExternalCall'
     )
  OR EXISTS (
       SELECT 1 FROM retry_expiry_audit_outbox e
       WHERE e.decision_identity=NEW.decision_identity
         AND e.attempt_identity=NEW.attempt_identity
         AND e.terminal_kind='ReservedExpiredBeforeSink'
     )
BEGIN
  SELECT RAISE(
    ABORT,
    'BR-192 sink result conflicts with pre-call expiry authority'
  );
END;

CREATE UNIQUE INDEX retry_cycle_event_logical_slot
ON retry_cycle_audit_outbox(
  cycle_identity,
  COALESCE(decision_identity,'__BR192_CYCLE_SCOPE__'),
  event_kind,
  event_ordinal
);

CREATE UNIQUE INDEX retry_cycle_one_terminal
ON retry_cycle_audit_outbox(cycle_identity)
WHERE event_kind IN ('Completed','Failed');
```

`sink_calls_state/sink_calls_count` is the durable form of
`RetryCycleSinkCalls`: `NotStarted/NULL`, `Confirmed/n`, or
`Indeterminate/NULL`. A clean path that proves no sink boundary was crossed
may record `Confirmed/0`; a panic, cancellation, process death or `JoinError`
after start/claim evidence records `Indeterminate/NULL`. Recovery may report
its own independently measured zero calls, but it must not rewrite an
interrupted cycle to `Confirmed/0`. The live final pre-call expiry protocol is
not an interrupted path: its transaction proves the current attempt did not
cross the boundary and recounts the exact same-cycle ownership/result
bijection. If any other same-cycle `Started|InterruptedUncertain` ownership
remains, it atomically keeps `Indeterminate/NULL`; otherwise it atomically
restores `Confirmed(n)`. It never assumes `n=0`.

Cycle terminalization is one irreversible phase machine. Completion is a
four-step protocol:

1. `prepare_retry_cycle_completed` validates complete evidence, freezes the
   exact canonical Completed bytes, inserts the unique Pending Completed slot
   and changes only
   `Running/NotPrepared -> Running/CompletionPending`;
2. append authority receives those exact canonical bytes;
3. one acknowledgement transaction changes that outbox to Appended and the
   cycle to `Running/CompletionAppended`; and
4. `terminalize_retry_cycle_completed` revalidates exact bytes/ref and CASes
   only `Running/CompletionAppended -> Completed/Terminalized`.

Failure is the symmetric four-step protocol, never a one-transaction terminal
write:

1. `prepare_retry_cycle_failed` validates/quarantines uncertainty, recomputes
   the opaque `RetryCycleFailure` preimage digest, inserts the unique canonical
   `Failed` outbox as `Pending`, and changes only
   `Running/NotPrepared -> Running/FailurePending`;
2. append authority receives those exact canonical bytes;
3. one acknowledgement transaction changes the exact outbox
   `Pending -> Appended` and the cycle
   `Running/FailurePending -> Running/FailureAppended`; and
4. `terminalize_retry_cycle_failed` revalidates the appended immutable ref,
   Failed-outbox bytes, full persisted canonical typed preimage/envelope,
   closed reason, both recomputed hashes and completed uncertainty, then CASes
   `Running/FailureAppended -> Failed/Terminalized`.

An append or acknowledgement failure leaves completion at
`Running/CompletionPending`; a crash after acknowledgement leaves
`Running/CompletionAppended`. Recovery resumes only exact Completed bytes.
Once either completion phase exists, no error/panic/JoinError/boot path may
prepare or append Failed.

A failure append or acknowledgement failure therefore leaves the cycle
`Running/FailurePending`; a crash after acknowledgement leaves
`Running/FailureAppended`. Neither state is a terminal `Failed` claim. Startup,
same-cycle error, panic and `JoinError` recovery resume the same persisted
pending/appended terminal kind and never create different terminal bytes. A prior-boot
`Running/NotPrepared` row first quarantines indeterminate attempts and
append/acknowledges all uncertainty outboxes to a fixed point; only a second
transaction may persist the complete `ProcessInterrupted` failure payload and
prepare Failed. Prior-boot completion/failure Pending/Appended rows must load
and validate their full persisted payload/outbox bytes and finish only the
already-prepared kind without an in-memory DTO.

Do not add `current_retry_authorization_identity` to `delivery_decisions`.
SQLite cannot add the required foreign-key semantics with the previous
`ALTER TABLE` proposal. The unique `Active` row in
`retry_authorization_bindings` is the v6 current reference. Fresh v0 creation
and every v1/v2/v3/v4/v5-to-v6 upgrade create exactly this companion-table
manifest. Every fresh or migrated v6 `retry_cycles` row therefore includes the
same positive unique `cycle_ordinal`; the v6 manifest includes its column,
`NOT NULL`, `UNIQUE` and `CHECK(cycle_ordinal >= 1)` constraints plus triggers
that reject ordinal update and row deletion. Cycle insertion rederives
`cycle_identity` from the exact retained ordinal and canonical fields before
acceptance.

The v5-to-v6 transaction snapshots every existing decision identity,
row count and canonical/hash column plus every BR-194 replay object, audit
kind and manifest definition before creating objects. It copies no legacy
`retry_authorized` boolean into an authorization or binding: legacy booleans
are not evidence. Existing decision/disposition/attempt rows remain in place
and their snapshots must compare byte-for-byte after object creation.
`foreign_key_check`, full manifest comparison and invariant validation run
before `user_version=6` is the transaction's final statement. Any mismatch
rolls back all objects and the version. Tests compare a fresh-v6 manifest to
each upgraded manifest and prove existing v5 data and all BR-194 replay
semantics are unchanged.

Add immutable payload/delete triggers and monotonic state triggers exactly as
specified by the design. In particular:

- `counted_producer_attestations` uses a deferred FK and must be inserted before
  its decision while `NOT EXISTS(delivery_decisions)` for that identity; its
  insert trigger rejects an already-existing decision. The subsequent
  `delivery_decisions` insert trigger requires the exact companion for every
  `ReviewProviderTopN` decision, so both rows must first appear in one
  transaction. The companion requires the exact `ReviewProviderTopN` push
  kind, the one enabled R-09 seam and the current
  validated catalog identity, and recomputes
  `sha256_hex(attestation_canonical)` before accepting the stored lowercase
  digest. Its private validator requires and strips the exact frozen domain
  prefix; decode followed by canonical reserialization of the JSON suffix plus
  that same prefix must equal the stored BLOB and bind kind, seam, catalog identity,
  source business date and first Asia/Shanghai midnight expiry; every update
  and delete is rejected. The v5-to-v6 migration creates zero companion rows
  for legacy decisions and never infers one from envelope bytes;
- `retry_authorizations` canonical/source/identity/`authorized_at` fields
  plus all four producer-provenance fields cannot update and no row can be
  deleted. Insert requires an exact `counted_producer_attestations` companion
  join for the same decision/current catalog and exact kind/seam/hash values;
  manual `authorized_at` is copied exactly from the validated PAM session
  `validated_at`, while `created_at` remains insertion metadata and cannot
  affect eligibility;
- `retry_authorization_events` canonical/identity/transition fields cannot
  update, only `Pending -> Appended` with its exact immutable ref is allowed,
  and no row can be deleted;
- `retry_authorization_bindings` identity/decision/authorization/disposition/
  generation fields cannot update, insert generation must be the next value,
  only `Active -> Cleared` is legal, and the partial index permits one active
  row per decision;
- active-binding insert triggers require `RejectedDurable`, the current
  appended disposition, an `Appended/Applied` authorization and its exact
  appended `Applied` event;
- every new disposition or accepted/manual terminal transition clears the
  active binding and compatibility boolean in the same transaction;
- `retry_attempt_bindings` rejects every update/delete and its insert trigger
  accepts only the authoritative attempt installed by the same admission
  transaction, requires exact decision/cycle/authorization/disposition/binding
  generation/reservation generation/owner/fence equality, requires the
  admission-selected next one-based `retry_ordinal`, and prevents a second
  attempt for the same decision/ordinal or cycle/decision/generation;
- `retry_cycles.cycle_ordinal` is positive and globally unique; ordinal,
  identity and canonical identity-preimage fields cannot update, the row
  cannot be deleted, and inserts whose exact domain/schema/field order/
  timestamp encoding/ordinal do not rederive `cycle_identity` fail closed;
- every fence column is SQLite `INTEGER CHECK(fence_token>0)` and every Rust
  fence field is `i64`; TEXT/String conversion or alternate fence types are
  forbidden;
- `retry_send_ownership` rejects update/delete of identity, binding,
  execution-cycle, reservation generation, owner, positive-i64 fence,
  `send_started_at` or `send_consumed`; only
  `Started -> FreshnessExpiredBeforeExternalCall|TerminalRecorded|
  InterruptedUncertain` is legal after ordinary claim. The freshness terminal
  requires the exact private pre-call clock observation at/after schedule
  expiry, exact reason/time equality, zero sink result and same-transaction
  deferred companion plus Pending expiry insertion. `TerminalRecorded`
  requires the exact pre-call observation and write-once
  `terminal_sink_result_identity` to be persisted with the sink result; every
  other state requires that identity to remain NULL. The atomic appended-start
  quarantine may directly insert
  `InterruptedUncertain` to consume an otherwise unowned send right;
- `terminal_sink_result_identity` is a nullable `UNIQUE DEFERRABLE INITIALLY
  DEFERRED` FK. `trg_retry_send_ownership_terminal_result_once` permits only
  `Started/NULL -> TerminalRecorded/non-NULL` once;
  `trg_retry_send_ownership_terminal_result_immutable` freezes that pointer,
  observation, reason and time thereafter; and
  `trg_retry_sink_result_requires_terminal_ownership` rejects every retry-
  attempt result satisfying exact predicate
  `authoritative_for_state=1 AND late_after_fence=0` unless it reverse-joins
  one same-attempt `TerminalRecorded` ownership with the same result identity,
  decision identity and fence token. The ownership pointer is updated before
  the result insert in the same transaction; the deferred FK rejects a commit
  missing the result. Late or non-authoritative rows do not satisfy the
  terminal relation. Fresh-v6 and every v5-to-v6 manifest must contain all
  three exact trigger definitions and the unique deferred FK;
- `TerminalRecorded` therefore requires the exact authoritative, non-late
  terminal sink-result row written by `record_sink_result` in the same
  transaction and its exact write-once identity; every authoritative cycle
  result must reverse-join one and only one same-cycle `TerminalRecorded`
  ownership;
  the separate negative test
  `br192_authoritative_retry_result_requires_exact_terminal_ownership_reverse_join`
  deliberately attempts a result-first authoritative/non-late INSERT while
  ownership is still `Started` with a NULL terminal-result pointer; it must be
  aborted immediately by
  `trg_retry_sink_result_requires_terminal_ownership`, leave zero result rows,
  and leave ownership byte-identical. This is distinct from the positive
  pointer-first transaction and its failpoint;
  `InterruptedUncertain` requires absence of any terminal sink result and exact
  reason `ProcessInterruptedAfterSinkStart`;
- `retry_pre_call_expiry_authorities` accepts only the private final pre-call
  path with the exact Active schedule, `AttemptInFlight` attempt, `Started`
  ownership, exact Appended start, matching cycle/generation/source date/
  expiry/observation and no sink result. Its canonical domain and compact JSON
  are fixed as
  `stock_analysis.durable_delivery.br192.pre_call_expiry_authority.v1\0` plus
  ordered `schema_version,rule_id,expiry_event_identity,
  decision_identity,attempt_identity,
  execution_cycle_identity,reservation_generation,source_business_date,
  expires_at,freshness_observed_at`; `authority_identity` is deliberately not
  an input so the identity rule is non-self-referential.
  `sha256_hex(authority_canonical)` must equal both hash and identity. It is
  immutable and its deferred expiry FK makes companion-only or
  companion+ownership partial commit impossible;
- `trg_retry_sink_result_insert_rejects_pre_call_expiry_authority` rejects every `sink_results` insert for the same
  decision/attempt once a pre-call companion, freshness-expired ownership or
  Reserved expiry row exists. Together with the companion-insert result check
  and the ownership/expiry result-absence rechecks, every possible result-first,
  result-between-members and result-after-commit ordering fails closed while
  the legal companion -> ownership -> expiry order remains executable;
- the exact interleaving regression proves: result first rejects companion;
  result after companion but before ownership rejects the ownership update;
  result after ownership but before expiry rejects expiry insertion; result
  after the committed triple, including from a second connection, is rejected
  by the reverse trigger; and the zero-result companion -> ownership -> expiry
  triple commits successfully;
- ownership insert requires the exact appended `SinkAttemptStarted` row,
  current attempt/binding/fence and a successful same-transaction
  `Reserved -> AttemptInFlight` CAS, except that the zero-sink failure
  quarantine may consume an appended-start send right and atomically insert it
  directly as `InterruptedUncertain` with exact
  `ProcessInterruptedAfterSinkStart`. In that exception the same transaction
  moves the decision through uncertainty and no `Reserved` ownership or
  externally visible `Started` row remains;
- `retry_cycle_failure_payloads` rejects every update/delete; its insert
  requires a Running/NotPrepared cycle, exact closed reason, canonical private
  decode→reserialize equality for the typed preimage/envelope, both
  domain-separated hashes and matching cycle identity. The same transaction
  points `retry_cycles.failure_payload_identity` at that row and prepares the
  Pending Failed outbox; the circular references are validated by
  `foreign_key_check` before migration commit;
- `retry_cycles` and `retry_cycle_audit_outbox` reject deletion and allow only
  their documented monotonic terminal-phase projection/append
  acknowledgements; one unique terminal slot irrevocably selects Completed or
  Failed, and any opposite-kind insert fails closed; a failure
  transition requires exact payload identity, typed-preimage hash and envelope
  hash agreement across all three authorities. The outbox
  accepts a byte-identical replay of one logical event slot but fails the cycle
  closed when the same slot carries different canonical bytes or a different
  hash; and
- `retry_schedules` rejects every deletion, a lower attempt count, a version
  change other than `OLD.version + 1` on an authority update, clearing/changing
  `exhausted_at`, first exhaustion below three attempts, and clearing or moving
  `next_eligible_at` earlier. Its last-attempt reference must resolve to an
  immutable attempt binding for the same decision whose `retry_ordinal` equals
  `automatic_attempts_started`; `source_business_date` and `expires_at` are
  immutable, the latter must be the exact first Asia/Shanghai midnight after
  the former, and terminal state may move only once from `Active` to
  `ExpiredFreshness|Exhausted|Completed`. `Exhausted` requires count three and
  the immutable first `exhausted_at`; `Completed`/`ExpiredFreshness` reject any
  later schedule authority update. Creating a FrozenRejection authorization
  and its rejection disposition must insert the zero-attempt schedule in the
  same transaction with `next_eligible_at=observed_at + 30s`, exact source
  business date, exact expiry and `terminal_state='Active'`; and
- `retry_expiry_audit_outbox` inserts require the exact current Active schedule,
  current appended rejection disposition and producer attestation join;
  scheduled definite/Reserved expiry also requires the exact appended/applied
  authorization, while the manual-before-authorization kind requires its
  absence. Its `event_canonical` is the exact domain
  `stock_analysis.durable_delivery.br192.retry_expired_freshness.v1\0` followed
  by compact ordered JSON
  `schema_version,rule_id,decision_identity,rejection_disposition_identity,
  authorization_identity(nullable),attempt_identity,source_business_date,expires_at,
  freshness_observed_at,terminal_kind`; date is ISO and UTC uses `Z` with nine fractional digits.
  The private validator strips the exact prefix and decode/re-encodes the JSON
  suffix; `sha256_hex(event_canonical)` must equal both the stored digest and
  `expiry_event_identity`, and all typed columns must equal the decoded values.
  `RejectedDurableExpired` requires authorization and no attempt;
  `freshness_observed_at` must be the library-owned production-clock reading
  captured in the owning `BEGIN IMMEDIATE`, must compare
  `freshness_observed_at>=expires_at`, and must exactly match the decoded
  canonical value. Production callers cannot supply it. Lexical SQL comparison
  is valid only after the trigger/private validator proves both timestamps are
  the exact fixed-width UTC `Z` form with nine fractional digits.
  `ReservedExpiredBeforeSink` requires authorization plus the exact current
  pre-start retry attempt with no Pending/Appended `SinkAttemptStarted` logical
  slot and no send-ownership row, except for the same-transaction final pre-call
  protocol whose exact deferred companion and ownership are
  `FreshnessExpiredBeforeExternalCall`, whose attempt and observation equal the
  expiry row, and which has no authoritative sink result;
  `ManualTargetExpiredBeforeAuthorization` requires both nullable identities
  absent and a same-transaction zero-attempt Active schedule for a current
  definite R-09 rejection that never had automatic authority. All payload/key fields
  are immutable, no row can be deleted, and the only update is exact
  `Pending -> Appended` with a valid immutable ref. Schedule terminalization,
  authorization-binding clear and optional pre-start reservation release are
  permitted only after that exact row is Appended; a byte-identical replay is
  idempotent while different bytes/hash/kind fail closed. Expiry insertion,
  start-slot insertion/acknowledgement and ownership insertion are serialized
  by `BEGIN IMMEDIATE` plus the exact trigger set
  `trg_retry_expiry_insert_requires_no_start_or_ownership`,
  `trg_retry_start_insert_rejects_expiry_authority`,
  `trg_retry_start_append_rejects_expiry_authority` and
  `trg_retry_send_ownership_rejects_expiry_authority`, plus
  `trg_retry_pre_call_expiry_authority_insert_requires_started_ownership`,
  `trg_retry_pre_call_expiry_authority_update_immutable`,
  `trg_retry_pre_call_expiry_authority_delete_immutable` and
  `trg_retry_send_ownership_pre_call_expiry_requires_authority`, plus the ninth
  reverse guard
  `trg_retry_sink_result_insert_rejects_pre_call_expiry_authority`. Schema
  post-validation and the compliance catalog require all twelve exact normalized
  SQL definitions. If any start logical
  slot for the current attempt already exists, including Pending, or ordinary
  current-attempt ownership exists, expiry
  writes no outbox row and routes the attempt to uncertainty. The sole
  non-uncertain ownership case is the same-transaction pre-call no-sink
  protocol ordered as deferred companion insert, ownership terminal update,
  then expiry insert. The companion FK is deferred to commit, the ownership
  trigger requires the companion and rechecks result absence, the expiry
  trigger requires both and rechecks result absence, and the reverse result
  trigger rejects any result inserted after companion/ownership/expiry. Its
  Appended start exactly matches ownership execution cycle and reservation
  generation; any Pending/different-cycle current-attempt start still
  conflicts. Historical terminal-attempt starts do not conflict with the
  current attempt. If expiry Pending/Appended
  exists first, all later start preparation/acknowledgement/ownership writes
  fail closed, so the expiry outbox cannot be superseded or stranded.

Add indexes for unique active authorization binding, pending authorization
append/apply, pending authorization-event append, pending cycle audit and
candidate schedule lookup, plus pending expiry-audit append ordered by
`created_at COLLATE BINARY, expiry_event_identity COLLATE BINARY`.

SQLite migrations must use the repository's idempotent column/table detection;
there is no v6 `ALTER TABLE delivery_decisions`. Detection does not replace
schema versioning: a database claiming v6 while any required companion object
is absent or incompatible fails post-validation.

Reuse or centralize the existing Rust `validate_immutable_ref` predicate so it
matches SQL exactly: after trimming only ASCII space, tab, LF and CR, at least
one character remains. Do not add a divergent v6 validator. Every v6 append
acknowledgement must call it. Test `NULL`, empty, each whitespace character
alone, mixed whitespace and a valid reference through both Rust and every new
SQLite ref constraint.

- [ ] **Step 4: Add post-validation**

Extend BR-192 database validation to reject:

- a Rust/rusqlite durable connection on which deterministic BLOB-only
  `sha256_hex` was registered before complete main/WAL/SHM descriptor binding,
  or was not registered/fixed-vector self-tested before configuration/schema
  work;
- a Python BR-194 verifier that calls `create_function`, performs DML or
  executes a trigger instead of independently hashing returned BLOBs with
  `hashlib.sha256` and reading the catalog;
- any fresh/upgraded v6 manifest that omits or changes a preserved v5 BR-194
  replay object, audit kind, trigger semantic or row;
- any authority row/outbox whose stored lowercase digest differs from
  `sha256_hex(canonical_blob)`, and any v6 trigger catalog that omits the same
  recomputation;
- any decision/authorization that claims enabled producer provenance without
  exactly one same-decision `counted_producer_attestations` row, any synthetic
  companion for a migrated v5 decision, or any companion whose canonical
  kind/seam/catalog/source-date/expiry differs from its columns/current
  validated catalog;
- active binding not targeting the current disposition;
- active binding whose authorization is not `Appended/Applied` or is missing
  its exact appended `Applied` authorization event;
- more than one active binding or an illegal binding generation;
- historical cleared binding that lost its immutable identity/FKs, while
  explicitly allowing it to differ from the later current disposition;
- retry-origin authoritative attempt without exactly one immutable attempt
  binding matching its authorization/disposition/cycle/binding generation/
  reservation generation/owner/fence;
- retry-origin `Reserved` without that attempt/binding and exact schedule
  increment, or an attempt/binding/schedule increment without the matching
  `Reserved` transition in the same committed admission;
- a FrozenRejection authorization/disposition without the same-transaction
  zero-attempt schedule at exact `observed_at + 30s`, immutable source
  business date, exact next-Shanghai-midnight expiry and Active terminal state;
- an expired schedule returned as candidate/admitted/manual-authorized, an
  expiry changed by a caller, or terminal `ExpiredFreshness` without its one
  exact appended/acknowledged immutable outbox event and zero sink call; also
  reject a Pending expiry outbox that was terminalized early, an Appended row
  whose schedule/binding/reservation projection did not terminalize, or an
  expiry outbox whose optional attempt/terminal kind disagrees with definite
  rejected versus pre-start Reserved state;
- a pre-call expiry companion without its exact expiry row at commit, matching
  terminal ownership, exact Active-schedule/start/binding join, fixed canonical
  bytes/hash or zero sink result; mutation/deletion of a companion; or a
  `FreshnessExpiredBeforeExternalCall` ownership without that exact companion;
- any sink-result row coexisting with a pre-call expiry companion,
  `FreshnessExpiredBeforeExternalCall` ownership or
  `ReservedExpiredBeforeSink` expiry authority; any manifest missing the ninth
  reverse sink-result trigger or either result-absence recheck; or a pre-call
  expiry transaction whose durable exact-result recount did not atomically
  keep `Indeterminate/NULL` when any other same-cycle
  `Started|InterruptedUncertain` remains, or restore `Confirmed(n)` when none
  remains;
- `AttemptInFlight` without exactly one matching consumed send-ownership row,
  consumed ownership whose state/attempt/binding/execution-cycle/generation/
  owner/fence/start time differs, or a `Started` ownership that does not point
  to the current `AttemptInFlight`;
- a historical `delivery_attempts.state='AttemptInFlight'` treated as live
  after the complete derived `ExpiredFreshnessBeforeSink` projection exists;
  conversely, an `AlreadyTerminalized` result when any Appended expiry,
  schedule terminal, decision detachment, fence revocation, reservation/head
  release, cleared binding, zero-result or companion/ownership member is
  absent or mismatched;
- `TerminalRecorded` ownership without its exact non-NULL
  `terminal_sink_result_identity`, without exactly that authoritative terminal
  result, or with a non-bijective cycle reverse join; any other ownership state
  with non-NULL result identity or any terminal result; or
  `InterruptedUncertain` with a reason other than
  `ProcessInterruptedAfterSinkStart`;
- `FailurePending|FailureAppended|Failed` without one immutable full
  failure-payload row whose closed reason, canonical typed preimage, canonical
  envelope, both recomputed hashes, cycle identity and Failed outbox agree; a
  prior-boot terminalizer is tested after all in-memory DTOs are gone;
- authorization canonical/hash mismatch;
- authorization-event canonical/hash/ref mismatch;
- missing, non-positive, duplicate, updated or deleted cycle ordinal, or any
  retained cycle whose identity does not rederive from the exact
  domain/schema/field order/timestamp encoding/ordinal canonical preimage;
- schedule attempts outside 0..=3, counter/version rollback, deletion,
  exhaustion clearing or earlier eligibility, or a nonzero attempt count whose
  unique `last_attempt_binding_identity` does not FK-resolve to the same
  decision and exact retry ordinal;
- completed cycle whose candidate-query count is not exactly one, and an
  authorization-reconciliation-blocked cycle whose count is not zero;
- any cycle without exactly one global `Started`, a terminal cycle without
  exactly one of global `Completed` or `Failed`, more than one global
  authorization-blocked event, or a logical event slot whose canonical bytes
  or hash conflict with the retained row;
- a candidate reaching admission without exactly one `AdmissionResult`, an
  attempted-set duplicate without exactly one `DuplicateSuppressed`, or a
  `SinkAttemptStarted` whose outbox cycle is not its current execution cycle
  or whose canonical admission-cycle/decision/attempt/authorization/
  disposition/binding generation/reservation generation/owner/fence does not
  match the immutable attempt binding;
- appended new outbox row whose ref is null, empty or space/tab/LF/CR-only; and
- mutation/deletion of retained canonical rows.

The cycle-identity mutation matrix changes one variable at a time: domain,
schema version, declared field order, compact UTF-8 encoding, timestamp
precision/offset, ordinal, namespace, owner or schedule. Every mutation must
fail exact rederivation with zero cycle/`Started` write. Schema mutation
fixtures separately remove `NOT NULL`, `UNIQUE`, the positive `CHECK`, ordinal
immutability or retained-row delete protection; complete v6 manifest
post-validation must reject every fixture before advancing `user_version`.

Preserve the existing fixed-HEAD identifier
`br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions`
and extend its fixture/assertion coverage in place; do not rename it. Add
separate BR-192 companion tests
`br192_schema_v6_fresh_and_v1_v2_v3_v4_v5_upgrade_paths_validate`,
`br192_schema_v6_cycle_ordinal_manifest_is_identical_across_v0_v1_v2_v3_v4_v5`
and `br192_schema_newer_than_v6_fails_before_mutation`. Preserve the existing
v1/v2 invalid-history cases. The migration tests build isolated databases at
v0, v1, v2, v3, v4 and v5, initialize them to v6, compare the
complete required-object manifest, initialize v6 again, prove v5
decision/disposition/attempt and BR-194 replay data is unchanged and no legacy
boolean was promoted, prove both authorization tables contain only
`created_at` plus their semantically authoritative apply time where applicable
and no append-authority timestamp column. `user_version=6` is validate-only
and repeatable; exact newer version `user_version=7` is rejected without
before/after metadata or schema change.

- [ ] **Step 5: Run schema tests and commit**

```bash
cargo test --lib durable_delivery::tests::br192_retry_authorization_ -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_append_acknowledgements_store_no_untrusted_timestamp -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_active_binding_is_unique_and_historical_binding_survives_current_change -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_attempt_binding_freezes_authorization_disposition_cycle_generations_owner_and_fence -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_reserved_attempt_binding_and_schedule_relation_is_atomic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_fence_is_positive_integer_i64_across_schema_and_contract -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_send_ownership_is_retained_consumed_and_monotonic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_ordinal_is_positive_unique_immutable_and_non_deletable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_identity_rederives_from_retained_ordinal_and_exact_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_schedule_is_retained_and_all_authority_fields_are_monotonic -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_schedule_last_attempt_is_fk_bound_to_exact_retry_attempt_ordinal -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_logical_slots_are_unique_and_conflicting_bytes_fail_closed -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_global_started_and_terminal_cardinality_is_exact -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_v6_cycle_ordinal_manifest_is_identical_across_v0_v1_v2_v3_v4_v5 -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_v6_ -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_newer_than_v6_fails_before_mutation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_all_v6_immutable_refs_reject_empty_and_ascii_whitespace_only -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v5_to_v6_preserves_br194_replay_manifest_audit_kinds_and_rows -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_is_registered_before_every_schema_path -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_registration_follows_complete_descriptor_binding -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_never_runs_before_wal_shm_attestation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rusqlite_031_exposes_utf8_deterministic_and_innocuous_function_flags -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_rejects_null_text_and_wrong_type -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v6_authority_triggers_recompute_canonical_sha256 -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v6_authority_triggers_reject_bytes_hash_and_combined_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_python_br194_verifier_uses_hashlib_without_sql_callback_or_trigger_execution -- --exact --test-threads=1
bash tools/compliance/lib/check_br192_provider_free_retry.sh
bash tools/compliance/lib/check_br194_review_dependency.sh
python3 tools/release/verify_br194_review_join.py --help
git add Cargo.toml src/durable_delivery/schema.rs src/durable_delivery/coordinator.rs src/durable_delivery/tests.rs tools/compliance/check.sh tools/compliance/lib/check_br192_provider_free_retry.sh tools/compliance/lib/check_br194_review_dependency.sh tools/release/verify_br194_review_join.py
git commit -m "feat: add BR-192 v6 authority schema"
```

Expected: every command reports a non-zero test count and passes.

### Task 3: Implement exact authorization append/apply recovery

**Files:**

- Modify `src/durable_delivery/coordinator.rs`
- Modify `src/durable_delivery/tests.rs`

- [ ] **Step 1: Write authorization recovery RED tests**

Add exact tests:

```rust
#[test]
fn br192_envelope_retry_boolean_never_authorizes_without_current_frozen_authority() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_frozen_rejection_authority_requires_appended_disposition_and_authorization() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_frozen_rejection_authorization_and_initial_schedule_share_one_transaction() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authorization_recovers_insert_append_ack_and_apply_crashes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authorization_apply_and_invalidate_events_recover_before_projection_cas() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_identical_authorization_replay_returns_stored_record() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_different_authorization_bytes_for_same_rejection_fail_closed() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_stale_authorization_is_invalidated_not_retargeted() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pending_authorization_blocks_cycle_before_candidate_query_and_sink() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_new_disposition_clears_active_binding_but_preserves_history() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authorization_reconcile_bound_returns_exact_typed_variant() { panic!("BR-192 RED: named contract is not implemented"); }
```

Use `MemoryAppendPort` to prove byte-identical append. Add no new append fake.
Use existing database operation test hooks to stop after insert, after append
ack SQL and before apply commit.

- [ ] **Step 2: Prove RED with full test paths**

Run each as:

```bash
cargo test --lib durable_delivery::tests::br192_envelope_retry_boolean_never_authorizes_without_current_frozen_authority -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_frozen_rejection_authority_requires_appended_disposition_and_authorization -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_frozen_rejection_authorization_and_initial_schedule_share_one_transaction -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_recovers_insert_append_ack_and_apply_crashes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_apply_and_invalidate_events_recover_before_projection_cas -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_identical_authorization_replay_returns_stored_record -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_different_authorization_bytes_for_same_rejection_fail_closed -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_stale_authorization_is_invalidated_not_retargeted -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pending_authorization_blocks_cycle_before_candidate_query_and_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_new_disposition_clears_active_binding_but_preserves_history -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_reconcile_bound_returns_exact_typed_variant -- --exact --test-threads=1
```

Expected: `running 1 test`, then failure.

- [ ] **Step 3: Create frozen-rejection authorization in the disposition transaction**

When a definite sink rejection freezes:

```rust
if rejection.retry_authorized {
    insert_retry_authorization(
        transaction,
        RetryAuthorizationSource::FrozenRejection,
        &stored.decision_identity,
        &disposition_identity,
        None,
        rejection.observed_at,
    )?;
    insert_initial_retry_schedule(
        transaction,
        &stored.decision_identity,
        rejection.observed_at + TimeDelta::seconds(30),
    )?;
}
```

Do not derive candidate status from `envelope.retry_authorized`. Pre-sink denial
creates no authorization. For this non-manual source,
`rejection.observed_at` is the authority-owned `authorized_at`; the transaction
may record a different insertion `created_at`, but candidate scheduling never
uses it. The exact schedule fields are `automatic_attempts_started=0`,
`next_eligible_at=rejection.observed_at + 30s`, `exhausted_at=NULL`,
`last_attempt_binding_identity=NULL`, `version=0`. The rejection disposition,
FrozenRejection authorization and schedule commit or roll back together; no
recovery step may synthesize a missing schedule later.

- [ ] **Step 4: Implement one recovery drain**

Add:

```text
reconcile_one_retry_authorization(append, now) -> Result<bool>
```

It:

1. loads one `PendingAppend` canonical row using
   `WHERE append_state='PendingAppend' ORDER BY created_at COLLATE BINARY ASC,
   authorization_identity COLLATE BINARY ASC LIMIT 1`; null, empty or
   ASCII-whitespace-only ordering keys are rejected before return;
2. requires the target rejection disposition to be `Appended` with a valid
   immutable ref; otherwise it leaves the authorization pending;
3. calls `append_exact("RetryAuthorization", identity, bytes, sha)`;
4. acknowledges the same hash/ref with one CAS;
5. only when no `PendingAppend` row exists, loads one row using
   `WHERE append_state='Appended' AND apply_state='PendingApply' ORDER BY
   created_at COLLATE BINARY ASC, authorization_identity COLLATE BINARY ASC
   LIMIT 1`, with the same key validation;
6. deterministically inserts the stable `Applied` event when the target is
   current at observation, otherwise the stable `Invalidated` event, without
   using a cycle identity;
7. calls `append_exact("RetryAuthorizationEvent", event_identity, bytes, sha)`
   and acknowledges the exact event ref;
8. only after that acknowledgement, CASes `PendingApply -> Applied` and creates
   the unique `Active` companion binding if the decision remains
   `RejectedDurable` and its target is current;
9. if the target becomes stale before the Applied CAS, leaves `PendingApply`,
   inserts/appends/acknowledges the distinct stable `Invalidated` event and only
   then CASes `PendingApply -> Invalidated`; and
10. returns after advancing at most one selected authorization state; the
    caller allows exactly
    `MAX_AUTHORIZATION_RECONCILE_STEPS_PER_RUN=4096` successful progress
    returns, then performs one non-mutating pending-row check. A remaining row
    returns
    `DurableDeliveryError::AuthorizationReconciliationBoundExceeded {
    max_steps: 4096, pending_authorization_identity }`, where the identity is
    the exact immutable identity selected by that check; false progress with no
    pending row is the only successful fixed point. The exact test must pattern
    match the public variant and both field values, not a message or generic
    error.

Crash hooks stop after transition-event insert, append, acknowledgement and
before the projection/binding CAS. Recovery at each hook reuses the same stable
event identity and bytes and never applies/invalidates/binds first.

Candidate discovery is forbidden until this drain reaches a fixed point. If a
row remains `PendingAppend`/`PendingApply` or an append fails, write and append
`AuthorizationReconciliationBlocked`, durably fail the cycle and assert zero
candidate-query and sink counts. Do not return unreachable authorization
pending variants from `admit_authorized_retry`.

After authorization reconciliation, actively drain expiry before querying
candidates. Add the three coordinator methods frozen in design §2.4 and a
runtime `drain_expired_retry_schedules_to_fixed_point`. Each pass materializes
the complete ordered `terminal_state=Active AND expires_at<=now` snapshot,
then consumes the closed `RetryExpiryPreparationOutcome`. `ExpiryPrepared`
appends/acknowledges the exact `retry_expiry_audit_outbox` bytes represented by
`PreparedRetryExpiredFreshness` and terminalizes by CAS. `StartAuthorityWins`
writes no expiry row, append/acknowledges the retained uncertainty authority,
clears the active binding and moves the schedule out of `Active` before the
drain advances. Its exact reason precedence is `SendOwnershipAuthority >
AppendedStartAuthority > PendingStartAuthority`. Freshness expiry is
returned as `RetryIneligibility::ExpiredFreshness { expires_at }`, never as a
`DurableDeliveryError` variant. Repeat with a fresh coordinator-owned clock
observation until the complete snapshot is empty. The candidate read returns the
closed `RetryCandidateSnapshot::ExpiredFound|Candidates`; it checks expired
Active rows first in the same transaction and may never hide one with only
`expires_at>now`. `ExpiredFound` is drained and the read is retried.

If `admit_authorized_retry` itself first observes expiry, it returns the closed
`RetryAdmission::ExpiredFreshnessPrepared { expires_at, prepared }`; it must not
claim `NoLongerEligible` yet. The runner first append/ack/reconciles the exact
prepared expiry and only then persists the cycle
`NoLongerEligible(RetryIneligibility::ExpiredFreshness { expires_at })`.
Append/reconcile failure typed-fails the cycle without claiming terminalized
expiry.

The expiry drain handles both definite rejected rows and a pre-start Reserved
attempt. For the latter it requires no current-attempt Pending/Appended start
and no ownership, writes retained terminal `ExpiredFreshnessBeforeSink`,
releases the reservation and terminalizes the schedule. Current-attempt start
or ordinary ownership routes through typed `StartAuthorityWins` uncertainty
instead. Reserved processing rechecks expiry before start preparation and in
the send-ownership claim transaction through closed outcomes, never by error
text. The total matrix is: no start and expiry before start preparation gives
definite expiry; Pending/Appended start followed by expiry before claim gives
typed uncertainty with no permit/expiry row/sink; ownership claimed fresh but
final pre-call expiry gives the Transaction-A/B definite no-call exception;
an existing terminal result wins normally. Historical terminal attempts do not
block current-attempt expiry; partial identity/canonical/hash matches fail as
corruption.

The final pre-call expiry is deliberately a two-transaction protocol separated
by the immutable append:

1. transaction A consumes the permit without external I/O, inserts the deferred
   companion, advances ownership to
   `FreshnessExpiredBeforeExternalCall`, inserts the exact Pending
   `ReservedExpiredBeforeSink` row and validates every same-cycle ownership.
   `TerminalRecorded` must join exactly one result satisfying
   `authoritative_for_state=1 AND late_after_fence=0`;
   `FreshnessExpiredBeforeExternalCall`, `Started` and
   `InterruptedUncertain` must join zero. Every authoritative sink result for
   the cycle must also join exactly one same-cycle `TerminalRecorded`
   ownership. Let `n` be the cardinality of that proven bijection. If any
   other same-cycle
   `Started|InterruptedUncertain` remains, checked-update the selected cycle
   from `Indeterminate/NULL` to `Indeterminate/NULL`; otherwise checked-CAS
   `Indeterminate/NULL -> Confirmed(n)`. The selected cycle update must affect
   exactly one row. Missing/double/orphan/extra results, a result joined to
   zero or multiple ownership rows, an ambiguous ownership with a result,
   unexpected state, stale evidence or any other count rolls back the entire
   transaction. Caller memory cannot provide `n`; and
2. after the exact expiry bytes are appended and the same row is acknowledged
   `Pending -> Appended`, transaction B revalidates the full companion,
   ownership, appended expiry, zero-result and attempt/binding/schedule join,
   then CASes only the effective expiry projection. For the pre-call case it
   changes the decision `AttemptInFlight -> RejectedDurable`, preserves the
   original rejection disposition, clears the current-attempt, budget and
   cooldown pointers, releases reservation/head authority, revokes the fence,
   clears the active binding with reason `expired_freshness`, and changes the
   schedule `Active -> ExpiredFreshness` atomically. It never inserts a new
   disposition or sink result.

The fixed v5 `delivery_attempts.state` CHECK has no expiry token, so transaction
B deliberately leaves that historical row as `AttemptInFlight`; it must never
mislabel the no-call outcome `Rejected` or `Uncertain`. The effective terminal
`ExpiredFreshnessBeforeSink` fact is derived only from the exact Appended
expiry row, terminal schedule, decision no longer pointing at the attempt,
revoked fence, released reservation/head, zero sink result and matching
companion/ownership. Candidate selection, orphan/uncertainty recovery,
post-validation and evidence verification must recognize this complete derived
terminal first and exclude it from resend/quarantine. A crash before append
resumes the same Pending bytes; a crash after acknowledgement but before
transaction B reruns only B; a crash after B returns `AlreadyTerminalized` only
after byte-for-byte validation of the complete projection. A partial or
different projection is never idempotent and fails closed. Ordinary pre-start
Reserved expiry uses the same appended-row-gated transaction B with
`Reserved -> RejectedDurable` and the corresponding no-ownership projection.

Candidate query joins, rather than trusts:

```sql
delivery_decisions.current_disposition_identity
= retry_authorization_bindings.rejection_disposition_identity
AND retry_authorization_bindings.binding_state='Active'
AND retry_authorization_bindings.authorization_identity
    = retry_authorizations.authorization_identity
AND retry_authorization_bindings.rejection_disposition_identity
    = retry_authorizations.rejection_disposition_identity
AND retry_authorizations.append_state='Appended'
AND retry_authorizations.apply_state='Applied'
AND EXISTS (
  SELECT 1 FROM retry_authorization_events event
  WHERE event.authorization_identity=retry_authorizations.authorization_identity
    AND event.event_kind='Applied'
    AND event.to_apply_state='Applied'
    AND event.target_disposition_identity
        = retry_authorization_bindings.rejection_disposition_identity
    AND event.append_state='Appended'
    AND event.immutable_audit_ref IS NOT NULL
)
```

The same join also requires exact persisted producer provenance:

```sql
AND delivery_decisions.push_kind='ReviewProviderTopN'
AND counted_producer_attestations.decision_identity
    = delivery_decisions.decision_identity
AND counted_producer_attestations.push_kind
    = retry_authorizations.push_kind
AND counted_producer_attestations.producer_seam
    = retry_authorizations.producer_seam
AND counted_producer_attestations.producer_catalog_identity_sha256
    = retry_authorizations.producer_catalog_identity_sha256
AND counted_producer_attestations.producer_attestation_sha256
    = retry_authorizations.producer_attestation_sha256
AND counted_producer_attestations.push_kind='ReviewProviderTopN'
AND counted_producer_attestations.producer_seam
    ='push_templates::dispatch_r09_provider_top_n_outcome'
AND counted_producer_attestations.producer_catalog_identity_sha256
    = :current_validated_catalog_identity_sha256
AND counted_producer_attestations.producer_attestation_sha256
    = sha256_hex(counted_producer_attestations.attestation_canonical)
```

Automatic and manual authorization use this join before row insert; recovery,
candidate materialization, admission and pre-sink claim repeat it. Missing v5
companion rows, any of the fourteen disabled kinds or any provenance mismatch
returns typed `RetryProducerNotEnabled` and cannot append an authorization.

When a new disposition, accepted/manual terminal state, or
`UncertainTaskTransitionPending -> UncertainManualReview` transition is
committed, the same `BEGIN IMMEDIATE` changes the active binding to `Cleared`,
stores the typed clear reason/time, and clears the compatibility boolean. A
committed `UncertainManualReview` row with an `Active` binding is a
post-validation failure. The transaction never changes or deletes the applied
authorization, event, cleared binding or any immutable attempt binding. This
same transition function is used by same-cycle, `JoinError` and prior-boot
quarantine.

- [ ] **Step 5: Run recovery tests and commit**

```bash
cargo test --lib durable_delivery::tests::br192_authorization_ -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_identical_authorization_replay_returns_stored_record -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_different_authorization_bytes_for_same_rejection_fail_closed -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_stale_authorization_is_invalidated_not_retargeted -- --exact --test-threads=1
git add src/durable_delivery/coordinator.rs src/durable_delivery/tests.rs
git commit -m "feat: recover frozen BR-192 retry authorization"
```

Expected: every command reports a non-zero test count and passes.

### Task 4: Implement typed admission and durable cycle audit

**Files:**

- Modify `src/durable_delivery/coordinator.rs`
- Modify `src/durable_delivery/tests.rs`

- [ ] **Step 1: Write RED tests for every result**

Add:

```rust
#[test]
fn br192_every_retry_deferral_has_one_cycle_bound_immutable_audit() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_every_retry_ineligibility_has_one_cycle_bound_immutable_audit() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_missing_decision_returns_typed_error_before_admission_result_or_audit() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_candidate_query_never_returns_a_missing_decision_identity() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_reserved_selector_returns_all_257_rows_in_total_order_without_truncation() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_prior_boot_selector_returns_all_257_rows_before_new_cycle() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_candidate_selector_returns_all_257_rows_in_one_frozen_snapshot() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_primary_selector_source_rejects_limit_offset_cursor_and_caller_cardinality() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_admission_revalidates_envelope_hash_and_all_frozen_policy_fields() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_backoff_is_persisted_and_attempt_cap_exhausts_once() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_uncertain_states_never_enter_candidates_or_sink_path() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_entering_uncertain_manual_review_clears_active_binding_and_preserves_history() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_admission_atomically_installs_attempt_binding_schedule_and_reserved() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_prepare_uses_existing_binding_and_never_creates_generation_or_attempt() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_pending_sink_attempt_started_event_never_calls_sink() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_unavailable_sink_attempt_started_returns_typed_error_and_failed_cycle_audit() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_start_audit_unavailable_prestart_reason_matrix_preserves_rows_hashes_and_zero_sink() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_start_audit_unavailable_post_ack_or_consumed_rejects_exception_and_quarantines() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_appended_sink_attempt_started_validator_rejects_each_identity_hash_ref_generation_and_fence_mismatch() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_sink_attempt_started_time_equals_prepared_canonical_and_persisted_outbox_time() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_crash_after_admission_before_prepare_recovers_same_binding_without_new_generation_or_sink_duplication() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_crash_after_pre_call_cas_before_sink_is_quarantined_without_resend() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_remote_accept_then_crash_before_result_is_quarantined_without_resend() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_same_process_repeated_execute_consumes_one_permit_and_calls_sink_once() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_record_sink_result_still_requires_attempt_in_flight() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_persisted_retry_sink_outcome_joins_terminal_result_and_ownership() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_ownership_pointer_update_then_result_insert_failure_rolls_back_and_quarantines_without_resend() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_rollback_preserves_four_stage_retry_origin_reserved_recovery() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_cycle_begin_rejects_empty_and_ascii_whitespace_boot_identity_before_insert() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_cycle_begin_public_signature_uses_only_durable_result_error_channel() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_cycle_begin_persists_exact_explicit_boot_identity() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_cycle_begin_derives_identity_before_running_check_and_binds_no_commit_proof() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_no_commit_branch_queries_zero_proposed_cycle_and_started_before_rollback() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_consume_no_commit_proof_rederives_next_identity_and_rejects_concurrent_change() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_no_commit_proof_rejects_domain_schema_field_order_encoding_and_witness_mutations() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_empty_running_check_atomically_persists_exact_ordinal_identity_and_started() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_cycle_ordinal_exhaustion_returns_exact_typed_variant_without_write() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_cycle_audit_reconcile_bound_returns_exact_typed_variant() { panic!("BR-192 RED: named contract is not implemented"); }
```

The uncertainty test iterates all three states explicitly. The admission
deferral test covers only reachable budget, business-date claim, reserved head,
uncertain head, cooldown and retry backoff. Authorization pending is covered by
Task 3 reconciliation tests and must never reach admission. The ineligibility
test enumerates the complete `RetryIneligibility` enum, which intentionally has
no `DecisionNotFound` variant. A direct/stale missing identity must return
`DurableDeliveryError::DecisionNotFound` before opening the admission
transaction or creating an `AdmissionResult`/cycle audit:
`AdmissionResult.decision_identity` is a non-null foreign key and therefore
cannot represent absence. Candidate discovery uses an inner join to
`delivery_decisions`, so its result set can never contain a missing decision
identity.

The three selector fixtures each insert 257 qualifying rows whose leading
order keys collide and whose final identity tie-break reverses insertion order.
They assert one call returns all 257 rows, the exact frozen count/hash is stable
across insertion orders, every row is consumed once and no production selector
surface accepts a limit, offset, cursor, page size or caller cardinality.
Prior-boot recovery consumes its entire vector before a new-cycle insert;
Reserved work consumes its entire vector before the one retry-candidate
snapshot. Source inspection of the three selector bodies rejects `LIMIT`,
`OFFSET` and keyset/caller-continuation branches.

The pre-start reason-matrix test iterates all three closed reason codes and
snapshots the full decision, attempt, active/historical binding and schedule
rows, their canonical hashes, retry ordinal/generation and pending start bytes.
After the narrow finalizer reaches `Failed/Terminalized`, every snapshot is
byte-identical, the only new records are the failure payload/terminal audit and
there are zero sink calls. The post-boundary test has separate acknowledged-
start and consumed-ownership fixtures for all three reason codes. It proves the
unchanged-state branch is rejected from persisted state, ordinary quarantine
advances retained ownership to `InterruptedUncertain`, all uncertainty is
append/acknowledged before `FailurePending`, and recovery makes zero sink calls.

- [ ] **Step 2: Prove RED**

```bash
cargo test --lib durable_delivery::tests::br192_every_retry_deferral_has_one_cycle_bound_immutable_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_every_retry_ineligibility_has_one_cycle_bound_immutable_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_missing_decision_returns_typed_error_before_admission_result_or_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_candidate_query_never_returns_a_missing_decision_identity -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_reserved_selector_returns_all_257_rows_in_total_order_without_truncation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_prior_boot_selector_returns_all_257_rows_before_new_cycle -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_candidate_selector_returns_all_257_rows_in_one_frozen_snapshot -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_primary_selector_source_rejects_limit_offset_cursor_and_caller_cardinality -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_admission_revalidates_envelope_hash_and_all_frozen_policy_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_backoff_is_persisted_and_attempt_cap_exhausts_once -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_uncertain_states_never_enter_candidates_or_sink_path -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_entering_uncertain_manual_review_clears_active_binding_and_preserves_history -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_admission_atomically_installs_attempt_binding_schedule_and_reserved -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_prepare_uses_existing_binding_and_never_creates_generation_or_attempt -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pending_sink_attempt_started_event_never_calls_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_unavailable_sink_attempt_started_returns_typed_error_and_failed_cycle_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_start_audit_unavailable_prestart_reason_matrix_preserves_rows_hashes_and_zero_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_start_audit_unavailable_post_ack_or_consumed_rejects_exception_and_quarantines -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_appended_sink_attempt_started_validator_rejects_each_identity_hash_ref_generation_and_fence_mismatch -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_sink_attempt_started_time_equals_prepared_canonical_and_persisted_outbox_time -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_crash_after_admission_before_prepare_recovers_same_binding_without_new_generation_or_sink_duplication -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_crash_after_pre_call_cas_before_sink_is_quarantined_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_remote_accept_then_crash_before_result_is_quarantined_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_same_process_repeated_execute_consumes_one_permit_and_calls_sink_once -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_record_sink_result_still_requires_attempt_in_flight -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_persisted_retry_sink_outcome_joins_terminal_result_and_ownership -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_ownership_pointer_update_then_result_insert_failure_rolls_back_and_quarantines_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rollback_preserves_four_stage_retry_origin_reserved_recovery -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_rejects_empty_and_ascii_whitespace_boot_identity_before_insert -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_public_signature_uses_only_durable_result_error_channel -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_persists_exact_explicit_boot_identity -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_derives_identity_before_running_check_and_binds_no_commit_proof -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_no_commit_branch_queries_zero_proposed_cycle_and_started_before_rollback -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_consume_no_commit_proof_rederives_next_identity_and_rejects_concurrent_change -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_no_commit_proof_rejects_domain_schema_field_order_encoding_and_witness_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_empty_running_check_atomically_persists_exact_ordinal_identity_and_started -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_ordinal_exhaustion_returns_exact_typed_variant_without_write -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_audit_reconcile_bound_returns_exact_typed_variant -- --exact --test-threads=1
```

- [ ] **Step 3: Implement cycle lifecycle**

Add coordinator methods:

The timestamp parameters shown on these cycle-lifecycle methods are audit
ordering metadata only. They must not be reused for schedule eligibility,
source-date expiry, manual authorization, candidate/start admission,
ownership claim or pre-call freshness; every such decision reads the private
`ProductionFreshnessClock` through the no-caller-time APIs defined above.

```rust
pub fn begin_retry_cycle_before_spawn(
    &self,
    namespace_sha256: &str,
    owner_boot_identity: &str,
    scheduled_for: DateTime<Utc>,
    now: DateTime<Utc>,
) -> durable_delivery::Result<RetryCycleBeginOutcome>;

pub fn consume_no_retry_cycle_committed(
    &self,
    proof: NoRetryCycleCommitted,
) -> Result<()>;

pub fn prepare_retry_cycle_completed(
    &self,
    cycle_identity: &str,
    evidence: &RetryCycleEvidence,
    now: DateTime<Utc>,
) -> Result<()>;

pub fn terminalize_retry_cycle_completed(
    &self,
    cycle_identity: &str,
    now: DateTime<Utc>,
) -> Result<RetryCycleEvidence>;

pub fn prepare_retry_cycle_failed(
    &self,
    cycle_identity: &str,
    failure: &RetryCycleFailure,
    now: DateTime<Utc>,
) -> Result<()>;

pub fn terminalize_retry_cycle_failed(
    &self,
    cycle_identity: &str,
    now: DateTime<Utc>,
) -> Result<RetryCycleEvidence>;

pub fn resume_retry_cycle_terminal_slot(
    &self,
    append: &dyn ImmutableAppendPort,
    cycle_identity: &str,
    now: DateTime<Utc>,
) -> Result<RetryCycleEvidence>;

pub fn resume_same_boot_retry_cycle_terminal_slots(
    &self,
    append: &dyn ImmutableAppendPort,
    current_owner_boot_identity: &str,
    now: DateTime<Utc>,
) -> Result<Vec<RetryCycleEvidence>>;

pub fn quarantine_same_cycle_attempts_before_failure(
    &self,
    cycle_identity: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>>;

pub fn recover_prior_boot_running_cycles(
    &self,
    current_owner_boot_identity: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>>;

pub fn prepare_prior_boot_retry_cycle_failed(
    &self,
    quarantined_cycle_identity: &str,
    now: DateTime<Utc>,
) -> Result<String>;

pub fn reconcile_one_retry_cycle_audit(
    &self,
    append: &dyn ImmutableAppendPort,
    now: DateTime<Utc>,
) -> Result<bool>;
```

`reconcile_one_retry_cycle_audit` selects exactly one Pending row with:

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

It rejects null, empty or ASCII-whitespace-only `created_at`,
`cycle_identity`, `cycle_event_identity` or `event_kind`; a non-null
`decision_identity` must be non-empty and ASCII-whitespace-stable. The explicit
case is the only allowed null order and puts cycle scope before decision scope;
`cycle_event_identity` is the final total tie-break. Each call acknowledges at
most one row. A caller allows exactly
`MAX_CYCLE_AUDIT_RECONCILE_STEPS_PER_RUN=4096` successful acknowledgements,
then performs one non-mutating pending-row check. A remaining row returns typed
`DurableDeliveryError::RetryCycleAuditReconciliationBoundExceeded {
max_steps: 4096, pending_cycle_event_identity }`, where the identity is the
exact immutable identity selected by that check; it retains bytes/phase,
creates no opposite terminal slot, makes zero sink calls and keeps the guard
latched. The exact test must pattern match the public variant and both field
values, not a message or generic error.

`begin_retry_cycle_before_spawn` validates `owner_boot_identity` before starting
its transaction and never derives it from coordinator state, a global, an
environment variable or a process helper. Missing identity is rejected by
pre-open boot-authority construction; empty or ASCII-whitespace-only values are
rejected defensively by this method before any cycle/outbox insert. In one
`BEGIN IMMEDIATE`, it computes the next retained ordinal and the exact proposed
identity from the frozen input tuple before the global Running query. If a
Running witness exists, the same transaction queries zero proposed cycle rows
and zero logical `Started` rows, constructs the exact opaque, non-cloneable
`NoRetryCycleCommitted`, performs no write and rolls back. The fresh
`consume_no_retry_cycle_committed` transaction must rederive the same next
ordinal/identity, byte-match the witness and reprove both zero counts; any
concurrent insert or witness change rejects consumption and leaves the guard
latched. If no Running witness exists, the begin transaction inserts the exact
ordinal, identity and input boot/timestamps in `Running` together with the
logical `Started` slot atomically. A commit-ambiguous error returns `Err`
without proof.
`prepare_retry_cycle_completed` freezes the exact canonical Completed bytes,
inserts its sole Pending terminal slot and changes only
`Running/NotPrepared -> Running/CompletionPending`.
`reconcile_one_retry_cycle_audit` acknowledges exact completion bytes and
changes only `CompletionPending -> CompletionAppended`;
`terminalize_retry_cycle_completed` performs only
`CompletionAppended -> Completed/Terminalized`.
Before `prepare_retry_cycle_failed` accepts failure
preparation, it rechecks that
`quarantine_same_cycle_attempts_before_failure` has selected every
attempt whose `execution_cycle_identity` equals this cycle, whose start is
appended/acknowledged or whose send ownership is consumed/`Started` or whose
decision is `AttemptInFlight`, and which has no terminal authoritative sink
result. The quarantine transaction persists
`ProcessInterruptedAfterSinkStart`, creates or advances retained ownership to
`InterruptedUncertain` and creates all exact uncertainty outboxes. The runtime
must append/acknowledge those outboxes to a fixed point before
`prepare_retry_cycle_failed` may insert the unique `Pending` `Failed` slot and
move the cycle only to `Running/FailurePending`. That transaction also inserts
the immutable complete typed-preimage/envelope payload row and binds its
identity plus both hashes to the cycle and Failed outbox; the precondition is
rechecked
in the preparation transaction. `reconcile_one_retry_cycle_audit` advances
that exact event to `Appended` and the cycle to
`Running/FailureAppended` in one acknowledgement transaction.
`terminalize_retry_cycle_failed` then rechecks the immutable ref, canonical
outbox bytes, privately decoded canonical typed-preimage/envelope bytes, both
recomputed hashes and uncertainty fixed point before the sole
`Running/FailureAppended -> Failed/Terminalized` CAS. None of these methods has
a sink capability.
The unique terminal outbox slot plus `terminal_phase` is irreversible. Once
`CompletionPending|CompletionAppended` exists, every normal error, caught
panic, JoinError and boot recovery invokes
`resume_retry_cycle_terminal_slot`, which loads and resumes exact completion
bytes and cannot prepare Failed. The same rule symmetrically prevents
Completed after a failure slot. Only `NotPrepared` with no terminal slot may
prepare `CycleOperationFailed`, `Panic`, `JoinError` or
`ProcessInterrupted`.
`recover_prior_boot_running_cycles` also validates its explicit identity before
its transaction. Its update predicate is exactly
`state='Running' AND owner_boot_identity<>current_owner_boot_identity`; it never
touches a same-boot row and never looks up the current identity itself. It
first returns recoverable terminal slots in the exact priority
`CompletionAppended, CompletionPending, FailureAppended, FailurePending` for
`resume_retry_cycle_terminal_slot`; only `NotPrepared` enters quarantine.
Within the same `BEGIN IMMEDIATE`, for every selected `NotPrepared` prior-boot cycle it first finds
every nonterminal retry attempt with an appended/acknowledged
`SinkAttemptStarted` or `retry_send_ownership.send_consumed=true`/
`AttemptInFlight` and no terminal authoritative sink result. It persists
`ProcessInterruptedAfterSinkStart`, creates a consumed
`InterruptedUncertain` ownership when an appended start had none or advances
existing ownership to `InterruptedUncertain`, and creates the exact
`UncertainAuditPending -> UncertainTaskTransitionPending ->
UncertainManualReview` recovery outboxes without calling the sink. This first
method then returns stable quarantined cycle identities; it is forbidden from
creating `ProcessInterrupted`, a failure payload, `OrphanRecovered` or
`Failed`.

`resume_same_boot_retry_cycle_terminal_slots` is the complementary,
non-orphan path. It validates the explicit current identity, then materializes
the complete validated snapshot selected by:

```sql
WHERE state='Running'
  AND owner_boot_identity=:current_owner_boot_identity
  AND terminal_phase IN (
    'CompletionAppended','CompletionPending',
    'FailureAppended','FailurePending'
  )
ORDER BY CASE terminal_phase
           WHEN 'CompletionAppended' THEN 0
           WHEN 'CompletionPending' THEN 1
           WHEN 'FailureAppended' THEN 2
           WHEN 'FailurePending' THEN 3
         END ASC,
         scheduled_for COLLATE BINARY ASC,
         started_at COLLATE BINARY ASC,
         cycle_identity COLLATE BINARY ASC
```

It rejects null, empty or ASCII-whitespace-only identity/order keys and invokes
only `resume_retry_cycle_terminal_slot` for every selected row. It cannot
quarantine, prepare a new terminal kind or receive a sink/provider/renderer.
It repeats the full selector until empty so a Pending acknowledgement that
becomes Appended is terminalized in the same pre-begin recovery. A failure
returns the original typed `DurableDeliveryError`, with zero new cycle,
`Started` or sink call. Same-boot `NotPrepared` is deliberately not selected.

`begin_retry_cycle_before_spawn` performs a second, transaction-authoritative
exclusion. Its `BEGIN IMMEDIATE` first reads the retained maximum
`cycle_ordinal`, checked-adds one and derives the exact proposed identity from
the frozen namespace/owner/schedule/start/ordinal tuple. It then selects any
`state='Running'` row by
`started_at COLLATE BINARY ASC, cycle_identity COLLATE BINARY ASC`. A row
causes the transaction to query and require zero proposed cycle rows and zero
proposed logical `Started` rows, construct the exact witness-bound
`NoRetryCycleCommitted`, perform no write and roll back. The returned
`RetryCycleAlreadyRunning` is definite only with that proof. A fresh
`consume_no_retry_cycle_committed` transaction must rederive the same next
ordinal/identity, byte-match the selected retained Running row and reprove
both zero counts. This global check also blocks a same-boot `NotPrepared`, an
incompletely recovered prior-boot row and an identity-mismatched safe-terminal
row. Only an empty Running check permits the atomic insert of that exact
ordinal/identity as Running plus its logical `Started` event.

Runtime must append/acknowledge every phase-one outbox to a fixed point. Only
then may
`prepare_prior_boot_retry_cycle_failed` recheck that fixed point in a new
`BEGIN IMMEDIATE`, persist the complete canonical `ProcessInterrupted`
typed-preimage/envelope payload, and prepare `OrphanRecovered` plus the unique
Pending `Failed` slot. Runtime appends/acks phase two and terminalizes before a
new cycle. A phase-one append/ack failure makes zero phase-two writes. A
prior-boot cycle already in
`CompletionPending`, `CompletionAppended`, `FailurePending` or
`FailureAppended` reuses its exact prepared bytes and kind/reason rather than
replacing them with `ProcessInterrupted`; fresh-process recovery
loads/decodes/re-hashes only the stored payload/outbox and never reconstructs a
terminal payload from display text or process memory. A prior-boot
admission that never appended its start remains an ordinary `Reserved`
recovery; a prior-boot appended start is never restored to `Reserved`.
`prepare_retry_cycle_completed` rejects evidence containing
`final_failure=Some`. Failure preparation and terminalization both validate the
closed failure reason and privately decode/re-serialize the reason-specific
typed preimage and complete envelope. They persist the exact payload identity,
stable reason token and both hashes in `retry_cycles`, the immutable payload
row and canonical `Failed` event, and terminalization returns
evidence with that same `final_failure=Some`. Neither accepts a display string
or caller-supplied bare digest.

`append_exact` uses record kind `RetryCycleAudit`. Every event identity is
domain-separated only by its logical slot: cycle identity, cycle/decision
scope, event kind and event ordinal. Payload bytes and hashes are immutable
content, never identity inputs. A byte-identical replay returns the retained
row; a different payload/hash for the same slot fails the cycle closed.

- [ ] **Step 4: Replace boolean reacquisition**

Delete `reacquire_rejected`. Add:

```rust
pub fn admit_authorized_retry(
    &self,
    cycle_identity: &str,
    decision_identity: &str,
) -> Result<RetryAdmission>;
```

Within one `BEGIN IMMEDIATE`, perform every design check. Insert one
`AdmissionResult` event for all reachable return variants. For `Reacquired`,
that same transaction:

1. selects the next retained reservation generation, deterministic attempt
   identity, next retry ordinal, owner identity and fence token;
2. inserts the authoritative delivery attempt;
3. inserts the immutable `retry_attempt_bindings` row containing exact
   decision/cycle/authorization/disposition/binding generation/reservation
   generation/owner/fence values;
4. increments the retained schedule, sets its exact attempt-binding FK and
   computes the persisted backoff;
5. inserts the state-transition audit and changes the decision to `Reserved`;
   and
6. commits only after operation post-validation proves the attempt, binding,
   schedule, state and admission event form one relation.

There must be no committed retry-origin `Reserved` state without the binding
and schedule increment, and no committed attempt/binding/schedule increment
without the matching `Reserved` transition. `RetryAdmission::Reacquired`
returns all selected identities, generations, ordinal, owner and fence.
Authority is loaded through the unique active companion binding; pending
authorization rows are absent from candidate SQL.

The frozen policy comparison includes:

```text
policy_version
push_kind
sub_kind
cooldown_scope
window_mode
counts_against_daily_budget
effective cooldown seconds
scope key validity
```

When the stored attempt count is already three, the same admission transaction
sets `retry_schedules.exhausted_at`, writes the one `AdmissionResult` whose
typed payload is `NoLongerEligible::RetryAttemptsExhausted`, and returns
ineligible. Candidate queries exclude non-null `exhausted_at`, so later cycles
do not emit the same governance decision repeatedly.

- [ ] **Step 5: Split prepare, append/ack, claim and execute sink seams**

Add explicit types and methods:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedRetryAttempt {
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
    // frozen envelope/sink binding fields
}

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

pub struct SinkExecutionPermit {
    // private fields: exact prepared/start/ownership identities and positive
    // i64 fence; intentionally !Clone, !Serialize and consumed by execute.
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

impl DurableDeliveryCoordinator {
    pub fn prepare_retry_attempt(
        &self,
        execution_cycle_identity: &str,
        attempt_identity: &str,
    ) -> Result<RetryAttemptPreparationOutcome>;
    pub fn reconcile_prepared_retry_attempt_audit(
        &self,
        prepared: &PreparedRetryAttempt,
        append: &dyn ImmutableAppendPort,
    ) -> Result<AppendedSinkAttemptStarted>;
    pub fn validate_appended_sink_attempt_started(
        &self,
        prepared: &PreparedRetryAttempt,
        appended: &AppendedSinkAttemptStarted,
    ) -> Result<()>;
    pub fn claim_retry_sink_execution(
        &self,
        prepared: &PreparedRetryAttempt,
        appended_start: &AppendedSinkAttemptStarted,
    ) -> Result<RetrySinkClaimOutcome>;
    pub fn execute_prepared_retry_sink(
        &self,
        permit: SinkExecutionPermit,
        sink: &dyn AuthoritativeSinkPort,
    ) -> Result<RetrySinkExecutionOutcome>;
}
```

All five operations explicitly use the same coordinator capability. They are
methods, not global-opening free functions; runtime invokes them through its
`Arc<DurableDeliveryCoordinator>`.

All freshness-bearing methods, including candidate/expiry selection,
admission, start preparation, ownership claim and final pre-sink execution,
read the coordinator's private `ProductionFreshnessClock`; their production
surface accepts no caller timestamp. The production constructor installs only
the real system clock and exposes no setter. A deterministic clock constructor
exists only under cfg(test), requires an invocation-unique TEST_CODE namespace,
and cannot open production roots.

The claim transaction also changes cycle sink-call evidence from `NotStarted`
or `Confirmed(n)` to `Indeterminate/NULL` before returning the consumed
permit. `record_sink_result` atomically commits the authoritative result,
writes its exact `terminal_sink_result_identity`, advances ownership to
`TerminalRecorded`, proves the cycle-wide result/terminal-ownership join is
bijective, proves no remaining nonterminal consumed ownership, counts that
exact joined set and changes
`Indeterminate/NULL -> Confirmed(n)`. A later claim may move
`Confirmed(n) -> Indeterminate/NULL` again. A clean terminal path with no claim
changes `NotStarted/NULL -> Confirmed/0`; any panic/crash/JoinError after claim
leaves `Indeterminate/NULL`, even though the recovery helper itself proves it
made zero new sink calls.

`event_canonical` is the canonical UTF-8 JSON encoding of the deny-unknown
`SinkAttemptStartedCanonicalV1` ordered fields:
`schema_version=1`, `rule_id="BR-192"`,
`event_kind="SinkAttemptStarted"`, `cycle_event_identity`,
`cycle_identity`, `admission_cycle_identity`, `attempt_identity`,
`decision_identity`,
`authorization_identity`, `rejection_disposition_identity`,
`authorization_binding_identity`, `retry_ordinal`, `binding_generation`,
`reservation_generation`, `owner_instance_identity`, `fence_token`,
`started_at`. Derive `cycle_event_identity` as lowercase hex SHA-256 over
`b"stock_analysis.durable_delivery.br192.retry_cycle_event.v1\0"` followed by
canonical UTF-8 JSON for the logical slot
`cycle_identity,decision_identity,"SinkAttemptStarted",reservation_generation`.

`DurableDeliveryCoordinator::prepare_retry_attempt` must never call
`begin_attempt`, create an attempt or attempt binding, select a generation/
ordinal/owner/fence, or update the schedule. In one transaction it loads the
immutable binding installed by admission, revalidates every field against the
authoritative attempt, binding's admission cycle, already-`Reserved` decision
and schedule FK, validates the provided execution cycle is `Running`, and reads
the private clock. If expired before any current-attempt start/ownership, it
writes the exact Pending expiry authority and returns
`RetryAttemptPreparationOutcome::Expiry(ExpiryPrepared(...))`. If a current
Pending/Appended start or ordinary ownership already exists, it writes no
expiry row and returns `Expiry(StartAuthorityWins(...))`. While fresh it
idempotently inserts that execution cycle's exact Pending
`SinkAttemptStarted` logical-slot outbox row and returns
`Prepared(PreparedRetryAttempt)`, carrying both execution and admission cycle
identities plus the persisted outbox `created_at` as `started_at`.

It has no sink capability.
`DurableDeliveryCoordinator::reconcile_prepared_retry_attempt_audit` has only
coordinator/append capabilities and must append and acknowledge the exact start
row. `DurableDeliveryCoordinator::validate_appended_sink_attempt_started`
requires every identity,
generation, ordinal, owner and fence to equal the prepared token/binding,
requires `admission_cycle_identity` to equal the immutable binding's
`cycle_identity` and `cycle_identity` to equal the current execution cycle,
requires positive ordinal/generations/fence, derives the exact domain-separated
logical-slot identity from cycle/decision/`SinkAttemptStarted`/reservation
generation, recomputes lowercase 64-hex SHA-256 over canonical bytes,
deserializes those bytes with `deny_unknown_fields`, and validates a
non-ASCII-whitespace immutable ref. DTO `started_at`, canonical `started_at`,
persisted outbox `created_at` and `PreparedRetryAttempt.started_at` must be
exactly equal after canonical timestamp normalization. No append-authority
timestamp is accepted or persisted.

`DurableDeliveryCoordinator::claim_retry_sink_execution` has no sink
capability. In one `BEGIN IMMEDIATE` it reruns the validator/read-back and reads
the private clock. At expiry, the already-Pending/Appended start wins and the
method returns `RetrySinkClaimOutcome::Expiry(StartAuthorityWins(...))`; it
writes no expiry row, mints no permit and routes the retained attempt through
uncertainty. While fresh, it CASes the exact current attempt from
`Reserved -> AttemptInFlight`, and inserts
retained `retry_send_ownership` with exact binding/execution-cycle/generation/
owner/positive-i64 fence/start time and `send_consumed=true`. Only the CAS
winner receives `RetrySinkClaimOutcome::Claimed` with the non-clone,
non-serializable permit.
`DurableDeliveryCoordinator::execute_prepared_retry_sink` is the only seam
with a sink capability, consumes that permit by value and revalidates
`AttemptInFlight` plus exact `Started` ownership. With no intervening await,
provider, renderer, filesystem, database transaction or other blocking work,
it reads the private production clock at the external-call linearization point.
If the observation is at/after expiry, it performs zero external calls and in
one `BEGIN IMMEDIATE` revalidates the single-use permit, stores the observation,
inserts the immutable deferred pre-call expiry companion, advances ownership
to `FreshnessExpiredBeforeExternalCall`, and inserts the matching Pending
`ReservedExpiredBeforeSink` expiry row, in that order. It then requires exactly
one result satisfying `authoritative_for_state=1 AND late_after_fence=0` for
each `TerminalRecorded` ownership and
zero result for every freshness-expired, `Started` or `InterruptedUncertain`
ownership. Every authoritative sink result for the cycle must join exactly one
same-cycle `TerminalRecorded` ownership. It recounts that bijective durable
result set as `n`: if another same-cycle
`Started|InterruptedUncertain` remains, it checked-updates
`Indeterminate/NULL` to itself; otherwise it checked-CASes
`Indeterminate/NULL -> Confirmed(n)` before commit. Missing/double results, an
orphan/extra result, any non-bijective join, an ambiguous ownership with a
result, unexpected state or a selected-cycle update count other than one rolls
back the complete transaction. Transaction B
revalidates the same branch and may terminalize this definite no-call expiry
while another ownership keeps cycle evidence Indeterminate. It returns only
`RetrySinkExecutionOutcome::ExpiredBeforeExternalCall(prepared)` for ordinary
append/ack and the separate effective-terminal transaction B described in Task
3. That exact ownership state/attempt/observation is the only ownership
exception accepted by the expiry-insert trigger.

If still fresh, it immediately performs one external call, retains the exact
pre-call observation in memory and invokes `self.record_sink_result`. That
transaction continues to require `decision=AttemptInFlight` and derives the
prospective authoritative/non-late result identity from the canonical result.
It first persists the pre-call observation and prospective result identity in
ownership while advancing `Started -> TerminalRecorded`, then inserts the
matching authoritative/non-late result, validates the cycle-wide
result/ownership bijection, changes sink-call evidence, and commits. The
deferred FK permits the temporary pointer-first state only inside the
transaction; the reverse trigger rejects a result-first insert immediately. A
crash before commit leaves no definite freshness/no-call claim and is
quarantined as `InterruptedUncertain`. The method returns only
`RetrySinkExecutionOutcome::Persisted(PersistedRetrySinkOutcome)` with the
stored result identity, persisted decision/ownership states and pre-call
observation; it never returns a bare `AuthoritativeSinkResult`.

Inject a transaction failure after the ownership pointer/state update but
before the authoritative/non-late result insert. The ownership transition,
result insert and decision transition must all roll back, leaving no result
row, decision `AttemptInFlight` and ownership `Started` with a NULL terminal-
result pointer. Startup/cycle recovery then conservatively records
`ProcessInterruptedAfterSinkStart`, converges through manual review and makes
zero sink calls. The same fixture without the failpoint must return an exact
`RetrySinkExecutionOutcome::Persisted(PersistedRetrySinkOutcome)` that joins the result row, decision terminal state
and `TerminalRecorded` ownership. Pending/missing/mismatched start evidence, a
lost CAS, repeated execute or ownership mismatch returns a typed failure with
zero sink calls. The repeated-execute regression does not forge or clone a
permit: it proves `SinkExecutionPermit: !Clone + !Serialize`, calls the claim
twice in one process, observes only one permit, consumes it once, and asserts
the second claim/execute path makes zero additional external calls.

A crash after admission commit but before prepare is an ordinary recoverable
`Reserved` retry: the next boot terminalizes the prior-boot cycle, then the new
startup cycle loads the same attempt/binding, preserves its admission-cycle
identity, prepares/appends the new execution cycle's stable logical-slot event,
claims ownership and executes at most once. It never appends to the terminal
prior cycle. The recovery path is forbidden from allocating another reservation
generation, attempt identity, retry ordinal or schedule increment.

By contrast, any prior-boot appended start row or `AttemptInFlight` without a
terminal authoritative result is indeterminate. Recovery must make zero sink
calls, persist `ProcessInterruptedAfterSinkStart`, change ownership to
`InterruptedUncertain`, and drive
`UncertainAuditPending -> UncertainTaskTransitionPending ->
UncertainManualReview`. A crash after the claim but before the actual call is
also conservatively uncertain because the sink has no queryable idempotency
key.

- [ ] **Step 6: Prohibit monolithic retry resume**

Ordinary non-retry Reserved recovery may continue using its existing path.
Retry-origin reservations must not call a convenience `resume_deliverable`
that internally begins and executes. A static dependency test requires the
runtime sequence:

```text
coordinator.prepare_retry_attempt
  Prepared -> coordinator.reconcile_prepared_retry_attempt_audit
           -> coordinator.claim_retry_sink_execution
              Claimed -> coordinator.execute_prepared_retry_sink
              Expiry(StartAuthorityWins) -> reconcile uncertainty; zero permit/sink
  Expiry(ExpiryPrepared) -> coordinator.reconcile_retry_expiry_preparation
  Expiry(StartAuthorityWins) -> coordinator.reconcile_retry_expiry_preparation
```

and rejects a direct retry-origin `resume_deliverable` call.

The same classifier remains mandatory in a forward-compatible rollback build:
disabling the periodic discovery loop does not authorize legacy resume for a
retry-origin `Reserved` row. Such a row either completes the same four-stage
path above or remains untouched and fail-closed when that path is unavailable.
Add exact tests
`br192_rollback_preserves_four_stage_retry_origin_reserved_recovery` and
`br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable`.

- [ ] **Step 7: Run typed admission tests and commit**

```bash
cargo test --lib durable_delivery::tests::br192_every_retry_deferral_has_one_cycle_bound_immutable_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_every_retry_ineligibility_has_one_cycle_bound_immutable_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_missing_decision_returns_typed_error_before_admission_result_or_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_candidate_query_never_returns_a_missing_decision_identity -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_admission_revalidates_envelope_hash_and_all_frozen_policy_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_backoff_is_persisted_and_attempt_cap_exhausts_once -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_uncertain_states_never_enter_candidates_or_sink_path -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_admission_atomically_installs_attempt_binding_schedule_and_reserved -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_prepare_uses_existing_binding_and_never_creates_generation_or_attempt -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pending_sink_attempt_started_event_never_calls_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_unavailable_sink_attempt_started_returns_typed_error_and_failed_cycle_audit -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_appended_sink_attempt_started_validator_rejects_each_identity_hash_ref_generation_and_fence_mismatch -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_sink_attempt_started_time_equals_prepared_canonical_and_persisted_outbox_time -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_crash_after_admission_before_prepare_recovers_same_binding_without_new_generation_or_sink_duplication -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_crash_after_pre_call_cas_before_sink_is_quarantined_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_remote_accept_then_crash_before_result_is_quarantined_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_same_process_repeated_execute_consumes_one_permit_and_calls_sink_once -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_record_sink_result_still_requires_attempt_in_flight -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_persisted_retry_sink_outcome_joins_terminal_result_and_ownership -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_ownership_pointer_update_then_result_insert_failure_rolls_back_and_quarantines_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rollback_preserves_four_stage_retry_origin_reserved_recovery -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_rejects_empty_and_ascii_whitespace_boot_identity_before_insert -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_public_signature_uses_only_durable_result_error_channel -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_persists_exact_explicit_boot_identity -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_derives_identity_before_running_check_and_binds_no_commit_proof -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_no_commit_branch_queries_zero_proposed_cycle_and_started_before_rollback -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_consume_no_commit_proof_rederives_next_identity_and_rejects_concurrent_change -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_no_commit_proof_rejects_domain_schema_field_order_encoding_and_witness_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_empty_running_check_atomically_persists_exact_ordinal_identity_and_started -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_ordinal_exhaustion_returns_exact_typed_variant_without_write -- --exact --test-threads=1
git add src/durable_delivery/coordinator.rs src/durable_delivery/tests.rs
git commit -m "feat: add audited BR-192 retry admission"
```

### Task 5: Prove cycle-global and cross-process fencing

**Files:**

- Modify `src/durable_delivery/tests.rs`

- [ ] **Step 1: Reuse existing helpers and define only local extensions**

Keep using `Fixture`, `MemoryAppendPort`, `StaticSink`, `prepare_reserved`,
`reconcile_terminal`, `envelope`, `rejection` and `uncertainty`.

Define in `src/durable_delivery/tests.rs`:

```text
RetryTestBuilder
  composes Fixture + existing helpers; creates authorized rejection states

CrossProcessRetryHarness
  owns child env, ready/gate files and locked counter under Fixture root

ProcessCountingSink
  increments a newline counter under fs2 exclusive lock before returning the
  configured StaticSink-equivalent result
```

`CrossProcessRetryHarness` borrows the parent Fixture and never deletes files.
Fixture Drop remains the only cleanup owner.

- [ ] **Step 2: Extend `ProductionStorageSnapshot`**

Capture retained handles/metadata identities or absence for:

```text
data/durable_delivery.sqlite3{,-journal,-wal,-shm}
data/durable_delivery_retry_runner.lock
data/push_log
data/durable_delivery_audit
data/event_audit
data/event_bus
```

Resolve actual production roots through the same manifest-root functions used
by runtime. Do not guess alternate paths. Refuse before opening any protected
production content. Assert identical existence/type/filesystem identity after
cleanup.

Receipts are rows/records in the protected SQLite and durable audit
authorities; do not invent a separate receipt directory.

- [ ] **Step 3: Add cycle-global attempt RED test**

```rust
#[test]
fn br192_reserved_phase_requeries_new_rejection_and_duplicate_suppresses_second_sink() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_test_control_without_attempted_set_still_cannot_bypass_send_claim() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_startup_reserved_reentry_reuses_admission_binding_without_new_generation_or_second_sink() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_startup_quarantines_prior_boot_started_or_inflight_without_terminal_result() { panic!("BR-192 RED: named contract is not implemented"); }
```

The runtime snapshots only Reserved work first. `StaticSink` returns an
authorized definite rejection, then fixed-point reconciliation appends/applies
authorization and candidate SQL runs **after** the Reserved phase. Assert the
candidate query includes the decision, the shared attempted set emits one
independent `DuplicateSuppressed` cycle event and `StaticSink.calls == 1`.

Prevent an empty test with a `#[cfg(test)]`-only
`AttemptedSetMode::Disabled` control. Against the identical fixture it must
admit the post-Reserved candidate through the real prepared sink seam and
still produce `StaticSink.calls == 1`: the second path loses admission or the
pre-call ownership CAS. No production flag disables tracking.

- [ ] **Step 4: Add real two-process test**

Add:

```rust
#[test]
fn br192_two_processes_race_one_retry_with_one_generation_and_one_sink_call() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_two_processes_race_pre_call_cas_with_one_consumed_owner_and_one_sink_call() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
#[ignore = "invoked only by the BR-192 parent process harness"]
fn br192_retry_cross_process_child() { panic!("BR-192 RED: named contract is not implemented"); }
```

Parent launches the current test executable twice with:

```text
--exact
durable_delivery::tests::br192_retry_cross_process_child
--ignored
--nocapture
--test-threads=1
```

Use these exact environment keys and clear any inherited value before setting
them on each child:

```text
TEST_CODE_BR192_RETRY_RACE_CHILD_ROLE       ONE | TWO
TEST_CODE_BR192_RETRY_RACE_DATABASE         shared DB below Fixture root
TEST_CODE_BR192_RETRY_RACE_NAMESPACE        exact nonce-bound TEST_CODE
TEST_CODE_BR192_RETRY_RACE_OWNER            distinct owner per role
TEST_CODE_BR192_RETRY_RACE_READY             role-specific ready file
TEST_CODE_BR192_RETRY_RACE_GATE              shared gate file
TEST_CODE_BR192_RETRY_RACE_COUNTER           shared locked sink counter
```

The ignored child refuses missing/unknown role, a namespace that does not start
with `TEST_CODE_`, any path outside the parent Fixture root, duplicate owners,
or any mismatch between namespace and target root. The parent waits for both
ready files, creates the gate, and retains cleanup ownership.

Children create distinct ready files, wait for the parent gate file, open the
same TEST_CODE database with distinct owners, then race admission and
prepare/append/claim/execute. The counter file is locked with
`fs2::FileExt::lock_exclusive`.

Parent asserts two successful exits, exactly one new generation, exactly one
immutable attempt binding, one schedule increment whose last binding is that
attempt, one committed retry-origin `Reserved` transition, one appended
`SinkAttemptStarted`, exactly one successful `Reserved -> AttemptInFlight` CAS,
exactly one retained `retry_send_ownership` row with `send_consumed=true` and
the exact positive `i64` fence, and one counter line.
Both children must observe that the winning admission committed the whole
attempt/binding/schedule/`Reserved` relation before prepare can start. The
loser creates none of those objects. All paths are below the Fixture root.

The pre-call CAS race test first arranges one admitted `Reserved` attempt and
one appended/acknowledged start, then releases both children against only
`coordinator.claim_retry_sink_execution`. Exactly one child wins the
same-transaction CAS,
persists the consumed ownership and receives the non-cloneable permit; the
other observes `StateChanged` and makes zero calls. This separates admission
fencing from send fencing and proves a shared attempted set is not the
cross-process safety authority.

The startup Reserved-reentry test stops immediately after the winning
admission commit, before prepare/start append. Reopening may reuse that same
attempt/binding/generation/ordinal/schedule and send once through a newly
completed append/claim/execute sequence. By contrast, the startup-quarantine
test stops after an appended start or after the pre-call
`Reserved -> AttemptInFlight` CAS, leaves no terminal authoritative result,
and restarts with a distinct boot identity. Recovery must make zero sink calls,
persist `ProcessInterruptedAfterSinkStart`, and enter
`UncertainAuditPending -> UncertainTaskTransitionPending ->
UncertainManualReview`; it must never reconstruct `Reserved` or issue another
permit.

- [ ] **Step 5: Add two-thread race test**

Use `Fixture::second_coordinator`, `Barrier`, existing `BlockingSink` and the
same database. Assert the loser reports `StateChanged` with generation and zero
sink calls.

- [ ] **Step 6: Run the parent tests**

```bash
cargo test --lib durable_delivery::tests::br192_reserved_phase_requeries_new_rejection_and_duplicate_suppresses_second_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_test_control_without_attempted_set_still_cannot_bypass_send_claim -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_startup_reserved_reentry_reuses_admission_binding_without_new_generation_or_second_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_startup_quarantines_prior_boot_started_or_inflight_without_terminal_result -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_two_processes_race_one_retry_with_one_generation_and_one_sink_call -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_two_processes_race_pre_call_cas_with_one_consumed_owner_and_one_sink_call -- --exact --test-threads=1
```

Expected: each reports `running 1 test`; child processes are visible in parent
test output; all production snapshots remain unchanged.

- [ ] **Step 7: Commit**

```bash
git add src/durable_delivery/tests.rs
git commit -m "test: prove BR-192 retry fencing"
```

### Deferred Task 8 recipe: one cancellation-safe startup/periodic runner

This section freezes the runtime implementation and tests, but is not an
independent landing task. Apply it only during Task 8, in the same compile step
as the sole atomic BR-192 root-export edit and both CLI integrations. Running or
committing this recipe earlier would require a partial public export and is
forbidden.

**Files:**

- Modify `Cargo.toml`
- Modify `src/bin/monitor/durable_delivery_runtime.rs`
- Modify `src/bin/monitor/main.rs`
- Modify `tests/durable_delivery_counted_cutover.rs` created as the mandatory
  first Gate-B file operation in Task 1

- [ ] **Step 1: Enable the Tokio test clock and define runtime test helpers**

Before adding any paused-time test, extend the existing Tokio dependency
feature list with explicit `test-util` (preserving its existing features):

```toml
tokio = { version = "1.49.0", features = ["full", "rt", "test-util"] }
```

Do not rely on `full` to imply the test clock.

Define only under `#[cfg(test)]` in
`src/bin/monitor/durable_delivery_runtime.rs`:

```text
RuntimeRetryFixture
  builds the existing isolated RuntimeState and owns its TEST_CODE cleanup

CountingProvider
  increments provider_calls when loading original TEST_CODE evidence

CountingRenderer
  increments renderer_calls when producing original frozen TEST_CODE bytes

TestBlockingTaskSpawner
  injects a deterministic JoinError before/around the long-lived blocking task

InjectedRetryRunnerBootAuthority
  supplies an explicit nonce-bound TEST_CODE boot identity and lock handle;
  construction accepts `Option<&str>` only in tests so missing, empty and every
  ASCII-whitespace-only identity can be rejected before coordinator open
```

These helpers create the initial rejected envelope, then reset counters before
the retry cycle. They do not exist in production code. The injected boot
authority uses the same validation predicate as production and cannot read an
identity from an inherited environment variable or global.

- [ ] **Step 2: Write runtime RED tests**

```rust
#[tokio::test]
async fn br192_retry_cycle_measures_zero_provider_and_renderer_calls() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_cycle_guard_survives_async_cancellation_until_blocking_exit() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_cycle_guard_covers_sink_and_completed_or_failed_terminalization() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_cycle_guard_latches_running_when_failure_pending_is_not_durable() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_precycle_namespace_error_never_acquires_guard() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_begin_not_committed_proof_releases_guard() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_no_commit_proof_concurrent_change_latches_outer_guard() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_begin_commit_ambiguous_error_latches_guard() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_same_boot_completion_pending_blocks_new_started_until_exact_resume_terminalizes() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_same_boot_failure_appended_blocks_new_started_until_exact_resume_terminalizes() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_begin_db_admission_rejects_any_global_running_before_second_started() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_same_boot_safe_terminal_selector_is_current_identity_exact_and_totally_ordered() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_runtime_boundary_uses_only_durable_result_and_exact_failure_constructors() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_cycle_failure_public_typed_constructors_compile_from_monitor_boundary() {
    use stock_analysis::durable_delivery::{
        DurableDeliveryError, RetryCycleFailure, RetryCycleOperation,
    };
    let _ = std::any::TypeId::of::<DurableDeliveryError>();
    let _ = std::any::TypeId::of::<RetryCycleOperation>();
    let _constructors = (
        RetryCycleFailure::from_retry_attempt_start_audit_unavailable,
        RetryCycleFailure::from_authorization_reconciliation_blocked_sha256,
        RetryCycleFailure::from_cycle_operation_failed,
        RetryCycleFailure::from_panic_sha256,
        RetryCycleFailure::from_join_error_sha256,
        RetryCycleFailure::from_process_interrupted_owner_boot_identity_sha256,
    );
    let _accessors = (RetryCycleFailure::reason, RetryCycleFailure::typed_fields_sha256);
}

#[tokio::test]
async fn br192_begin_not_committed_propagates_exact_typed_error_after_proof_release() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_begin_commit_ambiguous_propagates_exact_typed_error_and_latches_guard() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_guard_failures_return_exact_typed_variants_without_string_downgrade() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_normal_error_after_claim_quarantines_same_cycle_attempt_before_failed() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_caught_panic_after_claim_quarantines_same_cycle_attempt_before_failed() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_completion_pending_error_resumes_completed_without_failed_slot() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test(start_paused = true)]
async fn br192_periodic_runner_delays_first_tick_thirty_seconds() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_startup_and_periodic_paths_share_one_cycle_algorithm() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_r09_sourceonly_dispatch_is_reachable_exactly_once() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_cycle_identity_is_durable_before_sink_capable_spawn() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_boot_authority_rejects_missing_empty_and_ascii_whitespace_before_cycle_insert() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_cycle_persists_exact_preopen_boot_authority_identity() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_blocking_panic_is_caught_and_failed_event_is_appended() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_runtime_preserves_typed_start_audit_failure_through_failed_join() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_join_error_recovers_retained_cycle_identity() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_join_error_quarantines_started_or_inflight_attempt_before_cycle_failure() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_join_orphan_and_cancellation_recovery_capabilities_exclude_sink_provider_renderer() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_same_boot_running_cycle_is_not_orphan_recovered() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_next_startup_recovers_prior_boot_running_cycle() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_prior_boot_recovery_append_acks_uncertainty_before_failure_prepare() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_completion_pending_reboot_resumes_completed_without_failed_slot() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_completion_appended_reboot_resumes_completed_without_failed_slot() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_failure_pending_reboot_uses_only_persisted_bytes() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_failure_appended_reboot_uses_only_persisted_bytes() { panic!("BR-192 RED: named contract is not implemented"); }

#[tokio::test]
async fn br192_failure_reboot_corruption_matrix_fails_closed() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
#[ignore = "invoked only by the BR-192 boot-recovery parent harness"]
fn br192_boot_recovery_process_child() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
#[ignore = "invoked only by the BR-192 terminal-reboot parent harness"]
fn br192_terminal_reboot_process_child() { panic!("BR-192 RED: named contract is not implemented"); }
```

- [ ] **Step 3: Prove RED with full names**

```bash
cargo test --bin monitor durable_delivery_runtime::tests::br192_retry_cycle_measures_zero_provider_and_renderer_calls -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_guard_survives_async_cancellation_until_blocking_exit -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_guard_covers_sink_and_completed_or_failed_terminalization -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_guard_latches_running_when_failure_pending_is_not_durable -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_precycle_namespace_error_never_acquires_guard -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_not_committed_proof_releases_guard -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_no_commit_proof_concurrent_change_latches_outer_guard -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_commit_ambiguous_error_latches_guard -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_completion_pending_blocks_new_started_until_exact_resume_terminalizes -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_failure_appended_blocks_new_started_until_exact_resume_terminalizes -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_db_admission_rejects_any_global_running_before_second_started -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_safe_terminal_selector_is_current_identity_exact_and_totally_ordered -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_retry_runtime_boundary_uses_only_durable_result_and_exact_failure_constructors -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_not_committed_propagates_exact_typed_error_after_proof_release -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_commit_ambiguous_propagates_exact_typed_error_and_latches_guard -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_guard_failures_return_exact_typed_variants_without_string_downgrade -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_periodic_runner_delays_first_tick_thirty_seconds -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_startup_and_periodic_paths_share_one_cycle_algorithm -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_identity_is_durable_before_sink_capable_spawn -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_boot_authority_rejects_missing_empty_and_ascii_whitespace_before_cycle_insert -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_persists_exact_preopen_boot_authority_identity -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_blocking_panic_is_caught_and_failed_event_is_appended -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_runtime_preserves_typed_start_audit_failure_through_failed_join -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_normal_error_after_claim_quarantines_same_cycle_attempt_before_failed -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_caught_panic_after_claim_quarantines_same_cycle_attempt_before_failed -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_completion_pending_error_resumes_completed_without_failed_slot -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_join_error_recovers_retained_cycle_identity -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_join_error_quarantines_started_or_inflight_attempt_before_cycle_failure -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_join_orphan_and_cancellation_recovery_capabilities_exclude_sink_provider_renderer -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_running_cycle_is_not_orphan_recovered -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_next_startup_recovers_prior_boot_running_cycle -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_prior_boot_recovery_append_acks_uncertainty_before_failure_prepare -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_completion_pending_reboot_resumes_completed_without_failed_slot -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_completion_appended_reboot_resumes_completed_without_failed_slot -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_failure_pending_reboot_uses_only_persisted_bytes -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_failure_appended_reboot_uses_only_persisted_bytes -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_failure_reboot_corruption_matrix_fails_closed -- --exact --test-threads=1
```

Expected: each reports `running 1 test` and fails before implementation.
The runtime-boundary source/signature test scopes the BR-192 runner,
boot/namespace helpers, guard methods, terminal finalizers and prior-boot
recovery bodies and rejects `Result<_, String>`, `.to_string()`, textual
`map_err`, `RetryCycleFailure::panic` and `RetryCycleFailure::join_error`. It
also type-checks the exact `durable_delivery::Result<T>` signatures and the
`std::result::Result<RetryCycleEvidence, RetryCycleFailure>` blocking
signature. The two begin tests inject distinguishable exact
`DurableDeliveryError` variants: definite no-commit must return that same
variant only after consuming the proof and clearing the guard; ambiguous commit
must return that same variant with the guard still latched. The guard test
pattern-matches `RetryCycleAlreadyRunning` and
`RetryCycleGuardCompareExchangeInvariant` and proves neither path produces a
string or generic variant. Each failure-path assertion also joins the retained
cycle/terminal audit state appropriate to its boundary; pre-cycle failures
prove no cycle/audit row was created.
The four same-boot tests freeze the repaired boundary: append,
acknowledgement and terminal-CAS failpoints leave the old cycle in an exact
safe phase; a subsequent invocation is paused immediately before that exact
resume terminal CAS and proves zero new `Started`, then terminalizes it with
zero sink and commits exactly one later cycle/`Started`. A separate recovery
error assertion proves zero new `Started` and a latched guard. The
identity/order case mixes current,
different and malformed owner identities plus colliding leading keys; only the
exact current-identity four-phase snapshot is resumed in frozen order, while
the global begin check rejects any retained other row.

- [ ] **Step 4: Implement one cycle-global algorithm**

Add one pre-open authority type next to `RuntimeState`:

```rust
struct RetryRunnerBootAuthority {
    owner_boot_identity: String,
    process_lifetime_lock: std::fs::File,
}

impl RetryRunnerBootAuthority {
    fn acquire_before_open(namespace: &RuntimeNamespace) -> Result<Self>;
    fn owner_boot_identity(&self) -> &str;
}
```

`acquire_before_open` resolves the same durable-delivery root as the runtime,
acquires its exclusive provider-free retry runner lock, creates one
cryptographically strong domain-separated identity, validates that it contains
at least one non-ASCII-whitespace character and returns the lock-owning
authority. It runs before `DurableDeliveryCoordinator::open` and before the
immutable append authority is opened. Production exposes no identity override.
Only `InjectedRetryRunnerBootAuthority` under `#[cfg(test)]` may inject an
identity, and only below a matching nonce-bound TEST_CODE root.

Add to `RuntimeState`:

```rust
retry_boot_authority: Arc<RetryRunnerBootAuthority>,
retry_cycle_running: Arc<AtomicBool>,
```

Update all five existing `RuntimeState` literals atomically in the same compile
step: the runtime constructor and the four test literals in
`src/bin/monitor/durable_delivery_runtime.rs`. Each must receive an explicit
lock-owning production or injected TEST_CODE boot authority plus the running
flag; do not leave a later test-only compile fix. The runtime constructor order
is fixed:

```text
validate namespace/root
-> RetryRunnerBootAuthority::acquire_before_open
-> open coordinator/append/sink
-> retain the boot authority in RuntimeState
```

There is no lazy/global boot-identity getter, environment fallback or
coordinator-owned boot lookup.

Define all cycle functions, helpers and their imports inside one
brace-delimited `mod provider_free_retry` in
`src/bin/monitor/durable_delivery_runtime.rs`. The runtime projects the full
state into only:

```rust
pub(super) struct RetryCycleCapabilities {
    pub(super) coordinator: Arc<DurableDeliveryCoordinator>,
    pub(super) append: Arc<dyn ImmutableAppendPort>,
    pub(super) sink: AuthoritativeSink,
    pub(super) namespace_sha256: String,
    pub(super) owner_boot_identity: String,
}

#[derive(Clone)]
pub(super) struct RetryCycleRecoveryCapabilities {
    pub(super) coordinator: Arc<DurableDeliveryCoordinator>,
    pub(super) append: Arc<dyn ImmutableAppendPort>,
}

struct RetryCycleGuard {
    running: Arc<AtomicBool>,
    safe_release_proven: bool,
}

impl RetryCycleGuard {
    fn acquire(running: &Arc<AtomicBool>) -> Result<Option<Self>> {
        match running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire) {
            Ok(false) => Ok(Some(Self {
                running: Arc::clone(running),
                safe_release_proven: false,
            })),
            Err(true) => Ok(None),
            Ok(true) | Err(false) => {
                Err(DurableDeliveryError::RetryCycleGuardCompareExchangeInvariant)
            }
        }
    }

    fn release_after_verified_safe_state(
        &mut self,
        coordinator: &DurableDeliveryCoordinator,
        cycle_identity: &str,
    ) -> Result<()> {
        coordinator.verify_retry_cycle_guard_release_state(cycle_identity)?;
        self.safe_release_proven = true;
        self.running.store(false, Ordering::Release);
        Ok(())
    }

    fn release_after_verified_no_cycle(
        &mut self,
        coordinator: &DurableDeliveryCoordinator,
        proof: NoRetryCycleCommitted,
    ) -> Result<()> {
        coordinator.consume_no_retry_cycle_committed(proof)?;
        self.safe_release_proven = true;
        self.running.store(false, Ordering::Release);
        Ok(())
    }
}

impl Drop for RetryCycleGuard {
    fn drop(&mut self) {
        if !self.safe_release_proven {
            // Fail closed: retain `true`; same-boot cycles stay isolated.
            log::error!(
                "[DurableDelivery][BR-192] unsafe retry-cycle guard drop; \
                 running flag remains latched"
            );
        }
    }
}
```

Add:

```rust
fn retry_cycle_blocking(
    capabilities: &RetryCycleCapabilities,
    cycle_identity: &str,
) -> std::result::Result<RetryCycleEvidence, RetryCycleFailure>;
```

The guard is deliberately **not** an argument to `retry_cycle_blocking`.
`RetryCycleCapabilities` also contains no raw running flag or guard, so
`retry_cycle_blocking` cannot observe, clone or mutate the atomic. Only the
outer `RetryCycleGuard` created before the blocking closure owns and mutates
that raw flag after acquisition.
`run_provider_free_retry_cycle` moves it into the outer `spawn_blocking` owner
closure, keeps it outside the inner `catch_unwind`, and explicitly releases it
only after completion/failure finalization. This ownership covers the sink
call and every terminal append, acknowledgement and CAS. The coordinator's
guard-release validator accepts only terminal `Completed|Failed`, or
`Running/CompletionPending|CompletionAppended` with one valid complete
immutable Completed outbox, or `Running/FailurePending|FailureAppended` with
one valid complete immutable failure payload and its matching exact
Pending/Appended Failed outbox. The separate pre-cycle release accepts only
the consumed coordinator-issued `NoRetryCycleCommitted` proof. A
failure before that safety point leaves the flag latched and blocks same-boot
reentry.

Use the Task 1 library-owned `RetryCycleFailure` and
`RetryCycleFailureReason` plus the closed `RetryCycleOperation`; do not
redefine runtime-local lookalikes. Call only the six exact associated
functions frozen in Task 1:
`from_retry_attempt_start_audit_unavailable`,
`from_authorization_reconciliation_blocked_sha256`,
`from_cycle_operation_failed`, `from_panic_sha256`,
`from_join_error_sha256`, and
`from_process_interrupted_owner_boot_identity_sha256`. The first receives the
typed `DurableDeliveryError` directly and hashes its exact canonical
`attempt_identity,reason_code` fields; it never parses `Display` text. The
other paths pass only their closed operation enum and/or already-redacted
stable lowercase 64-hex digest. Runtime code cannot name or populate a private
failure field or preimage.

The module must not accept or name `RuntimeState`, `super::*`, callbacks or
unbounded generic capabilities. Its only production dependencies are the
coordinator/append/sink types above, BR-192 retry model types, chrono/Tokio,
the existing `log` facade and standard-library
synchronization/collections/hash primitives.
`RetryCycleCapabilities` is the non-clone sink-bearing execution capability.
Every JoinError, orphan, startup or cancellation-recovery helper accepts only
`RetryCycleRecoveryCapabilities`, whose fields are exactly coordinator and
append.

It owns one:

```rust
let mut attempted_decision_identities = BTreeSet::<String>::new();
```

After authorization/cycle-audit reconciliation, drain expired Active schedules
to a fixed point. Then process Reserved identities, applying the same pre-start
expiry check before any start/ownership operation. Drain expiry again and use
the closed `ExpiredFound|Candidates` snapshot loop before accepting a retry-
candidate vector. The same attempted set is passed to Reserved and candidate
phases. Insert before the sink
boundary, never clear it, and emit `DuplicateSuppressed` for a post-Reserved
candidate already present. Before either phase, quarantine every different-
boot attempt whose start is appended or whose consumed ownership/
`AttemptInFlight` lacks a terminal authoritative result. That quarantine
persists `ProcessInterruptedAfterSinkStart`, drains the three uncertainty
stages and makes zero sink calls; it runs before the algorithm considers
ordinary `Reserved` work.

The three primary cycle-work reads reject missing tie keys before returning
rows and use these exact stable orders:

```text
Reserved:
  business_date ASC, created_at ASC, decision_identity ASC,
  current_attempt_identity ASC

prior-boot Running:
  CASE terminal_phase
    WHEN 'CompletionAppended' THEN 0
    WHEN 'CompletionPending' THEN 1
    WHEN 'FailureAppended' THEN 2
    WHEN 'FailurePending' THEN 3
    WHEN 'NotPrepared' THEN 4
  END ASC,
  scheduled_for ASC, started_at ASC, cycle_identity ASC

retry candidates:
  next_eligible_at ASC, decision_identity ASC,
  rejection_disposition_identity ASC, authorization_identity ASC
```

Reserved requires non-null `current_attempt_identity`. Prior-boot rows require
non-null phase/schedule/start/cycle keys. Retry candidates require non-null
schedule, all three identities, exact R-09 kind/seam/catalog/attestation
provenance, `next_eligible_at<=now`, `expires_at>now`, and
`exhausted_at IS NULL`. In the same transaction the selector returns
`ExpiredFound` if any expired Active row exists; `expires_at>now` alone never
proves the drain completed. No rowid, caller/provider order, implicit NULL order,
unordered map/set iteration or result truncation is authoritative. All three
queries contain no SQL `LIMIT`, `OFFSET`, keyset predicate, caller cursor/page
size or caller cardinality. Each method validates and materializes the entire
qualifying read snapshot before returning any row, then returns the frozen
vector count and SHA-256 with the ordered rows. There is no continuation token:
exhaustion is consuming exactly that vector length. Prior-boot work is fully
recovered before new-cycle insertion; Reserved work is fully consumed before
the one candidate snapshot; authorization committed after that candidate read
belongs to the next cycle. The BTreeSet prevents a duplicate call but does not
define query order.

Implement source/static and 257-row boundary tests that reject any reintroduced
`LIMIT`, `OFFSET`, cursor or caller cardinality, collide every leading key,
reverse insertion order and still prove the complete tail, final tie-break,
frozen count/hash and one evidence outcome per row.

The authorization and cycle-audit drains use the exact one-row predicates,
binary orders, explicit null handling, `LIMIT 1` and 4096-progress bounds
specified in Tasks 3 and 4. Those contracts are independent of these three
primary selectors and must not be replaced by table order or a generic
“oldest” helper.

Keep `ReconcileSummary` unchanged as its existing eight-field type. Its one
constructor and the two existing runtime consumers continue to use
`progress_count`; candidate discovery and cycle evidence use the new dedicated
coordinator APIs rather than adding retry fields to that summary.

- [ ] **Step 5: Persist identity before spawn and recover every exit**

Add runtime helpers with explicit coordinator/append capabilities:

```rust
fn finish_cycle_completed_and_append(
    capabilities: &RetryCycleRecoveryCapabilities,
    cycle_identity: &str,
    evidence: RetryCycleEvidence,
) -> Result<RetryCycleEvidence>;

fn finish_cycle_failed_and_append(
    capabilities: &RetryCycleRecoveryCapabilities,
    cycle_identity: &str,
    failure: RetryCycleFailure,
    failed_at: DateTime<Utc>,
) -> Result<RetryCycleEvidence>;

fn recover_orphan_running_cycle_and_append(
    capabilities: &RetryCycleRecoveryCapabilities,
    cycle_identity: &str,
    failure: RetryCycleFailure,
) -> Result<RetryCycleEvidence>;

fn recover_prior_boot_running_cycles_and_append(
    capabilities: &RetryCycleRecoveryCapabilities,
    current_owner_boot_identity: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>>;

fn redacted_panic_hash(payload: Box<dyn Any + Send>) -> String;
fn redacted_error_hash(error: &str) -> String;
fn retry_namespace_preimage_v1(
    namespace: &RuntimeNamespace,
) -> Result<RetryNamespaceHashPreimageV1>;
fn retry_runtime_state() -> Result<Arc<RuntimeState>>;
```

Here and throughout this recipe, unqualified `Result<T>` is the imported
`stock_analysis::durable_delivery::Result<T>` alias. `retry_runtime_state` is a
typed sibling of the legacy monitor accessor: Task 8 factors the namespace,
cache and construction primitives so this function receives and propagates
their original `DurableDeliveryError` variants directly. It must not call a
`Result<_, String>` accessor, parse its message, hash it, or wrap it in a
generic variant. Existing non-retry callers may retain their legacy outer
logging boundary; no such boundary exists inside this BR-192 runner.

```rust
pub async fn run_provider_free_retry_cycle(
    scheduled_for: DateTime<Utc>,
) -> Result<RetryCycleEvidence> {
    let state = retry_runtime_state()?;
    // Every fallible namespace/boot validation precedes guard acquisition.
    let namespace_preimage = retry_namespace_preimage_v1(&state.namespace)?;
    let namespace_sha256 = retry_namespace_sha256(&namespace_preimage)?;
    let owner_boot_identity = state
        .retry_boot_authority
        .owner_boot_identity()
        .to_owned();
    validate_retry_owner_boot_identity(&owner_boot_identity)?;
    let mut guard = RetryCycleGuard::acquire(&state.retry_cycle_running)?
        .ok_or(DurableDeliveryError::RetryCycleAlreadyRunning)?;

    let now = Utc::now();
    // Recovery-only authority: exact same-boot Pending/Appended slots must
    // terminalize before begin. Any error creates no Started and deliberately
    // leaves the guard latched for restart recovery.
    state
        .coordinator
        .resume_same_boot_retry_cycle_terminal_slots(
            state.append.as_ref(),
            &owner_boot_identity,
            now,
        )?;
    // Bounded coordinator transaction. Running + Started outbox commit before
    // the long-lived sink-capable spawn; async parent retains this identity.
    let cycle_identity = match state
        .coordinator
        .begin_retry_cycle_before_spawn(
            &namespace_sha256,
            &owner_boot_identity,
            scheduled_for,
            now,
        ) {
        Ok(RetryCycleBeginOutcome::Started { cycle_identity }) => cycle_identity,
        Ok(RetryCycleBeginOutcome::NotCommitted { error, proof }) => {
            guard.release_after_verified_no_cycle(&state.coordinator, proof)?;
            return Err(error);
        }
        Err(commit_ambiguous) => {
            // No proof: Drop deliberately leaves the same-boot flag latched.
            return Err(commit_ambiguous);
        }
    };

    let execution_capabilities = provider_free_retry::RetryCycleCapabilities {
        coordinator: Arc::clone(&state.coordinator),
        append: Arc::clone(&state.append) as Arc<dyn ImmutableAppendPort>,
        sink: Arc::clone(&state.sink),
        namespace_sha256,
        owner_boot_identity,
    };
    let recovery_capabilities =
        provider_free_retry::RetryCycleRecoveryCapabilities {
            coordinator: Arc::clone(&state.coordinator),
            append: Arc::clone(&state.append) as Arc<dyn ImmutableAppendPort>,
        };
    let recovery_identity = cycle_identity.clone();
    let blocking_recovery_capabilities = recovery_capabilities.clone();
    let joined = tokio::task::spawn_blocking(move || {
        let mut guard = guard;
        let execution_result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            provider_free_retry::retry_cycle_blocking(
                &execution_capabilities,
                &cycle_identity,
            )
        }));
        let finalized = match execution_result {
            Ok(Ok(evidence)) => provider_free_retry::finish_cycle_completed_and_append(
                &blocking_recovery_capabilities,
                &cycle_identity,
                evidence,
            ),
            Ok(Err(error)) => provider_free_retry::finish_cycle_failed_and_append(
                &blocking_recovery_capabilities,
                &cycle_identity,
                error,
                Utc::now(),
            ),
            Err(payload) => {
                let panic_sha256 = redacted_panic_hash(payload);
                provider_free_retry::finish_cycle_failed_and_append(
                    &blocking_recovery_capabilities,
                    &cycle_identity,
                    RetryCycleFailure::from_panic_sha256(&panic_sha256)?,
                    Utc::now(),
                )
            }
        };
        guard.release_after_verified_safe_state(
            &blocking_recovery_capabilities.coordinator,
            &cycle_identity,
        )?;
        finalized
    })
    .await;

    match joined {
        Ok(result) => result,
        Err(join_error) => {
            let join_error_sha256 =
                redacted_error_hash(&format!("JoinError:{join_error}"));
            provider_free_retry::recover_orphan_running_cycle_and_append(
                &recovery_capabilities,
                &recovery_identity,
                RetryCycleFailure::from_join_error_sha256(&join_error_sha256)?,
            )
        }
    }
}
```

The bounded pre-spawn transaction has no sink/provider/renderer capability. It
receives the validated boot identity explicitly, stores that exact value in the
new cycle row and fails before insert for an empty/ASCII-whitespace-only value.
The module imports `DurableDeliveryError` and its `Result<T>` alias from the
public durable-delivery root. Boot-authority I/O/isolation, namespace
canonicalization, coordinator, append, terminal-finalization and guard-release
failures propagate their original exact variant. Guard contention returns
`RetryCycleAlreadyRunning`; the begin transaction also returns that exact
variant plus `NoRetryCycleCommitted` when its global Running exclusion fires.
The runtime may release only after `consume_no_retry_cycle_committed` has
transactionally rederived the exact next ordinal/identity, byte-matched the
selected retained Running witness and reproved zero proposed cycle/Started
rows. Concurrent insert or witness change returns its original typed error and
leaves the outer guard latched.
The impossible atomic outcome alone returns
`RetryCycleGuardCompareExchangeInvariant`. None may be substituted with
`InvalidConfiguration` or text.
`retry_namespace_preimage_v1` is a pure exhaustive adapter from the already
validated runtime namespace to the frozen Production-or-Test preimage. It does
not inspect paths, environment variables, globals or debug output; the shared
`retry_namespace_sha256` helper alone serializes and hashes it.
The sink-bearing narrow projection is built only after the cycle identity and
global `Started` slot commit succeeds, immediately before spawn. The parent
retains a typed `RetryCycleFailure` across every failure path. In particular,
`DurableDeliveryError::RetryAttemptStartAuditUnavailable` is mapped by direct
variant matching through
`RetryCycleFailure::from_retry_attempt_start_audit_unavailable(&error)`, which
fixes `RetryCycleFailureReason::RetryAttemptStartAuditUnavailable` and the
canonical `attempt_identity,reason_code` fields. The `Failed` outbox
canonical payload persists that reason as
`retry_attempt_start_audit_unavailable` and the unchanged digest. Neither
`retry_cycle_blocking` nor the finalizer accepts or parses display text.
The panic branch calls `from_panic_sha256(&panic_sha256)` and the JoinError
branch calls `from_join_error_sha256(&join_error_sha256)` exactly; short-form
`RetryCycleFailure::panic`/`join_error` methods do not exist.
Before selecting a failure-finalization branch, the coordinator reloads the
named attempt and classifies persisted state as exactly
`PreStartUnacknowledged` or `StartAcknowledgedOrOwnershipConsumed`. The first is
legal only when no acknowledged start and no consumed ownership exists and
uses the narrow unchanged-state finalizer. The second rejects that exception
and routes the same typed failure through ordinary quarantine. This classifier
has no caller boolean/default and runs in the failure transaction.
The async parent independently constructs
`RetryCycleRecoveryCapabilities` from coordinator and append; it never clones,
converts or projects the sink-bearing execution capability for recovery.
The actual implementation may use owned values rather than the illustrative
lifetimes, but identity and guard ownership must follow the shown boundary:
the guard is outside `catch_unwind`, survives sink execution and remains held
through completion/failure append, acknowledgement and terminal CAS. Its
explicit release performs the database safe-state check; Drop without that
proof leaves the running flag latched true.
The `Completed` helper inserts, appends and acknowledges its stable terminal
event and performs the Completed terminal CAS. An error before
`prepare_retry_cycle_completed` commits may map to `CycleOperationFailed` only
after authoritative proof that neither terminal slot exists. Once
`CompletionPending|CompletionAppended` exists, the helper reloads and resumes
only the exact stored Completed append/ack/CAS; it can never invoke failure
preparation. The narrow start-audit-unavailable helper stores and
append/acknowledges the same complete failure payload/slot but, only for
`PreStartUnacknowledged`, preserves byte-identical
decision/attempt/binding/schedule rows, canonical hashes and pending start
bytes. The single ordinary `finish_cycle_failed_and_append` helper is used for
a normal `retry_cycle_blocking` error, a caught panic and
`StartAcknowledgedOrOwnershipConsumed`. Before preparing the `Failed` slot, it
proves `terminal_phase=NotPrepared` and no terminal slot, then calls
`quarantine_same_cycle_attempts_before_failure(cycle_identity, failed_at)`, drains
every exact same-cycle uncertainty outbox to acknowledged fixed point with zero
sink calls, and only then invokes
`coordinator.prepare_retry_cycle_failed(cycle_identity, &failure, failed_at)`.
That library method privately persists the complete canonical typed preimage
and envelope plus both hashes, and freezes the payload identity/reason/hashes
into the `Running/FailurePending` cycle row and canonical Pending `Failed`
outbox; the helper then append/acknowledges that exact outbox,
observes `Running/FailureAppended`, invokes
`coordinator.terminalize_retry_cycle_failed(cycle_identity, failed_at)`, and
only then reads evidence whose `final_failure` equals the input DTO. This
selector includes any same-cycle nonterminal
attempt with appended/acknowledged `SinkAttemptStarted`,
`send_consumed=true`/ownership `Started`, or `AttemptInFlight`, and no terminal
authoritative result; it creates or advances the retained ownership to
`InterruptedUncertain` for every selected attempt. An append failure leaves the
exact pending uncertainty or terminal outbox and the cycle
`Running/FailurePending` only after the full immutable payload is durable; an
ack-before-CAS crash leaves
`Running/FailureAppended`. Neither path writes terminal `Failed` ahead of
unresolved uncertainty or exact immutable acknowledgement.
Join recovery is idempotent and uses the same quarantine-before-failure
ordering only for `NotPrepared`. If completion/failure Pending/Appended already
exists, it validates and resumes that exact kind first. It never calls the sink,
never creates an opposite terminal kind and never changes an indeterminate
attempt back to `Reserved`.

The cancellation test aborts the async waiter while `BlockingSink` is held,
proves a second cycle is rejected, releases the sink and only then proves a
later cycle can start. Separate ordinary-error-after-claim and
panic-after-claim tests require every same-cycle nonterminal start to reach
`ProcessInterruptedAfterSinkStart`/`InterruptedUncertain`, append/ack all
uncertainty evidence before appended `Failed`, converge through manual review
and make zero recovery sink calls. The JoinError test uses
`TestBlockingTaskSpawner` and proves recovery by the retained identity. Its
started/in-flight variant proves the same ordering.

Completion/failure append, acknowledgement or terminal-CAS failure after the
guard-release safety point has a different next-invocation contract. The next
guard owner first calls
`resume_same_boot_retry_cycle_terminal_slots` with the retained boot identity.
The test pauses that exact resumer before its terminal CAS and proves the
transactional begin path has not run and no second `Started` exists; after the
CAS it proves exactly one later Running+Started pair can commit. If same-boot
resumption returns an error, no begin is called and the guard remains latched.
If an unresolved row has another boot identity, is same-boot `NotPrepared`, or
otherwise survives recovery, the begin `BEGIN IMMEDIATE` returns definite
`RetryCycleAlreadyRunning + NoRetryCycleCommitted`, writes zero bytes and
releases only after a fresh transaction consumes that exact proof by
rederiving the next ordinal/identity, byte-matching the retained Running
witness and re-querying zero proposed cycle/Started rows.

At monitor startup, `RetryRunnerBootAuthority::acquire_before_open` acquires the
exclusive process-lifetime lock below the resolved durable-delivery root and
fixes the validated boot identity before coordinator/append open. Once those
authorities are open, startup calls
`recover_prior_boot_running_cycles_and_append(&recovery_capabilities,
state.retry_boot_authority.owner_boot_identity(), now)`
before `run_provider_free_retry_cycle`. The helper passes that same identity to
the coordinator method and drains the returned cycles' outboxes. The recovery
query excludes `owner_boot_identity = current`; a same-boot `Running` row
remains untouched by orphan recovery but is subject to the pre-begin
same-boot-safe resumer and global Running exclusion. For different prior-boot
rows it first resumes, in exact
order, `CompletionAppended`, `CompletionPending`, `FailureAppended` and
`FailurePending`, using only their stored bytes and zero sink calls. Only a
`NotPrepared` row quarantines every
appended-start/consumed-ownership/`AttemptInFlight` attempt without terminal
sink result and creates only phase-one uncertainty outboxes. It drains and
acknowledges those outboxes to a verified fixed point with zero sink calls. A
phase-one failure creates no failure payload or Failed slot. Only after the
fixed point does it call `prepare_prior_boot_retry_cycle_failed` to persist
the complete canonical typed `ProcessInterrupted` preimage/envelope and both
hashes, CAS the cycle to `Running/FailurePending`, and insert
`OrphanRecovered`/Pending `Failed`. It then append/acknowledges phase-two bytes
and terminalizes only from
`Running/FailureAppended` before the startup cycle. Existing completion/failure
Pending/Appended cycles resume their exact terminal kind and bytes and do not
replace their payload; the new process obtains completion bytes or failure
reason, full typed preimage, full envelope and both hashes solely from SQLite
and does not construct a caller-supplied replacement
`RetryCycleFailure`. Only after private
decode→canonical-reserialize→hash validation may the library rebuild its opaque
read-only failure evidence for the returned cycle DTO.

The real process-death harness launches an ignored TEST_CODE child with an
explicit injected boot authority, waits until that child has persisted a
`Running` cycle containing its exact boot identity, then terminates the child
and releases its lock. It runs separate prior-boot cases with no attempt start,
with appended/acknowledged `SinkAttemptStarted`, and with consumed ownership/
`AttemptInFlight`, all without a terminal authoritative result. The parent
starts a new TEST_CODE runtime at the same root with a distinct explicit boot
identity and asserts prior-boot convergence. Only the no-start case may remain
ordinary recoverable `Reserved`; both indeterminate-start cases must record
`ProcessInterruptedAfterSinkStart`, reach manual review and make zero recovery
sink calls.
The same harness first presents a `Running` row bearing the current identity and
asserts zero state/event mutation. Child role, namespace/root and boot identity
are explicit nonce-bound inputs; the child refuses missing/unknown role,
missing/empty/ASCII-whitespace identity, a production root or a namespace/root
mismatch, or an unknown attempt phase. Production has no boot-identity
environment override.

Use these exact child-only keys, clearing every inherited value before each
launch:

```text
TEST_CODE_BR192_BOOT_RECOVERY_CHILD_ROLE       PRIOR | CURRENT
TEST_CODE_BR192_BOOT_RECOVERY_DATABASE         shared DB below fixture root
TEST_CODE_BR192_BOOT_RECOVERY_NAMESPACE        exact nonce-bound TEST_CODE
TEST_CODE_BR192_BOOT_RECOVERY_IDENTITY         explicit validated boot identity
TEST_CODE_BR192_BOOT_RECOVERY_ATTEMPT_PHASE    NONE | START_APPENDED | ATTEMPT_IN_FLIGHT
TEST_CODE_BR192_BOOT_RECOVERY_READY             role-specific ready file
TEST_CODE_BR192_BOOT_RECOVERY_GATE              shared gate file
```

Add one ignored
`durable_delivery_runtime::tests::br192_boot_recovery_process_child` test. The
parent launches the current monitor test executable with `--exact`, that full
test name, `--ignored`, `--nocapture`, and `--test-threads=1`. The child validates
the role, TEST_CODE nonce, fixture-root confinement and boot identity before
opening anything. `PRIOR` persists `Running` with its explicit identity and the
requested attempt phase, then waits at the gate; `CURRENT` proves same-boot
exclusion and then, after the prior lock owner has exited, uses its distinct
identity to converge only the prior-boot row. Attempt-owner keys from the Task
5 race are not boot identities and must never be reused as such.

Add a second real child-process protocol for terminal-slot reboot. A writer
child under one nonce-bound TEST_CODE root and boot identity persists one of
`CompletionPending|CompletionAppended|FailurePending|FailureAppended`, writes
a ready receipt containing only root/cycle/phase/expected hashes, and exits
without returning an in-memory terminal DTO. A recovery child uses a distinct
boot identity and the production-shaped recovery entry at that exact root.
Pending appends/acknowledges exact stored bytes then CASes; Appended only
validates and CASes. All four cases make zero provider/renderer/sink calls and
cannot create the opposite terminal kind.

For both failure phases, clone the isolated fixture and corrupt one field per
case: typed-preimage bytes, typed-preimage hash, envelope bytes, envelope hash,
payload identity, and the Failed binding (cycle/payload/hash/reason). Each
recovery child must exit non-zero with no terminal mutation, no opposite/new
slot, no successful acknowledgement and zero sink calls. The exact child test
is
`durable_delivery_runtime::tests::br192_terminal_reboot_process_child`; parent
tests are:

```text
br192_completion_pending_reboot_resumes_completed_without_failed_slot
br192_completion_appended_reboot_resumes_completed_without_failed_slot
br192_failure_pending_reboot_uses_only_persisted_bytes
br192_failure_appended_reboot_uses_only_persisted_bytes
br192_failure_reboot_corruption_matrix_fails_closed
```

- [ ] **Step 6: Use delayed `interval_at` and existing shutdown select**

In `main.rs`, after eager authority binding and successful startup cycle:

```rust
let period = std::time::Duration::from_secs(30);
let mut interval = tokio::time::interval_at(
    tokio::time::Instant::now() + period,
    period,
);
interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
```

Use the existing shutdown token/select. Before starting the retry runner or
any counted producer, validate the design §1.1 producer catalog and emit its
15 exact startup lines once in `PushKind::ALL` order: fourteen disabled and
one exact enabled R-09 line. The retry runner then
emits its own startup banner once. Publish `delivery.retry.cycle` only after
the matching durable cycle terminal event is appended.

Add exact source/runtime tests:

```text
br192_counted_producer_registry_covers_all_kinds_once
br192_counted_producer_registry_rejects_missing_duplicate_and_empty_state
br192_missing_producers_emit_exact_startup_banners_before_acquisition
br192_enabled_producers_name_binding_seams_and_never_emit_disabled_banner
br192_disabled_producer_attempt_returns_before_acquisition_and_sink
br192_r09_enabled_producer_acquires_permit_before_gateway_and_freezes_expiry
br192_all_counted_callers_require_catalog_permit_or_binding
br192_fixed_head_inventory_classifies_every_counted_entry_call
br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297
br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink
br192_br198_same_day_1535_boundary_precedes_terminal_preflight
br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry
br192_br198_host_tz_cannot_change_shanghai_review_date_or_1535_boundary
br192_br198_capture_before_trusted_request_start_fails_pair_before_durable_sink
br192_br198_capture_after_trusted_request_completion_fails_pair_before_durable_sink
br192_br198_capture_raw_bytes_round_trip_and_mutation_rejects_pair_before_durable_sink
br192_br198_capture_before_request_date_fails_pair_before_durable_sink
br192_br198_capture_crosses_shanghai_midnight_fails_pair_before_durable_sink
br192_br198_invalid_provider_capture_timestamp_fails_pair_before_durable_sink
br192_br198_prior_date_initial_admission_ignores_retry_expiry_but_retry_rejects
br192_br200_r09_delivered_preflight_precedes_permit_gateway_renderer_and_sink
br192_br200_r09_delivered_missing_hydration_is_provider_free_retryable
br192_br200_r09_rejected_and_uncertain_are_provider_free_terminal
br192_br200_r09_nonterminal_is_provider_free_retryable
br192_br200_r09_corrupt_or_ambiguous_authority_fail_closed
br192_br200_r09_no_occurrence_orders_preflight_then_permit_then_provider_then_renderer_then_sink
br192_br200_r09_startup_barrier_failure_is_provider_free
br192_br200_business_date_once_claim_prevents_second_r09_decision
br192_r09_transition_and_hydration_bind_exact_ordered_rule_ids
br192_r09_sourceonly_dispatch_has_no_banner_account_or_broker_authority
```

The test harness records catalog validation, banner emission, acquisition and
sink sequence numbers; every disabled banner must precede the first possible
acquisition/sink sequence. A static test checks the literal
`disabled=no_producer reason=capability_unavailable:` schema and all frozen
reason codes from design §1.1.
The BR-200 tests independently assert outcome, `retryable`, `next_attempt` and
the exact mapping-table reason code; no combined state test may stand in for
those assertions. Every existing-occurrence case asserts zero permit/provider/
renderer/sink counters. The capture tests reject the complete atomic pair and
write no durable decision or sink result. The rule-ID test requires exact
ordered bytes `[BR-110,BR-140,BR-192,BR-194,BR-198,BR-200]` in producer,
schedule transition and hydration and rejects any mutation.

- [ ] **Step 7: Add a dependency-boundary static test**

In `tests/durable_delivery_counted_cutover.rs`, define a local
`extract_rust_module_body(source, module_name)` brace-balancing helper. It is
test-only and has no filesystem state.

Extract the complete `provider_free_retry` module slice, including its imports,
`RetryCycleCapabilities`, `RetryCycleRecoveryCapabilities`, guard and every
helper body. Reject:

```text
data_provider
data_gateway
Provider
Gateway
push_templates
render_
envelope_from_binding
deliver_counted_binding
RuntimeState
super::*
Fn(
FnMut(
FnOnce(
```

Allow only the coordinator/immutable-append/authoritative-sink types, BR-192
retry model types, chrono/Tokio and standard-library synchronization,
collections and hash primitives. Add a compile-time signature assertion that
the module entry accepts `&RetryCycleCapabilities` rather than `RuntimeState`,
and a source-graph assertion that every local call resolves within the scanned
module or the explicit allowlist. Reject provider constructors and any callback
or generic capability escape hatch. Freeze the exact five fields of
`RetryCycleCapabilities` (`coordinator`, `append`, `sink`,
`namespace_sha256`, `owner_boot_identity`) and reject `AtomicBool`,
`retry_cycle_running`, a raw running flag, or a guard field/reference in that
capability or anywhere reachable by `retry_cycle_blocking`; only the outer
`RetryCycleGuard` may own or mutate the atomic after acquisition.
Make the forbidden-dependency scan token/path/call aware: audited evidence
fields such as `provider_calls` and `renderer_calls` are allowed, but provider
or renderer types, imports, constructors and invocations are not.
Add exact signature/field assertions that every JoinError, orphan, startup and
cancellation-recovery helper accepts `&RetryCycleRecoveryCapabilities`; that
struct has exactly coordinator and append fields; and no recovery caller clones
or passes `RetryCycleCapabilities`. Any sink/provider/renderer/runtime field or
capability conversion in a recovery call fails the static test.

Also assert the module processes Reserved work before the candidate query,
calls `admit_authorized_retry`, then
`coordinator.prepare_retry_attempt ->
coordinator.reconcile_prepared_retry_attempt_audit ->
coordinator.claim_retry_sink_execution ->
coordinator.execute_prepared_retry_sink`, receives
`RetrySinkExecutionOutcome`, reconciles either exact expiry or sink-result
authority plus cycle audit and uses one
`attempted_decision_identities`. Reject direct retry-origin
`resume_deliverable`.
The source/runtime boundary also proves the forward-compatible rollback keeps
the retry-origin classifier and four-stage path; disabling periodic discovery
cannot route an existing retry-origin `Reserved` row through legacy resume.

- [ ] **Recipe Step 8: Run runtime/static tests during Task 8**

```bash
cargo test --bin monitor durable_delivery_runtime::tests::br192_retry_cycle_measures_zero_provider_and_renderer_calls -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_guard_survives_async_cancellation_until_blocking_exit -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_periodic_runner_delays_first_tick_thirty_seconds -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_startup_and_periodic_paths_share_one_cycle_algorithm -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_identity_is_durable_before_sink_capable_spawn -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_boot_authority_rejects_missing_empty_and_ascii_whitespace_before_cycle_insert -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_cycle_persists_exact_preopen_boot_authority_identity -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_blocking_panic_is_caught_and_failed_event_is_appended -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_runtime_preserves_typed_start_audit_failure_through_failed_join -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_join_error_recovers_retained_cycle_identity -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_join_error_quarantines_started_or_inflight_attempt_before_cycle_failure -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_join_orphan_and_cancellation_recovery_capabilities_exclude_sink_provider_renderer -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_running_cycle_is_not_orphan_recovered -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_next_startup_recovers_prior_boot_running_cycle -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rollback_preserves_four_stage_retry_origin_reserved_recovery -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_fixed_head_inventory_classifies_every_counted_entry_call -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297 -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_same_day_1535_boundary_precedes_terminal_preflight -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_host_tz_cannot_change_shanghai_review_date_or_1535_boundary -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_before_trusted_request_start_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_after_trusted_request_completion_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_raw_bytes_round_trip_and_mutation_rejects_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_before_request_date_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_crosses_shanghai_midnight_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_invalid_provider_capture_timestamp_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_prior_date_initial_admission_ignores_retry_expiry_but_retry_rejects -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_delivered_preflight_precedes_permit_gateway_renderer_and_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_delivered_missing_hydration_is_provider_free_retryable -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_rejected_and_uncertain_are_provider_free_terminal -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_nonterminal_is_provider_free_retryable -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_corrupt_or_ambiguous_authority_fail_closed -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_no_occurrence_orders_preflight_then_permit_then_provider_then_renderer_then_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_startup_barrier_failure_is_provider_free -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_business_date_once_claim_prevents_second_r09_decision -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_r09_transition_and_hydration_bind_exact_ordered_rule_ids -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_r09_sourceonly_dispatch_has_no_banner_account_or_broker_authority -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover -- --test-threads=1
```

Expected: measured provider/renderer deltas are zero, sink delta is one, and
all commands execute at least one test. Do not commit here; Task 8 owns the
single integration commit.

### Task 6: Define authenticated, recoverable manual authorization privately

**Files:**

- Modify `src/auth/operator.rs`
- Create `src/durable_delivery/retry_command.rs`
- Modify `src/durable_delivery/coordinator.rs`
- Modify `src/durable_delivery/mod.rs`
- Modify `src/durable_delivery/tests.rs`

Task 6 adds only the private `retry_command` module declaration to
`src/durable_delivery/mod.rs`. It does not publish a root re-export, create a
binary, add a `[[bin]]` stanza or add a `CARGO_BIN_EXE_*` reference. Those
integration changes wait for the single final integration step in Task 8.

- [ ] **Step 1: Define the library-owned production boundary and test-only injection seam**

Create:

```rust
// src/auth/operator.rs
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

// src/durable_delivery/retry_command.rs
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

The public request deliberately has no `authorized_at`, target/root, authority,
resolver, coordinator, append port, evidence bytes or test selector. The
library entry calls `authenticate_monitor_operator`, consumes its opaque
attestation and creates all remaining production authority internally. It is
the only production authorization path.

`OperatorAuthAttestation` is an opaque public return handle solely so the
library-owned authentication function has a legal public signature. All fields,
constructors and consuming accessors are private or `pub(crate)`; it has no
`Clone`, `Default`, serde implementation or public builder. The function
requires auth enabled, exact configured-subject match and TTY, then calls the
existing real PAM path. Only after PAM succeeds does `src/auth/operator.rs`
capture `Utc::now()`, read a fresh 32-byte nonce from `/dev/urandom`, and
construct the attestation with exact subject, configured PAM service and fixed
mechanism `"pam-password-v1"`. Any PAM/config/TTY/nonce failure returns no token.
`require_monitor_operator_auth()` delegates to this function when auth is
required and discards the result, preserving current monitor behavior without
a second attestation source.

`RetryCommandAuthorityKind`, `AuthenticatedRetryOperator`,
`ProductionRetryCommandTargetResolver`,
`RetryCommandTarget` and `ResolvedRetryCommandTarget` are module-private.
The `RetryCommandTarget::Test` variant and its constructors exist only under
`#[cfg(test)]`; the non-test private target is Production-only.
`AuthenticatedRetryOperator` contains lowercase 64-hex principal/session
evidence hashes, canonical authentication metadata, the exact authority kind
and authority-produced `validated_at`; its canonical bytes contain only hashed
principals and no credential or plaintext operator. The deny-unknown canonical
`RetryOperatorSessionEvidenceV1` has exact ordered fields
`schema_version=1`, `rule_id="BR-192"`, `authority_kind`, `auth_required`,
`expected_principal_sha256`, `claimed_principal_sha256`, `stdin_is_tty`,
`stdout_is_tty`, `pam_service`, `authentication_mechanism`,
`session_nonce_sha256`,
`validated_at`. Its lowercase digest is
`SHA-256(b"stock_analysis.durable_delivery.br192.retry_operator_session.v1\0"
|| canonical_utf8_bytes)`.
The module-private validator parses stored canonical bytes with
`deny_unknown_fields`, recomputes that hash, validates principal
hashes/timestamp and requires DTO/evidence authority kinds to equal the
successful production authority result, never command input.

`AuthenticatedRetryOperator` can be created only by consuming
`OperatorAuthAttestation`; the retry-command module has no raw-values
constructor and cannot call `Utc::now()` for authorization. The coordinator
receives the private authenticated token, not a free timestamp.
It requires the authorization row's `authorized_at`, canonical authorization
observation time and manual schedule eligibility input to equal
`AuthenticatedRetryOperator.validated_at` byte-for-byte. Delaying the command
after PAM authentication must not substitute `Utc::now()`, a request value,
filesystem time or append time. The public outcome reports that already
persisted authority time.

`ResolvedRetryCommandTarget` privately owns the fixed production target,
manifest-resolved database and immutable-append roots, fixed evidence path,
exact namespace preimage/hash, opened coordinator/append authority, exact
evidence bytes and evidence SHA-256. It has no public constructor and
implements neither serialization nor cloning because it contains live
capabilities. It is constructible only inside the production entry after the
authenticated operator has been validated.

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

`authorize_delivery_retry_production` performs this exact sequence:

1. validate request syntax without opening production artifacts;
2. require fixed Production target intent, `MONITOR_AUTH_REQUIRED=1`, matching
   configured operator and TTY;
3. call `authenticate_monitor_operator(&request.operator_identity)`, which
   returns an opaque attestation only after real PAM success, then consume that
   attestation exactly once to create and validate all private
   `AuthenticatedRetryOperator` fields/kind/session evidence, fixing its
   authority-owned `validated_at`; the command cannot construct subject,
   service, mechanism, nonce or time separately; and only then
4. construct `ProductionRetryCommandTargetResolver` internally and call its
   private resolution method, which rechecks compatibility/confinement,
   constructs
   `RetryNamespaceHashPreimageV1` from the now-fixed target, calls the sole
   `retry_namespace_sha256` helper, retains both on
   `ResolvedRetryCommandTarget`, and only then may open coordinator/append
   authority or read the evidence path; and
5. validate the exact persisted producer-attestation companion against the
   current library catalog, then drain every pending/expired retry schedule to
   a fixed point through its persisted expiry outbox before authorization;
6. call the coordinator's no-caller-time manual-authorization method. In its
   owning `BEGIN IMMEDIATE`, after all potentially blocking work is complete,
   the coordinator obtains a fresh `freshness_observed_at` from its sole private
   `ProductionFreshnessClock` immediately before the decision write and rechecks the
   target/current disposition, companion, schedule and
   `freshness_observed_at < expires_at`. `validated_at` remains only the manual
   authorization time and cannot satisfy freshness. If equality/later is
   observed, atomically create the zero-attempt schedule when absent and prepare
   only the stable Pending expiry-outbox row, commit, append
   and acknowledge its exact bytes, terminalize/clear authority, and return
   typed `NoLongerEligible::ExpiredFreshness` without inserting an
   authorization. A crash resumes the persisted outbox before a later command;
   and
7. otherwise insert/apply/reconcile the authorization and return only
   `ProductionRetryAuthorizationOutcome`.

The production freshness clock is private to `DurableDeliveryCoordinator`;
`retry_command.rs` owns no clock and cannot read or inject one. No request,
CLI, environment value or root export can provide time. A coordinator
cfg(test)-only nonce-bound TEST_CODE constructor exercises equality/midnight
races and cannot open production roots. There is no
provider/evidence/PAM/filesystem I/O between the final clock observation and
the transaction's freshness-dependent insert; the transaction persists that
observation in the expiry canonical bytes when it rejects.

The production resolver can return only fixed manifest-root production
capabilities and rejects any Test authority/root before open. Resolved
capabilities live for one command and are dropped after append/apply recovery.
The public function has no overload accepting authority/resolver/target,
pre-opened coordinator/append or evidence capabilities.

There is no authority trait or raw authority constructor, including in the
retry-command tests. Those tests receive an opaque attestation only from the
auth module's cfg(test) TEST_CODE factory and exercise a cfg(test)-only
recording target resolver/application seam with `pub(crate)`/private
visibility. These seams are absent from non-test builds and the root export
manifest. The test resolver requires the exact matching TEST_CODE nonce/root
and rejects Production authority, production roots and real-symbol authority.
No feature, environment variable, public constructor or CLI flag enables this
seam in production.

- [ ] **Step 2: Write command RED tests**

In `src/auth/operator.rs` define a `#[cfg(test)] pub(crate)` TEST_CODE
attestation factory; in `src/durable_delivery/retry_command.rs` define the
cfg(test) recording resolver/target with at most `pub(crate)` visibility.
`src/durable_delivery/tests.rs` consumes those only in unit-test builds. The
factory succeeds only for a named TEST_CODE subject/nonce, and the command
accepts it only with the identical nonce-bound test target. These tests reuse
the existing `Fixture` and `MemoryAppendPort`; helpers own no production path.
A recording resolver proves its open/read count remains zero on target-kind,
nonce or authentication failure.

Add:

```rust
#[test]
fn br192_test_attestation_can_mutate_only_nonce_bound_test_target() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_production_target_rejects_test_attestation_before_open() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_manual_authorization_identical_replay_returns_stored_identity() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_manual_authorization_rejects_all_three_uncertain_states() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_production_authority_rejects_before_open_when_not_authenticated() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authenticated_retry_operator_validates_kind_timestamp_and_session_evidence() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_resolver_is_unreachable_before_authentication_and_target_compatibility() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_resolved_production_target_cannot_hold_test_authority_root_or_capability() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_manual_target_namespace_is_derived_after_auth_before_open_and_persisted() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_manual_namespace_tampering_and_production_test_mismatch_fail_before_open() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_non_test_public_surface_has_no_injectable_authority_resolver_or_target() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_production_entry_owns_pam_and_fixed_manifest_resolver() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authorization_time_equals_authority_validated_at_even_when_command_is_delayed() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_authorization_request_has_no_caller_controlled_timestamp() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_operator_auth_attestation_is_opaque_and_created_only_after_pam_success() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_retry_command_consumes_attestation_without_constructing_time_subject_service_or_nonce() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_require_monitor_operator_auth_delegates_and_discards_attestation() { panic!("BR-192 RED: named contract is not implemented"); }
```

The first test uses a nonce-bound root under `data/test/TEST_CODE_*`, invokes
the real command application, drains `MemoryAppendPort`-equivalent production
interface in that test namespace and asserts the authorization is
`Appended/Applied`.
The auth-module test injects the PAM-success result below `try_pam_auth`,
proves no attestation exists on any earlier failure, and proves successful
subject/service/mechanism/time/nonce fields come from that module only. A
source/compile contract proves the opaque type has no public field,
constructor, clone/default/serde or retry-command literal. The
authenticated-operator test round-trips its canonical DTO with
`deny_unknown_fields` and independently corrupts the principal hash, authority
kind, validated timestamp and session-evidence hash. The recording resolver
asserts zero opens/reads before successful authentication and exact target-kind
compatibility. The resolved-target test uses only module-private test
inspection and proves no public constructor/serde/clone path can smuggle test
authority, roots or capabilities into Production. The namespace-ordering test
uses a recording resolver to prove auth and exact target compatibility precede
the one namespace-helper invocation, while coordinator/append/evidence open
counts remain zero until both the preimage and digest validate. It then proves
the canonical authorization row retains byte-exact preimage and digest. The
tampering test independently changes kind, TEST_CODE nonce, root, preimage and
digest and requires failure before every open/read. These module-private tests
intentionally do not exercise a public root contract; Task 8 adds that contract
only after every listed symbol exists. A non-test compile/source contract
rejects root exports or public constructors for the recording resolver, test
target, authenticated token and resolved capabilities. The time test freezes an
authority `validated_at`, advances the test process clock before apply, and
requires the authorization row, canonical authorization observation time,
schedule eligibility input and public outcome to retain that exact timestamp.
The command-consumption test independently changes raw process time, subject,
service, mechanism and nonce inputs and proves none can enter except by
consuming the attestation.

- [ ] **Step 3: Freeze the production PAM adapter contract**

The Task 8 binary parses syntax only into
`ProductionRetryAuthorizationRequest` and calls
`authorize_delivery_retry_production(request)`. It does not import, supply or
construct an authority, target, resolver, authenticated token, coordinator or
append port. The library-owned production sequence:

1. parses `--decision`, `--operator`, `--reason`, `--evidence-file`;
2. rejects unless `MONITOR_AUTH_REQUIRED=1`;
3. loads auth config and rejects operator inequality with
   `load_auth_config().expected_operator` before TTY/PAM;
4. rejects non-TTY before opening any artifact;
5. calls `authenticate_monitor_operator(claimed_operator)`, consumes the
   returned opaque `OperatorAuthAttestation` to construct/validate private
   `AuthenticatedRetryOperator`, and copies its authority-owned
   subject/service/mechanism/nonce/`validated_at`;
6. internally constructs the private fixed manifest-root production resolver
   only after successful authentication;
7. reads the explicit evidence file through that resolver only after
   authentication;
8. invokes the coordinator command; and
9. prints identities/hashes/ref only.

The CLI exposes no test-target/root/time flag. The TEST_CODE attestation
factory, recording resolver and test target compile only under `#[cfg(test)]`;
production code cannot name, construct or select them. Task 6 does not create
or register that CLI.

- [ ] **Step 4: Implement manual row insert/recovery**

`authorize_retry`:

- requires current definite `RejectedDurable`;
- obtains manual `authorized_at` solely from the private authenticated
  operator's `validated_at`, uses that exact value for canonical observation
  time and schedule eligibility, and rejects any internal disagreement;
- validates current appended rejection disposition;
- rejects all Uncertain/terminal/exhausted states;
- recomputes `retry_namespace_sha256` from the private resolved target's
  `namespace_preimage`, requires equality with its retained
  `namespace_sha256`, and includes both exact values in the deny-unknown
  canonical authorization bytes before deriving the stable identity;
- inserts `PendingAppend/PendingApply` using the unique rejection constraint;
- drains exact append/event/apply recovery and creates the unique active
  companion binding; and
- returns stored identity/state/ref for byte-identical replay.

Different bytes under the same unique rejection return
`ImmutableAppendConflict`.

- [ ] **Step 5: Freeze safe process help isolation for Task 8**

Freeze three negative process cases—authentication unset/zero, non-TTY and
operator mismatch—and the protected-artifact snapshot contract. Task 8 creates
the binary, process references and tests together. Task 6 adds none of them.

- [ ] **Step 6: Declare the private module and run focused library tests**

Add only `mod retry_command;` to `src/durable_delivery/mod.rs`. Use
module-private paths until Task 8. No `pub use`, binary stanza or integration
process reference is allowed in this task.

Run:

```bash
cargo test --lib durable_delivery::tests::br192_test_attestation_can_mutate_only_nonce_bound_test_target -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_production_target_rejects_test_attestation_before_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_manual_authorization_identical_replay_returns_stored_identity -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_manual_authorization_rejects_all_three_uncertain_states -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_production_authority_rejects_before_open_when_not_authenticated -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authenticated_retry_operator_validates_kind_timestamp_and_session_evidence -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_resolver_is_unreachable_before_authentication_and_target_compatibility -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_resolved_production_target_cannot_hold_test_authority_root_or_capability -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_manual_target_namespace_is_derived_after_auth_before_open_and_persisted -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_manual_namespace_tampering_and_production_test_mismatch_fail_before_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_non_test_public_surface_has_no_injectable_authority_resolver_or_target -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_production_entry_owns_pam_and_fixed_manifest_resolver -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_time_equals_authority_validated_at_even_when_command_is_delayed -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_request_has_no_caller_controlled_timestamp -- --exact --test-threads=1
cargo test --lib auth::operator::tests::br192_operator_auth_attestation_is_opaque_and_created_only_after_pam_success -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_command_consumes_attestation_without_constructing_time_subject_service_or_nonce -- --exact --test-threads=1
cargo test --lib auth::operator::tests::br192_require_monitor_operator_auth_delegates_and_discards_attestation -- --exact --test-threads=1
```

Expected: all library tests pass inside their nonce-bound TEST_CODE roots and
all protected production artifacts are unchanged. Public-root compile and
process isolation remain deliberately absent until Task 8.

- [ ] **Step 7: Commit**

```bash
git add src/auth/operator.rs src/durable_delivery/retry_command.rs src/durable_delivery/coordinator.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs
git commit -m "feat: authenticate BR-192 retry authorization"
```

### Task 7: Define the read-only evidence library privately

**Files:**

- Create `src/durable_delivery/retry_evidence.rs`
- Modify `src/durable_delivery/mod.rs`
- Modify `src/durable_delivery/tests.rs`

- [ ] **Step 1: Define the complete verifier API**

Create these symbols in the private `retry_evidence` module:

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

Both verifier functions use the existing public
`stock_analysis::durable_delivery::Result<T>` alias, whose error type is the
existing public `DurableDeliveryError`. No private verifier error may appear in
either signature, and no second public error enum, `Display` parsing, `String`
conversion or generic-error downgrade may mediate these failures.

The public verifier always resolves only fixed manifest-root production
read-only authorities. No public DTO or function accepts a root, target or test
selector. The cfg(test)-only target and verifier require an identical, trimmed
`TEST_CODE_*` nonce and isolated root, have at most `pub(crate)` visibility and
are unreachable from external integration tests and non-test binaries.
`require_count` must be in `1..=MAX_RETRY_EVIDENCE_RESULTS`. The library rejects
zero and 257+ before authority resolution/open with
`DurableDeliveryError::RetryEvidenceQueryCountOutOfRange {
requested, min: 1, max: 256 }`; the CLI applies the same closed range and the
library repeats the validation. Returned rows contain only the persisted
redacted exact join.
Each row's `durable_push_kind` equals the request, `verified_retry_count` equals
the final validated vector length and is at least `require_count`, and
`exact_join` is emitted as literal `true` only after the complete join passes.
No rendered text, raw account data, path or credential is returned.

Stream artifact candidates; never collect an unbounded path or result vector.
Validate each complete join into a bounded map keyed by the full tuple below.
An artifact replay with complete canonical bytes and hashes byte-identical to
the retained entry is accepted, deduplicated and does not increase the verified
count; it is not a duplicate-match error. The same logical tuple with any
different canonical bytes or hash returns
`DurableDeliveryError::RetryEvidenceConflictingDuplicate {
logical_tuple_sha256, canonical_bytes_mismatch,
canonical_hash_mismatch }`. The hash is only the lowercase domain-separated
SHA-256 of the complete logical tuple. Implement exactly the Task 1
`RetryEvidenceLogicalTuplePreimageV1` field order, validation, compact UTF-8
JSON encoding, literal
`stock_analysis.durable_delivery.br192.retry_evidence_logical_tuple.v1\0`
domain and frozen golden digest; rebuild both the typed map key and hash from
that same validated preimage and require decode→reserialize byte equality.
Exact flags are bytes-only
`(true,false)`, hash-only `(false,true)` and both `(true,true)`;
`(false,false)` is rejected before the conflicting-duplicate variant can be
constructed. Duplicate authority rows within one join remain an
ambiguous-match error. Inserting the 257th distinct complete join returns
`DurableDeliveryError::RetryEvidenceResultBoundExceeded {
max: 256, attempted_distinct_count: 257 }`, emits no partial JSON and performs
no write. After exhausting the stream, sort the at-most-256 final rows with
SQLite binary semantics by persisted non-null, non-empty
`decision_identity ASC, retry_ordinal ASC, attempt_identity ASC,
sink_result_identity ASC, authorization_identity ASC,
rejection_disposition_identity ASC`. The last identity is the total tie-break.
Filesystem/JSONL discovery order, SQL planner order and hash-map iteration are
never observable. Assign `verified_retry_count` only after this sort, then
serialize the vector directly.

- [ ] **Step 2: Write module-private RED tests**

Add:

```rust
#[test]
fn br192_retry_evidence_dtos_are_deny_unknown_and_redacted() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_verified_retry_evidence_and_cli_share_exact_required_fields() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_non_test_evidence_surface_exposes_only_production_verifier() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_test_evidence_verifier_exact_join_is_read_only_and_nonce_bound() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_verifier_accepts_and_deduplicates_byte_identical_artifact_replay() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_verifier_rejects_zero_conflicting_duplicate_mismatch_and_write_capable_target() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_query_count_bounds_apply_before_authority_open() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_stream_rejects_257th_distinct_join_without_partial_output() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_query_count_zero_returns_exact_typed_variant_before_authority_open() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_query_count_257_returns_exact_typed_variant_before_authority_open() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_logical_tuple_hash_matches_frozen_golden_and_recomputes() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_logical_tuple_hash_rejects_domain_schema_field_order_and_encoding_mutations() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_conflicting_canonical_bytes_returns_exact_typed_variant() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_conflicting_canonical_hash_returns_exact_typed_variant() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_conflicting_canonical_bytes_and_hash_returns_exact_typed_variant() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_conflicting_duplicate_zero_flags_rejected_before_variant_construction() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_evidence_257th_distinct_returns_exact_typed_variant_without_partial_output() { panic!("BR-192 RED: named contract is not implemented"); }
```

Run those seventeen exact commands from Step 4 now. Expected: each reports
`running 1 test` and fails before implementation.

- [ ] **Step 3: Implement the private read-only join**

The library implementation uses read-only SQLite flags, counted JSON
pending/commit/audit parsing with `deny_unknown_fields`, and the exact
decision/attempt/authorization/schedule/ownership/result/audit join frozen in
the design.

Task 7 adds only `mod retry_evidence;`; it adds no `pub use`, CLI source,
`[[bin]]`, `CARGO_BIN_EXE_*` reference or external integration test.

- [ ] **Step 4: Run private library tests and commit**

```bash
cargo test --lib durable_delivery::tests::br192_retry_evidence_dtos_are_deny_unknown_and_redacted -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_verified_retry_evidence_and_cli_share_exact_required_fields -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_non_test_evidence_surface_exposes_only_production_verifier -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_test_evidence_verifier_exact_join_is_read_only_and_nonce_bound -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_verifier_accepts_and_deduplicates_byte_identical_artifact_replay -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_verifier_rejects_zero_conflicting_duplicate_mismatch_and_write_capable_target -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_query_count_bounds_apply_before_authority_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_stream_rejects_257th_distinct_join_without_partial_output -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_query_count_zero_returns_exact_typed_variant_before_authority_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_query_count_257_returns_exact_typed_variant_before_authority_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_logical_tuple_hash_matches_frozen_golden_and_recomputes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_logical_tuple_hash_rejects_domain_schema_field_order_and_encoding_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_canonical_bytes_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_canonical_hash_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_canonical_bytes_and_hash_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_duplicate_zero_flags_rejected_before_variant_construction -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_257th_distinct_returns_exact_typed_variant_without_partial_output -- --exact --test-threads=1
```

Expected: each command runs exactly one test, all access stays below the
nonce-bound TEST_CODE root and no production artifact is opened or mutated.

```bash
git add src/durable_delivery/retry_evidence.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs
git commit -m "feat: define BR-192 retry evidence verifier"
```

### Task 8: Complete failure-path and physical-isolation regression

**Files:**

- Modify `src/durable_delivery/mod.rs`
- Modify `src/durable_delivery/tests.rs`
- Modify the private `src/durable_delivery/counted_producer_catalog.rs`
  created in Task 1; do not recreate it or move its opaque types
- Modify `src/bin/monitor/durable_delivery_runtime.rs`
- Modify `src/bin/monitor/main.rs`
- Modify `src/bin/monitor/review_batch.rs`
- Modify `src/bin/monitor/push_templates.rs`
- Modify `src/bin/monitor/notify.rs`
- Modify `src/bin/monitor/v14_adapter.rs`
- Create `src/data_gateway/capital.rs`
- Modify `src/data_gateway/mod.rs`
- Modify `src/lib.rs`
- Create `src/bin/authorize_delivery_retry.rs`
- Create `src/bin/verify_br192_retry_evidence.rs`
- Modify `Cargo.toml`
- Modify `Cargo.lock`
- Modify `tests/durable_delivery_counted_cutover.rs` created first in Task 1
- Create `tests/br192_counted_producer_catalog.rs`
- Create `tests/magic_market_release_revision.rs`
  - this fixed-HEAD-absent file is created in Task 8 before the dependency
    identity command runs and owns the exact BR-192/BR-198 14-direct/15-lock
    executable assertion
- Modify `tests/monitor_help_isolation.rs`
- Create `tools/release/disable_br192_periodic_retry.patch`
- Create `tools/release/verify_br192_forward_rollback.sh`

Steps 1 and 2 author the final RED regressions. Step 3 then applies the
deferred runtime recipe and lands the one atomic BR-192 root/CLI integration.
Existing unrelated root exports remain unchanged. Nothing in this task is
compiled or committed with a partial new BR-192 root surface;
Step 4 validates the complete atomic source state.

`tools/release/verify_br192_forward_rollback.sh` is not an accident-time
recipe. Task 8 creates it before GREEN and both Task 8 Step 4 and Task 9 Gate C
execute it. It accepts exactly one literal commit SHA whose tree is either the
fully staged Task-8 candidate tree or the accepted release tree; it must never
receive the pre-Task-8 branch HEAD. Task 8 creates the candidate object with
`git write-tree` + `git commit-tree` without moving the branch and later proves
the reviewed implementation commit has the same tree. The verifier creates an
isolated detached worktree at that SHA, and then:

1. requires exactly one `diff --git` line and the exact target
   `src/bin/monitor/main.rs` in
   `tools/release/disable_br192_periodic_retry.patch`;
2. rejects patch text or resulting diff that names catalog, R-09, BR-200,
   schema/UDF/manifest, dependency, reconciliation, audit, startup-cycle or
   generic-delivery authorities;
3. runs `git apply --check`, applies the patch and requires the resulting
   name-only diff to contain exactly `src/bin/monitor/main.rs`;
4. syntax-checks that the main-source occurrence count of
   `run_provider_free_retry_cycle` falls from exactly two accepted call sites
   (startup and periodic) to exactly one startup call site, while the startup
   prior-boot recovery block, the 15-row catalog, R-09 consumer, v6 schema,
   deterministic `sha256_hex`, BR-194 replay and four-stage retry-origin
   classifier identifiers remain present;
5. builds the rollback release monitor; and
6. in the patched tree, first proves exactly twelve listed `br192_br198_*`
   tests and exactly seven listed `br192_br200_r09_*` tests exist and then runs
   both groups plus `br192_br200_business_date_once_claim_prevents_second_r09_decision`;
7. runs the exact schema-v6 fresh/upgrade and v5-preservation tests, the full
   `br192_counted_producer_catalog` suite, and exact
   `br192_magic_market_release_revision_is_one_atomic_identity` test; and
8. runs exact recovery tests
   `br192_rollback_preserves_four_stage_retry_origin_reserved_recovery` and
   `br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable`
   plus exact startup-cycle test
   `durable_delivery_runtime::tests::br192_startup_and_periodic_paths_share_one_cycle_algorithm`.

The script exits non-zero on a stale patch, a second target, an extra semantic
edit, a build failure, zero selected tests or any recovery regression. It
always removes its isolated worktree through `git worktree remove` on exit and
never opens production data roots.

- [ ] **Step 1: Complete the failure matrix without duplicate declarations**

The exact same-cycle failure-finalization tests are owned only by
`src/bin/monitor/durable_delivery_runtime.rs` and were declared in the deferred
Task 8 runtime recipe (Step 2):
`br192_normal_error_after_claim_quarantines_same_cycle_attempt_before_failed`
and
`br192_caught_panic_after_claim_quarantines_same_cycle_attempt_before_failed`.
Task 8 completes those two bodies and runs their fully qualified commands; it
must not redeclare either bare name in
`tests/durable_delivery_counted_cutover.rs` or another harness.

Each fixture also includes a second appended-start-without-ownership attempt in
the same execution cycle. The finalizer must create `InterruptedUncertain`
ownership for that attempt, advance the claimed attempt's existing ownership,
append/ack all uncertainty before `Failed`, and make zero recovery sink calls.

Cover:

- pending authorization append/apply blocks before candidate query and writes
  `AuthorizationReconciliationBlocked`; after 4096 successful drain steps, an
  exact remaining immutable identity must instead pattern match
  `DurableDeliveryError::AuthorizationReconciliationBoundExceeded {
  max_steps: 4096, pending_authorization_identity }`;
- stale authorization disposition;
- active binding clear on new disposition/terminal with historical binding
  retained;
- policy mismatch;
- all typed budget/claim/head/cooldown/backoff deferrals;
- attempt exhaustion;
- schedule count whose `last_attempt_binding_identity` is null, points at
  another decision or does not match the exact retry ordinal;
- byte-identical logical cycle-event replay and a conflicting
  canonical-bytes/hash replay for the same cycle/scope/kind/ordinal slot;
- missing/duplicate global `Started` or conflicting/multiple terminal slots;
- cycle audit append failure before sink; after 4096 successful drain steps,
  an exact remaining immutable cycle-event identity must pattern match
  `DurableDeliveryError::RetryCycleAuditReconciliationBoundExceeded {
  max_steps: 4096, pending_cycle_event_identity }`;
- crash after admission commit/before prepare, followed by repeated startup
  reentry with the same binding/generation/ordinal/schedule and one sink call;
- prepare attempting to allocate an attempt, binding, generation or schedule
  change (must be structurally impossible and regression-tested);
- every `AppendedSinkAttemptStarted` identity/canonical/SHA/ref/generation/
  owner/positive-`i64` fence mismatch, plus any DTO/canonical/persisted-outbox/
  prepared `started_at` mismatch; no append-authority timestamp is trusted;
- pre-call crash after consumed ownership but before the external call,
  remote acceptance followed by crash before result persistence, same-process
  repeated execute and two-process pre-call CAS competition;
- successful execute returning
  `RetrySinkExecutionOutcome::Persisted(PersistedRetrySinkOutcome)` whose stored result
  identity, terminal decision state and `TerminalRecorded` ownership state join
  exactly; inject a `record_sink_result` transaction failure after the
  ownership pointer/state update but before the authoritative/non-late result
  insert and assert the whole transaction rolls back, the decision remains
  `AttemptInFlight`, ownership remains `Started` with a NULL terminal-result
  pointer, no result row remains, and recovery quarantines with zero resend;
- startup and JoinError quarantine of every prior-boot appended-start or
  `AttemptInFlight` attempt without a terminal authoritative result, with
  `ProcessInterruptedAfterSinkStart`, zero recovery sink calls and convergence
  through all three uncertainty stages;
- ordinary cycle error after claim and caught panic after claim, each proving
  the common failure finalizer quarantines every same-cycle appended-start,
  consumed/`Started` ownership or `AttemptInFlight` attempt without terminal
  result, append/acknowledges all uncertainty evidence before `Failed`, and
  makes zero recovery sink calls;
- fallible namespace/hash/boot validation before guard acquisition, definite
  identity-first begin rollback/readback producing the single-use
  input/ordinal/identity/witness-bound `NoRetryCycleCommitted` release proof,
  fresh transactional consumption that rederives every fact and rejects a
  concurrent insert/witness change, and commit-ambiguous plus unsafe post-claim
  exits retaining the latched guard;
- same-boot completion/failure append, acknowledgement and terminal-CAS
  failpoints followed by a current-identity exact safe-terminal resumer:
  while its exact CAS is paused the next begin has not run and the second
  `Started` count is zero; after terminalization exactly one later
  Running+Started commits. A wrong identity, same-boot `NotPrepared` or
  unresolved prior-boot row is not rewritten and the begin transaction returns
  definite `RetryCycleAlreadyRunning + NoRetryCycleCommitted` with zero writes
  and zero sink; its guard releases only after the exact proof is
  transactionally consumed;
- completion `Pending -> Appended -> Completed/Terminalized` and failure
  `Pending -> Appended -> Failed/Terminalized`, proving that once either
  terminal slot exists every error/panic/JoinError/reboot resumes only that
  exact kind;
- distinct writer/recovery child boots for all four recoverable terminal
  phases, plus the cloned-fixture corruption matrix over typed-preimage
  bytes/hash, envelope bytes/hash, payload identity and Failed binding; every
  corrupt recovery exits non-zero with no mutation, acknowledgement, opposite
  terminal slot or sink call;
- invalid/tampered namespace preimage/hash and non-canonical UTF-8 encoding;
- authenticated operator kind/timestamp/session-evidence mismatch and any
  attempt to resolve/open before authentication/target compatibility;
- absence of every constructible/injectable authority/resolver/target/session/
  resolved capability from the non-test public surface; the exported auth
  attestation is opaque and factory-origin-only, and the production CLI calls
  only `authorize_delivery_retry_production` with a timestamp-free request;
- absence of any public evidence target/root/test selector; the public verifier
  is production-only, the positive TEST_CODE fixture is a library unit test,
  and both public/private-test verifier signatures return only the existing
  public `durable_delivery::Result<T>`/`DurableDeliveryError` channel;
- a direct missing-decision admission returns typed
  `DurableDeliveryError::DecisionNotFound` before audit/transaction, while the
  candidate query cannot return a missing identity;
- delayed manual command execution preserving exact authority
  `validated_at` as persisted/canonical/schedule/outcome `authorized_at`;
- production resolved target containing any test authority/root/capability,
  manual namespace derivation occurring before authentication or after an
  open/read, or canonical authorization evidence not binding the exact resolved
  namespace preimage and digest;
- compile-only import of the complete retry runtime/CLI contract from the
  public durable-delivery root, with every private-module import rejected;
- pending/missing/mismatched `SinkAttemptStarted` append evidence before sink;
- all three start-audit-unavailable reason codes on both sides of the persisted
  boundary: pre-start preserves byte-identical authority rows/hashes/pending
  bytes, while acknowledged-start and consumed-ownership reject that exception
  and complete ordinary quarantine before `Failed`, with zero recovery sink;
- disposition/task/cycle append failure after sink;
- sink timeout/transport/cancel/write-after-loss;
- caught runner panic, runner JoinError recovery and next-startup orphan
  recovery;
- missing/empty/ASCII-whitespace boot identity rejection before cycle insert,
  exact persistence of the explicit pre-open boot identity, same-boot
  safe-terminal total-order resumption, global transactional `Running`
  exclusion and prior-boot `Running -> Failed` convergence;
- async cancellation;
- empty and space/tab/LF/CR-only immutable refs in Rust and SQLite;
- byte-identical and conflicting manual replay;
- all three Uncertain states; and
- production/test target mismatch;
- R-09 source business date expiry immediately before, exactly at and after
  next Shanghai midnight; equality/later creates one audited terminal
  `ExpiredFreshness`, invalidates active authority, cannot be manually revived
  and leaves provider/renderer/sink counters zero;
- `br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297`,
  `br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink`,
  `br192_br198_same_day_1535_boundary_precedes_terminal_preflight`, and
  `br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry`,
  plus independent capture-before-request, Shanghai-midnight crossing,
  invalid-provider-capture-timestamp and prior-date-initial-vs-retry-expiry
  tests;
- `br192_br200_r09_delivered_preflight_precedes_permit_gateway_renderer_and_sink`,
  independent Delivered-missing-hydration, Rejected/Uncertain terminal and
  nonterminal-retryable mapping tests,
  `br192_br200_r09_corrupt_or_ambiguous_authority_fail_closed`,
  `br192_br200_r09_no_occurrence_orders_preflight_then_permit_then_provider_then_renderer_then_sink`,
  `br192_br200_r09_startup_barrier_failure_is_provider_free`, and
  `br192_br200_business_date_once_claim_prevents_second_r09_decision`;
- exact R-09 ordered rule-vector persistence/hydration and static rejection of
  `BannerCtx`, banner text, AccountMode or broker authority in the SourceOnly
  dispatcher;
- expiry/start SQLite total-order races in both directions: expiry-first blocks
  Pending/Appended start plus ownership and converges its outbox, while even a
  Pending start-first writes no expiry row and routes to uncertainty; no
  Pending expiry authority can be stranded or superseded;
- final pre-call freshness immediately before, exactly at and after expiry,
  including a claim won before midnight and execution resumed after midnight;
  equality/later transaction A must atomically persist the exact private
  observation, `FreshnessExpiredBeforeExternalCall` ownership and Pending
  expiry, durably recount existing same-cycle terminal results, retain
  `Indeterminate/NULL` when another `Started|InterruptedUncertain` ownership
  remains or otherwise restore `Confirmed(n)`, and keep the external sink
  counter zero. Both branches cover zero and two prior results, orphan result,
  orphan `TerminalRecorded`, duplicate result, mismatched
  `terminal_sink_result_identity`, a second authoritative result, a result on
  non-terminal ownership, mixed ambiguous ownership, stale CAS, missing/double
  results and crash recovery; every non-bijective case rolls back. After exact append/
  acknowledgement, transaction B must complete or idempotently recover the
  effective expired terminal projection while leaving the fixed-v5 attempt row
  `AttemptInFlight`. Cover result-first, result-between-members and
  result-after-commit interleavings, transaction-A rollback, crash between A/
  append/B, candidate/orphan/evidence exclusion, and rejection of any partial
  `AlreadyTerminalized` projection. Mutating the observation, canonical bytes,
  ownership, expiry, result absence or terminal projection must fail closed;
- complete 15-kind caller/catalog classification, all fourteen disabled
  counted-specific loaders returning before acquisition, and the sole R-09
  producer obtaining the exact seam-bound permit before gateway acquisition;
- evidence `require_count` 0 and 257 pattern match
  `RetryEvidenceQueryCountOutOfRange { requested, min: 1, max: 256 }` before
  authority open; canonical-bytes-only, canonical-hash-only and both-changed
  conflicts pattern match `RetryEvidenceConflictingDuplicate` with the exact
  recomputed lowercase logical-tuple hash and flags `(true,false)`,
  `(false,true)` and `(true,true)` respectively; `(false,false)` is rejected
  before variant construction; and the 257th distinct complete join pattern
  matches `RetryEvidenceResultBoundExceeded {
  max: 256, attempted_distinct_count: 257 }` with zero partial output/write.

Task 2 adds these exact non-sentinel RED bodies before any Task 8 export. The
first body fails only when the frozen root surface/methods are absent. The two
library bodies use the Task-2 crate-private fixture helpers whose implementation
constructs all six cases independently; the helpers are not production/root
exports.

```rust
#[test]
fn br192_durable_delivery_root_reexports_complete_retry_runtime_and_cli_contract() {
    #[allow(unused_imports)]
    use stock_analysis::durable_delivery::{
        DurableDeliveryCoordinator, ImmutableAppendPort, AuthoritativeSink,
        AuthoritativeSinkPort, DecisionState, CountedProducerPermit,
        CountedProducerAttestation, CountedProducerDenied,
        acquire_counted_producer_permit, MAX_AUTOMATIC_RETRY_ATTEMPTS,
        RETRY_BACKOFF_SECONDS, RetryAuthorizationSource, RetryCandidate,
        ExpirableRetrySchedule, CompleteRetryExpirySnapshot,
        PreparedRetryExpiredFreshness, PreparedRetryExpiryUncertainty,
        RetryExpiryUncertaintyReason, RetryExpiryPreparationOutcome,
        RetryExpiryDisposition, RetryExpiryTerminalKind,
        RetryScheduleTerminalState, RetryCandidateSnapshot, RetryCycleEvidence,
        RetryCycleSinkCalls, RetryCycleFailureReason, RetryCycleOperation,
        RetryCycleFailure, RetryCycleTerminalPhase, RetryCycleBeginOutcome,
        NoRetryCycleCommitted, RetryNamespaceKind, RetryNamespaceHashPreimageV1,
        retry_namespace_sha256, RetryDeferral, RetryIneligibility,
        RetryAdmission, PreparedRetryAttempt, RetryAttemptPreparationOutcome,
        AppendedSinkAttemptStarted, RetrySendOwnershipState,
        SinkExecutionPermit, RetrySinkClaimOutcome, PersistedRetrySinkOutcome,
        RetrySinkExecutionOutcome, OperatorAuthAttestation,
        authenticate_monitor_operator, ProductionRetryAuthorizationRequest,
        ProductionRetryAuthorizationOutcome, authorize_delivery_retry_production,
        MAX_RETRY_EVIDENCE_RESULTS, RetryEvidencePushKind, RetryEvidenceQuery,
        VerifiedRetryEvidence, verify_br192_retry_evidence,
    };
    assert_eq!(MAX_RETRY_EVIDENCE_RESULTS, 256);
    let _free_functions = (
        acquire_counted_producer_permit,
        retry_namespace_sha256,
        authenticate_monitor_operator,
        authorize_delivery_retry_production,
        verify_br192_retry_evidence,
    );
    let _coordinator_methods = (
        DurableDeliveryCoordinator::prepare_retry_attempt,
        DurableDeliveryCoordinator::reconcile_prepared_retry_attempt_audit,
        DurableDeliveryCoordinator::validate_appended_sink_attempt_started,
        DurableDeliveryCoordinator::claim_retry_sink_execution,
        DurableDeliveryCoordinator::execute_prepared_retry_sink,
    );
}

#[test]
fn br192_retry_cycle_failure_typed_constructors_bind_exact_reason_fields_and_hashes() {
    let cases = retry_cycle_failure_constructor_contract_cases();
    assert_eq!(cases.len(), 6);
    for case in cases {
        assert_eq!(case.failure.reason(), case.expected_reason);
        assert_eq!(case.failure.typed_fields_sha256(), case.recomputed_sha256);
    }
}

#[test]
fn br192_retry_cycle_failure_constructors_reject_wrong_variant_invalid_digest_and_unknown_operation() {
    assert_retry_cycle_failure_wrong_variant_rejected();
    let invalid_digests = vec![String::new(), "a".repeat(63), "A".repeat(64)];
    for invalid in &invalid_digests {
        assert_all_retry_cycle_failure_sha_constructors_reject(invalid);
    }
    assert!(serde_json::from_str::<RetryCycleOperation>("\"unknown_operation\"").is_err());
}
```

Step 2 must add the real (non-empty, non-sentinel) RED test
`br192_durable_delivery_root_reexports_complete_retry_runtime_and_cli_contract`
before the Task 8 root export. Its body imports and type-uses every frozen
manifest item only through `stock_analysis::durable_delivery`, asserts
`MAX_RETRY_EVIDENCE_RESULTS == 256`, and assigns all coordinator method items to
their exact frozen function-pointer signatures. Its only valid RED cause is the
missing Task 8 root symbols; `0 tests`, an empty body, a panic sentinel or an
unrelated compile error is invalid evidence.

The compile-contract test imports the frozen Task 1 manifest only through
`stock_analysis::durable_delivery`; runtime code and both CLIs must compile
without a private-module import. It also type-checks the exact
prepare/reconcile/validate/claim/execute method signatures through the imported
`DurableDeliveryCoordinator`; method names are never separate root exports.
This task also reruns Task 4's exact
`br192_persisted_retry_sink_outcome_joins_terminal_result_and_ownership` and
`br192_ownership_pointer_update_then_result_insert_failure_rolls_back_and_quarantines_without_resend`
tests against the integrated public graph; it does not redefine them.

Step 2 also adds real (non-empty, non-sentinel) RED bodies for
`br192_retry_cycle_failure_public_typed_constructors_compile_from_monitor_boundary`,
`br192_retry_cycle_failure_typed_constructors_bind_exact_reason_fields_and_hashes`
and
`br192_retry_cycle_failure_constructors_reject_wrong_variant_invalid_digest_and_unknown_operation`.
The first invokes all six constructors through root-only imports and reads only
`reason()`/`typed_fields_sha256()`. The second independently constructs the six
ordered domain preimages, recomputes every digest and compares the closed
reason/digest. The third passes a wrong error variant to the start-audit
constructor, checks empty/63-byte/uppercase-64-byte digest rejection for every
SHA constructor, and checks serde rejection of an unknown
`RetryCycleOperation`. Their only valid RED cause is the missing Task 8 root
surface.

The first test is owned by
`src/bin/monitor/durable_delivery_runtime.rs`: it imports
`DurableDeliveryError`, `RetryCycleFailure` and `RetryCycleOperation` only
through `stock_analysis::durable_delivery`, calls all six exact constructors
with their frozen argument types and can observe only `reason()` and
`typed_fields_sha256()`. The second and third are owning library tests: they
pattern-match every constructor's fixed reason/private preimage through
crate-private test support, recompute the exact reason-domain digest, reject a
non-start-audit error passed to the first constructor, reject malformed/
uppercase digests and reject an unknown `RetryCycleOperation` token during
deserialization. The public `RetryCycleFailure` item also has a
`compile_fail` doctest proving external code cannot construct a struct literal,
read a private field or supply raw canonical bytes.

Each test asserts state, reservation generation, sink count, provider count,
renderer count, pending/appended immutable records and production snapshot.

Before the matching production edits, Task 8 adds the following exact RED
bodies to `tests/durable_delivery_counted_cutover.rs`; each command must report
`running 1 test` and the named BR-192 sentinel. A compile error, zero selected
tests or an empty/pass-through body is invalid RED evidence:

```rust
#[test]
fn br192_fixed_head_inventory_classifies_every_counted_entry_call() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_same_day_1535_boundary_precedes_terminal_preflight() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_host_tz_cannot_change_shanghai_review_date_or_1535_boundary() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_capture_before_trusted_request_start_fails_pair_before_durable_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_capture_after_trusted_request_completion_fails_pair_before_durable_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_capture_raw_bytes_round_trip_and_mutation_rejects_pair_before_durable_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_capture_before_request_date_fails_pair_before_durable_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_capture_crosses_shanghai_midnight_fails_pair_before_durable_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_invalid_provider_capture_timestamp_fails_pair_before_durable_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br198_prior_date_initial_admission_ignores_retry_expiry_but_retry_rejects() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_delivered_preflight_precedes_permit_gateway_renderer_and_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_delivered_missing_hydration_is_provider_free_retryable() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_rejected_and_uncertain_are_provider_free_terminal() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_nonterminal_is_provider_free_retryable() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_corrupt_or_ambiguous_authority_fail_closed() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_no_occurrence_orders_preflight_then_permit_then_provider_then_renderer_then_sink() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_r09_startup_barrier_failure_is_provider_free() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_br200_business_date_once_claim_prevents_second_r09_decision() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_r09_transition_and_hydration_bind_exact_ordered_rule_ids() { panic!("BR-192 RED: named contract is not implemented"); }
#[test]
fn br192_r09_sourceonly_dispatch_has_no_banner_account_or_broker_authority() { panic!("BR-192 RED: named contract is not implemented"); }
```

The owning library RED body
`br192_rollback_preserves_four_stage_retry_origin_reserved_recovery` uses the
same sentinel and is created with the other Task-4 library tests.

- [ ] **Step 2: Add no-resend regression**

For each of:

```text
AttemptInFlight (consumed send ownership; never ordinary resend)
RejectedAuditPending
RejectedTaskTransitionPending
RejectedDurable (without a unique active binding and appended/applied authorization event)
UncertainAuditPending
UncertainTaskTransitionPending
UncertainManualReview
AcceptedAuditPending
AcceptedTaskTransitionPending
Delivered
ManualRejectedAuditPending
ManualRejectedTaskTransitionPending
ManualResolvedRejected
```

run two cycles and assert zero retry sink calls, zero provider/renderer calls and
one durable typed deferral/ineligibility audit where the state is observed.
Separately, the authorized `RejectedDurable` success tests remain the only
matrix row allowed to reacquire and call the sink.

Before Step 3, prove the four real root/constructor tests are RED with the same
commands later used for GREEN:

```bash
cargo test --lib durable_delivery::tests::br192_durable_delivery_root_reexports_complete_retry_runtime_and_cli_contract -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_retry_cycle_failure_public_typed_constructors_compile_from_monitor_boundary -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_failure_typed_constructors_bind_exact_reason_fields_and_hashes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_failure_constructors_reject_wrong_variant_invalid_digest_and_unknown_operation -- --exact --test-threads=1
```

Each command must select exactly one test and fail only because Step 3 has not
published the complete root surface. `0 tests`, an empty body, a panic sentinel
or an unrelated compile failure is invalid RED evidence. Step 3's single atomic
root export/runtime integration makes these same four bodies GREEN; Step 4
reruns them unchanged.

Pending authorization is not a matrix admission row: its dedicated test proves
candidate-query count zero, `AuthorizationReconciliationBlocked` appended and
cycle `Failed`. A prepared retry attempt with pending start-event evidence is a
separate zero-sink seam test. A prior-boot appended start or consumed ownership
without a terminal result is likewise not ordinary `Reserved` recovery: its
dedicated startup/JoinError tests require uncertainty quarantine and zero sink
calls even when the process may have died before the actual call.

- [ ] **Step 3: Integrate the runtime, both CLIs and exact root contract**

First land the sole real consumer atomically with its guard:

1. create `tests/magic_market_release_revision.rs`, then replace fixed HEAD's
   path-only Magic TDX declaration and install the exact
   fourteen-direct/fifteen-lockfile same-revision dependency set frozen above,
   update `Cargo.lock`, and assert every Magic package resolves to rev
   `5f1ce93656a55854c844065390520cd4aecd9a14` and version `0.2.0` before
   compiling any gateway code; assert `magic-market-transport` is transitive
   only and that no sixteenth Magic package is present by running
   `cargo test --test magic_market_release_revision br192_magic_market_release_revision_is_one_atomic_identity -- --exact --test-threads=1`
   and requiring `running 1 test`;
2. implement `CapitalDataGateway::provider_top_n_pair` from the pinned
   magic-market router/core/composition crates, accepting the exact validated
   review-calendar business date and returning one complete, non-empty pair
   whose every provider `f297` equals that date, or typed `Failed`. Empty
   (including a provider-described verified-empty result), partial, stale or
   malformed batches are `Failed`, never `NoData`; they create zero durable
   binding and zero sink calls;
3. before any permit/provider/renderer/sink access, run the BR-194/BR-198
   static preflight: future date -> `provider_top_n_future_date`; current date
   before 15:35 -> `ExpectedWait`; current date at/after 15:35 or the
   dispatcher-resolved latest-settled prior review-calendar business date ->
   runnable, never arbitrary replay. Construct the observation through an
   explicit Asia/Shanghai conversion at the monitor context boundary, never
   host-local `chrono::Local`; freeze trusted `request_started_at` and
   `capture_completed_at` times,
   preserve the provider capture timestamp raw bytes, and reject unless
   `request_started_at <= provider_captured_at <= capture_completed_at` plus
   exact business-date/midnight validation. Build the two exact
   `ProviderCaptureEvidenceV1` records and freeze their raw arrays plus
   domain-separated hashes into the pair binding before any durable prepare;
   re-hash on read and reject normalization or byte mutation. Then run accepted BR-200
   `inspect_review_task_occurrence`; `Some(evidence)` maps only through
   `review_outcome_from_existing_durable`, while error/missing hydration/
   corrupt/ambiguous authority fails closed. Only exact `None` continues;
4. integrate Task 1's existing library-owned private
   `src/durable_delivery/counted_producer_catalog.rs` without moving or
   duplicating its opaque permit API, acquire the permit for
   `push_templates::dispatch_r09_provider_top_n_outcome` before calling the
   gateway, and consume it into `CountedDeliveryBinding::new_permitted`;
   in this same atomic source state change BR-200's closed
   `ReviewTaskProductionCapability` mapping for R-09 from
   `DisabledNoProducer` to `EnabledSourceOnly`. The checker/test must reject a
   capability-only, catalog-only or producer-only mutation before provider I/O;
5. derive exact first-next-Shanghai-midnight `expires_at` from the requested
   review-calendar `source_business_date`, render once, bind
   source date/expiry/batch/order/hash/task identity plus all four producer-
   attestation fields into one immutable counted binding and call only the
   permitted private envelope path;
6. preserve fixed HEAD's existing single `ReviewTask::R09`, its exact position
   in `ReviewTask::ALL` after R08 and before A10, label `R-09` and `SourceOnly`
   classification without adding or duplicating any of them; preserve the
   static-date and durable-preflight ordering above and `--test --review`
   dual-disable before durable/provider access, and invoke
   `dispatch_r09_provider_top_n_outcome(business_date, observed_at)` exactly
   once from the central `dispatch_post_session_review` SourceOnly phase;
   merge its typed result in stable `ReviewTask` order with duplicate rejection.
   Its signature and body must contain no `BannerCtx`, banner text,
   `current_banner`, AccountMode or broker snapshot authority;
7. guard `notify::{push_governor,push_governor_v3,
   push_governor_v3_with_sub_kind}`, `push_templates::{dispatch,
   dispatch_outcome}` and runtime `{deliver_counted_binding,deliver_envelope}`;
8. put catalog checks before every disabled counted-specific R-04/R-08/T0/
   Paper/other loader or startup call, not merely at the eventual sink; and
9. make `tests/br192_counted_producer_catalog.rs` build the syntax/multiline-
   aware all-15 caller/template-ID/label inventory required by design §1.2.

That integration test owns this exact declaration once:

```rust
#[test]
fn br192_r09_capability_catalog_and_producer_enable_atomically() {
    panic!("BR-192 RED: R-09 capability, catalog and producer are not atomic");
}
```

Before production edits it must run exactly one failing test; after the atomic
Task-8 edit it proves all three authorities are present and consistent and that
each single-authority mutation exits non-zero with zero provider calls.

No dirty-worktree candidate is accepted by existence alone. The atomic change
must prove fourteen disabled counters stay zero and R-09's enabled path has one
real gateway result, one binding and one counted sink attempt.
It must also prove manual/automatic authorization and retry reject every
legacy-v5 or non-R-09 decision before authorization append and that the active
expiry drain reaches an empty fixed point without hiding a row.
The R-09 integration tests also prove the central strict-review call path,
SourceOnly sequencing, exact once-only merge, all BR-194 finite/non-empty dual-
batch/provider/source/metric/unit/date/order checks, and zero provider calls in
future-date, same-day-before-15:35, durable-preflight-terminal/error/corrupt and
dual-disable paths. They prove a prior review-calendar business date uses that
exact date, can perform initial acquisition after its retry expiry, and any
subsequent rejection has zero retry without extending expiry. BR-192 Gate B is
not accepted until BR-200 Gate C is accepted in an earlier isolated
progression; BR-198's implementation and evidence are accepted only as part of
this BR-192 atomic Gate-B slice.

Use Task 7's already-tested private `verify_br192_retry_evidence` library API.
The authorization binary's complete authority-bearing path is only:

```rust
use clap::Parser;
use stock_analysis::durable_delivery::{
    authorize_delivery_retry_production,
    ProductionRetryAuthorizationRequest,
};

#[derive(Parser)]
struct Args {
    #[arg(long = "decision")]
    decision_identity: String,
    #[arg(long = "operator")]
    operator_identity: String,
    #[arg(long)]
    reason: String,
    #[arg(long = "evidence-file")]
    evidence_path: PathBuf,
}

fn run() -> Result<(), String> {
    let args = Args::parse();
    let outcome = authorize_delivery_retry_production(
        ProductionRetryAuthorizationRequest {
            decision_identity: args.decision_identity,
            operator_identity: args.operator_identity,
            reason: args.reason,
            evidence_path: args.evidence_path,
        },
    )
    .map_err(|error| error.to_string())?;
    print_redacted_authorization_outcome(&outcome);
    Ok(())
}
```

`print_redacted_authorization_outcome` exhaustively matches the two outcome
variants. `Authorized` prints only its six redacted persisted fields;
`NoLongerEligible` prints only decision identity and typed ineligibility. It is
defined in the binary and performs no authority, resolution, database, append
or evidence operation. There is no alternate run function or hidden
target/root/time flag. Every imported authorization symbol appears in the
frozen root manifest.

The binary owns a checked-in structured unit test:

```rust
#[test]
fn br192_authorization_cli_request_surface_is_timestamp_and_capability_free() { panic!("BR-192 RED: named contract is not implemented"); }
```

It uses `clap::CommandFactory` to assert the exact argument IDs/long flags
`decision,operator,reason,evidence-file`, constructs the exact
`ProductionRetryAuthorizationRequest` literal used by `run`, serializes that
request to a JSON object, and asserts its exact keys are
`decision_identity,operator_identity,reason,evidence_path`. This compile-time
literal plus structured key comparison rejects a future request
`authorized_at`, target/root, authority, resolver, coordinator, append port,
evidence bytes or test selector. It deliberately does not scan the whole
binary for `authorized_at`, because
`ProductionRetryAuthorizationOutcome::Authorized { authorized_at, .. }` is a required legitimate
redacted output field.

The production verifier binary exposes only:

```text
verify_br192_retry_evidence
  --date YYYY-MM-DD
  --push-kind ReviewProviderTopN
  --require-count <1..=256>
```

It builds `RetryEvidenceQuery` and calls
`verify_br192_retry_evidence(&query)`. It cannot name a target/root or import
the cfg(test)-only `TestRetryEvidenceTarget`/test verifier.
The structured Clap parser rejects `0` and `257` before `run` and therefore
before the production verifier can resolve/open an authority; direct library
callers are independently guarded by the same exact bounds.
On success it serializes the returned `Vec<VerifiedRetryEvidence>` directly,
without a binary-local wrapper or recomputed summary. Consequently every CLI
row contains the library-owned exact `durable_push_kind`,
`verified_retry_count` and `exact_join=true` fields required by Gate D.
Binary wiring and persisted join correctness are verified independently:
`br192_verify_evidence_cli_has_exact_production_arguments_and_library_call`
checks the binary's structured Clap surface plus its direct production-library
call, and
`br192_verify_evidence_cli_rejects_out_of_range_count_before_library_call`
checks the exact 0/257 errors plus accepted 1/256 boundaries, while
`br192_test_evidence_verifier_exact_join_is_read_only_and_nonce_bound` checks
the library's complete counted-artifact/SQLite join. No acceptance check
requires the binary call and deep join implementation to occur in the same
source file or within an arbitrary character window.

The binary owns each exact test once:

```rust
#[test]
fn br192_verify_evidence_cli_has_exact_production_arguments_and_library_call() { panic!("BR-192 RED: named contract is not implemented"); }

#[test]
fn br192_verify_evidence_cli_rejects_out_of_range_count_before_library_call() { panic!("BR-192 RED: named contract is not implemented"); }
```

Register it when the source exists:

```toml
[[bin]]
name = "authorize_delivery_retry"
path = "src/bin/authorize_delivery_retry.rs"

[[bin]]
name = "verify_br192_retry_evidence"
path = "src/bin/verify_br192_retry_evidence.rs"
```

In one final integration compile step:

1. create the production PAM `authorize_delivery_retry` adapter frozen in Task
   6 and the read-only verifier binary, and apply the deferred cancellation-safe
   runtime recipe above;
2. retain the private command/evidence owning modules from Tasks 6 and 7;
3. make one atomic edit to the existing root exports so the ordered BR-192
   contract is exactly the frozen Task 1 manifest; preserve all unrelated
   existing exports, do not duplicate already-public symbols, and expose no
   partial new BR-192 set before this point;
4. register both `[[bin]]` stanzas;
5. add both `CARGO_BIN_EXE_authorize_delivery_retry` and
   `CARGO_BIN_EXE_verify_br192_retry_evidence` process references and the
   frozen authorization negative-path tests to
   `tests/monitor_help_isolation.rs`; and
6. add the root compile-contract test shown in Step 1.

The two binary sources, final atomic root-export edit, both stanzas, both process
references and compile-contract test land together. Coordinator operations
remain methods and are not re-exported as free functions.

The final integration process tests create unique nonexistent evidence paths
and syntactically valid dummy decision identities. They launch exactly
`env!("CARGO_BIN_EXE_authorize_delivery_retry")`, clear inherited
auth/test-target variables, and cover:

- `MONITOR_AUTH_REQUIRED` removed and set to `0`;
- `MONITOR_AUTH_REQUIRED=1` with a matching operator but non-TTY stdin/stdout;
  and
- `MONITOR_AUTH_REQUIRED=1` with configured
  `MONITOR_OPERATOR=TEST_CODE_EXPECTED_OPERATOR` but a different claimed
  operator, rejected before TTY/PAM.

Each asserts the exact pre-open failure instead of an evidence/database error
and compares metadata-only existence/type/filesystem identity for the
production DB/journal/WAL/SHM, push log, immutable audit, event audit and event
bus roots. The binary `--help` path exits zero without mutation. Positive
mutation remains only the nonce-bound library TEST_CODE test.

It has no database, push-log or namespace path flags. For every counted
push-log `.json` candidate it:

1. deserializes the pending/commit schemas with `deny_unknown_fields`;
2. filters the exact `durable_push_kind` JSON value, never rendered text;
3. verifies pending bytes/hash, commit marker and exact counted delivery audit;
4. opens the durable database read-only and exactly joins the terminal decision
   through immutable `retry_attempt_bindings` to the historical rejection
   disposition, appended/applied authorization plus appended `Applied` event,
   binding generation, schedule `last_attempt_binding_identity` and matching
	   retry ordinal, reservation generation/fence owner, the exact retained
	   `retry_send_ownership` row and its legal `Started -> TerminalRecorded`
	   transition, sink terminal and all immutable state/cycle refs;
   and
5. prints redacted JSON hashes/counts only.

Step 5 serializes the exact library result objects, including
`durable_push_kind`, final `verified_retry_count` and `exact_join=true`; it does
not define a second CLI result struct.

Zero matches, conflicting duplicates, ambiguous joins (including duplicate
authority rows within one join), missing or ASCII-whitespace-only refs, hash
disagreement, non-terminal state, any write-capable open, an out-of-range
`require_count`, or a 257th distinct complete join exits non-zero. An artifact
replay whose complete canonical bytes and hashes are byte-identical is instead
accepted and deduplicated into one result without increasing
`verified_retry_count`. Artifact paths and complete joins are streamed; at most
256 distinct validated result tuples are retained, and no bound error
serializes a partial vector.

Every verifier input/output DTO and nested enum derives `Serialize` and
`Deserialize`; all externally read JSON structs use `#[serde(deny_unknown_fields)]`.
The verifier opens SQLite with read-only flags and rejects a URI/target that
could create or mutate a database.

The `retry_evidence` library unit-test module builds a complete TEST_CODE
pending/commit/audit/database fixture and proves one successful exact join,
successful byte-identical artifact replay deduplicated to the same one result,
the frozen logical-tuple golden/recomputation vector plus exact domain/schema/
field/order/encoding mutations, and fail-closed cases for the same logical
tuple with changed canonical bytes only, retained hash only, both changed, or
the forbidden zero-flag branch, wrong `durable_push_kind`, tampered JSON,
missing commit, duplicate terminal and join mismatch. It snapshots protected
production metadata and proves the cfg(test)-only run does not touch production
content. No external integration test can construct or call the test target.
The integration process-isolation/help check launches exactly
`env!("CARGO_BIN_EXE_verify_br192_retry_evidence")` with `--help` after removing
inherited production/test target variables; it has no positive injected-target
case. Parser/join success and failure cases stay in the owning library unit-test
module and invoke only `verify_br192_retry_evidence_test` with its nonce-bound
TEST_CODE target. The production binary continues to expose no path or
test-target flag.

- [ ] **Step 4: Run all focused suites**

```bash
cargo test --lib durable_delivery -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_delivery_root_reexports_complete_retry_runtime_and_cli_contract -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_persisted_retry_sink_outcome_joins_terminal_result_and_ownership -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_ownership_pointer_update_then_result_insert_failure_rolls_back_and_quarantines_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_authorization_reconcile_bound_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_audit_reconcile_bound_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_derives_identity_before_running_check_and_binds_no_commit_proof -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_no_commit_branch_queries_zero_proposed_cycle_and_started_before_rollback -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_consume_no_commit_proof_rederives_next_identity_and_rejects_concurrent_change -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_no_commit_proof_rejects_domain_schema_field_order_encoding_and_witness_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_empty_running_check_atomically_persists_exact_ordinal_identity_and_started -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_ordinal_exhaustion_returns_exact_typed_variant_without_write -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_query_count_zero_returns_exact_typed_variant_before_authority_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_query_count_257_returns_exact_typed_variant_before_authority_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_logical_tuple_hash_matches_frozen_golden_and_recomputes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_logical_tuple_hash_rejects_domain_schema_field_order_and_encoding_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_canonical_bytes_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_canonical_hash_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_canonical_bytes_and_hash_returns_exact_typed_variant -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_conflicting_duplicate_zero_flags_rejected_before_variant_construction -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_257th_distinct_returns_exact_typed_variant_without_partial_output -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_failure_typed_constructors_bind_exact_reason_fields_and_hashes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_cycle_failure_constructors_reject_wrong_variant_invalid_digest_and_unknown_operation -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_retry_cycle_failure_public_typed_constructors_compile_from_monitor_boundary -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_retry_runtime_boundary_uses_only_durable_result_and_exact_failure_constructors -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_not_committed_propagates_exact_typed_error_after_proof_release -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_commit_ambiguous_propagates_exact_typed_error_and_latches_guard -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_guard_failures_return_exact_typed_variants_without_string_downgrade -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_completion_pending_blocks_new_started_until_exact_resume_terminalizes -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_failure_appended_blocks_new_started_until_exact_resume_terminalizes -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_begin_db_admission_rejects_any_global_running_before_second_started -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_same_boot_safe_terminal_selector_is_current_identity_exact_and_totally_ordered -- --exact --test-threads=1
cargo test --bin monitor durable_delivery_runtime -- --test-threads=1
cargo test --doc durable_delivery::RetryCycleFailure -- --test-threads=1
cargo test --test durable_delivery_counted_cutover -- --test-threads=1
cargo test --test br192_counted_producer_catalog -- --test-threads=1
cargo test --test br192_counted_producer_catalog br192_r09_capability_catalog_and_producer_enable_atomically -- --exact --test-threads=1
cargo test --bin monitor br192_r09 -- --test-threads=1
cargo test --bin monitor br192_r09_sourceonly_dispatch_is_reachable_exactly_once -- --exact --test-threads=1
cargo test --lib data_gateway::capital -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_counted_producer_attestation_is_same_transaction_immutable_and_hash_valid -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v5_decisions_gain_no_synthetic_producer_attestation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_delivery_envelope_none_attestation_preserves_v5_canonical_bytes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_delivery_envelope_attestation_setter_has_one_permitted_production_caller -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_schedule_persists_exact_source_date_expiry_and_terminal_state -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_retry_expiry_outbox_replays_prepare_append_ack_and_terminalize_crashes -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_expiry_prepare_wins_total_order_and_blocks_later_start_or_ownership -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pending_start_wins_total_order_and_routes_expiry_to_uncertainty -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_expiry_canonical_binds_private_freshness_observation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_final_pre_call_expiry_consumes_permit_without_external_sink -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_companion_requires_complete_same_transaction_triple -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_companion_is_canonical_immutable_and_commit_deferred -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_rejects_sink_result_at_every_interleaving -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_committed_pre_call_expiry_rejects_later_sink_result_cross_connection -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_restores_cycle_confirmed_existing_result_count_atomically -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_requires_result_terminal_ownership_bijection -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_pre_call_expiry_terminal_transaction_is_recoverable_and_idempotent -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_effective_expired_attempt_is_excluded_from_candidate_orphan_and_evidence -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_expired_freshness_terminal_is_single_audited_and_not_revivable -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_cycle_begin_public_signature_uses_only_durable_result_error_channel -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rollback_preserves_four_stage_retry_origin_reserved_recovery -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_fixed_head_inventory_classifies_every_counted_entry_call -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297 -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_same_day_1535_boundary_precedes_terminal_preflight -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_host_tz_cannot_change_shanghai_review_date_or_1535_boundary -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_before_trusted_request_start_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_after_trusted_request_completion_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_raw_bytes_round_trip_and_mutation_rejects_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_before_request_date_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_capture_crosses_shanghai_midnight_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_invalid_provider_capture_timestamp_fails_pair_before_durable_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br198_prior_date_initial_admission_ignores_retry_expiry_but_retry_rejects -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_delivered_preflight_precedes_permit_gateway_renderer_and_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_delivered_missing_hydration_is_provider_free_retryable -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_rejected_and_uncertain_are_provider_free_terminal -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_nonterminal_is_provider_free_retryable -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_corrupt_or_ambiguous_authority_fail_closed -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_no_occurrence_orders_preflight_then_permit_then_provider_then_renderer_then_sink -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_r09_startup_barrier_failure_is_provider_free -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_br200_business_date_once_claim_prevents_second_r09_decision -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_r09_transition_and_hydration_bind_exact_ordered_rule_ids -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover br192_r09_sourceonly_dispatch_has_no_banner_account_or_broker_authority -- --exact --test-threads=1
cargo test --test monitor_help_isolation -- --test-threads=1

# Freeze the complete Task-8 source as a commit object without moving HEAD.
# All variables in Steps 4-6 remain in this same operator shell.
git add Cargo.toml Cargo.lock src/durable_delivery/counted_producer_catalog.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs src/bin/monitor/durable_delivery_runtime.rs src/bin/monitor/main.rs src/bin/monitor/review_batch.rs src/bin/monitor/push_templates.rs src/bin/monitor/notify.rs src/bin/monitor/v14_adapter.rs src/data_gateway/capital.rs src/data_gateway/mod.rs src/lib.rs src/bin/authorize_delivery_retry.rs src/bin/verify_br192_retry_evidence.rs tests/durable_delivery_counted_cutover.rs tests/br192_counted_producer_catalog.rs tests/magic_market_release_revision.rs tests/monitor_help_isolation.rs tools/release/disable_br192_periodic_retry.patch tools/release/verify_br192_forward_rollback.sh
BR192_VERIFIED_TREE="$(git write-tree)"
BR192_CANDIDATE_PARENT="$(git rev-parse HEAD)"
BR192_RELEASE_CANDIDATE_SHA="$(printf '%s\n' 'BR-192 verified release candidate' | git commit-tree "${BR192_VERIFIED_TREE}" -p "${BR192_CANDIDATE_PARENT}")"
test "$(git rev-parse "${BR192_RELEASE_CANDIDATE_SHA}^{tree}")" = "${BR192_VERIFIED_TREE}"
bash tools/release/verify_br192_forward_rollback.sh "${BR192_RELEASE_CANDIDATE_SHA}"
```

Expected: non-zero test counts, all pass, no protected production artifact
change.

- [ ] **Step 5: Run silent-path diff scan**

```bash
git diff --cached --unified=0 -- '*.rs' | rg 'unwrap_or_default\\(|let _ = .*\\.await|Err\\(.+\\) => \\{\\}|if .* \\{$'
```

Expected: every hit has an explicit reason or is changed to a visible typed
failure. Empty output is acceptable.

- [ ] **Step 6: Commit**

```bash
test -n "${BR192_VERIFIED_TREE}"
test "$(git write-tree)" = "${BR192_VERIFIED_TREE}"
git add Cargo.toml Cargo.lock src/durable_delivery/counted_producer_catalog.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs src/bin/monitor/durable_delivery_runtime.rs src/bin/monitor/main.rs src/bin/monitor/review_batch.rs src/bin/monitor/push_templates.rs src/bin/monitor/notify.rs src/bin/monitor/v14_adapter.rs src/data_gateway/capital.rs src/data_gateway/mod.rs src/lib.rs src/bin/authorize_delivery_retry.rs src/bin/verify_br192_retry_evidence.rs tests/durable_delivery_counted_cutover.rs tests/br192_counted_producer_catalog.rs tests/magic_market_release_revision.rs tests/monitor_help_isolation.rs tools/release/disable_br192_periodic_retry.patch tools/release/verify_br192_forward_rollback.sh
test "$(git write-tree)" = "${BR192_VERIFIED_TREE}"
git commit -m "feat: integrate BR-192 retry runtime and evidence"
test "$(git rev-parse 'HEAD^{tree}')" = "${BR192_VERIFIED_TREE}"
bash tools/release/verify_br192_forward_rollback.sh "$(git rev-parse HEAD)"
```

### Task 9: Gate B, C and D verification

**Files:**

- Modify only files implicated by a failing gate
- Record raw command/output evidence in the PR

- [ ] **Step 1: Before/after protected production artifact proof**

Use the checked-in isolation tests as the authoritative proof. In addition,
record metadata-only existence/type/inode snapshots for the actual roots
resolved by runtime. Do not read or hash production content.

Required protected classes:

```text
SQLite DB/journal/WAL/SHM
provider-free retry runner lock
push-log root
durable immutable audit root
event-audit root
observation event-bus root
```

- [ ] **Step 2: Gate B module tests**

```bash
cargo test --lib durable_delivery -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_delivery_root_reexports_complete_retry_runtime_and_cli_contract -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_persisted_retry_sink_outcome_joins_terminal_result_and_ownership -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_ownership_pointer_update_then_result_insert_failure_rolls_back_and_quarantines_without_resend -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_schema_v6_fresh_and_v1_v2_v3_v4_v5_upgrade_paths_validate -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v5_to_v6_preserves_br194_replay_manifest_audit_kinds_and_rows -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_is_registered_before_every_schema_path -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_registration_follows_complete_descriptor_binding -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_durable_sha256_udf_never_runs_before_wal_shm_attestation -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_rusqlite_031_exposes_utf8_deterministic_and_innocuous_function_flags -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_v6_authority_triggers_reject_bytes_hash_and_combined_mutations -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_python_br194_verifier_uses_hashlib_without_sql_callback_or_trigger_execution -- --exact --test-threads=1
cargo build --lib
cargo test --bin monitor durable_delivery_runtime -- --test-threads=1
cargo test --test durable_delivery_counted_cutover -- --test-threads=1
cargo test --test br192_counted_producer_catalog -- --test-threads=1
cargo test --bin monitor br192_r09 -- --test-threads=1
cargo test --lib data_gateway::capital -- --test-threads=1
cargo test --lib durable_delivery::tests::br192_expired_freshness_terminal_is_single_audited_and_not_revivable -- --exact --test-threads=1
cargo test --test monitor_help_isolation -- --test-threads=1
bash tools/release/verify_br192_forward_rollback.sh "$(git rev-parse HEAD)"
```

Expected: all exit 0; output reports non-zero tests; cross-process parent test
shows both child exits and exactly one sink call.

- [ ] **Step 3: Multiline-aware production integration evidence**

```bash
rg -n -U 'run_provider_free_retry_cycle\\([\\s\\S]{0,300}' src/bin/monitor/main.rs
rg -n -U 'retry_cycle_blocking\\([\\s\\S]{0,2000}coordinator\\.prepare_retry_attempt[\\s\\S]{0,1000}coordinator\\.reconcile_prepared_retry_attempt_audit[\\s\\S]{0,1000}coordinator\\.claim_retry_sink_execution[\\s\\S]{0,1000}coordinator\\.execute_prepared_retry_sink[\\s\\S]{0,500}RetrySinkExecutionOutcome' src/bin/monitor/durable_delivery_runtime.rs
rg -n -U 'finish_cycle_failed_and_append\\([\\s\\S]{0,1600}quarantine_same_cycle_attempts_before_failure[\\s\\S]{0,1600}prepare_retry_cycle_failed[\\s\\S]{0,1600}reconcile_one_retry_cycle_audit[\\s\\S]{0,1600}terminalize_retry_cycle_failed' src/bin/monitor/durable_delivery_runtime.rs
rg -n -U 'ProductionRetryAuthorizationRequest[\\s\\S]{0,500}authorize_delivery_retry_production\\(' src/bin/authorize_delivery_retry.rs
cargo test --bin authorize_delivery_retry br192_authorization_cli_request_surface_is_timestamp_and_capability_free -- --exact --test-threads=1
! rg -n 'RetryCommandAuthority|RetryCommandTargetResolver|RetryCommandTarget|AuthenticatedRetryOperator|ResolvedRetryCommandTarget' src/bin/authorize_delivery_retry.rs
! rg -n 'OperatorAuthAttestation|authenticate_monitor_operator' src/bin/authorize_delivery_retry.rs
! rg -n -U 'pub\\s+(?:mod\\s+(?:retry_command|retry_evidence)\\s*;|use(?s:[^;])*\\b(?:RetryCommandAuthority|RetryCommandTargetResolver|RetryCommandTarget|AuthenticatedRetryOperator|ResolvedRetryCommandTarget|PamRetryCommandAuthority|ProductionRetryCommandTargetResolver|RetryEvidenceTarget|TestRetryEvidenceTarget|verify_br192_retry_evidence_test)\\b(?s:[^;])*;)' src/durable_delivery/mod.rs
rg -n -U 'authorize_delivery_retry_production\\([\\s\\S]{0,1200}authenticate_monitor_operator\\([\\s\\S]{0,1200}AuthenticatedRetryOperator' src/durable_delivery/retry_command.rs
! rg -n '(validated_at|authorized_at)\\s*:\\s*(Utc::now|SystemTime::now)' src/durable_delivery/retry_command.rs
rg -n 'try_pam_auth|Utc::now|/dev/urandom|OperatorAuthAttestation' src/auth/operator.rs
rg -n -U 'fn run\\([^)]*\\)[\\s\\S]{0,1200}verify_br192_retry_evidence\\(&query\\)' src/bin/verify_br192_retry_evidence.rs
cargo test --bin verify_br192_retry_evidence br192_verify_evidence_cli_has_exact_production_arguments_and_library_call -- --exact --test-threads=1
cargo test --bin verify_br192_retry_evidence br192_verify_evidence_cli_rejects_out_of_range_count_before_library_call -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_test_evidence_verifier_exact_join_is_read_only_and_nonce_bound -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_query_count_bounds_apply_before_authority_open -- --exact --test-threads=1
cargo test --lib durable_delivery::tests::br192_evidence_stream_rejects_257th_distinct_join_without_partial_output -- --exact --test-threads=1
! rg -n 'RetryEvidenceTarget|TestRetryEvidenceTarget|verify_br192_retry_evidence_test|--root|--target|--test' src/bin/verify_br192_retry_evidence.rs
rg -n 'retry_authorization_bindings|retry_attempt_bindings|retry_cycle_audit_outbox|RetryAuthorization' src/durable_delivery
cargo test --test br192_counted_producer_catalog -- --test-threads=1
rg -n -U 'dispatch_r09_provider_top_n_outcome[\s\S]{0,1200}CountedProducerPermit[\s\S]{0,1200}provider_top_n_pair[\s\S]{0,2400}(deliver_counted_binding|deliver_envelope)' src/bin/monitor/push_templates.rs
rg -n -U 'ReviewTask::R09[\s\S]{0,1800}dispatch_r09_provider_top_n_outcome\(business_date, observed_at\)' src/bin/monitor/review_batch.rs src/bin/monitor/push_templates.rs
! rg -n -U 'dispatch_r09_provider_top_n_outcome[\s\S]{0,2400}(BannerCtx|banner_text|current_banner|AccountMode|broker.*snapshot)' src/bin/monitor/push_templates.rs src/bin/monitor/review_batch.rs
rg -n 'CapitalDataGateway|provider_top_n_pair|source_business_date|expires_at' src/data_gateway/capital.rs src/bin/monitor/push_templates.rs
rg -n -U '(push_governor|push_governor_v3|push_governor_v3_with_sub_kind|dispatch_outcome|deliver_counted_binding|deliver_envelope)[\s\S]{0,1200}(CountedProducerPermit|counted_binding_required)' src/bin/monitor/notify.rs src/bin/monitor/push_templates.rs src/bin/monitor/durable_delivery_runtime.rs
rg -n 'data/push_log|_audit_pending.json|_committed.json|data/event_bus|push.delivery.audit|delivery.retry.cycle' src/bin/monitor/notify.rs src/bin/monitor/main.rs src/durable_delivery
```

Expected: main has startup/periodic call sites; runtime has the ordered
coordinator-owned prepare/append-ack/pre-call-CAS/execute/persisted-outcome
seam, returns no bare sink result, and has no direct retry-origin resume;
authorization CLI calls the library command; the evidence CLI calls the
checked-in JSON parser/exact join; schema/coordinator own companion authority.
The catalog test reports all 15 kinds with exactly one enabled R-09 seam;
R-09's permit precedes its gateway; all generic counted entries require a
permit/binding; and evidence is bound to the exact design §10.1 paths/types.

- [ ] **Step 4: Gate C**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/lib/check_br192_provider_free_retry.sh
bash tools/compliance/lib/check_br194_review_dependency.sh
bash tools/release/verify_br192_forward_rollback.sh "$(git rev-parse HEAD)"
bash tools/compliance/check.sh
```

Expected: all exit 0. The two focused checkers must independently recognize
the same v6 manifest and deterministic canonical SHA-256 contract. Rust checks
the attestation-safe UDF registration/flags and trigger execution; Python
checks read-only catalog text and independently hashes returned BLOBs with
`hashlib.sha256` without a SQL callback. Neither may pass from trigger names,
logged success or `user_version` alone.

- [ ] **Step 5: Isolated coverage Gate D (BR-202 authority only)**

```bash
bash tools/coverage/run_isolated_gate.sh
```

Expected: the BR-202 wrapper exits 0 and mints the fixed-source isolated Gate-D
bundle/terminal evidence with global line coverage at least 80% and core
trading/data paths at least 95%. Raw `cargo llvm-cov` or
`tools/coverage/check_thresholds.py` runs are diagnostics only and cannot mint
release authority. LCOV is never passed to the JSON checker.

- [ ] **Step 6: Release and isolated `--test` smoke**

```bash
cargo build --release --bin monitor --bin authorize_delivery_retry --bin verify_br192_retry_evidence
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | tee /tmp/TEST_CODE_BR192_monitor_test.log
rg -n 'provider-free retry|delivery.retry.cycle|BR-192' /tmp/TEST_CODE_BR192_monitor_test.log
```

Expected: release build exits 0; TEST_CODE smoke uses isolated authorities,
prints the BR-192 runner evidence and no production artifact changes.

- [ ] **Step 7: Bounded normal monitor evidence**

Use one release process and terminate only that resolved PID:

```bash
RUST_LOG=info ./target/release/monitor > /tmp/br192_normal_monitor.log 2>&1 &
BR192_MONITOR_PID=$!
sleep 65
kill -INT "${BR192_MONITOR_PID}"
wait "${BR192_MONITOR_PID}"
rg -n 'provider-free retry runner started|delivery.retry.cycle|监控已安全关闭' /tmp/br192_normal_monitor.log
if kill -0 "${BR192_MONITOR_PID}" 2>/dev/null; then exit 1; fi
```

Expected: exactly 15 counted-producer catalog lines in `PushKind::ALL` order,
fourteen exact `disabled=no_producer` lines and one exact enabled R-09 line
from design §1.1; at least one
retry-runner startup banner, at least one delayed cycle after
30 seconds, graceful shutdown, no overlapping-cycle error and no surviving
process launched by this harness. Do not use a broad `pkill`.

- [ ] **Step 8: Production R-09 consumer and real receipt evidence**

Gate D must prove fourteen disabled banners, the sole enabled R-09 banner, a
real provider batch for the exact completed review-calendar business date,
its counted binding and an authoritative
receipt join. Run the checked-in catalog checker and bounded harness, then the
strict review path during the valid R-09 window:

```bash
bash tools/compliance/lib/check_br192_provider_free_retry.sh
test "$(rg -c '^.*\[BR-192\]\[counted-producer\].*disabled=no_producer' /tmp/br192_normal_monitor.log)" -eq 14
test "$(rg -c '^.*\[BR-192\]\[counted-producer\] push_kind=ReviewProviderTopN enabled=durable_binding producer=push_templates::dispatch_r09_provider_top_n_outcome' /tmp/br192_normal_monitor.log)" -eq 1
if rg -n 'push_kind=(HoldingPlan|HoldingEvent|T0Advice|CandidateTriggered|CloseCall|ForbiddenOps|PaperTrade|ReviewMarket|ReviewLhb|ReviewSignal|ReviewFailure|TomorrowWatch|EventCalendar|DailyReport).*provider acquisition started' /tmp/br192_normal_monitor.log; then
  exit 1
fi
cargo run --bin monitor -- --review
cargo run --bin verify_br192_retry_evidence -- \
  --date <LATEST_COMPLETED_SHANGHAI_TRADING_DATE> \
  --push-kind ReviewProviderTopN --require-count 1
```

Expected: checker/review/verifier exit 0, exact 14/1 startup catalog, no
disabled acquisition, R-09 provider/binding evidence and
`verified_retry_count>=1`. Do not pass `--require-count 0`. If no authentic
`RejectedDurable` occurrence exists, do not manufacture one and do not weaken
the verifier: Gate D remains explicitly **In Progress: awaiting one authentic
R-09 rejection and subsequent authorized retry** while initial R-09 delivery
continues fail-closed under the durable counted path. Once such a row exists,
authorize it through the PAM command and join the real retried terminal:

```text
retry_attempt_bindings cycle/authorization/disposition/binding/reservation
  generations + retry ordinal + owner identity + fence token
retry_authorization_bindings historical binding identity
retry_authorizations historical authorization + immutable_audit_ref
retry_authorization_events historical Applied + immutable_audit_ref
retry_schedules.last_attempt_binding_identity + exact retry ordinal
delivery_attempts exact reservation generation/owner/fence
retry_cycle_audit_outbox exact SinkAttemptStarted identity/canonical/SHA/ref
retry_send_ownership exact attempt/binding/execution-cycle/generation/owner/
  positive-i64 fence/start time/send_consumed and legal terminal transition
sink_results authoritative terminal
immutable delivery/state/cycle audit refs
```

Expected: one consistent decision/envelope hash, immutable attempt binding,
historical appended authorization and `Applied` event refs, matching
binding/reservation generations and one sink result. A later current
disposition or cleared active binding does not invalidate this historical
proof. The PR must paste the exact command and redacted JSON output produced by
the checked-in verifier; shell grep and ad-hoc production SQLite reads are
forbidden.

- [ ] **Step 9: Cross-version debt check**

```bash
rg -n 'is_active_spec_target_|is_legacy_v17_|ReviewProviderTopN' src/bin/monitor/notify.rs src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
rg -n -U 'ReviewProviderTopN[\\s\\S]{0,500}(deliver_counted_binding|dispatch|push_governor)' src/bin/monitor
```

Expected: `ReviewProviderTopN` has exactly the enabled seam from design §1.1,
uses `CapitalDataGateway::provider_top_n_pair`, acquires its permit before the
provider and reaches only the durable counted entry. Every other kind remains
classified disabled and has no counted-specific acquisition/sink path. Legacy
annotations have an explicit deletion plan or are recorded as upstream debt;
the complete syntax-aware catalog test, not a single-line grep, is the
authoritative all-15 proof.

- [ ] **Step 10: Independent review and PR evidence**

A fresh reviewer brief must begin with the mandatory Upstream debt, Rename
impact and Production evidence sections, followed verbatim by:

> **DO NOT trust the implementer's report as ground truth.** Independently:
> - Re-run any command the implementer claimed to have run (cargo test, grep, build).
> - grep `data/push_log/$(date +%Y-%m-%d)/` for evidence the new module is on a real push path.
> - grep `data/event_bus/$(date +%Y-%m-%d).jsonl` for the audit event_type.
> - Verify exactly fourteen disabled banners and the exact enabled R-09 banner; inspect permit-before-gateway ordering.
> A verifier that returns "Approved" without independent production-log evidence is auto-rejected.

The PR must include:

```text
Refs: design §1-15; BR-192; BR-194 SourceOnly/R-09 preservation; BR-198 review-calendar date; BR-200 durable occurrence preflight; BR-202 isolated Gate-D authority
Data-Redlines: [2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 2.10]
Data-Redline-Evidence: plan “Data-red-line execution matrix”; 2.3 requires finite, complete, non-empty dual-batch validation and typed Failed evidence for empty/partial/stale/malformed R-09 input; 2.6/2.9 are scoped N/A proofs; 2.4 requires expiry-terminal evidence; all others require positive evidence
OldModules: design §13 table
Threshold-Proof: retry constants govern delivery recovery only; no financial threshold changed
Business-Rules: BR-192, BR-194, BR-198, BR-200, BR-202
Rollback: design §14
Validation: raw Gate B/C/D commands and outputs
```

Do not merge until independent review reports `Critical=0 / Important=0`.

## Rollback execution

If Gate B/C/D fails:

1. stop the monitor and record the literal accepted `BR192_RELEASE_SHA`;
2. create `rollback/br192-provider-free-retry` at that exact SHA, verify the
   checked-in `tools/release/disable_br192_periodic_retry.patch` contains exactly
   one `diff --git` target (`src/bin/monitor/main.rs`), run `git apply --check`,
   then `git apply --index` that forward patch. Never `git revert` the atomic
   Task-8 commit. Before any rollback binary is built, the staged diff must
   contain only the periodic runner installation removal and retain Task 2's v6
   schema recognition, deterministic `sha256_hex` registration and complete
   shared-manifest validation;
3. retain `SCHEMA_VERSION=6` recognition/validation, all v5 baseline and v6
   companion tables/triggers/indexes/outboxes/immutable records, and every
   BR-194 replay object/audit kind/manifest semantic in a new
   forward-compatible rollback build;
4. never launch a historical schema-v2/v3/v4 binary, lower `user_version`,
   unregister `sha256_hex` or remove v5 baseline/v6 companion objects;
5. do not restore automatic boolean `reacquire_rejected`;
6. build the new v6-aware rollback release binary from the previous runtime
   behavior;
7. restart with only the periodic retry runner absent/disabled at its single
   startup call site; preserve the exact validated 15-row catalog bytes/hash,
   the enabled R-09 row, generic counted-binding guard, retained-attestation
   validation and initial R-09 durable delivery unchanged. Never introduce a
   rollback catalog row/reason or restore legacy generic delivery; any catalog
   transition requires a new Gate-A design after all retained/pending authority
   has terminalized;
8. preserve all `ExpiredFreshness` terminal rows/events, including
   `retry_pre_call_expiry_authorities`, and retain the retry-origin classifier.
   Existing retry-origin `Reserved` rows may use only the four-stage
   prepare→append/ack-start→claim→execute seam (including expiry/uncertainty),
   never legacy `resume_deliverable`; if the rollback build cannot provide the
   seam, leave those rows untouched and fail closed. Pending terminal bytes
   continue only through their exact idempotent reconciliation path; and
9. reopen Gate A for any state-model or audit-contract failure, Gate B for an
   implementation failure, and Gate C for a compliance failure.

No rollback command may delete a database, WAL/SHM, authorization, reservation,
attempt, receipt, disposition, push log or audit record.
