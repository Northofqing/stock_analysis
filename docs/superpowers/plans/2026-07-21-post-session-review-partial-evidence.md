# Post-Session Review Partial-Evidence Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the 19:00 post-session review deliver every report backed by complete real evidence while isolating bad symbols/sources and retrying each unfinished task independently.

**Architecture:** Add a small `review_batch` module that owns typed task outcomes and per-task schedule state. Existing renderers and real data providers remain authoritative; A-01/A-10/R-03/R-08 loaders return accepted evidence plus explicit rejections instead of failing unrelated records. R-02/R-05/R-06 become typed disabled capabilities and R-04 becomes a 21:00 expected wait.

**Tech Stack:** Rust, Tokio, chrono, serde, existing SQLite/audit and push-governor infrastructure.

---

## File Map

- Create `src/bin/monitor/review_batch.rs`: task identity, typed outcomes, batch summary, and per-task retry state.
- Modify `src/bin/monitor/main.rs`: register the new module and make BR-139 commit/retry individual tasks.
- Modify `src/bin/monitor/push_templates.rs`: typed dispatcher entry and per-record evidence isolation.
- Modify `src/data_provider/announcement.rs`: strictly parsed alternate real announcement list protocol and per-detail isolation if RED tests prove the primary-only boundary.
- Modify `src/data_provider/industry.rs`: expose the existing real industry-name lookup for R-03 without fetching unrelated peer metrics.
- Modify `docs/business_rules.md`: register BR-140 before production logic.
- Test in the owning Rust modules; do not create production fixtures containing real account or security data.

### Task 1: Register BR-140 and lock typed outcome semantics

**Files:**
- Modify: `docs/business_rules.md`
- Create: `src/bin/monitor/review_batch.rs`
- Modify: `src/bin/monitor/main.rs`

- [x] **Step 1: Register BR-140 before code**

Add a table row defining: per-task outcome classes, per-symbol isolation, task-level terminal state, 21:00 wait, retryable failure backoff, and the rule that only confirmed sink delivery is `Delivered`.

- [x] **Step 2: Add RED tests for the complete state table**

```rust
#[test]
fn br140_batch_classifies_every_outcome_without_calling_wait_disabled_failed_success() {
    let batch = ReviewBatchOutcome::new(vec![
        (ReviewTask::A01, ReviewTaskOutcome::delivered(1)),
        (ReviewTask::R04, ReviewTaskOutcome::expected_wait(at(21, 0), "source not published")),
        (ReviewTask::R05, ReviewTaskOutcome::disabled("signal_outcome", "source absent")),
        (ReviewTask::R08, ReviewTaskOutcome::failed(true, "transport")),
    ]);
    assert_eq!(batch.delivered_count(), 1);
    assert_eq!(batch.waiting_tasks(), vec![ReviewTask::R04]);
    assert_eq!(batch.disabled_tasks(), vec![ReviewTask::R05]);
    assert_eq!(batch.failed_tasks(), vec![ReviewTask::R08]);
}

#[test]
fn br140_zero_delivery_is_not_cli_success() {
    let batch = ReviewBatchOutcome::new(vec![
        (ReviewTask::R05, ReviewTaskOutcome::disabled("signal_outcome", "source absent")),
    ]);
    assert!(!batch.has_confirmed_delivery());
}
```

- [x] **Step 3: Run RED**

Run: `cargo test --bin monitor br140_batch_ -- --test-threads=1`  
Expected: compile failure because `review_batch` types do not exist.

- [x] **Step 4: Implement the minimal types**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReviewTask { R02, R03, R04, R05, R06, R08, A10, A01 }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewTaskOutcome {
    Delivered { count: usize },
    NoData { reason: String },
    ExpectedWait { retry_at: chrono::NaiveTime, reason: String },
    Disabled { capability: String, reason: String },
    Failed { retryable: bool, reason: String },
}

pub struct ReviewBatchOutcome {
    pub tasks: Vec<(ReviewTask, ReviewTaskOutcome)>,
}
```

Implement explicit constructor/query methods used by the tests; do not add a blanket `bool` conversion.

- [x] **Step 5: Run GREEN and commit**

Run: `cargo test --bin monitor br140_batch_ -- --test-threads=1`  
Expected: all `br140_batch_` tests pass.

```bash
git add docs/business_rules.md src/bin/monitor/main.rs src/bin/monitor/review_batch.rs
git commit -m "feat: type post-session review outcomes"
```

### Task 2: Make BR-139 state task-aware

**Files:**
- Modify: `src/bin/monitor/review_batch.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/push_templates.rs`

- [x] **Step 1: Add RED scheduler tests**

```rust
#[test]
fn br140_one_delivery_does_not_complete_waiting_or_retryable_tasks() {
    let mut state = ReviewScheduleState::for_date(day());
    state.apply(&ReviewBatchOutcome::new(vec![
        (ReviewTask::A01, ReviewTaskOutcome::delivered(1)),
        (ReviewTask::R04, ReviewTaskOutcome::expected_wait(at(21, 0), "not ready")),
        (ReviewTask::R08, ReviewTaskOutcome::failed(true, "transport")),
    ]), at_datetime(19, 0));
    assert!(!state.is_due(ReviewTask::A01, at_datetime(19, 1)));
    assert!(!state.is_due(ReviewTask::R04, at_datetime(20, 59)));
    assert!(state.is_due(ReviewTask::R04, at_datetime(21, 0)));
    assert!(state.has_unfinished_tasks());
}

#[test]
fn br140_disabled_task_is_terminal_without_source_retry() {
    let mut state = ReviewScheduleState::for_date(day());
    state.apply(&ReviewBatchOutcome::new(vec![
        (ReviewTask::R05, ReviewTaskOutcome::disabled("signal_outcome", "source absent")),
    ]), at_datetime(19, 0));
    assert!(!state.is_due(ReviewTask::R05, at_datetime(23, 0)));
}
```

- [x] **Step 2: Run RED**

Run: `cargo test --bin monitor br140_ -- --test-threads=1`  
Expected: compile failure because task-aware state is absent.

- [x] **Step 3: Implement task state and due filtering**

Use `BTreeMap<ReviewTask, TaskScheduleState>` keyed by review date. Delivered/NoData/Disabled are terminal; ExpectedWait stores its exact due time; retryable Failed stores the next retry instant using 1 minute after the first failure, 5 minutes after the second, and 15 minutes thereafter; non-retryable Failed is terminal. Date rollover creates a fresh map for all eight tasks.

Change the batch entry to accept a due set and return `ReviewBatchOutcome`:

```rust
pub async fn dispatch_post_session_review(
    date: &str,
    hhmm: &str,
    banner: &BannerCtx,
    due: &BTreeSet<ReviewTask>,
) -> ReviewBatchOutcome;
```

Only spawn a dispatcher future when its task is due. Log aggregate counts by typed status without message bodies or account data.

- [x] **Step 4: Wire scheduler and strict CLI**

The scheduler passes `state.due_tasks(now)` and applies every returned task outcome. `--review` passes all eight tasks and exits zero only when the returned batch has at least one confirmed delivery. Remove the obsolete day-level `completed_date` transition and retain one production owner.

- [x] **Step 5: Run GREEN and commit**

Run: `cargo test --bin monitor br1 -- --test-threads=1`  
Expected: scheduler and outcome tests pass.

```bash
git add src/bin/monitor/main.rs src/bin/monitor/review_batch.rs src/bin/monitor/push_templates.rs
git commit -m "fix: retry post-session reports per task"
```

### Task 3: Isolate A-01 virtual-observation records

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`

- [x] **Step 1: Write RED candidate tests with TEST_CODE fixtures**

Create an injected pure seam whose fetch closure returns already BR-092-validated `(source, Vec<KlineData>)` or an explicit error.

```rust
#[test]
fn br140_a01_bad_first_symbol_does_not_block_later_valid_record() {
    let rows = vec![bad_record("TEST_CODE_000001"), valid_record("TEST_CODE_000002")];
    let result = build_paper_review_candidate(day_str(), &rows, |code, _| {
        if code.ends_with("000001") { Err("quality rejected".into()) }
        else { Ok((valid_kline(), "TEST_REAL_FIXTURE".into())) }
    }).unwrap();
    assert_eq!(result.snapshot.unwrap().code, "TEST_CODE_000002");
    assert_eq!(result.rejections.len(), 1);
}

#[test]
fn br140_a01_all_invalid_records_return_aggregated_failure() {
    let error = build_paper_review_candidate(day_str(), &two_records(), |code, _| {
        Err(format!("{code}: quality rejected"))
    }).unwrap_err();
    assert!(error.contains("2 records rejected"));
}
```

- [x] **Step 2: Run RED**

Run: `cargo test --bin monitor br140_a01_ -- --test-threads=1`  
Expected: compile failure for the missing candidate seam.

- [x] **Step 3: Implement per-record isolation**

Move the current first-record calculation into `build_paper_review_candidate`. Validate every record, continue after explicit errors, and return accepted snapshot plus rejection metadata. Never catch a BR-092 error and reuse the rejected bars. Use the selected snapshot code as the daily push dedup key.

- [x] **Step 4: Run GREEN and commit**

Run: `cargo test --bin monitor br140_a01_ -- --test-threads=1`  
Expected: all A-01 isolation tests pass.

```bash
git add src/bin/monitor/push_templates.rs
git commit -m "fix: isolate paper review data failures"
```

### Task 4: Isolate A-10 and R-03 candidates

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`
- Modify: `src/bin/monitor/main.rs`

- [x] **Step 1: Add RED tests**

Add `br140_a10_one_missing_name_keeps_verified_candidates`, `br140_a10_all_names_missing_fails`, `br140_r03_missing_sector_keeps_later_verified_stock`, `br140_r03_kline_failure_is_per_symbol`, and `br140_r03_partial_scope_is_not_source_complete` using only `TEST_CODE_` fixtures and injected name/sector/K-line resolvers.

- [x] **Step 2: Run RED**

Run: `cargo test --bin monitor br140_ -- --test-threads=1`  
Expected: tests fail because current loaders use `?` inside whole-batch loops.

- [x] **Step 3: Implement accepted/rejected collections**

For A-10, parse every rotation row independently, resolve a missing name only through the injected real-name resolver, and render when at least one selected candidate has complete name evidence. For R-03, return `ReviewLimitChainBatch { accepted, rejected }`; only accepted rows reach `aggregate`, and `source_complete` is `rejected.is_empty()`.

- [x] **Step 4: Run GREEN and commit**

Run: `cargo test --bin monitor br140_ -- --test-threads=1`  
Expected: all candidate-isolation tests pass.

```bash
git add src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
git commit -m "fix: isolate catalyst and chain review candidates"
```

### Task 5: Make R-02/R-04/R-05/R-06 status truthful

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`
- Modify: `src/bin/monitor/review_batch.rs`

- [x] **Step 1: Add RED behavior tests**

Add `br140_r02_disabled_does_not_fetch_market`, `br140_r04_before_2100_is_expected_wait`, `br140_r04_due_at_2100_after_1900_delivery`, `br140_r05_disabled_does_not_access_source`, and `br140_r06_disabled_does_not_access_source`.

- [x] **Step 2: Run RED**

Run: `cargo test --bin monitor br140_ -- --test-threads=1`  
Expected: current bool dispatchers cannot express the asserted status.

- [x] **Step 3: Return typed statuses before I/O**

R-02 returns Disabled for missing BR-093 capabilities without calling `fetch_market_review_snapshot`; R-05 and R-06 return Disabled with stable capability codes; R-04 takes the evaluated local time and returns ExpectedWait until 21:00, then calls the real producer. Remove the stale `monitor_loop` retry text.

- [x] **Step 4: Run GREEN and commit**

Run: `cargo test --bin monitor br140_ -- --test-threads=1`  
Expected: all truthful-status tests pass.

```bash
git add src/bin/monitor/review_batch.rs src/bin/monitor/push_templates.rs
git commit -m "fix: classify unavailable review capabilities"
```

### Task 6: Isolate R-08 announcement transport and components

**Files:**
- Modify: `src/data_provider/announcement.rs`
- Modify: `src/bin/monitor/push_templates.rs`

- [x] **Step 1: Add RED provider and component tests**

Use local HTTP fixtures to add `br140_announcement_primary_failure_uses_verified_alternate_protocol`, `br140_announcement_all_transports_fail_explicitly`, `br140_one_detail_failure_keeps_other_verified_announcements`, `br140_r08_stale_positions_are_rejected`, and `br140_r08_requires_one_complete_component`.

- [x] **Step 2: Run RED**

Run: `cargo test br140_announcement_ -- --test-threads=1` and `cargo test --bin monitor br140_r08_ -- --test-threads=1`  
Expected: current single list URL and fail-fast detail collection fail the tests.

- [x] **Step 3: Add strict real fallback and partial component assembly**

Reuse the already implemented Eastmoney alternate announcement protocol shape, normalize it into `Announcement` with source/date/id provenance, and accept it only after strict response validation. Collect detail results per announcement instead of `collect::<Result<Vec<_>>>()`; rejected details remain audited. R-08 obtains real positions with source time and enforces the 30-second gate. It may render a degraded report only when at least one independent component is complete and explicitly labels unavailable components.

- [x] **Step 4: Run GREEN and commit**

Run: `cargo test br140_announcement_ -- --test-threads=1` and `cargo test --bin monitor br140_r08_ -- --test-threads=1`  
Expected: fallback, isolation, freshness, and minimum-component tests pass.

```bash
git add src/data_provider/announcement.rs src/bin/monitor/push_templates.rs
git commit -m "fix: isolate event calendar data sources"
```

Implementation evidence through Task 6:

- typed outcomes and task-aware scheduling: `e64f8f3`, `913b305`;
- A-01 isolation: `1e1f75f`;
- A-10/R-03 isolation and real industry lookup: `509f27d`, `e03c05b`;
- R-08 component degradation: `8d974db`, `e03c05b`;
- strict primary/alternate announcement protocols and per-detail isolation: `aac7698`;
- focused GREEN: 35 announcement-provider tests, 13 BR-140 monitor tests plus the added R-03/R-08 isolation cases. Gate D remains open until Task 7 completes.

### Task 7: Release gates, review, merge, and restart

**Files:**
- Modify only when a gate exposes a root-cause defect.
- Add final evidence to the PR body and the 48-hour operations plan/report.

- [ ] **Step 1: Run focused and full gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Expected: all commands exit 0; global line coverage is at least 80%, registered core at least 95%.

- [ ] **Step 2: Independent review and PR evidence**

Review code and spec against BR-140 and all triggered data red lines. PR fields must include Refs, Data-Redlines, OldModules, Threshold-Proof, Business-Rules, Validation, and Rollback.

- [ ] **Step 3: Merge and deploy one instance**

Push the feature branch, merge the PR without bypassing required checks, fast-forward local `master`, rebuild release, identify the exact existing supervisor/process, stop only that instance, and restart it through the same supervisor. Never start a second production monitor.

- [ ] **Step 4: Canary and 48-hour continuation**

Run one explicit strict review canary or observe the scheduled batch. Verify only aggregate task-status and sink-confirmation markers; do not print message bodies or account/security data. Restart the cumulative 48-hour timer from the deployed process start, record blockers in the local private evidence directory, and merge the final redacted operations report through a separate PR.
