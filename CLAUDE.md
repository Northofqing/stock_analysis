# CLAUDE.md

Project context only. This file defines no repository-specific agent process or development gates.

## Commands

```bash
cargo build
cargo test --lib
cargo run --bin monitor
cargo run --bin monitor -- --test --push-dry-run
cargo run --bin monitor -- --review
```

## Architecture

The project is an event-driven live A-share trading monitor. Its main bounded contexts are:

| Context | Directory |
| --- | --- |
| Portfolio | `portfolio/` |
| Market | `data_gateway/`, `monitor/`, `market_analyzer/` |
| Signal | `signal/` |
| Opportunity | `opportunity/` |
| Review | `review/` |
| Decision | `decision/` |
| Risk | `risk/` |
| Breakout | `breakout/` |

## Configuration

- `.env`: `STOCK_LIST`, `WECHAT_SEND_SCRIPT`, `DATABASE_PATH`
- Runtime TOML inputs: `config/strategy.toml`, `config/chain.toml`
