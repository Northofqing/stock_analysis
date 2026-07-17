# Event Replay Safety Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use TDD to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make v17.3 replay and history CLI behavior fail-closed, rate-limited, uniquely traceable, and faithful to its documented command forms.

**Architecture:** Preserve the existing event modules and terminal monitor integration. Deepen `ReplayRunner` with validated replay rows and a structured summary, then make the CLI propagate that summary to a truthful exit status.

**Tech Stack:** Rust, Tokio, Serde JSON, Chrono, `EventBus`, module tests.

---

### Task 1: Make event CLI parsing compositional

**Files:**
- Modify: `src/event/cli.rs`
- Modify: `src/event/history.rs`

- [x] Add public-interface tests for monitor flags combined with history in both orders, `--replay-rate-ms=N`, and `--limit=0`.
- [x] Run `cargo test --lib event::cli::tests` and confirm the new tests fail for the reviewed symptoms.
- [x] Change known monitor flags from early return to skip, parse the replay-rate prefix form, and reject only negative limits.
- [x] Add a 101-row query test proving explicit zero is not normalized to the default 100.
- [x] Run CLI and zero-limit history tests and confirm they pass.

### Task 2: Reject replay rows without a valid marked body

**Files:**
- Modify: `src/event/replay.rs`

- [x] Add a test JSONL source envelope whose `payload.text` is missing/non-string and assert that force mode publishes nothing and records a failure.
- [x] Run the targeted test and confirm it fails because the current runner publishes the row.
- [x] Add explicit non-empty string validation before cloning/publishing and expose failed-row accounting through `ReplaySummary`.
- [x] Run the targeted test and the existing replay module tests.

### Task 3: Make publish outcomes truthful

**Files:**
- Modify: `src/event/replay.rs`
- Modify: `src/event/mod.rs`
- Modify: `src/bin/monitor/main.rs`

- [x] Add tests for `NoSubscribers` and shutdown rejection, asserting published is zero and failed is nonzero.
- [x] Run them red against the current `Ok(count)` behavior.
- [x] Count only `PublishOutcome::Published` as published; count `NoSubscribers` and `Rejected` as failed; export `ReplaySummary`.
- [x] Replace monitor `unwrap_or(0)` with explicit `ReplayError` handling and exit 1 on errors or failed rows.
- [x] Run replay tests and `cargo build --bin monitor`.
- [x] Add an async `ReplayPublisher` seam and make monitor force mode await the real notification sink instead of publishing to an unsubscribed fresh bus.
- [x] Always print the full replay summary, including zero-replayable scans.

### Task 4: Enforce replay pacing

**Files:**
- Modify: `src/event/replay.rs`

- [x] Add a paused-time or elapsed-time test with two publish attempts and a nonzero rate, asserting no delay before the first attempt and at least one configured interval overall.
- [x] Run the test red and verify `_rate_ms` is ignored.
- [x] Rename the parameter to `rate_ms` and sleep between force-mode publish attempts using `tokio::time::sleep`.
- [x] Run the pacing test and all replay tests.
- [x] Strengthen the pacing test to prove the first publish occurs before the configured interval.

### Task 5: Guarantee fresh replay envelope IDs

**Files:**
- Modify: `src/event/replay.rs`

- [x] Add a test that runs force replay twice over the same file and asserts the two emitted IDs differ.
- [x] Run it red against `replay-{original_id}-1` reuse.
- [x] Generate IDs from original ID, process ID, and a process-wide `AtomicU64` sequence (no fallible clock dependency).
- [x] Run the unique-ID test and all replay tests.

### Task 6: Validate gates and review

**Files:**
- Verify: all files above and `docs/business_rules.md`

- [x] Run `cargo fmt --all -- --check` (blocked by repository-wide pre-existing formatting drift).
- [x] Run strict clippy (blocked by 290 pre-existing diagnostics in the latest run; no diagnostic points to the changed event files).
- [x] Run `cargo test --all-targets --all-features` (blocked by the pre-existing `v14_e2e` two-argument dispatcher call); `cargo test --lib` passes 1254/1254 with 7 ignored.
- [x] Run `bash tools/compliance/check.sh` (passes after the mandatory daily-data backfill).
- [x] Generate coverage: changed event files are 88.80%–98.00%; repository total is 51.14%, below Gate D's 80% requirement.
- [x] Review the final diff against all seven findings and BR-043; preserve unrelated dirty files.
- [x] Commit only remediation files with PR evidence fields recorded in the commit and handoff.

### Task 7: Close final two-axis review findings

**Files:**
- Modify: `src/event/replay.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: design/evidence documents

- [x] Remove silent clock fallback from replay ID generation.
- [x] Make `--limit=0` print every returned history entry and cover the formatter with the 101-row test.
- [x] Add injected production-publisher tests for real-sink acceptance, dry-run rejection, marker validation, sink failure, and audit failure.
- [x] Add a file-audit integration test covering trace fields and the previous-hash chain.
- [x] Reject malformed, missing-hash, hash-mismatched, or broken existing audit chains and cover corrupt-tail rejection.
- [x] Rerun Standards/Spec review after the final audit changes (both PASS).
