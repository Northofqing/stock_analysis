# Source-Backed Dynamic Price Limits Design

**Status:** Gate A candidate. The user explicitly approved removal of the fixed
`20%` data red line on 2026-08-04. Gate B is blocked until this document and the
BR-205 registry row receive a fresh independent C0/I0/M0 review.

## 1. Goal

Remove every production data-quality rejection, filter, panic and manual-
confirmation requirement whose only evidence is that a daily or adjacent change
exceeds a fixed percentage. Preserve the real source value. Where a current
price-limit boundary is genuinely needed, use the exact source-backed boundary
for that instrument and trading session.

This separates two concepts that the old `20%` gate conflated:

1. **observed market fact** -- the return or price change that the provider
   actually reported; and
2. **current trading rule** -- the source-backed upper/lower executable prices,
   or an explicit statement that the session has no daily limit.

## 2. Scope and non-goals

### In scope

- repository data-redline wording and BR-205 registration;
- historical K-line, selection and review admission;
- realtime equity/index quote field validation;
- current-session limit-up/limit-down classification;
- order-safety range evidence;
- retirement of the BR-171 fixed-20% production confirmation path;
- tests and compliance checks that currently require fixed `20%` rejection.

### Not changed

- strategy-owned thresholds such as take profit, portfolio caps, factor bands,
  scoring weights and candidate page limits;
- price positivity, finite-number, OHLC, volume/amount, identity, completeness,
  freshness, continuity/duplicate and provider-consistency checks;
- the requirement that an order price be inside a proven current daily range;
- immutable retention of already-written BR-171 audit rows.

## 3. Domain model

One owner module exposes a closed current-session contract:

```rust
pub enum DailyPriceLimitState {
    Bounded(SourceBackedPriceRange),
    NoLimit(SourceBackedNoLimitEvidence),
    Unavailable(PriceLimitUnavailable),
}

pub struct SourceBackedPriceRange {
    pub instrument: InstrumentId,
    pub trading_date: NaiveDate,
    pub lower: Price,
    pub upper: Price,
    pub provider: ProviderId,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
}

pub struct SourceBackedNoLimitEvidence {
    pub instrument: InstrumentId,
    pub trading_date: NaiveDate,
    pub provider: ProviderId,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub rule_version: String,
}

pub struct PriceLimitUnavailable {
    pub instrument: InstrumentId,
    pub trading_date: NaiveDate,
    pub attempted_provider: ProviderId,
    pub source_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: Option<String>,
    pub reason: PriceLimitUnavailableReason,
    pub retryable: bool,
}

pub enum PriceLimitUnavailableReason {
    CompleteResponseMissingBothBounds,
    UnsupportedSourceContract,
}
```

Construction validates canonical identity, `0 < lower <= upper`, exact request
date/session binding, provider and batch identity, no future timestamps and the
applicable freshness gate. Fields stay private after construction.

`InstrumentId` is accepted only after the BR-173 canonical A-share resolver has
proved exchange/share-class/current-identity compatibility; constructing an
`InstrumentId` from a code prefix alone is not sufficient evidence.

`NoLimit` is not inferred from a listing-date arithmetic rule or missing bounds.
It requires an explicit source fact bound to the same instrument/date/session.
One missing bound is `Unavailable`, not `NoLimit`.

`Unavailable` is evidence, not a bare error label. It binds the exact request
identity, attempted provider, available source timestamp/batch identity, local
observation time, stable reason and retryability. A consumer must retain that
evidence in its decision/audit record. Absence of provider time or batch ID is
represented explicitly as missing and never replaced with local time or a
synthetic identifier.

The initial real bounded source is the existing Magic Tencent
`MarketStatistics` contract exposed through `CompanyDataGateway`; it carries
exact `upper_limit` and `lower_limit` prices plus source evidence. Magic TDX
remains the primary bars/lifecycle source. TDX-derived board/ST/code information
cannot manufacture a percentage or boundary when TDX does not supply one.

## 4. Historical data flow

```text
Magic bars batch
  -> exact instrument/provider/batch evidence
  -> validate and sort unique trading dates into ascending order
  -> finite positive OHLC + valid volume/amount
  -> trading-date uniqueness and continuity
  -> provider-reported pct/reference-previous-close consistency when supplied
     OR derive the adjacent return from the now-sorted source closes and label it
     `DerivedFromSourceCloses`
  -> split/dividend/lifecycle consistency when required
  -> preserve actual return without magnitude ceiling
  -> admitted batch
```

Historical admission never asks BR-171 for operator confirmation. A `35%`,
`100%` or larger structurally consistent fact is not a bad row merely because of
its magnitude. A contradictory provider `pct_chg`, broken OHLC, duplicate/gap,
wrong instrument or missing source evidence still fails explicitly.

Provider-reported and locally derived percentages are different evidence kinds.
The derived value is computed only after canonical date ordering and duplicate
checks, and its provenance binds both source close records. It must never be
described as provider-reported. Input/provider order cannot affect the result.

## 5. Current-session data flow

```text
canonical instrument + trading date
  -> CompanyDataGateway / Magic MarketStatistics
  -> exact source upper + lower + provenance
  -> DailyPriceLimitState::Bounded
  -> quote classification and order safety
```

If the real source explicitly supports a no-limit state, the gateway may return
`NoLimit`. Until that source contract exists, a response with no bounds is
`Unavailable`. No consumer may reconstruct 5/10/20/30 percent from the code,
board or name.

Observed quote/index percentage fields are checked for finiteness and source
consistency, not against a universal absolute bound. Indexes normally have no
equity daily price-limit range, so no equity limit inference applies to them.

### 5.1 Async acquisition boundary

`CompanyDataGateway::market_statistics` remains asynchronous. The async
monitor/order orchestration layer must acquire and validate the quote plus
`DailyPriceLimitState` before it constructs an order-safety request. It then
moves an unforgeable typed capability into the synchronous pure validation
layer. Synchronous broker/order traits must not create or destroy a Tokio
runtime, call `block_on`, or hide provider I/O. This prevents recurrence of the
runtime-drop panic and keeps acquisition failure auditable before any business
ID, reservation, order or delivery side effect.

For the initial provider set, only a fresh exact two-sided Magic Tencent
response can construct `Bounded`. No integrated provider currently exposes an
explicit no-limit fact, so the `NoLimit` variant has no production constructor
until that upstream contract exists. Missing, one-sided or unsupported bounds
therefore become auditable `Unavailable`; they are never derived from listing
age.

## 6. BR-171 retirement

The `daily_change_confirmation` tables and hash chain remain readable and
immutable for five-year audit retention. Production code stops:

- generating pending confirmations from a fixed percentage;
- requiring a matching confirmation before admitting bars;
- writing new confirmations through `confirm_daily_change`.

The CLI becomes a read-only legacy audit inspector or is removed if no retained
operational reader needs it. Database startup continues validating the legacy
hash chain; rollback must never delete the tables or triggers.

## 7. Failure modes

| Condition | Result |
| --- | --- |
| large but structurally consistent historical move | accept actual value |
| provider percentage contradicts prices | `InvalidEvidence` |
| invalid OHLC, non-finite value, duplicate or gap | `InvalidEvidence` |
| both fresh exact current bounds available | `Bounded` |
| explicit same-session no-limit source fact | `NoLimit` |
| both bounds absent under a complete supported response | `Unavailable` |
| one-sided/stale/conflicting bounds | explicit `GatewayError` |
| code/board/ST heuristic is the only rule evidence | `Unavailable` |

No failure path returns a default percentage, guessed boundary, zero, stale
cache or operator-confirmation workaround.

## 8. Migration slices

1. **Policy/docs:** add BR-205, correct AGENTS 2.3, supersede the fixed clauses
   in older BR rows and retire conflicting design text.
2. **Historical facts:** remove fixed-magnitude gates and BR-171 calls from bars,
   database, selection, review, closing valuation, LHB audit storage, prediction
   outcomes, decision input and paper-data admission while retaining all
   structural checks; sort before deriving returns and record the derivation
   kind.
3. **Current limits:** add the closed `DailyPriceLimitState` owner and project
   exact Magic MarketStatistics bounds through it.
4. **Consumers:** migrate broker/order safety first, followed by realtime quote
   admission, monitor limit classification/events, paper execution, sector/limit
   breadth, TDX T0, intraday shape, minute data, index/market regime and
   selection. Delete `LimitStatusCalculator`, `infer_limit_pct`, `9.5%`,
   current-price-times-`1.1`, and all other rule inference only after no
   production caller remains. `Unavailable` disables only rule-dependent
   classification/trading and never discards an otherwise valid market fact.
5. **Legacy audit:** disable BR-171 writes, retain startup verification/readback,
   immutable triggers and canonical hash compatibility, and update
   fixtures/checkers. The legacy ledger's internal canonical `20%` field may
   remain for historical byte/hash verification only; no production admission
   or new append may consult it.

Each slice must compile and test independently. A slice that removes an old
consumer must remove its fallback in the same commit.

### 8.1 Closed production-consumer inventory

The Gate-B migration is incomplete while any row below still uses a fixed
magnitude or inferred limit rule. Exact line numbers may move; the named files
and responsibilities are the stable owners.

| Responsibility | Production owner(s) | Required BR-205 migration |
| --- | --- | --- |
| shared daily quality / BR-171 admission | `src/monitor/data_quality.rs` | remove fixed magnitude/pending generation; retain structure and pct consistency |
| historical route and outcome bars | `src/data_gateway/historical_bars.rs` | remove confirmation lookup; sort before derived return; preserve evidence kind |
| daily persistence/readback | `src/database/kline.rs`, `src/database/repository.rs` | remove fixed-magnitude rejections |
| selection quality | `src/selection/quality.rs` | accept structurally valid actual returns |
| index/market regime | `src/data_gateway/index.rs`, `src/pipeline/market_regime.rs` | finite/source consistency only; no equity-rule inference |
| TDX settled daily / intraday shapes | `src/data_gateway/magic_tdx_t0.rs`, `src/data_gateway/intraday_shape.rs` | remove 20% admission gates; use exact state only for same-session price bounds |
| minute capabilities | `src/data_gateway/market_capabilities.rs` | remove adjacent-minute 20% gate; retain timestamp/price/volume invariants |
| realtime monitor batch and limit identity | `src/bin/monitor/market_data.rs`, `src/bin/monitor/main.rs` | retain actual quote; replace inferred 5/10/20/30, 9.5 and `price*1.1` identities |
| broker/order safety | `src/broker.rs`, `src/trading/order_safety.rs`, `src/strategy/core.rs` | acquire async typed state before synchronous validation; Unavailable rejects |
| legacy inferred-limit implementation | `src/data_provider/limit_status.rs` | delete after all typed-state consumers migrate |
| sector/limit presentation | `src/market_analyzer/sector_monitor.rs`, `src/bin/monitor/push_templates.rs` | classify only from exact Bounded evidence; unknown stays unknown |
| admitted-position persistence | `src/pipeline/position_tracker.rs` | remove downstream 20% re-rejection |
| prediction/backtest outcomes | `src/database/mod.rs`, `src/bin/produce_winrate_samples.rs` | accept finite source-backed actual outcomes |
| legacy LHB audit storage | `src/database/lhb.rs` | remove fixed 20% validity rule; retain arithmetic/identity invariants |
| decision input | `src/decision/decision_decide.rs` | remove fixed magnitude rejection |
| dormant tier/auction/statistics checks | `src/monitor/data_quality.rs`, `src/opportunity/auction_agent.rs`, `src/market_analyzer/statistics.rs` | remove only data-validity ceilings; retain clearly named strategy thresholds |
| BR-171 legacy ledger | `src/database/daily_change_confirmation.rs`, `src/bin/confirm_daily_change.rs`, `src/database/mod.rs` | disable append/admission use; preserve immutable schema/hash verification/read-only audit |

BR-205 supersedes the fixed-magnitude, manual-confirmation or inferred-rule
clauses in BR-092/097/122/125/127/131/134/147/156/164/171/179/187/193. Other
clauses in those rules remain in force. Old rows are retained as historical
decision records rather than rewritten; the later BR-205 row is authoritative
for this conflict.

## 9. Validation

Focused tests cover:

- main-board, ST, STAR/ChiNext/Beijing and newly listed examples whose actual
  source changes exceed `20%` without magnitude rejection;
- `Bounded`, explicit `NoLimit`, missing-bound `Unavailable`, stale/conflicting
  evidence and cross-instrument/date/batch rejection;
- provider percentage mismatch, bad OHLC, gaps, duplicates and corporate-action
  conflicts still failing;
- order range accepting exact boundaries and rejecting outside prices;
- repository scan proving no production data-quality path uses a fixed
  percentage magnitude or `manual_confirmation_required` for price change.

Required gates:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --summary-only
cargo build --release --bin monitor
```

Gate D additionally requires isolated real-source evidence showing the exact
instrument/date/batch bounds and a no-fixed-threshold historical batch. It does
not require or authorize a real order.

## 10. Rollback

Use dedicated commits per migration slice and revert in reverse order. Rollback
may restore old consumers only together with their prior source contract, but it
must not restore the fixed `20%` red line or delete legacy audit evidence. If the
new current-bound source is unavailable, the safe rollback behavior is typed
`Unavailable` and order refusal.
