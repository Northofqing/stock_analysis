# Persistent DataMode Reminder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit an audited reminder every 30 minutes while the same real DataMode remains Unsafe, without weakening ordinary business or trading data gates.

**Architecture:** A pure reminder-state module decides whether current Unsafe is due and commits time only after confirmed delivery. The existing monitor template orchestrator renders a distinct persistent reminder and sends it through the governed DataMode delivery path.

**Tech Stack:** Rust, Tokio, `std::time::Instant`, v14 L4/L5/L6/L7, SQLite analytics, hash-chain delivery audit, cargo llvm-cov.

---

## Required SDD brief fields

**Upstream debt:**

- PR #6 made first Unsafe audible and receipt-backed, but BR-116 intentionally suppresses the same confirmed state forever. The live caller is `evaluate_data_mode_hook -> push_data_mode_change`.
- Current production evidence has one accepted DataMode row and zero business push receipts while source/risk retries continue.

**Rename impact:**

- No identifier is removed or renamed. `push_data_mode_change` gains one boolean input and the internal plan gains a dispatch reason; every caller is enumerated with multiline-aware grep.

**Production evidence:**

- `push_analytics` must gain a post-restart or due `data_mode` row with `pushed=1` and sink `feishu`.
- `data/event_audit/<date>.jsonl` must gain `push.delivery.audit` for `data_mode_v1`.
- Fixed private-log counters must show `[BR-135]` due/confirmed or explicit retry without printing message text, target, receipt identities, account values, or securities.

## File map

- `docs/business_rules.md`: BR-135, registered before code.
- `docs/superpowers/specs/2026-07-20-persistent-data-mode-reminder-design.md`: Gate A evidence.
- `src/monitor/data_mode.rs`: deep pure reminder-state interface and tests.
- `src/bin/monitor/push_templates.rs`: reminder plan, distinct rendering, governed delivery tests.
- `src/bin/monitor/main.rs`: one reminder state, fail-closed locks, commit after confirmation.

### Task 1: Add the pure persistent-Unsafe reminder state

**Files:**
- Modify: `src/monitor/data_mode.rs`

- [ ] **Step 1: Write one failing public-behavior test**

```rust
#[test]
fn br135_persistent_unsafe_reminder_is_due_only_after_confirmed_interval() {
    let start = Instant::now();
    let mut state = PersistentUnsafeReminder::default();

    assert!(state.should_dispatch(DataMode::Unsafe, start).unwrap());
    state.record_confirmed(DataMode::Unsafe, start);

    assert!(!state
        .should_dispatch(DataMode::Unsafe, start + Duration::from_secs(1_799))
        .unwrap());
    assert!(state
        .should_dispatch(DataMode::Unsafe, start + Duration::from_secs(1_800))
        .unwrap());

    state.record_confirmed(DataMode::Full, start + Duration::from_secs(1_801));
    assert!(!state
        .should_dispatch(DataMode::Full, start + Duration::from_secs(3_600))
        .unwrap());
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test --lib monitor::data_mode::tests::br135_persistent_unsafe_reminder_is_due_only_after_confirmed_interval -- --exact
```

Expected: compile failure because `PersistentUnsafeReminder` does not exist.

- [ ] **Step 3: Implement the smallest deep interface**

```rust
pub const PERSISTENT_UNSAFE_REMINDER_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Default)]
pub struct PersistentUnsafeReminder {
    last_confirmed_at: Option<Instant>,
}

impl PersistentUnsafeReminder {
    pub fn should_dispatch(&self, mode: DataMode, now: Instant) -> Result<bool, String> {
        if mode != DataMode::Unsafe {
            return Ok(false);
        }
        let Some(last) = self.last_confirmed_at else {
            return Ok(true);
        };
        let elapsed = now
            .checked_duration_since(last)
            .ok_or_else(|| "BR-135 monotonic reminder clock moved backwards".to_string())?;
        Ok(elapsed >= PERSISTENT_UNSAFE_REMINDER_INTERVAL)
    }

    pub fn record_confirmed(&mut self, mode: DataMode, now: Instant) {
        self.last_confirmed_at = (mode == DataMode::Unsafe).then_some(now);
    }
}
```

- [ ] **Step 4: Run GREEN and commit**

```bash
cargo fmt --all
cargo test --lib monitor::data_mode::tests::br135_persistent_unsafe_reminder_is_due_only_after_confirmed_interval -- --exact
git add src/monitor/data_mode.rs docs/business_rules.md docs/superpowers/specs/2026-07-20-persistent-data-mode-reminder-design.md docs/superpowers/plans/2026-07-20-persistent-data-mode-reminder.md
git commit -m "fix: schedule persistent unsafe data reminders"
```

Expected: focused behavior passes and Gate A/BR evidence is traceable in the same first commit.

### Task 2: Route a due reminder through the real DataMode path

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`

- [ ] **Step 1: Write one failing planning test**

```rust
#[test]
fn br135_same_unsafe_dispatches_only_when_reminder_is_due() {
    use stock_analysis::monitor::data_mode::{DataHealthInput, DataMode as LibDM};

    assert!(matches!(
        data_mode_notification_plan(&DataHealthInput::default(), Some(LibDM::Unsafe), true),
        DataModeNotificationPlan::Dispatch {
            current: LibDM::Unsafe,
            reason: DataModeDispatchReason::PersistentUnsafeReminder,
            ..
        }
    ));
    assert_eq!(
        data_mode_notification_plan(&DataHealthInput::default(), Some(LibDM::Unsafe), false),
        DataModeNotificationPlan::EstablishSilently
    );
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor push_templates::tests::br135_same_unsafe_dispatches_only_when_reminder_is_due -- --exact
```

Expected: compile failure because the plan has no reminder input or dispatch reason.

- [ ] **Step 3: Implement plan and rendering**

Add `DataModeDispatchReason::{Transition, PersistentUnsafeReminder}`. First non-Full and real changes use `Transition`; due same-Unsafe uses `PersistentUnsafeReminder`.

Change the interface exactly to:

```rust
pub async fn push_data_mode_change(
    input: &DataHealthInput,
    prev: Option<LibDM>,
    persistent_reminder_due: bool,
    banner: Option<&BannerCtx>,
) -> Result<ModeDispatchResult, String>
```

Add `render_data_mode_reminder` with heading `📡 数据状态持续异常`, current real mode, real missing capability labels, existing restrictions, and ETA. Both reasons call `dispatch_outcome(PushKind::DataMode, ...)`; no direct sink or fake adapter is added.

- [ ] **Step 4: Add a governed delivery test**

```rust
#[tokio::test]
#[serial_test::serial(cooldown_memo)]
async fn br135_due_unsafe_reminder_uses_governed_delivery() {
    let _e2e_guard = E2E_MUTEX.lock().await;
    let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
    init_test_db();
    reset_daily_budget_for_test();
    crate::v14_adapter::_reset_dedup_for_test();

    let input = DataHealthInput::default();
    let banner = BannerCtx {
        data_mode: DataMode::Unsafe,
        data_missing_note: Some("Quote/Kline/MoneyFlow/News/OrderBook".to_string()),
        ..BannerCtx::test_default()
    };
    *crate::LATEST_BANNER.lock().unwrap_or_else(|e| e.into_inner()) = Some(banner.clone());

    assert_eq!(
        push_data_mode_change(&input, Some(LibDM::Unsafe), true, Some(&banner))
            .await
            .unwrap(),
        ModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
    );
}
```

Update every existing caller with `false`.

- [ ] **Step 5: Run GREEN and commit**

```bash
cargo fmt --all
cargo test --bin monitor br135 -- --nocapture
cargo test --bin monitor data_mode -- --nocapture
git add src/bin/monitor/push_templates.rs
git commit -m "fix: deliver persistent unsafe reminders"
```

Expected: due reminder uses the governed DataMode path; not-due same mode remains silent.

### Task 3: Commit reminder time only after authoritative delivery

**Files:**
- Modify: `src/bin/monitor/main.rs`

- [ ] **Step 1: Write a failing integration-state test**

Extract a helper with this interface:

```rust
fn commit_data_mode_reminder_result(
    state: &mut PersistentUnsafeReminder,
    mode: LibDM,
    now: Instant,
    result: &ModeDispatchResult,
) -> bool
```

Test exact observable outcomes:

```rust
#[test]
fn br135_reminder_confirmation_requires_pushed() {
    let now = Instant::now();
    let mut state = PersistentUnsafeReminder::default();

    assert!(!commit_data_mode_reminder_result(
        &mut state,
        LibDM::Unsafe,
        now,
        &ModeDispatchResult::Delivery(PushOutcome::Denied("TEST_CODE".to_string())),
    ));
    assert!(state.should_dispatch(LibDM::Unsafe, now).unwrap());

    assert!(commit_data_mode_reminder_result(
        &mut state,
        LibDM::Unsafe,
        now,
        &ModeDispatchResult::Delivery(PushOutcome::Pushed),
    ));
    assert!(!state.should_dispatch(LibDM::Unsafe, now).unwrap());
}
```

Add parallel assertions for `Deduped` and `SinkError("TEST_CODE".to_string())`.

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor br135_reminder_confirmation -- --nocapture
```

Expected: compile failure because the helper and process state do not exist.

- [ ] **Step 3: Wire the production hook**

Create one `Lazy<Mutex<PersistentUnsafeReminder>>`. In `evaluate_data_mode_hook`, take one `Instant::now()`, compute `reminder_due` from the evaluated health, and pass it to `push_data_mode_change`.

The helper returns true only for `Delivery(Pushed)` and calls `record_confirmed`. Full/Degraded confirmation clears Unsafe state. `Denied`, `Deduped`, `SinkError`, lock failure, or monotonic-time error advances nothing. Log failures as `[DataMode-hook][BR-135]`. Never refresh Unsafe on `EstablishedSilently`.

- [ ] **Step 4: Run GREEN and commit**

```bash
cargo fmt --all
cargo test --bin monitor br135 -- --nocapture
cargo test --bin monitor data_mode -- --nocapture
git add src/bin/monitor/main.rs
git commit -m "fix: confirm unsafe reminders after delivery"
```

Expected: only confirmed delivery advances time; every failure remains immediately retryable.

### Task 4: Prove Gates, merge, restart, and collect production evidence

**Files:**
- Modify only if a gate identifies a root-cause defect.

- [ ] **Step 1: Run Gate B and C**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

- [ ] **Step 2: Run Gate D and release**

```bash
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

- [ ] **Step 3: Run the independent five-step verifier**

The fresh verifier must independently run:

```bash
cargo test --lib monitor::data_mode::tests::
cargo build --lib
rg -n -A4 'push_data_mode_change\(' src/bin/monitor --glob '*.rs'
cargo build --release --bin monitor
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test
rg -n 'is_active_spec_target_|is_legacy_v17_' src/bin/monitor/notify.rs
```

After restart it must also verify, using counts only, one real `data_mode` push in L7 and immutable `push.delivery.audit`, plus zero panic/fatal/sink/audit failure. It must not trust the implementer report.

- [ ] **Step 4: PR and merge**

Push the branch, create a Draft PR with every AGENTS field, attach Gate evidence, obtain independent zero-blocker sign-off, mark ready, and merge into master.

- [ ] **Step 5: Rebuild and restart exactly one process**

Preserve the current release binary, build merged master, terminate only the old monitor PID, start one private-log release process, and append a new active-runtime segment excluding restart downtime.

- [ ] **Step 6: Production acceptance**

Using aggregate evidence only, verify the process remains alive; BR-135 reminder is confirmed or explicitly retrying; L7 and immutable audit contain the real outcome; panic/fatal/sink/audit errors are zero; and no payload, destination, account, security, credential, or platform identity entered Git or console output.
