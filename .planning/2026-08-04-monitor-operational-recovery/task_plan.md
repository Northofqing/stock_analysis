# Task Plan: Monitor Operational Recovery

## Goal

Restore evidence-backed `monitor --review` and the complete isolated `monitor --test`
catalog, while preserving data red lines and test/live notification isolation.

## Current Phase

Phase 10 — Monitor CLI recovery verified; retain external release blockers explicitly.

## Phases

### Phase 1: Reproduce and classify
- [x] Capture `monitor --review` runtime outcome.
- [x] Capture complete 48-template isolated dry-run outcome.
- [x] Capture bare `monitor --test` live-isolation refusal.
- [x] Capture focused BR-171 regression failure.
- **Status:** complete

### Phase 2: Focused Gate B repair
- [x] Repair the deterministic BR-171 output/conflict regressions.
- [x] Run focused confirmation and admitted-persistence suites.
- [x] Re-run both monitor command paths and capture exact exit evidence.
- **Status:** complete

### Phase 3: Gate C evidence
- [x] `cargo fmt --all -- --check`.
- [x] Strict workspace Clippy (`--all-targets --all-features -D warnings`).
- [x] Full compliance, including business rules and data freshness.
- [x] Re-run the full workspace test command after repairing stale monitor and integration assertions.
- [x] Build the release `monitor` binary.
- [ ] Satisfy the mandatory coverage thresholds (global >=80%, core >=95%).
- **Status:** blocked at Gate D coverage only

### Phase 4: Handoff
- [x] Confirm README already documents BR-196 dry-run/live-target behavior.
- [x] Record exact operational results and remaining blockers.
- **Status:** complete

### Phase 5: Parallel closure
- [x] Add a high-value coverage slice and remeasure global/core coverage.
- [x] Diagnose and, where real account evidence permits, recover R-03 review output.
- [x] Audit BR-196 bare `--test` target discovery/loading without weakening test/live isolation.
- [x] Re-run both requested CLI paths and all mandatory gates after integration.
- **Status:** operational validation complete; release remains blocked by Gate D coverage and
  bare `--test` remains externally blocked by the missing independent non-production Feishu target.

### Phase 6: Quiet-hour operational recovery
- [x] Register BR-207 and document the failure/retry semantics before implementation.
- [x] Preserve `quiet_hour` as a retryable A-10 outcome instead of consuming it as a terminal failure.
- [x] Run the focused regression, formatting, strict Clippy, full workspace tests and compliance.
- [x] Re-run real `monitor --review` and the complete isolated `monitor --test --push-dry-run` path.
- **Status:** operationally complete; Gate D and the independent test Feishu target remain external release blockers.

### Phase 7: Provider-free defer and capability closure
- [x] Prove the A-10 provider-before-quiet-hour call order from current code and durable audit.
- [x] Register an absolute provider-free defer contract with truthful manual-reinvoke semantics.
- [x] Implement the smallest TDD slice without weakening L5 quiet-hour governance.
- [x] Independently re-audit the current R-08 Magic CFFEX production capability.
- [x] Re-audit current Gate D arithmetic and select a non-overlapping next coverage slice.
- [x] Re-run the three operational entry points against the current worktree.
- **Status:** operational paths recovered. BR-209 passed focused independent review and Gate C.
  Bare live `--test`, R-08, R-03 and Gate D retain their explicit external/design blockers.

### Phase 8: Parallel executable closure
- [x] Reproduce the exact bare `cargo run --bin monitor -- --test` boundary and
  determine whether the accepted BR-196 contract permits a zero-transport successful default
  without weakening the separately configured non-production live-acceptance path.
- [x] Independently close or precisely re-prove the R-03 real-account evidence gate.
- [x] Independently close or precisely re-prove the R-08 formal Magic CFFEX capability.
- [x] Integrate only Gate-A-authorized fixes and run focused RED/GREEN tests; no production edit
  was authorized by the accepted designs in this phase, so no behavior was bypassed.
- [x] Re-run exact normal, review and test entry points plus final consistency checks.
- **Status:** operational entry points are green where the accepted contracts permit execution;
  the exact bare live-acceptance command, R-03 and R-08 retain externally/sequentially proven
  fail-closed blockers.

### Phase 9: Release-authority and R-03 prerequisite closure
- [in_progress] Repair or precisely bound the authoritative Gate-D coverage generator/checker;
  stale or caller-weakened reports must not count as release evidence.
- [in_progress] Determine BR-203 P4's exact current DoD and complete only already-authorized
  artifacts needed before BR-204/R-03.
- [in_progress] Independently audit the three monitor entry points for any additional internally
  fixable blocker that would prevent the next production session.
- [pending] Integrate non-overlapping accepted changes and rerun focused RED/GREEN evidence.
- [pending] Rerun Gate C, authoritative coverage, release build and operational commands.
- **Status:** in progress; external BR-196 target and formal upstream R-08 capability remain
  explicitly out of scope for local fabrication.

### Phase 10: Current monitor operational verification
- [x] Separate BR-138 lifecycle, classification and audience dispositions and validate every
  provider announcement before any handled filter.
- [x] Run focused BR-137/BR-138 tests, formatting and strict workspace Clippy.
- [x] Run the repository-mandated serial all-workspace/all-target/all-feature test command.
- [x] Run the full compliance suite and optimized monitor build.
- [x] Run real `monitor --review`, isolated complete `monitor --test --push-dry-run`, and a
  bounded normal monitor startup/shutdown canary.
- [ ] Provision and release-pin an independent non-production Feishu target before bare live
  `monitor --test` may send externally (Rule 2.5; external prerequisite).
- **Status:** monitor/review and the complete isolated template catalog are operational. Bare
  live test delivery remains intentionally fail-closed until the independent target exists;
  Gate-D coverage, R-03 account evidence and formal R-08 CFFEX capability remain separate
  release/capability work rather than monitor process failures.

## Errors Encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| Cargo test waited on artifact lock | 1 | Existing process completed; reuse its exact result instead of launching a duplicate. |
| `review_output_is_machine_readable_and_contains_all_exact_evidence` expected TEST_CODE but fixture emitted `600396` | 1 | Active focused repair; production path remains untouched. |
| Prefixing BR-171 query facts with TEST_CODE made the lower canonical evidence hasher reject them | 2 | Reverted the semantic change; the pure zero-I/O test keeps the provider-shaped code and fixes only the stale assertion. |
| BR-171 library suite passed 6/7; conflicting second decision did not return `Conflict` | 1 | Reproduced deterministically; inspect the v1/v2 idempotency boundary before patching. |
| Bare `monitor --test` exits 2 | 1 | Classified as BR-196 fail-closed: no independent non-production Feishu target is configured. |
| Full workspace tests failed in 3/568 monitor tests | 1 | Two tests contradicted their own non-quiet test guard; one R-08 source inspection expected the pre-presentation-wrapper function name. Test contracts were updated without production behavior changes; `cargo test --bin monitor` now passes 564/564 with 4 helpers ignored. |
| Full workspace rerun found 2 stale BR-192 source-shape integration assertions | 1 | Updated the test-only source inspection to the current strict review and E2E entry points; focused test passes 2/2. |
| Coverage report output directory was absent | 1 | Created `target/coverage` and generated the report from the already completed profraw data without rerunning tests. |
| Gate D coverage thresholds fail | 1 | Exact evidence: global 78.66% vs 80%; core 78.17% vs 95%. This is the remaining repository-wide release blocker, not an operational monitor failure. |
| `bash tools/docs/check_links.sh` missing | 1 | Repository has no docs-link helper at that path; validate README changes through formatting/manual inspection and the mandatory compliance suite instead of repeating the command. |
| Gate D still fails after the isolated global-market coverage slice | 2 | The focused 10/10 tests pass and raise `global_market.rs` line coverage from 42.57% to 83.76%; repository totals are now global 78.77% and core 78.30%, still below 80%/95%. Keep Gate D blocked while proceeding with operational CLI validation. |
| BR-192 concurrent coordinator open/drop intermittently failed physical SQLite descriptor attestation | 1 | Root cause was the SQLite Unix VFS unused-fd pool reusing an already-open descriptor without a process-fd delta. BR-206 now serializes open/drop and performs a bounded, fail-closed retry while retaining unproved connections without issuing SQL/PRAGMA. The focused regression passed 100 consecutive process runs and the exact full workspace suite passed. |
| Exact bare `cargo run --bin monitor -- --test` exits 2 | 2 | Reconfirmed after all code gates: `.env` contains none of the four BR-196 live-acceptance fields and the release-pinned non-production allowlist is empty. This remains an external target-provisioning blocker; production target reuse is prohibited by rule 2.5. |
| A-10 obtains public provider batches before a predictable quiet-hour denial | 1 | Root-cause and restart-safe defer design are the active Phase 7 work; no semantic patch is allowed before exact current-state evidence and a registered rule. |
| First BR-209 draft used `ExpectedWait(06:00)` for a prior business date | 1 | Independent audit proved it would persist an already-past `next_attempt`. Removed the unaccepted draft/RED test; the future design must use an absolute wall-clock `DeferredUntil` and explicitly require manual reinvocation. |
| Bare `monitor --test` with live opt-in rejects the current Feishu target | 3 | Exact result is `production_feishu_target_rejected`; no external process was attempted. A separate non-production conversation identity and reviewed allowlist hash are required by rule 2.5. |
