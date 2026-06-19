# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```
cargo build                             # compilation
cargo test --lib                        # all unit tests (~289)
cargo run --bin monitor                 # live monitoring
cargo run --bin monitor -- --test       # full pipeline smoke test
cargo run --bin monitor -- --review     # manual post-market review
```

## Architecture (v3–v6)

The system is an **event-driven live trading monitor** for A-share (Chinese stocks), not a batch quant strategy.

**7 Contexts** (DDD bounded contexts, no Clean Architecture layers):

| Context | Directory | Job |
|---------|-----------|-----|
| Portfolio | `portfolio/` | Single source of truth for positions, trades, ledger |
| Market | `monitor/` + `data_provider/` + `market_analyzer/` | Quotes, announcements, detection |
| Signal | `signal/` | Unified Signal/SignalSet data structures |
| Opportunity | `opportunity/` | News → industry chain → candidate discovery |
| Review | `review/` | Daily/weekly post-trade review & falsification |
| Decision | `decision/` | Exclusion, sector tiering, capital verification, rotation |
| Risk | `risk/` | Hard position/sector/cash limits (parallel to monitor/risk.rs) |
| Breakout | `breakout/` | Multi-dimensional volume breakout analysis (v6) |

**Data sources** (multi-host fallback): Eastmoney push2 (3 hosts) → Sina → Yahoo. Flash news from Jin10 + WallStreetCN.

## Critical Rules (from AGENTS.md, MUST priority)

**Data**: All data must be real. No mock data in production paths. Missing data → log warning, don't silently fill. Failed data source → explicit error.

**Development flow**: `/architecture-patterns` → 4-angle challenge → `/project-planner` → code → `/review` (must check old modules!) → fix → test.

**When new capability is added**: Check whether existing old modules should upgrade to use it. Document the decision for each.

## Configuration

- `.env`: `STOCK_LIST` (watchlist codes), `WECHAT_SEND_SCRIPT`, `DATABASE_PATH`
- `config/*.toml`: chain rules, exclusion boards, announcement keywords, monitor timers — SIGHUP hot-reloadable
- All config files have code-level `const` fallbacks if toml is missing
