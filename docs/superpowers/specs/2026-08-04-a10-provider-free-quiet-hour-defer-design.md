# BR-209 A-10 Provider-Free Quiet-Hour Defer Design

**Status:** Gate C passed for the narrow A-10 preflight slice; independent
review C0/I0/M0. Gate D remains blocked by repository-wide coverage authority
and threshold closure, not by BR-209 behavior.

## Intent

Manual review can run during the existing 02:00–06:00 notification quiet
window. A-10 currently acquires and validates multiple real provider batches,
renders a report, and only then reaches the L5 quiet-hour denial. This creates
avoidable provider traffic and a retryable failure whose next-attempt time does
not describe the actual policy boundary.

BR-209 adds an A-10-only static review preflight result. During the quiet
window the task is removed before dependency partitioning and returns a typed
absolute defer instant. Provider, renderer and sink call counts are therefore
all zero for that invocation. The existing L5 check remains as the final race
defence and is not weakened.

## Reproducible problem evidence

The pre-change production audit is inspectable without contacting a provider:

```bash
rg -n 'A-10|quiet_hour' data/review_audit/2026-08-03.jsonl | tail -12
```

Observed records at 02:26, 02:42, 03:08, 03:56 and 04:44 show A-10 with
`source=chain_rotation_security_master` followed by a terminal quiet-hour
governance failure. Records at 05:18 and 05:46 show the later BR-207 retryable
classification, but still retain the provider source and schedule another
attempt one minute later. The same task delivers outside the window at 06:04.
Those immutable records prove the time gate was reached after source work and
that retryability alone did not create a provider-free boundary.

Exact compact evidence command and output:

```text
$ sed -n '57p;102p' data/review_audit/2026-08-03.jsonl | jq -c '{observed_at:.payload.observed_at,task:.payload.task,source:.payload.source,status:.payload.status,retryable:.payload.retryable,next_attempt:.payload.next_attempt,failure_reason:.payload.failure.reason}'
{"observed_at":"2026-08-04T02:26:31","task":"A-10","source":"chain_rotation_security_master","status":"failed","retryable":false,"next_attempt":null,"failure_reason":"delivery denied by push governance: quiet_hour"}
{"observed_at":"2026-08-04T05:18:30","task":"A-10","source":"chain_rotation_security_master","status":"failed","retryable":true,"next_attempt":"2026-08-04T05:19:30","failure_reason":"delivery deferred by push governance: quiet_hour"}
```

The executable sequence is visible with line-stable symbol queries:

```bash
rg -n 'review_preflight|partition_review_tasks|dispatch_catalyst_review_daily_outcome' \
  src/bin/monitor/{review_batch,push_templates}.rs
rg -n 'quiet_hour_active_at|current_quiet_hour' \
  src/bin/monitor/v14_adapter.rs
```

`review_preflight` precedes `partition_review_tasks`; the source-only A-10
dispatcher is called from that partition, and L5 evaluates the quiet predicate
at the delivery boundary. Therefore removing A-10 in preflight is the narrow
boundary that guarantees zero A-10 provider, renderer and sink calls.

Exact symbol-query output proving that sequence:

```text
src/bin/monitor/push_templates.rs:7215:    let preflight = review_preflight(context, due, is_test);
src/bin/monitor/push_templates.rs:7216:    let phases = partition_review_tasks(&preflight.runnable);
src/bin/monitor/push_templates.rs:7249:                    dispatch_catalyst_review_daily_outcome(&date).await,
src/bin/monitor/v14_adapter.rs:1351:fn current_quiet_hour(now: chrono::DateTime<Local>) -> bool {
src/bin/monitor/v14_adapter.rs:1352:    quiet_hour_active_at(now.time())
src/bin/monitor/v14_adapter.rs:1358:pub(crate) fn quiet_hour_active_at(now: chrono::NaiveTime) -> bool {
```

## Data flow

1. `ReviewRunContext` freezes the evidence business date and the real wall
   observation time.
2. Test isolation and the existing disabled-task gates run first.
3. If A-10 is still due and the shared quiet-hour policy admits the wall time,
   preflight removes A-10 and emits `DeferredUntil` with the earliest 06:00
   Asia/Shanghai recheck instant.
4. The task never reaches source partitioning, provider acquisition, rendering
   or the sink in that invocation.
5. BR-140 writes a typed transition with `status=deferred`, BR-209 evidence and
   the exact RFC3339 `+08:00` instant. Manual `--review` reports that the user
   must invoke the command again at or after that instant; no durable automatic
   wake-up is claimed.
6. Every newly constructed task transition is serialized and read back through
   the strict transition validator before it is eligible for audit append.

## Absolute-time rule

- For a normal 02:00–05:59:59 observation, defer to the same wall date at
  06:00:00+08:00.
- When the explicit operations override forces quiet mode outside that range,
  defer to the next wall date at 06:00:00+08:00.
- The defer instant is based on observation wall date, never the earlier
  review/evidence business date. This prevents weekend or early-morning manual
  review from recording an already-expired attempt.

## Failure modes and non-goals

- No provider data is fabricated or cached as a substitute.
- A defer is neither a successful delivery nor a source failure.
- In-memory scheduler eligibility may use the absolute instant, but this slice
  does not claim restart-durable wake-up. Manual CLI exits non-zero if nothing
  was delivered and prints an explicit reinvocation instruction.
- Test isolation takes precedence and produces exactly one Disabled outcome,
  with no defer duplicate.
- The existing L5 quiet-hour denial and BR-207 retry classification remain for
  races and non-A-10 callers.
- The first 2026-08-04 live probe emitted one validly hash-chained deferred
  record before the typed defer payload existed. Rule 2.7 forbids rewriting or
  deleting it. The reader admits only that exact immutable JSON value and
  record hash; every other deferred record requires typed defer evidence. This
  compatibility exception creates no new producer path.

## Old modules

| module | decision | reason |
| --- | --- | --- |
| `review_preflight` | amend | It is the only provider-free task boundary. |
| L5 quiet-hour policy | reuse via a pure predicate | Preflight and sink governance must not drift. |
| BR-207 late denial classification | retain | Race defence and other review paths still need it. |
| source dispatchers | unchanged | They must not learn time-policy rules. |

## Validation

- Focused RED/GREEN tests for 02:00, 05:59:59, 06:00, test-mode precedence,
  an earlier review business date, override behaviour and transition wire.
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`
- `bash tools/compliance/check.sh`
- release monitor build plus bounded review/test/normal monitor invocations.

Final evidence on 2026-08-04:

- Independent review: C0/I0/M0; Gate A and Gate B accepted.
- BR-209 focused tests: 8/8 passed, including rejection of wrong offsets,
  wrong wall times, prior/next invalid dates and malformed observation time.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  passed.
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`:
  passed.
- `bash tools/compliance/check.sh`: passed, including one-trading-day data
  freshness.
- `cargo build --release --bin monitor`: passed.
- `cargo run --bin monitor -- --review`: exited 0; A-10 used admitted
  Eastmoney/TDX evidence and obtained a validated Feishu receipt.
- `cargo run --bin monitor -- --test --push-dry-run`: exited 0 with 48/48
  template families, 6/6 smoke checks and zero external processes.
- Bounded `cargo run --bin monitor`: started all schedulers, refreshed 7/7
  actual-position chain assignments, obtained a validated DataMode Feishu
  receipt and shut down cleanly on SIGINT without a Tokio runtime panic.
- Bare `cargo run --bin monitor -- --test` remains deliberately fail-closed at
  `BR-196 live_acceptance_not_opted_in` because no physically separate,
  allowlisted test Feishu target is configured; it attempted no external
  process.

## Rollback

Revert the BR-209 outcome/schedule/transition additions, the shared pure
predicate call, preflight branch, CLI diagnostic, tests and this registration.
Do not remove or weaken the underlying L5 quiet-hour gate or any audit history.
