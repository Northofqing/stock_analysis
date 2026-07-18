# Gate D Coverage Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use TDD to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise registered core line coverage to at least 95% and repository-wide line coverage to at least 80%, add truthful nullable same-day account evidence, pass Gate A-D, and merge PR #2 into `master`.

**Architecture:** Add vertical behavior tests through existing deep module interfaces, extracting only internal dependency seams where orchestration code is otherwise unreachable. Work core-first, recompute llvm-cov after each batch, then close global gaps; keep live account data local and persist missing daily P&L as nullable with source provenance.

**Tech Stack:** Rust, Tokio, Serde JSON, Chrono, SQLite/rusqlite/r2d2, `cargo llvm-cov`, shell compliance gates, GitHub PR workflow.

---

## File structure

- Modify core behavior/tests beside implementation under `src/{pipeline,data_provider,database,trading,decision,risk,event}/`.
- Add integration/process coverage under `tests/` only when behavior crosses a public process or crate interface.
- Modify owning `src/database/` migration/model/repository files for nullable same-day account evidence; private values never enter source or tests.
- Update `docs/business_rules.md`, the Gate D design, handoff, and `.planning/2026-07-16-event-replay-safety-remediation/` evidence after every batch.
- Regenerate `target/coverage/coverage.json`; it remains local build evidence unless repository policy explicitly tracks it.

### Task 1: Cover score breakdown and veto behavior

**Files:**
- Modify: `src/pipeline/score_breakdown.rs`
- Modify: `src/pipeline/veto_rules.rs`

- [x] **Step 1: Add one score tracer bullet**

Add a `#[cfg(test)]` module with a complete deterministic `KlineData` builder and assert missing optional evidence remains neutral while sentiment is clamped:

```rust
let score = compute(
    &ScoreInputs {
        sentiment_score: 120,
        money_flow: None,
        money_flow_section: None,
        volume_ratio_5d: None,
    },
    &kline(10.0),
);
assert_eq!(score.technical, 100);
assert_eq!(score.fundamental_quality, 50);
assert_eq!(score.valuation_safety, 50);
assert_eq!(score.capital_flow, 50);
assert_eq!(score.growth_sustainability, 50);
```

- [x] **Step 2: Run the tracer bullet**

Run: `cargo test --lib pipeline::score_breakdown::tests::missing_evidence_is_neutral_and_sentiment_is_clamped -- --exact`

Expected: PASS. This is a characterization slice and should require no production behavior change.

- [x] **Step 3: Add score behaviors vertically**

Cover independently worked outcomes for factor actions/clamps, valuation percentile and target-price bands, raw money-flow bands, one-day bounce cap, legacy section parsing, volume-ratio adjustment, financial quality, growth bands/ROE trend, Markdown traffic-light rendering, and default equal-weight ranking. Run the named test after each addition.

- [x] **Step 4: Add veto behaviors vertically**

Assert exact outcomes for no signal, three negative revenue periods, low CFO/NI divergence, target-price overvaluation, P99/P99 and P95/P95 and P80/P90 tiers, one-day bounce, strictest cap precedence, downgrade precedence, and rendered advice/cap/flag text. Use a five-day flow whose sum is `-40e8` and latest inflow is `2e8` for the documented bounce condition.

- [x] **Step 5: Validate and commit the pure-core batch**

```bash
cargo fmt --all -- --check
cargo test --lib pipeline::score_breakdown::tests
cargo test --lib pipeline::veto_rules::tests
cargo clippy --lib --all-features -- -D warnings
```

Expected: all PASS. Commit only the two modules plus design/plan/progress evidence.

### Task 2: Cover remaining pure pipeline modules

**Files:**
- Modify: `src/pipeline/{market_regime,price_stats,trade_type,multi_timeframe,data,result_types,extra_context,summary_notify,technical_report,reporting}.rs`
- Modify: `src/pipeline/section_utils.rs`

- [ ] Add one characterization test per existing interface, then boundary cases for every match arm and optional field. Expected literals include exact regime labels, price statistics, trade-type labels, timeframe agreement, missing-section omission, and rendered headings.
- [ ] If a module only exposes an `AnalysisPipeline` method, instantiate the existing pipeline with test-safe dependencies or place tests beside private methods; do not make helpers public solely for coverage.
- [ ] Run each named module test immediately, then `cargo test --lib pipeline::` and strict library Clippy.
- [ ] Regenerate coverage and record per-file percentages. Continue until the pure batch is at least 95% per file or any remaining platform-only defensive branch is explicitly documented.

### Task 3: Cover strict data-provider contracts

**Files:**
- Modify: `src/data_provider/{money_flow,financials,valuation_history,consensus,industry,chip_distribution,gtimg_provider,eastmoney_provider,baostock_provider,announcement,rustdx_provider,intraday_kline,service,mod}.rs`
- Test: existing provider tests under `tests/`

- [ ] For each parser, add a valid `TEST_CODE_` protocol fixture with independent expected domain values.
- [ ] Add one malformed-present-field, missing-required-field, non-finite/non-positive-price, and relevant stale/time-continuity case. Assert explicit `Err`, never empty or zero-filled success.
- [ ] Cover request code/market normalization without network access.
- [ ] Where transport creation blocks parser tests, extract a private `parse_*(&str, observed_at) -> Result<OwnedDomainValue>` seam; public async fetchers remain responsible for status/body/freshness checks.
- [ ] Run focused provider tests after each slice, then all data-provider tests and strict Clippy before committing.

### Task 4: Cover pipeline orchestration

**Files:**
- Modify: `src/pipeline/{position_tracker,backtest_runner,analyze,mod}.rs`
- Modify: `src/pipeline/chain_analysis/{mod,fetchers}.rs`
- Test: `tests/position_tracker_tests.rs`

- [ ] Expand position tests for open/closed/new states, quantity/cost/return calculations, test/live rejection, missing quote, bad price, and persistence failure with isolated `TEST_CODE_` records.
- [ ] Cover backtest no-data, insufficient history, valid signal, sell/close, costs, as-of factor snapshots, and persistence errors through the existing `AnalysisPipeline` interface.
- [ ] Extract internal adapters for chain transport/clock only when behavior genuinely varies; validate strict parsing, registered dedup/sort/limit semantics, missing source, malformed source, and incomplete quote batches.
- [ ] Drive `analyze` through the pipeline interface with validated K-lines and explicit unavailable optional sources. Assert result fields, veto/ranking application, and failure propagation; never register a production mock source.
- [ ] Run `cargo test --lib pipeline::`, affected integration tests, and strict Clippy after every vertical slice.

### Task 3A: Execute public provider protocol tracer bullets

**Files:**
- Modify: `src/data_provider/baostock_provider.rs`
- Modify: `src/data_provider/sina_provider.rs`
- Modify: `src/data_provider/sina_news_provider.rs`

- [x] Add BaoStock tests beside the public functions. Build non-compressed and zlib frames with the documented 21-byte header, then assert version/type/body/CRC fields. Exercise `read_tcp_response` through `tokio::io::duplex` for chunked marker completion, EOF, timeout and the 1 MiB limit.
- [x] Add a valid two-day CSV/CDATA batch whose independent expected values include `close=10.1`, `amount=101000.0`, `pct_chg=1.0`, `AdjustType::Qfq`; then reject missing headers/fields, bad date/number/OHLC, duplicate/gapped dates and an adjacent close jump above 20%.
- [x] Add Sina hq tests using exactly 32 locally constructed fields and assert the UTC conversion of `2026-07-16 15:00:00` from Shanghai time plus price/volume/amount values. Reject missing quotes, short rows, bad/negative/zero prices and invalid source time. Characterize empty K-line arrays as empty and every present row as unavailable because the source protocol lacks real `amount`.
- [x] Add Sina news tests with fixed epoch `1700000000`, covering financial/stock source labels, media/oid/docid/default source names, optional intro, stable content hash, UTF-8 and GBK decode. Reject invalid JSON/data/category/code, missing URL/title/integer time, pre-2000 and more-than-five-minute future times.
- [x] Run `cargo test --lib data_provider::baostock_provider::inline_tests`, `cargo test --lib data_provider::sina_provider`, and `cargo test --lib data_provider::sina_news_provider`; each must pass without network access. Then regenerate focused coverage for the three files and run strict library Clippy.
- [x] Commit the tests and exact coverage evidence independently before extracting any Tencent/Eastmoney private parser seam.

### Task 5: Close remaining registered core deficits

**Files:**
- Modify tests beside uncovered code under `src/{database,trading,decision,risk,event}/`

- [ ] Regenerate coverage and sort registered core files by missed lines using `tools/coverage/check_thresholds.py` prefixes.
- [ ] From the current baseline, prioritize `trading/mod.rs`, `database/concepts.rs`, `decision/intraday_monitor.rs`, `database/mod.rs`, and `database/kline.rs`, then continue largest-first.
- [ ] Database tests use isolated files or unique `TEST_CODE_` rows and validate audit/source/time fields through repository interfaces.
- [ ] Trading/risk tests cover cash, lot size, daily limits, idempotency, secondary confirmation, environment isolation, and fail-closed missing quote/account paths.
- [ ] Event tests preserve immutable audit/hash-chain behavior and explicit sink outcomes.
- [ ] Repeat until `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json --global-min 0 --core-min 95` exits 0.

### Task 6: Raise repository-wide coverage to 80%

**Files:**
- Modify tests beside the largest remaining non-core gaps, initially `src/bin/monitor/{push_templates,main,notify}.rs`, `src/search_service/service.rs`, `src/opportunity/mod.rs`, `src/strategy/rsi/standard.rs`, `src/analyzer/analyze.rs`, `src/agent/multi_agent/slices.rs`, and `src/notification/report.rs`.

- [ ] Cover pure renderers and scoring/state transitions first with exact output/result assertions.
- [ ] Move monitor orchestration bodies behind small internal functions accepting parsed configuration, time, repository, and notification dependencies; `main` remains environment/argument parsing plus exit-code mapping.
- [ ] Add process tests for exit-code/failure boundaries with webhook/broker credentials removed.
- [ ] Recompute global coverage after every commit and continue by largest safe missed-line cluster until `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json --core-min 95 --global-min 80` exits 0.

### Task 7: Persist truthful nullable same-day account evidence

**Files:**
- Modify: owning migration/model/repository files under `src/database/`
- Modify: `docs/business_rules.md` (BR-103)
- Test: owning database module tests

- [x] Add a RED repository test proving a same-day account snapshot stores `daily_pnl=None` with total assets, market value, cash, source, source timestamp, observed-at timestamp, and nullable account mode.
- [x] Add RED validation tests rejecting non-finite/impossible negative values, inconsistent totals beyond documented tolerance, stale source timestamps, and missing provenance.
- [x] Implement the smallest idempotent migration and repository interface; keep legacy ledger reads compatible and never coerce `None` to zero.
- [x] Back up the ignored local database, migrate locally, persist only user-attested fields, and revalidate integrity/totals. Private values/evidence remain outside Git.
- [x] Update BR-103, design, handoff, and rollback commands; run focused tests, database tests, and compliance.

### Task 8: Full release validation, review, and merge

**Files:**
- Verify all changed source/tests/docs and PR #2 evidence.

- [ ] Run `cargo fmt --all -- --check` and `git diff --check`.
- [ ] Run `cargo check --all-targets --all-features`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test --all-targets --all-features`.
- [ ] Run `bash tools/compliance/check.sh`; on freshness failure run the mandated backfill and recheck.
- [ ] Run `cargo build --release --bin monitor` plus a test-mode dry-run with delivery credentials removed.
- [ ] Generate llvm-cov JSON and require global >=80% plus registered core >=95%.
- [ ] Validate the local real-account/account-audit trail without printing or uploading private values.
- [ ] Run independent Standards, Spec, and Audit reviews against a fixed SHA; resolve every Critical/Important finding and repeat.
- [ ] Complete the PR checklist, mark PR #2 Ready, merge through GitHub into `master`, update local `master`, and verify the merge commit plus clean worktree.
