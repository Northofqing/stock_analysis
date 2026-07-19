# Standards audit coverage

- Fixed commit: `c1e53321b2f4fb5d1f21cc0baf7ff4ade1ffcb7b`
- Target set: latest `code_manifest.tsv` (snapshot code/config/support plus explicitly excluded workspace-only assets)
- Inventory: **430 paths**; **402 checked**, **28 skipped with reasons**
- Method: every fixed-snapshot text blob was read in full; lexical candidates were traced back to surrounding implementation and call sites before findings were recorded.
- Finding ledger: `standards_findings.tsv` (63 concrete rows).
- Important boundary: workspace-only/excluded paths and binary assets are listed explicitly but are not attributed to the fixed commit.

## Fixed-commit gate evidence

| command | result |
| --- | --- |
| `cargo fmt --check` | FAIL (exit 1; production files require formatting) |
| `cargo clippy --all-targets --all-features -- -D warnings` | FAIL (exit 101; 335 errors) |
| `cargo test --quiet` | FAIL (exit 101; v14_e2e line 285 E0061) |
| `bash tools/compliance/check.sh` | FAIL (exit 1; two referenced check scripts absent) |

These commands ran from a clean `git archive` of the fixed commit, so unrelated worktree changes did not affect the evidence.

| path | status | reason |
| --- | --- | --- |
| `.claude/settings.json` | checked | plugin/tooling configuration reviewed; no additional violation |
| `.env.example` | checked | operational defaults, source modes, auth and isolation reviewed; corroborates STD-058 |
| `.github/pull_request_template.md` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `.github/workflows/ci.yml` | checked | workflow/gate enforcement and PR evidence |
| `.github/workflows/compliance.yml` | checked | workflow/gate enforcement and PR evidence |
| `.github/workflows/coverage.yml` | checked | workflow/gate enforcement and PR evidence |
| `.github/workflows/pr-template-lint.yml` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `.gitignore` | checked | process-output tracking policy reviewed; see STD-062 |
| `AGENTS.md` | checked | mandatory Standards source read in full; rules 2.1-2.10 and Gates applied |
| `CLAUDE.md` | checked | mandatory Completion/Spec Evidence source read in full and applied |
| `Cargo.lock` | checked | build/dependency and target configuration |
| `Cargo.toml` | checked | build/dependency and target configuration |
| `benches/intraday_tick.rs` | checked | benchmark implementation and evidence value reviewed; fmt gate covered by STD-003 |
| `config/chain.toml` | checked | thresholds, defaults, code linkage and 2.9 drift |
| `config/strategy.toml` | checked | thresholds, defaults, code linkage and 2.9 drift |
| `deploy/grafana_dashboard.json` | checked | operational monitoring/freshness configuration reviewed; see STD-063 |
| `diesel.toml` | checked | migration/schema-generation configuration reviewed; no additional violation |
| `migrations/v10-p0-1-virtual-reason/down.sql` | checked | schema safety, isolation and audit constraints |
| `migrations/v10-p0-1-virtual-reason/up.sql` | checked | schema safety, isolation and audit constraints |
| `migrations/v12-p0-paper-and-adjust/down.sql` | checked | schema safety, isolation and audit constraints |
| `migrations/v12-p0-paper-and-adjust/up.sql` | checked | schema safety, isolation and audit constraints |
| `migrations/v12-p1-account-mode/down.sql` | checked | schema safety, isolation and audit constraints |
| `migrations/v12-p1-account-mode/up.sql` | checked | schema safety, isolation and audit constraints |
| `src/agent/auction_agent.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/context.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/loop_runner.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/analysts.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/arbitrator.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/cost_board.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/debate.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/slices.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/multi_agent/trace.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/state.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tool.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/toolbelt.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tools.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tools_chip.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tools_money_flow.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tools_news.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tools_research.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/tools_sector.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/agent/validation.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/analyzer/analyze.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/analyzer/client.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/analyzer/macro_rec.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/analyzer/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/analyzer/prompts.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/analyzer/types.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/app/bootstrap.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/app/context.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/app/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/app/modes.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/app/schedule.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/auth/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/auth/operator.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/agent_test.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/backfill_daily.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/backfill_predictions.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/boll_macd_backtest.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/lhb_query.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/daily_report_router.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/dryrun_report.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/e2e_test.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/freshness.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/health.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/l6_sink.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/main.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/market_data.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/metrics.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/news_aggregator_init.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/notify.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/push_templates.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/recovery.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/v13_diag.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/v14_adapter.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/monitor/webhook_alert.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/produce_winrate_samples.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/rsi_optimize.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/test_em_fetch.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/v14_e2e.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bin/winrate_simulator.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/breakout/engine.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/breakout/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/breakout/position.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/breakout/signal.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/broker.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/broker/ib.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/bus/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/calendar.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/chart_generator.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/cli.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/config.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/announcement.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/baostock_provider.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/chain_registry.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/chip_distribution.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/consensus.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/eastmoney_provider.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/fallback.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/financials.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/gtimg_provider.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/halt_status.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/industry.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/intraday_kline.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/ipo_date.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/limit_status.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/money_flow.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/news_item.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/north_flow.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/rustdx_provider.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/service.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/sina_news_provider.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/sina_provider.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/stock_code_map.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/valuation_history.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/data_provider/yahoo.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/account_mode_log.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/agent_logs.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/concepts.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/execution_tracking.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/factor_snapshot.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/kline.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/lhb.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/position_shares.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/positions.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/database/repository.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/capital_verify.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/decision_decide.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/decision_panel.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/decision_render.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/exclusion.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/holding_plan.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/intraday_monitor.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/layers.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/leader.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/live_plan.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/pre_trade_filter.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/rotation.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/sector_score.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/decision/t0_advisor.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/deep_analyzer.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/enums.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/errors.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/event/bus.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/event/dispatcher.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/event/envelope.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/event/jsonl_writer.rs` | skip | workspace-only concurrent user change; absent from fixed snapshot and excluded from evidence |
| `src/event/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/event/push_record.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/http_client.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/cross.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/divergence.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/kdj.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/macd.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/multi_period.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/rsi.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/indicators/skdj.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/lhb_analyzer.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/lib.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/llm/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/llm/providers.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/llm/registry.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/llm/ticker_extractor.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/main.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/async_overview.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/indices.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/lhb_review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/limit_chain_review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/limit_up.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/market_stage_confidence.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/performance_feedback.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/post_close_review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/sector_history.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/sector_monitor.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_analyzer/statistics.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/market_data.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/models.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/adaptive.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/alert.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/alert_log.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/attribution.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/auction.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/checklist.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/data_mode.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/data_quality.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/detector.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/entity_linker.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/event_bus.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/integration.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/news_ai.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/news_monitor.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/prediction.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/rate_budget.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/risk.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/scanner.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/signal_fusion.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/monitor/signal_state.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/aggregator/feed.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/aggregator/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/dispatcher.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/impact.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/ipo/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/ipo/supply_chain.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/sink.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/news/stock_mapper.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/config.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/email.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/feishu.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/report.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/service.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/notification/wechat.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/auction_agent.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/bom_kb.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/candidate_panel.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/candidate_state.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/chain_mapper.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/discover.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/event_extractor/adapter.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/event_extractor/classifier.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/event_extractor/core.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/event_extractor/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/event_extractor/rule_filter.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/hit_case.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/impact.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/launch_gate.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/news_audit.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/news_outcome.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/news_ranker.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/real_alpha.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/scheduler.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/score.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/virtual_reason.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/opportunity/winrate.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/performance/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/performance/snapshot.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/analyze.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/backtest_runner.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/chain_analysis/fetchers.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/chain_analysis/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/data.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/extra_context.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/macro_news.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/market_regime.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/multi_timeframe.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/position_tracker.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/price_stats.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/reporting.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/result_types.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/score_breakdown.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/section_utils.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/summary_notify.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/technical_report.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/trade_type.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/pipeline/veto_rules.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/portfolio/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/portfolio/store.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l1/event.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l1/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l2/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l2/template.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l4/dispatcher.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l4/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l5/governance.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l5/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l6/external_sinks.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l6/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l6/sink.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l7/analytics.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l7/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/push_l7/sqlite_store.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/registry/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/equity.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/factor_ic.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/factor_report.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/failure_attribution.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/journal.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/lhb_review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/limit_chain_review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/market_stage.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/performance_feedback.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/report.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/signal_review.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/sop.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/review/tomorrow_watchlist.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/account_mode.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/action_gate.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/cash_guard.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/env_guard.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/limits.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/sector_exit.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/stop_loss.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/veto_chain.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/risk/veto_rules_live.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/schema.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/bocha.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/cls.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/cls_sign.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/cninfo.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/eastmoney.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/em_announcement.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/em_industry_news.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/gelonghui.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/gov_policy.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/jin10.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/kcb_daily.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/serpapi.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/sina_flash.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/sse_szse.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/tavily.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/wallstreetcn.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/weibo_hot.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/providers/xueqiu.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/service.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/search_service/types.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/sharpe_calculator.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/signal/market_event.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/signal/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/signal/push_recorder.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/boll_macd.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/bollinger_zscore.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/contrarian.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/core.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/lot.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/multi_factor.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/multi_timeframe.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/rsi/common.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/rsi/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/rsi/precision.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/rsi/standard.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/_helpers.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/auction_anomaly.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/breakout.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/llm_select.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/main_net_inflow.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/momentum.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/news_catalyst.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/sector_leader.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/strategy/v16_4/volume_surge.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/trading/mod.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/trading/paper_engine.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/trading/paper_trade.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/trading/risk_adapter.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/traits.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/trend_analyzer.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/types.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `src/util.rs` | checked | full production blob; rules 2.1-2.10, Gates and Completion integration |
| `tests/baostock_provider_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/bom_kb_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/chain_exclusive.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/e2e_dedup.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/e2e_prediction_verify.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/event_extractor_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/fallback_post_close_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/fallback_sina_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/flash_filter.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/holding_summary_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/launch_gate_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/market_event_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/news_item_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/north_flow_option_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/notification_channels_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/opportunity_e2e_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/position_tracker_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/ranking.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/review_timeout_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/rule_filter_benchmark.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/score_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/sina_news_provider_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/sina_provider_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/stock_code_map_test.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/test_data_freshness_check.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/test_design_contradiction.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/v11_three_sources.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/v12_p0_3_halt.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tests/winrate_tests.rs` | checked | regression oracle, isolation and Gate D evidence |
| `tools/EMQuantAPI_CPP_Mac/EMQuantAPI_CPP_Mac.pdf` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/EmQuantAPI/EmQuantAPI.h` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/EMQuantAPI_CPP_Mac/x64/EmQuantAPITestExe/Makefile` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/EMQuantAPI_CPP_Mac/x64/EmQuantAPITestExe/main.cpp` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/ServerList.json.e` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/emquantapitest` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/image/EMApp.ico` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/image/Tips_error.png` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/image/edit_bg.png` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/image/tab1_bg.png` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/image/tab2_bg.png` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/image/tab3_bg.png` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/libEMQuantAPIx64.dylib` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/x64/bin/loginactivator_mac` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/EMQuantAPI_CPP_Mac/指标手册V2.7.2.0.CHM` | skip | workspace-only binary/vendor asset; absent from fixed snapshot and excluded from semantic review |
| `tools/compliance/check.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/compliance/fixtures/test_check_business_rules_dynamic.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/compliance/lib/check_business_rules.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/compliance/lib/check_data_freshness.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/compliance/lib/check_design_contradiction.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/compliance/lib/check_fake_impl.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/compliance/lib/check_no_silent_fallback_global.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/compliance/lib/check_no_silent_fallback_push.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/_timeout_lib.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/backfill_daily.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/one_shot/backfill_predictions.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/one_shot/check_dispatcher_health.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/one_shot/fixtures/test_g5a_baseline.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/fixtures/test_timeout_lib.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/migrate_rollback_v10_p0_1.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/migrate_run_v10_p0_1.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/p0_loop_validation.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/seed_chain_lhb.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/one_shot/seed_trades.sh` | checked | compliance/release behavior and fail-open paths |
| `tools/one_shot/verify_prediction.sh` | skip | workspace-only/excluded; absent from fixed snapshot, so not attributable to audited commit |
| `tools/one_shot/winrate_review.sh` | checked | compliance/release behavior and fail-open paths |
