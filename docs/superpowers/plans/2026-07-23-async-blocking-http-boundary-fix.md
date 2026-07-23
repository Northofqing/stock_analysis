# Async Blocking HTTP Boundary Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent `reqwest::blocking::Client` from being created or dropped on monitor Tokio async worker threads while preserving every existing market-data error and retry decision.

**Architecture:** Add one monitor-internal generic adapter that executes synchronous `Result<T, String>` market-data operations on Tokio's blocking pool and converts only `JoinError` into a labeled explicit error. Route every audited async dispatcher and scheduler path that directly or indirectly uses blocking market-data clients through that adapter; synchronous review paths remain unchanged.

**Tech Stack:** Rust 2021, Tokio `spawn_blocking`, reqwest blocking client, existing monitor binary unit-test harness, Cargo compliance tooling.

---

## File map

- Create `src/bin/monitor/blocking_market_data.rs`: the only async-to-synchronous market-data boundary and its deterministic regression tests.
- Modify `src/bin/monitor/main.rs`: register the module and route P-05 virtual-observation quote fallback through the boundary.
- Modify `src/bin/monitor/push_templates.rs`: route I-02, I-03, D-01, P-03 and P-02 synchronous snapshot loaders through the boundary; existing safe I-04/I-10 blocking-pool paths remain unchanged.
- Modify this plan only to record completed checkboxes and validation evidence.

No business-rule registration is added because the change does not alter deduplication, mutex, filtering, sorting, limiting, thresholds, source order or data semantics.

### Task 1: Add the tested async-to-blocking adapter

**Files:**
- Create: `src/bin/monitor/blocking_market_data.rs`
- Modify: `src/bin/monitor/main.rs` near the monitor module declarations

- [ ] **Step 1: Register the new module**

Add beside `mod market_data;` in `src/bin/monitor/main.rs`:

```rust
mod blocking_market_data;
mod market_data;
```

- [ ] **Step 2: Write adapter regression tests first**

Create `src/bin/monitor/blocking_market_data.rs` with tests that call the not-yet-defined adapter:

```rust
#[cfg(test)]
mod tests {
    use super::run_blocking_market_data;

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_market_data_owns_reqwest_blocking_client_off_async_worker() {
        run_blocking_market_data("TEST_CODE reqwest lifecycle", || {
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .build()
                .map_err(|error| error.to_string())?;
            drop(client);
            Ok(())
        })
        .await
        .expect("blocking client lifecycle must remain outside async worker");
    }

    #[tokio::test]
    async fn blocking_market_data_preserves_business_error() {
        let error = run_blocking_market_data("TEST_CODE source", || {
            Err::<(), _>("source batch rejected".to_string())
        })
        .await
        .expect_err("business rejection must remain visible");

        assert_eq!(error, "source batch rejected");
    }

    #[tokio::test]
    async fn blocking_market_data_converts_worker_panic_to_labeled_error() {
        let error = run_blocking_market_data("TEST_CODE panic", || -> Result<(), String> {
            panic!("forced blocking worker panic")
        })
        .await
        .expect_err("worker panic must become an explicit error");

        assert!(error.contains("TEST_CODE panic blocking task failed"), "{error}");
        assert!(error.contains("panicked"), "{error}");
    }

    #[test]
    fn audited_async_call_sites_do_not_directly_call_blocking_loaders() {
        let main_source = include_str!("main.rs");
        let push_source = include_str!("push_templates.rs");

        assert!(!main_source.contains(
            "match market_data::fetch_eastmoney_quotes(&virt_codes)"
        ));
        for direct_call in [
            "match load_news_catalyst_snapshot_real(hhmm)",
            "match load_industry_chain_snapshot_real(hhmm)",
            "match load_news_to_idea_snapshot_real(hhmm)",
            "match load_real_candidate_batch()",
            "match load_auction_volume_snapshot_real(hhmm)",
        ] {
            assert!(
                !push_source.contains(direct_call),
                "async dispatcher still uses direct blocking call: {direct_call}"
            );
        }
    }
}
```

- [ ] **Step 3: Run the focused test and verify the missing adapter fails compilation**

Run:

```bash
cargo test --bin monitor blocking_market_data --offline -- --nocapture
```

Expected: compilation fails because `run_blocking_market_data` is not defined.

- [ ] **Step 4: Implement the minimal adapter**

Add above the tests in `src/bin/monitor/blocking_market_data.rs`:

```rust
pub async fn run_blocking_market_data<T, F>(
    label: &'static str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(result) => result,
        Err(error) => Err(format!("{label} blocking task failed: {error}")),
    }
}
```

- [ ] **Step 5: Run the focused tests and formatting check**

Run:

```bash
cargo test --bin monitor blocking_market_data --offline -- --nocapture
cargo fmt --all -- --check
```

Expected: all four `blocking_market_data` tests pass; formatting passes.

- [ ] **Step 6: Commit the adapter**

```bash
git add src/bin/monitor/blocking_market_data.rs src/bin/monitor/main.rs
git diff --cached --check
git commit -m "fix(runtime): isolate blocking market data clients"
```

### Task 2: Migrate async push dispatchers

**Files:**
- Modify: `src/bin/monitor/push_templates.rs:3051`
- Modify: `src/bin/monitor/push_templates.rs:3401`
- Modify: `src/bin/monitor/push_templates.rs:3964`
- Modify: `src/bin/monitor/push_templates.rs:5198`
- Modify: `src/bin/monitor/push_templates.rs:5560`

- [ ] **Step 1: Route I-02 through the adapter**

Replace the direct snapshot call at the start of `dispatch_news_catalyst_daily` with:

```rust
let hhmm_owned = hhmm.to_string();
let mut snapshot = match crate::blocking_market_data::run_blocking_market_data(
    "I-02 news catalyst snapshot",
    move || load_news_catalyst_snapshot_real(&hhmm_owned),
)
.await
{
    Ok(snapshot) => snapshot,
    Err(error) => {
        log::error!("[I-02] 快照批次拒绝: {}", error);
        log_dispatcher_attempt("I-02", false, 0, &error);
        return false;
    }
};
```

- [ ] **Step 2: Route I-03 through the adapter**

At the start of `dispatch_industry_chain_intraday_daily_result`, replace the direct loader with:

```rust
let hhmm_owned = hhmm.to_string();
let mut snapshot = match crate::blocking_market_data::run_blocking_market_data(
    "I-03 industry chain snapshot",
    move || load_industry_chain_snapshot_real(&hhmm_owned),
)
.await
{
    Ok(snapshot) => snapshot,
    Err(error) => {
        log::error!("[I-03][BR-098] 快照批次拒绝: {}", error);
        log_dispatcher_attempt("I-03", false, 0, &error);
        return PeriodicDispatchResult::Failed(error);
    }
};
```

- [ ] **Step 3: Route D-01 and P-03 through the adapter**

At the start of `dispatch_news_to_idea_daily`, use:

```rust
let hhmm_owned = hhmm.to_string();
let mut snapshot = match crate::blocking_market_data::run_blocking_market_data(
    "D-01 news-to-idea snapshot",
    move || load_news_to_idea_snapshot_real(&hhmm_owned),
)
.await
{
    Ok(snapshot) => snapshot,
    Err(error) => {
        log::error!("[D-01] 真实候选批次拒绝: {error}");
        log_dispatcher_attempt("D-01", false, 0, &error);
        return false;
    }
};
```

At the P-03 batch load, use:

```rust
let batch = match crate::blocking_market_data::run_blocking_market_data(
    "P-03 real candidate batch",
    load_real_candidate_batch,
)
.await
{
    Ok(batch) => batch,
    Err(error) => {
        log::error!("[P-03] 真实候选批次拒绝: {error}");
        log_dispatcher_attempt("P-03", false, 0, &error);
        return false;
    }
};
```

The D-01 loader constructs the snapshot from one candidate batch inside the blocking closure, so no duplicate fetch is introduced.

- [ ] **Step 4: Route P-02 through the adapter**

At the start of `dispatch_auction_volume_daily`, use:

```rust
let hhmm_owned = hhmm.to_string();
let snapshot = match crate::blocking_market_data::run_blocking_market_data(
    "P-02 auction volume snapshot",
    move || load_auction_volume_snapshot_real(&hhmm_owned),
)
.await
{
    Ok(snapshot) => snapshot,
    Err(error) => {
        log_dispatcher_attempt("P-02", false, 0, &error);
        log::warn!("[P-02] 竞价量能快照不可用: {}", error);
        return false;
    }
};
```

- [ ] **Step 5: Run focused tests and compile the binary**

Run:

```bash
cargo test --bin monitor blocking_market_data --offline -- --nocapture
cargo check --bin monitor --offline
```

Expected: adapter tests pass and the monitor binary compiles without lifetime or `Send` errors.

- [ ] **Step 6: Commit dispatcher migration**

```bash
git add src/bin/monitor/push_templates.rs
git diff --cached --check
git commit -m "fix(monitor): route async dispatchers through blocking boundary"
```

### Task 3: Migrate the main-loop quote fallback and audit direct calls

**Files:**
- Modify: `src/bin/monitor/main.rs:7308`

- [ ] **Step 1: Route P-05 quote fallback through the adapter**

Replace the direct `fetch_eastmoney_quotes(&virt_codes)` call with:

```rust
let virt_quotes = crate::blocking_market_data::run_blocking_market_data(
    "P-05 virtual observation quotes",
    move || market_data::fetch_eastmoney_quotes(&virt_codes),
)
.await;

match virt_quotes {
    Ok(virt_quotes) => {
        for quote in virt_quotes {
            for virtual_pos in &mut virtual_observation {
                if virtual_pos.0 == quote.code && virtual_pos.2 == 0.0 {
                    virtual_pos.2 = quote.price;
                }
            }
        }
    }
    Err(error) => {
        log::error!("[P-05 开盘] 虚拟观察报价批次拒绝: {}", error);
    }
}
```

No value is synthesized on failure, so positions without verified quotes remain excluded from snapshot records.

- [ ] **Step 2: Audit every monitor market-data call site**

Run:

```bash
rg -n -C 4 "market_data::fetch_|load_(news_catalyst|industry_chain|news_to_idea|real_candidate|auction_volume).*real" src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
```

Expected: synchronous review paths remain inside existing `spawn_blocking` closures; async I-02/I-03/D-01/P-03/P-02/P-05 paths call `run_blocking_market_data`; no audited async path directly creates or drops a blocking client.

- [ ] **Step 3: Run focused tests and binary compile**

```bash
cargo test --bin monitor blocking_market_data --offline -- --nocapture
cargo check --bin monitor --offline
```

Expected: PASS.

- [ ] **Step 4: Commit the main-loop migration**

```bash
git add src/bin/monitor/main.rs
git diff --cached --check
git commit -m "fix(monitor): isolate virtual quote fallback"
```

### Task 4: Gate C and Gate D validation

**Files:**
- Modify: `docs/superpowers/plans/2026-07-23-async-blocking-http-boundary-fix.md` only for validation evidence

- [ ] **Step 1: Run repository formatting and static checks**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: PASS with no warning.

- [ ] **Step 2: Run the complete test suite serially**

```bash
cargo test --workspace --all-targets --all-features -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 3: Run compliance checks**

```bash
bash tools/compliance/check.sh
```

Expected: PASS, including data freshness. If freshness alone fails, run the mandated real-data recovery command `bash tools/one_shot/backfill_daily.sh`, then rerun compliance; do not fabricate rows.

- [ ] **Step 4: Run coverage gates**

```bash
cargo llvm-cov --workspace --all-features --summary-only
```

Expected: repository total coverage is at least 80% and core trading/data paths are at least 95%. If the repository's pre-existing baseline is lower, report Gate D as blocked with the exact measured values rather than claiming completion.

- [ ] **Step 5: Run isolated monitor validation**

Use an isolated audit/data environment so the known production JSONL chain mismatch cannot mask startup:

```bash
V10_DRY_RUN_PUSH=1 RUST_BACKTRACE=1 cargo run --bin monitor -- --test
```

Expected: no `Cannot drop a runtime in a context where blocking is not allowed` panic. Any independent audit-chain or live-source failure remains explicit and is reported separately.

- [ ] **Step 6: Record evidence and commit**

Record exact command outcomes in this plan, then:

```bash
git add -f docs/superpowers/plans/2026-07-23-async-blocking-http-boundary-fix.md
git diff --cached --check
git commit -m "docs(runtime): record blocking boundary validation"
```

## Rollback

Revert the implementation commits in reverse order, leaving audit and market data untouched:

```bash
git revert <main-loop-commit> <dispatcher-commit> <adapter-commit>
cargo check --bin monitor --offline
```

If rollback reintroduces the Tokio panic, stop the monitor instead of accepting a crash-prone production path.
