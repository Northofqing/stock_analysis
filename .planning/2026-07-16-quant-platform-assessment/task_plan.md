# Task Plan: Quant Platform Assessment and 18.x Design

## Goal
Assess the current A-share live-trading monitor from quantitative engineering, data governance, product-closure, and institutional-practice perspectives, then publish an actionable, safety-first 18.x design in `docs/18.x/`.

## Current Phase
Phase 4

## Phases

### Phase 1: Repository and Documentation Discovery
- [x] Map code, data, trading, risk, observability, and documentation surfaces.
- [x] Record the current production path and explicit evidence.
- [x] Record repository constraints and pre-flight gaps.
- **Status:** complete

### Phase 2: Quantitative Engineering and Data Assessment
- [x] Assess data lineage, validity, freshness, corporate-action handling, replay, and research-to-live consistency.
- [x] Assess order lifecycle, risk controls, portfolio construction, and auditability.
- **Status:** complete

### Phase 3: Product Closure and Institutional Benchmark
- [x] Assess whether the product closes the loop from research to review and governance.
- [x] Benchmark capabilities against public, non-proprietary institutional practices.
- [x] Prioritize gaps by fund/data safety and expected value.
- **Status:** complete

### Phase 4: Design Proposal and Review Gate
- [x] Present design alternatives and request user approval, per brainstorming workflow.
- [x] Publish the approved 18.x design, including data flow, failure modes, old-module relations, rollout, rollback, and verification.
- [x] Self-review for ambiguity and contradictions.
- [x] Add implementation-level interfaces, state machines, persistence seams, audit durability, and test seams for the four core modules; register BR-038 ~ BR-042.
- **Status:** complete

### Phase 5: Verification and Delivery
- [x] Check documentation consistency and whitespace.
- [x] Record scope-limited validation results and PR evidence template.
- [x] Re-run compliance and staged-diff checks after the detailed design and registry repair.
- [x] Deliver paths, key findings, and next decisions.
- **Status:** blocked — repository baseline fails mandatory format, lint, and full-test gates outside this documentation-only change.

## Key Questions
1. What parts of the claimed live-trading path are actually integrated into `src/bin/monitor/`?
2. Does the current system preserve correct, fresh, real data from ingest through decision, execution, and review?
3. Which missing capabilities prevent a safe and useful research-to-live product loop?
4. Which institutional controls are feasible and most urgent for an A-share live monitor?

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Assess before proposing | Prevents recommendations that ignore existing modules or hard data-safety rules. |
| Use an 18.x document set | Matches user-requested location and separates assessment evidence from executable design. |
| Treat external benchmarks as public-practice references | Avoids claiming access to proprietary institutional strategies. |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| `.github/copilot-instructions.md` is absent | 1 | Record it as a pre-flight documentation gap; continue with higher-priority repository rules. |
| `cargo fmt --check` fails on baseline source files | 1 | Verified no source files changed in this task; record as a repository Gate blocker, do not bulk-format unrelated code. |
| New v18 files were ignored by `/docs` | 1 | Force-staged only `docs/README.md` and `docs/v18.x/` to comply with Git-tracking rule. |
| Initial staged diff had five Markdown trailing spaces | 1 | Removed them and re-ran `git diff --cached --check` successfully. |
| `cargo clippy -- -D warnings` fails on existing source lint errors | 1 | Recorded root cause and scope; no unrelated code cleanup attempted. |
| Full `cargo test` fails to compile `v14_e2e` | 1 | Root cause identified: `src/bin/v14_e2e.rs:285` supplies two arguments to a three-argument `Dispatcher::dispatch`; no change made because it predates this documentation-only task. |
