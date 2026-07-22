# User Position Closing Valuation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist user-confirmed complete position snapshots and calculate an immutable, auditable display-only valuation from real validated unadjusted closing prices.

**Architecture:** The Portfolio context owns pure snapshot and valuation types; Database adapters persist immutable batches; a one-shot importer accepts future user updates. A production RustDX adapter returns typed closing-price evidence, while renderers later consume only a persisted `ClosingValuationView`.

**Tech Stack:** Rust, serde, chrono, sha2, Diesel/SQLite raw typed queries, rustdx-complete through the existing `RustdxProvider`, existing strict K-line/calendar validation.

---

## File map

- Create: `src/portfolio/user_position_snapshot.rs` — normalized complete-snapshot input and validation.
- Create: `src/database/user_position_snapshot.rs` — immutable header/items and latest stable query.
- Create: `src/portfolio/closing_valuation.rs` — price evidence, formulas, partial coverage and view.
- Create: `src/database/closing_valuation.rs` — immutable run/items and latest persisted view.
- Create: `src/portfolio/closing_price.rs` — real RustDX unadjusted close adapter.
- Create: `src/bin/import_user_position_snapshot.rs` — operator import command.
- Modify: `src/portfolio/mod.rs`, `src/database/mod.rs` — module export/schema creation only.
- Do not modify in this parallel task: any file under `src/bin/monitor/`, `src/schema.rs`, `stock_position`, or existing `stock_daily` rows.

### Task 1: Complete snapshot input and canonical identity

- [ ] **Step 1: Write the first RED behavior test**

In the new `src/portfolio/user_position_snapshot.rs`, test this public interface:

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UserPositionItemInput {
    pub code: String,
    pub name: String,
    pub quantity: u64,
    pub cost_price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserPositionSnapshotInput {
    pub snapshot_id: String,
    pub effective_at: chrono::DateTime<chrono::FixedOffset>,
    pub confirmed_at: chrono::DateTime<chrono::FixedOffset>,
    pub source: String,
    pub confirm_empty: bool,
    pub evidence_sha256: String,
    pub items: Vec<UserPositionItemInput>,
}

pub fn user_position_snapshot_input_from_json(
    json: &str,
    confirmed_at: chrono::DateTime<chrono::FixedOffset>,
) -> Result<UserPositionSnapshotInput, String>;
```

Use a literal JSON full snapshot containing `TEST_CODE_000001` and `TEST_CODE_600000` in reverse order. Assert returned items are sorted by code, source equals `user_confirmed_full_snapshot`, and the snapshot/evidence hashes are stable lowercase hex.

- [ ] **Step 2: Run RED**

```bash
cargo test --lib portfolio::user_position_snapshot::tests::complete_snapshot_is_canonical_and_stable -- --exact --test-threads=1
```

Expected: compile failure because the module/interface does not exist.

- [ ] **Step 3: Implement the accepted JSON contract**

Accepted file shape:

```json
{
  "schema_version": 1,
  "effective_at": "2026-07-22T15:00:00+08:00",
  "confirm_empty": false,
  "items": [
    {"code": "TEST_CODE_000001", "name": "测试一", "quantity": 150, "cost_price": 10.0}
  ]
}
```

Validation must reject unknown schema versions, blank code/name, duplicate code, zero quantity, non-finite/non-positive cost, an empty item list without `confirm_empty=true`, and non-empty items with `confirm_empty=true`. Quantity 150 is valid because BR-146 explicitly does not apply order-lot rules to holdings.

Canonicalize by code ascending, serialize a private closed canonical struct, and compute:

```text
evidence_sha256 = sha256("stock_analysis.user_position_snapshot.v1\0" + canonical_json)
snapshot_id = "ups_v1_" + evidence_sha256
```

Do not accept caller-supplied hashes or snapshot IDs.

- [ ] **Step 4: Add the rejection matrix and run GREEN**

```bash
cargo test --lib portfolio::user_position_snapshot::tests:: -- --test-threads=1
```

Expected: all snapshot validation tests pass with `TEST_CODE_` identities.

- [ ] **Step 5: Export and commit**

```bash
git add src/portfolio/user_position_snapshot.rs src/portfolio/mod.rs
git commit -m "feat(portfolio): validate complete user snapshots"
```

### Task 2: Atomic immutable snapshot repository

- [ ] **Step 1: Write a RED repository test for latest-wins**

Create `src/database/user_position_snapshot.rs`. Through public save/load functions, insert three valid snapshots whose ordering differs across `effective_at`, `confirmed_at`, and `snapshot_id`; assert the latest query matches stable SQL ordering and returns all items from exactly one batch.

Public interface:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveUserPositionSnapshotReceipt {
    pub snapshot_row_id: i64,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserPositionSnapshot {
    pub snapshot_row_id: i64,
    pub snapshot_id: String,
    pub effective_at: chrono::DateTime<chrono::FixedOffset>,
    pub confirmed_at: chrono::DateTime<chrono::FixedOffset>,
    pub source: String,
    pub confirm_empty: bool,
    pub evidence_sha256: String,
    pub items: Vec<UserPositionItemInput>,
}

pub fn save_user_position_snapshot(
    input: &UserPositionSnapshotInput,
) -> Result<SaveUserPositionSnapshotReceipt, String>;

pub fn latest_user_position_snapshot() -> Result<Option<UserPositionSnapshot>, String>;
```

- [ ] **Step 2: Run RED**

```bash
cargo test --lib database::user_position_snapshot::tests::latest_complete_snapshot_wins_stably -- --exact --test-threads=1
```

Expected: compile failure before repository implementation.

- [ ] **Step 3: Add schema and transactional save**

Use two tables with foreign keys and immutable triggers:

```sql
CREATE TABLE IF NOT EXISTS user_position_snapshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_id TEXT NOT NULL UNIQUE,
    effective_at TEXT NOT NULL,
    confirmed_at TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source = 'user_confirmed_full_snapshot'),
    confirm_empty INTEGER NOT NULL CHECK(confirm_empty IN (0,1)),
    evidence_sha256 TEXT NOT NULL UNIQUE,
    item_count INTEGER NOT NULL CHECK(item_count >= 0),
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS user_position_snapshot_item (
    snapshot_id TEXT NOT NULL REFERENCES user_position_snapshot(snapshot_id),
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK(quantity > 0),
    cost_price REAL NOT NULL CHECK(cost_price > 0),
    PRIMARY KEY(snapshot_id, code)
);
```

Add `BEFORE UPDATE` and `BEFORE DELETE` triggers to both tables. The save function validates before opening a transaction, inserts header and every item in one transaction, reloads via the public repository mapping, and rejects an evidence-hash collision with different facts.

Latest SQL must use:

```sql
ORDER BY effective_at DESC, confirmed_at DESC, snapshot_id DESC LIMIT 1
```

- [ ] **Step 4: Test atomicity, immutability and idempotency**

Add tests for: duplicate evidence returns original receipt, child insert failure rolls back header, stored bad timestamp propagates error, update/delete triggers reject, empty confirmed snapshot round-trips, and a newer snapshot omitting an old code returns only the new items.

```bash
cargo test --lib database::user_position_snapshot::tests:: -- --test-threads=1
```

Expected: PASS; no test reads `stock_position`.

- [ ] **Step 5: Register schema creation and commit**

Call `user_position_snapshot::create_schema(conn)?` from `DatabaseManager::run_migrations` after environment isolation is known.

```bash
git add src/database/user_position_snapshot.rs src/database/mod.rs
git commit -m "feat(database): persist immutable user snapshots"
```

### Task 3: Safe one-shot importer for future user updates

- [ ] **Step 1: Add a process-level RED test target**

Add unit-testable argument handling inside `src/bin/import_user_position_snapshot.rs` and assert a valid file imports without printing code, name, quantity, cost or raw JSON.

Command interface:

```text
import_user_position_snapshot --database <db> --snapshot <json>
```

- [ ] **Step 2: Implement the importer**

Mirror `src/bin/import_real_account_snapshot.rs` safety:

```rust
#[derive(clap::Parser)]
struct Args {
    #[arg(long)]
    database: std::path::PathBuf,
    #[arg(long)]
    snapshot: std::path::PathBuf,
}
```

Require a regular UTF-8 JSON file no larger than 1 MiB, parse with an injected/current fixed-offset confirmation time, initialize only the requested database, save once, and print only:

```text
snapshot_id_hash=<sha256> inserted=<bool> item_count=<n>
```

Never print the raw snapshot ID because it is itself the evidence hash; print a domain-separated receipt hash instead.

- [ ] **Step 3: Run binary tests and commit**

```bash
cargo test --bin import_user_position_snapshot -- --test-threads=1
git add src/bin/import_user_position_snapshot.rs
git commit -m "feat(portfolio): import user snapshot batches"
```

### Task 4: Pure closing valuation with partial coverage

- [ ] **Step 1: Write one RED worked-example test**

Create `src/portfolio/closing_valuation.rs` around this interface:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ClosingPriceEvidence {
    pub code: String,
    pub trade_date: chrono::NaiveDate,
    pub close: f64,
    pub previous_close: f64,
    pub provider: String,
    pub provider_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub acquired_at: chrono::DateTime<chrono::FixedOffset>,
    pub adjustment: crate::data_provider::AdjustType,
    pub settled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosingPriceFailure {
    pub code: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClosingValuationItemStatus {
    Valued {
        close: f64,
        previous_close: f64,
        market_value: f64,
        unrealized_pnl: f64,
        unrealized_return_pct: f64,
        daily_price_pnl: f64,
    },
    Unavailable { reason_code: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosingValuationItem {
    pub code: String,
    pub name: String,
    pub quantity: u64,
    pub cost_price: f64,
    pub status: ClosingValuationItemStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosingValuation {
    pub run_id: String,
    pub snapshot_id: String,
    pub price_trade_date: chrono::NaiveDate,
    pub calculation_version: String,
    pub provider: String,
    pub valued_count: usize,
    pub total_count: usize,
    pub total_market_value: Option<f64>,
    pub total_unrealized_pnl: Option<f64>,
    pub total_daily_price_pnl: Option<f64>,
    pub items: Vec<ClosingValuationItem>,
}

pub fn calculate_closing_valuation(
    snapshot: &UserPositionSnapshot,
    target_date: chrono::NaiveDate,
    prices: &[Result<ClosingPriceEvidence, ClosingPriceFailure>],
) -> Result<ClosingValuation, String>;
```

Worked literal: quantity 200, cost 10, close 12, previous close 11.5. Assert market value 2400, unrealized P&L 400, return 20%, daily price P&L 100.

- [ ] **Step 2: Run RED**

```bash
cargo test --lib portfolio::closing_valuation::tests::worked_example_uses_literal_expected_values -- --exact --test-threads=1
```

Expected: compile failure before implementation.

- [ ] **Step 3: Implement validation and formulas**

Reject duplicate price codes, unexpected codes, wrong trade date, `adjustment != AdjustType::None`, `settled=false`, non-positive/non-finite close or previous close, and a price code that does not match its snapshot item. Store failures as structured reason codes without embedding code/name in aggregate logs.

For each successful item calculate exactly:

```rust
let market_value = price.close * f64::from(quantity_as_u32);
let unrealized_pnl = (price.close - cost_price) * f64::from(quantity_as_u32);
let unrealized_return_pct = (price.close / cost_price - 1.0) * 100.0;
let daily_price_pnl = (price.close - price.previous_close) * f64::from(quantity_as_u32);
```

Use checked conversion for quantities too large for exact supported arithmetic; do not silently cast. Totals are `None` when valued_count is zero, otherwise sums over successful items with explicit coverage.

- [ ] **Step 4: Add partial/zero coverage tests and run GREEN**

Cover 1/2, 0/2, empty confirmed snapshot, invalid adjusted price, wrong date and duplicate evidence.

```bash
cargo test --lib portfolio::closing_valuation::tests:: -- --test-threads=1
```

Expected: PASS with no cost-price or zero fallback.

- [ ] **Step 5: Export and commit**

```bash
git add src/portfolio/closing_valuation.rs src/portfolio/mod.rs
git commit -m "feat(portfolio): calculate closing valuation coverage"
```

### Task 5: Real unadjusted RustDX closing-price adapter

- [ ] **Step 1: Write selection tests from strict K-line fixtures**

Create `src/portfolio/closing_price.rs` with a pure selector plus a production fetcher:

```rust
pub fn select_unadjusted_closing_price(
    code: &str,
    target_date: chrono::NaiveDate,
    acquired_at: chrono::DateTime<chrono::FixedOffset>,
    rows: Vec<crate::data_provider::KlineData>,
    provider: &str,
) -> Result<ClosingPriceEvidence, String>;

pub fn fetch_rustdx_unadjusted_closing_price(
    code: &str,
    target_date: chrono::NaiveDate,
) -> Result<ClosingPriceEvidence, String>;
```

Tests assert exact target/previous trading-day selection, `AdjustType::None`, `settled=true`, and missing provider time remains `None`.

- [ ] **Step 2: Run RED**

```bash
cargo test --lib portfolio::closing_price::tests:: -- --test-threads=1
```

Expected: compile failure before module implementation.

- [ ] **Step 3: Implement the adapter using the existing real provider**

Production function constructs `RustdxProvider`, calls the `DataProvider::get_daily_data(code, 5)` interface, then uses the selector. It must not read `stock_daily`, use the fallback race (which can return Qfq), or overwrite missing provider publication time with acquisition time.

The selector calls/relies on `validate_kline_series_strict`, verifies the expected previous date via `calendar::prev_trading_day(target_date)`, and returns explicit errors for suspended/missing target bars.

- [ ] **Step 4: Run provider unit tests and commit**

```bash
cargo test --lib portfolio::closing_price::tests:: -- --test-threads=1
git add src/portfolio/closing_price.rs src/portfolio/mod.rs
git commit -m "feat(portfolio): load real unadjusted closes"
```

### Task 6: Immutable valuation run repository

- [ ] **Step 1: Write a RED round-trip test**

Create `src/database/closing_valuation.rs` exposing:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveClosingValuationReceipt {
    pub run_id: String,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosingValuationView {
    pub persisted_run_row_id: i64,
    pub valuation: ClosingValuation,
}

pub fn save_closing_valuation(
    valuation: &ClosingValuation,
) -> Result<SaveClosingValuationReceipt, String>;

pub fn latest_persisted_valuation_view(
) -> Result<Option<ClosingValuationView>, String>;
```

Assert a partial run round-trips all item statuses, totals, `snapshot_id`, target price date, provider and calculation version.

- [ ] **Step 2: Implement immutable run/item/attempt schema**

Use an immutable `closing_valuation_run` header and `closing_valuation_item` children. The deterministic `run_id` is the domain-separated SHA-256 of `(snapshot_id, price_trade_date, calculation_version)`. Store successful numeric fields as nullable columns; failed items keep them NULL plus a closed reason code. Add update/delete rejection triggers and unique `(snapshot_id, price_trade_date, calculation_version)`.

Save all rows in one transaction, then reload and compare to the input. `latest_persisted_valuation_view` returns only persisted data and never recalculates.

- [ ] **Step 3: Test idempotency, conflicts, immutability and 0/N**

```bash
cargo test --lib database::closing_valuation::tests:: -- --test-threads=1
```

Expected: PASS; all totals are NULL for 0/N.

- [ ] **Step 4: Register schema and commit**

```bash
git add src/database/closing_valuation.rs src/database/mod.rs
git commit -m "feat(database): persist closing valuation runs"
```

## Focused completion checks

- [ ] **Step 1: Run focused and strict checks**

```bash
cargo fmt --all -- --check
cargo clippy --lib --bin import_user_position_snapshot --all-features -- -D warnings
cargo test --lib portfolio::user_position_snapshot:: -- --test-threads=1
cargo test --lib database::user_position_snapshot:: -- --test-threads=1
cargo test --lib portfolio::closing_ -- --test-threads=1
cargo test --lib database::closing_valuation:: -- --test-threads=1
cargo test --bin import_user_position_snapshot -- --test-threads=1
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 2: Prove forbidden sources are absent**

```bash
rg -n 'stock_position|stock_daily|cost.*fallback|unwrap_or\(0' src/portfolio/user_position_snapshot.rs src/portfolio/closing_valuation.rs src/portfolio/closing_price.rs src/database/user_position_snapshot.rs src/database/closing_valuation.rs
```

Expected: no production valuation read/fallback; schema names in negative-test text must be explained.

- [ ] **Step 3: Hand off the public persisted view to integration**

The handoff reports only snapshot/run hashes, counts and coverage. It must not paste a holding code, name, quantity, cost, market value or P&L.

Status remains **In Progress** until monitor scheduling, sensitive rendering and Gate D production evidence complete.
