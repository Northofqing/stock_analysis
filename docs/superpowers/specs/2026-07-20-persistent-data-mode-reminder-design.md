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

`monitor::data_mode` gains a deep reminder-state module with a two-method interface:

- `should_dispatch(mode, now)` returns true only for Unsafe when no confirmed alert exists or the
  last confirmed alert is at least 30 minutes old;
- `record_confirmed(mode, now)` stores the timestamp for Unsafe and clears it for Full/Degraded.

The caller supplies `Instant`; the module does not read wall time, databases, environment, or the
notification sink. A poisoned lock is an explicit error. Process restart deliberately starts with
no inferred timestamp: the existing first-Unsafe alert re-establishes confirmation.

`push_templates::data_mode_notification_plan` adds one reason: real transition (including first
non-Full), or persistent Unsafe reminder. Both use `PushKind::DataMode`. A reminder renders a
distinct “数据状态持续异常” text with the current real mode, missing capability labels,
restrictions, and recovery ETA. It does not pretend Unsafe changed to Unsafe and does not add
account, price, holding, or security fields.

The main hook computes the reminder decision from the same evaluated `DataHealth` snapshot used to
build the banner. A `Pushed` result commits both latest mode and reminder time. Denied, deduped,
sink, L7, or immutable-audit failure commits neither reminder time nor a new mode. Silent/no-op
evaluation never refreshes an Unsafe reminder timestamp. Full or Degraded confirmation clears the
Unsafe reminder state.

The 30-minute threshold is a registered BR-135 code constant, not a `config/*.toml` change. It is
longer than source retry loops, short enough to expose a half-hour outage, and bounds a stable
outage to at most 48 reminders per day. No ordinary business-message gate changes.

## 5. Data flow

```text
real capability successes -> DataHealth evaluate -> mode + missing capabilities
                                      |
                                      +-> mode transition? --------+
                                      |                            |
                                      +-> same Unsafe + 30m due? --+-> DataMode render
                                                                   -> L4 dedup
                                                                   -> L5 DataSourceDown exemption
                                                                   -> real Feishu sink
                                                                   -> L7 + immutable audit
                                                                   -> commit reminder timestamp
```

## 6. Failure modes

- Reminder lock poisoned: log an explicit BR-135 error and do not dispatch or advance state.
- Clock ordering anomaly: `Instant::checked_duration_since` failure keeps the reminder due and
  reports an explicit error; no zero-duration fallback.
- Governance/sink/audit failure: retain the old timestamp so the next evaluation retries.
- Capability data unavailable: the existing hook returns before notification; no fabricated
  health snapshot or reminder is produced.
- Recovery: confirmed Full/Degraded clears the Unsafe reminder timestamp.
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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Production acceptance after merge/restart uses aggregate evidence only: an initial or due
`data_mode` row must record `pushed=1`, Feishu sink, and immutable `push.delivery.audit`; a failed
attempt must leave the reminder due. No message content or destination is printed or committed.

Rollback is `git revert <merge-commit>`, rebuild release, terminate only the current monitor PID,
and restart the preserved previous master binary if the reverted build cannot start. Databases,
audit chains, real-account evidence, and the private append-only log are never rewritten.
