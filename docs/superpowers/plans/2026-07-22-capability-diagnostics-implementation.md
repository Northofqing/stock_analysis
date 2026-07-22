# Capability Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Represent each data capability truthfully as Warming, Healthy, Stale, Failed, or Unsupported while keeping existing Full/Degraded/Unsafe/Down governance fail-closed.

**Architecture:** A capability tracker is a deep module with injected clocks and explicit attempt facts. It produces a diagnostic snapshot and a separate governance projection; the monitor integration later owns real scheduling and rendering, so startup display changes cannot accidentally authorize trading.

**Tech Stack:** Rust, chrono wall-clock evidence, std::time::Instant monotonic age, RwLock, existing `monitor::data_mode`, provider adapters and A-share calendar sessions.

---

## File map

- Modify or split: `src/monitor/data_mode.rs` — capability state, tracker, snapshot, governance projection.
- Optional create: `src/monitor/capability_health.rs` — use only if it makes `data_mode.rs` smaller while preserving one public interface.
- Modify: `src/monitor/mod.rs` — re-export only.
- Modify focused provider success/failure markers where no bin-monitor file is involved: `src/data_provider/fallback.rs`, `src/data_provider/announcement.rs`, `src/market_analyzer/sector_monitor.rs`, `src/broker.rs`.
- Do not modify in this parallel task: `src/bin/monitor/main.rs`, `market_data.rs`, `push_templates.rs`, `notify.rs`, `v14_adapter.rs`.

### Task 1: Five-state capability observation

- [ ] **Step 1: Write the first RED public-interface test**

Target interface:

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CapabilityState {
    Warming,
    Healthy,
    Stale,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityObservation {
    pub capability: Capability,
    pub state: CapabilityState,
    pub expected_now: bool,
    pub provider: Option<String>,
    pub provider_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub locally_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub last_success_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub age_secs: Option<u64>,
    pub last_error_code: Option<String>,
    pub retryable: Option<bool>,
    pub next_retry_at: Option<chrono::DateTime<chrono::FixedOffset>>,
}
```

Assert a newly registered supported capability is Warming and OrderBook registered unsupported is Unsupported with no ETA/retry.

- [ ] **Step 2: Run RED**

```bash
cargo test --lib monitor::data_mode::tests::br148_new_capabilities_are_warming_or_unsupported -- --exact --test-threads=1
```

Expected: compile failure before the types exist.

- [ ] **Step 3: Implement a tracker with injected time**

Expose a small interface:

```rust
pub struct CapabilityTracker { /* private map and clock state */ }

pub struct CapabilityNow {
    pub wall: chrono::DateTime<chrono::FixedOffset>,
    pub monotonic: std::time::Instant,
    pub expected_now: bool,
}

impl CapabilityTracker {
    pub fn new() -> Self;
    pub fn register_supported(&self, capability: Capability) -> Result<(), String>;
    pub fn register_unsupported(&self, capability: Capability) -> Result<(), String>;
    pub fn record_attempt_started(
        &self,
        capability: Capability,
        provider: &str,
        locally_observed_at: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<(), String>;
    pub fn record_success(&self, success: CapabilitySuccess) -> Result<(), String>;
    pub fn record_failure(&self, failure: CapabilityFailure) -> Result<(), String>;
    pub fn snapshot_at(&self, now: CapabilityNow) -> Result<CapabilityDiagnosticSnapshot, String>;
}
```

`CapabilityNow` contains both wall time and monotonic `Instant`. Age is computed only from monotonic instants. Provider time remains optional and never receives local time as a fallback.

- [ ] **Step 4: Add state-transition tests and run GREEN**

Test vertical transitions: Warming→Healthy, Healthy→Stale when expected, Healthy remains non-stale when `expected_now=false`, attempt failure→Failed, failure preserves `last_success_at`, success clears error, Unsupported rejects attempt/success mutation, and lock poison returns explicit error.

```bash
cargo test --lib monitor::data_mode::tests::br148_ -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 5: Commit the state model**

```bash
git add src/monitor/data_mode.rs src/monitor/capability_health.rs src/monitor/mod.rs
git commit -m "feat(data-mode): track capability diagnostics"
```

Omit `capability_health.rs` from `git add` if the split was not needed.

### Task 2: Governance projection and Warming suppression

- [ ] **Step 1: Write RED tests separating display from governance**

Add/extend:

```rust
#[derive(Clone, Debug)]
pub struct CapabilityDiagnosticSnapshot {
    pub observations: Vec<CapabilityObservation>,
    pub fingerprint: String,
    pub first_probe_complete: bool,
}

#[derive(Clone, Debug)]
pub struct DataHealth {
    pub mode: DataMode,
    pub diagnostics: CapabilityDiagnosticSnapshot,
    pub notification_eligible: bool,
    pub prev_mode: Option<DataMode>,
}
```

Assertions:

```rust
assert_eq!(health.mode, DataMode::Unsafe);
assert!(!health.notification_eligible);
assert_eq!(quote.state, CapabilityState::Warming);
```

- [ ] **Step 2: Run RED**

```bash
cargo test --lib monitor::data_mode::tests::warming_is_fail_closed_without_unsafe_notification -- --exact --test-threads=1
```

Expected: current implementation incorrectly treats Missing Quote as notification-eligible Unsafe.

- [ ] **Step 3: Implement the projection**

Rules:

```text
Quote Warming        -> governance Unsafe, notification_eligible=false
Quote Healthy        -> evaluate other critical capabilities
Quote Stale/Failed   -> governance Unsafe, notification_eligible=true
Kline/MoneyFlow/News Warming -> governance Degraded, notification_eligible=false until every supported capability attempted
Kline/MoneyFlow/News Stale/Failed -> Degraded
OrderBook Unsupported -> diagnostics only, no degradation, no ETA
expected_now=false   -> do not manufacture stale/failed solely from advancing age
```

Fingerprint input is stable sorted `(capability,state,provider,last_error_code,retryable,expected_now)` and excludes age/timestamps/next-retry so the same outage does not notify every minute. Hash with domain `stock_analysis.capability_diagnostic.v1`.

- [ ] **Step 4: Replace static ETA behavior**

Remove `DataHealth.eta = Some("Quote 恢复后")`. Each observation carries actual `next_retry_at`; Unsupported always has none. Keep a compatibility projection only if an existing caller still needs it, and mark that caller for serial removal in the integration plan.

- [ ] **Step 5: Run all existing and new data-mode tests**

```bash
cargo test --lib monitor::data_mode::tests:: -- --test-threads=1
```

Expected: existing Full/Degraded/Unsafe governance semantics remain green except tests intentionally updated for BR-148 startup behavior.

- [ ] **Step 6: Commit**

```bash
git add src/monitor/data_mode.rs src/monitor/capability_health.rs
git commit -m "fix(data-mode): suppress warming false alarms"
```

### Task 3: Explicit production success/failure evidence

- [ ] **Step 1: Add typed attempt facts**

Use closed inputs:

```rust
pub struct CapabilitySuccess {
    pub capability: Capability,
    pub provider: String,
    pub provider_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub locally_observed_at: chrono::DateTime<chrono::FixedOffset>,
}

pub struct CapabilityFailure {
    pub capability: Capability,
    pub provider: String,
    pub locally_observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub reason_code: String,
    pub retryable: bool,
    pub next_retry_at: Option<chrono::DateTime<chrono::FixedOffset>>,
}
```

Reject blank provider/reason, future provider time, a success without an actually completed provider call, and a failure without an attempt.

- [ ] **Step 2: Replace success-only markers in library-owned producers**

At each real acquisition boundary, call `record_attempt_started` before the actual provider operation, then exactly one of `record_success` or `record_failure`:

```text
src/data_provider/fallback.rs              Kline, provider=resolved real source name
src/data_provider/announcement.rs          News, provider=real announcement provider
src/market_analyzer/sector_monitor.rs      MoneyFlow, provider=eastmoney
src/broker.rs                              Quote, provider=tencent (existing execution quote only)
```

Keep original provider errors as return values. Tracking failure is also explicit and must not convert an acquisition failure into success.

- [ ] **Step 3: Add focused provider-marker tests**

Use existing local fixtures/parser seams with `TEST_CODE_` identities. Assert success records provider/local time, and HTTP/protocol failures record a closed reason without becoming an empty-success batch.

```bash
cargo test --lib data_provider::fallback::tests:: -- --test-threads=1
cargo test --lib data_provider::announcement::tests:: -- --test-threads=1
cargo test --lib market_analyzer::sector_monitor::tests:: -- --test-threads=1
cargo test --lib broker::tests:: -- --test-threads=1
```

- [ ] **Step 4: Commit**

```bash
git add src/monitor/data_mode.rs src/data_provider/fallback.rs src/data_provider/announcement.rs src/market_analyzer/sector_monitor.rs src/broker.rs
git commit -m "feat(data-mode): retain provider attempt facts"
```

### Task 4: Diagnostic fingerprint confirmation state

- [ ] **Step 1: Write a RED test for confirmation-only advancement**

Add a small state holder near `PersistentUnsafeReminder`:

```rust
#[derive(Debug, Default)]
pub struct ConfirmedDiagnosticState {
    last_confirmed_fingerprint: Option<String>,
}

impl ConfirmedDiagnosticState {
    pub fn should_dispatch(&self, snapshot: &CapabilityDiagnosticSnapshot) -> bool;
    pub fn record_confirmed(&mut self, fingerprint: String);
}
```

Assert same fingerprint suppresses, Stale→Failed dispatches even while mode remains Unsafe, age-only changes suppress, and unconfirmed attempts never advance state.

- [ ] **Step 2: Run RED then implement GREEN**

```bash
cargo test --lib monitor::data_mode::tests::diagnostic_fingerprint_advances_only_after_confirmation -- --exact --test-threads=1
```

Expected: RED before implementation, PASS after minimal code.

- [ ] **Step 3: Commit**

```bash
git add src/monitor/data_mode.rs src/monitor/capability_health.rs
git commit -m "feat(data-mode): confirm diagnostic fingerprints"
```

## Focused completion checks

- [ ] **Step 1: Run format, strict Clippy and focused tests**

```bash
cargo fmt --all -- --check
cargo clippy --lib --all-features -- -D warnings
cargo test --lib monitor::data_mode::tests:: -- --test-threads=1
cargo test --lib data_provider::fallback::tests:: -- --test-threads=1
cargo test --lib data_provider::announcement::tests:: -- --test-threads=1
cargo test --lib market_analyzer::sector_monitor::tests:: -- --test-threads=1
cargo test --lib broker::tests:: -- --test-threads=1
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 2: Prove no static recovery text/default-fresh path remains in the module**

```bash
rg -n 'Quote 恢复后|Capability::ALL.*fresh|unwrap_or_default\(' src/monitor/data_mode.rs src/monitor/capability_health.rs
```

Expected: no production match for static ETA or fabricated fresh capabilities; every `unwrap_or_default` match, if any, has a documented non-evidence reason.

- [ ] **Step 3: Hand off integration requirements**

Handoff must state:

```text
Upstream debt: current process-local empty success map causes startup Missing→Unsafe and Quote is coupled to position freshness.
Rename impact: existing Full/Degraded/Unsafe enums are preserved; capability diagnostics are additive.
Production evidence: independent Quote probe and monitor rendering are pending serial integration; library markers alone are not completion.
```

Status remains **In Progress** until the independent Quote probe, startup ordering and governed messages are wired and verified.
