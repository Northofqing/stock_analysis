# Repository Safety Closure Handoff

**Updated:** 2026-07-18

## Current status

The recovered repository-safety work is implemented on
`codex/repository-safety-closure-20260718` and published through PR #2 into
`master`.

Gate B and Gate C pass. Gate D remains blocked by mandatory coverage
thresholds, so PR #2 must remain Draft and must not be merged yet.

## Latest completed work

- SQLite WAL bootstrap validates the returned journal mode.
- All ten initial pool connections are configured and verified directly by
  `DatabaseManager::init`; r2d2 cannot hide connection-PRAGMA failures through
  `CustomizeConnection` retries.
- Every runtime `get_conn` call reapplies the idempotent connection PRAGMAs and
  propagates errors.
- Monitor exits 2 for non-WAL mode, database initialization failure, and
  database parent-directory creation failure.
- Process tests remove `ALERT_WEBHOOK_URL`, so test startup failures cannot send
  real alerts.
- Fresh-database regression requires the expected BR-108 missing same-day
  ledger evidence, not only a generic exit code 2.
- `.github/copilot-instructions.md` now exists as the mandatory Gate-0 input.

## Latest validation

- `cargo fmt --all -- --check`: PASS
- `git diff --check`: PASS
- `cargo check --all-targets --all-features`: PASS
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS
- `cargo test --all-targets --all-features`: PASS
  - library: 1,337 passed, 10 ignored
  - monitor: 293 passed
  - process isolation: 5 passed
  - all other targets: zero failures
- `bash tools/compliance/check.sh`: PASS
  - `stock_daily MAX(date)=2026-07-16`, one trading day behind on 2026-07-18
- `cargo build --release --bin monitor`: PASS
- fresh fixed-tree coverage report generated successfully

Coverage threshold result:

- global: 43,129 / 84,231 = **51.20%**, required at least 80%
- core: 11,834 / 21,342 = **55.45%**, required at least 95% across 94 files

## Live-account evidence

The user provided and attested a 2026-07-18 17:38 Asia/Shanghai real-account
snapshot. Its structured values and a pre-import database backup are stored
only under ignored local `data/private_evidence/`; the source image, position
values, and private evidence must not be committed or uploaded.

The seven open `stock_position` rows were reconciled locally by the existing
unique name/code mapping. Database integrity and the screenshot market-value
total were verified. `ledger` was not updated because the source displayed
daily P&L as unavailable and the current schema cannot represent it as NULL;
zero must not be fabricated.

This closes the manual position/cash snapshot evidence only. It does not waive
coverage or the remaining same-day ledger/account integration requirements.

## Merge blocker and continuation

Status: **In Progress / Blocked — Gate D FAIL**.

1. Keep PR #2 Draft.
2. Raise core coverage from 55.45% to at least 95% and global coverage from
   51.20% to at least 80% with real behavior tests; do not shrink denominators.
3. Add a nullable, source-traceable real-account daily P&L/NAV path before
   persisting the same-day ledger; never substitute zero.
4. Rerun all gates and obtain independent auditor sign-off.
5. Only then mark the PR Ready and merge it through GitHub into `master`.

## Key artifacts

- Design: `docs/superpowers/specs/2026-07-17-repository-history-and-gate-remediation-design.md`
- Plan: `.planning/2026-07-16-event-replay-safety-remediation/task_plan.md`
- Findings: `.planning/2026-07-16-event-replay-safety-remediation/findings.md`
- Progress: `.planning/2026-07-16-event-replay-safety-remediation/progress.md`
- Business rules: `docs/business_rules.md`
- Coverage report: `target/coverage/coverage.json`
- PR: <https://github.com/Northofqing/stock_analysis/pull/2>
