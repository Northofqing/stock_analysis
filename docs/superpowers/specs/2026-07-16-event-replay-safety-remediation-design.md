# Event Replay Safety Remediation Design

## Status and references

- Status: Approved for autonomous execution by the user's instruction to complete all fixes without confirmation.
- Refs: `docs/v17.x/v17.3-migration-and-persistence.md` §5.8, D8, D9, AC32.
- Implementation plan: `docs/superpowers/plans/2026-07-16-event-replay-safety-remediation.md`.
- Business rule: BR-043.
- Data red lines: 2.1, 2.2, 2.4, 2.7, 2.10.

## Problem

The v17.3 event CLI and replay implementation passes its focused happy-path tests but violates seven documented behaviors: monitor flags terminate event parsing, the documented replay-rate equals form is rejected, history limit zero is rejected, forced replay is not throttled, invalid source payloads can be published without a replay marker, failed bus publication is counted as success, and replay IDs repeat across separate runs.

## Selected design

Keep the existing `EventCommand`, `EventBus`, `EventEnvelope`, `HistoryQuery`, and monitor terminal-command architecture. Do not introduce another CLI framework or replay bus.

1. CLI parsing skips known legacy monitor flags and continues scanning. Both `--replay-rate-ms N` and `--replay-rate-ms=N` populate the same `u32` value. History limit rejects only negative integers; explicit zero remains `Some(0)` and `HistoryQuery` does not truncate when the propagated limit is zero.
2. `ReplayRunner::run` returns a `ReplaySummary` with attempted, replayable, published, skipped, and failed counts. File-open and stream I/O remain `ReplayError`; record and publish failures are visible in the summary so valid rows can continue while the CLI still exits nonzero if any row failed.
3. A `push.source` row is replayable only when `payload.text` is a non-empty string. Force mode prefixes it before publication. Missing, non-string, or blank text increments failed and is never published.
4. Replay IDs combine the original ID with process ID and a process-wide atomic sequence. Replaying the same file repeatedly cannot reuse IDs within a process, and ID creation has no silent clock fallback.
5. Nonzero `rate_ms` sleeps between force-mode publish attempts, not before the first attempt and not for dry-run/non-replayable rows.
6. `ReplayRunner` publishes through an async `ReplayPublisher` with structured errors. The normal `EventBus` adapter preserves bus-level tests; the monitor supplies an injected publisher that writes a durable attempt audit, awaits the real notification sink, writes the result audit, and succeeds only after sink acceptance and audit persistence.
7. The monitor always prints the full summary. `ReplayError` or `summary.failed > 0` returns exit code 1; zero replayable rows with no failures remains a truthful zero-dispatch success.
8. History output iterates every query result. The default query still limits to 100, while explicit zero is unbounded through parsing, querying, and printing.

## Data flow

`argv` → `event::cli::parse_args` → terminal `EventCommand::Replay` → JSONL line parse → event type filter → replay body validation → fresh envelope ID and `replay_of` → replay marker → optional inter-attempt delay → `ReplayPublisher::publish` → durable attempt audit → awaited real notification sink → durable result audit → `ReplaySummary` → monitor stdout/stderr and exit code.

## Failure modes

| Failure | Behavior |
|---|---|
| Missing date file / read error | Return `ReplayError`; CLI exits 1. |
| Malformed JSON | Increment failed, log the line error, continue; CLI exits 1 after the scan. |
| `push.source` missing/blank/non-string text | Increment failed, never publish, continue; CLI exits 1. |
| Non-source event | Increment skipped, never publish. |
| No subscribers / bus rejected | Increment failed, never count as published; CLI exits 1. |
| Notification sink rejects/fails | Await the sink result, increment failed, never count as published; CLI exits 1. |
| `V10_DRY_RUN_PUSH=1` during force replay | Reject before calling the sink; a log-only dry run is never counted as a real publication. |
| Replay attempt audit write fails | Do not call the notification sink; increment failed and exit 1. |
| Replay result audit write fails | Report failure and exit 1; the durable attempt record still identifies the envelope and decision. |
| Existing replay audit is malformed, missing hashes, hash-mismatched, or chain-broken | Reject the audit append before calling the notification sink; never silently restart from `GENESIS`. |
| Nonzero rate | Delay only between consecutive force-mode publish attempts. |

## Old module relations

| Module | Decision | Reason |
|---|---|---|
| `src/event/cli.rs` | adopt | Single event CLI parser; correct its composition and documented forms. |
| `src/event/replay.rs` | adopt | Single JSONL replay engine; deepen validation and result semantics. |
| `src/event/bus.rs` | adopt unchanged | Its `PublishOutcome` already distinguishes success and failure. |
| `src/event/history.rs` | modify | Remove the old zero-to-100 normalization so explicit unbounded mode reaches query results. |
| `src/bin/monitor/main.rs` | adopt | Keep the existing terminal-command integration and make error exits truthful. |
| `src/bin/monitor/notify.rs::push_wechat` | adopt | Explicit force replay already carries rendered, marked text and must reach the real configured sink without ordinary live-event dedup suppressing the operator-requested replay. `V10_DRY_RUN_PUSH=1` is rejected before this call. |
| `data/replay_audit/<year>.jsonl` | add | Append and `sync_data` attempt/result records with envelope ID, `replay_of`, source, time, decision basis, and a SHA-256 previous-hash chain. No automatic cleanup is configured, preserving the five-year retention requirement. |

## Rollback

Revert the remediation commit. No schema or persisted-data migration is introduced. Existing JSONL files remain compatible because only read-time validation and cloned replay envelope IDs change.

## Acceptance criteria

1. Known monitor flags do not suppress an event command in either order.
2. Both replay-rate syntaxes parse identically.
3. `--limit=0` produces explicit unbounded mode; negative limits fail.
4. A two-event force replay observes at least the requested inter-attempt delay.
5. Invalid source text is never published and is reported failed.
6. `NoSubscribers` and `Rejected` are failed, not published.
7. Two runs over the same source create disjoint IDs.
8. Module tests, monitor build, fmt, clippy, full tests, and compliance are executed; any unrelated pre-existing gate failure is reported without fabrication.

## Reproduction evidence

| Reviewed symptom | Failing command / observed result before fix | Passing evidence after fix |
|---|---|---|
| Monitor flag swallowed history | `cargo test --lib event::cli::tests::cli_keeps_history_command_when_monitor_flags_are_present -- --exact` → expected `History`, got `None` | `cargo test --lib event::cli::tests` → 15/15 pass |
| Equals-form replay rate rejected | `cli_parses_documented_replay_rate_equals_form` → `UnrecognizedFlag` | CLI suite pass |
| `limit=0` rejected/truncated | parser test → `InvalidLimit(0)`; 101-row query test → 100 rows | parser preserves `Some(0)`; query returns 101 rows |
| Invalid source body published | `force_replay_rejects_source_without_string_text` → published/count 1 | replay suite pass; published 0, failed 1 |
| Rate ignored | two-event 30ms test → elapsed below 30ms | receiver test observes the first event before the configured interval and rejects the second until that interval elapses |
| Replay IDs repeated | repeated-run test produced `replay-original-1` twice | repeated-run IDs differ |
| Publish failure reported success | no-subscriber/shutdown tests could only observe `Ok(1)` | `ReplaySummary { published: 0, failed: 1 }` |

## Executable acceptance evidence

| Acceptance | Command | Expected terminal result |
|---|---|---|
| AC1–AC3 | `cargo test --lib event::cli::tests` | `15 passed; 0 failed` |
| AC4–AC7 | `cargo test --lib event::replay::tests` | `11 passed; 0 failed`; pacing test completes in about 0.21s |
| Production publisher failures/audit | `cargo test --bin monitor monitor_replay_publisher_tests` | `5 passed; 0 failed`; includes corrupt-audit rejection and no real notification sink is called |
| History unbounded query/output | `cargo test --lib event::history::tests::zero_limit_returns_and_formats_all_matching_history_entries -- --exact` | `1 passed; 0 failed`; query and formatter lengths are 101 |
| Integrated event regressions | `cargo test --lib event::` | `74 passed; 0 failed` |
| Production call path compilation | `cargo build --bin monitor` | exit 0; `Finished dev profile` |
| Data/business-rule compliance | `bash tools/compliance/check.sh` | exit 0; `[compliance] ALL CHECKS PASSED` |

### Captured raw terminal output

```text
$ cargo test --lib event::cli::tests -- --nocapture
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 1246 filtered out

$ cargo test --lib event::replay::tests -- --nocapture
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1250 filtered out; finished in 0.21s

$ cargo test --bin monitor monitor_replay_publisher_tests -- --nocapture
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 258 filtered out

$ cargo test --lib event:: -- --nocapture
test result: ok. 74 passed; 0 failed; 0 ignored; 0 measured; 1187 filtered out; finished in 0.21s

$ cargo test --lib
test result: ok. 1254 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 2.69s

$ bash tools/compliance/check.sh
[compliance] ALL CHECKS PASSED

$ cargo llvm-cov --lib --summary-only
event/cli.rs      lines 91.50%
event/history.rs  lines 88.80%
event/replay.rs   lines 98.00%
TOTAL             lines 51.14%
```
