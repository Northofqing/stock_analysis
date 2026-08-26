# R-04 Dragon-Tiger Unified Gateway Design (BR-194 / BR-197)

## Status

Gate A design for the BR-194 R-04 production review slice. The approved parent
contract is to replace only the active R-04 acquisition caller, preserve every
distinct source disclosure, aggregate the user report by stock, and retain
explicit empty/unavailable states.

## Problem and current production path

`monitor --review` reaches
`dispatch_post_session_review -> dispatch_r04_lhb_outcome`, but that dispatcher
still imports `market_analyzer::lhb_review::fetch_recent_lhb`. The legacy loader
performs consumer-local Eastmoney HTTP, groups rows in a `HashMap` by stock
code, sums distinct source disclosures, and cannot retain exact buy-five and
sell-five seats. That explains why the executable still uses the old
acquisition style even when the adjacent data library contains a new typed
provider.

## Data flow

1. R-04 waits until the existing 21:00 publication threshold.
2. `DragonTigerGateway` builds one bounded `MarketDragonTigerRequest` for the
   requested trading date and the source-ranked top five disclosures. Eastmoney
   performs whole-market discovery before applying that bound; lower-ranked
   incomplete disclosures cannot be silently admitted or used to poison the
   confirmed top-five batch.
3. A blocking worker creates, uses, and drops `EastmoneyClient`.
4. A real empty provider response becomes `GatewayBatch::VerifiedEmpty` with
   its original provenance.
5. A non-empty provider batch is routed once through
   `MarketDragonTigerRouter` admission without issuing a second request.
6. The Gateway validates date, A-share identity, shared evidence, exact
   buy-five/sell-five completeness, and source entry identity.
7. It groups all distinct source `TRADE_ID` disclosures by stock. Distinct
   reasons remain distinct; their net amounts are not summed. The stock ranking
   value is the highest valid source net amount.
8. Only positive-net stocks are sorted and the stock Top 5 is applied after
   grouping.
9. The full acquisition outcome is appended through the BR-159 immutable
   acquisition audit helper. Audit failure closes the Gateway.
10. R-04 renders one stock section containing every retained disclosure and
    each disclosure's exact five buy and five sell seats.
11. The immutable source binding retains the provider `observed_at` bytes
    exactly. Preparation and durable source-binding revalidation must call the
    same strict parser for canonical RFC3339 or Magic provenance
    `unix-ms:<unsigned-decimal>`, then compare the decoded UTC instant with the
    typed delivery origin. A format accepted at preparation may not be rejected
    by a narrower downstream parser. Malformed, non-canonical, out-of-range or
    instant-mismatched values reject the whole R-04 delivery before any Launch,
    L5, durable admission or sink call.

## Typed downstream contract

`DragonTigerStockReview` contains the instrument identity, the ranking net
amount and all `DragonTigerSourceDisclosure` values. Each source disclosure
contains its source `entry_id` (`TRADE_ID` identity), reason, amounts, turnover
and ten typed `DragonTigerSeatReview` values. Batch evidence remains on
`GatewayBatch`, so the consumer cannot detach records from the provider batch.

The upstream whole-market contract currently does not expose a security name.
R-04 therefore renders the canonical stock code rather than joining a second
provider or fabricating a name. Adding a provider-sourced name is a separate
upstream contract change.

## Failure modes

- Invalid date/limit: non-retryable `invalid_request`.
- Transport/rate/provider failure: retryable `unavailable`.
- Protocol/decode/core, evidence mismatch, incomplete seats, non-A-share row,
  duplicate conflict, or invalid ordering: non-retryable `partial`.
- Real complete empty response: `VerifiedEmpty`, mapped to R-04 `NoData`.
- BR-159 database/audit unavailable: retryable fail-closed error.
- Malformed provider observation time: non-retryable fail-closed error; no
  timestamp is inferred from local time.
- Parser-contract drift between preparation and durable binding validation:
  non-retryable fail-closed error and a required cross-layer regression; no
  second format-specific parser is permitted.
- Task join failure: retryable `blocking_task_failed` and audited if possible.
- Push rejection remains governed by the existing ReviewLhb delivery path.

No missing field is filled, no other provider is joined, and unavailable is
never converted to empty.

## BR-197 component-scoped data-quality amendment

R-04 is an after-close, provider-complete report. Its computation consumes the
admitted Dragon-Tiger batch and does not consume realtime Quote, Kline, News,
MoneyFlow or OrderBook state. Requiring the process-wide intraday DataMode to
be at least `Degraded` makes the 21:00 report impossible whenever realtime
Quote has correctly become stale after market close, even though the R-04
batch is complete and current for the requested trading date.

The canonical R-04 SourceOnly route therefore uses `DataMode::Down` as its L5
minimum. This is not a fake-data or always-send bypass: the route still reads
the real process-local DataMode for analytics, while its data-quality authority
is the exact R-04 binding validation described above. Invalid/missing provider
evidence, date mismatch, partial ten-seat disclosure, canonical/hash drift, or
text drift still reject before Launch/L5/durable admission/sink. Quiet hours,
daily limits, Launch, analytics, durable budget/dedup/fence, sink, push log,
audit and hydration remain unchanged. Generic counted delivery and every other
PushKind retain their existing DataMode contracts.

## Old module disposition and source guard

The final BR-164 cutover deletes
`market_analyzer::lhb_review::fetch_recent_lhb` and the mixed
`LhbDataFetcher` acquisition/cache facade. R-04, the general analysis target
append, `--lhb`, and the `lhb_query` Today/Date commands all consume
`DragonTigerGateway`; no production path reads `lhb_daily` as a realtime
source. The historical table remains untouched for audit/migration evidence.
Pure auxiliary scoring accepts an explicit `DragonTigerStockReview` and uses
only disclosure count and explicit net amounts under BR-162; it does not infer
security names, price changes, institutions, hot-money style, or ratings. A
source guard locks the production R-04 dispatcher to `DragonTigerGateway` and
rejects the old loader symbol or direct HTTP in that block.

## Validation

- Focused Gateway aggregation/validation unit tests.
- R-04 wait/empty/unavailable/available dispatcher tests.
- End-to-end preparation -> counted binding -> durable revalidation tests for
  both RFC3339 and `unix-ms`, plus malformed/non-canonical `unix-ms` rejection.
- R-04 SourceOnly L5 accepts a complete canonical binding when global DataMode
  is `Down`, while invalid bindings and non-R-04 kinds remain rejected.
- Source guard for the active production dispatcher.
- Compile/tests temporarily against
  `target/magic_market_unified_work`, then restore committed dependency paths.
- `cargo fmt --check`
- focused `cargo clippy -- -D warnings`
- `cargo build --bin monitor`
- `bash tools/compliance/check.sh`
- real-date Gateway probe and `monitor --review` log inspection when the
  upstream contract is present in the resolved dependency.

## Rollback

Revert the new `dragon_tiger` Gateway module/export, the R-04 dispatcher and
renderer changes, BR-162, and this design document. Do not alter
`src/data_gateway/review.rs`, acquisition-audit storage, or the upstream R-04
implementation as part of rollback.
