# Late-document audit coverage

Fixed code snapshot: `c1e53321b2f4fb5d1f21cc0baf7ff4ade1ffcb7b`.

Scope is every manifest entry not physically under `docs/v9.x/` through `docs/v14.x/`. Workspace-only documents were read for requirements, but implementation evidence was taken only from the fixed commit. Historical-only text was recovered from its manifest `content_commit`. Uncommitted business-code changes were ignored.

Result: 129 late-scope documents reconciled from the expanded 243-entry manifest; 89 claim-bearing documents produced 181 normalized claims. Status totals: 19 `verified_complete`, 81 `partial`, 56 `unresolved`, 6 `contradicted`, 11 `unverifiable`, and 8 `superseded`. The other manifest paths are assigned to the early v9-v14/archive audit. A `verified_complete` diagnosis row means the diagnosis is confirmed, not that its underlying bug is fixed.

## Claim-bearing documents

| Manifest path | Claims |
| --- | ---: |
| `docs/KNOWN_BUGS-2026-06-28.md` | 1 |
| `docs/analysis-2026-06-29-project-audit.md` | 1 |
| `docs/architecture/v12-e2e-real-push-2026-07-05.md` | 1 |
| `docs/architecture/v12-monitor-test-2026-07-05.md` | 1 |
| `docs/architecture/v12-mvp-progress.md` | 1 |
| `docs/architecture/v12-mvp0-acceptance-2026-07-05.md` | 1 |
| `docs/architecture/v12-push-uncertainty-notes.md` | 1 |
| `docs/architecture_p0_risk_unification.md` | 1 |
| `docs/code_review_p0_risk_unification.md` | 1 |
| `docs/emquant-api-integration-plan-调研-2026-06-05.md` | 1 |
| `docs/operations/broker-api-integration.md` | 1 |
| `docs/p0-4-done.md` | 1 |
| `docs/p0-5-done.md` | 1 |
| `docs/p0-5-plus-done.md` | 1 |
| `docs/p0-5-plus-v2-done.md` | 1 |
| `docs/p0-5-plus-v3-done.md` | 1 |
| `docs/p0-5-plus-v4-todo.md` | 1 |
| `docs/project_plan_p0_risk_unification.md` | 1 |
| `docs/root-causes/E-multi-agent-bypass-freshness.md` | 1 |
| `docs/root-causes/F-tool-silent-fallback.md` | 1 |
| `docs/root-causes/G-no-fast-fail.md` | 1 |
| `docs/sina_baostock_integration.md` | 1 |
| `docs/superpowers/plans/2026-06-28-event-extractor.md` | 1 |
| `docs/superpowers/plans/2026-07-06-v13-push-templates-impl.md` | 1 |
| `docs/superpowers/plans/2026-07-07-v29-d01-dispatcher.md` | 1 |
| `docs/superpowers/plans/2026-07-08-qmt-integration.md` | 1 |
| `docs/superpowers/plans/2026-07-08-sina-baostock-integration.md` | 1 |
| `docs/superpowers/plans/2026-07-11-v13-push-templates-remediation.md` | 1 |
| `docs/superpowers/plans/2026-07-16-remove-openai-config-plan.md` | 1 |
| `docs/superpowers/plans/2026-07-16-v17-r2-a-event-seam.md` | 1 |
| `docs/superpowers/plans/2026-07-16-v17.3-persistence-query.md` | 1 |
| `docs/superpowers/plans/2026-07-16-v17.7-source-pushes.md` | 1 |
| `docs/superpowers/plans/compressed-wondering-parasol.md` | 1 |
| `docs/superpowers/plans/radiant-frolicking-starfish.md` | 1 |
| `docs/superpowers/plans/refactored-meandering-bird-agent-aa580bbef2a2358a9.md` | 1 |
| `docs/superpowers/plans/refactored-meandering-bird.md` | 1 |
| `docs/superpowers/plans/unified-cooking-tulip-agent-a893df278aa4ff06a.md` | 1 |
| `docs/superpowers/plans/unified-cooking-tulip.md` | 1 |
| `docs/superpowers/specs/2026-06-29-process-discipline-design.md` | 1 |
| `docs/superpowers/specs/2026-07-06-v13-push-templates-audit.md` | 1 |
| `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md` | 1 |
| `docs/superpowers/specs/2026-07-06-v13-push-templates-impl-log.md` | 1 |
| `docs/superpowers/specs/2026-07-06-v19-14-push-template-refactor-design.md` | 1 |
| `docs/superpowers/specs/2026-07-06-v19-15-push-simplify-design.md` | 1 |
| `docs/superpowers/specs/2026-07-06-v19-16-remove-demo-data-design.md` | 1 |
| `docs/superpowers/specs/2026-07-07-d01-dispatcher-design.md` | 1 |
| `docs/superpowers/specs/2026-07-08-qmt-integration-design.md` | 1 |
| `docs/superpowers/specs/2026-07-08-sina-baostock-integration-design.md` | 1 |
| `docs/superpowers/specs/2026-07-16-remove-openai-config-design.md` | 1 |
| `docs/superpowers/specs/2026-07-16-v17-event-infrastructure-and-data-sources-design.md` | 1 |
| `docs/v15.x/dead-pushkinds.md` | 3 |
| `docs/v15.x/news-consolidation-plan.md` | 1 |
| `docs/v15.x/post-mortem-v15.1.1.md` | 3 |
| `docs/v15.x/v15-plan-news-stock-analysis.md` | 3 |
| `docs/v15.x/v15.3-phase-d-expansion.md` | 9 |
| `docs/v16.x/v16.1-virtual-trading-redesign.md` | 5 |
| `docs/v16.x/v16.2-trading-kernel-redesign.md` | 2 |
| `docs/v16.x/v16.3-development-plan.md` | 1 |
| `docs/v16.x/v16.3-domain-model-event-driven.md` | 4 |
| `docs/v16.x/v16.4-design.md` | 3 |
| `docs/v16.x/v16.4-development-plan.md` | 1 |
| `docs/v16.x/v16.5-design.md` | 1 |
| `docs/v16.x/v16.5-development-plan.md` | 1 |
| `docs/v16.x/v16.6-design.md` | 2 |
| `docs/v16.x/v16.6-development-plan.md` | 1 |
| `docs/v16.x/v16.7-design.md` | 1 |
| `docs/v16.x/v16.7-development-plan.md` | 1 |
| `docs/v16.x/v16.8-design.md` | 1 |
| `docs/v16.x/v16.8-development-plan.md` | 1 |
| `docs/v17.x/v17.1-push-timing-redesign.md` | 1 |
| `docs/v17.x/v17.1-r2-event-infrastructure.md` | 29 |
| `docs/v17.x/v17.2-eventbus-broadcast-persistence.md` | 1 |
| `docs/v17.x/v17.3-migration-and-persistence.md` | 18 |
| `docs/v17.x/v17.4-news-and-review.md` | 5 |
| `docs/v17.x/v17.5-batch2-push-migration.md` | 4 |
| `docs/v17.x/v17.6-batch3-push-migration.md` | 3 |
| `docs/v17.x/v17.7-batch4-dead-code-cleanup.md` | 1 |
| `docs/v17.x/v17.8-final-batch-cleanup.md` | 1 |
| `docs/v17.x/v17.x-dev-plan-revised.md` | 1 |
| `docs/v17.x/v17.x-dev-plan.md` | 1 |
| `docs/v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md` | 6 |
| `docs/v18.x/v18.0-2026-07-16-codebase-design-four-core-modules.md` | 1 |
| `docs/v18.x/v18.0-2026-07-16-review-quant-platform-assessment.md` | 1 |
| `docs/v18.x/v18.0-2026-07-16-writing-plans-implementation-roadmap.md` | 6 |
| `.planning/2026-07-16-quant-platform-assessment/findings.md` | 1 |
| `.superpowers/sdd/final-review.md` | 2 |
| `CHANGELOG.md` | 2 |
| `IMPROVEMENTS_SUMMARY.md` | 2 |
| `reports/sentiment_score_IC_IR_analysis-2026-06-29.md` | 1 |

## Zero-claim documents

These were inspected/reconciled but intentionally yielded no independent executable claim. Their requirements, where any, are represented by a canonical later plan/spec row.

### Superseded pre-v9 archive and changelog (0 each)

- `docs/CHANGELOG-v9.4.md`
- `docs/_archive/pre-v9-history/README.md`
- `docs/_archive/pre-v9-history/architecture-overview-2026-06-16.md`
- `docs/_archive/pre-v9-history/architecture-v2-2026-06-15.md`
- `docs/_archive/pre-v9-history/architecture-v3-2026-06-15.md`
- `docs/_archive/pre-v9-history/architecture-v4-2026-06-16.md`
- `docs/_archive/pre-v9-history/architecture-v5-2026-06-16.md`
- `docs/_archive/pre-v9-history/architecture-v5.1-2026-06-16.md`
- `docs/_archive/pre-v9-history/architecture-v6-2026-06-16.md`
- `docs/_archive/pre-v9-history/architecture-v7-2026-06-16.md`
- `docs/_archive/pre-v9-history/plans-README.md`
- `docs/_archive/pre-v9-history/v3-project-plan-2026-06-15.md`
- `docs/_archive/pre-v9-history/v4-project-plan-2026-06-16.md`
- `docs/_archive/pre-v9-history/v5-project-plan-2026-06-16.md`
- `docs/_archive/pre-v9-history/v6-project-plan-2026-06-16.md`

### Rules, indexes and operational references (0 each)

- `docs/ENGINEERING_RULES_V2.md` — audit constraint, not a historical feature claim.
- `docs/README.md` — index.
- `docs/business_rules.md` — rule registry; individual implementation findings carry the relevant BR/red-line references.
- `docs/crontab.example` — deployment example, no standalone functional acceptance.
- `docs/review-output-diagnosis-codex.md` — review methodology/diagnostic output, no additional canonical requirement.
- `docs/root-causes/README.md` — index; E/F/G are claim-bearing.
- `docs/v11-grill-decisions.md` — decision transcript superseded by later v12+ designs.
- `docs/v12-template-verify-2026-07-05.md` — historical verification note covered by v12/v13 canonical rows.
- `docs/v18.x/README.md` — index.
- `docs/业务规则清单-registry.md` — compatibility/registry listing, not an independent implementation target.

### Release/deployment duplicates (0 each)

- `docs/operations/v13-push-templates-deployment.md` — operational instructions; feature claims are under v13 design/remediation.
- `docs/operations/v13-release-notes.md` — historical completion narrative contradicted/qualified by later v13 remediation rows.
- `docs/operations/v13.10.1-release-notes.md` — same canonical coverage as v13 remediation/dead-kind rows.

### Binary reference (0)

- `docs/EMQuantAPI_CPP_Mac.pdf` — vendor API reference; the actionable integration plan is captured as `LROOT-EMQ`.

### Expanded-manifest working records and generated evidence (0 each)

- `.planning/2026-07-16-quant-platform-assessment/progress.md` — working log; substantive findings are canonicalized in `.planning/2026-07-16-quant-platform-assessment/findings.md` and v18 claim rows.
- `.planning/2026-07-16-quant-platform-assessment/task_plan.md` — assessment workflow; its blocked full-test gate is captured as `LNEW-PLAN-GATES`.
- `.superpowers/sdd/base.txt` — one-line base commit pointer, not a requirement.
- `.superpowers/sdd/progress.md` — v17 recovery ledger duplicating `LSP-V17R2` and `LSP-V173`.
- `AGENTS.md` — mandatory audit/development rules, applied as claim rule IDs rather than treated as historical feature promises.
- `CLAUDE.md` — completion/evidence constraints, applied to production-caller and live-verification classification.
- `docs/monitor-runs/2026-06-30/sandbox-blocked.log` — raw failure transcript supporting root-cause G; no independent requirement beyond `LROOT-G`.
- `reports/backtest_report_20260305.md` — generated one-trade backtest output, not an implementation specification.
- `reports/macro_recommendations_20260305.md` — generated dated market output, not an implementation specification.
- `reports/stock_analysis_20260305.md` — generated dated stock-analysis output, not an implementation specification.
- `README.md` — current landing-page summary; its functional/completion statements duplicate canonical claims already captured for event extraction, backtesting, data red lines, monitor/review, Sina/Baostock and PAM authentication (`LSP-EVENT`, `LNEW-IMPROVE-DONE`, `L18-P0`, `LROOT-SINA`, `LNEW-SDD-BAO`, `LSP-AUTH`). It adds no independent acceptance criterion.

## Audit caveats

- No live network, external broker, real-account, real push-sink, coverage, or retained audit-log validation was performed; claims requiring those are `unverifiable`, `partial`, or `unresolved`.
- Type/module existence never counted as completion without a production caller. A production caller without tests/failure handling also did not count as complete.
- Historical “done” notes were not trusted when later documents reopened the same work or fixed-SHA code contradicted them.
- The strongest blocking evidence is independent of documentation drift: swallowed/default data, synthetic health, 120-second quote freshness, fake broker implementations, and missing persistence/reconciliation are present at the fixed snapshot.
