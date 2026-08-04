# Progress Log

## Session: 2026-07-22

### Current Status
- **Phase:** 1 - Approved design and written review
- **Started:** 2026-07-22

### Actions Taken

- Diagnosed the exact production push artifact, account/database state, DataMode bootstrap path,
  delivery-audit legacy incompatibility, and async/blocking runtime boundary.
- Consolidated 13 immediate defects and six unimplemented v18 workstreams without conflating their
  levels of granularity.
- Received user approval to suppress ordinary pushes while audit health is unavailable.
- Received user decision to defer real broker integration and use validated daily closes for local
  position valuation.
- Received user decision that every holdings update is a complete snapshot and later confirmed
  snapshots take precedence.
- Created the isolated planning session `2026-07-22-user-position-valuation-push-recovery` without
  deleting or overwriting the existing monitor-observation plan.
- Created the written design artifact and updated task/findings/progress planning files.

### Test Results
| Test | Expected | Actual | Status |
|------|----------|--------|--------|
| Design scope check | No real-account authorization from local valuation | Explicit display/action split | PASS |
| Audit safety check | No immutable JSONL rewrite | Read-only exact legacy compatibility only | PASS |
| Parallel ownership check | No concurrent shared-file edits | Three disjoint streams plus serial integration | PASS |

### Errors
| Error | Resolution |
|-------|------------|
| Initial diagnostic query used a nonexistent position column | Read schema and reran with the correct `quantity/status` columns. |
| Panic stderr was not in the private long-running log | Source-traced the reqwest message and recorded backtrace capture as an implementation acceptance requirement. |

## Session: 2026-07-22 evening snapshot update

### Actions Taken

- Imported the user-provided complete 2026-07-22 position snapshot into the append-only
  `user_position_snapshot` tables; latest-wins now exposes the seven updated quantities and costs.
- Imported the screenshot account summary: total assets 60855.46, securities market value 50156.00,
  available cash 10699.46, position ratio 82.4%, daily P&L +613.40.
- Updated the real-holding T0 path to carry confirmed share quantity and render a distinct
  `【真实持仓】` label with bounded sell/buy share quantities. Paper positions remain in the paper
  ledger and are not mixed into this T0 source.

### Validation

- `cargo check --bin monitor --offline`: PASS
- `cargo test --bin monitor --offline t0 -- --nocapture`: 25 passed
- `run_closing_valuation -- 2026-07-22`: BLOCKED by missing settled close for 000813; no fake value written.
- The local DB currently has no `closing_valuation` table, so the schema/production initialization path
  still needs verification before daily return statistics can be considered complete.

### Follow-up implementation

- Verified the valuation tables are created by the current database initializer; the earlier `no such
  table` probe was run before the initializer had reopened the database.
- Updated monitor closing valuation to use validated `stock_daily` closes when RustDX has no batch;
  this is a real-data fallback, not a fabricated value. Missing both sources still fails closed.
- `cargo check --bin monitor --offline`: PASS; focused monitor tests: 25 passed.
- Real/paper combined summary push remains the next integration slice; T0 remains a separate push.
