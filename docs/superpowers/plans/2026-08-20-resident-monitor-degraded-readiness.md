# Resident Monitor With Degraded Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep the production monitor resident after authority initialization even when market-data capabilities are unavailable, continue governed DataMode/risk notifications with explicit missing-data banners, and automatically recover affected producers without a process restart.

**Architecture:** Preserve the strict BR-238 nine-route probe as release evidence, but remove it from the resident process startup permission. Add two independent background supervisors: a 30-second static-readiness diagnostic/audit loop and an immediate-then-300-second position-chain refresh loop. Every business producer retains its existing local evidence/freshness gate, so degraded runtime liveness never grants permission to manufacture facts, price advice, paper executions, or orders.

**Tech Stack:** Rust, Tokio intervals with `MissedTickBehavior::Skip`, existing `GrpcSource` diagnostics, BR-159 immutable acquisition audit, existing DataMode/Launch/L5/durable notification path, Cargo test/clippy/llvm-cov, repository compliance scripts.

**Spec:** `docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md` and BR-246 in `docs/business_rules.md`.

## Global Constraints

- Apply AGENTS.md rules 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, and 2.10 at every task boundary.
- Do not weaken `external_static_opening_readiness()`, the nine-route contract, provider identity, completeness, freshness, quorum, or the release probe.
- Do not introduce mock/default/cache data into production paths. Tests use `TEST_CODE` identities only.
- A background failure must be an explicit typed state plus immutable audit; it must not terminate all resident producers.
- A producer missing required evidence remains fail-closed with zero physical sink/order calls. Only DataMode/risk status and independently complete facts may proceed through their existing governed delivery paths.
- Preserve PushKind, template IDs, audit schemas, dedup, cooldown, budget, Launch/L5, durable receipt, and order-safety semantics.
- The shared worktree contains unrelated changes. Stage only reviewed hunks/files for this feature; do not use whole-file checkout/reset and do not overwrite user changes.
- The spec and this plan are ignored by `/docs`; include them later with exact `git add -f` paths.
- Do not claim Done until Gate C and Gate D evidence is fresh after the implementation.

---

## Task 1: Persist Every Static Diagnostic Outcome

**Files:**

- Modify: `src/data_gateway/grpc_source.rs:510-875`
- Test: `src/data_gateway/grpc_source.rs` adjacent BR-238/BR-246 test module
- Verify: `docs/business_rules.md` BR-246 remains registered before behavior changes

### Contract

Add a production function:

```rust
pub fn audit_opening_diagnostic_report(
    phase: &'static str,
    report: &OpeningDiagnosticReport,
) -> Result<(), GatewayError>;
```

It must append one BR-159 row for every attempted route in report order. Ready routes retain their real provider/source/source_at/observed_at/batch ID/count. Failed routes retain their route/capability/provider/reason/retryable fields, use no fabricated source time or batch ID, and are recorded as rejected/unavailable. A database/audit failure is returned explicitly; already appended immutable rows are never deleted.

### Steps

- [ ] Add a RED projection test that constructs a nine-attempt `OpeningDiagnosticReport` with eight admitted `TEST_CODE` routes and one `InstrumentNews` failure.

```rust
#[test]
fn br246_diagnostic_audit_projection_retains_all_nine_route_outcomes() {
    let report = test_diagnostic_report_with_instrument_news_failure();
    let rows = opening_diagnostic_audit_rows(
        "TEST_CODE_OpeningStaticResident",
        &report,
        "2099-01-02T01:00:00.000Z",
    )
    .expect("closed audit projection");

    assert_eq!(rows.len(), 9);
    assert_eq!(rows.iter().filter(|row| row.accepted_count > 0).count(), 8);
    let failed = rows
        .iter()
        .find(|row| row.route == "InstrumentNews")
        .expect("failed route retained");
    assert_eq!(failed.reason_code, "TEST_CODE_instrument_cutoff_empty");
    assert!(failed.retryable);
    assert!(failed.source_at.is_none());
    assert!(failed.batch_id.is_none());
}
```

- [ ] Run the focused RED and confirm the missing helper/function is the first failure.

```bash
cargo test --lib br246_diagnostic_audit_projection_retains_all_nine_route_outcomes -- --nocapture --test-threads=1
```

Expected RED: unresolved `opening_diagnostic_audit_rows` or equivalent missing production seam.

- [ ] Introduce a private owned row type and a pure `opening_diagnostic_audit_rows` projection. Do not serialize arbitrary upstream error messages; retain only the closed diagnostic fields already in `OpeningDiagnosticFailure`.

- [ ] Implement `audit_opening_diagnostic_report` by obtaining `DatabaseManager::try_get()` and calling the real `record_data_acquisition` for each projected row. Use the existing domain-separated request hash helper with phase, route, profile/capability, and terminal classification.

- [ ] Add a negative test for an invalid record count conversion and verify it returns `GatewayError::audit_failure` rather than truncating/defaulting.

- [ ] Run focused GREEN.

```bash
cargo test --lib br246_diagnostic_audit_ -- --nocapture --test-threads=1
```

Expected GREEN: all new audit projection tests pass.

- [ ] Run the existing strict readiness regression to prove this task did not weaken the release contract.

```bash
cargo test --lib br238_static_gate_requires_exact_limit_pools_route_and_all_nine_attempts -- --nocapture --test-threads=1
```

- [ ] Format and inspect only this file.

```bash
rustfmt --edition 2021 --check src/data_gateway/grpc_source.rs
git diff --check -- src/data_gateway/grpc_source.rs
```

- [ ] Commit only after the diff contains no unrelated hunk.

```bash
git add src/data_gateway/grpc_source.rs
git commit -m "feat: audit resident opening diagnostics" -m "Refs: docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md"
```

---

## Task 2: Replace the Blocking Static Gate With a Resident Supervisor

**Files:**

- Modify: `src/bin/monitor/main.rs:3560-3785`
- Modify: `src/bin/monitor/main.rs:4689-4765`
- Modify: `src/bin/monitor/main.rs:5140-5180`
- Test: `src/bin/monitor/main.rs` adjacent BR-238/BR-246 tests

### Contract

Add these private seams:

```rust
const OPENING_STATIC_READINESS_PERIOD: Duration = Duration::from_secs(30);

fn opening_static_readiness_interval(period: Duration) -> tokio::time::Interval;

async fn run_opening_static_readiness_scheduler<F, Fut>(
    interval: tokio::time::Interval,
    evaluate: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>;

async fn evaluate_opening_static_readiness_once();
async fn opening_static_readiness_loop();
```

The first interval tick is immediate. Missed ticks use `Skip`. A diagnostic route failure, including a non-retryable data-contract failure, changes readiness state and audit output but never exits the resident monitor. A hard audit failure is logged explicitly and remains retryable by the supervisor; it must not be represented as a ready state.

### Steps

- [ ] Replace the old source-order assertion with a RED test describing the new lifecycle.

```rust
#[test]
fn br246_resident_producers_do_not_wait_for_static_data_readiness() {
    let source = include_str!("main.rs");
    assert!(!source.contains("external_static_opening_readiness().await"));
    assert!(source.contains("tokio::spawn(opening_static_readiness_loop())"));
    assert!(source.contains("p01::p01_scheduler_loop()"));
    assert!(source.contains("monitor_loop()"));
    assert!(source.contains("news_monitor_loop(selection_v2_enabled)"));
    assert!(source.contains("data_mode_monitor_loop()"));
}
```

- [ ] Add an async RED scheduler test using a short interval and an injected hook. It must observe the first call immediately, a second call after the period, and continued execution after the hook records a failure state.

```rust
#[tokio::test]
async fn br246_static_failures_do_not_complete_the_resident_scheduler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(tokio::sync::Notify::new());
    // Spawn run_opening_static_readiness_scheduler with a 50 ms interval.
    // The hook increments calls and notifies even when it models unavailable.
    // Assert first and second notifications, then abort the intentionally infinite task.
}
```

- [ ] Run RED.

```bash
cargo test --bin monitor br246_resident_producers_do_not_wait_for_static_data_readiness -- --nocapture --test-threads=1
cargo test --bin monitor br246_static_failures_do_not_complete_the_resident_scheduler -- --nocapture --test-threads=1
```

Expected RED: the blocking call still exists and the scheduler seam is missing.

- [ ] Implement `evaluate_opening_static_readiness_once` with this order:

```text
external_static_opening_diagnostics
  -> audit_opening_diagnostic_report (all returned attempts)
  -> derive ready/degraded banner from production_ready + route counts
  -> log only state changes
```

If diagnostics fails before a report exists (connection/capability authority), call the existing `audit_opening_readiness_failure` and emit the safe typed failure banner. Never use the failure message as a reason code and never synthesize 9/9.

- [ ] Remove the entire normal-production blocking loop around `external_static_opening_readiness().await`. Keep `opening_readiness=not_applicable` behavior for test/review modes.

- [ ] Spawn `opening_static_readiness_loop()` in the service branch background task vector. It must be supervised and aborted/drained by the existing `supervise_long_running_lifecycle` shutdown path.

- [ ] Keep `external_static_opening_readiness()` reachable from the bundle probe/release verification; do not rename it or redirect it to diagnostics.

- [ ] Run GREEN and the surrounding monitor tests.

```bash
cargo test --bin monitor br246_ -- --nocapture --test-threads=1
cargo test --bin monitor tests_br238_opening_readiness -- --nocapture --test-threads=1
```

- [ ] Static safety scan.

```bash
rg -n "external_static_opening_readiness\(\)\.await|opening_static_readiness_loop|exit_after_jsonl_writer" src/bin/monitor/main.rs
rustfmt --edition 2021 --check src/bin/monitor/main.rs
git diff --check -- src/bin/monitor/main.rs
```

The only normal production use of `exit_after_jsonl_writer` around this area must remain for hard authority/lifecycle failures, not diagnostic data-route failures.

- [ ] Commit the scoped lifecycle change.

```bash
git add src/bin/monitor/main.rs
git commit -m "feat: keep monitor resident during data outages" -m "Refs: docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md"
```

---

## Task 3: Move Position-Chain Refresh Into Its Own Retry Loop

**Files:**

- Modify: `src/bin/monitor/main.rs:900-955`
- Modify: `src/bin/monitor/main.rs:3930-3985`
- Modify: `src/bin/monitor/main.rs:5015-5038`
- Modify: `src/bin/monitor/main.rs:5160-5178`
- Test: `src/bin/monitor/main.rs` adjacent BR-170/BR-246 tests

### Contract

Add:

```rust
const POSITION_CHAIN_REFRESH_PERIOD: Duration = Duration::from_secs(300);

fn position_chain_refresh_interval(period: Duration) -> tokio::time::Interval;

async fn run_position_chain_refresh_scheduler<F, Fut>(
    interval: tokio::time::Interval,
    refresh: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<PositionChainRefreshReport, String>>;

async fn position_chain_refresh_loop();
```

The first tick is immediate and missed ticks use `Skip`. Loading positions, provider refresh, or partial-item failures remain explicit in logs/audit and are retried after 300 seconds. Dependent consumers still require exact current evidence and remain fail-closed until it exists.

### Steps

- [ ] Replace `br170_position_chain_refresh_precedes_long_running_consumers` with a RED lifecycle test.

```rust
#[test]
fn br246_position_chain_refresh_is_background_and_cannot_block_main_loops() {
    let source = include_str!("main.rs");
    let service_branch = source
        .rsplit_once("} else if !selection_cli.requires_service_enablement()")
        .expect("service branch")
        .1;
    assert!(!service_branch.contains(
        "let position_chain_report = match refresh_startup_position_chains().await"
    ));
    assert!(service_branch.contains("tokio::spawn(position_chain_refresh_loop())"));
    assert!(service_branch.contains("let main_loops = async"));
}
```

- [ ] Add a RED async scheduler test that injects `Err("TEST_CODE unavailable")` on the immediate tick and `Ok(test_report)` on the next tick. Assert both calls occur and the scheduler remains alive after the first error.

- [ ] Run RED.

```bash
cargo test --bin monitor br246_position_chain_refresh_ -- --nocapture --test-threads=1
```

- [ ] Implement the immediate interval with `tokio::time::interval(period)` and `MissedTickBehavior::Skip`; do not use a blocking `sleep` loop that can accumulate ticks.

- [ ] Keep `refresh_startup_position_chains()` as the one real acquisition/persistence operation. Update its log prefix from startup-only wording to `[position-chain][BR-170][BR-246]` without changing its data or persistence contract.

- [ ] Remove the blocking call and its `exit_after_jsonl_writer` branch from service startup. Spawn `position_chain_refresh_loop()` in `background_tasks` so lifecycle supervision owns it.

- [ ] Verify independent consumers still do not treat a missing chain as a real chain. Run existing BR-170 exact-refresh tests plus the new scheduler suite.

```bash
cargo test --bin monitor br246_position_chain_refresh_ -- --nocapture --test-threads=1
cargo test --workspace br170_ -- --nocapture --test-threads=1
```

- [ ] Format, diff-check, and commit only scoped hunks.

```bash
rustfmt --edition 2021 --check src/bin/monitor/main.rs
git diff --check -- src/bin/monitor/main.rs
git add src/bin/monitor/main.rs
git commit -m "feat: retry position chains without stopping monitor" -m "Refs: docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md"
```

---

## Task 4: Prove Degraded Messages Stay Truthful and Unsafe Actions Stay Closed

**Files:**

- Modify tests only if needed: `src/bin/monitor/main.rs`
- Modify tests only if needed: `src/bin/monitor/push_templates.rs`
- Modify tests only if needed: `src/monitor/data_mode.rs`
- Review only: `src/bin/monitor/notify.rs`, order/paper-trade call sites

### Steps

- [ ] Run the existing DataMode reminder and banner tests first. Do not change production rendering unless a test exposes a real BR-246 gap.

```bash
cargo test --bin monitor br135_ -- --nocapture --test-threads=1
cargo test --bin monitor data_mode -- --nocapture --test-threads=1
cargo test --lib monitor::data_mode::tests:: -- --nocapture --test-threads=1
```

- [ ] Add a focused regression asserting an Unsafe banner names the actual missing capability set and continues to forbid price/order-book conclusions. Use only `TEST_CODE` fixtures.

```rust
#[test]
fn br246_unsafe_banner_reports_missing_capabilities_without_price_advice() {
    let banner = test_banner_with_missing(
        &["Quote", "MoneyFlow", "News", "OrderBook"],
        DataMode::Unsafe,
    );
    let text = banner.render_header();
    assert!(text.contains("Quote/MoneyFlow/News/OrderBook"));
    assert!(!text.contains("买入价"));
    assert!(!text.contains("盘口承接"));
}
```

- [ ] Add or retain source-level assertions that BR-246 introduces no direct/generic sink call. All notifications must still pass through the existing presentation token, Launch, L5, durable/audit, and authoritative receipt path.

- [ ] Run negative producer/order regressions for missing realtime evidence. Select existing exact filters discovered with `rg -n "missing.*quote|quote_stale|order.*Unavailable|sink_calls.*0"`; do not invent a permissive fallback test.

- [ ] Run scoped checks.

```bash
rustfmt --edition 2021 --check src/bin/monitor/main.rs src/bin/monitor/push_templates.rs src/monitor/data_mode.rs
git diff --check -- src/bin/monitor/main.rs src/bin/monitor/push_templates.rs src/monitor/data_mode.rs
rg -n "unwrap_or_default|Default::default\(\)|mock|fallback" src/bin/monitor/main.rs src/bin/monitor/push_templates.rs src/monitor/data_mode.rs
```

- [ ] If no production rendering change was needed, do not create a no-op code commit. If test-only coverage changed, commit only those tests.

```bash
git add src/bin/monitor/main.rs src/bin/monitor/push_templates.rs src/monitor/data_mode.rs
git commit -m "test: lock degraded resident safety behavior" -m "Refs: docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md"
```

---

## Task 5: Gate C — Full Repository Verification

**Files:**

- Update: `docs/business_rules.md` BR-246 status only after all commands below pass
- Review: `README.md` release status must remain suspended if Gate D is incomplete

### Steps

- [ ] Ensure no competing Cargo/rustc process owns the shared target before each Cargo command.

- [ ] Run formatting.

```bash
cargo fmt --all -- --check
```

- [ ] Run strict Clippy.

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

- [ ] Run all workspace tests serially.

```bash
cargo test --workspace --all-targets --all-features -- --test-threads=1
```

- [ ] Run compliance.

```bash
bash tools/compliance/check.sh
```

- [ ] Run focused source/audit checks.

```bash
git diff --check
rg -n "BR-246" docs/business_rules.md docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md src/bin/monitor/main.rs src/data_gateway/grpc_source.rs
```

- [ ] If any command fails, classify the root cause and return to Gate A or B per AGENTS.md §3.2. Do not mark the task complete or change README to release-ready.

- [ ] When all Gate C commands pass, change BR-246 status to `🟡 Gate C passed; Gate D live/coverage pending`, rerun both business-rule and design-contradiction checks, then commit the exact docs.

```bash
git add docs/business_rules.md
git add -f docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md docs/superpowers/plans/2026-08-20-resident-monitor-degraded-readiness.md
git commit -m "docs: register resident degraded readiness" -m "Refs: docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md"
```

---

## Task 6: Gate D — Coverage and Live Recovery Evidence

**Files:**

- Update only after evidence: `README.md`
- Update: relevant `docs/operations/` release evidence without account/holding identities
- Do not modify/delete immutable production audit rows

### Steps

- [ ] Build release server, monitor, and strict probe using the documented feature profiles.

```bash
cargo build --release --bin stock_data_service --features magic-gateway
cargo build --release --bin monitor
cargo build --release --bin grpc_bundle_probe
```

- [ ] Run coverage and enforce AGENTS.md thresholds. Do not reuse the earlier 76.82%/76.84% report as passing evidence.

```bash
cargo llvm-cov --workspace --all-targets --all-features --summary-only
```

Acceptance: global line coverage is at least 80%; core trading/data links are at least 95% under the repository's core-scope verifier. Add tests, not exclusions, for any attributable gap.

- [ ] With the currently documented gRPC fault present, start the current release pair under the production singleton lease and verify for at least two static supervisor periods:

```text
resident monitor remains alive
static diagnostic attempts = 9 each cycle
InstrumentNews remains explicit unavailable/invalid_evidence
DataMode/risk message obtains a real typed Accepted receipt
price advice calls = 0
paper execution calls = 0
order calls = 0
```

- [ ] After the gRPC route is fixed, do not restart the monitor. Verify the same process changes static readiness to 9/9 and the formerly blocked producer becomes eligible at its next legal schedule/freshness window.

- [ ] Run the strict release probe separately. It must report exact 9/9 before production cutover; 8/9 remains a release blocker even though the monitor is resident.

- [ ] Preserve privacy in evidence: record domain-separated identities/hashes and counts required by the verifier, never real holding codes, account identifiers, tokens, cookies, or raw upstream diagnostics.

- [ ] Obtain auditor sign-off. Only then update README from suspended/in-progress to the exact verified release state.

- [ ] Prepare the required PR description fields:

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-08-20-resident-monitor-degraded-readiness-design.md`

### Data-Redlines
- [2.1] No mock/default data; unavailable capabilities remain explicit
- [2.4] Consumer freshness gates unchanged
- [2.7] Every diagnostic attempt is immutably audited
- [2.8] Background refresh functions perform real provider/DB work
- [2.10] BR-246 registered before implementation

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| `external_static_opening_readiness` | adopt | strict release/probe 9/9 authority remains unchanged |
| `external_static_opening_diagnostics` | adopt | evidence-preserving resident observation |
| `opening_live_readiness_loop` | adopt | existing live observation remains independent |
| blocking startup loops | reject | one capability failure must not silence all governed producers |

### Threshold-Proof
- no threshold/config change

### Business-Rules
- BR-246, BR-238, BR-170, BR-159, BR-135

### Rollback
- revert BR-246 implementation commits; keep immutable audit and delivery receipts; rebuild prior compatible release pair
```

- [ ] Create the PR and merge only after every checklist item, Gate C, Gate D, coverage, live recovery, and reviewer sign-off are green. Direct push to `master` is not an allowed substitute for the PR.

---

## Final Verification Checklist

- [ ] Strict BR-238 probe still requires exact 9/9.
- [ ] Static data failure never exits or delays the resident producer loops.
- [ ] Static supervisor runs immediately and then every 30 seconds with skipped backlog.
- [ ] Position-chain refresh runs immediately and then every 300 seconds with skipped backlog.
- [ ] Every static diagnostic route outcome is immutably audited.
- [ ] DataMode/risk notifications continue through the real governed sink with explicit missing capability text.
- [ ] Missing-evidence price advice, order-book conclusions, paper executions, and orders remain zero-call fail-closed.
- [ ] Recovery occurs in the same monitor process without restart.
- [ ] `cargo fmt`, strict Clippy, full workspace tests, and compliance all pass.
- [ ] Global coverage is at least 80% and core coverage is at least 95%.
- [ ] Live typed receipt, audit join, strict 9/9 recovery, privacy review, and auditor sign-off exist.
- [ ] PR fields are complete and all ignored design/plan documents are force-added explicitly.
