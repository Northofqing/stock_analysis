# Progress: Event Replay Safety Remediation

## 2026-07-16

- Read mandatory rules, v17.3 spec/plan, current CLI/replay code, monitor integration, and relevant skills.
- Reproduced the baseline fact that focused CLI and replay tests pass despite the reviewed defects.
- Chose structured replay summary design and preserved current module boundaries.
- Registered BR-043 and wrote design/implementation plan.
- Next: start CLI vertical TDD slices.
- CLI composition test failed under the early-return implementation, then passed after known monitor flags became skippable.
- Documented replay-rate equals form failed as unrecognized, then passed after adding the value-bearing prefix parser.
- Explicit `limit=0` failed as `InvalidLimit(0)`, then passed after rejecting only negative values.
- Full CLI module result: 15/15 PASS.
- Invalid/non-string replay text test failed by publishing one envelope, then passed after explicit text validation; blank text is also rejected.
- Rate test failed with zero elapsed delay, then passed after sleeping only between force publish attempts.
- Repeated replay ID test reproduced the duplicate ID, then passed with process/time/atomic-sequence IDs.
- No-subscriber and shutdown cases now increment `failed`; `ReplaySummary` separates scan and dispatch outcomes.
- Monitor replay handling no longer uses `unwrap_or(0)` and exits nonzero for replay errors or failed rows/publishes.
- A downstream test showed `HistoryQuery` still truncated explicit `limit=0` at 100; zero now skips truncation and returns all 101 test rows.
- Replay tests initially interfered through a shared temp directory; unique test directory suffixes removed the race.
- Full event test run exposed the same collision in history helpers plus a fixed-noon timestamp that can fall in the future; both test-only hazards were removed.
- `cargo build --bin monitor`: PASS (124 existing warnings).
- `cargo test --lib`: PASS, 1253 passed / 7 ignored.
- Mandatory freshness backfill initially returned 33/33 empty providers in the network sandbox; the approved non-sandbox retry succeeded 33/33 and advanced `stock_daily` to 2026-07-16 (3008 rows).
- `bash tools/compliance/check.sh`: ALL CHECKS PASSED after backfill.
- Coverage: `event/cli.rs` 91.50%, `event/history.rs` 88.61%, `event/replay.rs` 97.83%; repository total 51.11%, so Gate D remains blocked globally.
- Global `cargo fmt --check` reports about 17,510 pre-existing diff lines; strict clippy reports 342 diagnostics; all-target test compilation fails at `src/bin/v14_e2e.rs:285` because an old dispatcher call omits argument 3.
- First two-axis review found force mode still had no real subscriber, unbounded history still printed only 20 rows, zero-result summary was partial, pacing did not prove the first attempt was immediate, and ID generation silently defaulted a failed clock read.
- Added awaited `ReplayPublisher`; monitor force mode now calls the real notification sink and counts success only after sink acceptance. EventBus remains a publisher adapter for isolated bus failure tests.
- History formatting moved to `format_history_lines`, and the 101-row test proves all unbounded results reach terminal formatting.
- Replay IDs now use pid + process-wide atomic sequence without a fallible clock; pacing test observes the first event before the 200ms interval and blocks the second until the interval elapses.
- Post-review verification: replay 11/11, event 74/74, lib 1254/1254, monitor build PASS, compliance PASS.
- Refreshed coverage: CLI 91.50%, history 88.80%, replay 98.00%, repository 51.14%.
- Final Standards review found that a corrupt existing replay-audit tail could silently restart at `GENESIS`; the file sink now validates every existing record hash and parent link and rejects corruption before notification.
- Added corrupt-audit regression coverage; production publisher suite is 5/5 PASS. Final event suite is 74/74, library suite 1254/1254 with 7 ignored, monitor build PASS, and compliance PASS.
- Final Spec and Standards re-reviews both returned PASS; only scoped Git tracking/commit remains.
- Scoped remediation code and evidence were committed without staging the user's unrelated dirty files; release readiness remains blocked only by recorded repository-wide gates.

## Test Log

| Command | Result |
|---|---|
| `cargo test --lib event::cli::tests -- --nocapture` | PASS baseline; missing reviewed combinations |
| `cargo test --lib event::replay::tests -- --nocapture` | 4/4 PASS baseline; missing failure/rate/repeat-run cases |
| `cargo test --lib event::replay::tests -- --nocapture` | 9/9 PASS after replay summary/rate/ID fixes (before blank-text test addition) |
| `cargo test --lib event::history::tests::zero_limit_returns_and_formats_all_matching_history_entries -- --exact --nocapture` | RED 100/101, then PASS 101/101 |
| `cargo test --lib event:: -- --nocapture` | PASS 73/73 after test-isolation repair |
| `cargo build --bin monitor` | PASS |
| `cargo test --lib` | PASS 1253/1253; 7 ignored |
| `bash tools/compliance/check.sh` | PASS after §2.4 backfill |
| `cargo llvm-cov --lib --summary-only` | PASS; total lines 51.11%, replay lines 97.83% |
