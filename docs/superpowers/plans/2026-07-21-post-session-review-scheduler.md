# Post-Session Review Scheduler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the long-running production monitor automatically run the strict R-series post-session review after 19:00 on each real trading day.

**Architecture:** Add a small pure due gate and explicit in-process schedule state beside the existing strict review wrapper in `main.rs`. A dedicated Tokio scheduler reuses `evaluate_account_mode_hook`, `run_strict_review_only_inner`, the existing timeout contract, and `dispatch_post_session_review`; it commits the local completed date only after at least one confirmed delivery.

**Tech Stack:** Rust, Tokio interval/timeout, chrono local trading calendar, existing monitor push governance and Rust unit tests.

---

## File Map

- Modify `docs/business_rules.md`: register BR-139 before production scheduling logic.
- Modify `src/bin/monitor/main.rs`: due gate, state transition, strict single-attempt runner, scheduler registration, and unit tests.
- Verify `src/bin/monitor/push_templates.rs`: reuse the existing BR-110 dispatcher without changing report data semantics.

### Task 1: Lock the missing schedule contract with RED tests

**Files:**
- Modify: `src/bin/monitor/main.rs` near `run_strict_review_only_inner` and the bottom test modules.

- [x] **Step 1: Add the failing due-gate tests**

```rust
#[cfg(test)]
mod tests_post_session_review_scheduler {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};

    fn at(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 21)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn br139_review_is_due_only_after_threshold_on_a_trading_day() {
        let state = PostSessionReviewScheduleState::default();
        assert!(!post_session_review_due(at(18, 59), true, &state));
        assert!(post_session_review_due(at(19, 0), true, &state));
        assert!(!post_session_review_due(at(19, 0), false, &state));
    }

    #[test]
    fn br139_completed_or_in_flight_review_is_not_due() {
        let date = at(19, 0).date();
        let mut state = PostSessionReviewScheduleState {
            completed_date: Some(date),
            in_flight: false,
        };
        assert!(!post_session_review_due(at(19, 1), true, &state));
        state.completed_date = None;
        state.in_flight = true;
        assert!(!post_session_review_due(at(19, 1), true, &state));
    }

    #[test]
    fn br139_only_confirmed_attempt_commits_completion() {
        let date = at(19, 0).date();
        let mut state = PostSessionReviewScheduleState {
            completed_date: None,
            in_flight: true,
        };
        let _ = finish_post_session_review_attempt(
            &mut state,
            date,
            Err("TEST_CODE sink failed".into()),
        );
        assert_eq!(state.completed_date, None);
        assert!(!state.in_flight);
        state.in_flight = true;
        finish_post_session_review_attempt(&mut state, date, Ok(())).unwrap();
        assert_eq!(state.completed_date, Some(date));
        assert!(!state.in_flight);
    }
}
```

- [x] **Step 2: Run the focused test and confirm RED**

Run: `cargo test --bin monitor br139_ -- --test-threads=1`

Expected: compile failure because `PostSessionReviewScheduleState`, `post_session_review_due`, and `finish_post_session_review_attempt` do not exist.

### Task 2: Implement the minimal scheduler and production wiring

**Files:**
- Modify: `docs/business_rules.md`
- Modify: `src/bin/monitor/main.rs` near strict review functions and the normal long-running branch.

- [x] **Step 1: Add the state and pure transition seam**

```rust
#[derive(Debug, Default)]
struct PostSessionReviewScheduleState {
    completed_date: Option<chrono::NaiveDate>,
    in_flight: bool,
}

fn post_session_review_due(
    now: chrono::NaiveDateTime,
    is_trading_day: bool,
    state: &PostSessionReviewScheduleState,
) -> bool {
    let threshold = chrono::NaiveTime::from_hms_opt(19, 0, 0).expect("valid threshold");
    is_trading_day
        && now.time() >= threshold
        && state.completed_date != Some(now.date())
        && !state.in_flight
}

fn finish_post_session_review_attempt(
    state: &mut PostSessionReviewScheduleState,
    date: chrono::NaiveDate,
    result: Result<(), String>,
) -> Result<(), String> {
    state.in_flight = false;
    if result.is_ok() {
        state.completed_date = Some(date);
    }
    result
}
```

- [x] **Step 2: Add a non-exiting strict attempt and scheduler**

```rust
fn review_timeout_secs() -> u64 {
    std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&value| value > 0)
        .unwrap_or(300)
}

async fn attempt_post_session_review() -> Result<(), String> {
    if !evaluate_account_mode_hook(true).await {
        return Err("real AccountMode/banner initialization failed".to_string());
    }
    let timeout_secs = review_timeout_secs();
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        run_strict_review_only_inner(),
    )
    .await
    .map_err(|_| format!("strict review timed out after {timeout_secs}s"))?
}

async fn post_session_review_scheduler() {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut state = PostSessionReviewScheduleState::default();
    log::info!("[复盘调度][BR-139] started threshold=19:00 interval=60s");
    loop {
        interval.tick().await;
        let now = chrono::Local::now();
        if !post_session_review_due(
            now.naive_local(),
            stock_analysis::calendar::is_trading_day(now.date_naive()),
            &state,
        ) {
            continue;
        }
        state.in_flight = true;
        let result = attempt_post_session_review().await;
        if let Err(error) = finish_post_session_review_attempt(&mut state, now.date_naive(), result)
        {
            log::error!("[复盘调度][BR-139] attempt failed; retry remains eligible: {error}");
        } else {
            log::info!("[复盘调度][BR-139] confirmed for {}", now.date_naive());
        }
    }
}
```

Replace the duplicate environment parsing inside `run_review_only()` with `let review_timeout_secs = review_timeout_secs();`. The scheduler must call the existing `stock_analysis::calendar::is_trading_day(now.date_naive())`; do not add a weekend-only approximation.

- [x] **Step 3: Register the scheduler only in the long-running production branch**

Add beside `post_close_news_scheduler()`:

```rust
tokio::spawn(post_session_review_scheduler());
```

Do not register it before terminal `--review`, `--test`, `--help`, or dry-run branches.

Delete the stale `monitor_loop` `evening_pushed` state and its direct 19:00 dispatcher block. Add a source-level regression assertion that `main.rs` contains exactly one production `dispatch_post_session_review` call and no stale owner identifier.

- [x] **Step 4: Run focused tests and confirm GREEN**

Run: `cargo test --bin monitor br139_ -- --test-threads=1`

Expected: 3 passed, 0 failed.

- [x] **Step 5: Commit the scheduler slice**

```bash
git add docs/business_rules.md src/bin/monitor/main.rs docs/superpowers/plans/2026-07-21-post-session-review-scheduler.md
git commit -m "fix: schedule strict post-session review"
```

### Task 3: Release gates and production evidence

**Files:**
- Modify only if a gate reveals a root-cause defect in the changed paths.

- [x] **Step 1: Run formatting and lint gates**

Run: `cargo fmt --all -- --check`

Expected: exit 0.

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: exit 0.

- [ ] **Step 2: Run all tests and compliance**

Run: `cargo test --workspace --all-targets --all-features -- --test-threads=1`

Expected: all non-ignored tests pass.

Run: `bash tools/compliance/check.sh`

Expected: all compliance gates pass, including freshness and BR registration.

- [ ] **Step 3: Run coverage and Release build**

Run: `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json`

Run: `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json`

Expected: global coverage at least 80% and registered core coverage at least 95%.

Run: `cargo build --release --bin monitor`

Expected: exit 0.

- [ ] **Step 4: Review, merge, and verify**

Request the existing independent reviewer to inspect BR-138 and BR-139 against the current diff and gate evidence. After approval, update PR evidence, push, merge to `master`, fast-forward the deployment checkout, restart the release monitor through the existing supervisor, and verify only aggregate scheduler/audit markers. Do not print holdings, account identifiers, webhook content, or notification bodies.

Expected evidence: scheduler startup marker; before 19:00 no auto dispatch; after 19:00 or explicit strict canary, either a confirmed delivery audit or an explicit BR-108/BR-110/BR-139 retryable failure.
