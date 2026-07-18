# Task Plan: Event Replay Safety Remediation

## Goal
Fix the reviewed v17.3 replay defects, then continue through the repository's remaining documented/code/test debt until historical claims are reconciled and all achievable gates pass with explicit evidence.

## Current Phase
Phase 12 (Gate D coverage and live-account release closure)

## Phases

### Phase 1: Gate A design and rule registration
- [x] Read mandatory repository rules and relevant specs/code
- [x] Register BR-043 before changing limit/rate/filter logic
- [x] Write remediation design and implementation plan
- **Status:** complete

### Phase 2: CLI TDD slices
- [x] RED/GREEN monitor flag composition
- [x] RED/GREEN replay-rate equals syntax
- [x] RED/GREEN explicit unbounded limit zero
- **Status:** complete

### Phase 3: Replay safety TDD slices
- [x] RED/GREEN invalid replay body rejection
- [x] RED/GREEN truthful publish outcomes
- [x] RED/GREEN rate throttling
- [x] RED/GREEN process-unique replay IDs
- **Status:** complete

### Phase 4: Monitor integration
- [x] Export summary and make CLI errors/failures nonzero
- [x] Verify production monitor call path
- [x] Propagate explicit unbounded zero into `HistoryQuery`
- **Status:** complete

### Phase 5: Gates and review
- [x] Targeted/event tests
- [x] fmt, clippy, full tests, compliance executed with blockers recorded
- [x] Final two-axis review
- [x] Commit scoped files
- **Status:** remediation complete; repository-wide Gate D remains blocked by pre-existing failures

### Phase 6: Full historical baseline and traceability map
- [x] Inventory active specs/plans and identify completion/bug claims requiring evidence
- [ ] Map each active completion/bug claim to code and executable evidence
- [ ] Reproduce every current fmt/clippy/build/test/coverage/compliance blocker
- [x] Classify discovered findings by data/fund risk and separate stale docs from code defects
- [x] Publish Gate A design and sequenced repair batches
- **Status:** in progress

### Phase 7: Restore compilation and regression gates
- [x] Fix all-target compilation failures one root cause at a time
- [x] Add or update behavior-focused regression tests before each semantic fix
- [x] Keep production monitor integration compiling after every slice
- **Status:** complete (`cargo test --all-targets --all-features` exit 0)

### Phase 8: Eliminate strict lint and formatting debt safely
- [x] Group Clippy diagnostics by root cause and module ownership
- [x] Fix correctness diagnostics before style diagnostics
- [x] Review and format the complete Rust tree required by the repository-wide fmt gate
- [x] Reach strict Clippy and fmt check PASS; re-run after later semantic batches
- **Status:** complete; verification is repeated during Phase 9

### Phase 9: Coverage and historical-spec closure
- [x] Measure per-module and repository coverage after stable tests
- [ ] Add tests for uncovered critical trading/data paths before low-risk utilities
- [ ] Correct stale completion claims and attach exact commands/output
- [x] Reach Gate D thresholds or report the exact remaining testable gaps
- [x] Replace fixed strategy scores, fake multi-source candidates, zero-value real-time fields, and placeholder win-rate sample generation with explicit real-data contracts
- [x] Remove audited production placeholders/silent fallbacks from active paths or disable incomplete producers fail-closed
- **Status:** implementation complete; Gate D remains blocked by measured coverage and unavailable real-account evidence

### Phase 10: Final integration, compliance, review, and commit
- [x] Build release monitor and run a safe production-path smoke test
- [x] Run full compliance and freshness gates
- [x] Run parallel Standards and Spec review against the fixed point
- [x] Fix first-round review findings and rerun Gate B/C
- [x] Complete final independent Standards/Spec/Audit re-review
- [x] Commit only scoped files/evidence and publish draft PR #2
- **Status:** delivery complete; Gate B/C and independent reviews pass, Gate D coverage and real-account evidence remain blocked

### Phase 11: Commit all remaining changes and merge evaluation
- [x] Audit every remaining worktree change and its impact on live-data/fund-safety paths
- [x] Restore Gate B/C validation after including the remaining changes
- [x] Commit and push all authorized worktree changes to PR #2
- [x] Resolve fixed-SHA review findings and obtain a clean independent re-review
- [x] Re-run Gate D evidence checks; keep merge blocked unless every mandatory item passes
- **Status:** complete; clean fixed-SHA follow-up review recorded, merge remains blocked by Gate D

### Phase 12: Gate D coverage and live-account release closure
- [x] Write and commit the coverage-closure design addendum and executable implementation plan
- [ ] Raise registered core trading/data line coverage from 55.45% to at least 95%
- [ ] Raise repository-wide line coverage from 51.20% to at least 80%
- [ ] Add a nullable, source-traceable same-day real-account P&L/NAV persistence path without fabricating unavailable values
- [ ] Re-run Gate A-D validation, live-data validation, and independent auditor review
- [ ] Mark PR #2 Ready and merge through GitHub only after every mandatory checkbox passes
- **Status:** in progress; user authorized autonomous continuation until all merge gates pass

## Decisions

| Decision | Rationale |
|---|---|
| Use `ReplaySummary` rather than `Result<usize>` | Separates attempted, published, skipped, and failed outcomes without aborting valid rows. |
| Preserve existing event modules | The bugs are contract violations, not evidence that another event architecture is needed. |
| Skip user confirmation gates | The user explicitly authorized completing all fixes without confirmation. |
| Include the four formerly out-of-scope worktree changes | The user explicitly directed that all code be committed and merged on 2026-07-18; each change still requires safety review and gate validation. |
| Close coverage core-first, then global | Gate D requires both thresholds; core paths carry the highest data/fund risk and must reach 95% before low-risk global modules. |

## Constraints

- The existing `.gitignore`, `.superpowers/sdd/progress.md`, `src/app/context.rs`, and `src/broker/ib.rs` changes are now authorized for review and commit; do not alter their intent without evidence.
- Do not run a real force replay or real push sink.
- Project remains In Progress if global gates fail for unrelated pre-existing problems.

## Errors Encountered

| Error | Attempt | Resolution |
|---|---:|---|
| User-level skills were first searched under repo `.agents/skills` | 1 | Read them from `/Users/zhangzhen/.agents/skills`. |
| `.github/copilot-instructions.md` is missing | 1 | Record Gate A evidence gap; continue under AGENTS and CLAUDE precedence. |
| Replay tests shared one process-scoped temp path and raced in parallel | 1 | Add an atomic per-test directory suffix; replay module tests then pass together. |
| History tests shared temp paths and used a future-at-some-hours fixed timestamp | 1 | Add atomic test-directory suffixes and use `Local::now()` for the 24-hour window test. |
| Global fmt/clippy/all-target gates fail outside the changed scope | 1 | Preserve evidence; do not broaden the remediation into thousands of unrelated edits. |
| Coverage tooling lacked `llvm-tools-preview` and sandboxed provider tests failed | 2 | Install the official component, rerun outside the sandbox, and capture the successful report. |
| Repository rustfmt rewrote unrelated whitespace in the large monitor file | 1 | Reverted the formatting-only rewrite, reapplied only scoped semantic edits, and reran targeted plus integrated tests. |
| Full regression returned `Unknown` for Cold/Fade because corrupt unrelated sector history was read first | 1 | Register BR-117 and evaluate complete single-day evidence before loading history required only by Start/Ferment/cumulative Climax. |
| Full `cargo test` exposed a race between the two global news-sink sender tests | 1 | Serialize install/receiver lifetime with a test-only mutex; 10 consecutive parallel focused runs and the full suite pass. |
| `.planning/.active_plan` is absent | 1 | Continue explicitly with `.planning/2026-07-16-event-replay-safety-remediation/`, the plan referenced by the current task handoff. |
| Initial multi-file plan update used stale tail context | 1 | Re-read exact file tails and apply smaller targeted patches. |
| Module inventory command assumed `src/broker/mod.rs` exists | 1 | Treat `src/broker.rs` as the likely crate module and inspect exact `rg --files` output before deciding deletion impact. |
| Removing the broad planning/workspace ignores exposed many old generated review artifacts | 1 | Restore the user's ignore rules; existing tracked evidence files still remain tracked, while unrelated generated workspaces stay out of the PR. |
| Isolated release smoke emitted repeated `database is locked` errors during first-time database initialization | 1 | Enter systematic root-cause investigation before treating the smoke as clean evidence or changing code. |
| First WAL-bootstrap fix reduced fresh-DB lock errors from nine to one but did not make the RED test green | 1 | Do not stack another guess; return to root-cause evidence gathering and label each remaining customizer failure stage. |
| PR-body shell argument interpreted Markdown backticks as command substitutions | 1 | Interrupted the process before later substitutions, audited HEAD/index/worktree/PR, confirmed no history or PR-body change, and switch to an apply-patch-created temporary body file for `gh pr edit --body-file`. |
| Fixed-SHA review found WAL return-mode/customizer retry ambiguity and inherited webhook configuration in process tests | 1 | Add public process RED cases, make bootstrap and pooled customization fail closed, remove outbound webhook configuration from child environments, then re-review the follow-up SHA. |
| First Phase-12 multi-file planning patch used a findings line that existed only in progress | 1 | Re-read the exact tails, split the patch around stable anchors, and record the Gate D continuation successfully. |
| First Gate-D design/plan patch expected a blank line absent from the historical design | 1 | Inspect the exact rollback/addendum boundary and apply the design and new plan as separate patches. |
| Task-1 format check found only rustfmt layout drift in new tests | 1 | Run repository formatter once, then rerun the check; the pre-existing tree was already format-clean. |
## Follow-up review slice (2026-07-18)

- [x] Add RED test for `:memory:` journal-mode failure.
- [x] Implement strict WAL result validation and nonzero DB-init exit.
- [x] Narrow ignores so process evidence is not hidden.
- [ ] Re-run full gates and obtain fixed-SHA independent review.

## Final fixed-SHA review closure (2026-07-18)

- [x] Reproduce and fix DB parent creation success-exit bug.
- [x] Assert the expected BR-108 ledger boundary for fresh DB startup.
- [x] Remove r2d2 customizer retry-to-health from connection PRAGMAs.
- [x] Add mandatory Gate-0 Copilot instructions.
- [ ] Run full Gate B/C/D evidence and request final fixed-SHA review.
- [x] Run full Gate B/C and regenerate Gate D coverage evidence.
- [ ] Commit/push and request final fixed-SHA review.
- [x] Commit/push and request final fixed-SHA review.
- [x] Close final review Important findings and rerun Gate B/C.
- [x] Push final follow-up and keep merge blocked on Gate D coverage.
