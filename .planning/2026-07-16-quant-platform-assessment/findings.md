# Findings & Decisions

## Requirements
- Understand the repository's code and documentation.
- Review it as a quantitative developer, especially code/data integrity and trading safety.
- Review it as a quantitative product, especially the end-to-end product loop.
- Compare the capability model with public, non-proprietary practices of leading quantitative institutions and execution systems.
- Land the outcome as an 18.x design under `docs/18.x/`.

## Research Findings
- The system describes itself as an event-driven, live A-share trading monitor rather than a batch strategy.
- `CLAUDE.md` names bounded contexts for portfolio, market, signals, opportunities, review, decisions, risk, and breakout analysis.
- Repository-wide red lines require real data, explicit error handling, validation, freshness, live/test isolation, order limits, and auditable traces.
- The repository has versioned design history through `docs/v17.x/`; no `docs/18.x/` directory exists yet.
- The codebase contains broad surfaces for data providers, database repositories, broker/QMT integration, pipeline/backtest, strategies, portfolio, decision, review, risk, notifications, and a production monitor binary.
- The worktree was clean before this assessment; the only untracked files are the assessment's persistent planning records.
- `docs/v17.x/v17.x-dev-plan-revised.md` reports partial L6 push and news-aggregator integrations, but the remaining v17.5–v17.8 migration is explicitly pending approval.
- `src/lib.rs` exports both a broad business-domain surface and a push stack (`push_l1`, `push_l2`, `push_l4`–`push_l7`); comments acknowledge a missing L3 renderer and preserve compatibility paths.
- The production monitor initializes L6 and the global news aggregator, but L6 delivery remains opt-in through `STOCK_ANALYSIS_PUSH_V6_ENABLE=1`; the default remains the legacy delivery path.
- The trading module identifies its provided gateway as `SimulatedExecutionGateway`; paper trading is explicitly isolated from real positions. The observed production monitor code contains paper-trade status/persistence references, so evidence currently supports a monitored/paper-assisted workflow rather than verified live order routing.
- `src/bin/monitor/main.rs` (~11,855 LOC) and `push_templates.rs` (~10,570 LOC) are dominant integration surfaces, which raises change-locality and review-risk concerns.
- The K-line fallback path performs a multi-source first-valid race and runs `validate_daily_kline_quality` before accepting data; all source failures surface as an error. This is a strong design point for daily bars.
- `DataFetchService::get_financials`, `get_money_flow`, and `get_intraday_shape` return non-`Result` values and use empty/default fallbacks on provider failure. That behavior conflicts with red lines 2.1–2.2 for any production decision path that cannot distinguish unavailable data from a genuine zero/empty observation.
- Intraday caching is five minutes during market sessions in `DataFetchService`, materially looser than the repository's five-second realtime-quote freshness requirement (2.4). The assessment must distinguish this service from any separately-wired quote path.
- The database stores daily bars, simulated positions/trades, paper-trade executions, position adjustments, factor snapshots, and account-mode logs. The observed schemas do not yet evidence a normalized real broker order/execution/commission/reconciliation ledger.
- The repository has reusable quote/position/NAV freshness functions, but its `DataMode` state model treats quotes as unsafe only after 120 seconds. That is materially inconsistent with rule 2.4's five-second quote requirement unless a stricter production gate is demonstrably enforced before each action.
- Current QMT material is a dated v14 planning/implementation document for local market-data cache, not proof of broker execution. The actual source inventory does not expose a QMT provider file, while `trading::SimulatedExecutionGateway` is concrete.
- Risk adapters provide pre-trade checks for paper trading and the simulated gateway includes 60-second business-ID deduplication. These controls are useful prototypes, but they are not sufficient evidence of a broker-confirmed live execution lifecycle or account reconciliation.
- Backtesting includes explicit look-ahead-risk commentary and a factor-snapshot-aware path, which is a sound direction. However, the current fallback path knowingly computes factors from the slice-end state where historical snapshots are unavailable; any result from that compatibility path must be labelled non-production research evidence.
- `execution_tracking` is an in-memory report model; it does not demonstrate durable broker execution reconciliation.
- Production market-data fetches call the reusable quote freshness validator, but some monitor paths synthesize capability statuses as fresh or assign `DataMode::Full` directly. The 18.x design must make a single, timestamped health snapshot authoritative and ban synthetic “full” inputs outside tests.
- The existing decision layer permits executable advice in `Degraded` mode. For live execution, 18.x must separate “may notify” from “may trade” and use stricter, action-specific data contracts.
- Public AQR material treats transaction cost as part of portfolio construction, and frames portfolio construction, risk management, and cost control as core sources of implementation quality rather than post-processing.
- Public Man Group research describes execution impact as a sequence-level problem, and its systematic-investing material separates alpha research, portfolio construction, risk management, and execution research. This supports an explicit execution/TCA domain rather than a direct strategy-to-order shortcut.
- SEC market-access guidance requires systematic pre-trade financial/regulatory controls, authorized access, immediate post-trade execution reporting, and ongoing documented review. It is a U.S. rule, not a direct A-share requirement, but a high-quality safety benchmark.
- The Federal Reserve's 2026 model-risk guidance emphasizes a model inventory, validation/continuous performance monitoring, governance, and effective challenge. This is adopted as an institutional design reference, not claimed as a legal requirement for this project.
- `cargo test --lib` passed: 1,164 passed, 0 failed, 7 ignored. The compiler emitted many warnings (unused imports/variables, deprecated dispatcher API, and an unreachable pattern); this means the library test suite is green but does not meet a warnings-as-errors readiness bar.
- Full `cargo test` is blocked during `src/bin/v14_e2e.rs` compilation: line 285 calls `Dispatcher::dispatch` with two arguments while the current signature requires three. This is outside the changed documentation paths. `bash tools/compliance/check.sh` passes, including a daily-data freshness result of 2026-07-15 (one trading day behind the 2026-07-16 assessment date).

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Evidence-led review | Findings must cite concrete repository paths and execution-path checks. |
| No data/config/code changes | The requested deliverable is a design; avoiding runtime changes preserves fund/data safety. |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Mandatory Copilot instructions file is absent | Treat as a documented pre-flight gap rather than inventing its contents. |
| Large command output truncated while reading architecture/code | Switched to targeted reads and symbol searches; do not infer details from omitted output. |
| `cargo test --lib` output was lengthy | Captured exit status and test summary; recorded warning categories rather than treating the truncated transcript as exhaustive. |

## Resources
- `AGENTS.md`
- `docs/ENGINEERING_RULES_V2.md`
- `CLAUDE.md`
- `Cargo.toml`
- `docs/v16.x/`, `docs/v17.x/`
- `src/bin/monitor/`, `src/data_provider/`, `src/database/`, `src/broker/`, `src/risk/`, `src/portfolio/`, `src/pipeline/`
- SEC Rule 15c3-5: https://www.sec.gov/rules-regulations/2011/06/risk-management-controls-brokers-or-dealers-market-access
- Federal Reserve SR 26-2: https://www.federalreserve.gov/supervisionreg/srletters/SR2602.htm
- AQR, Transaction Costs: Practical Application: https://www.aqr.com/insights/research/white-papers/transactions-costs-practical-application
- AQR, The Alpha in Portfolio Construction: https://www.aqr.com/Insights/Research/Trade-Publication/The-Alpha-in-Portfolio-Construction
- Man Group, Counting the Costs: https://www.man.com/insights/counting-the-costs
- Man Group, graduate-programme role overview: https://www.man.com/graduate-programmes
