# Task Plan: Event Replay Safety Remediation

## Goal
Fix all seven reviewed v17.3 CLI/replay defects with explicit failure handling, tests, integration, and gate evidence.

## Current Phase
Phase 5

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

## Decisions

| Decision | Rationale |
|---|---|
| Use `ReplaySummary` rather than `Result<usize>` | Separates attempted, published, skipped, and failed outcomes without aborting valid rows. |
| Preserve existing event modules | The bugs are contract violations, not evidence that another event architecture is needed. |
| Skip user confirmation gates | The user explicitly authorized completing all fixes without confirmation. |

## Constraints

- Do not modify existing dirty `.gitignore` or `.superpowers/sdd/progress.md`.
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
