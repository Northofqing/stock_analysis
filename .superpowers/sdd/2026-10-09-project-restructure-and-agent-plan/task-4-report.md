# Task 4 — offline SELL preview and StreakLeader research

Status: implementation and scoped local validation complete; ready for independent review. Production qualification, installed reminders and executable real-history acceptance are **not delivered**. No production operations, source acquisition, broker/paper execution, account seed, notification, activation or E5 facade changes were performed.

Base: `2c2a100787b6d0d76b8d14e3e9aecc1b6ff901dd`, branch `codex/goal-first-delivery-20261009`. Implementation commit: `1853ac3b2` (`feat: add conservative offline SELL and StreakLeader previews`). A separate documentation/evidence commit retains this report and the exact Task 4 raw logs.

## User-facing entry points

Both commands require explicit immutable input and Shanghai RFC3339 `--as-of`. Default output is Chinese Markdown on stdout. JSON, CSV and saved Markdown are optional explicit destinations, all rendered from the same canonical in-memory result. Existing destination files are never overwritten. Preflight rejects duplicate/resolved-alias output paths; `create_new` remains the final race-safe guard. Unix files are 0600. A filesystem error during a multi-file export can leave earlier newly created artifacts; it never overwrites an old artifact.

```sh
cargo run --bin sell_reminder_preview -- \
  --database /absolute/detached-normalized-snapshot.db \
  --as-of 2026-10-09T15:05:00+08:00

cargo run --bin sell_reminder_preview -- \
  --evidence /absolute/private-observed-pack.json \
  --as-of 2026-10-09T15:05:00+08:00 \
  --json /absolute/new-preview.json --markdown /absolute/new-preview.md \
  --handling-template /absolute/new-human-record.json

cargo run --bin sell_reminder_preview -- \
  --reinspect /absolute/previous-preview.json \
  --as-of 2026-10-09T15:30:00+08:00

cargo run --bin streak_leader_research -- \
  --evidence /absolute/private-streak-observations.json \
  --as-of 2026-10-09T15:05:00+08:00 \
  --max-picks 3 --shares 100 --max-entry-slippage-bps 100 \
  --json /absolute/new-study.json --csv /absolute/new-study.csv \
  --markdown /absolute/new-study.md

cargo run --bin streak_leader_research -- \
  --store /absolute/isolated-existing-observed-store \
  --artifact-ref /absolute/observed-artifact-pin.json \
  --as-of 2026-10-09T15:05:00+08:00
```

These are usage examples, not claims that any listed private file exists or that a production capture was performed. `--evidence` and `--store` can be combined for one study; neither source grants authority. SELL requires exactly one of database/evidence/reinspect. Missing sources/schema errors are not silently replaced with fabricated test data. Malformed CLI/input or output failure returns exit 2; a successfully rendered unavailable/expired result returns exit 0.

## Files and boundaries

- `src/offline_products/mod.rs`: shared timestamp/source metadata and escaping; no provider/DB/notification initialization.
- `src/offline_products/io.rs`: bounded 32 MiB JSON reads, snapshot hash/read-only preconditions and private exclusive local outputs; CSV formula injection protection.
- `src/offline_products/sell_reminder.rs`: pure observation/qualified-engine boundary, DB aggregate diagnostic adapter, expiry and human-record template.
- `src/offline_products/streak_leader_research.rs`: frozen strategy, deterministic selection and conservative historical model; canonical JSON/CSV/Markdown.
- New bins: `src/bin/sell_reminder_preview.rs`, `src/bin/streak_leader_research.rs`.
- Minimal `data_gateway::read_offline_observed_artifact` facade inside `historical_observed_store.rs`, re-exported by `data_gateway/mod.rs`. It only uses `HistoricalObservedStore::open_existing/read_checked`. Existing byte pin, digest/length, file/directory identity, no-follow and reopen consistency checks remain in force. A directory lock is acquired by the existing reader. No live capture, Gateway admission, database liveness or provider call is reconstructed.

Public JSON input DTOs use `deny_unknown_fields`; a `qualified=true` addition is rejected. `Source` fields describe observed assertions, never a certificate. Source strings/hashes and persisted upstream `admission`/`complete` text cannot activate either qualified engine. `QualifiedSource` and `QualifiedHistory` are module-private, non-Deserialize types with **no production issuer**. Unit tests construct synthetic trusted fixtures inside the modules. No test input can be loaded via either CLI to obtain that authority.

## Exact schemas

### Shared observed Source

`source`, `revision`, `sha256`, `known_at` (RFC3339 timestamp), `conflicted`, `invalidated`. Engine validation requires nonempty source/revision, a 64-character hex hash, nonfuture known time and no conflict/invalidation. These checks establish structural consistency only. External qualification still requires a separately delivered trusted adapter.

### SELL input: `sell-evidence-observed/v1`

Top level: `schema`, `account` (nullable), `lots` array, `securities` array. A minimum unavailable diagnostic pack is:

```json
{"schema":"sell-evidence-observed/v1","account":null,"lots":[],"securities":[]}
```

- `Account`: account_ref, ownership (`self`), environment (`real`), captured_at, observed_at, complete, confirmed_empty, source. Completeness/identity assertions in a public pack remain observed, not admitted.
- `Lot`: account_ref, instrument, optional lot_id/acquired/sellable_from, total (shares), optional sellable/reserved, optional cost_micro_cny and allocated_buy_fee_micro_cny, optional sell_fees, source. `sellable` is pre-reservation quantity; reserved is a subset. Unknown optional fields remain missing. Cost is **per share**, allocated buy fee is **for the entire current lot**.
- `SellFees`: shares (exact proposed order quantity), commission_micro_cny, stamp_micro_cny, transfer_micro_cny, other_micro_cny (each nullable), source. No absent fee defaults to zero. Explicit evidenced zero is accepted only behind the qualified source boundary.
- `Security`: instrument, exact board, quantity contract, date, optional close_micro_cny, close_finalized, close_source, optional listed/suspended_at_close, independent status_source, bars_source, adjustment, bars. Supported board labels are SSE.MainA, SSE.StarA, SZSE.MainA, SZSE.ChiNextA. Unknown/BSE/ETF are unavailable; no prefix/name inference.
- `QuantityContract`: minimum, step, max_per_order (bounded to one million), whole_remaining_odd_lot, source. Exact effective board/quantity authority is still a blocker, so these JSON fields alone never authorize suggestions.
- `Bar`: date, open/high/low/close (CNY/share), volume; descending dates, finite positive prices, valid OHLC, nonnegative finite volume, verified calendar dates, first bar equal exact confirmed session close. Only unadjusted continuity is supported behind the qualified source boundary; no corporate-action history is invented.

### SELL output: `sell-preview/v1-legacy-rule-units`

Top level includes state (Candidate/NoSuggestion/Unavailable/Expired), created_at, inspected_at, expires_at, input_hash, authority, account_state/account_snapshot, missing[], rows[], semantics/manual_notes, execution and next_open_comparison states. Row fields preserve instrument/account/lot/date, observed total and observed close, state/reason, actual eligible sellable, suggested shares, qualified reference close, covered-fee net scenario, allocated buy fee/sell fee components, indicator scope, evidence hashes and Source records. Public unavailable input clears qualified eligible quantity/reference close/net scenario and suggestions; it preserves explicitly observed data and diagnostics.

Expiry is same-session 15:30+08:00. In-memory reinspection is monotonic and refuses clock rollback. Expired rows have zero suggested shares and never reactivate. Imported JSON is always reclassified as ImportedReport/NotAdmitted, including already expired reports; actionable/covered quantities, prices, net and fee values are cleared while explicitly observed values and raw provenance remain. Replay cannot restore authority; genuine in-memory trusted inspection retains its original scenario evidence until it expires.

`--handling-template` writes `sell-human-observation/v1`: preview hash, not_settlement=true, statuses declared/filled/partial/unfilled/handled-without-action, null actual observation time/account/evidence/quantities/prices/commission/stamp/transfer/other fields. This artifact is an editable human observation template, not an order submission, execution receipt, durable settlement or automatic performance attribution.

### Streak input: `streak-observed/v1`

Top level: schema, days[]. Minimum empty observed pack:

```json
{"schema":"streak-observed/v1","days":[]}
```

`Day`: date, optional universe_source, observations[]. `Observation`: instrument, nullable source-carried streak/amount_micro_cny/close_micro_cny, source, nullable next_close_micro_cny/next_close_date/next_close_source. Source-known-by-15:00 and structural values are necessary for descriptive rank; they do not certify PIT. A next-close comparison additionally needs the exact next verified date and a nonfuture source timestamp covering that close. It is always a **gross descriptive price comparison**, never a trade return.

Artifact pin JSON: `capture_sha256`, `file_sha256`, `byte_length`. These identify existing checked bytes only. The facade exports capture/file identities, observed response time/binding outcome, raw record schema/version/content type, raw SHA-256, raw bytes hex and optional parsed raw JSON. It does not turn the stored outcome into a live capability or normalized admitted bars. Raw store observations appear separately in JSON and flat CSV (`record_type=checked_store_raw_observation`); no streak is inferred from OHLC. Markdown reports capture identities/counts. The CSV raw-record column intentionally preserves original record JSON rather than inventing Qlib-ready market classifications or dates. No Qlib dependency was added.

### Streak output: `streak-study/v1`

Canonical Study includes as_of, policy/strategy_hash, input_hash, optional fee_hash/descriptor/source, fill_hash, authority, headline, denominators, unavailable_days, rows, nullable modeled_win_rate/modeled_covered_net_micro_cny, fee/fill scope and checked store observations. CSV contains one summary row even with no observations, plus strategy observation rows and separate raw-store rows; metadata/denominators remain present when metrics are unavailable.

Trade rows preserve decision/knowledge/source identity, observed rank, qualified selection/exclusion reason, entry/exit dates and actual modeled window times, entry/exit states, requested/model quantities, partially closed/censored shares, modeled prices and separate commission/stamp components, modeled covered-fee net, gross observation, actual settled net (always unavailable), evidence hashes. Separate denominators: decision days, qualified days, observed rows, picks, executable entries, fully closed trades, unfilled/unknown entries and censored entries. Only fully closed modeled trades enter headline net/win-rate; partial closed components remain row-level with censored quantities visible.

## Deterministic rule and arithmetic semantics

SELL invokes the existing unchanged `pipeline::position_tracker::evaluate_sell_rules_with_net_return`. Rule priority remains ATR/fallback stop, tiered hard/technical/structural stops, net >=20% with MA5 break, Boll TopSell at net >=5% after two natural days, then >14-natural-day loss. No buy-signal or market-regime veto is present. T+1, source freshness, reservations and quantity bounds are enforced before a suggestion.

Both capture and observation must be nonfuture and at most exactly 30,000 ms old. Calendar/session gate is verified Shanghai trading date and 15:00 <= as_of <15:30. Independently finalized close must match that exact session, be known at/after15:00 and by as_of. Lifecycle/status must cover the close; suspended-at15:00 is excluded. Invalidated, conflicting or future metadata never produces a usable suggestion.

Lots are evaluated independently; durations and costs are preserved. There is no aggregation across lots. Each independent proposed order has its own supplied fee scenario. Suggestions are capped to available sellable minus reserved and the source quantity contract. Odd-lot exception only covers the sole complete unreserved remaining instrument balance; the engine will not split an odd residual or manufacture aggregate fee allocation.

Net input uses existing checked micro-CNY notional/checked integer arithmetic: `(sell notional - buy notional - allocated original buy fee - covered sell fees) / buy notional *100`. Original buy fees allocate proportionally with integer truncation and remainder retained in the lot. Missing fees/cost/quantity never become confident Hold.

Indicator computation calls the existing StockTrendAnalyzer on ascending real OHLC observations and existing narrow ascending BollMacdObservation engine. The extraction reproduces live latest-first ATR14 mean(high-low). **This is CNY/share but existing StopLoss interprets it as a percent.** The preview preserves/discloses this legacy mismatch and production promotion stays pending an explicit resolution. ATR absent/nonpositive uses the existing fixed net -8% fallback; invalid evidence cannot disguise itself as that fallback. MA60<60 bars substitutes MA20 exactly as existing code does. Fewer than35 bars cannot produce confident Hold, although a deterministically triggered stop can still be shown behind a trusted source seam with fallback scope disclosed.

Research freezes policy version `streak-leader/v1-next-session-window`, max_picks1..20, positive whole100 shares<=one million and slippage0..1000bps before processing. Source-carried streak>=2 ranks streak descending, amount descending, instrument ascending. Missing historical PIT universe/pool/status means unavailable day, including an empty observed day.

Qualified synthetic research reuses `paper_book_v2_fill_model::{WindowRecord::validate,model}` and explicit `AShareFeePolicyV2`, plus verified calendar and checked money arithmetic. No account, parent intent, ledger, seed or execution gateway is involved. Entry is next verified session in09:30–09:35 (09:35 exclusive expiry), with limit floor(D-close*(10000+slippage)/10000) and independently qualified adverse-price/contra-liquidity. Exit is the next verified session after entry, same window/expiry, source lower-band limit. One window per side; no carry-forward fake exit. Whole100 partial volume is preserved; missing window/unknown queue/lifecycle/corporate-action scope is unavailable. Suspended/unfilled or missing exit remains censored. No signal-close fill. Historical fee engine supports explicit Shanghai MainA/StarA only; Shenzhen/BSE/other fee scopes are not widened.

Fees use the supplied explicit dated policy descriptor, per-fill commission minimum and date-dependent stamp tax. Transfer and other charges are excluded in the reused model. Covered-fee modeled net is not complete trading cost or actual settlement. A single-entry FIFO proportional allocation retains cost/fee remainder on the censored lot. Real-receipt comparison and real settled net remain unavailable.

## Actual adapter coverage and outstanding external acceptance

- Delivered: explicit detached SQLite aggregate holdings diagnostic reader using AttributionDatabaseSession::ReadOnly, no singleton/init/migrations/reconciliation. It chooses only effective/confirmed timestamps at/before as_of, checks source main hash before/after and rejects nonempty WAL/journal snapshots. Existing user_position_snapshot/item can supply observed total quantities and aggregate cost only. Confirmed complete empty, absent and incomplete/conflicting aggregate snapshots remain distinct.
- Not delivered: real account-to-lot authority, batch acquisition/sellable/reservation/original-fee receipts, independently finalized exact close plus lifecycle/status/quantity authority. Public packs and raw DB rows cannot fill these gaps. Existing aggregate cost is explicitly diagnostic; no paper-to-real projection.
- Delivered: minimal checked HistoricalObservedStore raw offline reader on macOS/Linux, preserving ObservedOnly/NotAdmitted/PIT NotCertified.
- Not delivered: historical PIT universe/limit pool/lifecycle/band/tick/status/corporate-action and contra-liquidity adapters. Public executable history sample count remains zero; numeric headline metrics are unavailable.
- Not delivered: installed reminder kind/route/deadline/counting owner, Windows runtime proof, broker/manual receipt validation, qualified next-open prices or outcome efficacy. No time-saving or return improvement claim is made.

## Local validation

Final frozen-source validation passed: 20 new offline_products library tests, the single new offline-store facade test, and 6 tests across the two explicit binary targets. Existing sell/indicator/fee/fill production implementations are not modified. No unaffected-task/full-suite/check/clippy/release/deployment run is needed.

Raw logs originate in `/tmp/task4-offline-scratch/` (the first compile used `/tmp/task4-lib-tests.log`). Completed originals are preserved under `.superpowers/sdd/2026-10-09-project-restructure-and-agent-plan/task-4-validation/`; no unrelated scratch content is copied. Initial compile found an error converting the existing non-Send boxed DB connection error into anyhow directly; the new reader was corrected to convert its display string. This failure is diagnostic, not hidden by a success claim.

Human rendering starts with a concise Chinese conclusion and expiry/counts, then a short Chinese missing-source summary and useful position table. Exact technical gaps are an escaped appendix. This wording change was made during self-review while scoped compilation was running; final target checks will include the updated source.

First compiled module run: 13 passed, 6 failed. All six failures were traced to synthetic fixture calendar assumptions: the SELL fixture incorrectly assigned 2026-09-25 sellability to a 2026-09-24 lot despite the holiday, and one research assertion expected 2026-10-09 when the verified calendar returns 2026-10-08. Fixtures now use verified sellability/calendar dates. No production calendar was edited. Final results below supersede this diagnostic run.

A root boundary inspection identified that imported expired reports could retain self-asserted authority and qualified scenario fields. The shared unqualified-number mask now applies to every imported report, before/after expiry, always labels it ImportedReport/NotAdmitted and clears covered sell fees as well. A serialize/deserialize positive-fixture regression verifies imported numbers cannot become actionable and trusted in-memory reinspection is unaffected. The DB adapter also rejects malformed/out-of-range aggregate quantities instead of substituting zero.

Retained Task 4 raw evidence (relative to this report directory):

- `task-4-validation/01-lib-initial-compile-error.log`: initial boxed DB-error conversion compile failure.
- `task-4-validation/02-lib-calendar-fixture-failures.log`: 13 pass / 6 fixture calendar failures.
- `task-4-validation/03-lib-calendar-corrected-19-pass.log`: 19 pass before final import-boundary regression.
- `task-4-validation/04-lib-final-20-pass.log`: final frozen-source library run, 20 pass, including forged-positive import before/after expiry.
- `task-4-validation/05-offline-store-1-pass.log`: exact offline store facade test, 1 pass.
- `task-4-validation/06-binary-targets-6-pass.log`: two explicit binary targets, 3 + 3 pass.

## Final validation and delivery decision

| Command | Result |
| --- | --- |
| `cargo test --lib offline_products` | 20 passed, 0 failed; 5,461 unrelated tests filtered out |
| `cargo test --lib task4_offline_store_reader_exports_observed_bytes_without_restoring_admission` | 1 passed, 0 failed; 5,480 unrelated tests filtered out |
| `cargo test --bin sell_reminder_preview --bin streak_leader_research` | 3 + 3 passed, 0 failed |
| `git diff --cached --check` before implementation commit | Passed |

The final validation shell exited 0. No source changed after the final sequence started. Existing repository unused/dead-code/style warnings appear in raw compiler output; they are not suppressed or repaired by this task. Tests exercise synthetic internal positive boundaries and observed private SQLite/store fixtures, not real captured market/account samples. They prove local deterministic behavior, refusal paths, source-byte preservation and exclusive outputs; they do not prove external qualification, real execution, reminder delivery or profitability.

Source side effects are excluded by the narrow call graph: pure evaluation has no provider, broker, paper executor, notification or global DB initialization dependency. Local integration tests pin DB/input bytes before/after and the observed-store directory contents; the existing store reader performs its integrity-only file lock/reopen checks. No audit/ledger/order/stock_position write path is invoked. Independent review remains the next step before any separate integration decision.

The DB reader can report user-confirmed aggregate snapshot metadata, but it does not establish historical recorded-at/PIT completeness, account-to-lot ownership or independent lot authority. The absence of that adapter stays explicit even for an observed confirmed-empty account. Public artifacts preserve these distinctions rather than converting unavailable facts to zero sample returns or confident Hold.

Raw-log preservation note: Cargo ends successful test logs with a blank line. The task-4-validation-only `.gitattributes` marks `*.log -whitespace` so those original bytes are retained instead of edited for a whitespace check. Source and report whitespace checks remain enabled. Final staged diff check includes this narrowly scoped raw-evidence attribute.
