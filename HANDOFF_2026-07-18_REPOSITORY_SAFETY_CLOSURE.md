# Repository Safety Closure Handoff

**Updated:** 2026-07-18

**Next-session focus:** complete Gate D, obtain required approval, and merge PR #2 into `master` only after every mandatory gate is green.

## Current status

The user asked to recover the previously deleted task, finish it without repeated confirmation, and merge it into `master`.

The repository safety and compliance remediation has been recovered, implemented, committed, and pushed. Gate B, Gate C, and the independent Standards, Spec, and scoped Audit reviews passed. Gate D is still blocked by coverage, real-account same-day validation, and auditor sign-off. Under `AGENTS.md` Part 4, the task status is therefore **In Progress / Blocked**.

No merge has been performed. PR #2 must remain Draft until Gate D passes.

## Git and PR state

- Branch: `codex/repository-safety-closure-20260718`
- HEAD: `70b7221` (evidence/handoff update)
- Main implementation commit: `e7db307`
- Local task base: `289b7b9`
- Last observed upstream divergence: `0 0`
- PR: <https://github.com/Northofqing/stock_analysis/pull/2>
- Last observed PR state: OPEN, Draft, base=`master`, head=`codex/repository-safety-closure-20260718`
- Last observed GitHub merge metadata: `mergeable=MERGEABLE`, `mergeStateStatus=CLEAN`; no remote checks or review decision were reported

The PR was initially based on `main` and was corrected to `master`. Remote `master` was approximately 73 commits behind the local task base, so the PR against remote `master` includes inherited history in addition to the two task commits. Do not rewrite or drop that history without an explicit history strategy.

Recheck all external GitHub state before acting; the observations above may become stale.

## User-owned unstaged changes

The following changes were present before this handoff document and are outside the remediation task. Preserve them and do not stage, modify, restore, or clean them:

- `.gitignore` — modified
- `.superpowers/sdd/progress.md` — modified
- `src/app/context.rs` — deleted
- `src/broker/ib.rs` — deleted

This handoff document is the only intended new workspace change from the current session.

## Completed scope

- Production paths require real data and strict freshness/provider validation; source failure is explicit rather than a mock or silent fallback.
- Order safety enforces cash/amount/lot/price constraints, persistent 60-second idempotency, secondary confirmation, and account evidence.
- A tamper-evident SHA-256 order audit chain is committed in the same transaction as order, audit, chain, and position/paper-account mutations. Startup validates the full chain, rejects bad or partial chains, and only backfills a wholly empty legacy chain.
- Notification delivery requires strict protocol confirmation and fails closed.
- Periodic timer/hash state advances only on a confirmed delivery, an explicit deduplication result, or a genuinely empty result.
- BR-087, BR-116, and BR-117 were implemented, including real current-ledger evidence requirements.
- `TEST_CODE`, test databases, test logs, and test audit data are physically isolated from live paths.
- The `news::sink` global sender test race was fixed; the focused parallel test set passed ten consecutive runs.

Do not reproduce the full implementation in this document. Inspect commits `e7db307` and `70b7221`, the design, and the planning artifacts listed below.

## Validation evidence

Passed:

- `cargo fmt --all -- --check`
- `cargo check --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
  - library: 1,336 passed, 10 ignored
  - monitor: 293 passed
  - all other binary, integration, and doctest targets: zero failures
- `bash tools/compliance/check.sh`
  - `stock_daily MAX(date)=2026-07-16`, one A-share trading day behind on 2026-07-18
- workflow YAML parsing
- `cargo build --release --bin monitor`
- independent Standards review: PASS
- independent Spec review for BR-087/116/117: PASS
- independent scoped Audit review: PASS

The isolated smoke command was:

```bash
/usr/bin/env DATABASE_PATH=/private/tmp/stock_analysis_release_smoke_20260718_final.db \
  STOCK_LIST=TEST_CODE_000001 \
  STOCK_ENV_MODE=test \
  MONITOR_ENABLED=true \
  V10_DRY_RUN_PUSH=1 \
  ./target/release/monitor --test --review
```

It exited with code 2 as designed because the required 2026-07-18 ledger NAV was absent. No real order or real notification was sent. This is fail-closed evidence, not Gate D live-account evidence.

## Gate D blockers

Coverage was regenerated with:

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
```

The threshold check failed:

- Global: 42,895 / 84,187 = **50.95%**, required ≥ 80%
- Core trading/data paths: 11,802 / 21,298 = **55.41%**, required ≥ 95% across 94 files

Also missing:

- Real-account same-trading-day cash, position, and NAV validation
- Real-data evidence for the accounting identity and freshness requirements
- Gate D auditor sign-off

These are mandatory merge gates under `AGENTS.md` Parts 1, 2, and 4. The Controlled Exception Path cannot bypass hard data or fund-safety red lines.

## Safe continuation

1. Keep PR #2 Draft and do not merge it.
2. Add tests for core trading/data paths first, then close global coverage. Use TDD; do not shrink the denominator or relabel required live-path tests as ignored.
3. Connect and validate genuine account cash/position/NAV snapshots, the same-day ledger, accounting identity, and freshness without sending real orders.
4. Rerun complete Gate B, Gate C, coverage, regression, and live-data checks after changes.
5. Obtain auditor sign-off and update the planning evidence and complete PR checklist.
6. Run final Standards and Spec review, mark the PR Ready, and merge through the PR. Never force or directly bypass the PR gates.

## Mandatory guardrails

- Before any edit, reread `AGENTS.md`, `docs/ENGINEERING_RULES_V2.md`, and `CLAUDE.md`. `.github/copilot-instructions.md` was absent at the time of this handoff and must be reported honestly.
- Output the required pre-flight plan before every file/code change.
- Preserve the four user-owned unstaged changes listed above.
- Never fabricate coverage, account evidence, production logs, audit approval, or GitHub checks.
- Data safety > fund safety > process compliance > development efficiency.
- To roll back the task commits, revert newest first: `git revert 70b7221 e7db307`. An order-audit-chain downgrade additionally requires a writer freeze or a compatible writer.

## Existing artifacts

- Design: `docs/superpowers/specs/2026-07-17-repository-history-and-gate-remediation-design.md`
- Plan: `.planning/2026-07-16-event-replay-safety-remediation/task_plan.md`
- Findings: `.planning/2026-07-16-event-replay-safety-remediation/findings.md`
- Progress: `.planning/2026-07-16-event-replay-safety-remediation/progress.md`
- Business rules: `docs/business_rules.md`
- Coverage report: `target/coverage/coverage.json`
- PR: <https://github.com/Northofqing/stock_analysis/pull/2>

## Suggested skills

- `planning-with-files` — continue the multi-stage task with durable progress tracking.
- `tdd` — add core-path, global-coverage, and real-account integration tests.
- `systematic-debugging` or `diagnosing-bugs` — investigate any new test/runtime failure before changing behavior.
- `requesting-code-review` — obtain the final pre-merge review after Gate D closes.
- `review` — rerun independent Standards and Spec review.
- `executing-plans` — execute the existing plan with checkpoints.

## Resume commands

```bash
git status --short
git log --oneline -3
git rev-list --left-right --count '@{upstream}...HEAD'
gh pr view 2 --repo Northofqing/stock_analysis \
  --json isDraft,state,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,baseRefName,headRefName
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
```
