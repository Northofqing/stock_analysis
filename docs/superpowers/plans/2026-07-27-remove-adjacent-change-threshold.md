# Remove Adjacent Daily-Change Threshold Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove percentage-based rejection of adjacent historical daily values while retaining structural, continuity, adjustment, source-evidence, and freshness validation.

**Architecture:** Remove the threshold from the three existing admission seams rather than adding a bypass flag. The shared `KlineData` validator loses its threshold parameter and board-prefix helper; the typed selection and review validators lose only their adjacent-close percentage branch. Existing reference-previous-close and provider-change consistency checks continue to detect unverified adjustment discontinuities.

**Tech Stack:** Rust 2021, Tokio, chrono, Diesel/SQLite, Magic Market Data typed batches, Cargo test/clippy/fmt, shell compliance gates.

---

### Task 1: Register the Business-Rule Change

**Files:**
- Modify: `docs/business_rules.md`
- Modify: `tests/v11_three_sources.rs`

- [ ] **Step 1: Rewrite BR-092 without a percentage threshold**

Replace the threshold portion of BR-092 with this contract:

```text
日 K 数据进入计算或数据库 provider 批次写入前必须经过同一个
validate_daily_kline_quality 边界；价格、OHLC、量额、来源涨跌幅、
日期连续性和重复日期继续严格校验。相邻开盘/收盘涨跌百分比不再作为
批次拒绝或人工确认条件。来源 pct_chg 与相邻收盘不一致仍须拒绝，
复权/除权连续性仍须由真实来源字段和证券生命周期证据验证。
```

Remove `max_gap_for` from BR-092’s implementation list and remove its historical
K-line role from BR-131. Keep BR-131’s exchange price-limit inference for realtime
limit-up/limit-down logic.

- [ ] **Step 2: Align cross-domain rules**

In `docs/business_rules.md`:

```text
BR-147: remove “相邻收盘变动 ±20%”
BR-156: remove “相邻变化 ±20%”
BR-159: replace “相邻涨跌” with “来源涨跌幅一致性”
```

Do not change BR-156’s 5-day-return selection feature range; that is a strategy
feature, not historical data-quality rejection.

- [ ] **Step 3: Update the legacy integration-test description**

In `tests/v11_three_sources.rs`, replace comments that promise
`max_gap_for(code)` routing with comments that require positive finite prices,
OHLC/amount completeness, date continuity, and provider change consistency.

- [ ] **Step 4: Verify rule consistency**

Run:

```bash
rg -n "max_gap_for|相邻.*(?:±20|20%)|adjacent.*20%" docs/business_rules.md tests/v11_three_sources.rs
```

Expected: no historical-admission threshold reference.

- [ ] **Step 5: Commit the rule change**

Before staging, run `git diff --cached --name-only` and preserve any pre-staged
user files. Stage only the two files when the index is otherwise clean:

```bash
git add docs/business_rules.md tests/v11_three_sources.rs
git commit -m "docs: remove adjacent daily change rejection rule"
```

### Task 2: Remove the Shared Daily-Kline Threshold API

**Files:**
- Modify: `src/monitor/data_quality.rs`
- Modify: `src/data_gateway/historical_bars.rs`
- Modify: `src/database/kline.rs`
- Modify: `src/database/repository.rs`
- Test: unit tests in the four files above

- [ ] **Step 1: Change the shared-validator tests first**

Replace threshold-rejection tests in `src/monitor/data_quality.rs` with:

```rust
#[test]
fn br092_large_adjacent_move_is_not_a_quality_rejection() {
    let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
    let mut bars = vec![
        make_kline(d1, 10.0, 10.5, 9.8, 10.0),
        make_kline(d2, 13.0, 13.2, 12.8, 13.0),
    ];
    bars[1].pct_chg = 30.0;

    validate_daily_kline_quality(&mut bars, "TEST_CODE_000001")
        .expect("a structurally consistent 30% move is valid");
}
```

Retain the test that rejects a contradictory `pct_chg`, duplicate date, missing
trading day, invalid OHLC, zero amount, and non-finite values. Delete tests for
board-prefix thresholds, IPO/ex-rights percentage exemptions, and
`max_gap_for`.

- [ ] **Step 2: Run the focused test and observe the expected compile failure**

Run:

```bash
cargo test --lib br092_large_adjacent_move_is_not_a_quality_rejection -- --exact
```

Expected: FAIL to compile because the validator still requires `max_gap_pct`.

- [ ] **Step 3: Remove the threshold from the validator**

Change the signature to:

```rust
pub fn validate_daily_kline_quality(
    kline: &mut [KlineData],
    code: &str,
) -> Result<(), String>
```

Delete `max_gap_for`, threshold validation, adjacent open/close percentage
calculation, IPO/ex-rights percentage exemption branches, and the over-20 warning.
Keep this consistency check unchanged:

```rust
let computed_pct = (cur.close - prev.close) / prev.close * 100.0;
if cur.pct_chg.abs() > 1e-9 && (cur.pct_chg - computed_pct).abs() > 0.25 {
    return Err(format!(
        "[{code}] {} 源涨跌幅不一致: source={:.3}% computed={computed_pct:.3}%",
        cur.date, cur.pct_chg
    ));
}
```

- [ ] **Step 4: Migrate every shared-validator caller**

Use exactly:

```rust
validate_daily_kline_quality(&mut output, storage_code)
validate_daily_kline_quality(&mut checked, code)
validate_daily_kline_quality(&mut data, code)
```

Update imports in `src/data_gateway/historical_bars.rs` to remove
`max_gap_for`.

- [ ] **Step 5: Change database persistence tests**

In `src/database/kline.rs`, change both board-threshold rejection tests into
successful persistence assertions:

```rust
let saved = db
    .save_kline_data(&code, &bars, "TEST_PROVIDER")
    .expect("large structurally valid move must persist");
assert_eq!(saved, 2);
```

Keep unique `TEST_CODE_` symbols and cleanup guards.

- [ ] **Step 6: Run focused shared-path tests**

Run:

```bash
cargo test --lib monitor::data_quality::tests
cargo test --lib database::kline::tests
cargo test --lib database::repository::tests
cargo test --lib data_gateway::historical_bars::tests
```

Expected: all selected tests PASS.

- [ ] **Step 7: Commit the shared-path implementation**

```bash
git add src/monitor/data_quality.rs src/data_gateway/historical_bars.rs \
  src/database/kline.rs src/database/repository.rs
git commit -m "fix: accept large adjacent daily moves"
```

### Task 3: Remove the Selection Daily-Bar Threshold

**Files:**
- Modify: `src/selection/quality.rs`

- [ ] **Step 1: Write the new selection acceptance test**

Replace `large_change_explicitly_requires_manual_confirmation` with:

```rust
#[test]
fn large_change_is_accepted_when_structurally_consistent() {
    let mut jump = consecutive_bars(2);
    let price = jump[0].close * 1.30;
    jump[1].open = price;
    jump[1].high = price;
    jump[1].low = price;
    jump[1].close = price;

    validate_daily(&jump).expect("large adjacent change is not a quality failure");
}
```

Keep or add this reference-close failure:

```rust
#[test]
fn reference_previous_close_mismatch_still_fails() {
    let mut bars = consecutive_bars(2);
    bars[1].reference_previous_close = Some(bars[0].close * 0.90);
    assert_eq!(
        validate_daily(&bars).unwrap_err().code(),
        "split_continuity_unverified"
    );
}
```

- [ ] **Step 2: Run the focused test and observe failure**

Run:

```bash
cargo test --lib selection::quality::tests::large_change_is_accepted_when_structurally_consistent -- --exact
```

Expected: FAIL with `adjacent_change_gt_20pct`.

- [ ] **Step 3: Remove the selection threshold**

Delete:

```rust
const MAX_ADJACENT_CHANGE: f64 = 0.20;
```

Delete the adjacent-change branch from `validate_daily`. Delete `manual_error` if
no caller remains. Retain `QualityError::manual_confirmation_required` only if
another quality error uses it; otherwise remove the field and accessor and make
`error` construct only `{ code, message }`.

- [ ] **Step 4: Run all selection quality tests**

Run:

```bash
cargo test --lib selection::quality::tests
```

Expected: all tests PASS, including the reference-close mismatch test.

- [ ] **Step 5: Commit the selection change**

```bash
git add src/selection/quality.rs
git commit -m "fix: remove selection adjacent change threshold"
```

### Task 4: Remove the Review Daily-Close Threshold

**Files:**
- Modify: `src/data_gateway/review.rs`

- [ ] **Step 1: Split the mixed review test**

Keep a duplicate-date rejection test and replace the large-jump assertion with:

```rust
#[test]
fn br158_accepts_large_structurally_valid_daily_move() {
    let first = bar(
        "2099-01-02",
        10.0,
        BarInterval::Day,
        Adjustment::Unadjusted,
        ProviderId::Tdx,
    );
    let jumped = bar(
        "2099-01-03",
        13.0,
        BarInterval::Day,
        Adjustment::Unadjusted,
        ProviderId::Tdx,
    );

    let records =
        validate_daily_close_records(&[first, jumped]).expect("large move is valid");
    assert_eq!(records.len(), 2);
}
```

- [ ] **Step 2: Run the focused test and observe failure**

Run:

```bash
cargo test --lib data_gateway::review::tests::br158_accepts_large_structurally_valid_daily_move -- --exact
```

Expected: FAIL with `invalid_evidence` and `exceeds 20%`.

- [ ] **Step 3: Remove only the percentage branch**

In `validate_daily_close_records`, keep strict ordering and duplicates. Replace:

```rust
let mut previous: Option<(NaiveDate, f64)> = None;
```

with:

```rust
let mut previous_date: Option<NaiveDate> = None;
```

Validate `date > previous_date` and remove the close-ratio calculation.

- [ ] **Step 4: Run review tests**

Run:

```bash
cargo test --lib data_gateway::review::tests
```

Expected: all tests PASS.

- [ ] **Step 5: Commit the review change**

```bash
git add src/data_gateway/review.rs
git commit -m "fix: remove review adjacent close threshold"
```

### Task 5: Prove No Historical Threshold Remains

**Files:**
- Modify only if a production historical threshold is found by the scans below.

- [ ] **Step 1: Scan production code**

Run:

```bash
rg -n "max_gap_for|MAX_ADJACENT_CHANGE|adjacent_change_gt_20pct|相邻跳变未确认|adjacent daily close change exceeds 20%" src tests
```

Expected: no matches.

- [ ] **Step 2: Confirm unrelated safety limits remain**

Run:

```bash
rg -n "jump_threshold_pct|max_change_pct|infer_limit_pct|price.*limit" \
  src/monitor src/risk src/trading src/bin/monitor
```

Expected: realtime tick and order/price-limit safety checks still exist.

- [ ] **Step 3: Run formatting and focused architecture checks**

Run:

```bash
cargo fmt --check
git diff --check
cargo test --test unified_data_architecture
```

Expected: all PASS.

- [ ] **Step 4: Run release gates**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tools/compliance/check.sh
```

Expected: all PASS. If a gate fails, fix from its root cause and rerun that gate.

- [ ] **Step 5: Record evidence**

Append the exact command, pass/fail count, and any fixed failure to:

```text
progress.md
task_plan.md
```

Do not mark Gate D complete until the full migration’s coverage, live monitor, PR,
and merge requirements also pass.
