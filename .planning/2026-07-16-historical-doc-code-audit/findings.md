# Findings: Historical documentation-to-code audit

## Requirements
- Audit all historical documentation descriptions of features and bugs.
- Decide whether each is completely resolved at code level.
- Require production integration, tests/failure paths, and compliance evidence; source text or passing unit tests alone are insufficient.

## Initial Findings
- Repository root commit: `3c7fad274a462a972bc5f6ef183d2119ef30a708`.
- Audit HEAD at start: `0d85dc5c74eb3655c825a35eaaa91825b6ca4725` (2026-07-16 19:45:38 +08:00).
- Worktree was already dirty before the audit. These changes are user-owned and cannot be treated as committed completion evidence.
- Required `.github/copilot-instructions.md` is absent.
- Review skill's expected `docs/agents/issue-tracker.md` is absent, so external issues cannot yet be claimed as exhaustively covered.
- `CLAUDE.md` explicitly records a v17.x post-mortem: v17.4-v17.8 specs contained unverified/false completion claims, including incomplete enum deletions and nonexistent APIs. This is direct prior evidence that documentation claims cannot be accepted without code tracing.
- The repository has 549 commits at the pinned HEAD and 173 current files under `docs/`; the docs tree contains about 70,327 lines by `wc -l` (including a 7,344-line PDF count artifact), so the audit must be ledger-driven rather than a manual spot check.
- Documentation is versioned across pre-v9 archive and v9.x through v18.x. High-volume areas include v9.x (31 files), v13 (20), v14.x (19), v11 (15), v16.x (14), and v17.x (11).
- Git history contains many deleted/relocated evidence sources, including the v9 known-bugs list, prior audits, root-cause reports, architecture/acceptance records, release notes, and implementation specs. Current-file-only review would therefore be incomplete.
- Commit messages themselves contain numerous numbered review findings and explicit deferred items. They must be treated as an additional claim source, but a missing external issue tracker prevents resolving every `Task #N` reference to its full body.
- The first historical-path count command undercounted because its Git pathspec did not cover the current docs tree reliably; this result is rejected and will be replaced with a tree/history-object inventory.
- A second inventory exposed a more important discrepancy: the filesystem contains 173 files under `docs/`, while `git ls-tree -r HEAD docs/` reports only 26 tracked paths at pinned HEAD. This may mean most docs are ignored/untracked or materialized outside HEAD; their provenance must be checked before treating them as Git-tracked process evidence.
- `docs/README.md` claims 168 documentation files and says all process outputs are organized by BR-029, which conflicts with the 26-path HEAD tree count until tracking state is explained.
- The current `docs/v17.x/v17.x-dev-plan-revised.md` is explicitly `待审批` and records 4 deferred review findings: real Magiclaw health check, stronger L6 sink assertion, Frozen warning noise, repeated env syscall, and stable template ID mapping (the table labels F4/F6/F8/F9/F10 as deferred; five rows despite the document's text saying four). It also leaves v17.5-v17.8 advancement unchecked.
- The current `docs/v17.x/v17.8-final-batch-cleanup.md` explicitly retracts its “100%/收官” claim: enum deletion target was not met, earlier 0-caller evidence was false, and status is `In Progress`. This alone disproves a universal “all documented features/bugs completely resolved” conclusion at the audit snapshot.
- v17.8 later audit found 4 of 6 allegedly dead push variants were actually on live call paths. This confirms that absence claims based on earlier grep evidence were materially unsafe.
- Tracking provenance is now confirmed: `.gitignore:11` ignores `/docs`. Of 173 non-`.DS_Store` files currently under `docs/`, only 26 are tracked at HEAD and 147 are ignored workspace-only files (the `git status --ignored` count is 148 including `.DS_Store`). `docs/ENGINEERING_RULES_V2.md` is workspace-only. Selected docs were force-added, but most historical design/bug/completion records are not Git/PR evidence.
- `tools/` is also globally ignored, but 11 compliance files were force-tracked; 36 tool files are visible in the workspace. The mandatory core compliance scripts are tracked at HEAD. `.github/copilot-instructions.md` is absent even though three other `.github` files are tracked.
- Automated open-claim scanning found 1,918 lines matching unresolved/deferred/checklist vocabulary. This is a discovery index, not an issue count, because plan checkboxes and explicit non-goals also match.
- Tracked `docs/v15.x/v15.3-phase-d-expansion.md` contains 60 unchecked implementation tasks and its status table says Phase D expansion is `⏸ 待做`, 0 commits. Some planned news modules now exist, but the document itself still lacks acceptance closure and requires claim-by-claim mapping.
- Tracked `docs/v18.x/v18.0-2026-07-16-review-quant-platform-assessment.md` explicitly concludes the project is not yet an auditable research-to-live system and lists four P0/P1 functional gaps: failures coerced to defaults, synthetic Full data health, 120s freshness versus 5s rule, and no proven broker order/fill/reconciliation lifecycle.
- HEAD code corroborates the v18 broker gap: `src/bin/monitor/health.rs` says broker SDK is unintegrated and mock-returns true; `main.rs` registers a default `MockQuoteProvider` with 0.0 fallback; `src/broker/ib.rs` documents zero network calls and returns 0.0; paper trading falls back to avg cost/zero and skips slippage validation. These are production-path red-line candidates under 2.1/2.3/2.6/2.8, not merely missing future live-trading scope.
- HEAD has multiple production `DataMode::Full` constructors and a code path around `main.rs:3253-3267` that synthesizes capabilities with age 200 before yielding Full, matching the v18 P0 finding that health can be asserted rather than observed.
- The compliance gate itself cannot prove the red lines it claims:
  - `check_data_freshness.sh:21-30` exits success when the DB or `sqlite3` is absent, contradicting the strict merge-blocker language of 2.4.1; it also compares calendar days, not trading days/holidays.
  - `check_fake_impl.sh:26-45` detects only one `update_*result*0.0*false` regex and existence of a single test file. It does not enforce the full verify/save/notify/push/sync/update_result/reconcile semantics in 2.8 and misses the production mock/0.0 paths already found.
  - `check_design_contradiction.sh:27-32` returns success when the one configured threshold is missing and checks only `event_risk_score_threshold`; it does not validate all `config/*.toml` threshold/spec cross-references required by 2.9.
- The test corpus itself contains an explicit unresolved rule: `tests/ranking.rs:140` says BR-005 daily push ≤5 is currently not implemented. The business-rule registry also contains PENDING rules such as BR-015 (chain concentration disabled with hardcoded zeros, no test) and BR-026/027/029 marked `待实现`; these require current-code verification rather than assuming later completion.

## Classification Model
| Status | Meaning |
|--------|---------|
| Verified complete | Code + production call path + relevant passing tests/failure paths + required compliance evidence all found. |
| Partial | Some implementation exists, but one or more required layers are missing. |
| Unresolved | Requested behavior/fix is absent or current behavior still contradicts it. |
| Contradicted | A completion claim is disproved by code/history/newer authoritative documentation. |
| Unverifiable | Evidence source or safe validation is unavailable; must not be counted as complete. |
| Superseded | Newer explicit decision replaces the claim; provenance retained. |

## Evidence Rules
- Current uncommitted changes are labelled separately from HEAD evidence.
- Single-line grep is not accepted for caller absence; call chains must be traced multiline-aware back to production entry points.
- No live execution that may place orders or send notifications without explicit safety proof/authorization.

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Mandatory pre-flight file missing | Continue read-only audit and report the gap as blocking a universal-completeness claim. |
| External issue tracker not configured | Scope the guaranteed audit to Git-resident evidence and label external issue exhaustiveness unverifiable. |
| Initial Git documentation pathspec undercounted historical paths | Reject count; inventory from `git ls-tree` plus name-status across all commits instead. |
| Filesystem docs count and HEAD tracked docs count conflict | Inspect ignore/tracking/branch state before assigning provenance. |
| 1,918 open-vocabulary matches are not normalized requirements | Use them as extraction candidates; deduplicate and classify before reporting counts. |

## Resources
- `AGENTS.md`
- `docs/ENGINEERING_RULES_V2.md`
- `CLAUDE.md`
- Git history from root commit through audit HEAD
- Consolidated report: `.planning/2026-07-16-historical-doc-code-audit/audit_report.md`

## Independent Review Summary
- Standards: 9 hard findings + 1 smell; worst is fake account/cash/price in production risk paths.
- Early Spec (v9-v14): 9 findings; worst is missing tamper-resistant five-year audit trail.
- Late Spec (v15-v18/superpowers): 8 findings; worst is defaulted decision data and fake paper-risk state.
- Deduplicated consolidated matrix: 9 Standards rows + 15 Spec rows.

## Validation Results
- Clean pinned-HEAD `cargo fmt --check`: FAIL.
- Clean pinned-HEAD `cargo clippy -- -D warnings`: FAIL with 285 errors.
- Clean pinned-HEAD `cargo test`: FAIL before test execution because `src/bin/v14_e2e.rs:285` calls `Dispatcher::dispatch` with two arguments instead of three.
- Worktree compliance entrypoint: exit 0, but semantic evidence includes 16 pending BRs, 21 warnings, 155 global zero fallbacks, and missing-template SKIP/PASS.
- No live monitor execution was attempted due notification/order safety risk.
