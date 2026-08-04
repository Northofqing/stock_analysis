# Task Plan: master release monitor 48-hour observation

## Goal

Build and continuously observe the `stock_analysis` release `monitor` from current `master` for
48 cumulative active hours; collect only sanitized operational evidence; repair only runtime-
blocking defects through the full Gate/PR flow and restart immediately; then merge a sanitized
operations report into `master` through a PR.

## Current Phase

Phase 3 — BR-142 delivery-audit blocker remediation (reactivated 2026-07-22).

## Phases

### Phase 1: Baseline and safety envelope

- [x] Read repository rules and record the required pre-flight.
- [x] Verify local `master` equals remote `master`.
- [x] Build the optimized `monitor` and capture its non-secret checksum.
- [x] Start exactly one master release process with a private append-only local log.
- [x] Establish the observation start and privacy/rollback boundaries.
- **Status:** complete

### Phase 2: Accumulate 48 active hours

- [ ] Keep exactly one release `monitor` process alive.
- [ ] Track active-runtime segments and exclude restart downtime from the 48-hour total.
- [ ] At each observation, record only fixed aggregate counters and health/timestamp metadata.
- [ ] Confirm log progress, process liveness, platform receipt outcomes, governance availability,
      source failures, panics/fatal exits, database/audit failures, and retry behavior.
- [ ] Classify every concrete issue as blocking or non-blocking without exposing raw payloads.
- **Status:** pending restart after the BR-142 blocker is merged

### Phase 3: Conditional blocker remediation

- [x] Reproduce the notification-liveness defect and identify its root cause.
- [x] Before code edits, update Gate A design/BR evidence and issue a fresh pre-flight.
- [x] Implement the smallest compliant fix with explicit failure handling and regression tests.
- [x] Run fmt, strict Clippy, full tests, compliance, coverage, and the final release build.
- [ ] Complete final-HEAD live validation and an independent five-step Gate verifier.
- [ ] Merge via PR, rebuild on `master`, restart exactly one process immediately, and append the
      new active-runtime segment without discarding earlier valid runtime.
- [ ] Complete PR #10 announcement relevance/failure-isolation canary, review, merge, and restart.
- [ ] Separate operational DataMode alerts from account-banner availability without weakening any
      live-order account gate.
- [ ] Restore the persisted virtual portfolio through a dedicated paper-ledger risk context; do not
      clear Filled history or invent cash/account facts.
- [ ] Repair auction market-rule validation and isolate one-symbol provider failures after the
      notification/virtual-account boundary is safe.
- **Status:** in_progress (BR-142 persisted-legacy compatibility)

### Phase 4: Completion audit and operations report

- [ ] Prove cumulative active runtime is at least 48 hours from authoritative segment evidence.
- [ ] Reconcile every explicit objective requirement against source/runtime/PR evidence.
- [ ] Write a sanitized report in `docs/` covering provenance, observation windows, aggregate
      findings, incidents/fixes, unresolved non-blocking debt, privacy controls, and rollback.
- [ ] Verify the report contains no account values, security identifiers, credentials,
      notification targets, platform/message identities, or message bodies.
- **Status:** pending

### Phase 5: Report Gate and merge

- [ ] Run all repository-required validation applicable to the report/final tree.
- [ ] Obtain fresh independent Gate A–D/audit sign-off with zero blocking objections.
- [ ] Open a complete PR, mark Ready only after every checklist item passes, and merge to master.
- [ ] Verify local/remote master equality and the continued single release process.
- **Status:** pending

## Time Accounting

- Segment 1 start: `2026-07-20T16:01:07+08:00` (first master-release log record).
- Segment 1 end: `2026-07-20T17:45:51+08:00` (last structured base-master record before controlled candidate switch).
- Segment 2 start: `2026-07-20T17:46:09+08:00` (first release-candidate record).
- Segment 2 end: `2026-07-20T19:12:46+08:00` (last structured superseded-candidate record).
- Segment 3 start: `2026-07-20T19:13:16+08:00` (first final-HEAD candidate record).
- Segment 3 end: `2026-07-20T20:08:43+08:00` (last structured pre-scheduler-fix candidate record).
- Segment 4 start: `2026-07-20T20:09:54+08:00` (first `a7dfd02` candidate record).
- Segment 4 end: `2026-07-20T21:01:25+08:00` (last structured candidate record before the merged-master switch).
- Segment 5 start: `2026-07-20T21:02:09+08:00` (first merged-master `0e06543` record).
- Segment 5 end: `2026-07-20T21:48:10+08:00` (last pre-BR-136-master record).
- Segment 6 start: `2026-07-20T21:48:31+08:00` (first BR-136 master `c55411b` record).
- Segment 6 end: `2026-07-21T08:03:43+08:00` (last structured record before the next startup).
- Segment 7 start: `2026-07-21T08:08:51+08:00` (first structured record after the 308-second gap).
- Segment 7 end: `2026-07-21T19:52:21+08:00` (last structured record in the private log).
- Segment 8 start: `2026-07-22T09:46:33+08:00` (first current-master startup record).
- Segment 8 end: `2026-07-22T10:04:41+08:00` (last structured record before the user-requested stop).
- Closed-segment cumulative active runtime: `27:43:02` (`99,782` seconds). Excluded restart
  gaps total `00:08:12`; the later unsupervised gap after segment 7 is excluded in full.
- Final cumulative active runtime: `28:01:10` (`100,870` seconds). The 48-hour target was not met
  before the stop. The objective has since resumed; only a new verified segment may add time.
- Cumulative active runtime: recompute from closed/open segments at every checkpoint; never infer
  completion from wall-clock date alone.
- Continuation audit at `2026-07-22T09:21:49+08:00`: no release monitor, caffeinate wrapper,
  supervisor, or launchd service is running. The entire gap after segment 7 is excluded.

## Key Questions

1. Does exactly one master release process stay alive and continue producing expected heartbeat
   or retry evidence?
2. Do any failures stop the process or a required loop, rather than merely degrade one capability?
3. Are all notifications marked `Pushed` backed by a real non-placeholder platform receipt and
   durable audit row?
4. Can every final report claim be supported by sanitized aggregate evidence without inspecting or
   committing sensitive payloads?
5. Has the final operations report itself passed PR/Gate review and landed on `master`?

## Decisions Made

| Decision | Rationale |
|---|---|
| Count cumulative active runtime in explicit segments | A compliant hotfix restart must not erase earlier valid observation, while downtime must not be counted. |
| Keep raw runtime output only in `/private/tmp/stock_analysis_monitor.log` with mode `0600` | The log may contain live-account or notification context and must never enter Git. |
| Persist only aggregate counters/classifications in planning and final docs | Satisfies the objective without leaking account, security, credential, target, identity, or message data. |
| Treat stale/missing real evidence as fail-closed degradation, not fabrication | Required by red lines 2.1–2.4; degraded operation alone is not a runtime blocker. |
| Repair only defects that block process/required-loop runtime during the 48-hour window | Matches the authorized monitoring scope and avoids unrelated behavioral expansion. |
| All code and final report changes go through PR and independent Gate verification | Required by AGENTS and CLAUDE completion rules. |

## Blocking Classification

A blocker includes process exit/crash/panic, startup failure, durable database/audit failure that
prevents required loops, or a required loop that stops making progress despite its retry schedule.
External-source unavailability, stale account evidence, and fail-closed risk restrictions remain
non-blocking when the process and retries stay alive; they are still recorded as concrete issues.

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| `ps -p 10935 ...` returned `operation not permitted` in the restricted sandbox | 1 | Use approved `pgrep -fl` and, when needed, the approved full `ps -axo ...` form with output filtered to monitor processes. |
| Sanitized Perl delta parser had an unmatched closing brace | 1 | Removed the stray brace, discarded partial stdout, and reran successfully. |
| Combined liveness probe stopped when sandboxed `pgrep` could not access the process list | 1 | Do not repeat the combined `&&` probe; use the already-approved targeted `ps -p` query and run log/database discovery independently. |
| First transport-route aggregate counted the full append-only log, including pre-segment records | 1 | Do not use it as master-segment evidence; rerun with the authoritative 16:01 timestamp boundary. |
| Sandboxed `ps eww` denied access and the downstream parser misleadingly emitted all keys as unset | 1 | Discard that output, rerun the exact read-only query with approved escalation, and never use piped output when the producer exit status is unchecked. |
| First target-shape classifier had mismatched shell quoting | 1 | Do not repeat the Perl form; use separate simple `awk` classifiers that never print configuration values. |
| Source search included nonexistent `docs/architecture` and emitted one path error | 1 | Results from existing paths were usable; future searches must enumerate actual `docs/` directories first. |
| Combined Gate A patch failed because two plan code-block lines lacked add-file `+` prefixes | 1 | No repository change was applied; split BR, design, and plan into smaller patches and verify every added line prefix. |
| First staged Gate A check found a blank line at plan EOF | 1 | Remove the single extra EOF line, restage only the plan, and rerun cached diff check before commit. |
| Task 2 RED cargo calls yielded before completion and the wrapper omitted the inner session ID | 1 | Treat the result as ambiguous, confirm no cargo/rustc process remains, then rerun with a 30-second yield and capture session_id/exit_code explicitly. |
| Sandboxed full process listing was denied during the ambiguous-cargo check | 1 | Approved read-only escalation confirmed no cargo/rustc worker remains; use the approved process-list form for future ambiguity checks. |
| First Task 3 GREEN command used `--exact` with an incomplete module path and ran 0 tests | 1 | Reject the false-green result; rerun with `br135_data_mode_reminder_tests::br135_reminder_confirmation_requires_pushed`. |
| Isolated smoke query assumed event IDs carry the `TEST_CODE` prefix | 1 | Reject that classifier; prove isolation from test-mode configuration, stripped live channel credentials, isolated paths, empty stock list, and dry-run output instead. |
| Sandboxed `kill -TERM 10935` was denied | 1 | Rerun the exact verified old monitor PID with scoped approval; both the old monitor and its caffeinate child exited before replacement startup. |
| First durable baseline query referenced a nonexistent `outcome` column | 1 | Discard that predicate; use registered `pushed`, `sink_name`, and fixed before/after counts, plus event-bus and immutable-audit counts. |
| Initial immutable-audit probe used a daily path that does not exist | 1 | Discover paths without reading content; the authoritative immutable audit is yearly `data/event_audit/2026.jsonl`. |
| First attempt staged ignored docs and Rust paths in one `git add` | 1 | The command stopped on the ignored `docs/` paths; stage Rust normally and tracked ignored docs explicitly with `git add -f`. |
| Two concurrent `git add` commands contended on `.git/index.lock` | 1 | Do not parallelize Git index mutations; verify no Git process and that the transient lock disappeared, then stage sequentially. |
| Sandboxed `pgrep` failed while checking the transient Git lock | 1 | Discard the failed combined probe; use the approved read-only process list and a separate lock-file stat. |
| Stopping the superseded candidate required a sandbox approval that remained pending for about 53 minutes | 1 | The old candidate stayed alive and logging during the approval wait; count that interval as active runtime, then record only the actual 30-second switch gap. |
| First segment-2 endpoint parser had an unmatched Perl brace | 1 | Discard its empty output and use a fixed-shape `awk` timestamp-only parser; endpoint is `19:12:46 +08:00`. |
| Sandboxed targeted process query was denied; the escalated unfiltered `ps` result was too broad and truncated | 1 | Discard the broad output as monitor evidence and use three exact `pgrep` patterns. All returned no match. |
| Rollover parser used GNU awk's third `match` argument, unsupported by macOS awk | 1 | No evidence was produced; replace it with fixed-position `substr` arithmetic and log only rollover metadata. |
| First BSD `date` completion calculation placed `-v` modifiers after input parsing and returned the unchanged start time | 1 | Reject the result; rerun with modifiers before `-f` and cross-check with epoch arithmetic. |
| First JSON key-shape query used ambiguous jq pipe precedence and attempted to index the generated key array | 1 | Discard the repeated jq diagnostics; parenthesize each projection and emit only aggregated key sets. |
| Fresh handoff coverage run exhausted disk space in a temporary worktree | 1 | Cleaned only generated temporary Cargo artifacts, reran the same serial coverage command, and passed at 80.75% global / 95.13% core. |

## Plan Execution Notes

- The requested subagent-driven execution skill and worktree skill are not available in this
  session. Execution stays inline on the dedicated branch, preserving the running master process;
  the repository-mandated fresh independent Gate verifier will still be used before merge.

## Rollback

- Observation-only work does not modify runtime state.
- Before a blocker fix deployment, preserve the currently running master binary in a private
  temporary path and record its checksum.
- After the fix PR merges, build `master` first, stop only the current monitor PID, and restart one
  process immediately.
- If startup validation fails, restore the preserved binary and continue from a new runtime segment.
- Never delete or rewrite the database, audit chain, real-account evidence, or private raw log.
