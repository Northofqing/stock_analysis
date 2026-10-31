# magic-market-core

Provider-neutral checked market-data contracts.

The core distinguishes Shanghai, Shenzhen, and Beijing exchanges and exposes
source-evidenced contracts for quotes, bars, current/historical minute points,
trades, five-level books, intraday money flow, ranked post-close flow, auctions,
security metadata, research, signals, capital events, public content, financial
statements, limit pools and ETF options. Minute
points keep cumulative quantity and optional cumulative amount; metadata keeps
board, ST status, listing date, and the price-limit rule/version independently
optional. A provider may return fields it can prove without inventing the rest.

`NorthboundDailyStat` keeps the trading date/channel, CNY turnover, source
trade count, explicit quota availability, ETF turnover and exactly ranked
Top10 securities. `DragonTigerEntry`/`DragonTigerSeat` keep official reasons
and complete side/rank evidence without treating public trading information as
Level-2 data.

Construction and deserialization use the same validation boundary. Invalid
prices, quantities, identifiers, dates, evidence strings, OHLC ranges, and
contradictory quality reports are rejected instead of entering normalized
data. Validated requests expose read-only accessors so callers cannot bypass
their constructors with struct literals.

`PostCloseFlow` is a contract rather than a synthetic calculation: it keeps the
source rank, close/change, main net amount and optional source-backed board and
price-limit rule. Its checked constructor and deserializer require record-level
`source_at` with the same calendar date as `trading_date`; no Provider should
advertise it until exact post-close ranking semantics and source time have been
verified.

```rust
use magic_market_core::{
    AssetClass, DataBatch, Exchange, InstrumentId, Price, Provenance,
};

let instrument = InstrumentId::new(
    Exchange::Shanghai,
    "600396",
    AssetClass::Equity,
)?;
let price = Price::new(14.92)?;
let provenance = Provenance::new("tdx", "2026-07-22T10:00:01+08:00")?
    .with_source_at("2026-07-22T10:00:00+08:00")?
    .with_batch_id("tdx-quote-1")?;
let batch = DataBatch::strict(vec![(instrument, price)], provenance);

assert!(batch.quality().is_complete());
assert!(batch.quality().issues().is_empty());
# Ok::<(), magic_market_core::CoreError>(())
```
