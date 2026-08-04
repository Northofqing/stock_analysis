# Findings & Decisions

## Requirements

- Restore abnormal delivery behavior, account information, and useful notifications quickly.
- During audit repair, ordinary business pushes may be paused; risk pushes may not bypass audit safety.
- Do not integrate a real broker account in this scope.
- Accept complete user-confirmed position snapshots; the latest activated update wins for local valuation.
- Calculate per-position market value, unrealized P&L, return, and daily closing-price movement P&L.
- Use real validated daily closes and show both position-update time and price trade date.
- Preserve Frozen and all action gates; local valuation is advisory and cannot authorize orders.
- Document the design, create an implementation plan, then execute independent workstreams in parallel.

## Research Findings

- The local database has open position rows, but the real-account snapshot is stale and daily P&L is unavailable.
- `compute_account_mode_metrics_blocking` cannot currently return complete metrics because broker trade-sync watermark evidence is absent.
- Metric failure collapses all display fields to `None`, causing a misleading “position missing” banner.
- DataMode capability successes are process-local; startup evaluates before producers warm the tracker.
- OrderBook has no real production depth source, but the current message presents it as recoverable Missing.
- The DataMode template lacks provider, last-success, age, error, and retry fields and uses a static Quote recovery condition.
- The authoritative yearly delivery audit contains valid legacy rows with byte-exact empty code/entity keys.
- New full-chain semantic validation rejects that historical shape and poisons the process writer after the first failed append.
- Sink delivery occurs before authoritative audit; current failure handling can release dedup after a physical send.
- Top10 loading already uses `spawn_blocking`; the adjacent I-01 async path directly invokes a blocking reqwest loader.
- v18.x platform closure is explicitly design-only. The proposed quick closure implements a bounded subset without claiming full v18 completion.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Add versioned snapshot tables instead of overwriting `stock_position` | Full-snapshot completeness, source time, atomic activation, and audit history need a separate authority. |
| Empty full snapshot requires explicit confirmation | Prevent accidental omission from clearing all positions. |
| Do not require position quantity to be a multiple of 100 | Order-lot rules do not prove current holdings after corporate actions. |
| Use unadjusted closing prices with corporate-action validation | Compare the user’s current broker-style cost basis with actual tradable close. |
| Persist valuation runs before rendering | Messages must consume traceable results, not ephemeral calculations. |
| Qualify partial totals with coverage | Missing data must not become zero or an apparently complete portfolio result. |
| Split sink, L7, and audit outcomes | A physical send is not the same fact as durable delivery confirmation. |
| Normalize only exact empty legacy pairs in a read-only view | Restores compatibility without weakening v2 or rewriting immutable evidence. |
| Separate DisplayAccountFacts from ActionPortfolioMetrics | Keeps useful estimates visible while maintaining fail-closed trading safety. |

## New user snapshot (2026-07-22)

- Screenshot is a complete replacement snapshot, not an incremental trade update.
- Confirmed positions: 000813=1000 @ 6.407; 002131=3000 @ 6.118; 002208=200 @ 18.235;
  002421=3000 @ 4.064; 600396=500 @ 26.569; 600703=800 @ 17.092; 603948=400 @ 24.040.
- Account facts shown: total assets 60855.46, securities market value 50156.00,
  available cash 10699.46, position ratio 82.4%, daily P&L +613.40.
- Existing database snapshot remains the previous 2026-07-21 quantities, so all consumers
  currently read stale quantities until this complete snapshot is imported.
- Required presentation: real holdings and paper holdings may share one message, but must have
  separate sections, separate valuation totals, and separate trade/advice labels. T0 advice is a
  dedicated push kind and must use the latest confirmed real holdings as its reference.

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Existing 48-hour monitor plan is still present | Created a separate isolated plan and preserved the older plan files. |
| `BR-142` is referenced by an existing design while the current business-rule table ends at BR-141 | Each implementation plan must allocate IDs against then-current HEAD before editing code; no speculative ID is reserved in this design. |

## Resources

- `docs/superpowers/specs/2026-07-22-user-position-valuation-push-recovery-design.md`
- `docs/v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md`
- `docs/v19.x/v19.0-operational-clarity-design.md`
- `docs/superpowers/specs/2026-07-21-terminal-monitor-lifecycle-design.md`
- `src/event/dispatcher.rs`, `src/event/push_record.rs`, `src/bin/monitor/notify.rs`
- `src/bin/monitor/main.rs`, `src/bin/monitor/push_templates.rs`, `src/monitor/data_mode.rs`
- `src/database/positions.rs`, `src/database/account_snapshot.rs`, `src/risk/account_mode.rs`
