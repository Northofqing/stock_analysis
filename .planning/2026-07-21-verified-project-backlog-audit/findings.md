# Findings & Decisions

## Requirements
- Produce one audit document under `docs/audits/` plus isolated planning memory required by the planning skill.
- Map items 1–48 to only Confirmed/Partial/Fixed/False/Duplicate.
- Include concise code/doc/log evidence paths; no account-specific values, security details, or screenshots.
- Deduplicate remaining work into P0/P1/P2.
- BR-138 and BR-139 are fixed and must not appear as unfinished work.

## Research Findings
- Corrected authoritative rollup from parent: 23 Confirmed, 13 Partial, 12 Fixed/False; the earlier 24/13/11 was an arithmetic error.
- Item 41 remains `Confirmed` in the exhaustive appendix and is annotated as duplicate of item 35; the deduplicated priority view lists the work once.
- Fixed: 5, 7, 46, plus BR-138/BR-139 outside the numbered list.
- False/Insufficient: 8, 16, 33, 34, 37, 42, 43, 44, 47.
- Duplicate relations: 35/41 duplicate; 27/33 near-duplicate; 20/21 related but distinct.
- New unresolved log-backed P0s: post-session upstream failures (Eastmoney K-line and R-08 announcement), exceptional price-change validation requiring listing/board rules, A-01 single-row failure aborting its batch, R-02 missing index fields, R-03 missing chain evidence, A-10 missing name evidence.
- BR-138/BR-139 code/spec review approved; production Gate D markers/evidence were still pending in the prior review.
- Operational items A1–A7/A18 have a consolidated evidence source in `docs/v19.x/v19.0-operational-clarity-design.md`; it explicitly distinguishes measured pain from unimplemented v19 proposals.
- v18 contract items D24–D27 and E31–E35 are explicitly design-only in `docs/v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md`, `docs/v18.x/v18.0-2026-07-16-codebase-design-four-core-modules.md`, and `.planning/2026-07-18-v18-ws0-test-inventory/findings.md`.
- Concrete current-code evidence includes `src/opportunity/news_outcome.rs` (MAE/MFE absent→0), `src/monitor/signal_fusion.rs` (default zero weights), `src/opportunity/news_ranker.rs` (missing limit-up count→0), and `src/bin/monitor/metrics.rs` (six metrics).
- `docs/v16.x/v16.x-completion-audit-2026-07-19.md` is authoritative evidence for the 1,748-test full-suite count and fixed paper/live isolation work; it also states privacy-safe evidence practices.
- Additional current-code evidence: `src/pipeline/analyze.rs` invokes `track_position` then `save_analysis_result` separately; `src/trading/mod.rs::SimulatedExecutionGateway` depends on global `DatabaseManager::get`; `src/trading/paper_engine.rs` contains hard-coded iron-rule text/threshold interpretation; `src/review/failure_attribution.rs` exists but does not establish a fully wired loss-attribution loop.
- PR evidence is partially enforced: `.github/pull_request_template.md` and `tools/compliance/lib/check_pr_evidence.sh` exist, and `.github/workflows/compliance.yml` invokes the latter on PR events; value quality/completeness beyond regex remains a review concern.
- The completed audit is `docs/audits/2026-07-21-verified-project-backlog.md`. The repository's `/docs` ignore rule hides this new file from normal `git status`; the parent/release owner must explicitly force-add it when preparing the required PR. It was intentionally not staged or committed in this subtask.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Do not invent a 24th Confirmed item | Parent confirmed 23/13/12 as the corrected exhaustive rollup; disclose the corrected arithmetic in the audit header. |
| Evidence uses paths and aggregate marker names only | Avoids leaking real account values or notification content. |
| Keep all seven newly log-confirmed failures in P0 | They are unresolved production data-source, bad-data, batch-isolation, or evidence-integrity failures and therefore engage AGENTS 2.1–2.4/2.7. |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Requested rollup conflicted with enumerated statuses | Parent resolved it: use exhaustive 23/13/12 and deduplicate item 41 only in the backlog view. |

## Resources
- `AGENTS.md`, `docs/ENGINEERING_RULES_V2.md`, `.github/copilot-instructions.md`, `CLAUDE.md`
- `docs/business_rules.md`
- `docs/superpowers/specs/2026-07-21-announcement-relevance-design.md`
- `docs/superpowers/specs/2026-07-21-post-session-review-scheduler-design.md`
- `.planning/2026-07-20-monitor-48h/{findings,progress}.md`
