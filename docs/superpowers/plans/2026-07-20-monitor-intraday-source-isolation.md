# Monitor Intraday Source Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure a rejected limit-up batch cannot prevent the production monitor from attempting and consuming independently validated real position quotes.

**Architecture:** Add one small acquisition seam that always executes both external boundaries and preserves their separate `Result` values. The production loop logs each failure independently and gates each consumer on source availability; it never converts an unavailable source into an empty-data fact.

**Tech Stack:** Rust, Tokio `spawn_blocking`, existing `TopStock`, built-in unit tests, Cargo/llvm-cov, repository compliance scripts.

---

## File map

- Create `src/bin/monitor/intraday_market.rs`: independent two-source acquisition seam and focused unit tests.
- Modify `src/bin/monitor/main.rs`: register the module, call the seam, preserve unavailable state with `Option`, and gate limit/position consumers separately.
- Use `docs/superpowers/specs/2026-07-20-monitor-intraday-source-isolation-design.md` as the Gate A source of truth.

### Task 1: Add the independently testable acquisition seam

**Files:**
- Create: `src/bin/monitor/intraday_market.rs`
- Modify: `src/bin/monitor/main.rs` module declarations
- Test: `src/bin/monitor/intraday_market.rs`

- [ ] **Step 1: Write the failing tracer-bullet test**

Register `mod intraday_market;` next to `mod market_data;`. Create the new file with a test that calls the not-yet-defined public interface:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn test_stock(code: &str) -> stock_analysis::market_data::TopStock {
        stock_analysis::market_data::TopStock {
            code: code.to_string(),
            name: "TEST_CODE position".to_string(),
            change_pct: 1.0,
            price: 10.0,
            volume_ratio: Some(1.5),
            main_net_yi: Some(0.2),
        }
    }

    #[test]
    fn limit_failure_does_not_prevent_position_quote_acquisition() {
        let position_called = Cell::new(false);
        let inputs = acquire_intraday_market_inputs(
            || Err("TEST_CODE limit source rejected".to_string()),
            || {
                position_called.set(true);
                Ok(vec![test_stock("TEST_CODE_000001")])
            },
        );

        assert!(position_called.get());
        assert!(inputs.limit_stocks.is_err());
        assert_eq!(
            inputs.position_quotes.expect("position source succeeds")[0].code,
            "TEST_CODE_000001"
        );
    }
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --bin monitor intraday_market::tests::limit_failure_does_not_prevent_position_quote_acquisition
```

Expected: compile failure because `acquire_intraday_market_inputs` is not defined.

- [ ] **Step 3: Add the minimal public seam**

Add above the test module:

```rust
use stock_analysis::market_data::TopStock;

pub struct IntradayMarketInputs {
    pub limit_stocks: Result<Vec<TopStock>, String>,
    pub position_quotes: Result<Vec<TopStock>, String>,
}

pub fn acquire_intraday_market_inputs<LimitFetch, PositionFetch>(
    limit_fetch: LimitFetch,
    position_fetch: PositionFetch,
) -> IntradayMarketInputs
where
    LimitFetch: FnOnce() -> Result<Vec<TopStock>, String>,
    PositionFetch: FnOnce() -> Result<Vec<TopStock>, String>,
{
    let limit_stocks = limit_fetch();
    let position_quotes = position_fetch();
    IntradayMarketInputs {
        limit_stocks,
        position_quotes,
    }
}
```

- [ ] **Step 4: Verify GREEN**

Run the same focused test. Expected: one test passes.

- [ ] **Step 5: Add the reverse failure behavior**

Add the reverse test:

```rust
#[test]
fn position_failure_does_not_discard_limit_up_data() {
    let inputs = acquire_intraday_market_inputs(
        || Ok(vec![test_stock("TEST_CODE_LIMIT")]),
        || Err("TEST_CODE position source rejected".to_string()),
    );

    assert_eq!(
        inputs.limit_stocks.expect("limit source succeeds")[0].code,
        "TEST_CODE_LIMIT"
    );
    assert!(inputs.position_quotes.is_err());
}
```

- [ ] **Step 6: Run both focused tests and commit**

```bash
cargo test --bin monitor intraday_market
git add src/bin/monitor/intraday_market.rs src/bin/monitor/main.rs
git commit -m "test: define independent intraday source contract"
```

Expected: two tests pass.

### Task 2: Integrate independent results into the live loop

**Files:**
- Modify: `src/bin/monitor/main.rs` around the morning/afternoon source acquisition and consumers
- Test: `src/bin/monitor/intraday_market.rs`

- [ ] **Step 1: Replace the short-circuit acquisition**

Call the seam from the existing blocking task:

```rust
let result = tokio::task::spawn_blocking(|| {
    intraday_market::acquire_intraday_market_inputs(
        || {
            let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None)
                .map_err(|error| format!("初始化市场分析器失败: {error}"))?;
            analyzer
                .get_limit_up_stocks()
                .map_err(|error| format!("涨停池获取失败: {error}"))
        },
        || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            market_data::fetch_position_quotes()
        },
    )
})
.await;
```

- [ ] **Step 2: Convert each result to explicit availability**

Resolve both source results separately and preserve unavailable state. Task failure also keeps
the independently sourced jobs eligible:

```rust
let resolved = intraday_market::resolve_intraday_market_inputs(result);
let limit_stocks = resolved.limit_stocks;
let position_quotes = resolved.position_quotes;

if let Some(error) = resolved.limit_error {
    log::error!("[盘中监控] 涨停池批次拒绝: {error}");
}
if let Some(error) = resolved.position_error {
    log::error!("[盘中监控] 持仓行情批次拒绝: {error}");
}
if let Some(error) = resolved.task_error {
    log::error!("[盘中监控] 行情任务失败: {error}");
}

// resolved.consumer_plan.run_independent_jobs remains true for every source state.
```

- [ ] **Step 3: Gate every consumer by its real source**

Use explicit guards. The holding map is created only after a validated position batch; the optional limit batch may enrich it and provide ranking:

```rust
let mut health_lines = Vec::new();
if let Some(position_quotes) = position_quotes.as_ref() {
    let mut stock_map = std::collections::HashMap::new();
    if let Some(limit_stocks) = limit_stocks.as_ref() {
        for stock in limit_stocks {
            if our_codes.contains(&stock.code) {
                stock_map.insert(stock.code.clone(), stock);
            }
        }
    }
    for quote in position_quotes {
        stock_map.entry(quote.code.clone()).or_insert(quote);
    }

    let mut ranked = limit_stocks.as_ref().map(|stocks| {
        stocks
            .iter()
            .filter(|stock| stock.main_net_yi.is_some())
            .collect::<Vec<_>>()
    });
    if let Some(ranked) = ranked.as_mut() {
        ranked.sort_by(|a, b| {
            b.main_net_yi
                .partial_cmp(&a.main_net_yi)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Move the unchanged holding signal loop into this validated-position guard.
}
```

Wrap the virtual-position contribution and limit-board block in matching `if let Some(...)` guards. Keep the stock screener and dedicated T0 path after these guards so their own strict sources decide availability. Do not use `unwrap_or_default`, `Vec::new()` or cached rows to represent a failed production source.

- [ ] **Step 4: Run focused tests and formatting**

```bash
cargo fmt --all
cargo test --bin monitor intraday_market
cargo build --bin monitor
```

Expected: focused tests and monitor build pass.

- [ ] **Step 5: Audit changed paths for silent fallbacks**

```bash
git diff -- src/bin/monitor/main.rs src/bin/monitor/intraday_market.rs
rg -n 'unwrap_or_default\(|let _ = .*\.await|Err\(.+\) => \{\}' src/bin/monitor/main.rs src/bin/monitor/intraday_market.rs
git diff --check
```

Expected: no newly introduced silent fallback and no whitespace errors.

- [ ] **Step 5a: Close review findings with a complete routing matrix**

Add a named resolved-availability type and tests for both success, each asymmetric failure,
both source failures, and task-level failure. The consumer plan must keep independently sourced
jobs eligible in every state. Remove the outer guard that currently suppresses those jobs.

Refactor the Sina and Feishu loopback transport tests to inject an explicit client and URL; do
not mutate process-wide proxy variables. Repeat the default-parallel monitor tests to prove the
race is absent.

- [ ] **Step 6: Commit integration**

```bash
git add src/bin/monitor/main.rs src/bin/monitor/intraday_market.rs
git commit -m "fix: isolate intraday market source failures"
```

### Task 3: Prove gates, merge, and resume observation

**Files:**
- Modify only if a gate finds a root-cause defect in the hotfix.
- Preserve `/private/tmp/stock_analysis_monitor.log` outside Git.

- [ ] **Step 1: Gate B and C**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tools/compliance/check.sh
```

Expected: all commands exit 0.

- [ ] **Step 2: Gate D**

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Expected: repository and core-link coverage meet AGENTS thresholds; release build exits 0.

- [ ] **Step 3: Runtime evidence without private values**

During market hours, start the new release monitor with the existing production environment. Count only hard-coded module/failure labels. If the limit pool remains rejected, prove the independent position source is nevertheless attempted; Quote capability changes only after a real successful quote batch.

- [ ] **Step 4: PR and merge**

Push the branch, create a draft PR with every required AGENTS field, attach Gate evidence, obtain independent review, mark ready, merge to `master`, and verify local and remote `master` contain the merge.

- [ ] **Step 5: Restart and continue cumulative monitoring**

Terminate only the old monitor process after the merge and successful release build. Start one new release process writing to the same local private log with an explicit restart delimiter that contains no account data. Continue the remaining useful duration toward the cumulative 48-hour observation target.
