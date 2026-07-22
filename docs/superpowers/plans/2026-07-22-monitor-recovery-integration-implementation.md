# Monitor Recovery Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the three reviewed core modules into the production monitor so audit health gates sinks, real capability probes drive truthful messages, and persisted user closing valuation is delivered without leaking sensitive details or authorizing actions.

**Architecture:** This is deliberately serial because `main.rs`, `notify.rs`, `push_templates.rs`, and `v14_adapter.rs` are shared choke points. The integration layer orchestrates only: audit/portfolio/data-mode modules retain validation and persistence, while templates render typed persisted views.

**Tech Stack:** Existing Tokio monitor binary, L4-L7 governance, real Tencent/RustDX providers, Diesel/SQLite, existing `PushKind` catalog and process-isolation tests.

---

## Preconditions and owned files

Preconditions: audit, valuation and capability commits have passed focused tests and an independent review. The integrator owns all files below until this plan completes.

- Create: `src/bin/monitor/data_mode_probe.rs` — independent real Quote probe/scheduler seam.
- Create: `src/bin/monitor/closing_valuation_runtime.rs` — latest snapshot → real closes → persisted run orchestration.
- Modify: `src/bin/monitor/main.rs` — startup order and schedulers.
- Modify: `src/bin/monitor/notify.rs` — audit gate, typed settlement, valuation identity/privacy.
- Modify: `src/bin/monitor/push_templates.rs` — I-01 blocking boundary, Banner/DataMode/valuation rendering.
- Modify: `src/bin/monitor/v14_adapter.rs` — dedicated governance identity without misusing security code.
- Modify: `src/bin/monitor/market_data.rs` — remove Quote-health dependence on position freshness and record typed attempts.
- Modify: `tests/monitor_help_isolation.rs` — startup/audit/process regressions.
- Modify if exhaustive mappings require: `src/push_l2/template.rs`, `src/push_l5/*`, `src/push_l7/*`.

### Task 1: Audit preflight before any sink or ordinary producer

- [ ] **Step 1: Add a RED process test with a corrupted isolated chain**

Extend `tests/monitor_help_isolation.rs` using its isolated command builder. Seed a syntactically complete but hash-invalid `<temp>/prod/2026.jsonl`, start the normal enabled monitor with dry-run sink counters, then assert:

```text
exit != 0 or controlled AuditDegraded supervisor state
stderr/stdout contains "AuditDegraded"
ordinary sink call count/file count = 0
original bad-chain bytes unchanged
```

The test must also prove `--help` and disabled bare monitor still exit before creating an audit directory per BR-141.

- [ ] **Step 2: Run RED**

```bash
cargo test --test monitor_help_isolation audit_degraded_blocks_sink_initialization -- --exact --test-threads=1
```

Expected: current monitor initializes L6/news components before any audit preflight.

- [ ] **Step 3: Reorder startup in `main.rs`**

Required order:

```rust
// CLI/environment/service gates have already returned where appropriate.
let audit_receipt = tokio::task::spawn_blocking(
    stock_analysis::event::preflight_runtime_delivery_audit,
)
.await
.map_err(|error| format!("audit preflight join: {error}"))??;
log::info!("[AuditHealthy] year={} chain_verified=true", audit_receipt.year);

// Only after this point may l6_sink::sink_count(), NewsAggregator,
// producer tasks, and ordinary schedulers initialize.
```

On failure, emit only a local structured error and leave before sink/router/producer initialization. Do not attempt a push about the audit failure because it cannot satisfy 2.7.

- [ ] **Step 4: Add a final governor pre-sink health check**

Every generic/source-fact/valuation governor path must call the same read-only health function after governance reservation and immediately before invoking L6/legacy sink. `Unverified` and `Degraded` return `PushOutcome::AuditUnavailableBeforeDelivery(reason_code)` and release the reservation.

- [ ] **Step 5: Run process and notify tests GREEN**

```bash
cargo test --test monitor_help_isolation -- --test-threads=1
cargo test --bin monitor notify::tests:: -- --test-threads=1
```

- [ ] **Step 6: Commit**

```bash
git add src/bin/monitor/main.rs src/bin/monitor/notify.rs tests/monitor_help_isolation.rs
git commit -m "fix(monitor): gate sinks on delivery audit health"
```

### Task 2: Physically delivered audit failure never resends

- [ ] **Step 1: Replace the old opposite RED test**

Replace `br137_sink_success_with_post_delivery_audit_failure_releases_identity_for_retry` with a behavior test that injects a counting sink and failing audit writer. It must assert:

```rust
assert!(matches!(outcome, PushOutcome::PhysicallyDeliveredAuditFailed(_)));
assert_eq!(sink_calls.load(Ordering::SeqCst), 1);
assert!(same_identity_is_committed());
assert!(matches!(runtime_delivery_audit_health(), AuditHealth::Degraded { .. }));
```

A second invocation must stop before the sink and keep the count at 1.

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor notify::tests::br145_physical_delivery_is_not_retried_after_audit_failure -- --exact --test-threads=1
```

Expected: current result is `SinkError` and reservation is released.

- [ ] **Step 3: Extend `PushOutcome` and integrate `settle_delivery`**

Add:

```rust
AuditUnavailableBeforeDelivery(String),
PhysicallyDeliveredAuditFailed(Vec<String>),
```

After sink/L7/hash-chain calls, use `stock_analysis::event::settle_delivery`. Apply `IdentityAction::Commit` whenever the sink accepted, even if post-delivery records failed; apply Release only when the sink did not accept or the pre-sink gate denied. On post-audit failure mark the runtime dispatcher degraded and create a non-sensitive append-only incident containing hashes/reason codes only.

Do not report `is_pushed=true` for `PhysicallyDeliveredAuditFailed`; BR-116 timers/latest-notified remain unconfirmed, while the committed identity prevents physical replay.

- [ ] **Step 4: Update every exhaustive outcome match**

Use multiline search:

```bash
rg -n -A8 'PushOutcome::' src/bin/monitor tests
```

Every caller must distinguish pre-sink unavailable, actual sink error, and physically-delivered audit failure. None may coerce the last case to retryable sink failure.

- [ ] **Step 5: Run GREEN and commit**

```bash
cargo test --bin monitor notify::tests:: -- --test-threads=1
cargo test --bin monitor main_tests:: -- --test-threads=1
git add src/bin/monitor/notify.rs src/bin/monitor/main.rs src/bin/monitor/v14_adapter.rs
git commit -m "fix(push): never resend accepted deliveries"
```

### Task 3: Tokio blocking boundary for I-01

- [ ] **Step 1: Add a RED async test**

In `push_templates.rs`, test an injected blocking loader that constructs and drops `reqwest::blocking::Client` from the helper; assert no Tokio runtime-drop panic and distinguish join error from loader error.

Target helper:

```rust
async fn load_sector_snapshot_for_dispatch(
    hhmm: String,
) -> Result<SectorSnapshot, String> {
    tokio::task::spawn_blocking(move || load_sector_snapshot_real(&hhmm))
        .await
        .map_err(|error| format!("I-01 blocking task join: {error}"))?
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor push_templates::tests::i01_blocking_loader_is_off_async_worker -- --exact --test-threads=1
```

Expected: current async dispatcher directly executes the blocking loader.

- [ ] **Step 3: Replace both known unsafe entrances**

Use the helper in `dispatch_intraday_market_daily_result` and the `run_push_dry_run` I-01 branch. Do not hide errors or advance timers on JoinError/provider failure.

- [ ] **Step 4: Add the process panic regression and commit**

```bash
cargo test --bin monitor push_templates::tests:: -- --test-threads=1
cargo test --test monitor_help_isolation push_dry_run_does_not_drop_blocking_runtime_on_async_worker -- --exact --test-threads=1
git add src/bin/monitor/push_templates.rs src/bin/monitor/main.rs tests/monitor_help_isolation.rs
git commit -m "fix(monitor): isolate I-01 blocking provider"
```

Record the additional I-02/I-03/D-01/P-02/A paths found by the audit as explicit follow-up findings; do not claim a repository-wide blocking audit from this scoped fix.

### Task 4: Independent real Quote capability probe

- [ ] **Step 1: Write RED tests around an injected transport seam**

Create `src/bin/monitor/data_mode_probe.rs`:

```rust
pub struct QuoteProbeResult {
    pub provider: String,
    pub provider_observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub locally_observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub price: f64,
    pub previous_close: f64,
}

pub enum QuoteProbeDisposition {
    NotExpected,
    Attempted(Result<QuoteProbeResult, String>),
}

pub async fn run_quote_capability_probe_at(
    now: chrono::DateTime<chrono::Local>,
) -> Result<QuoteProbeDisposition, String>;
```

Tests use a local protocol fixture/closure and assert no portfolio/database read, price >0/finite, provider timestamp no older than 5 seconds when expected, absolute change no greater than 20%, and closed/lunch sessions return `NotExpected` without success/failure fabrication.

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor data_mode_probe::tests:: -- --test-threads=1
```

Expected: module does not exist.

- [ ] **Step 3: Implement production probe**

During Auction/Morning/Afternoon, query a fixed liquid production probe symbol through the existing Tencent provider carrying upstream time. The production constructor uses a real six-digit A-share symbol; tests use `TEST_CODE_`. Record attempt then success/failure in the capability tracker. During Closed/LunchBreak/AfterHours, set `expected_now=false` and do not call the provider.

The fixed symbol is a liveness probe only and never enters recommendations, positions, audit identity or user messages.

- [ ] **Step 4: Decouple existing position quote path**

In `market_data.rs`, remove the implication “position source freshness passed, therefore Quote capability is healthy.” Position freshness continues to gate position valuation/position-specific consumers, but Quote health comes from real quote attempts independently.

- [ ] **Step 5: Wire first probe before DataMode notification eligibility**

Startup publishes a fail-closed Warming banner, starts the independent probe, and calls DataMode notification planning only after all supported capabilities have a first attempt disposition. OrderBook is registered Unsupported at initialization.

- [ ] **Step 6: Run and commit**

```bash
cargo test --bin monitor data_mode_probe::tests:: -- --test-threads=1
cargo test --bin monitor main_tests::data_mode -- --test-threads=1
git add src/bin/monitor/data_mode_probe.rs src/bin/monitor/market_data.rs src/bin/monitor/main.rs
git commit -m "fix(data-mode): probe quotes independently"
```

### Task 5: Truthful Banner and capability messages

- [ ] **Step 1: Write RED Banner tests**

Refactor `BannerCtx` into typed action/display facts:

```rust
pub struct ActionAccountFacts {
    pub account_mode: AccountMode,
    pub metrics_complete: bool,
    pub total_pos: Option<u8>,
    pub today_pnl: Option<f64>,
}

pub enum ClosingValuationAvailability {
    Available,
    Partial { valued: usize, total: usize },
    Unavailable,
}

pub struct DisplayAccountFacts {
    pub realtime_account_connected: bool,
    pub closing_valuation: ClosingValuationAvailability,
}
```

Assert the no-broker/available case renders exactly the semantic fields:

```text
[🔴 Frozen | 实时账户未接入 | 本地收盘估值可用 | 数据Warming]
```

and contains none of `仓位缺失`, `日盈亏缺失`, `仓位0成`.

- [ ] **Step 2: Keep action conversion fail-closed**

`paper_risk_context_from_banner()` reads only `ActionAccountFacts`; `metrics_complete=false` still returns BR-134 error even when closing valuation is Available.

- [ ] **Step 3: Write RED capability renderer tests**

Renderer consumes `CapabilityDiagnosticSnapshot` and prints each capability state plus provider/last_success/age/last_error/next_retry only when present. OrderBook Unsupported prints no ETA. No output contains static `Quote 恢复后`.

- [ ] **Step 4: Replace mode-only notification dedup with confirmed fingerprint**

The monitor holds `ConfirmedDiagnosticState`. Warming is never sent as `未建立 → Unsafe`; the first completed non-healthy diagnostic is governed once. Stale→Failed can notify even if aggregate mode remains Unsafe. Only `PushOutcome::Pushed` plus complete authoritative records commits the fingerprint/reminder time.

- [ ] **Step 5: Run and commit**

```bash
cargo test --bin monitor push_templates::tests::incomplete_banner -- --test-threads=1
cargo test --bin monitor push_templates::tests::data_mode -- --test-threads=1
cargo test --bin monitor main_tests::data_mode -- --test-threads=1
git add src/bin/monitor/push_templates.rs src/bin/monitor/main.rs src/bin/monitor/v14_adapter.rs
git commit -m "fix(messages): render account and data facts truthfully"
```

### Task 6: Closing valuation runtime and persisted-view message

- [ ] **Step 1: Write RED runtime orchestration tests**

Create `closing_valuation_runtime.rs` with dependency-injected unit tests for:

```rust
pub fn calculate_and_persist_latest_closing_valuation_at(
    now: chrono::DateTime<chrono::Local>,
) -> Result<SaveClosingValuationReceipt, String>;
```

Test no snapshot → explicit unavailable, latest snapshot selected, target date from `calendar::latest_completed_trading_day_at`, per-symbol fetch failure isolated, and persistence occurs before any renderer/dispatch call.

- [ ] **Step 2: Run RED then implement production orchestration**

```bash
cargo test --bin monitor closing_valuation_runtime::tests:: -- --test-threads=1
```

Production code loads the latest user snapshot, fetches each RustDX unadjusted close inside `spawn_blocking`, calculates the run, persists it, and returns a receipt/view hash plus coverage only to logs.

- [ ] **Step 3: Add a dedicated governed identity**

Add `PushKind::PositionClosingValuation` and a governor interface that accepts security code separately from a non-sensitive governance identity:

```rust
pub async fn push_governor_v3_with_identity(
    text: &str,
    kind: PushKind,
    security_code: Option<&str>,
    governance_identity: &str,
    privacy: PushPrivacy,
) -> PushOutcome;
```

For valuation, `security_code=None`; derive event ID/dedup identity from the domain-separated hash of `(snapshot_id, price_trade_date, calculation_version)`. Update reserve/commit/rollback to use that identity without classifying the event as BR-137 source-fact or weakening DataMode governance.

- [ ] **Step 4: Add sensitive-message handling**

```rust
pub enum PushPrivacy {
    Standard,
    SensitivePortfolio { summary_hash: String, item_count: usize },
}
```

The real authorized sink receives the full valuation body. Dry-run stdout/stderr, pre-delivery push log, L7, delivery audit and aggregate health logs receive only summary hash, count, coverage, status and rendered length. The controlled valuation tables remain the durable detailed store.

- [ ] **Step 5: Write the valuation renderer against persisted view only**

Message contains snapshot confirmation time, price trade date/provider, coverage, each persisted successful item’s quantity/cost/close/value/unrealized P&L/return/daily price P&L, failed item reason labels, totals only when coverage >0, and the fixed footer:

```text
本地收盘估值，非券商实时盈亏、非下单指令
```

Reject an in-memory/non-persisted valuation object at the interface by accepting only `ClosingValuationView` returned by the repository.

- [ ] **Step 6: Schedule after completed close and on a newer user snapshot**

The scheduler runs after the target trading day is complete and when a run identity has not already been confirmed/physically sealed. New user snapshot identity can produce a new same-day run; unchanged identity is deduped. AuditDegraded prevents sink invocation.

- [ ] **Step 7: Add privacy and outcome tests**

Capture dry-run output, push-log file, delivery audit, L7 rows and aggregate logs; assert none contains fixture code/name/quantity/cost/value/P&L while the injected authorized sink receives the full expected body.

```bash
cargo test --bin monitor closing_valuation_runtime::tests:: -- --test-threads=1
cargo test --bin monitor push_templates::tests::position_closing_valuation -- --test-threads=1
cargo test --bin monitor notify::tests::sensitive_portfolio -- --test-threads=1
```

- [ ] **Step 8: Commit**

```bash
git add src/bin/monitor/closing_valuation_runtime.rs src/bin/monitor/main.rs src/bin/monitor/notify.rs src/bin/monitor/push_templates.rs src/bin/monitor/v14_adapter.rs src/push_l2 src/push_l5 src/push_l7
git commit -m "feat(monitor): push persisted closing valuation safely"
```

Only add push-layer files actually changed; do not stage unrelated paths.

### Task 7: Integration and release verification

- [ ] **Step 1: Verify actual production imports/call sites with multiline-aware search**

```bash
rg -n -A4 'preflight_runtime_delivery_audit\(|calculate_and_persist_latest_closing_valuation_at\(|run_quote_capability_probe_at\(|latest_persisted_valuation_view\(' src/bin/monitor
```

Expected: each interface has an import and a live `main.rs`/scheduler/governor call site.

- [ ] **Step 2: Run mandatory Gate C**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo build --release --bin monitor
```

Expected: all exit 0.

- [ ] **Step 3: Run coverage and isolated smoke**

```bash
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test
```

Expected: coverage thresholds pass; output shows AuditHealthy and truthful capability state, never the sensitive fixture/body.

- [ ] **Step 4: Validate production evidence or report the exact blocker**

For the current date, verify the new PushKind has a real producer path and delivery audit event. If no user snapshot has been imported, startup must visibly report `disabled=no_user_position_snapshot`; this is **In Progress / Blocked for canary**, not completion.

- [ ] **Step 5: Commit only evidence/doc updates**

```bash
git diff --check
git status --short
```

Then commit the scoped validation/PR-evidence document without adding local user snapshot files, HTML reports, production logs, databases or audit JSONL.

## Rollback order

1. Disable valuation scheduling/diagnostic rendering through the documented forward-compatible switch; keep AuditDegraded gate active.
2. `git revert` the serial integration commits in reverse order.
3. Revert parallel core commits only if their own interfaces are defective.
4. Never delete/rewrite audit JSONL, incidents, user snapshots, valuation runs or delivery identities.

Status is **Done** only after all AGENTS Part 4 conditions, independent review, and required real-path evidence pass.
