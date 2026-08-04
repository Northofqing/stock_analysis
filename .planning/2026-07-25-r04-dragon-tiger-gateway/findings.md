# Findings — R-04 Dragon-Tiger Gateway

## Reproduction

`rg -n "dispatch_r04_lhb_outcome|fetch_recent_lhb|market_analyzer::lhb_review"
src/bin/monitor/push_templates.rs` deterministically shows the active production
dispatcher importing and invoking the legacy loader.

## Dependency resolution

Committed Cargo paths point to `../magic-market-data-rs`. Its current `main`
commit is `7c0267de0379ade81002b36ae5850bd7e7ae4d83`, which does not yet contain the
whole-market dragon-tiger types. The approved upstream implementation exists
in `target/magic_market_unified_work`.

## Upstream contract

- `MarketDragonTigerRequest` is one date plus a maximum-100 disclosure limit.
- `EastmoneyClient::market_dragon_tiger` returns complete
  `DragonTigerDisclosure` records.
- Every disclosure has one entry and exactly five buy plus five sell seats.
- Source `entry_id` preserves the Eastmoney `TRADE_ID` identity.
- The Router validates complete evidence, unique entry IDs and canonical order.
- The contract does not currently carry a security name.

## Live probe

- A request limit of 100 correctly failed closed on lower-ranked
  `301234:2026-07-22:100379791`, for which the source returned 4 buy and 5 sell
  seats.
- The upstream provider performs whole-market discovery before applying the
  requested disclosure bound. R-04 therefore requests the canonical top five
  disclosures, whose upstream live probe proved complete, and then groups
  those disclosures by stock.
