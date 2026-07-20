# Persistent DataMode Reminder Design

**Date:** 2026-07-20

**Scope:** production monitor DataMode notification liveness only

**Rules:** AGENTS 2.1, 2.2, 2.4, 2.7, 2.8, 2.10; BR-108, BR-111, BR-113, BR-116, BR-135

## 1. Observed failure and reproducible evidence

The release monitor remained alive, but the account owner observed no user-visible notification.
Privacy-safe durable aggregation for the master segment produced:

```text
daily_report|Deny:data_quality|0|none|1
data_mode|Approve|1|feishu|1
earnings_miss|Deny:data_quality|0|none|40
RED business_push_receipts=0
```

The assertion was run twice with the same result. It excludes event IDs, user IDs, securities,
targets, message bodies, validation payloads, credentials, and platform identifiers. The route
aggregate for the same segment contained one validated CLI receipt and zero HTTP, dry-run, sink,
or target-resolution failures.

Source tracing proves ordinary messages are rejected by L5 before dedup and delivery while
DataMode is Unsafe/Down. The one DataMode alert was accepted by Feishu OpenAPI with a real platform
message ID. BR-116 then commits the Unsafe state; every later evaluation of the same mode is a
no-op even while capability failures keep retrying. Provider acceptance is not proof that a
particular human saw the configured chat.

Reproduction commands:

```bash
sqlite3 -batch -noheader data/push_analytics.db \
  "SELECT template_id, governance_decision, pushed, sink_name, COUNT(*) \
   FROM push_analytics WHERE ts >= '2026-07-20T16:01:07+08:00' \
   GROUP BY template_id, governance_decision, pushed, sink_name \
   ORDER BY template_id, governance_decision, pushed, sink_name;"

sqlite3 -batch -noheader data/push_analytics.db \
  "WITH x AS (SELECT COUNT(*) AS n FROM push_analytics \
   WHERE ts >= '2026-07-20T16:01:07+08:00' AND pushed=1 \
   AND template_id NOT IN ('data_mode','account_mode')) \
   SELECT CASE WHEN n=0 THEN 'RED business_push_receipts=0' \
   ELSE 'GREEN business_push_receipts=' || n END FROM x;"
```

Observed output from those commands is the four-line aggregate at the start of this section. The
production call-chain claim is independently reproducible with multiline context:

```bash
rg -n -A6 -B2 'evaluate_data_mode_hook\(|push_data_mode_change\(' \
  src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
rg -n -A7 -B3 'dispatch_outcome\(crate::notify::PushKind::DataMode|publish_delivery\(' \
  src/bin/monitor/push_templates.rs src/bin/monitor/notify.rs
```

Observed production-path output (test callers omitted here, but retained by the command):

```text
src/bin/monitor/main.rs:1696:async fn evaluate_data_mode_hook() {
src/bin/monitor/main.rs:1780:        match pt::push_data_mode_change(&input, prev, persistent_reminder_due, Some(&banner)).await
src/bin/monitor/main.rs:2966:        evaluate_data_mode_hook().await;
src/bin/monitor/main.rs:5692:            evaluate_data_mode_hook().await;
src/bin/monitor/push_templates.rs:6348:pub async fn push_data_mode_change(
src/bin/monitor/push_templates.rs:6438:    let outcome = dispatch_outcome(crate::notify::PushKind::DataMode, "", banner, text).await;
src/bin/monitor/notify.rs:1134:    let audit_result = stock_analysis::event::publish_delivery(
```

## 2. Root cause

There are two separate boundaries:

1. Unsafe data correctly blocks ordinary business messages under BR-113. News, earnings, reports,
   and trading advice must not bypass missing real evidence.
2. DataMode itself is transition-only. After one provider-accepted alert, a persistent Unsafe state
   has no new business fact under BR-116 and therefore produces no further notification.

The first behavior is a safety invariant. The second is the notification-liveness defect.

## 3. Considered approaches

| Approach | Decision | Reason |
| --- | --- | --- |
| Fixed reminder for persistent Unsafe | adopt | bounded change, existing real health input and delivery path, avoids silent indefinite degradation |
| One alert per failing source/retry | reject | retry frequency would amplify into an alert storm and require new source-specific contracts |
| Human/client acknowledgement protocol | reject for this repair | strongest semantics but requires a new external callback/interaction subsystem |

## 4. Chosen design

`monitor::data_mode` gains a deep reminder-state module with a three-method interface:

- `should_dispatch(mode, now)` returns true only for Unsafe when no confirmed alert exists or the
  last confirmed alert is at least 30 minutes old;
- `observe_mode(mode)` clears the prior outage timestamp as soon as real health is Full/Degraded,
  independently of whether the recovery notification can be delivered;
- `record_confirmed(mode, now)` stores the timestamp for Unsafe only after authoritative delivery.

The caller supplies `Instant`; the module does not read wall time, databases, environment, or the
notification sink. A poisoned lock is an explicit error. Process restart deliberately starts with
no inferred timestamp: the existing first-Unsafe alert re-establishes confirmation.

`push_templates::data_mode_notification_plan` adds one reason: real transition (including first
non-Full), or persistent Unsafe reminder. Both use `PushKind::DataMode`. A reminder renders a
distinct “数据状态持续异常” text with the current real mode, missing capability labels,
restrictions, and recovery ETA. It does not pretend Unsafe changed to Unsafe and does not add
account, price, holding, or security fields.

The main hook computes the reminder decision from the same evaluated `DataHealth` snapshot used to
build the banner. Real Full/Degraded observation clears the prior outage interval before any
notification attempt. After the awaited dispatch returns `Pushed`, the hook samples a fresh
`Instant` and commits the Unsafe reminder time. Denied, deduped, sink, L7, or immutable-audit
failure commits neither reminder time nor a new latest-notified mode. Silent/no-op Unsafe
evaluation never refreshes an Unsafe reminder timestamp.

The 30-minute threshold is a registered BR-135 code constant, not a `config/*.toml` change. It is
longer than source retry loops, short enough to expose a half-hour outage, and bounds a stable
outage to at most 48 confirmed reminders per day. Unconfirmed attempts intentionally remain due
and may retry at the hook cadence until one authoritative delivery succeeds. No ordinary
business-message gate changes.

The startup hook remains an immediate one-shot. A single dedicated scheduler, joined alongside
the market and news loops rather than nested inside a market-session branch, invokes the same hook
every 60 seconds in every session, including weekends and closed markets. Its first tick is delayed
by one full period so startup does not double-dispatch, and missed ticks use `Skip` rather than
bursting delayed evaluations. The prior session-internal call is removed to keep exactly one
recurring owner.

## 5. Data flow

```text
startup one-shot ----------+
dedicated 60s scheduler ---+-> DataHealth evaluate -> mode + missing capabilities
                                      |
                                      +-> Full/Degraded? clear prior outage interval
                                      |
                                      +-> mode transition? --------+
                                      |                            |
                                      +-> same Unsafe + 30m due? --+-> DataMode render
                                                                   -> L4 dedup
                                                                   -> L5 DataSourceDown exemption
                                                                   -> real Feishu sink
                                                                   -> L7 + immutable audit
                                                                   -> sample confirmation time
                                                                   -> commit reminder timestamp
```

## 6. Failure modes

- Reminder lock poisoned: log an explicit BR-135 error and do not dispatch or advance state.
- Clock ordering anomaly: `Instant::checked_duration_since` failure keeps the reminder due and
  reports an explicit error; no zero-duration fallback.
- Governance/sink/audit failure: retain the old timestamp so the next evaluation retries.
- Capability data unavailable: the existing hook returns before notification; no fabricated
  health snapshot or reminder is produced.
- Scheduler ownership: the recurring loop is independent of market-session branches, so weekend,
  closed-session, and early-continue paths cannot suppress it. Its unexpected exit/panic is a
  monitor-loop failure, not a silently ignored background task.
- Delayed or suspended runtime: missed ticks use `Skip`; the scheduler performs one current
  evaluation instead of replaying a burst of stale checks.
- Startup: the immediate one-shot establishes governance, while the recurring scheduler waits one
  full 60-second period before its first evaluation to avoid a duplicate startup attempt.
- Recovery: observed real Full/Degraded clears the Unsafe reminder timestamp even when the
  recovery notification is unconfirmed; a later Unsafe state starts a new outage interval.
- Wrong but valid configured chat: outside this code repair. The system must not guess or commit a
  replacement notification target; target verification remains an operational secret action.

## 7. Old-module decisions

| Module | Decision | Reason |
| --- | --- | --- |
| `monitor::data_mode::evaluate` | adopt | real capability freshness remains the sole health source |
| `LATEST_DATA_MODE` exact transition state | adopt | BR-116 retry semantics stay unchanged |
| v14 L4/L5/L6/L7 and BR-091 audit | adopt | the reminder must use the same real governed path |
| per-source retry logs | reject as notification source | high-frequency and not an aggregate outage contract |
| MagicLaw/Feishu destination configuration | unchanged | intended human target cannot be safely inferred |

## 8. Acceptance and rollback

Automated acceptance:

```bash
cargo test --lib monitor::data_mode::tests::br135_persistent_unsafe_reminder -- --exact
cargo test --bin monitor br135 -- --nocapture
cargo test --bin monitor br135_scheduler_waits_before_first_tick_and_runs_independently -- --exact
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Machine-checkable expected outputs:

| Command | Expected output |
| --- | --- |
| exact library BR-135 test | `1 passed; 0 failed` |
| monitor `br135` filter | all selected tests pass; `0 failed`; at least four tests selected |
| independent scheduler test | no call before the delayed first tick; calls continue after ticks |
| fmt | exit 0, no diff |
| strict Clippy | exit 0 and no warning/error diagnostics |
| full workspace test | exit 0 and every executed test target reports `0 failed` |
| compliance | final line `[compliance] ALL CHECKS PASSED` |
| coverage threshold script | global `>= 80.00%` and core `>= 95.00%`, exit 0 |
| release build | exit 0 and `Finished release profile` |

Release-candidate and merged-master production acceptance is also machine-checkable with fixed
before/after counts: one due reminder must add exactly one `data_mode` row with `pushed=1` and
`sink_name='feishu'`, one `push.delivery.audit` event-bus row, and one immutable hash-chain audit
row; private-log counters must add one BR-135 due and one confirmation, with zero banner, sink,
audit, panic, or fatal failure. A failed attempt must add no confirmation and remain due.

No message content or destination is printed or committed during production acceptance.

Rollback is `git revert <merge-commit>`, rebuild release, terminate only the current monitor PID,
and restart the preserved previous master binary if the reverted build cannot start. Databases,
audit chains, real-account evidence, and the private append-only log are never rewritten.
