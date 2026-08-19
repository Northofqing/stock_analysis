# P-01 Reachability and Safe Compensation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make P-01 reachable from the resident pre-market owner and safely compensable exactly once with real completed-day LimitPools/chain evidence, exact head identities/news, and an authoritative Feishu receipt.

**Architecture:** A closed `p01` module owns date, schedule, mode, and outcome decisions. Scheduled and exclusive compensation modes share one runner: generic BusinessDateOnce preflight, exact previous-trading-day LimitPools, deterministic chain projection, exact SecurityIdentity and per-head InstrumentNews, explicit scheduled/late rendering, and existing `notify::push_counted_with_binding`. The compensation command holds the normal production monitor lease and cannot run beside the resident.

**Tech Stack:** Rust, Tokio, Chrono, Diesel/SQLite, LocalBridge gRPC, ExternalV1 InstrumentNews, durable-delivery coordinator, MagicLaw CLI typed receipts, SHA-256 immutable audits.

**Business Rule:** BR-241

---

## File map

- Create `src/bin/monitor/p01.rs`: dates, modes, due decisions, source binding,
  outcomes, errors, orchestration, and scheduler.
- Modify `src/bin/monitor/main.rs`: install pre-market owner, retire unreachable
  P-01 block, dispatch exclusive compensation.
- Modify `src/selection/process_bootstrap.rs`: strict compensation grammar.
- Modify `src/database/concepts.rs`: exact-date chain read.
- Create `src/pipeline/chain_analysis/p01_projection.rs`: deterministic chain
  projection/persistence from one admitted LimitPools batch, with no sink.
- Modify `src/pipeline/chain_analysis/mod.rs`: export the projection.
- Modify `src/bin/monitor/push_templates.rs`: scheduled/late rendering and
  canonical evidence composition.
- Modify `src/durable_delivery/model.rs`, `src/durable_delivery/schema.rs`, and
  `src/durable_delivery/tests.rs`: durable kind, policy/catalog migration, and
  state-machine coverage.
- Modify `src/bin/monitor/durable_delivery_runtime.rs`: generic BusinessDateOnce
  preflight, P-01 binding validation, and monitor-kind mapping.
- Modify `src/bin/monitor/notify.rs`: use the existing public counted entry.
- Modify `tests/monitor_help_isolation.rs`: CLI/lease/process isolation.
- Modify `tests/durable_delivery_counted_cutover.rs`: exact join and crash
  recovery.

## Parallel execution map

```text
Wave A, parallel and file-disjoint
  Task 1: P-01 date/schedule types
  Task 2: LimitPools-to-chain projection
  Task 3: durable kind + generic BusinessDateOnce preflight
  Task 4: strict CLI grammar

Wave B
  Task 5: identity/news binding + two render modes   depends on Tasks 1,2
  Task 6: resident and compensation integration      depends on Tasks 1,3,4,5

Wave C, parallel and file-disjoint
  Task 7: process/lease isolation                     depends on Task 6
  Task 8: crash recovery/exact join                   depends on Task 6

Wave D
  Task 9: Gate C/D and controlled production acceptance
```

Task 1 leaves `mod p01;` to Task 6. No Wave A worker edits another lane's files.

### Task 1: Close P-01 dates and schedule

**Files:**
- Create: `src/bin/monitor/p01.rs`
- Test: `src/bin/monitor/p01.rs`

- [ ] **Step 1: Write failing date/window tests**

```rust
#[test]
fn p01_tuesday_uses_completed_monday() {
    let context = P01BusinessContext::new(date(2026, 8, 18)).unwrap();
    assert_eq!(context.evidence_date, date(2026, 8, 17));
}

#[test]
fn p01_monday_uses_previous_friday() {
    let context = P01BusinessContext::new(date(2026, 8, 24)).unwrap();
    assert_eq!(context.evidence_date, date(2026, 8, 21));
}

#[test]
fn p01_calendar_authority_fails_closed() {
    assert_eq!(
        P01BusinessContext::new(date(2026, 10, 1))
            .unwrap_err()
            .reason_code(),
        "p01_business_date_not_trading",
    );
    assert_eq!(
        P01BusinessContext::new(date(2027, 1, 1))
            .unwrap_err()
            .reason_code(),
        "p01_trading_calendar_unavailable",
    );
}

#[test]
fn p01_window_is_start_inclusive_end_exclusive() {
    let due = date(2026, 8, 18);
    assert!(matches!(classify_scheduled_due(due.and_hms_opt(9, 0, 0).unwrap()), P01Due::Due(_)));
    assert!(matches!(classify_scheduled_due(due.and_hms_opt(9, 15, 0).unwrap()), P01Due::NotDue(P01NotDueReason::ScheduledWindowClosed)));
}
```

- [ ] **Step 2: Run RED**

Run: `cargo test --bin monitor p01_tuesday_ p01_monday_ p01_calendar_ p01_window_ -- --nocapture`

Expected: FAIL because the P-01 types/functions are absent.

- [ ] **Step 3: Implement the closed types**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct P01BusinessContext {
    pub business_date: chrono::NaiveDate,
    pub evidence_date: chrono::NaiveDate,
}

impl P01BusinessContext {
    pub fn new(business_date: chrono::NaiveDate) -> Result<Self, P01Failure> {
        match stock_analysis::calendar::verified_a_share_trading_day(business_date) {
            Ok(true) => {}
            Ok(false) => {
                return Err(P01Failure::terminal(
                    "p01_business_date_not_trading",
                    "calendar",
                ));
            }
            Err(_) => {
                return Err(P01Failure::terminal(
                    "p01_trading_calendar_unavailable",
                    "calendar",
                ));
            }
        }
        let evidence_date =
            stock_analysis::calendar::verified_prev_a_share_trading_day(business_date)
                .map_err(|_| {
                    P01Failure::terminal("p01_trading_calendar_unavailable", "calendar")
                })?;
        Ok(Self {
            business_date,
            evidence_date,
        })
    }
}

pub enum P01ExecutionMode { Scheduled, Compensation }
pub enum P01Due { Due(P01BusinessContext), NotDue(P01NotDueReason) }
pub enum P01NotDueReason {
    NonTradingDay,
    CalendarUnavailable,
    BeforeWindow,
    ScheduledWindowClosed,
    CompensationBeforeWindowClosed,
    BusinessDateMismatch,
}

pub enum P01RunOutcome {
    Delivered { decision_identity: String, receipt_sha256: String },
    AlreadyDelivered { decision_identity: String },
    AwaitingReconciliation { attempt_identity: String },
    RetryableFailure(P01Failure),
    TerminalFailure(P01Failure),
}
```

`classify_scheduled_due` accepts one captured `NaiveDateTime`; it does not read
the system clock internally.

- [ ] **Step 4: Run GREEN and commit**

```bash
cargo test --bin monitor p01_tuesday_ p01_monday_ p01_window_ -- --nocapture
git add src/bin/monitor/p01.rs
git commit -m "fix(p01): close business and evidence dates"
```

Expected: all Task 1 tests PASS.

### Task 2: Derive exact-date chain from admitted LimitPools

**Files:**
- Modify: `src/database/concepts.rs`
- Create: `src/pipeline/chain_analysis/p01_projection.rs`
- Modify: `src/pipeline/chain_analysis/mod.rs`
- Test: both implementation files

- [ ] **Step 1: Write exact-date DB RED test**

```rust
#[test]
fn p01_chain_read_uses_requested_date_not_max_date() {
    let rows = DatabaseManager::get()
        .get_chain_clusters_for_date_strict(date(2026, 8, 17)).unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row.date == "2026-08-17"));
}
```

Run: `cargo test --lib database::concepts::tests::p01_chain_read_ -- --nocapture`

Expected: FAIL because `get_chain_clusters_for_date_strict` is absent.

- [ ] **Step 2: Add exact-date query**

```rust
pub fn get_chain_clusters_for_date_strict(
    &self,
    date: chrono::NaiveDate,
) -> Result<Vec<ChainDailyRow>, String>;
```

Bind `date.format("%Y-%m-%d")`, use only `WHERE date = ?`, and order by
`continuation_count DESC, concept ASC`. Empty is distinct from query failure.
Do not add a P-01 rotation query.

- [ ] **Step 3: Write projection RED tests**

```rust
#[test]
fn p01_projection_uses_only_supplied_limit_pool() {
    let batch = complete_limit_pool(date(2026, 8, 17));
    let receipt = persist_p01_chain_from_limit_pool(&batch, date(2026, 8, 17)).unwrap();
    assert_eq!(receipt.evidence_date, date(2026, 8, 17));
    assert_eq!(receipt.limit_pool_batch_id, batch.evidence().batch_id);
}

#[test]
fn p01_projection_fails_when_no_record_has_provider_classification() {
    let error = persist_p01_chain_from_limit_pool(&unclassified_limit_pool(), date(2026, 8, 17)).unwrap_err();
    assert_eq!(error.reason_code(), "p01_limit_pool_has_no_classified_chain_members");
}
```

Run: `cargo test --lib pipeline::chain_analysis::p01_projection::tests::p01_ -- --nocapture`

Expected: FAIL because `p01_projection` is absent.

- [ ] **Step 4: Implement deterministic projection**

```rust
pub struct P01ChainProjectionReceipt {
    pub evidence_date: chrono::NaiveDate,
    pub limit_pool_batch_id: String,
    pub ordered_limit_pool_record_hashes: Vec<String>,
    pub excluded_record_hashes: Vec<(String, &'static str)>,
    pub ordered_chain_row_hashes: Vec<String>,
    pub persistence_receipt_sha256: String,
}

pub fn persist_p01_chain_from_limit_pool(
    batch: &GatewayBatch<LimitPoolEntry>,
    evidence_date: chrono::NaiveDate,
) -> Result<P01ChainProjectionReceipt, P01ProjectionError>;

pub fn acquire_and_persist_p01_chain(
    evidence_date: chrono::NaiveDate,
) -> Result<P01CompletedDayEvidence, P01ProjectionError>;
```

Classification precedence is `industry`, then `board_name`, then `reason`.
All-three-missing rows are excluded with
`p01_chain_classification_missing` and retained in canonical exclusions.
Within group order by `streak DESC NULLS LAST`, `change DESC`, code ASC. Order
groups by member count DESC, maximum streak DESC, concept ASC.
`continuation_count` counts members with provider `streak >= 2`. Persist/read
back only `evidence_date`. This module has no notification dependency.
`acquire_and_persist_p01_chain` is the public library seam: inside the library
crate it calls crate-private `ReviewDataGateway::current_upper_limit_pool`, then
the pure projection above, and returns the original admitted LimitPools batch
plus projection receipt. The monitor invokes this synchronous function inside
`spawn_blocking`; it does not expose or bypass the gateway admission boundary.

- [ ] **Step 5: Run GREEN and commit**

```bash
cargo test --lib database::concepts::tests::p01_chain_read_ -- --nocapture
cargo test --lib pipeline::chain_analysis::p01_projection::tests::p01_ -- --nocapture
git add src/database/concepts.rs src/pipeline/chain_analysis/mod.rs src/pipeline/chain_analysis/p01_projection.rs
git commit -m "feat(p01): derive exact chain from limit pools"
```

Expected: both focused suites PASS.

### Task 3: Add durable P-01 and generic BusinessDateOnce preflight

**Files:**
- Modify: `src/durable_delivery/model.rs`
- Modify: `src/durable_delivery/schema.rs`
- Modify: `src/durable_delivery/tests.rs`
- Modify: `src/bin/monitor/durable_delivery_runtime.rs`
- Test: durable unit/runtime tests

- [ ] **Step 1: Write policy RED test**

```rust
#[test]
fn p01_policy_is_global_business_date_once_and_budget_exempt() {
    let row = compiled_policy_catalog().into_iter()
        .find(|row| row.push_kind == PushKind::PreopenNewsHot).unwrap();
    assert_eq!(row.cooldown_scope, CooldownScope::Global);
    assert_eq!(row.window_mode, WindowMode::BusinessDateOnce);
    assert_eq!(row.sub_kind, DeliverySubKind::None);
    assert!(!row.counts_against_daily_budget);
}
```

Run: `cargo test --lib durable_delivery::tests::p01_policy_ -- --nocapture`

Expected: FAIL because durable P-01 is absent.

- [ ] **Step 2: Add kind, policy, and policy-only migration**

Add `PushKind::PreopenNewsHot`, stable ID `preopen_news_hot_v1`,
`Global + BusinessDateOnce + Some(86_400) + counts_against_daily_budget=false`.
Increment `POLICY_VERSION`, kind/catalog cardinalities, and schema version.
Migration replaces only `delivery_policy_catalog`; it cannot edit decisions,
attempts, claims, receipts, cooldown heads, or immutable audits.

- [ ] **Step 3: Write generic preflight RED test**

```rust
#[tokio::test]
async fn p01_preflight_is_generic_and_uses_no_review_task() {
    let result = inspect_business_date_once_claim(
        date(2026, 8, 18),
        DurablePushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "GLOBAL",
        "p01:2026-08-18",
    ).await.unwrap();
    assert!(result.is_none());
}
```

- [ ] **Step 4: Implement generic preflight and P-01 validator**

```rust
pub async fn inspect_business_date_once_claim(
    business_date: chrono::NaiveDate,
    push_kind: stock_analysis::durable_delivery::PushKind,
    sub_kind: stock_analysis::durable_delivery::DeliverySubKind,
    scope_key: &str,
    occurrence_identity: &str,
) -> Result<Option<DurableDispatchEvidence>, String>;
```

Require compiled `BusinessDateOnce` policy and completed startup
reconciliation; query before providers without constructing a `ReviewTask`.
Refactor the review-specific wrapper to validate its review identity then
delegate. Add `CountedDeliveryBinding::validate_p01_text`, exact-key checking
the occurrence, LimitPools/chain date, identity exact set, one news batch per
head/range, exclusions, ordered hashes, and rendered bytes hash. Immediately
before sink authorization, the validator first requires
`verified_a_share_trading_day(business_date) == Ok(true)` and only then resolves
`verified_prev_a_share_trading_day(business_date)`. A known non-trading date or
unavailable immutable-calendar coverage returns the stable
`counted_p01_calendar_authority_unavailable` reason and performs zero sink I/O.

- [ ] **Step 5: Run GREEN and commit**

```bash
cargo test --lib durable_delivery::tests::p01_ -- --nocapture
cargo test --bin monitor durable_delivery_runtime::tests::p01_ -- --nocapture
git add src/durable_delivery/model.rs src/durable_delivery/schema.rs src/durable_delivery/tests.rs src/bin/monitor/durable_delivery_runtime.rs
git commit -m "feat(p01): add durable policy and generic preflight"
```

Expected: policy, migration, generic preflight, and validation tests PASS.

### Task 4: Parse one exclusive compensation command

**Files:**
- Modify: `src/selection/process_bootstrap.rs`
- Test: same file

- [ ] **Step 1: Write parser RED tests**

```rust
#[test]
fn p01_compensation_accepts_only_exact_pair() {
    let parsed = parse(&["monitor", "--compensate=P-01", "--business-date=2026-08-18"]).unwrap();
    assert_eq!(parsed.compensation, Some(CompensationCommand::P01 { business_date: date(2026, 8, 18) }));
}

#[test]
fn p01_compensation_rejects_test_push_review_unknown_and_duplicates() {
    for args in invalid_p01_compensation_argv_cases() {
        assert!(parse(&args).is_err(), "accepted {args:?}");
    }
}
```

Run: `cargo test --lib selection::process_bootstrap::tests::p01_compensation_ -- --nocapture`

Expected: FAIL because compensation grammar is absent.

- [ ] **Step 2: Implement strict grammar**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompensationCommand {
    P01 { business_date: chrono::NaiveDate },
}
```

Require one `--compensate=P-01` plus one `--business-date=YYYY-MM-DD`; reject
missing, duplicate, malformed, test, and every other operational argument.
Parser performs no clock, DB, provider, audit, or lease operation.

- [ ] **Step 3: Run GREEN and commit**

```bash
cargo test --lib selection::process_bootstrap::tests::p01_compensation_ -- --nocapture
git add src/selection/process_bootstrap.rs
git commit -m "feat(p01): parse exclusive compensation command"
```

Expected: all parser tests PASS.

### Task 5: Bind identity/news and render scheduled versus late

**Depends on:** Tasks 1 and 2

**Files:**
- Modify: `src/bin/monitor/p01.rs`
- Modify: `src/bin/monitor/push_templates.rs`
- Test: `src/bin/monitor/push_templates.rs`

- [ ] **Step 1: Write binding/render RED tests**

```rust
#[test]
fn p01_binding_requires_exact_head_identities() {
    let error = complete_builder().with_missing_identity().build().unwrap_err();
    assert_eq!(error.reason_code(), "p01_security_identity_exact_set_mismatch");
}

#[test]
fn p01_binding_rejects_news_outside_range() {
    let error = complete_builder().with_news_source_date(date(2026, 8, 16)).build().unwrap_err();
    assert_eq!(error.reason_code(), "p01_instrument_news_range_mismatch");
}

#[test]
fn p01_compensation_render_is_explicitly_late() {
    let text = render_bound_preopen_news_hot(P01RenderMode::Compensation, &complete_input()).unwrap();
    assert!(text.contains("盘前热点补发"));
    assert!(text.contains("依据前一交易日 2026-08-17"));
}
```

Run:

```bash
cargo test --bin monitor push_templates::tests::p01_binding_ -- --nocapture
cargo test --bin monitor push_templates::tests::p01_compensation_render_ -- --nocapture
```

Expected: FAIL because binding/render modes are absent.

- [ ] **Step 2: Implement exact acquisition and canonical bytes**

```rust
pub async fn load_p01_input_binding(
    context: P01BusinessContext,
    observed_at: chrono::DateTime<chrono::Local>,
) -> Result<P01InputBinding, P01Failure>;
```

Sequence: call library `acquire_and_persist_p01_chain(evidence_date)` in
`spawn_blocking`; use its exact LimitPools plus same-batch chain read-back; derive top
head code set; exact `security_identities`; for each head call
`SinaInstrumentNewsGateway::instrument_news_in_range` from local start of
`evidence_date` through captured `observed_at` on `business_date`. Every head
must be Available or VerifiedEmpty and the full set needs at least one real
news row. Canonical bytes include every evidence/record hash and exclusion.
Never read `board_rotation_daily`, `get_latest_*`, `UpperLimitPoolReview`, or a
cache; never synthesize a name/title.

- [ ] **Step 3: Implement two render modes**

```rust
pub enum P01RenderMode { Scheduled, Compensation }

pub fn render_bound_preopen_news_hot(
    mode: P01RenderMode,
    input: &P01InputBinding,
) -> Result<String, P01Failure>;
```

Compensation starts `盘前热点补发`, states business date and
`依据前一交易日 <evidence_date>`, and displays captured late time. Scheduled
keeps the normal title. Both render only bound facts.

- [ ] **Step 4: Run GREEN and commit**

```bash
cargo test --bin monitor push_templates::tests::p01_ -- --nocapture
git add src/bin/monitor/p01.rs src/bin/monitor/push_templates.rs
git commit -m "feat(p01): bind exact identities and instrument news"
```

Expected: exact-set, range, hash, and two-mode render tests PASS.

### Task 6: Wire resident owner, compensation, and counted entry

**Depends on:** Tasks 1, 3, 4, and 5

**Files:**
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/p01.rs`
- Modify: `src/bin/monitor/push_templates.rs`
- Modify: `src/bin/monitor/notify.rs`
- Modify: `src/bin/monitor/durable_delivery_runtime.rs`
- Test: `src/bin/monitor/p01.rs`

- [ ] **Step 1: Write orchestration RED tests**

```rust
#[tokio::test]
async fn p01_preflight_precedes_every_provider() {
    let ports = RecordingP01Ports::already_delivered();
    let outcome = run_p01_once(P01ExecutionMode::Scheduled, context(), &ports).await;
    assert!(matches!(outcome, P01RunOutcome::AlreadyDelivered { .. }));
    assert_eq!(ports.provider_calls(), 0);
    assert_eq!(ports.sink_calls(), 0);
}

#[tokio::test]
async fn p01_acceptance_is_not_coupled_to_other_push_kinds() {
    let ports = RecordingP01Ports::accepted();
    let outcome = run_p01_once(P01ExecutionMode::Scheduled, context(), &ports).await;
    assert!(matches!(outcome, P01RunOutcome::Delivered { .. }));
    assert_eq!(ports.p01_sink_calls(), 1);
    assert_eq!(ports.other_push_kind_calls(), 0);
}
```

Run: `cargo test --bin monitor p01_preflight_ p01_acceptance_ -- --nocapture`

Expected: FAIL before integration.

- [ ] **Step 2: Implement shared runner and public counted call**

```rust
pub async fn run_p01_once(
    mode: P01ExecutionMode,
    context: P01BusinessContext,
    observed_at: chrono::DateTime<chrono::Local>,
) -> P01RunOutcome;
```

First call generic preflight. Existing delivered returns immediately; uncertain
or resumable states use existing reconciliation without provider reacquisition.
Only absent occurrence loads/render/binds and calls exactly:

```rust
notify::push_counted_with_binding(token, &text, None, binding).await
```

P-01 does not call private `deliver_authoritative_blocking`.

- [ ] **Step 3: Install resident owner and terminal handler**

Add `p01_scheduler_loop` as a sibling after static readiness and before
`while !is_market_active()`. Poll every 30 seconds. Remove legacy P-01 and
`preopen_pushed`; P-03 cannot settle/reopen P-01.

After normal production lease acquisition, compensation validates exact local
today/trading day/`now >= 09:15`, calls only P-01, and exits. Help contains:

```text
monitor --compensate=P-01 --business-date=YYYY-MM-DD
```

- [ ] **Step 4: Run GREEN and commit**

```bash
cargo test --bin monitor p01_ -- --nocapture
cargo test --lib durable_delivery::tests::p01_ -- --nocapture
git add src/bin/monitor/main.rs src/bin/monitor/p01.rs src/bin/monitor/push_templates.rs src/bin/monitor/notify.rs src/bin/monitor/durable_delivery_runtime.rs
git commit -m "fix(p01): wire reachable owner and safe compensation"
```

Expected: focused suites PASS and other-kind call count is zero.

### Task 7: Prove CLI and lease isolation

**Depends on:** Task 6

**Files:**
- Modify: `tests/monitor_help_isolation.rs`

- [ ] **Step 1: Add process tests**

```text
p01_compensation_rejects_test_and_mixed_commands_before_runtime_state
p01_compensation_rejects_future_past_nontrading_and_pre_0915_dates
p01_compensation_loses_resident_lease_before_provider_or_sink
p01_compensation_never_dispatches_other_push_kinds
```

The lease test holds the isolated production-equivalent lease, asserts
`monitor_instance_already_running`, and proves provider/durable/audit/sink
sentinels remain absent.

- [ ] **Step 2: Run RED, make boundary-only fixes, then GREEN**

Run: `cargo test --test monitor_help_isolation p01_compensation_ -- --nocapture`

Expected before fixes: at least one new test FAIL. Fix only parser,
lease/handler ordering, help, or exit; never bypass the lease. Expected after:
all P-01 process tests PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/monitor_help_isolation.rs src/selection/process_bootstrap.rs src/bin/monitor/main.rs
git commit -m "test(p01): prove exclusive compensation process"
```

### Task 8: Prove exactly-once receipt and crash recovery

**Depends on:** Task 6

**Files:**
- Modify: `tests/durable_delivery_counted_cutover.rs`
- Modify: `src/durable_delivery/tests.rs`

- [ ] **Step 1: Add recovery tests**

```text
p01_first_acceptance_has_one_sink_call_and_exact_join
p01_second_same_day_run_is_preflight_deduped
p01_crash_after_remote_accept_before_audit_never_resends
p01_crash_after_audit_before_database_ack_never_resends
p01_uncertain_result_requires_reconciliation
p01_any_source_or_rendered_byte_change_breaks_binding_validation
```

- [ ] **Step 2: Run RED, reuse existing reconciliation, then GREEN**

Run: `cargo test --test durable_delivery_counted_cutover p01_ -- --nocapture`

Expected before fixes: new recovery tests FAIL. Reuse accepted/uncertain
reconciliation; do not alias another kind or add resend. Expected after: all
P-01 tests PASS with one total sink call per accepted occurrence.

- [ ] **Step 3: Commit**

```bash
git add tests/durable_delivery_counted_cutover.rs src/durable_delivery/tests.rs
git commit -m "test(p01): prove exact receipt recovery"
```

### Task 9: Gates and controlled production acceptance

**Depends on:** Tasks 7 and 8

**Files:**
- Modify: `docs/operations/2026-08-18-data-grpc-known-issues.md` only to append
  new acceptance evidence

- [ ] **Step 1: Run Gate C**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

Expected: every command exits 0; otherwise return to Gate B.

- [ ] **Step 2: Run Gate D build/coverage**

```bash
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Expected: global >=80%, core P-01 >=95%, release build exits 0, independent
review has no blocker.

- [ ] **Step 3: Perform single-owner compensation**

Stop old resident through established service control; confirm the normal
production lease is released; run:

```bash
target/release/monitor --compensate=P-01 --business-date=$(date +%Y-%m-%d)
```

Never run beside old resident or with `--push`, `--test`, or `--review`.

- [ ] **Step 4: Independently validate exact production chain**

```text
LimitPools(evidence_date, Upper, 200)
  -> exact chain projection receipt
  -> exact SecurityIdentity head set
  -> InstrumentNews/head over [evidence_date,business_date]
  -> late-labeled P01_SOURCE_BINDING_V1/render hash
  -> preopen_news_hot_v1 Feishu Accepted
  -> decision/attempt/result/receipt/audit exact join
```

- [ ] **Step 5: Repeat command and restart resident**

Expected: existing durable delivery, zero providers, zero new Feishu. Restart
new resident and verify zero extra P-01 sink calls for the date.

- [ ] **Step 6: Append recovery evidence**

Preserve original incident observations. Mark `FIXED` only after focused tests,
Gate C/D, release binary, real typed receipt, exact join, repeat dedup, and
resident restart all pass.

## Self-review

- Coverage: Tasks 1/6 close reachability; 4/7 close CLI/lease; 2/5 bind exact
  LimitPools/chain/identity/news; 3/6/8 close durable preflight/receipt/join; 9
  closes production acceptance.
- Producer reality: no task calls `save_board_rotations`; P-01 adopts existing
  SecurityIdentity and InstrumentNews instead.
- Parallel safety: Waves A/C are file-disjoint; shared integration is Task 6.
- Type consistency: `P01BusinessContext`, `P01ExecutionMode`, `P01Due`,
  `P01RunOutcome`, `P01InputBinding`, `P01Failure`, `P01RenderMode`, and
  `CompensationCommand::P01` keep one meaning.
- Date consistency: LimitPools/chain use `evidence_date`; news uses inclusive
  `[evidence_date,business_date]`; Monday-to-Friday is tested.
- Delivery consistency: only `notify::push_counted_with_binding` is the public
  P-01 sink entry after generic BusinessDateOnce preflight.
- Completion: only Gate A is complete now; implementation/gates/production
  acceptance remain pending.
