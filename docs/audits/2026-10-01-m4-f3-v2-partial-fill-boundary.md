# M4/F3 V2 partial-fill boundary audit — 2026-10-01

## Finding

`PaperBookV2` is currently a verified, zero-fill genesis cutover, not an order or fill ledger. A partial-fill writer would need a new reviewed catalog generation and an admitted investment/market-fact contract. Adding a fill to CatalogV5 or treating a research fee quote as a fill would break its frozen meaning. No V2 fill or production cutover is enabled by this audit.

## Present contracts

| Area | Evidence | Current boundary |
| --- | --- | --- |
| V1 paper ledger | `src/trading/paper_ledger_execution.rs`, `src/trading/paper_ledger_tests.rs` | Independently seeded, event-replayed account with whole-order `Filled` / `NotFilled` / `Invalidated` / `Rejected` outcomes and FIFO lots. A partial **lot sale** is supported; a parent order with multiple partial fills is not represented. V1 fee model is frozen as `lot-rates-v1`. |
| V2 catalog | `src/database/paper_book_v2_ledger_schema_v1.rs` | `paper_book_v2_event` requires `seq=1` and `kind='Genesis'`; `paper_book_v2_head` requires `version=1` and rejects updates/deletes. There are no V2 parent-order or fill rows. |
| V2 owner and writer | `src/trading/paper_book_v2.rs` | `verify_owner_rows_on` replays the V1 source and checks the V2 genesis anchor. The only cutover writer, `cutover_for_isolated_test`, is compiled only under `cfg(test)` and requires an isolated `TEST_CODE` database. `read_v2_on` exposes a read-only genesis view. |
| V2 fee model | `src/performance/fee_policy.rs`, `src/performance/fee_evidence.rs` | An explicit descriptor computes modeled commission and stamp tax for each assumed Shanghai A-share stock fill, with a per-fill minimum and execution-date tax bracket. Instrument qualification is a caller assertion; transfer and other charges are excluded. Research batch quotes already reject duplicate fill IDs but cannot attest execution or provide complete trading cost. |
| F2 action gate | `src/decision/trading_fact_denial.rs`, `src/data_gateway/qualified_trading_facts.rs` | The production trading-fact gateway currently yields a denial for lifecycle, price regime, and suspension. Its receipt is not an `InvestmentDecisionId` or `ApprovedPaperIntent`. |

The existing CatalogV5 isolated test in `src/trading/paper_ledger_tests.rs` deliberately rejects an attempted `Fill` event after genesis. That is the correct result for the current schema, not missing DML to work around.

## Required decisions before a fill writer

1. Freeze the source-qualified, as-of market snapshot and F2 immutable decision/approved paper intent. A fill model must not infer lifecycle, price limits, suspension, volume, or liquidity from a ticker, an unverified caller claim, or a research fee quote.
2. Define the parent order and fill identity/state contract: stable order and fill keys, cumulative and remaining quantity, partial fill followed by more fills or cancel/expiry, duplicate and conflicting retry behavior, and the timestamp used for fee-date and T+1 checks. Specify the versioned participation/slippage/no-fill rules from evidence rather than assuming an ideal fill fraction.
3. Introduce a separately reviewed catalog generation for append-only order/fill events and a replay-checked head. Preserve CatalogV5 genesis bytes and the V1 event chain; test overfill, duplicate/reordered events, per-fill minimum, FIFO and cash/projection replay in an isolated database before any writer is compiled for production.
4. Bind every generated fill to the explicit reviewed fee descriptor and its stated coverage. Resolve missing transaction-cost components or reject `CompleteTradingCost`; do not silently substitute the V1 tax model or a fixed research assumption.
5. Complete the Gate P archive, receipt, daily reconciliation, restore, and observation evidence and obtain a separate explicit production cutover decision. The zero-fill test cutover is not that decision.

## Verification scope

This is a source and contract audit only. No database schema, event bytes, owner, fill algorithm, trade path, or production state was changed.
